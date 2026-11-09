//! `assets/clauses/*.jsonl` 문장 풀 검증 (PLAN.md 2단계).
//!
//! - 형식: id 유일·접두어, 허용된 자리표시만, 조사 표시는 자리표시 바로 뒤에만, 숫자 5자리 이상 금지(번호는 자리표시로)
//! - 개수: 일반 ~100, 민감 ~60(라벨별 10 이상), 하드네거티브 60~70(13자리 종류 필수)
//! - 트리거 없는 문장: trigger_free 문장 × trigger_free 값을 **모두 조합**해 채운 문장에 트리거 키워드가 없어야 함
//! - 모든 민감 문장 × 넣을 수 있는 값(topics·업무 제한 조건)이 조사까지 오류 없이 채워져야 함
//!
//! 전체 조합 검토 문서: `cargo test -p synth --test clauses -- --ignored` → `results/clauses_review_full.md`

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use pii_core::Label;
use synth::clauses::{
    ClauseError, DOC_SLOTS, GeneralClause, HARD_NEGATIVE_SLOTS, HardNegativeClause, INDUSTRIES,
    SENSITIVE_LABELS, SensitiveClause, SensitiveValue, check_placeholders, load_jsonl,
    load_keywords, render_text, stray_josa_markers, triggers_in,
};
use synth::template::KNOWN_GENERATORS;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn dir() -> PathBuf {
    root().join("assets/clauses")
}

fn load<T: serde::de::DeserializeOwned>(name: &str) -> Vec<T> {
    match load_jsonl(&dir().join(name)) {
        Ok(v) => v,
        Err(e) => panic!(
            "{e}: {:?}",
            std::error::Error::source(&e).map(ToString::to_string)
        ),
    }
}

fn keywords() -> Vec<String> {
    match load_keywords(&dir().join("slm_triggers_draft.txt")) {
        Ok(k) => k,
        Err(e) => panic!("{e}"),
    }
}

/// 문서 슬롯 자리표시에 넣을 시험값 (받침 있음/없음을 번갈아). 병원 업종이면 병원 이름
fn doc_value(name: &str, variant: usize, industry: &str) -> Option<String> {
    let v = match (name, variant % 2, industry) {
        ("WORKER_NAME", 0, _) => "김가람",
        ("WORKER_NAME", _, _) => "이하나",
        ("COMPANY", 0, "병원") => "늘해병원",
        ("COMPANY", _, "병원") => "새솔요양병원",
        ("COMPANY", 0, _) => "가람물산",
        ("COMPANY", _, _) => "하나테크",
        _ => return None,
    };
    Some(v.to_string())
}

/// 값의 업종 (조건이 없으면 일반 업종으로 채움)
fn industry_of(v: &SensitiveValue) -> &str {
    v.industry.as_deref().unwrap_or("제조")
}

/// 값 자체에 든 문서 자리표시(`{COMPANY} 노동조합`)를 채운다
fn render_value(v: &SensitiveValue, variant: usize) -> Result<String, ClauseError> {
    render_text(&v.value, |n| doc_value(n, variant, industry_of(v)))
}

/// 민감 문장 하나를 값 하나로 채운다
fn fill(c: &SensitiveClause, v: &SensitiveValue, variant: usize) -> Result<String, ClauseError> {
    let label = label_name(c.label);
    let value = render_value(v, variant)?;
    render_text(&c.template, |n| {
        if n == label {
            Some(value.clone())
        } else {
            doc_value(n, variant, industry_of(v))
        }
    })
}

fn label_name(l: Label) -> String {
    match serde_json::to_value(l) {
        Ok(serde_json::Value::String(s)) => s,
        _ => format!("{l:?}"),
    }
}

/// 공통 형식 검사: id 접두어·유일, 5자리 이상 숫자 금지, 잘못 놓인 조사 표시
fn check_common<'a>(
    file: &str,
    prefix: char,
    items: impl Iterator<Item = (&'a str, &'a str)>,
    p: &mut Vec<String>,
) {
    let mut seen = BTreeSet::new();
    for (id, text) in items {
        let ok_id =
            id.len() == 4 && id.starts_with(prefix) && id[1..].chars().all(|c| c.is_ascii_digit());
        if !ok_id {
            p.push(format!("{file} {id}: id는 `{prefix}000` 형식"));
        }
        if !seen.insert(id.to_string()) {
            p.push(format!("{file} {id}: id 중복"));
        }
        let mut run = 0;
        for c in text.chars() {
            run = if c.is_ascii_digit() { run + 1 } else { 0 };
            if run >= 5 {
                p.push(format!(
                    "{file} {id}: 5자리 이상 숫자는 자리표시로 (LLM이 번호를 쓰지 않음)"
                ));
                break;
            }
        }
        for m in stray_josa_markers(text) {
            p.push(format!(
                "{file} {id}: 조사 표시 `{m}`는 자리표시 바로 뒤에만"
            ));
        }
        if text.trim() != text || text.is_empty() {
            p.push(format!("{file} {id}: 앞뒤 공백 또는 빈 문장"));
        }
    }
}

