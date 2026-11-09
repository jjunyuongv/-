//! D0 텍스트 레이어 PDF 검증: 만든 PDF를 pdfium으로 다시 읽어 정답 라벨과 대조
//!
//! - 쪽 수 = 라벨 쪽 수, 손글씨 span 없음 (D0는 인쇄체만)
//! - PDF 텍스트(스트림 순서, 공백 제외) = 라벨 텍스트(공백 제외) → 텍스트 레이어에 빠진·더한 글자 없음
//! - span마다: 그 박스(픽셀 → pt) 안에 중심이 있는 PDF 글자 = span 값 (공백 제외) → 박스와 텍스트 레이어 위치 일치
//! - 확인용 샘플: `cargo test -p synth --test pdf -- --ignored` → `results/d0_sample/`
//!
//! pdfium DLL 필요: `PDFIUM_DYNAMIC_LIB_PATH` 또는 `vendor/pdfium/bin/pdfium.dll` (CLAUDE.md)

use std::path::PathBuf;

use pdfium_render::prelude::*;
use pii_core::DocLabel;
use synth::doc::{DocRequest, SynthConfig, generate};
use synth::render::pdf::{PT_PER_PX, write_pdf};
use synth::render::{Fonts, layout, overlay, png};
use synth::template::Layout;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn pdfium() -> Pdfium {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::var_os("PDFIUM_DYNAMIC_LIB_PATH") {
        dirs.push(PathBuf::from(d));
    }
    dirs.push(root().join("vendor/pdfium/bin"));
    for d in dirs {
        let lib = Pdfium::pdfium_platform_library_name_at_path(&d);
        if let Ok(b) = Pdfium::bind_to_library(&lib) {
            return Pdfium::new(b);
        }
    }
    panic!("pdfium DLL이 없습니다 (vendor/pdfium/bin/pdfium.dll 또는 PDFIUM_DYNAMIC_LIB_PATH)");
}

fn cases() -> Vec<(Layout, Vec<&'static str>)> {
    vec![
        (Layout::A, vec!["standard_indefinite"]),
        (Layout::B, vec!["standard_fixed_term"]),
        (Layout::C, vec!["foreign_worker"]),
        (Layout::C, vec!["foreign_worker_agri"]),
        (Layout::D, vec!["minor", "guardian_consent"]),
        (Layout::E, vec!["part_time"]),
        (Layout::F, vec!["construction_daily"]),
    ]
}

fn req(layout: Layout, forms: &[&str], seed: u64) -> DocRequest {
    DocRequest {
        doc_id: format!("D0_{layout:?}_{seed:04}"),
        layout,
        forms: forms.iter().map(|s| s.to_string()).collect(),
        seed,
        digital: true,
        extra: Default::default(),
    }
}

/// PDF 글자 하나: 문자 + 픽셀 좌표 중심
struct PdfChar {
    c: char,
    page: u32,
    cx: f32,
    cy: f32,
}

fn pdf_chars(pdf: &PdfDocument) -> Vec<PdfChar> {
    let mut out = Vec::new();
    for (pi, page) in pdf.pages().iter().enumerate() {
        let h = page.height().value;
        let text = page.text().unwrap_or_else(|e| panic!("{e}"));
        for ch in text.chars().iter() {
            let Some(c) = ch.unicode_char() else { continue };
            if c.is_whitespace() || ch.is_generated().unwrap_or(false) {
                continue;
            }
            let r = ch.loose_bounds().unwrap_or_else(|e| panic!("{e}"));
            out.push(PdfChar {
                c,
                page: pi as u32,
                cx: (r.left().value + r.right().value) / 2.0 / PT_PER_PX,
                cy: (h - (r.top().value + r.bottom().value) / 2.0) / PT_PER_PX,
            });
        }
    }
    out
}

