//! 2단계 문장 풀 (`assets/clauses/*.jsonl`) 스키마·검증·조사 처리.
//!
//! LLM(Claude)은 **슬롯 문장만** 만든다. 값 채우기·라벨 위치 계산은 Rust가 한다 (CLAUDE.md).
//! - 자리표시는 대문자 `{HEALTH}`, `{CORP_NO}` 등. 숫자·번호는 문장에 직접 쓰지 않고 자리표시로 둔다
//!   (법인등록번호 체크섬처럼 코드가 계산해야 하는 값).
//! - 조사는 자리표시 바로 뒤에 `을(를)`, `이(가)`, `(으)로` 같은 표시로 쓰고, 값의 받침에 따라 고른다.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use pii_core::{CharOffset, Label};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::template::placeholders;

/// 민감정보 라벨 (개인정보보호법 제23조) — `sensitive.jsonl`의 자리표시 이름과 같다
pub const SENSITIVE_LABELS: [Label; 5] = [
    Label::Health,
    Label::Union,
    Label::Criminal,
    Label::Religion,
    Label::Political,
];

/// 일반·민감 문장에서 쓸 수 있는 문서 슬롯 자리표시 → 템플릿 슬롯 이름 (그 문서의 값을 그대로 씀)
pub const DOC_SLOTS: &[(&str, &str)] = &[
    ("WORKER_NAME", "worker_name"),
    ("COMPANY", "employer_company"),
];

/// 하드네거티브 자리표시 → 값 생성기 (3단계 구현, `template::KNOWN_GENERATORS`에 있어야 함)
pub const HARD_NEGATIVE_SLOTS: &[(&str, &str)] = &[
    ("COMPANY_PHONE", "company_phone"),
    ("NATIONAL_REP", "national_rep"),
    ("INTERNET_PHONE", "internet_phone"),
    ("FAX", "landline"),
    ("EXTENSION", "number:extension"),
    ("CORP_NO", "corp_reg_no"),
    ("CORP_BIZ_NO", "bizno:corporation"),
    ("DOC_NO", "doc_no"),
    ("DOC_NO13", "doc_no13"),
    ("RRN_INVALID", "rrn_invalid_date"),
    ("WORKPLACE_MGMT_NO", "workplace_mgmt_no"),
    ("ZIPCODE", "zipcode"),
    ("COMPANY_ACCOUNT", "company_account"),
    ("SPECIAL_NUMBER", "special_number"),
    ("PUBLIC_REP", "public_rep_number"),
    ("AMOUNT", "won:amount"),
    ("MIN_WAGE", "won:min_wage_hourly"),
    ("DATE", "date:notice:ymd"),
];

/// 조사 표시 → (받침 있을 때, 없을 때). `(으)로`는 ㄹ받침이면 `로`.
pub const JOSA_MARKERS: &[(&str, &str, &str)] = &[
    ("을(를)", "을", "를"),
    ("이(가)", "이", "가"),
    ("은(는)", "은", "는"),
    ("과(와)", "과", "와"),
    ("(으)로", "으로", "로"),
    ("(이)라는", "이라는", "라는"),
    ("(이)나", "이나", "나"),
    ("(이)며", "이며", "며"),
    ("(이)고", "이고", "고"),
];

#[derive(Debug, thiserror::Error)]
pub enum ClauseError {
    #[error("문장 풀 파일을 읽을 수 없습니다: {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("문장 풀 JSONL 형식 오류: {path}:{line}")]
    Parse {
        path: String,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("문장 채우기 실패: {0}")]
    Render(String),
}

/// 일반 특약 문장 (`general.jsonl`)
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralClause {
    pub id: String,
    pub text: String,
    /// 질병·노동조합·종교 같은 말이 나오지만 특정 개인의 민감정보가 아닌 문장 → SLM 과잉 탐지 측정용
    #[serde(default)]
    pub sensitive_like: bool,
}

