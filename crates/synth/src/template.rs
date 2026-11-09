//! 양식 템플릿 (`assets/templates/*.toml`) 스키마·로딩·검증 (PLAN.md 1-2).
//!
//! 템플릿 = 고정 문구 + `{slot_id}` 자리표시 + 블록 목록. 값 채우기·렌더링은 3~4단계.
//!
//! 규칙
//! - 같은 `{slot_id}`가 여러 번 나오면 **같은 값**이 들어간다 (문서 안 span은 여러 개).
//!   레이아웃 D처럼 문서를 묶을 때도 슬롯 이름이 같으면 같은 값을 쓴다.
//! - 체크박스(`□`, `( )`, `[ ]`)는 고정 문구에 남기지 않고 `kind = "check"` 슬롯 + `style`로 둔다.
//! - 라벨이 있는 슬롯은 `owner`를 반드시 적는다 (1-2 결정: 사업주 쪽 라벨을 정책으로 따로 켜고 끔).
//! - 라벨 체계에 없는 참고 항목(관계·만 나이·국적)은 `attribute`로 적는다 (평가에서 중립).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use pii_core::{AttributeKind, Label, Owner};
use serde::Deserialize;

use crate::ratios::RatioConfig;

/// 값 생성기 이름 (`gen`의 `:` 앞부분). 3단계에서 이 이름들을 구현한다.
/// `date:<이름>:<year|month|day|ymd>`, `time:<이름>:<hour|minute|hm>`, `won:<이름>`, `number:<종류>`처럼
/// `:` 뒤 인자로 같은 이름의 값끼리 일관성을 맞춘다 (예: 같은 `date:start`의 연·월·일).
pub const KNOWN_GENERATORS: &[&str] = &[
    // 개인정보
    "person_name_ko",
    "person_name_en",
    "address_ko",
    "address_foreign",
    "mobile_phone",
    "birth_date",
    "rrn",
    "bizno",
    // 개인정보 아님 (하드네거티브 포함)
    "company_name",
    "company_phone",
    "relation_guardian",
    "age_minor",
    // 근로조건
    "date",
    "time",
    "won",
    "number",
    "weekday",
    "workplace",
    "job_title",
    "industry",
    "business_desc",
    "job_desc",
    "allowance_name",
    "consent_status",
    // 서식 안 "기타( )" 칸의 짧은 메모
    "other_note",
    // 전화번호 종류 (3단계, company_phone = landline + national_rep 혼합)
    "landline",
    "national_rep",
    "internet_phone",
    // 하드네거티브 문장 풀 (clauses::HARD_NEGATIVE_SLOTS)
    "corp_reg_no",
    "doc_no",
    "doc_no13",
    "rrn_invalid_date",
    "workplace_mgmt_no",
    "zipcode",
    "company_account",
    // 공공 번호 (2단계 검토 추가): 4자리 특수번호, 공공기관 대표번호
    "special_number",
    "public_rep_number",
    // 템플릿에 적은 값 그대로 (`value`) — 처음 보는 서식 시험용
    "literal",
];

/// 템플릿 오류. 템플릿은 개인정보가 없는 고정 문구라 메시지에 문구를 넣어도 된다.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("템플릿 파일을 읽을 수 없습니다: {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("템플릿 TOML 형식 오류: {path}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("템플릿 검증 실패: {id}\n  - {}", problems.join("\n  - "))]
    Invalid { id: String, problems: Vec<String> },
}

/// 템플릿 파일 하나
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub meta: Meta,
    #[serde(default)]
    pub slots: BTreeMap<String, Slot>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    /// 파일 이름(확장자 제외)과 같아야 함. `forms extract`의 slug와 같다.
    pub id: String,
    pub title: String,
    /// 원본 PDF 쪽 번호 (1부터). 추출 텍스트 대조에 쓴다.
    pub source_pages: Vec<u32>,
    pub layouts: Vec<Layout>,
    /// 원본 PDF에서 곡선으로 그려져 텍스트로 뽑히지 않는 블록 문구 (추출 대조에서 제외)
    #[serde(default)]
    pub not_in_pdf: Vec<String>,
}

/// 레이아웃 (PLAN.md 1-2)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub enum Layout {
    A,
    B,
    C,
    D,
    E,
    F,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    /// 글자로 채우는 칸 (인쇄/손글씨는 합성 단계에서 결정)
    #[default]
    Text,
    /// 체크 표시 칸
    Check,
}

