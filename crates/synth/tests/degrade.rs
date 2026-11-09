//! 열화(PLAN.md 5단계) 검증 + 체크포인트 샘플
//!
//! - 같은 기하 변환이 박스에도 적용되는지: 검은 사각형을 그린 쪽을 열화한 뒤, 옮긴 박스 안이 밖보다 어두운지
//! - 같은 시드 → 같은 바이트
//! - 체크포인트: `cargo test --release -p synth --test degrade -- --ignored` → `results/checkpoint5/`

use std::path::PathBuf;

use pii_core::{BoxLabel, DocLabel, Label};
use synth::degrade::{Difficulty, PageImage, degrade_page};
use synth::doc::{DocRequest, SynthConfig, generate};
use synth::fake::seeded;
use synth::render::{Fonts, overlay, png, render};
use synth::template::Layout;
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn setup() -> (SynthConfig, Fonts) {
    let cfg = SynthConfig::load(&root()).unwrap_or_else(|e| panic!("{e}"));
    let fonts = Fonts::load(&root().join("assets/fonts")).unwrap_or_else(|e| panic!("{e}"));
    (cfg, fonts)
}

/// 흰 쪽에 검은 사각형 하나 + 그 박스 라벨
fn square_page() -> (Pixmap, DocLabel) {
    let (w, h) = (400u32, 560u32);
    let Some(mut pix) = Pixmap::new(w, h) else {
        panic!("pixmap")
    };
    pix.fill(Color::WHITE);
    let r = [150.0f32, 220.0, 260.0, 300.0];
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 0, 0, 255);
    pix.fill_rect(
        Rect::from_ltrb(r[0], r[1], r[2], r[3]).unwrap_or_else(|| panic!("rect")),
        &paint,
        Transform::identity(),
        None,
    );
    let label = DocLabel {
        doc_id: "t".into(),
        form: "t".into(),
        layout: "A".into(),
        variant: "standard".into(),
        employer_kind: "individual".into(),
        industry: "제조".into(),
        difficulty: "clean".into(),
        handwriting_doc: false,
        seed: 0,
        pages: 1,
        text: String::new(),
        spans: vec![],
        attributes: vec![],
        negatives: vec![],
        boxes: vec![BoxLabel {
            span_id: Some(1),
            attribute_id: None,
            negative_id: None,
            label: Some(Label::Name),
            owner: None,
            kind: None,
            page: 0,
            poly: [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]],
        }],
    };
    (pix, label)
}

fn luma_at(img: &PageImage, x: f32, y: f32) -> Option<f32> {
    let (w, h) = img.size();
    if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
        return None;
    }
    Some(match img {
        PageImage::Jpeg(i, _) => {
            let p = i.get_pixel(x as u32, y as u32).0;
            0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])
        }
        PageImage::Png(g) => f32::from(g.get_pixel(x as u32, y as u32).0[0]),
    })
}

#[test]
fn boxes_follow_the_same_geometry() {
    let (cfg, fonts) = setup();
    for level in [
        Difficulty::D1,
        Difficulty::D2,
        Difficulty::D3,
        Difficulty::D4,
    ] {
        for seed in 0..4u64 {
            let (pix, mut label) = square_page();
            let img = degrade_page(&cfg, &fonts, &pix, 0, &mut label, level, &mut seeded(seed))
                .unwrap_or_else(|e| panic!("{e}"));
            let poly = label.boxes[0].poly;
            // 옮긴 사각형 안쪽(가장자리에서 안으로 20%)은 어둡고, 바깥 고리(밖으로 15px)는 밝아야 함
            let c = poly
                .iter()
                .fold([0.0, 0.0], |a, p| [a[0] + p[0] / 4.0, a[1] + p[1] / 4.0]);
            let mut inside = Vec::new();
            let mut outside = Vec::new();
            for p in &poly {
                let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
                let len = (dx * dx + dy * dy).sqrt();
                inside.extend(luma_at(&img, c[0] + dx * 0.8, c[1] + dy * 0.8));
                outside.extend(luma_at(
                    &img,
                    p[0] + dx / len * 15.0,
                    p[1] + dy / len * 15.0,
                ));
            }
            inside.extend(luma_at(&img, c[0], c[1]));
            let max_in = inside.iter().copied().fold(0.0, f32::max);
            let min_out = outside.iter().copied().fold(255.0, f32::min);
            assert!(
                max_in + 60.0 < min_out,
                "{level:?} seed {seed}: 안 {inside:?} / 밖 {outside:?}"
            );
        }
    }
}

#[test]
fn same_seed_same_bytes() {
    let (cfg, fonts) = setup();
    for level in [Difficulty::D2, Difficulty::D3, Difficulty::D4] {
        let run = |seed| {
            let (pix, mut label) = square_page();
            let img =
                degrade_page(&cfg, &fonts, &pix, 0, &mut label, level, &mut seeded(seed)).unwrap();
            (img.encode().unwrap(), label.boxes[0].poly)
        };
        assert_eq!(run(5), run(5), "{level:?}");
        assert_ne!(run(5).0, run(6).0, "{level:?}");
    }
}

/// 체크포인트: 난이도별 1쪽씩 (열화 이미지 + 옮긴 박스 오버레이)
#[test]
#[ignore = "확인용 출력 (--release 권장)"]
fn write_checkpoint_samples() {
    let (cfg, fonts) = setup();
    let out = root().join("results/checkpoint5");
    std::fs::create_dir_all(&out).unwrap();
    // 레이아웃도 섞는다: D1 A, D2 E(손글씨일 수 있음), D3 D(연소+친권자), D4 C
    let picks: [(Difficulty, Layout, &[&str], u64); 4] = [
        (Difficulty::D1, Layout::A, &["standard_fixed_term"], 51),
        (Difficulty::D2, Layout::E, &["part_time"], 52),
        (
            Difficulty::D3,
            Layout::D,
            &["minor", "guardian_consent"],
            53,
        ),
        (Difficulty::D4, Layout::C, &["foreign_worker"], 54),
    ];
    for (level, layout, forms, seed) in picks {
        let req = DocRequest {
            doc_id: format!("{}_{layout:?}_{seed:04}", level.name()),
            layout,
            forms: forms.iter().map(|s| s.to_string()).collect(),
            seed,
            digital: false,
            extra: Default::default(),
        };
        let doc = generate(&cfg, &req).unwrap();
        let r = render(&doc, &fonts).unwrap();
        let mut label = r.label.clone();
        label.difficulty = level.name().into();
        let mut rng = seeded(seed ^ 0xD15C);
        // 첫 쪽만 (체크포인트용)
        let img = degrade_page(&cfg, &fonts, &r.pages[0], 0, &mut label, level, &mut rng).unwrap();
        let bytes = img.encode().unwrap();
        let id = &req.doc_id;
        std::fs::write(out.join(format!("{id}_p1.{}", img.ext())), &bytes).unwrap();
        // 저장한 파일을 다시 읽어 그 위에 박스 (JPEG 손실까지 반영된 모습)
        let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
        let (w, h) = (decoded.width(), decoded.height());
        let Some(mut pix) = Pixmap::new(w, h) else {
            panic!("pixmap")
        };
        pix.data_mut().copy_from_slice(decoded.as_raw());
        std::fs::write(
            out.join(format!("{id}_p1_overlay.png")),
            png(&overlay(&pix, &label, 0)).unwrap(),
        )
        .unwrap();
        std::fs::write(
            out.join(format!("{id}.json")),
            serde_json::to_string_pretty(&label).unwrap(),
        )
        .unwrap();
        println!("{id}: {w}x{h} {}", img.ext());
    }
}
