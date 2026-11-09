//! 데이터 세트 계획(PLAN.md 5단계) 검증
//!
//! - 이미지 세트: 난이도·레이아웃 배분이 건수로 정확, 같은 시드 같은 계획
//! - 텍스트 세트: 정형 라벨별 span 최소 500개 (1,000건, 시드 43 — 실제 생성과 같은 설정)

use std::collections::BTreeMap;
use std::path::PathBuf;

use pii_core::Label;
use synth::dataset::{TEXT_TARGET, TEXT_TARGET_LABELS, generate_text_set, plan_image_set};
use synth::degrade::Difficulty;
use synth::doc::SynthConfig;
use synth::template::Layout;

fn cfg() -> SynthConfig {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    SynthConfig::load(&root).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn image_set_allocation_is_exact() {
    let cfg = cfg();
    for (n, seed) in [(20usize, 41u64), (100, 42)] {
        let plan = plan_image_set(&cfg, n, seed).unwrap();
        assert_eq!(plan.len(), n);
        let mut by_diff: BTreeMap<Difficulty, usize> = BTreeMap::new();
        let mut by_layout: BTreeMap<String, usize> = BTreeMap::new();
        for p in &plan {
            *by_diff.entry(p.difficulty).or_default() += 1;
            *by_layout.entry(format!("{:?}", p.layout)).or_default() += 1;
        }
        assert!(by_diff.values().all(|c| *c == n / 5), "{by_diff:?}");
        let expect = |share: f64| (share * n as f64).round() as usize;
        assert_eq!(by_layout["A"], expect(0.3), "{by_layout:?}");
        assert_eq!(by_layout["B"], expect(0.2), "{by_layout:?}");
        assert_eq!(
            by_layout["D"],
            expect(0.15).max(by_layout["D"].min(expect(0.15) + 1))
        );
        assert_eq!(
            plan.iter().filter(|p| p.layout == Layout::F).count(),
            expect(0.1)
        );
        // 같은 시드 같은 계획, 다른 시드는 문서 시드가 겹치지 않음
        let again = plan_image_set(&cfg, n, seed).unwrap();
        assert!(
            plan.iter()
                .zip(&again)
                .all(|(a, b)| a.seed == b.seed && a.doc_id == b.doc_id)
        );
    }
    let dev = plan_image_set(&cfg, 20, 41).unwrap();
    let eval = plan_image_set(&cfg, 100, 42).unwrap();
    assert!(dev.iter().all(|d| eval.iter().all(|e| e.seed != d.seed)));
}

#[test]
fn text_set_reaches_structured_label_targets() {
    let cfg = cfg();
    let set = generate_text_set(&cfg, 1000, 43).unwrap();
    let mut spans: BTreeMap<Label, u32> = BTreeMap::new();
    for l in &set.labels {
        assert!(l.offset_problems().is_empty(), "{}", l.doc_id);
        for s in &l.spans {
            *spans.entry(s.label).or_default() += 1;
        }
    }
    let report: Vec<String> = TEXT_TARGET_LABELS
        .iter()
        .map(|l| format!("{l:?} {}", spans.get(l).copied().unwrap_or(0)))
        .collect();
    println!("{}", report.join(", "));
    for l in TEXT_TARGET_LABELS {
        let n = spans.get(&l).copied().unwrap_or(0);
        assert!(n >= TEXT_TARGET, "{l:?}: {n} < {TEXT_TARGET} ({report:?})");
    }
}