/// 체크 칸 모양: `□` / `( )` / `[ ]`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStyle {
    Box,
    Paren,
    Bracket,
}

/// 사업주 종류 — 문서마다 합성 단계에서 뽑는다. 사업주 쪽 값(전화·식별번호)의 선택지를 가른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmployerKind {
    /// 개인사업자
    Individual,
    /// 법인
    Corporation,
}

/// 칸 하나
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    #[serde(default)]
    pub kind: SlotKind,
    /// 값 생성기 (`choices`와 둘 중 하나). TOML 키는 `gen`
    #[serde(rename = "gen")]
    pub generator: Option<String>,
    /// `gen = "literal"`일 때 넣을 값
    pub value: Option<String>,
    pub label: Option<Label>,
    pub owner: Option<Owner>,
    pub attribute: Option<AttributeKind>,
    /// 양식 안 하드네거티브 값 (라벨 JSON `negatives`에 기록)
    #[serde(default)]
    pub hard_negative: bool,
    /// 사업주 종류 등에 따라 갈리는 선택지
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// 체크 칸 묶음 이름 (같은 묶음은 기본적으로 하나만 체크)
    pub group: Option<String>,
    pub style: Option<CheckStyle>,
    /// 묶음 안에서 여러 개 체크 가능 (사회보험 4종 등)
    #[serde(default)]
    pub multi: bool,
    /// 이 체크 칸이 체크됐을 때만 채움 (예: 상여금 "있음"일 때만 금액).
    /// `!` 접두사는 반대: 체크 안 됐을 때만 (예: 단시간 양식 기타급여 금액은 `!extra_no`)
    pub requires: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    /// 이 사업주 종류일 때만 후보 (없으면 항상)
    pub when: Option<EmployerKind>,
    /// TOML 키는 `gen`
    #[serde(rename = "gen")]
    pub generator: String,
    pub label: Option<Label>,
    #[serde(default)]
    pub hard_negative: bool,
    /// 비율 참조 `"표.키"` (`assets/synth/ratios.toml`). 같은 사업주 종류의 후보가 하나뿐이면 생략
    pub ratio: Option<String>,
}

/// 템플릿에서 선택지가 갈리는 사업주 종류 전체
const EMPLOYER_KINDS: [EmployerKind; 2] = [EmployerKind::Individual, EmployerKind::Corporation];

/// 문단 정렬
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// 블록. 순서대로 위에서 아래로 놓인다.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Block {
    Title {
        text: String,
    },
    Paragraph {
        text: String,
        /// 들여쓰기 단계 (0: 번호 조항, 1: `-` 항목, 2: `·` 항목)
        #[serde(default)]
        indent: u8,
        #[serde(default)]
        align: Align,
    },
    /// 표. `rows`는 행 목록, 셀은 문자열 또는 `{ text, rowspan, colspan }`. 셀 안 줄바꿈은 `\n`.
    Table {
        /// 열 폭 비율 (합 1)
        #[serde(default)]
        widths: Vec<f32>,
        rows: Vec<Vec<Cell>>,
    },
    /// 서명란. `anchor`는 `lines` 안에 있어야 하고, 서명·도장은 앵커 주변에 놓인다.
    Signature {
        owner: Owner,
        anchor: String,
        lines: Vec<String>,
    },
    /// 회사 추가 항목 표가 들어갈 자리 (레이아웃 B·C, 텍스트 모드에서는 A에도 확률적으로)
    CompanyExtra,
    /// 특약 문장(2단계 문장 풀, 민감정보 슬롯 포함)이 들어갈 자리
    SpecialTerms,
    /// 쪽 나눔 (앞·뒤 2쪽 양식)
    PageBreak,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    Text(String),
    Spanned {
        text: String,
        #[serde(default = "one")]
        rowspan: u8,
        #[serde(default = "one")]
        colspan: u8,
    },
}

fn one() -> u8 {
    1
}

impl Cell {
    pub fn text(&self) -> &str {
        match self {
            Cell::Text(t) | Cell::Spanned { text: t, .. } => t,
        }
    }
}

