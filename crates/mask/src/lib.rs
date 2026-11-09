//! 마스킹 (PLAN.md 12단계)
//!
//! - 쪽 텍스트: OCR(또는 PDF 텍스트 레이어) 줄을 읽기 순서로 이어 붙인다 — 같은 높이의 줄은 탭(표 칸), 행은 줄바꿈.
//!   글자마다 박스를 들고 있어 탐지 위치 → 박스로 바꾼다
//! - 이미지: 탐지 글자 박스를 줄마다 합친 사각형 + 여백 → 검은 상자. 서명란 영역도 소속 정책대로 검게
//! - PDF: **래스터화한 쪽 이미지만으로 새 PDF**를 만든다 → 텍스트 레이어·문서 정보·XMP·주석·양식 필드·책갈피·첨부가
//!   원본에서 넘어오지 않음 (새 문서, 원본 객체 복사 없음, XMP 끔)

use std::path::{Path, PathBuf};

use image::RgbImage;
use krilla::geom::{Size, Transform};
use krilla::image::Image;
use krilla::page::PageSettings;
use krilla::{Document, SerializeSettings};
use ocr::{OcrLine, OcrPage};
use pdfium_render::prelude::*;
use pii_core::Detection;

#[derive(Debug, thiserror::Error)]
pub enum MaskError {
    #[error("PDF 처리 실패: {0}")]
    Pdf(String),
    #[error(
        "pdfium 라이브러리를 찾지 못했습니다 (vendor/pdfium/bin/pdfium.dll 또는 PDFIUM_DYNAMIC_LIB_PATH)"
    )]
    NoPdfium,
}

/// 읽기 순서로 이은 쪽 텍스트 + 글자마다 박스 (구분자 탭·줄바꿈은 None)
#[derive(Debug, Clone, Default)]
pub struct PageText {
    pub text: String,
    pub boxes: Vec<Option<[f32; 4]>>,
    /// 글자마다 줄 번호 (구분자는 앞 줄)
    pub line_of: Vec<usize>,
}

fn rect_of(poly: &[[f32; 2]; 4]) -> [f32; 4] {
    let xs = poly.iter().map(|p| p[0]);
    let ys = poly.iter().map(|p| p[1]);
    [
        xs.clone().fold(f32::MAX, f32::min),
        ys.clone().fold(f32::MAX, f32::min),
        xs.fold(f32::MIN, f32::max),
        ys.fold(f32::MIN, f32::max),
    ]
}

/// OCR 줄들 → 읽기 순서 쪽 텍스트 (y 중심이 줄 높이 절반 안이면 같은 행 → x 순서, 탭으로 이음)
pub fn page_text(page: &OcrPage) -> PageText {
    // 쪽 기울기: 긴 줄 박스 윗변 기울기의 중앙값 → 기울기를 뺀 y로 행을 묶는다
    // (회전·원근 쪽에서 오른쪽 칸이 아래로 처져 키 칸과 값 칸이 다른 행으로 갈라지지 않게)
    let mut slopes: Vec<f32> = page
        .lines
        .iter()
        .filter_map(|l| {
            let dx = l.poly[1][0] - l.poly[0][0];
            (dx > 80.0).then(|| (l.poly[1][1] - l.poly[0][1]) / dx)
        })
        .collect();
    slopes.sort_by(f32::total_cmp);
    let slope = slopes.get(slopes.len() / 2).copied().unwrap_or(0.0);
    let ycorr = |r: &[f32; 4]| (r[1] + r[3]) / 2.0 - slope * (r[0] + r[2]) / 2.0;
    let mut idx: Vec<(usize, [f32; 4])> = page
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| (i, rect_of(&l.poly)))
        .collect();
    idx.sort_by(|a, b| ycorr(&a.1).total_cmp(&ycorr(&b.1)));
    let mut rows: Vec<Vec<(usize, [f32; 4])>> = Vec::new();
    for it in idx {
        let cy = ycorr(&it.1);
        let h = it.1[3] - it.1[1];
        match rows.last_mut() {
            Some(row)
                if row
                    .iter()
                    .any(|r| (ycorr(&r.1) - cy).abs() < 0.5 * h.min(r.1[3] - r.1[1]).max(4.0)) =>
            {
                row.push(it)
            }
            _ => rows.push(vec![it]),
        }
    }
    let mut out = PageText::default();
    for (ri, row) in rows.iter_mut().enumerate() {
        row.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
        if ri > 0 {
            out.text.push('\n');
            out.boxes.push(None);
            out.line_of.push(row[0].0);
        }
        for (k, (li, _)) in row.iter().enumerate() {
            if k > 0 {
                out.text.push('\t');
                out.boxes.push(None);
                out.line_of.push(*li);
            }
            let line: &OcrLine = &page.lines[*li];
            for (ci, ch) in line.text.chars().enumerate() {
                out.text.push(ch);
                out.boxes.push(line.char_boxes.get(ci).copied());
                out.line_of.push(*li);
            }
        }
    }
    out
}

