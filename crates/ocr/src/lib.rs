//! `oar-ocr` 래퍼 (PLAN.md 7·8단계): PP-OCRv5 server 검출 + 한국어 인식 → 줄 텍스트 + 줄 박스 + 글자 박스
//!
//! 모델 파일 (기본 `models/ocr/monkt/`, `monkt/paddleocr-onnx`): `det.onnx`(server_det), `rec.onnx`(한국어), `dict.txt`.
//! 오류 메시지에는 인식한 글자를 넣지 않는다 (개인정보 원문이 로그에 남지 않게).

use std::path::{Path, PathBuf};

use image::RgbImage;
use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("OCR 모델 파일이 없습니다: {0}")]
    MissingModel(String),
    #[error("OCR 엔진 준비 실패: {0}")]
    Build(String),
    #[error("OCR 실행 실패: {0}")]
    Predict(String),
}

/// 모델 파일 경로
#[derive(Debug, Clone)]
pub struct OcrModels {
    pub det: PathBuf,
    pub rec: PathBuf,
    pub dict: PathBuf,
}

impl OcrModels {
    /// `dir/det.onnx`, `dir/rec.onnx`, `dir/dict.txt`
    pub fn in_dir(dir: &Path) -> Self {
        OcrModels {
            det: dir.join("det.onnx"),
            rec: dir.join("rec.onnx"),
            dict: dir.join("dict.txt"),
        }
    }
}

/// 인식한 줄 하나
#[derive(Debug, Clone)]
pub struct OcrLine {
    pub text: String,
    pub confidence: f32,
    /// 줄 박스 4점 (이미지 픽셀)
    pub poly: [[f32; 2]; 4],
    /// 글자마다 박스 [x0, y0, x1, y1] (`text`의 글자 순서, 없으면 줄 박스를 글자 수로 나눔)
    pub char_boxes: Vec<[f32; 4]>,
}

/// 쪽 하나의 OCR 결과
#[derive(Debug, Clone)]
pub struct OcrPage {
    pub width: u32,
    pub height: u32,
    pub lines: Vec<OcrLine>,
}

pub struct OcrEngine {
    inner: OAROCR,
}

fn rect_of(points: &[oar_ocr::processors::Point]) -> [f32; 4] {
    let xs = points.iter().map(|p| p.x);
    let ys = points.iter().map(|p| p.y);
    [
        xs.clone().fold(f32::MAX, f32::min),
        ys.clone().fold(f32::MAX, f32::min),
        xs.fold(f32::MIN, f32::max),
        ys.fold(f32::MIN, f32::max),
    ]
}

/// 기울어진 줄에서 글자 박스의 세로 범위를 줄 4점의 위·아래 변에 맞춘다.
/// oar-ocr 글자 박스는 줄의 축 정렬 박스 높이를 그대로 써서, 기울어진 줄이면 위아래 줄까지 걸친다 (ISSUE-005).
/// 줄 안쪽으로만 줄인다 (원래 범위를 넘지 않음).
pub fn fit_box_to_poly(poly: &[[f32; 2]; 4], b: [f32; 4]) -> [f32; 4] {
    // 위 변·아래 변: 평균 y가 작은 쪽이 위
    let e1 = (poly[0], poly[1]);
    let e2 = (poly[3], poly[2]);
    let (top, bottom) = if e1.0[1] + e1.1[1] <= e2.0[1] + e2.1[1] {
        (e1, e2)
    } else {
        (e2, e1)
    };
    let at = |e: ([f32; 2], [f32; 2]), x: f32| -> Option<f32> {
        let dx = e.1[0] - e.0[0];
        if dx.abs() < 1.0 {
            return None;
        }
        let t = ((x - e.0[0]) / dx).clamp(0.0, 1.0);
        Some(e.0[1] + t * (e.1[1] - e.0[1]))
    };
    let (Some(t0), Some(t1), Some(b0), Some(b1)) = (
        at(top, b[0]),
        at(top, b[2]),
        at(bottom, b[0]),
        at(bottom, b[2]),
    ) else {
        return b;
    };
    let y0 = t0.min(t1).max(b[1]);
    let y1 = b0.max(b1).min(b[3]);
    if y1 - y0 < 2.0 {
        return b;
    }
    [b[0], y0, b[2], y1]
}