/// 민감정보 슬롯 문장 (`sensitive.jsonl`)
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitiveClause {
    pub id: String,
    pub label: Label,
    pub template: String,
    /// 트리거 키워드 없이 쓴 문장 → `trigger_free` 값과만 조합 (SLM 트리거 누락률 측정용)
    #[serde(default)]
    pub trigger_free: bool,
    /// 이 문장에 넣을 수 있는 값의 `topic` (비어 있으면 같은 라벨의 모든 값).
    /// 예: "보조기기를 지원한다"는 장애, "당원으로서"는 정당 가입만 — 어색한 조합을 막는다
    #[serde(default)]
    pub topics: Vec<String>,
    /// 이 문장에 넣을 수 있는 값 id 목록 (`topics` 대신 값을 직접 지정. 둘을 함께 쓰지 않는다).
    /// HEALTH는 분류만으로는 사실관계가 맞지 않는 조합이 생겨 값 목록으로 지정 (2단계 2차 검토)
    #[serde(default)]
    pub values: Vec<String>,
    /// 업무를 제한하는 문장 (야간근무 제외, 운전 업무 제외, 재검 의무 등) →
    /// `no_work_restriction` 값(HIV 감염 등)은 넣지 않는다 — 차별 소지 (2단계 검토 결정)
    #[serde(default)]
    pub restricts_work: bool,
}

impl SensitiveClause {
    /// 이 문장에 넣을 수 있는 값인지 (라벨·topic·trigger_free·업무 제한 조건).
    /// 값의 업종 조건(`industry`)은 문서마다 다르므로 여기서 보지 않는다 → [`SensitiveValue::fits_industry`]
    pub fn accepts(&self, v: &SensitiveValue) -> bool {
        v.label == self.label
            && (self.topics.is_empty() || self.topics.contains(&v.topic))
            && (self.values.is_empty() || self.values.contains(&v.id))
            && (!self.trigger_free || v.trigger_free)
            && !(self.restricts_work && v.no_work_restriction)
    }
}

/// 민감정보 값 (`sensitive_values.jsonl`)
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitiveValue {
    pub id: String,
    pub label: Label,
    pub value: String,
    /// 세부 분류 (예: HEALTH의 질환·장애·임신, 리포트 집계용)
    pub topic: String,
    #[serde(default)]
    pub trigger_free: bool,
    /// 업무 제한 문장(`restricts_work`)에 넣지 않는 값 (예: HIV 감염 — 차별 소지)
    #[serde(default)]
    pub no_work_restriction: bool,
    /// 이 업종의 문서에서만 쓰는 값 (예: 병원 노조 지회). 값은 [`INDUSTRIES`] 중 하나
    pub industry: Option<String>,
}

impl SensitiveValue {
    /// 문서 업종에 맞는 값인지 (업종 조건이 없으면 항상 맞음)
    pub fn fits_industry(&self, doc_industry: &str) -> bool {
        self.industry.as_deref().is_none_or(|i| i == doc_industry)
    }
}

/// 문서 업종 (3단계: 문서마다 하나 뽑아 회사명·직무·업종 조건 값에 쓴다)
pub const INDUSTRIES: &[&str] = &[
    "제조",
    "물류",
    "건설",
    "병원",
    "유통",
    "사무",
    "교육",
    "농축수산",
    "가사돌봄",
];

/// 하드네거티브 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardNegativeKind {
    /// 법 조항 번호 (고정 문구, 예: 근로기준법 제17조)
    LawArticle,
    DocNumber,
    /// 13자리 문서·관리번호
    DocNumber13,
    CompanyPhone,
    NationalRep,
    InternetPhone,
    Fax,
    Extension,
    /// 사업장관리번호 (4대보험)
    WorkplaceMgmtNo,
    /// 법인등록번호 (13자리, 6-7 형식, 자체 체크섬)
    CorpRegNo,
    /// 법인 사업자등록번호
    CorpBizNo,
    /// 주민번호 형식이지만 생년월일이 무효
    RrnInvalidDate,
    MinWage,
    Amount,
    Zipcode,
    CompanyAccount,
    /// 4자리 특수번호 (공공 상담전화, 예: 1350)
    SpecialNumber,
    /// 공공기관 대표번호 (전국 대표번호 또는 정부청사 지역번호 유선)
    PublicRep,
}

impl HardNegativeKind {
    /// 이 종류의 문장에 반드시 있어야 할 자리표시 (`LawArticle`은 고정 문구라 없음)
    pub fn required_slot(self) -> Option<&'static str> {
        use HardNegativeKind::*;
        Some(match self {
            LawArticle => return None,
            DocNumber => "DOC_NO",
            DocNumber13 => "DOC_NO13",
            CompanyPhone => "COMPANY_PHONE",
            NationalRep => "NATIONAL_REP",
            InternetPhone => "INTERNET_PHONE",
            Fax => "FAX",
            Extension => "EXTENSION",
            WorkplaceMgmtNo => "WORKPLACE_MGMT_NO",
            CorpRegNo => "CORP_NO",
            CorpBizNo => "CORP_BIZ_NO",
            RrnInvalidDate => "RRN_INVALID",
            MinWage => "MIN_WAGE",
            Amount => "AMOUNT",
            Zipcode => "ZIPCODE",
            CompanyAccount => "COMPANY_ACCOUNT",
            SpecialNumber => "SPECIAL_NUMBER",
            PublicRep => "PUBLIC_REP",
        })
    }

    /// 13자리 숫자 하드네거티브 (2020.10 이후 주민번호 규칙의 오탐 측정용, PLAN.md 2단계 필수)
    pub fn is_13_digit(self) -> bool {
        matches!(
            self,
            HardNegativeKind::CorpRegNo
                | HardNegativeKind::DocNumber13
                | HardNegativeKind::RrnInvalidDate
        )
    }
}

