//! 규칙 팩 스키마·로딩·검증 (PLAN.md 1장·9단계)
//!
//! 팩은 데이터만 담는다 (정규식·검증기 이름·문맥 키워드·양식 키·SLM 트리거·서명란 앵커·마스킹 정책).
//! 검증기 코드는 앱(`detect-rules`)에 내장. 판정용 범위: 스키마 검사 + 정규식 컴파일 + `\d`·`\w` 금지 +
//! **고유식별정보 정책 고정 검사**. 서명 검증·lint 상세·내장 테스트·롤백은 후속 (PLAN.md 일정 변경).

use std::collections::BTreeMap;
use std::path::Path;

use pii_core::Label;
use regex::Regex;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("규칙 팩 파일을 읽을 수 없습니다: {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("규칙 팩 형식 오류: {0}")]
    Parse(String),
    #[error("규칙 팩 거부:\n  - {}", .0.join("\n  - "))]
    Rejected(Vec<String>),
}

/// 앱에 내장된 검증기 이름 (팩은 이 이름만 참조할 수 있다)
pub const KNOWN_VALIDATORS: [&str; 6] = [
    "id13",
    "bizno_individual",
    "license",
    "date",
    "not_all_same",
    "account",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternDef {
    pub id: String,
    pub label: Label,
    pub regex: String,
    /// 값 위치 = 이 이름의 캡처 그룹 (없으면 전체 일치)
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub validator: Option<String>,
    /// 이 단어 중 하나가 앞쪽 `window` 글자 안에 있어야 함
    #[serde(default)]
    pub need_context: Vec<String>,
    /// 이 단어가 앞쪽 `window` 글자 안에 있으면 버림
    #[serde(default)]
    pub deny_context: Vec<String>,
    #[serde(default = "default_window")]
    pub window: usize,
    /// 앞뒤가 숫자(또는 하이픈+숫자)면 버림 — 더 긴 번호의 일부를 잡지 않게
    #[serde(default = "yes")]
    pub digit_boundary: bool,
}

fn default_window() -> usize {
    30
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NameFilterDef {
    pub stopwords: Vec<String>,
}

/// 필드 영역 마스킹 (이미지): 이 양식 키의 값 칸 전체를 가린다
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldsDef {
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlmDef {
    pub triggers: Vec<String>,
    /// SLM이 이 단어만 돌려주면 버림
    #[serde(default)]
    pub generic: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureDef {
    pub anchors: Vec<String>,
    pub worker: Vec<String>,
    pub employer: Vec<String>,
    pub guardian: Vec<String>,
    /// 소속 키워드를 찾는 범위: 앵커 위 줄 수
    pub lines_above: usize,
    /// 이 키 뒤 값이 비어 읽히면 키 오른쪽 칸을 가림
    #[serde(default)]
    pub empty_value_keys: Vec<String>,
    /// 앵커가 안 보여도 서명 줄로 보는 줄 머리 (이름 키). 줄 끝을 앵커로 삼는다
    #[serde(default)]
    pub sign_line_keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDef {
    /// 가리지 않는 라벨 (고유식별정보는 넣을 수 없음)
    #[serde(default)]
    pub off: Vec<Label>,
    /// 서명란 소속별 가림 (worker / employer / guardian / unknown)
    pub signature: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    pub meta: Meta,
    pub pattern: Vec<PatternDef>,
    pub name_filter: NameFilterDef,
    #[serde(default)]
    pub fields: FieldsDef,
    pub slm: SlmDef,
    pub signature: SignatureDef,
    pub policy: PolicyDef,
}

/// 컴파일된 패턴
#[derive(Debug, Clone)]
pub struct Pattern {
    pub def: PatternDef,
    pub regex: Regex,
}

/// 검증을 통과한 팩
#[derive(Debug, Clone)]
pub struct RulePack {
    pub meta: Meta,
    pub patterns: Vec<Pattern>,
    pub name_filter: NameFilterDef,
    pub fields: FieldsDef,
    pub slm: SlmDef,
    pub signature: SignatureDef,
    pub policy: PolicyDef,
}

impl RulePack {
    pub fn load(path: &Path) -> Result<Self, PackError> {
        let src = std::fs::read_to_string(path).map_err(|source| PackError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&src)
    }

    pub fn parse(src: &str) -> Result<Self, PackError> {
        let file: PackFile = toml::from_str(src).map_err(|e| PackError::Parse(e.to_string()))?;
        let mut problems = Vec::new();
        // 고유식별정보는 정책으로 끌 수 없음 (CLAUDE.md 도메인 규칙)
        for l in &file.policy.off {
            if Label::ALWAYS_MASKED.contains(l) {
                problems.push(format!("고유식별정보 {l:?}는 끌 수 없음"));
            }
        }
        for k in ["worker", "employer", "guardian", "unknown"] {
            if !file.policy.signature.contains_key(k) {
                problems.push(format!("서명란 정책에 `{k}` 없음"));
            }
        }
        let mut patterns = Vec::new();
        for p in &file.pattern {
            // 유니코드 클래스 금지 (전각 숫자 등까지 맞음) → [0-9] 같은 ASCII 클래스만
            if p.regex.contains("\\d") || p.regex.contains("\\w") {
                problems.push(format!("`{}`: \\d·\\w 금지", p.id));
            }
            if let Some(v) = &p.validator
                && !KNOWN_VALIDATORS.contains(&v.as_str())
            {
                problems.push(format!("`{}`: 모르는 검증기 `{v}`", p.id));
            }
            match Regex::new(&p.regex) {
                Ok(regex) => {
                    if let Some(g) = &p.group
                        && !regex.capture_names().flatten().any(|n| n == g)
                    {
                        problems.push(format!("`{}`: 캡처 그룹 `{g}` 없음", p.id));
                    }
                    patterns.push(Pattern {
                        def: p.clone(),
                        regex,
                    });
                }
                Err(e) => problems.push(format!("`{}`: 정규식 오류 {e}", p.id)),
            }
        }
        if !problems.is_empty() {
            return Err(PackError::Rejected(problems));
        }
        Ok(RulePack {
            meta: file.meta,
            patterns,
            name_filter: file.name_filter,
            fields: file.fields,
            slm: file.slm,
            signature: file.signature,
            policy: file.policy,
        })
    }

    /// 이름으로 잡았지만 양식 고정 문구인가 (공백 무시), 또는 한 글자씩 띄어 쓴 문구인가 (예: "근 무 장 소")
    pub fn is_not_a_name(&self, text: &str) -> bool {
        let core: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if core.chars().count() < 2 {
            return true;
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        let spaced = words.len() >= 2 && words.iter().all(|w| w.chars().count() == 1);
        spaced || self.name_filter.stopwords.contains(&core)
    }

    /// 이 라벨을 가리는가
    pub fn masks(&self, label: Label) -> bool {
        Label::ALWAYS_MASKED.contains(&label) || !self.policy.off.contains(&label)
    }

    /// 이 소속의 서명란을 가리는가 (모르면 unknown 정책)
    pub fn masks_signature(&self, owner: &str) -> bool {
        self.policy
            .signature
            .get(owner)
            .or_else(|| self.policy.signature.get("unknown"))
            .copied()
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/rulepacks/kr-contract-0.1.0.toml")
    }

    #[test]
    fn default_pack_loads() {
        let p = RulePack::load(&pack_path()).unwrap_or_else(|e| panic!("{e}"));
        assert!(!p.patterns.is_empty());
        assert!(!p.masks(Label::Salary));
        assert!(p.masks(Label::Rrn));
        assert!(!p.masks_signature("employer"));
        assert!(p.masks_signature("worker"));
    }

    #[test]
    fn rejects_pack_that_turns_off_unique_identifiers() {
        let src = std::fs::read_to_string(pack_path()).unwrap();
        let bad = src.replace("off = [\"SALARY\"]", "off = [\"SALARY\", \"RRN\"]");
        assert_ne!(src, bad);
        let err = RulePack::parse(&bad).unwrap_err().to_string();
        assert!(err.contains("끌 수 없음"), "{err}");
        let bad = src.replacen("regex = '[0-9]{6}", "regex = '\\d{6}", 1);
        assert_ne!(src, bad);
        assert!(RulePack::parse(&bad).is_err());
    }
}