impl Template {
    /// TOML 문자열에서 읽고 검증한다. `expected_id`가 있으면 `meta.id`와 같아야 한다.
    pub fn parse(
        src: &str,
        origin: &str,
        expected_id: Option<&str>,
    ) -> Result<Self, TemplateError> {
        let template: Template = toml::from_str(src).map_err(|source| TemplateError::Parse {
            path: origin.to_string(),
            source,
        })?;
        let problems = template.validate(expected_id);
        if problems.is_empty() {
            Ok(template)
        } else {
            Err(TemplateError::Invalid {
                id: origin.to_string(),
                problems,
            })
        }
    }

    /// 파일에서 읽는다. 파일 이름(확장자 제외)이 `meta.id`와 같아야 한다.
    pub fn load(path: &Path) -> Result<Self, TemplateError> {
        let origin = path.display().to_string();
        let src = fs::read_to_string(path).map_err(|source| TemplateError::Io {
            path: origin.clone(),
            source,
        })?;
        let stem = path.file_stem().and_then(|s| s.to_str());
        Self::parse(&src, &origin, stem)
    }

    /// 폴더 안 `*.toml`을 모두 읽는다 (이름순)
    pub fn load_dir(dir: &Path) -> Result<Vec<Self>, TemplateError> {
        let io_err = |source| TemplateError::Io {
            path: dir.display().to_string(),
            source,
        };
        let mut paths: Vec<_> = fs::read_dir(dir)
            .map_err(io_err)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        paths.sort();
        paths.iter().map(|p| Self::load(p)).collect()
    }

    /// 고정 문구가 들어 있는 조각을 읽기 순서로 돌려준다 (제목·문단·표 셀(행 우선)·서명란 줄).
    /// 자리표시 `{slot}`은 그대로 둔다.
    pub fn text_segments(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for block in &self.blocks {
            match block {
                Block::Title { text } | Block::Paragraph { text, .. } => out.push(text.as_str()),
                Block::Table { rows, .. } => {
                    out.extend(rows.iter().flatten().map(Cell::text));
                }
                Block::Signature { lines, .. } => out.extend(lines.iter().map(String::as_str)),
                Block::CompanyExtra | Block::SpecialTerms | Block::PageBreak => {}
            }
        }
        out
    }

    /// 선택지의 비율 참조를 설정과 대조한다. 사업주 종류마다
    /// - 후보가 하나도 없으면 오류 (그 종류의 문서에서 값을 못 만듦)
    /// - 후보가 2개 이상이면 모두 같은 비율 표를 참조하고, 키가 겹치지 않으며 표의 키를 빠짐없이 써야 함
    pub fn check_ratios(&self, config: &RatioConfig) -> Vec<String> {
        let mut p = Vec::new();
        for (name, slot) in self.slots.iter().filter(|(_, s)| !s.choices.is_empty()) {
            for c in &slot.choices {
                if let Some(r) = &c.ratio
                    && config.get(r).is_none()
                {
                    p.push(format!("`{name}`: 비율 설정에 없는 참조 `{r}`"));
                }
            }
            for kind in EMPLOYER_KINDS {
                let cands: Vec<&Choice> = slot
                    .choices
                    .iter()
                    .filter(|c| c.when.is_none_or(|w| w == kind))
                    .collect();
                if cands.is_empty() {
                    p.push(format!("`{name}`: 사업주 종류 {kind:?}의 선택지가 없음"));
                    continue;
                }
                if cands.len() == 1 {
                    continue;
                }
                let refs: Vec<(&str, &str)> = cands
                    .iter()
                    .filter_map(|c| c.ratio.as_deref().and_then(|r| r.split_once('.')))
                    .collect();
                if refs.len() != cands.len() {
                    p.push(format!(
                        "`{name}`: {kind:?} 후보가 여럿이면 모두 ratio 필요"
                    ));
                    continue;
                }
                let tables: BTreeSet<&str> = refs.iter().map(|(t, _)| *t).collect();
                let keys: BTreeSet<&str> = refs.iter().map(|(_, k)| *k).collect();
                let [table] = tables.into_iter().collect::<Vec<_>>()[..] else {
                    p.push(format!(
                        "`{name}`: {kind:?} 후보가 서로 다른 비율 표를 참조"
                    ));
                    continue;
                };
                let expected: Option<BTreeSet<&str>> = config
                    .ratio
                    .get(table)
                    .map(|t| t.weights.keys().map(String::as_str).collect());
                if keys.len() != refs.len() || expected.is_some_and(|e| e != keys) {
                    p.push(format!(
                        "`{name}`: {kind:?} 후보가 비율 표 `{table}`의 키를 한 번씩 모두 써야 함"
                    ));
                }
            }
        }
        p
    }