/// 탐지 → 검은 상자 사각형 (줄마다 글자 박스 합 + 여백)
pub fn detection_rects(pt: &PageText, dets: &[Detection]) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for d in dets {
        // 줄마다 글자 박스를 모아 묶음 사각형으로 (기울어진 줄은 여러 묶음 — ISSUE-005)
        let mut run: Vec<[f32; 4]> = Vec::new();
        let mut cur_line: Option<usize> = None;
        for i in d.start.0..d.end.0.min(pt.boxes.len()) {
            let Some(b) = pt.boxes[i] else { continue };
            let li = pt.line_of[i];
            if cur_line.is_some_and(|l| l != li) {
                out.extend(ocr::segment_rects(run.drain(..)).into_iter().map(pad));
            }
            cur_line = Some(li);
            run.push(b);
        }
        out.extend(ocr::segment_rects(run).into_iter().map(pad));
    }
    out
}

/// 여백: 위아래 높이의 8%(OCR 박스는 이미 글자보다 위아래로 넓다), 좌우 높이의 30%
/// (CTC로 얻은 글자 박스는 양 끝 글자가 실제보다 좁게 잡힌다 — 개발용 세트 확인, 유출 방지 우선)
fn pad(r: [f32; 4]) -> [f32; 4] {
    let h = r[3] - r[1];
    let pv = (h * 0.08).max(3.0);
    let ph = (h * 0.3).max(4.0);
    [r[0] - ph, r[1] - pv, r[2] + ph, r[3] + pv]
}

/// 사각형들을 검게 칠한다
pub fn fill(img: &mut RgbImage, rects: &[[f32; 4]]) {
    let (w, h) = img.dimensions();
    for r in rects {
        let x0 = r[0].max(0.0) as u32;
        let y0 = r[1].max(0.0) as u32;
        let x1 = (r[2].ceil().max(0.0) as u32).min(w);
        let y1 = (r[3].ceil().max(0.0) as u32).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
    }
}

/// 쪽 이미지들만으로 새 PDF (dpi 기준 크기). 원본 PDF 객체를 하나도 쓰지 않는다
pub fn image_pdf(pages: &[RgbImage], dpi: f32) -> Result<Vec<u8>, MaskError> {
    let settings = SerializeSettings {
        xmp_metadata: false,
        ..SerializeSettings::default()
    };
    let mut doc = Document::new_with(settings);
    let k = 72.0 / dpi;
    for img in pages {
        let (w, h) = img.dimensions();
        let ps = PageSettings::from_wh(w as f32 * k, h as f32 * k)
            .ok_or_else(|| MaskError::Pdf("쪽 크기".into()))?;
        let mut page = doc.start_page_with(ps);
        let mut s = page.surface();
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for p in img.pixels() {
            rgba.extend_from_slice(&[p.0[0], p.0[1], p.0[2], 255]);
        }
        let image = Image::from_rgba8(rgba, w, h);
        let size = Size::from_wh(w as f32 * k, h as f32 * k)
            .ok_or_else(|| MaskError::Pdf("크기".into()))?;
        s.push_transform(&Transform::from_translate(0.0, 0.0));
        s.draw_image(image, size);
        s.pop();
        s.finish();
        page.finish();
    }
    doc.finish().map_err(|e| MaskError::Pdf(e.to_string()))
}