/// 같은 줄의 연속된 글자 박스들을 사각형으로 묶는다. 묶음 높이가 글자 박스 높이의 1.25배를 넘으면
/// (기울어진 줄) 새 묶음을 시작해, 긴 값도 기울기를 따라 여러 사각형으로 덮는다
pub fn segment_rects(boxes: impl IntoIterator<Item = [f32; 4]>) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    let mut cur: Option<([f32; 4], f32)> = None; // (묶음, 묶음 안 가장 큰 글자 높이)
    for b in boxes {
        let bh = b[3] - b[1];
        cur = match cur {
            Some((r, mh)) => {
                let u = [
                    r[0].min(b[0]),
                    r[1].min(b[1]),
                    r[2].max(b[2]),
                    r[3].max(b[3]),
                ];
                let mh2 = mh.max(bh);
                if u[3] - u[1] > 1.25 * mh2 {
                    out.push(r);
                    Some((b, bh))
                } else {
                    Some((u, mh2))
                }
            }
            None => Some((b, bh)),
        };
    }
    if let Some((r, _)) = cur {
        out.push(r);
    }
    out
}

impl OcrEngine {
    pub fn new(models: &OcrModels) -> Result<Self, OcrError> {
        for p in [&models.det, &models.rec, &models.dict] {
            if !p.is_file() {
                return Err(OcrError::MissingModel(p.display().to_string()));
            }
        }
        let inner = OAROCRBuilder::new(&models.det, &models.rec, &models.dict)
            .return_word_box(true)
            .build()
            .map_err(|e| OcrError::Build(e.to_string()))?;
        Ok(OcrEngine { inner })
    }

    pub fn recognize(&self, image: RgbImage) -> Result<OcrPage, OcrError> {
        let (width, height) = image.dimensions();
        let results = self
            .inner
            .predict(vec![image])
            .map_err(|e| OcrError::Predict(e.to_string()))?;
        let mut lines = Vec::new();
        for r in results.iter().flat_map(|r| r.text_regions.iter()) {
            let Some(text) = r.text.as_ref().map(|t| t.to_string()) else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            let pts = &r.bounding_box.points;
            let rect = rect_of(pts);
            let poly = if pts.len() == 4 {
                [
                    [pts[0].x, pts[0].y],
                    [pts[1].x, pts[1].y],
                    [pts[2].x, pts[2].y],
                    [pts[3].x, pts[3].y],
                ]
            } else {
                [
                    [rect[0], rect[1]],
                    [rect[2], rect[1]],
                    [rect[2], rect[3]],
                    [rect[0], rect[3]],
                ]
            };
            let n = text.chars().count();
            let char_boxes: Vec<[f32; 4]> = match &r.word_boxes {
                Some(wb) if wb.len() == n => wb.iter().map(|b| rect_of(&b.points)).collect(),
                // 글자 박스가 없거나 수가 다르면 줄 박스를 글자 수로 나눈다
                _ => {
                    let w = (rect[2] - rect[0]) / n.max(1) as f32;
                    (0..n)
                        .map(|i| {
                            [
                                rect[0] + w * i as f32,
                                rect[1],
                                rect[0] + w * (i + 1) as f32,
                                rect[3],
                            ]
                        })
                        .collect()
                }
            };
            let char_boxes = char_boxes
                .into_iter()
                .map(|b| fit_box_to_poly(&poly, b))
                .collect();
            lines.push(OcrLine {
                text,
                confidence: r.confidence.unwrap_or(0.0),
                poly,
                char_boxes,
            });
        }
        Ok(OcrPage {
            width,
            height,
            lines,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_box_follows_skewed_line() {
        // 오른쪽으로 40px 내려가는 줄 (높이 30)
        let poly = [[0.0, 0.0], [400.0, 40.0], [400.0, 70.0], [0.0, 30.0]];
        let b = fit_box_to_poly(&poly, [0.0, 0.0, 20.0, 70.0]);
        assert!(b[1] <= 0.5 && (b[3] - 32.0).abs() < 0.5, "{b:?}");
        let b = fit_box_to_poly(&poly, [380.0, 0.0, 400.0, 70.0]);
        assert!(
            (b[1] - 38.0).abs() < 0.5 && (b[3] - 70.0).abs() < 0.5,
            "{b:?}"
        );
    }

    #[test]
    fn segments_split_when_skewed() {
        // 반듯한 줄 → 한 묶음
        let flat: Vec<[f32; 4]> = (0..10)
            .map(|i| [i as f32 * 20.0, 0.0, i as f32 * 20.0 + 20.0, 30.0])
            .collect();
        assert_eq!(segment_rects(flat).len(), 1);
        // 글자마다 4px씩 내려감 → 여러 묶음, 묶음 높이 제한
        let skew: Vec<[f32; 4]> = (0..10)
            .map(|i| {
                let y = i as f32 * 4.0;
                [i as f32 * 20.0, y, i as f32 * 20.0 + 20.0, y + 30.0]
            })
            .collect();
        let s = segment_rects(skew);
        assert!(s.len() > 1);
        assert!(s.iter().all(|r| r[3] - r[1] <= 30.0 * 1.25));
    }
}
