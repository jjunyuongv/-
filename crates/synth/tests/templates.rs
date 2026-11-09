//! `assets/templates/*.toml` 검증 + 원본 PDF 추출 텍스트와 고정 문구 대조 (PLAN.md 1-2).
//!
//! 대조는 문자(한글·영문)만 남겨 비교한다 (공백·문장부호·숫자·빈칸 밑줄은 무시):
//! - 정방향: 추출한 쪽 텍스트의 **각 줄**이 템플릿 고정 문구(읽기 순서로 이어 붙인 것) 안에 있어야 함 → 빠진 문구 검출
//! - 역방향: 템플릿의 **각 조각**(문단·표 셀·서명란 줄)이 추출 텍스트 안에 있어야 함 → 오타·지어낸 문구 검출
//!
//! 추출 결과(`assets/forms/extracted/`)는 git에 넣지 않는 생성물이라, 없으면 대조만 건너뛴다
//! (`cargo run -p pii-mask -- forms extract`로 만든 뒤 다시 실행).

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use synth::ratios::RatioConfig;
use synth::template::{KNOWN_GENERATORS, Layout, Template, strip_placeholders};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_all() -> Vec<Template> {
    let dir = workspace_root().join("assets/templates");
    match Template::load_dir(&dir) {
        Ok(t) => t,
        Err(e) => panic!("{e}"),
    }
}

fn letters(s: &str) -> String {
    s.chars().filter(|c| c.is_alphabetic()).collect()
}

/// `needle`의 글자가 `hay` 안에 순서대로 (띄엄띄엄이라도) 모두 나오는지
fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut hay = hay.chars();
    needle.chars().all(|c| hay.any(|h| h == c))
}

#[test]
fn all_templates_are_valid_and_cover_documents_and_layouts() {
    let templates = load_all();
    let ids: BTreeSet<&str> = templates.iter().map(|t| t.meta.id.as_str()).collect();
    let expected: BTreeSet<&str> = [
        "standard_indefinite",
        "standard_fixed_term",
        "minor",
        "guardian_consent",
        "construction_daily",
        "part_time",
        "foreign_worker",
        "foreign_worker_agri",
    ]
    .into_iter()
    .collect();
    assert_eq!(ids, expected, "문서 8개 템플릿");

    let layouts: BTreeSet<Layout> = templates
        .iter()
        .flat_map(|t| t.meta.layouts.iter().copied())
        .collect();
    assert_eq!(layouts.len(), 6, "레이아웃 A~F 모두: {layouts:?}");
}

#[test]
fn ratio_references_match_ratio_config() {
    let path = workspace_root().join("assets/synth/ratios.toml");
    let config = match RatioConfig::load(&path) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    let templates = load_all();

    let mut problems = Vec::new();
    for t in &templates {
        problems.extend(
            t.check_ratios(&config)
                .into_iter()
                .map(|m| format!("{}: {m}", t.meta.id)),
        );
    }
    // used_by가 실제 쓰는 곳을 가리키는지 (template:<슬롯 이름> / generator:<생성기>)
    for (name, table) in &config.ratio {
        for target in &table.used_by {
            let ok = match target.split_once(':') {
                Some(("template", slot)) => templates.iter().any(|t| {
                    t.slots.get(slot).is_some_and(|s| {
                        s.choices.iter().any(|c| {
                            c.ratio
                                .as_deref()
                                .is_some_and(|r| r.starts_with(&format!("{name}.")))
                        })
                    })
                }),
                Some(("generator", g)) => KNOWN_GENERATORS.contains(&g),
                // 문서 조립 단계의 결정 (synth::doc에서 이름으로 꺼내 씀)
                Some(("doc", d)) => !d.is_empty(),
                _ => false,
            };
            if !ok {
                problems.push(format!(
                    "ratios.toml `{name}`: used_by `{target}`를 찾을 수 없음"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn layout_d_bundle_shares_slot_names() {
    // 연소근로자 계약서 + 친권자 동의서는 한 건으로 묶이므로 같은 사람·회사 값을 같은 슬롯 이름으로 공유
    let templates = load_all();
    let get = |id: &str| {
        templates
            .iter()
            .find(|t| t.meta.id == id)
            .unwrap_or_else(|| panic!("{id} 없음"))
    };
    let minor = get("minor");
    let consent = get("guardian_consent");
    for shared in [
        "worker_name",
        "worker_address",
        "worker_phone",
        "employer_company",
        "employer_address",
        "employer_rep",
        "employer_phone",
    ] {
        assert!(minor.slots.contains_key(shared), "minor에 {shared}");
        assert!(
            consent.slots.contains_key(shared),
            "guardian_consent에 {shared}"
        );
    }
}

#[test]
fn fixed_text_matches_extracted_pdf_text() {
    let pages_dir = workspace_root().join("assets/forms/extracted/pages");
    if !pages_dir.is_dir() {
        eprintln!(
            "건너뜀: {} 없음 — `cargo run -p pii-mask -- forms extract` 후 다시 실행",
            pages_dir.display()
        );
        return;
    }

    let mut failures = Vec::new();
    for t in load_all() {
        let mut extracted_lines = Vec::new();
        for page in &t.meta.source_pages {
            let path = pages_dir.join(format!("page_{page:02}.txt"));
            match fs::read_to_string(&path) {
                Ok(text) => extracted_lines.extend(text.lines().map(str::to_string)),
                Err(e) => {
                    failures.push(format!("{}: {} 읽기 실패: {e}", t.meta.id, path.display()))
                }
            }
        }
        let extracted_all: String = extracted_lines.iter().map(|l| letters(l)).collect();

        // 정방향은 모든 조각과 비교 (곡선 제목에서 텍스트로 남은 괄호·`ㆍ`도 템플릿 제목 안에 있음),
        // 역방향은 원본에 텍스트가 없는 조각(meta.not_in_pdf)을 뺀다
        let all_segments = t.text_segments();
        let template_all: String = all_segments
            .iter()
            .map(|s| letters(&strip_placeholders(s)))
            .collect();
        let segments: Vec<&str> = all_segments
            .into_iter()
            .filter(|s| !t.meta.not_in_pdf.iter().any(|n| n == s))
            .collect();

        for line in &extracted_lines {
            let l = letters(line);
            // 곡선 제목에서 텍스트로 남은 조각(예: `( ㆍ ㆍ )`)은 제목 글자의 부분 수열
            let remnant_of_curved_text = t
                .meta
                .not_in_pdf
                .iter()
                .any(|n| is_subsequence(&l, &letters(n)));
            if !l.is_empty() && !template_all.contains(&l) && !remnant_of_curved_text {
                failures.push(format!("{}: 템플릿에 없는 추출 줄: {line:?}", t.meta.id));
            }
        }
        for seg in &segments {
            let s = letters(&strip_placeholders(seg));
            if !s.is_empty() && !extracted_all.contains(&s) {
                failures.push(format!("{}: 원본에 없는 템플릿 조각: {seg:?}", t.meta.id));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{}건:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