fn no_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn check(pdfium: &Pdfium, label: &DocLabel, bytes: &[u8]) -> Vec<String> {
    let mut problems = Vec::new();
    let pdf = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .unwrap_or_else(|e| panic!("{e}"));
    if pdf.pages().len() as u32 != label.pages {
        problems.push(format!(
            "쪽 수 {} ≠ 라벨 {}",
            pdf.pages().len(),
            label.pages
        ));
    }
    if label.spans.iter().any(|s| s.hand) {
        problems.push("D0에 손글씨 span".into());
    }
    let chars = pdf_chars(&pdf);
    let stream: String = chars.iter().map(|c| c.c).collect();
    if stream != no_ws(&label.text) {
        let a: Vec<char> = stream.chars().collect();
        let b: Vec<char> = no_ws(&label.text).chars().collect();
        let i = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let ctx = |v: &[char]| {
            v[i.saturating_sub(10)..(i + 10).min(v.len())]
                .iter()
                .collect::<String>()
        };
        problems.push(format!(
            "텍스트 레이어 ≠ 라벨 텍스트 ({i}번째 글자부터: PDF `{}` / 라벨 `{}`)",
            ctx(&a),
            ctx(&b)
        ));
    }
    for s in label.spans.iter().filter(|s| !s.label.is_image_only()) {
        let mut got = String::new();
        for b in label.boxes.iter().filter(|b| b.span_id == Some(s.id)) {
            let xs = b.poly.iter().map(|p| p[0]);
            let ys = b.poly.iter().map(|p| p[1]);
            let (x0, x1) = (
                xs.clone().fold(f32::MAX, f32::min),
                xs.fold(f32::MIN, f32::max),
            );
            let (y0, y1) = (
                ys.clone().fold(f32::MAX, f32::min),
                ys.fold(f32::MIN, f32::max),
            );
            got.extend(
                chars
                    .iter()
                    .filter(|c| {
                        c.page == b.page && c.cx > x0 && c.cx < x1 && c.cy > y0 && c.cy < y1
                    })
                    .map(|c| c.c),
            );
        }
        if got != no_ws(&s.value) {
            // 값은 원문 글자 그대로 남기지 않는다 (라벨·id·글자 수만)
            problems.push(format!(
                "span {} {:?}: 박스 안 PDF 글자 {}개 ≠ 값 {}개",
                s.id,
                s.label,
                got.chars().count(),
                no_ws(&s.value).chars().count()
            ));
        }
    }
    problems
}

#[test]
fn d0_pdf_text_layer_matches_labels() {
    let cfg = SynthConfig::load(&root()).unwrap_or_else(|e| panic!("{e}"));
    let fonts = Fonts::load(&root().join("assets/fonts")).unwrap_or_else(|e| panic!("{e}"));
    let pdfium = pdfium();
    let mut problems = Vec::new();
    for (layout_kind, forms) in cases() {
        for seed in 0..6u64 {
            let doc = generate(&cfg, &req(layout_kind, &forms, seed)).unwrap();
            let laid = layout(&doc, &fonts);
            let bytes = write_pdf(&laid, &fonts).unwrap();
            for p in check(&pdfium, &laid.label, &bytes) {
                problems.push(format!("{}: {p}", laid.label.doc_id));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// 확인용 샘플: D0 PDF + 같은 배치의 이미지·오버레이 (`results/d0_sample/`)
#[test]
#[ignore = "확인용 출력"]
fn write_d0_samples() {
    let cfg = SynthConfig::load(&root()).unwrap_or_else(|e| panic!("{e}"));
    let fonts = Fonts::load(&root().join("assets/fonts")).unwrap_or_else(|e| panic!("{e}"));
    let pdfium = pdfium();
    let out = root().join("results/d0_sample");
    std::fs::create_dir_all(&out).unwrap();
    for (layout_kind, forms, seed) in [
        (Layout::C, vec!["foreign_worker"], 43u64),
        (Layout::D, vec!["minor", "guardian_consent"], 49),
    ] {
        let doc = generate(&cfg, &req(layout_kind, &forms, seed)).unwrap();
        let laid = layout(&doc, &fonts);
        let bytes = write_pdf(&laid, &fonts).unwrap();
        let id = &laid.label.doc_id;
        std::fs::write(out.join(format!("{id}.pdf")), &bytes).unwrap();
        std::fs::write(
            out.join(format!("{id}.json")),
            serde_json::to_string_pretty(&laid.label).unwrap(),
        )
        .unwrap();
        // PDF를 pdfium으로 래스터화한 쪽 위에 라벨 박스 → 텍스트 레이어와 박스가 맞는지 눈으로 확인
        let pdf = pdfium.load_pdf_from_byte_slice(&bytes, None).unwrap();
        for (i, page) in pdf.pages().iter().enumerate() {
            let img = page
                .render_with_config(&PdfRenderConfig::new().set_target_width(1654))
                .unwrap()
                .as_image()
                .unwrap()
                .into_rgba8();
            let (w, h) = (img.width(), img.height());
            let mut pix = tiny_skia::Pixmap::new(w, h).unwrap();
            pix.data_mut().copy_from_slice(img.as_raw());
            let ov = overlay(&pix, &laid.label, i as u32);
            std::fs::write(
                out.join(format!("{id}_p{}_pdf_overlay.png", i + 1)),
                png(&ov).unwrap(),
            )
            .unwrap();
        }
    }
}
