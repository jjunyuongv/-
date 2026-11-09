//! 합성 비율 설정 (`assets/synth/ratios.toml`).
//!
//! 템플릿 선택지(`choices`)와 값 생성기가 쓰는 비율을 한곳에 모은다. 템플릿은 `ratio = "표.키"`로 참조한다.
//! `arbitrary = true`인 표는 근거 없이 정한 초기값 → 6단계 데이터 리포트에 "임의로 정한 비율"로 표시한다.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Deserialize;

/// 비율 합 허용 오차
const SUM_TOL: f64 = 1e-6;

#[derive(Debug, thiserror::Error)]
pub enum RatioError {
    #[error("비율 설정 파일을 읽을 수 없습니다: {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("비율 설정 TOML 형식 오류: {path}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("비율 설정 검증 실패: {path}\n  - {}", problems.join("\n  - "))]
    Invalid { path: String, problems: Vec<String> },
}

/// 실제로 뽑힌 횟수 (표 → 키 → 횟수). 6단계 리포트에서 설정값과 나란히 보여 준다
pub type TallyCounts = BTreeMap<String, BTreeMap<String, u64>>;

/// 비율 집계 (복제본끼리 공유)
#[derive(Debug, Clone, Default)]
pub struct Tally(Arc<Mutex<TallyCounts>>);

impl Tally {
    /// 표 `table`에서 `key`가 한 번 뽑힘
    pub fn record(&self, table: &str, key: &str) {
        if let Ok(mut t) = self.0.lock() {
            *t.entry(table.to_string())
                .or_default()
                .entry(key.to_string())
                .or_default() += 1;
        }
    }

    /// 지금까지 집계를 꺼내고 비운다
    pub fn take(&self) -> TallyCounts {
        self.0
            .lock()
            .map(|mut t| std::mem::take(&mut *t))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RatioConfig {
    pub ratio: BTreeMap<String, RatioTable>,
    /// 뽑힌 횟수 집계 (설정 파일에는 없음)
    #[serde(skip)]
    pub tally: Tally,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RatioTable {
    /// 무엇을 나누는 비율인지
    pub note: String,
    /// 근거 없이 정한 초기값이면 true (6단계 리포트에 표시)
    pub arbitrary: bool,
    /// 이 비율을 쓰는 곳 (예: `template:employer_phone`, `generator:company_phone`)
    pub used_by: Vec<String>,
    /// 키 → 비율 (합 1)
    pub weights: BTreeMap<String, f64>,
}

impl RatioConfig {
    pub fn parse(src: &str, origin: &str) -> Result<Self, RatioError> {
        let config: RatioConfig = toml::from_str(src).map_err(|source| RatioError::Parse {
            path: origin.to_string(),
            source,
        })?;
        let problems = config.validate();
        if problems.is_empty() {
            Ok(config)
        } else {
            Err(RatioError::Invalid {
                path: origin.to_string(),
                problems,
            })
        }
    }

    pub fn load(path: &Path) -> Result<Self, RatioError> {
        let origin = path.display().to_string();
        let src = fs::read_to_string(path).map_err(|source| RatioError::Io {
            path: origin.clone(),
            source,
        })?;
        Self::parse(&src, &origin)
    }

    fn validate(&self) -> Vec<String> {
        let mut p = Vec::new();
        for (name, table) in &self.ratio {
            if table.weights.len() < 2 {
                p.push(format!("`{name}`: 키가 2개 이상이어야 함"));
            }
            if table.used_by.is_empty() {
                p.push(format!("`{name}`: used_by가 비어 있음"));
            }
            for (key, w) in &table.weights {
                if w.is_nan() || *w <= 0.0 || *w > 1.0 {
                    p.push(format!("`{name}.{key}`: 비율은 0 초과 1 이하: {w}"));
                }
            }
            let sum: f64 = table.weights.values().sum();
            if (sum - 1.0).abs() > SUM_TOL {
                p.push(format!("`{name}`: 비율 합이 1이 아님: {sum}"));
            }
        }
        p
    }

    /// `"표.키"` 참조로 뽑힌 것을 집계한다
    pub fn record_ref(&self, reference: &str) {
        if let Some((table, key)) = reference.split_once('.') {
            self.tally.record(table, key);
        }
    }

    /// `"표.키"` 참조를 비율 값으로 바꾼다
    pub fn get(&self, reference: &str) -> Option<f64> {
        let (table, key) = reference.split_once('.')?;
        self.ratio.get(table)?.weights.get(key).copied()
    }

    /// 6단계 리포트용: 임의로 정한 비율 표 목록
    pub fn arbitrary(&self) -> impl Iterator<Item = (&str, &RatioTable)> {
        self.ratio
            .iter()
            .filter(|(_, t)| t.arbitrary)
            .map(|(n, t)| (n.as_str(), t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
[ratio.phone]
note = "전화 종류"
arbitrary = true
used_by = ["template:employer_phone"]
weights = { mobile = 0.7, company = 0.3 }

[ratio.fixed]
note = "근거 있는 비율"
arbitrary = false
used_by = ["generator:x"]
weights = { a = 0.5, b = 0.5 }
"#;

    #[test]
    fn parses_and_looks_up() {
        let c = RatioConfig::parse(SRC, "t").unwrap();
        assert_eq!(c.get("phone.mobile"), Some(0.7));
        assert_eq!(c.get("phone.none"), None);
        assert_eq!(c.get("nodot"), None);
        let arb: Vec<&str> = c.arbitrary().map(|(n, _)| n).collect();
        assert_eq!(arb, vec!["phone"]);
    }

    #[test]
    fn rejects_bad_sums() {
        let bad = SRC.replace("company = 0.3", "company = 0.2");
        let err = RatioConfig::parse(&bad, "t").unwrap_err();
        assert!(err.to_string().contains("합이 1이 아님"), "{err}");
    }
}
