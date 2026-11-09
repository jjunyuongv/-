//! 이미지·PDF 경로 엔드투엔드 평가 (PLAN.md 13단계 E5a·E5b)
//!
//! 문서마다: (D0) PDF 텍스트 레이어 + 래스터 / (D1~D4) OCR → 탐지 → 서명란 → 마스킹 → 정답 박스와 비교
//! - **유출률 (주 지표)**: 정답 PII 박스마다 마스크 면적 비율 < 90%면 그 span 유출 (정책상 가리지 않는 SALARY 제외)
//! - 서명란 커버리지: 근로자·친권자 서명·도장·지장 박스가 90% 이상 가려진 비율. 사업주(정책상 끔)는 50% 이상 가려졌으면 "과잉"
//! - 오탐: 하드네거티브 박스가 50% 이상 가려지면 오탐. 참고 항목은 중립
//! - 시간: 쪽당·문서당 처리 시간 (OCR·추출 / 탐지 / 서명란·마스킹), 파일 저장·채점 시간은 제외

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use image::RgbImage;
use ocr::{OcrEngine, OcrPage};
use pii_core::{BoxLabel, Detection, DocLabel, Label};
use serde::Serialize;

use crate::{EvalError, Pipeline};

/// 난이도별 집계
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct DiffStats {
    pub docs: usize,
    pub pages: usize,
    pub spans: usize,
    pub leaked: usize,
    /// 라벨 → (유출, 전체)
    pub by_label: BTreeMap<String, (usize, usize)>,
    /// OCR(또는 PDF 텍스트)에 PII 값이 온전히 나온 span 수 (마스킹 상한)
    pub value_in_text: usize,
    pub sign_boxes: usize,
    pub sign_covered: usize,
    pub employer_sign_boxes: usize,
    pub employer_sign_masked: usize,
    pub negatives: usize,
    pub negatives_masked: usize,
    /// 실패 유형 → 건수
    pub failures: BTreeMap<String, usize>,
    pub page_secs: Vec<f64>,
    pub doc_secs: Vec<f64>,
    pub extract_secs: f64,
    pub detect_secs: f64,
    pub mask_secs: f64,
    pub slm_calls: usize,
}