/// pdfium 연결: `PDFIUM_DYNAMIC_LIB_PATH` → `<root>/vendor/pdfium/bin`
pub fn bind_pdfium(root: &Path) -> Result<Pdfium, MaskError> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("PDFIUM_DYNAMIC_LIB_PATH") {
        dirs.push(PathBuf::from(d));
    }
    dirs.push(root.join("vendor/pdfium/bin"));
    for d in dirs {
        let lib = Pdfium::pdfium_platform_library_name_at_path(&d);
        if let Ok(b) = Pdfium::bind_to_library(&lib) {
            return Ok(Pdfium::new(b));
        }
    }
    Err(MaskError::NoPdfium)
}

/// PDF 한 쪽: 래스터 이미지(dpi) + 텍스트 레이어 줄(픽셀 좌표, 글자 박스)
pub struct PdfPageData {
    pub image: RgbImage,
    pub text: OcrPage,
}

/// PDF 읽기 → 쪽마다 래스터 + 텍스트 레이어 (D0 경로)
pub fn read_pdf(pdfium: &Pdfium, bytes: &[u8], dpi: f32) -> Result<Vec<PdfPageData>, MaskError> {
    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| MaskError::Pdf(e.to_string()))?;
    let scale = dpi / 72.0;
    let mut out = Vec::new();
    for page in doc.pages().iter() {
        let w_pt = page.width().value;
        let h_pt = page.height().value;
        let img = page
            .render_with_config(
                &PdfRenderConfig::new().set_target_width((w_pt * scale).round() as i32),
            )
            .map_err(|e| MaskError::Pdf(e.to_string()))?
            .as_image()
            .map_err(|e| MaskError::Pdf(e.to_string()))?
            .to_rgb8();
        let tp = page.text().map_err(|e| MaskError::Pdf(e.to_string()))?;
        // 글자 → 줄 (기준선 y가 비슷하고 x가 이어지는 글자)
        let mut chars: Vec<(char, [f32; 4])> = Vec::new();
        for ch in tp.chars().iter() {
            let Some(c) = ch.unicode_char() else { continue };
            if c == '\r' || c == '\n' || ch.is_generated().unwrap_or(false) {
                continue;
            }
            let Ok(r) = ch.loose_bounds() else { continue };
            chars.push((
                c,
                [
                    r.left().value * scale,
                    (h_pt - r.top().value) * scale,
                    r.right().value * scale,
                    (h_pt - r.bottom().value) * scale,
                ],
            ));
        }
        let mut lines: Vec<OcrLine> = Vec::new();
        let mut cur: Vec<(char, [f32; 4])> = Vec::new();
        let flush = |cur: &mut Vec<(char, [f32; 4])>, lines: &mut Vec<OcrLine>| {
            while cur.last().is_some_and(|x| x.0.is_whitespace()) {
                cur.pop();
            }
            if cur.is_empty() {
                return;
            }
            let r = cur
                .iter()
                .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, (_, b)| {
                    [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[2].max(b[2]),
                        a[3].max(b[3]),
                    ]
                });
            lines.push(OcrLine {
                text: cur.iter().map(|x| x.0).collect(),
                confidence: 1.0,
                poly: [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]],
                char_boxes: cur.iter().map(|x| x.1).collect(),
            });
            cur.clear();
        };
        for (c, b) in chars {
            let new_line = cur.last().is_some_and(|(_, p)| {
                let h = (p[3] - p[1]).max(4.0);
                ((p[1] + p[3]) / 2.0 - (b[1] + b[3]) / 2.0).abs() > 0.5 * h
                    || b[0] < p[0] - h
                    || b[0] > p[2] + 3.0 * h
            });
            if new_line {
                flush(&mut cur, &mut lines);
            }
            if cur.is_empty() && c.is_whitespace() {
                continue;
            }
            cur.push((c, b));
        }
        flush(&mut cur, &mut lines);
        let (w, h) = img.dimensions();
        out.push(PdfPageData {
            image: img,
            text: OcrPage {
                width: w,
                height: h,
                lines,
            },
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use krilla::action::{Action, LinkAction};
    use krilla::annotation::{Annotation, LinkAnnotation, Target};
    use krilla::destination::XyzDestination;
    use krilla::embed::{AssociationKind, EmbeddedFile};
    use krilla::geom::{Point, Rect};
    use krilla::metadata::Metadata;
    use krilla::outline::{Outline, OutlineNode};

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// 문서 정보·주석·책갈피·첨부를 모두 넣은 원본 PDF (합성, 가짜 값)
    fn rich_pdf() -> Vec<u8> {
        let mut doc = Document::new();
        doc.set_metadata(
            Metadata::new()
                .title("SECRET-TITLE-홍길동".into())
                .authors(vec!["SECRET-AUTHOR".into()]),
        );
        let mut ol = Outline::new();
        ol.push_child(OutlineNode::new(
            "SECRET-BOOKMARK".into(),
            XyzDestination::new(0, Point::from_xy(0.0, 0.0)),
        ));
        doc.set_outline(ol);
        doc.embed_file(EmbeddedFile {
            path: "SECRET-ATTACH.txt".into(),
            mime_type: None,
            description: None,
            association_kind: AssociationKind::Supplement,
            data: b"900101-1234568".to_vec().into(),
            modification_date: None,
            compress: Some(false),
            location: None,
        });
        let mut page = doc.start_page_with(PageSettings::from_wh(200.0, 200.0).unwrap());
        page.add_annotation(Annotation::new_link(
            LinkAnnotation::new(
                Rect::from_xywh(10.0, 10.0, 50.0, 20.0).unwrap(),
                Target::Action(Action::Link(LinkAction::new(
                    "https://secret.example".into(),
                ))),
            ),
            None,
        ));
        page.finish();
        doc.finish().unwrap()
    }

    #[test]
    fn masked_pdf_drops_metadata_annotations_outline_attachments_and_text() {
        let pdfium = bind_pdfium(&root()).unwrap();
        let src = rich_pdf();
        let s = String::from_utf8_lossy(&src);
        assert!(s.contains("/Annots") && s.contains("/Outlines") && s.contains("/EmbeddedFiles"));
        // 마스킹 경로: 래스터 → 새 이미지 PDF
        let pages = read_pdf(&pdfium, &src, 72.0).unwrap();
        let out = image_pdf(
            &pages.into_iter().map(|p| p.image).collect::<Vec<_>>(),
            72.0,
        )
        .unwrap();
        let o = String::from_utf8_lossy(&out);
        for key in [
            "/Annots",
            "/Outlines",
            "/EmbeddedFiles",
            "/AcroForm",
            "/Metadata",
            "SECRET",
            "/Title",
            "/Author",
        ] {
            assert!(!o.contains(key), "결과 PDF에 {key} 남음");
        }
        let doc = pdfium.load_pdf_from_byte_slice(&out, None).unwrap();
        assert_eq!(doc.attachments().len(), 0);
        for page in doc.pages().iter() {
            assert_eq!(page.annotations().len(), 0);
            assert!(
                page.text().unwrap().all().trim().is_empty(),
                "텍스트 레이어 남음"
            );
        }
    }

    #[test]
    fn detections_map_to_char_boxes_per_line() {
        let line = |t: &str, y: f32| {
            let n = t.chars().count();
            OcrLine {
                text: t.into(),
                confidence: 1.0,
                poly: [
                    [0.0, y],
                    [10.0 * n as f32, y],
                    [10.0 * n as f32, y + 10.0],
                    [0.0, y + 10.0],
                ],
                char_boxes: (0..n)
                    .map(|i| [10.0 * i as f32, y, 10.0 * (i + 1) as f32, y + 10.0])
                    .collect(),
            }
        };
        let page = OcrPage {
            width: 100,
            height: 100,
            lines: vec![line("가나다", 0.0), line("라마", 30.0)],
        };
        let pt = page_text(&page);
        assert_eq!(pt.text, "가나다\n라마");
        let d = Detection {
            label: pii_core::Label::Name,
            start: pii_core::CharOffset(1),
            end: pii_core::CharOffset(6),
            detector: pii_core::Detector::Rule,
        };
        let r = detection_rects(&pt, &[d]);
        assert_eq!(r.len(), 2);
    }
}