#[test]
fn general_clauses() {
    let items: Vec<GeneralClause> = load("general.jsonl");
    let mut p = Vec::new();
    check_common(
        "general",
        'g',
        items.iter().map(|c| (c.id.as_str(), c.text.as_str())),
        &mut p,
    );
    let doc: Vec<&str> = DOC_SLOTS.iter().map(|(n, _)| *n).collect();
    for c in &items {
        check_placeholders(&c.text, &doc, &mut p, &c.id);
    }
    let n = items.len();
    let like = items.iter().filter(|c| c.sensitive_like).count();
    if !(90..=110).contains(&n) {
        p.push(format!("일반 특약 개수 {n} (목표 ~100)"));
    }
    if like < 10 {
        p.push(format!("sensitive_like 문장 {like}개 (10개 이상)"));
    }
    assert!(p.is_empty(), "{}", p.join("\n"));
}

#[test]
fn sensitive_clauses_and_values() {
    let clauses: Vec<SensitiveClause> = load("sensitive.jsonl");
    let values: Vec<SensitiveValue> = load("sensitive_values.jsonl");
    let kw = keywords();
    let mut p = Vec::new();

    check_common(
        "sensitive",
        's',
        clauses.iter().map(|c| (c.id.as_str(), c.template.as_str())),
        &mut p,
    );
    check_common(
        "values",
        'v',
        values.iter().map(|v| (v.id.as_str(), v.value.as_str())),
        &mut p,
    );

    let doc: Vec<&str> = DOC_SLOTS.iter().map(|(n, _)| *n).collect();
    let mut by_label: BTreeMap<Label, Vec<&SensitiveValue>> = BTreeMap::new();
    for v in &values {
        if !SENSITIVE_LABELS.contains(&v.label) {
            p.push(format!("{}: 민감정보 라벨이 아님 {:?}", v.id, v.label));
        }
        check_placeholders(&v.value, &doc, &mut p, &v.id);
        if let Some(i) = &v.industry
            && !INDUSTRIES.contains(&i.as_str())
        {
            p.push(format!("{}: 모르는 업종 `{i}`", v.id));
        }
        if v.trigger_free {
            match render_value(v, 0) {
                Ok(text) => {
                    let t = triggers_in(&text, &kw);
                    if !t.is_empty() {
                        p.push(format!("{}: trigger_free 값에 트리거 {t:?}", v.id));
                    }
                }
                Err(e) => p.push(format!("{}: {e}", v.id)),
            }
        }
        by_label.entry(v.label).or_default().push(v);
    }

    let mut combos = 0usize;
    for c in &clauses {
        let name = label_name(c.label);
        if !SENSITIVE_LABELS.contains(&c.label) {
            p.push(format!("{}: 민감정보 라벨이 아님 {:?}", c.id, c.label));
            continue;
        }
        let mut allowed = doc.clone();
        allowed.push(&name);
        let used = check_placeholders(&c.template, &allowed, &mut p, &c.id);
        if !used.contains(name.as_str()) {
            p.push(format!("{}: 자기 라벨 자리표시 `{{{name}}}`가 없음", c.id));
        }
        if c.template.matches(&format!("{{{name}}}")).count() > 1 {
            p.push(format!("{}: `{{{name}}}`는 한 번만", c.id));
        }
        if c.trigger_free {
            let t = triggers_in(&c.template, &kw);
            if !t.is_empty() {
                p.push(format!("{}: trigger_free 문장에 트리거 {t:?}", c.id));
            }
        }
        let pool = by_label
            .get(&c.label)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for t in &c.topics {
            if !pool.iter().any(|v| v.topic == *t) {
                p.push(format!("{}: 값 풀에 없는 topic `{t}`", c.id));
            }
        }
        if !c.topics.is_empty() && !c.values.is_empty() {
            p.push(format!("{}: topics와 values를 함께 쓰지 않음", c.id));
        }
        for id in &c.values {
            if !pool.iter().any(|v| v.id == *id) {
                p.push(format!("{}: 같은 라벨 값 풀에 없는 값 `{id}`", c.id));
            }
        }
        // 한 종교에만 맞는 문장(금요 합동 예배 → 이슬람교 등)이 있어 최소 1개
        let accepted: Vec<&&SensitiveValue> = pool.iter().filter(|v| c.accepts(v)).collect();
        if accepted.is_empty() {
            p.push(format!("{}: 넣을 수 있는 값이 없음", c.id));
        }
        for (i, v) in accepted.iter().enumerate() {
            if c.restricts_work && v.no_work_restriction {
                p.push(format!(
                    "{} × {}: 업무 제한 문장에 제한 금지 값",
                    c.id, v.id
                ));
            }
            match fill(c, v, i) {
                Ok(text) => {
                    combos += 1;
                    if c.trigger_free {
                        let t = triggers_in(&text, &kw);
                        if !t.is_empty() {
                            p.push(format!(
                                "{} × {}: 채운 문장에 트리거 {t:?}: {text}",
                                c.id, v.id
                            ));
                        }
                    }
                }
                Err(e) => p.push(format!("{} × {}: {e}", c.id, v.id)),
            }
        }
    }

    let n = clauses.len();
    if !(55..=65).contains(&n) {
        p.push(format!("민감정보 슬롯 문장 개수 {n} (목표 ~60)"));
    }
    for label in SENSITIVE_LABELS {
        let cs = clauses.iter().filter(|c| c.label == label).count();
        let tf = clauses
            .iter()
            .filter(|c| c.label == label && c.trigger_free)
            .count();
        let vs = by_label.get(&label).map_or(0, Vec::len);
        let vtf = by_label
            .get(&label)
            .map_or(0, |v| v.iter().filter(|v| v.trigger_free).count());
        if cs < 10 || tf < 2 || vs < 8 || vtf < 2 {
            p.push(format!(
                "{label:?}: 문장 {cs}(10+)·트리거 없는 문장 {tf}(2+)·값 {vs}(8+)·트리거 없는 값 {vtf}(2+)"
            ));
        }
    }
    eprintln!("민감 문장 × 값 조합 {combos}개 채움 확인");
    assert!(p.is_empty(), "{}건:\n{}", p.len(), p.join("\n"));
}