    /// 검증. 문제 목록을 돌려준다 (비어 있으면 통과).
    pub fn validate(&self, expected_id: Option<&str>) -> Vec<String> {
        let mut p = Vec::new();
        let meta = &self.meta;
        if meta.id.is_empty() {
            p.push("meta.id가 비어 있음".to_string());
        }
        if let Some(id) = expected_id
            && id != meta.id
        {
            p.push(format!("meta.id `{}`가 파일 이름 `{id}`와 다름", meta.id));
        }
        // 처음 보는 서식(`holdout_*`)은 원본 PDF가 없다
        if meta.source_pages.is_empty() && !meta.id.starts_with("holdout_") {
            p.push("meta.source_pages가 비어 있음".to_string());
        }
        if meta.layouts.is_empty() {
            p.push("meta.layouts가 비어 있음".to_string());
        }

        // 자리표시 → 슬롯
        let mut used: BTreeSet<&str> = BTreeSet::new();
        for seg in self.text_segments() {
            match placeholders(seg) {
                Ok(names) => {
                    for n in names {
                        if !self.slots.contains_key(n) {
                            p.push(format!("정의되지 않은 슬롯 `{{{n}}}`: {seg:?}"));
                        }
                        used.insert(n);
                    }
                }
                Err(e) => p.push(format!("{e}: {seg:?}")),
            }
        }
        for name in self.slots.keys() {
            if !used.contains(name.as_str()) {
                p.push(format!("쓰지 않는 슬롯 `{name}`"));
            }
        }

        // 슬롯 규칙
        let mut group_multi: BTreeMap<&str, bool> = BTreeMap::new();
        for (name, slot) in &self.slots {
            validate_slot(name, slot, &self.slots, &mut p);
            if let Some(g) = &slot.group {
                match group_multi.get(g.as_str()) {
                    Some(&m) if m != slot.multi => {
                        p.push(format!("체크 묶음 `{g}` 안에서 multi 값이 다름 (`{name}`)"))
                    }
                    _ => {
                        group_multi.insert(g, slot.multi);
                    }
                }
            }
        }

        // 블록 규칙
        let mut extra = 0;
        let mut special = 0;
        for block in &self.blocks {
            match block {
                Block::Signature { anchor, lines, .. } => {
                    if lines.is_empty() {
                        p.push(format!("서명란 `{anchor}`에 줄이 없음"));
                    }
                    if !lines.iter().any(|l| l.contains(anchor.as_str())) {
                        p.push(format!("서명란 앵커 `{anchor}`가 줄 안에 없음"));
                    }
                }
                Block::Table { widths, rows } => {
                    if rows.is_empty() || rows.iter().any(Vec::is_empty) {
                        p.push("빈 표 또는 빈 행".to_string());
                    }
                    if !widths.is_empty() {
                        let sum: f32 = widths.iter().sum();
                        if (sum - 1.0).abs() > 0.01 {
                            p.push(format!("표 열 폭 합이 1이 아님: {sum}"));
                        }
                    }
                }
                Block::CompanyExtra => extra += 1,
                Block::SpecialTerms => special += 1,
                _ => {}
            }
        }
        if extra > 1 || special > 1 {
            p.push("company_extra·special_terms 블록은 각각 한 번까지".to_string());
        }
        p
    }
}

