//! 렌더링(PLAN.md 4단계) 검증 + 체크포인트 3건 생성
//!
//! - 모든 span·참고 항목·네거티브가 박스를 1개 이상 가짐, 박스는 페이지 안, 페이지 수 일치
//! - 서명란 표시(서명·도장·지장) 박스는 owner·kind를 가짐
//! - 체크포인트: `cargo test -p synth --test render -- --ignored` → `results/checkpoint4/`

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use synth::doc::{DocRequest, SynthConfig, generate};
use synth::render::{Fonts, PAGE_H, PAGE_W, overlay, png, render};
use synth::template::Layout;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn setup() -> (SynthConfig, Fonts) {
    let cfg = SynthConfig::load(&root()).unwrap_or_else(|e| panic!("{e}"));
    let fonts = Fonts::load(&root().join("assets/fonts")).unwrap_or_else(|e| panic!("{e}"));
    (cfg, fonts)
}

fn req(layout: Layout, forms: &[&str], seed: u64) -> DocRequest {
    DocRequest {
        doc_id: format!("{layout:?}_{seed:04}"),
        layout,
        forms: forms.iter().map(|s| s.to_string()).collect(),
        seed,
        digital: false,
        extra: Default::default(),
    }
}

#[test]
fn every_field_gets_boxes_inside_the_page() {
    let (cfg, fonts) = setup();
    let cases: Vec<(Layout, Vec<&str>)> = vec![
        (Layout::A, vec!["standard_fixed_term"]),
        (Layout::B, vec!["standard_indefinite"]),
        (Layout::C, vec!["foreign_worker_agri"]),
        (Layout::D, vec!["minor", "guardian_consent"]),
        (Layout::E, vec!["part_time"]),
        (Layout::F, vec!["construction_daily"]),
    ];
    let mut problems = Vec::new();
    for (layout, forms) in cases {
        for seed in [1u64, 2] {
            let doc = generate(&cfg, &req(layout, &forms, seed)).unwrap_or_else(|e| panic!("{e}"));
            let r = render(&doc, &fonts).unwrap_or_else(|e| panic!("{e}"));
            let l = &r.label;
            assert_eq!(l.pages as usize, r.pages.len());
            problems.extend(l.offset_problems());
            let span_ids: BTreeSet<u32> = l.boxes.iter().filter_map(|b| b.span_id).collect();
            for s in &l.spans {
                if !span_ids.contains(&s.id) {
                    problems.push(format!(
                        "{}: span {} {:?} 박스 없음",
                        l.doc_id, s.id, s.label
                    ));
                }
            }
            let attr_ids: BTreeSet<u32> = l.boxes.iter().filter_map(|b| b.attribute_id).collect();
            for a in &l.attributes {
                if !attr_ids.contains(&a.id) {
                    problems.push(format!("{}: 참고 항목 {} 박스 없음", l.doc_id, a.id));
                }
            }
            for b in &l.boxes {
                for [x, y] in b.poly {
                    if !(-40.0..=PAGE_W as f32 + 40.0).contains(&x)
                        || !(-40.0..=PAGE_H as f32 + 40.0).contains(&y)
                    {
                        problems.push(format!("{}: 박스가 페이지 밖 {:?}", l.doc_id, b.poly));
                        break;
                    }
                }
                if b.kind.is_some() && b.owner.is_none() {
                    problems.push(format!("{}: 서명란 박스에 owner 없음", l.doc_id));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{}건:\n{}",
        problems.len(),
        problems[..problems.len().min(20)].join("\n")
    );
}

/// 체크포인트: 레이아웃이 다른 3건 (A, C 외국인, D 연소+친권자) — 페이지 PNG, 박스 오버레이 PNG, 라벨 JSON
#[test]
#[ignore = "체크포인트 결과물 생성"]
fn write_checkpoint_samples() {
    let (cfg, fonts) = setup();
    let out = root().join("results/checkpoint4");
    fs::create_dir_all(&out).unwrap();
    // D는 손글씨 문서가 나오는 첫 시드를 고른다 (손글씨 필드 박스 확인용)
    let d_seed = (44u64..)
        .find(|s| {
            generate(&cfg, &req(Layout::D, &["minor", "guardian_consent"], *s))
                .map(|d| d.handwriting_doc)
                .unwrap_or(false)
        })
        .unwrap();
    for (layout, forms, seed) in [
        (Layout::A, vec!["standard_indefinite"], 42u64),
        (Layout::C, vec!["foreign_worker"], 43),
        (Layout::D, vec!["minor", "guardian_consent"], d_seed),
    ] {
        let doc = generate(&cfg, &req(layout, &forms, seed)).unwrap();
        let r = render(&doc, &fonts).unwrap();
        let id = &r.label.doc_id;
        for (i, page) in r.pages.iter().enumerate() {
            fs::write(out.join(format!("{id}_p{}.png", i + 1)), png(page).unwrap()).unwrap();
            let ov = overlay(page, &r.label, i as u32);
            fs::write(
                out.join(format!("{id}_p{}_overlay.png", i + 1)),
                png(&ov).unwrap(),
            )
            .unwrap();
        }
        fs::write(
            out.join(format!("{id}.json")),
            serde_json::to_string_pretty(&r.label).unwrap(),
        )
        .unwrap();
        println!(
            "{id}: {}쪽, span {}, 참고 {}, 네거티브 {}, 박스 {}, 손글씨 문서 {}, 사업주 {}, 업종 {}",
            r.pages.len(),
            r.label.spans.len(),
            r.label.attributes.len(),
            r.label.negatives.len(),
            r.label.boxes.len(),
            r.label.handwriting_doc,
            r.label.employer_kind,
            r.label.industry
        );
    }
}

/// 5단계 이미지 장수 추정: 1-2 배분(A 30, B 20, C 15, D 15, E 10, F 10)대로 렌더링한 평균 쪽 수
/// (`cargo test -p synth --test render page_count_estimate -- --ignored --nocapture`)
#[test]
#[ignore = "확인용 출력"]
fn page_count_estimate() {
    let (cfg, fonts) = setup();
    // (레이아웃, 양식, 100건 중 건수)
    let plan: Vec<(Layout, Vec<&str>, f64)> = vec![
        (Layout::A, vec!["standard_indefinite"], 15.0),
        (Layout::A, vec!["standard_fixed_term"], 15.0),
        (Layout::B, vec!["standard_indefinite"], 10.0),
        (Layout::B, vec!["standard_fixed_term"], 10.0),
        (Layout::C, vec!["foreign_worker"], 10.0),
        (Layout::C, vec!["foreign_worker_agri"], 5.0),
        (Layout::D, vec!["minor", "guardian_consent"], 15.0),
        (Layout::E, vec!["part_time"], 10.0),
        (Layout::F, vec!["construction_daily"], 10.0),
    ];
    let n = 30u64;
    let mut total = 0.0;
    for (layout, forms, docs) in &plan {
        let mut hist = std::collections::BTreeMap::new();
        for seed in 1000..1000 + n {
            let doc = generate(&cfg, &req(*layout, forms, seed)).unwrap();
            let r = render(&doc, &fonts).unwrap();
            *hist.entry(r.pages.len()).or_insert(0u32) += 1;
        }
        let avg = hist
            .iter()
            .map(|(p, c)| *p as f64 * f64::from(*c))
            .sum::<f64>()
            / n as f64;
        total += avg * docs;
        println!("{layout:?} {forms:?}: 평균 {avg:.2}쪽 (쪽수별 {hist:?}) × {docs}건");
    }
    println!("100건 추정 이미지 장수: {total:.0}");
}