/// 하드네거티브 문장 (`hard_negatives.jsonl`)
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardNegativeClause {
    pub id: String,
    pub kind: HardNegativeKind,
    pub template: String,
}

/// JSONL 파일 읽기 (빈 줄 무시)
pub fn load_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, ClauseError> {
    let origin = path.display().to_string();
    let src = fs::read_to_string(path).map_err(|source| ClauseError::Io {
        path: origin.clone(),
        source,
    })?;
    src.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|source| ClauseError::Parse {
                path: origin.clone(),
                line: i + 1,
                source,
            })
        })
        .collect()
}

/// 트리거 키워드 목록 읽기 (한 줄에 하나, `#`부터 주석)
pub fn load_keywords(path: &Path) -> Result<Vec<String>, ClauseError> {
    let src = fs::read_to_string(path).map_err(|source| ClauseError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(src
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim())
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// 값의 마지막 글자 받침으로 조사를 고른다. 값이 한글 음절로 끝나지 않으면 `None`
/// (끝의 닫는 괄호·따옴표는 건너뜀: `안식일(토요일)` → `일`).
pub fn resolve_josa(value: &str, marker: &str) -> Option<&'static str> {
    let (_, with, without) = JOSA_MARKERS.iter().find(|(m, _, _)| *m == marker)?;
    let last = value
        .chars()
        .rev()
        .find(|c| !matches!(c, ')' | ']' | '”' | '’' | '"' | '\''))?;
    let code = u32::from(last);
    if !(0xAC00..=0xD7A3).contains(&code) {
        return None;
    }
    let jong = (code - 0xAC00) % 28;
    let has_batchim = jong != 0;
    // (으)로: ㄹ받침(종성 8)은 "로"
    let use_with = if marker == "(으)로" {
        has_batchim && jong != 8
    } else {
        has_batchim
    };
    Some(if use_with { with } else { without })
}

/// 자리표시를 값으로 바꾸고 바로 뒤의 조사 표시를 받침에 맞게 고른 문장.
/// 라벨 위치(span) 계산은 4단계에서 `CharOffset`으로 한다 — 여기서는 텍스트만 만든다.
pub fn render_text(
    template: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, ClauseError> {
    render_fields(template, lookup).map(|r| r.text)
}

/// 채운 문장 + 자리표시마다 채운 값의 위치 (문자 오프셋, 조사 제외)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub text: String,
    /// (자리표시 이름, 시작, 끝)
    pub fields: Vec<(String, CharOffset, CharOffset)>,
}

/// [`render_text`]와 같지만 채운 값의 문자 위치도 돌려준다 (라벨 span 계산용)
pub fn render_fields(
    template: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Rendered, ClauseError> {
    let names = placeholders(template).map_err(ClauseError::Render)?;
    let mut out = String::with_capacity(template.len() + 32);
    let mut chars = 0usize;
    let mut fields = Vec::new();
    let mut rest = template;
    let push = |out: &mut String, s: &str| {
        out.push_str(s);
        s.chars().count()
    };
    for name in names {
        let pat = format!("{{{name}}}");
        let Some(pos) = rest.find(&pat) else {
            return Err(ClauseError::Render(format!(
                "자리표시 `{pat}` 위치를 못 찾음"
            )));
        };
        chars += push(&mut out, &rest[..pos]);
        let value =
            lookup(name).ok_or_else(|| ClauseError::Render(format!("값 없음: `{name}`")))?;
        let start = chars;
        chars += push(&mut out, &value);
        fields.push((name.to_string(), CharOffset(start), CharOffset(chars)));
        rest = &rest[pos + pat.len()..];
        if let Some((marker, _, _)) = JOSA_MARKERS.iter().find(|(m, _, _)| rest.starts_with(m)) {
            let josa = resolve_josa(&value, marker).ok_or_else(|| {
                ClauseError::Render(format!(
                    "`{name}` 값이 한글 음절로 끝나지 않아 조사 `{marker}`를 고를 수 없음"
                ))
            })?;
            chars += push(&mut out, josa);
            rest = &rest[marker.len()..];
        }
    }
    push(&mut out, rest);
    Ok(Rendered { text: out, fields })
}

/// 문장 안에 자리표시 바로 뒤가 아닌 조사 표시가 있으면 그 표시들 (고정 문구에는 쓰지 않는다)
pub fn stray_josa_markers(template: &str) -> Vec<&'static str> {
    let mut stripped = String::new();
    let mut rest = template;
    // 자리표시 + 바로 뒤 조사 표시를 지운 나머지에서 찾는다
    while let Some(open) = rest.find('{') {
        stripped.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            stripped.push_str(&rest[open..]);
            rest = "";
            break;
        };
        rest = &rest[open + close + 1..];
        if let Some((m, _, _)) = JOSA_MARKERS.iter().find(|(m, _, _)| rest.starts_with(m)) {
            rest = &rest[m.len()..];
        }
    }
    stripped.push_str(rest);
    JOSA_MARKERS
        .iter()
        .map(|(m, _, _)| *m)
        .filter(|m| stripped.contains(m))
        .collect()
}