#[test]
fn hard_negative_clauses() {
    let items: Vec<HardNegativeClause> = load("hard_negatives.jsonl");
    let mut p = Vec::new();
    check_common(
        "hard_negatives",
        'n',
        items.iter().map(|c| (c.id.as_str(), c.template.as_str())),
        &mut p,
    );

    let mut allowed: Vec<&str> = HARD_NEGATIVE_SLOTS.iter().map(|(n, _)| *n).collect();
    allowed.extend(DOC_SLOTS.iter().map(|(n, _)| *n));
    for c in &items {
        let used = check_placeholders(&c.template, &allowed, &mut p, &c.id);
        match c.kind.required_slot() {
            Some(slot) if !used.contains(slot) => {
                p.push(format!("{}: {:?} 문장에 `{{{slot}}}`가 없음", c.id, c.kind))
            }
            None if !(c.template.contains('제') && c.template.contains('조')) => {
                p.push(format!("{}: 법 조항 문장에 `제○조`가 없음", c.id))
            }
            _ => {}
        }
    }
    for (slot, generator) in HARD_NEGATIVE_SLOTS {
        let base = generator.split(':').next().unwrap_or_default();
        if !KNOWN_GENERATORS.contains(&base) {
            p.push(format!(
                "`{slot}` 생성기 `{generator}`가 KNOWN_GENERATORS에 없음"
            ));
        }
    }

    let n = items.len();
    if !(60..=70).contains(&n) {
        p.push(format!("하드네거티브 개수 {n} (목표 60~70)"));
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for c in &items {
        *counts.entry(format!("{:?}", c.kind)).or_default() += 1;
    }
    for c in &items {
        if c.kind.is_13_digit() && counts[&format!("{:?}", c.kind)] < 4 {
            p.push(format!("13자리 종류 {:?}는 4개 이상", c.kind));
            break;
        }
    }
    let n13 = items.iter().filter(|c| c.kind.is_13_digit()).count();
    if n13 < 15 {
        p.push(format!("13자리 하드네거티브 {n13}개 (15개 이상)"));
    }
    eprintln!("하드네거티브 종류별: {counts:?}");
    assert!(p.is_empty(), "{}", p.join("\n"));
}

/// 사람 검토용: 민감 문장마다 넣을 수 있는 **모든** 값으로 채운 전체 조합 → `results/clauses_review_full.md`.
/// 실제 합성과 같은 `render_text`로 채운다 (조사 선택 포함).
#[test]
#[ignore = "검토 문서 생성 — cargo test -p synth --test clauses -- --ignored"]
fn write_full_review() {
    let clauses: Vec<SensitiveClause> = load("sensitive.jsonl");
    let values: Vec<SensitiveValue> = load("sensitive_values.jsonl");
    let negatives: Vec<HardNegativeClause> = load("hard_negatives.jsonl");

    let mut md = String::new();
    let mut total = 0usize;
    let _ = writeln!(md, "# 문장 풀 전체 조합 검토 (2단계)\n");
    let _ = writeln!(
        md,
        "> `cargo test -p synth --test clauses -- --ignored`로 생성. 실제 합성과 같은 Rust `render_text`로 채움.\n\
         > 굵은 글씨 = 채운 민감정보 값. 이름은 김가람/이하나, 회사는 가람물산/하나테크를 번갈아 넣음 (병원 업종 값은 늘해병원/새솔요양병원).\n\
         > 표시: `트리거 없음` = SLM 트리거 키워드가 없는 문장, `업무 제한` = HIV 감염 등 제한 금지 값을 넣지 않는 문장.\n"
    );

    for label in SENSITIVE_LABELS {
        let name = label_name(label);
        let _ = writeln!(md, "## {name}\n");
        for c in clauses.iter().filter(|c| c.label == label) {
            let mut flags = Vec::new();
            if c.trigger_free {
                flags.push("`트리거 없음`".to_string());
            }
            if c.restricts_work {
                flags.push("`업무 제한`".to_string());
            }
            if !c.topics.is_empty() {
                flags.push(format!("`허용: {}`", c.topics.join(", ")));
            }
            if !c.values.is_empty() {
                flags.push(format!("`허용 값: {}`", c.values.join(", ")));
            }
            let _ = writeln!(md, "### {} {}\n", c.id, flags.join(" "));
            let _ = writeln!(md, "`{}`\n", c.template);
            let accepted: Vec<&SensitiveValue> = values.iter().filter(|v| c.accepts(v)).collect();
            for (i, v) in accepted.iter().enumerate() {
                let value = render_value(v, i).unwrap_or_else(|e| format!("(오류: {e})"));
                let text = fill(c, v, i).unwrap_or_else(|e| format!("(오류: {e})"));
                let shown = text.replacen(&value, &format!("**{value}**"), 1);
                let industry = v
                    .industry
                    .as_deref()
                    .map(|i| format!(" (업종: {i})"))
                    .unwrap_or_default();
                let _ = writeln!(md, "- `{}` {shown}{industry}", v.id);
                total += 1;
            }
            let _ = writeln!(md);
        }
    }

    let _ = writeln!(md, "## 값 풀\n");
    let _ = writeln!(
        md,
        "| id | 라벨 | 값 | 분류 | 트리거 없음 | 업무 제한 금지 | 업종 |\n|---|---|---|---|---|---|---|"
    );
    for v in &values {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} |",
            v.id,
            label_name(v.label),
            v.value,
            v.topic,
            if v.trigger_free { "✓" } else { "" },
            if v.no_work_restriction { "✓" } else { "" },
            v.industry.as_deref().unwrap_or("")
        );
    }

    let _ = writeln!(
        md,
        "\n## 하드네거티브 ({}개, 번호 자리표시는 3단계 생성기가 채움)\n",
        negatives.len()
    );
    for n in &negatives {
        let _ = writeln!(md, "- `{}` `{:?}` {}", n.id, n.kind, n.template);
    }

    let header = format!("\n> 민감 문장 × 값 전체 조합 **{total}개**\n");
    let md = md.replacen("\n## HEALTH", &format!("{header}\n## HEALTH"), 1);
    let out = root().join("results/clauses_review_full.md");
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&out, md) {
        panic!("{}: {e}", out.display());
    }
    eprintln!("{} ({total}개 조합)", out.display());
}