fn validate_slot(name: &str, slot: &Slot, all: &BTreeMap<String, Slot>, p: &mut Vec<String>) {
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        p.push(format!("슬롯 이름은 영문 소문자·숫자·_만: `{name}`"));
    }
    if slot.label.is_some() && slot.attribute.is_some() {
        p.push(format!("`{name}`: label과 attribute를 함께 쓸 수 없음"));
    }
    if (slot.generator.as_deref() == Some("literal")) != slot.value.is_some() {
        p.push(format!("`{name}`: gen = literal과 value는 함께 씀"));
    }
    if let Some(r) = &slot.requires {
        match all.get(r.strip_prefix('!').unwrap_or(r)) {
            Some(s) if s.kind == SlotKind::Check => {}
            _ => p.push(format!("`{name}`: requires `{r}`는 체크 슬롯이어야 함")),
        }
    }
    match slot.kind {
        SlotKind::Check => {
            if slot.generator.is_some()
                || !slot.choices.is_empty()
                || slot.label.is_some()
                || slot.attribute.is_some()
                || slot.hard_negative
            {
                p.push(format!(
                    "`{name}`: 체크 슬롯에는 gen·choices·label·attribute·hard_negative를 쓰지 않음"
                ));
            }
            if slot.group.is_none() || slot.style.is_none() {
                p.push(format!("`{name}`: 체크 슬롯에는 group과 style이 필요"));
            }
        }
        SlotKind::Text => {
            if slot.group.is_some() || slot.style.is_some() || slot.multi {
                p.push(format!("`{name}`: group·style·multi는 체크 슬롯에만"));
            }
            match (&slot.generator, slot.choices.is_empty()) {
                (Some(g), true) => check_gen(name, g, p),
                (None, false) => {
                    if slot.label.is_some() || slot.hard_negative {
                        p.push(format!(
                            "`{name}`: choices를 쓰면 label·hard_negative는 선택지마다 적음"
                        ));
                    }
                    for c in &slot.choices {
                        check_gen(name, &c.generator, p);
                        if let Some(r) = &c.ratio
                            && r.split_once('.').is_none()
                        {
                            p.push(format!("`{name}`: ratio는 `표.키` 형식: `{r}`"));
                        }
                        if c.label.is_some() && c.hard_negative {
                            p.push(format!(
                                "`{name}`: 선택지에 label과 hard_negative를 함께 쓸 수 없음"
                            ));
                        }
                        check_label(name, c.label, slot.owner, p);
                    }
                }
                _ => p.push(format!("`{name}`: gen과 choices 중 하나만 있어야 함")),
            }
            if slot.label.is_some() && slot.hard_negative {
                p.push(format!("`{name}`: label과 hard_negative를 함께 쓸 수 없음"));
            }
            check_label(name, slot.label, slot.owner, p);
        }
    }
}

fn check_label(name: &str, label: Option<Label>, owner: Option<Owner>, p: &mut Vec<String>) {
    if let Some(l) = label {
        if l.is_image_only() {
            p.push(format!(
                "`{name}`: {l:?}는 서명란 블록으로 표현 (슬롯 라벨 불가)"
            ));
        }
        if owner.is_none() {
            p.push(format!("`{name}`: 라벨이 있으면 owner 필수"));
        }
    }
}

fn check_gen(name: &str, generator: &str, p: &mut Vec<String>) {
    let base = generator.split(':').next().unwrap_or_default();
    if !KNOWN_GENERATORS.contains(&base) {
        p.push(format!("`{name}`: 모르는 생성기 `{generator}`"));
    }
}

/// 문구에서 `{slot}` 이름을 순서대로 뽑는다. 괄호 짝이 안 맞으면 오류.
pub fn placeholders(text: &str) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            return Err("짝 없는 `}`".to_string());
        }
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or("닫히지 않은 `{`")?;
        let name = &after[..close];
        if name.is_empty() || name.contains('{') {
            return Err(format!("잘못된 자리표시 `{{{name}}}`"));
        }
        out.push(name);
        rest = &after[close + 1..];
    }
    Ok(out)
}

/// 자리표시를 지운 고정 문구
pub fn strip_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0u32;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
[meta]
id = "t"
title = "시험"
source_pages = [1]
layouts = ["A"]

[slots]
worker_name = { gen = "person_name_ko", label = "NAME", owner = "worker" }
bonus_yes = { kind = "check", group = "bonus", style = "paren" }
bonus_no = { kind = "check", group = "bonus", style = "paren" }
bonus = { gen = "won:bonus", label = "SALARY", owner = "worker", requires = "bonus_yes" }
employer_phone = { owner = "employer", choices = [
  { when = "individual", gen = "mobile_phone", label = "PHONE", ratio = "phone.mobile" },
  { gen = "company_phone", hard_negative = true, ratio = "phone.company" },
] }

[[blocks]]
kind = "title"
text = "시험 양식"

[[blocks]]
kind = "paragraph"
text = "{worker_name}(이하 “근로자”라 함) 상여금 : 있음 {bonus_yes} {bonus}원, 없음 {bonus_no}"
indent = 1

[[blocks]]
kind = "table"
widths = [0.3, 0.7]
rows = [["전화", "{employer_phone}"], [{ text = "합친 칸", colspan = 2 }]]

[[blocks]]
kind = "company_extra"