/// 볼록 사각형 안의 점인가
fn in_quad(q: &[[f32; 2]; 4], x: f32, y: f32) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let c = (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
        if c.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// 정답 박스(4점)가 사각형들에 덮인 면적 비율 (격자 표본)
pub fn quad_coverage(q: &[[f32; 2]; 4], rects: &[[f32; 4]]) -> f64 {
    let xs = q.iter().map(|p| p[0]);
    let ys = q.iter().map(|p| p[1]);
    let (x0, x1) = (
        xs.clone().fold(f32::MAX, f32::min),
        xs.fold(f32::MIN, f32::max),
    );
    let (y0, y1) = (
        ys.clone().fold(f32::MAX, f32::min),
        ys.fold(f32::MIN, f32::max),
    );
    let step = ((x1 - x0).min(y1 - y0) / 8.0).clamp(0.5, 4.0);
    let (mut inside, mut covered) = (0usize, 0usize);
    let mut y = y0 + step / 2.0;
    while y < y1 {
        let mut x = x0 + step / 2.0;
        while x < x1 {
            if in_quad(q, x, y) {
                inside += 1;
                if rects
                    .iter()
                    .any(|r| r[0] <= x && x <= r[2] && r[1] <= y && y <= r[3])
                {
                    covered += 1;
                }
            }
            x += step;
        }
        y += step;
    }
    if inside == 0 {
        return 1.0;
    }
    covered as f64 / inside as f64
}

fn flat(s: &str) -> String {
    pii_core::Normalized::new(s)
        .text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn bbox(q: &[[f32; 2]; 4]) -> [f32; 4] {
    let xs = q.iter().map(|p| p[0]);
    let ys = q.iter().map(|p| p[1]);
    [
        xs.clone().fold(f32::MAX, f32::min),
        ys.clone().fold(f32::MAX, f32::min),
        xs.fold(f32::MIN, f32::max),
        ys.fold(f32::MIN, f32::max),
    ]
}

fn intersects(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// 쪽 하나 처리 결과 (마스크 사각형 + 쪽 텍스트)
pub struct PageRun {
    pub masked: RgbImage,
    pub rects: Vec<[f32; 4]>,
    pub text: OcrPage,
    pub page_text: String,
    pub detections: Vec<Detection>,
    /// 가린 서명란 영역 (진단용)
    pub sign_rects: Vec<[f32; 4]>,
    /// `rects`와 같은 순서의 출처 (탐지·서명란·필드·빈 값) — 오탐 원인 진단용
    pub rect_sources: Vec<String>,
}

/// 쪽 하나: 탐지 → 서명란 → 마스킹 (시간 기록)
pub fn process_page(
    pipe: &mut Pipeline,
    image: &RgbImage,
    text: OcrPage,
    stats: &mut DiffStats,
) -> Result<PageRun, EvalError> {
    let t = Instant::now();
    let pt = mask::page_text(&text);
    let (dets, timing) = pipe.detect(&pt.text)?;
    stats.detect_secs += t.elapsed().as_secs_f64();
    stats.slm_calls += timing.slm_calls;
    let t = Instant::now();
    let mut rects = Vec::new();
    let mut rect_sources = Vec::new();
    for d in &dets {
        let rs = mask::detection_rects(&pt, std::slice::from_ref(d));
        let src = format!("탐지 {}/{:?}", crate::label_key(d.label), d.detector);
        rect_sources.extend(std::iter::repeat_n(src, rs.len()));
        rects.extend(rs);
    }
    let mut sign_rects = Vec::new();
    for r in detect_seal::detect(&text, Some(image), &pipe.pack.signature) {
        if pipe.pack.masks_signature(&r.owner) {
            rects.push(r.rect);
            rect_sources.push("서명란".to_string());
            sign_rects.push(r.rect);
        }
    }
    // 필드 영역: 개인정보 양식 키의 값 칸 전체 (ISSUE-002·003)
    for (src, rs) in [
        (
            "필드",
            detect_seal::field_regions(&text, &pipe.pack.fields.keys),
        ),
        (
            "빈 값",
            detect_seal::empty_value_regions(&text, &pipe.pack.signature),
        ),
    ] {
        rect_sources.extend(std::iter::repeat_n(src.to_string(), rs.len()));
        rects.extend(rs);
    }
    let mut masked = image.clone();
    mask::fill(&mut masked, &rects);
    stats.mask_secs += t.elapsed().as_secs_f64();
    Ok(PageRun {
        masked,
        rects,
        text,
        page_text: pt.text,
        detections: dets,
        sign_rects,
        rect_sources,
    })
}

/// 쪽 하나 채점
pub fn score_page(
    label: &DocLabel,
    page_no: u32,
    run: &PageRun,
    pack: &rulepack::RulePack,
    stats: &mut DiffStats,
) {
    let boxes: Vec<&BoxLabel> = label.boxes.iter().filter(|b| b.page == page_no).collect();
    let text_flat = flat(&run.page_text);
    let ocr_rects: Vec<[f32; 4]> = run
        .text
        .lines
        .iter()
        .flat_map(|l| l.char_boxes.iter().copied())
        .collect();
    // span: 이 쪽에 박스가 있는 것
    for s in &label.spans {
        if !pack.masks(s.label) || s.label.is_image_only() {
            continue;
        }
        let sb: Vec<&&BoxLabel> = boxes.iter().filter(|b| b.span_id == Some(s.id)).collect();
        if sb.is_empty() {
            continue;
        }
        stats.spans += 1;
        let value = flat(&s.value);
        let in_text = !value.is_empty() && text_flat.contains(&value);
        stats.value_in_text += usize::from(in_text);
        let leaked = sb.iter().any(|b| quad_coverage(&b.poly, &run.rects) < 0.9);
        let e = stats.by_label.entry(crate::label_key(s.label)).or_default();
        e.1 += 1;
        if leaked {
            e.0 += 1;
            stats.leaked += 1;
            // 실패 유형
            let partial = sb.iter().any(|b| quad_coverage(&b.poly, &run.rects) > 0.05);
            let kind = if partial {
                "박스 부분 가림 (매핑·경계)"
            } else if !in_text {
                let seen = sb
                    .iter()
                    .any(|b| ocr_rects.iter().any(|r| intersects(*r, bbox(&b.poly))));
                if seen {
                    "OCR 오인식"
                } else {
                    "OCR 미검출"
                }
            } else if crate::SENSITIVE.contains(&s.label) {
                let line = run
                    .page_text
                    .lines()
                    .find(|l| flat(l).contains(&value))
                    .unwrap_or("");
                if pack.slm.triggers.iter().any(|t| line.contains(t.as_str())) {
                    "SLM 탐지 누락"
                } else {
                    "SLM 트리거 누락"
                }
            } else {
                "규칙·NER 탐지 누락"
            };
            *stats.failures.entry(kind.to_string()).or_default() += 1;
        }
    }
    // 서명란 표시
    for b in boxes.iter().filter(|b| b.kind.is_some()) {
        let cov = quad_coverage(&b.poly, &run.rects);
        let owner = b
            .owner
            .map_or("unknown".to_string(), |o| format!("{o:?}").to_lowercase());
        if pack.masks_signature(&owner) {
            stats.sign_boxes += 1;
            stats.sign_covered += usize::from(cov >= 0.9);
            if cov < 0.9 {
                // 앵커 미검출(이 자리 근처에 서명란 영역 없음) / 영역 어긋남(있는데 덜 덮음)
                let bb = bbox(&b.poly);
                let h = (bb[3] - bb[1]).max(8.0);
                let near = [bb[0] - h, bb[1] - h, bb[2] + h, bb[3] + h];
                let kind = if run.sign_rects.iter().any(|r| intersects(*r, near)) {
                    "서명란 영역 어긋남"
                } else {
                    "서명란 앵커 미검출"
                };
                *stats.failures.entry(kind.into()).or_default() += 1;
            }
        } else {
            stats.employer_sign_boxes += 1;
            stats.employer_sign_masked += usize::from(cov >= 0.5);
        }
    }
    // 하드네거티브
    for b in boxes.iter().filter(|b| b.negative_id.is_some()) {
        stats.negatives += 1;
        let hit = quad_coverage(&b.poly, &run.rects) >= 0.5;
        stats.negatives_masked += usize::from(hit);
        if hit {
            // 오탐 종류 (하드네거티브 source 접미사, 값 없음)
            let kind = label
                .negatives
                .iter()
                .find(|n| Some(n.id) == b.negative_id)
                .map_or("?".to_string(), |n| crate::negative_kind(&n.source));
            *stats.failures.entry(format!("오탐 {kind}")).or_default() += 1;
            // 어느 영역이 덮었나 (가장 많이 덮은 출처)
            let src = run
                .rects
                .iter()
                .zip(&run.rect_sources)
                .map(|(r, s)| (quad_coverage(&b.poly, std::slice::from_ref(r)), s.as_str()))
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map_or("?", |x| x.1);
            *stats.failures.entry(format!("오탐:{src}")).or_default() += 1;
        }
    }
}

/// 이미지 파일 → OCR 쪽 (시간 기록)
pub fn ocr_image(
    engine: &OcrEngine,
    path: &Path,
    stats: &mut DiffStats,
) -> Result<(RgbImage, OcrPage), EvalError> {
    let img = image::open(path)
        .map_err(|e| EvalError::Other(format!("{}: {e}", path.display())))?
        .to_rgb8();
    let t = Instant::now();
    let page = engine.recognize(img.clone())?;
    stats.extract_secs += t.elapsed().as_secs_f64();
    Ok((img, page))
}

/// 라벨이 Label 하나라도 정책상 가리는가 (리포트용)
pub fn masked_label(pack: &rulepack::RulePack, l: Label) -> bool {
    pack.masks(l)
}

/// 확인용: 마스킹 이미지 위에 정답 박스 (가려짐 초록 / 유출 빨강 / 서명란 가려짐 파랑·덜 가림 주황)
pub fn overlay(
    masked: &RgbImage,
    label: &DocLabel,
    page_no: u32,
    rects: &[[f32; 4]],
    pack: &rulepack::RulePack,
) -> RgbImage {
    let mut img = masked.clone();
    let (w, h) = img.dimensions();
    let mut line = |a: [f32; 2], b: [f32; 2], c: [u8; 3]| {
        let n = ((b[0] - a[0]).abs().max((b[1] - a[1]).abs()) as usize).max(1);
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let (x, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            for dx in 0..3i32 {
                for dy in 0..3i32 {
                    let (px, py) = (x as i32 + dx - 1, y as i32 + dy - 1);
                    if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                        img.put_pixel(px as u32, py as u32, image::Rgb(c));
                    }
                }
            }
        }
    };
    for b in label.boxes.iter().filter(|b| b.page == page_no) {
        let cov = quad_coverage(&b.poly, rects);
        let color = if b.kind.is_some() {
            let owner = b
                .owner
                .map_or("unknown".to_string(), |o| format!("{o:?}").to_lowercase());
            if !pack.masks_signature(&owner) {
                continue;
            }
            if cov >= 0.9 {
                [30, 60, 230]
            } else {
                [255, 140, 0]
            }
        } else if b.negative_id.is_some() {
            // 하드네거티브가 가려졌으면 보라 (오탐)
            if cov >= 0.5 { [200, 0, 200] } else { continue }
        } else if b.span_id.is_some() {
            if b.label.is_some_and(|l| !pack.masks(l)) {
                continue;
            }
            if cov >= 0.9 { [0, 170, 0] } else { [230, 0, 0] }
        } else {
            continue;
        };
        for i in 0..4 {
            line(b.poly[i], b.poly[(i + 1) % 4], color);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_of_rotated_quad() {
        let q = [[10.0, 10.0], [50.0, 12.0], [49.0, 30.0], [9.0, 28.0]];
        assert!(quad_coverage(&q, &[[0.0, 0.0, 100.0, 100.0]]) > 0.999);
        assert!(quad_coverage(&q, &[]) < 0.001);
        let half = quad_coverage(&q, &[[0.0, 0.0, 29.5, 100.0]]);
        assert!((half - 0.5).abs() < 0.1, "{half}");
    }
}