/// 문장에 들어 있는 트리거 키워드
pub fn triggers_in<'a>(text: &str, keywords: &'a [String]) -> Vec<&'a str> {
    keywords
        .iter()
        .filter(|k| text.contains(k.as_str()))
        .map(String::as_str)
        .collect()
}

/// 자리표시 이름이 허용 목록에 있는지 검사하고, 쓰인 이름 집합을 돌려준다
pub fn check_placeholders<'t>(
    template: &'t str,
    allowed: &[&str],
    problems: &mut Vec<String>,
    id: &str,
) -> BTreeSet<&'t str> {
    match placeholders(template) {
        Ok(names) => {
            for n in &names {
                if !allowed.contains(n) {
                    problems.push(format!("{id}: 허용되지 않은 자리표시 `{{{n}}}`"));
                }
            }
            names.into_iter().collect()
        }
        Err(e) => {
            problems.push(format!("{id}: {e}"));
            BTreeSet::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn josa_follows_final_consonant() {
        assert_eq!(resolve_josa("당뇨병", "(으)로"), Some("으로"));
        assert_eq!(resolve_josa("허리 디스크", "(으)로"), Some("로"));
        // ㄹ받침은 "로"
        assert_eq!(resolve_josa("주 3회 통원 치료 일", "(으)로"), Some("로"));
        assert_eq!(resolve_josa("불교", "을(를)"), Some("를"));
        assert_eq!(resolve_josa("천주교 신앙", "을(를)"), Some("을"));
        assert_eq!(resolve_josa("안식일(토요일)", "이(가)"), Some("이"));
        assert_eq!(resolve_josa("HIV", "이(가)"), None);
        assert_eq!(resolve_josa("불교", "없는표시"), None);
    }

    #[test]
    fn renders_values_and_josa() {
        let t = "근로자는 {HEALTH}(으)로 인해 {WORKER_NAME}은(는) 조퇴한다";
        let out = render_text(t, |n| match n {
            "HEALTH" => Some("허리 디스크".to_string()),
            "WORKER_NAME" => Some("김가람".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(out, "근로자는 허리 디스크로 인해 김가람은 조퇴한다");
        assert!(render_text("{X}", |_| None).is_err());
    }

    #[test]
    fn rendered_fields_point_at_values() {
        let r = render_fields(
            "근로자 {WORKER_NAME}은(는) {HEALTH}(으)로 조퇴",
            |n| match n {
                "WORKER_NAME" => Some("김가람".into()),
                "HEALTH" => Some("천식".into()),
                _ => None,
            },
        )
        .unwrap();
        assert_eq!(r.text, "근로자 김가람은 천식으로 조퇴");
        for (name, s, e) in &r.fields {
            let got = pii_core::slice_chars(&r.text, *s, *e).unwrap();
            assert_eq!(
                got,
                if name == "HEALTH" {
                    "천식"
                } else {
                    "김가람"
                }
            );
        }
    }

    #[test]
    fn stray_markers_are_found() {
        assert!(stray_josa_markers("{HEALTH}(으)로 인해").is_empty());
        assert_eq!(stray_josa_markers("사업주과(와) 근로자"), vec!["과(와)"]);
    }
}