[[blocks]]
kind = "signature"
owner = "worker"
anchor = "(서명)"
lines = ["성 명 : {worker_name} (서명)"]
"#;

    #[test]
    fn minimal_template_parses_and_validates() {
        let t = Template::parse(MINIMAL, "minimal", Some("t")).unwrap();
        assert_eq!(t.slots.len(), 5);
        assert_eq!(t.blocks.len(), 5);
        let segs = t.text_segments();
        assert_eq!(segs[2], "전화");
        assert_eq!(segs.last().copied(), Some("성 명 : {worker_name} (서명)"));
    }

    fn problems_of(src: &str) -> Vec<String> {
        match Template::parse(src, "x", Some("t")) {
            Err(TemplateError::Invalid { problems, .. }) => problems,
            other => panic!("검증 실패여야 함: {other:?}"),
        }
    }

    #[test]
    fn rejects_undefined_and_unused_slots() {
        let src = MINIMAL.replace("{bonus_no}\"", "{bonus_none}\"");
        let p = problems_of(&src);
        assert!(
            p.iter()
                .any(|m| m.contains("정의되지 않은 슬롯 `{bonus_none}`")),
            "{p:?}"
        );
        assert!(
            p.iter().any(|m| m.contains("쓰지 않는 슬롯 `bonus_no`")),
            "{p:?}"
        );
    }

    #[test]
    fn rejects_label_without_owner_and_unknown_generator() {
        let src = MINIMAL.replace(
            r#"worker_name = { gen = "person_name_ko", label = "NAME", owner = "worker" }"#,
            r#"worker_name = { gen = "person_nam", label = "NAME" }"#,
        );
        let p = problems_of(&src);
        assert!(p.iter().any(|m| m.contains("owner 필수")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("모르는 생성기")), "{p:?}");
    }

    #[test]
    fn rejects_missing_anchor_and_bad_id() {
        let src = MINIMAL.replace("anchor = \"(서명)\"", "anchor = \"(인)\"");
        let p = problems_of(&src);
        assert!(p.iter().any(|m| m.contains("앵커 `(인)`")), "{p:?}");

        let err = Template::parse(MINIMAL, "x", Some("other")).unwrap_err();
        assert!(err.to_string().contains("파일 이름 `other`"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let src = MINIMAL.replace("indent = 1", "indent = 1\nfont = \"bold\"");
        assert!(matches!(
            Template::parse(&src, "x", None),
            Err(TemplateError::Parse { .. })
        ));
    }

    const RATIOS: &str = r#"
[ratio.phone]
note = "t"
arbitrary = true
used_by = ["template:employer_phone"]
weights = { mobile = 0.7, company = 0.3 }
"#;

    #[test]
    fn ratio_references_are_checked_per_employer_kind() {
        let config = RatioConfig::parse(RATIOS, "r").unwrap();
        let t = Template::parse(MINIMAL, "minimal", Some("t")).unwrap();
        // 법인은 후보가 company_phone 하나뿐 → ratio 없어도 됨. 개인은 두 후보가 phone 표를 모두 씀
        assert!(
            t.check_ratios(&config).is_empty(),
            "{:?}",
            t.check_ratios(&config)
        );

        let missing = MINIMAL.replace("ratio = \"phone.company\"", "ratio = \"phone.landline\"");
        let t = Template::parse(&missing, "m", Some("t")).unwrap();
        let p = t.check_ratios(&config);
        assert!(
            p.iter().any(|m| m.contains("없는 참조 `phone.landline`")),
            "{p:?}"
        );
        assert!(p.iter().any(|m| m.contains("키를 한 번씩 모두")), "{p:?}");

        let no_corp = MINIMAL.replace(
            "{ gen = \"company_phone\", hard_negative = true, ratio = \"phone.company\" }",
            "{ when = \"individual\", gen = \"company_phone\", hard_negative = true, ratio = \"phone.company\" }",
        );
        let t = Template::parse(&no_corp, "n", Some("t")).unwrap();
        let p = t.check_ratios(&config);
        assert!(
            p.iter().any(|m| m.contains("Corporation의 선택지가 없음")),
            "{p:?}"
        );
    }

    #[test]
    fn placeholder_parsing() {
        assert_eq!(placeholders("{a}년 {b}월").unwrap(), vec!["a", "b"]);
        assert!(placeholders("{a").is_err());
        assert!(placeholders("a}").is_err());
        assert!(placeholders("{}").is_err());
        assert_eq!(strip_placeholders("({a})요일 {b}시"), "()요일 시");
    }
}
