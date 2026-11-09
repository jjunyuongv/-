//! 탐지 조합과 평가 지표 (PLAN.md 9~13단계)
//!
//! - [`Pipeline`]: 규칙 팩 → (NER) → (SLM 트리거 청크) → 같은 값 전파
//! - 텍스트 평가: 정답 span의 **글자 90% 이상이 어떤 탐지에든 덮이면 찾음** (가리는 목적이므로 라벨 무관).
//!   정책상 가리지 않는 라벨(SALARY)은 재현율에서 뺌. 참고 항목(attributes)에 걸린 탐지는 중립.
//!   하드네거티브(negatives)에 걸리면 오탐
//! - 비율은 Wilson 95% 신뢰구간과 함께

pub mod dashboard;
pub mod image;
pub mod issues;
pub mod runlog;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use detect_ner::NerDetector;
use detect_slm::{SlmClient, SlmStats};
use pii_core::{Detection, DocLabel, Label};
use rulepack::RulePack;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error(transparent)]
    Pack(#[from] rulepack::PackError),
    #[error(transparent)]
    Ner(#[from] detect_ner::NerError),
    #[error(transparent)]
    Slm(#[from] detect_slm::SlmError),
    #[error(transparent)]
    Ocr(#[from] ocr::OcrError),
    #[error("{0}")]
    Other(String),
}

/// NER 결과 중 쓰는 라벨 (비정형)
pub const NER_LABELS: [Label; 2] = [Label::Name, Label::Address];

/// 탐지 조합
pub struct Pipeline {
    pub pack: RulePack,
    pub ner: Option<NerDetector>,
    pub slm: Option<SlmClient>,
}

/// 문서 하나의 탐지 시간 (초)
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct DetectTiming {
    pub rules: f64,
    pub ner: f64,
    pub slm: f64,
    pub slm_calls: usize,
    pub slm_dropped: usize,
    pub slm_multi: usize,
}

impl Pipeline {
    pub fn new(
        pack_path: &Path,
        ner_dir: Option<&Path>,
        slm_url: Option<&str>,
    ) -> Result<Self, EvalError> {
        let pack = RulePack::load(pack_path)?;
        let ner = match ner_dir {
            Some(d) => Some(NerDetector::load(d)?),
            None => None,
        };
        let slm = match slm_url {
            Some(u) => Some(SlmClient::new(
                u,
                pack.slm.triggers.clone(),
                pack.slm.generic.clone(),
            )?),
            None => None,
        };
        Ok(Pipeline { pack, ner, slm })
    }

    /// 텍스트 → 탐지 (원문 글자 위치)
    pub fn detect(&mut self, text: &str) -> Result<(Vec<Detection>, DetectTiming), EvalError> {
        let mut t = DetectTiming::default();
        let t0 = Instant::now();
        let mut found = detect_rules::detect(&self.pack, text);
        t.rules = t0.elapsed().as_secs_f64();
        if let Some(ner) = &mut self.ner {
            let t0 = Instant::now();
            // NER은 비정형(이름·주소)만 쓴다. 정형 번호는 규칙+검증기가 맡음 — NER은 회사 대표번호·법인번호까지
            // PHONE·RRN으로 잡아 하드네거티브 오탐이 커짐 (개발용 세트 확인, PLAN.md 10단계 메모)
            let chars: Vec<char> = text.chars().collect();
            let pack = &self.pack;
            found.extend(ner.detect(text)?.into_iter().filter(|d| {
                let v: String = chars[d.start.0.min(chars.len())..d.end.0.min(chars.len())]
                    .iter()
                    .collect();
                NER_LABELS.contains(&d.label) && !(d.label == Label::Name && pack.is_not_a_name(&v))
            }));
            t.ner = t0.elapsed().as_secs_f64();
        }
        if let Some(slm) = &self.slm {
            let (d, s): (Vec<Detection>, SlmStats) = slm.detect(text)?;
            found.extend(d);
            t.slm = s.seconds;
            t.slm_calls = s.calls;
            t.slm_dropped = s.dropped;
            t.slm_multi = s.multi;
        }
        // 정책상 가리지 않는 라벨은 뺀다 (예: SALARY)
        found.retain(|d| self.pack.masks(d.label));
        let extra = detect_rules::propagate(text, &found);
        found.extend(extra);
        Ok((found, t))
    }
}

/// Wilson 95% 신뢰구간 (k/n)
pub fn wilson(k: usize, n: usize) -> (f64, f64, f64) {
    if n == 0 {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let (k, n) = (k as f64, n as f64);
    let p = k / n;
    let z = 1.96f64;
    let denom = 1.0 + z * z / n;
    let centre = (p + z * z / (2.0 * n)) / denom;
    let half = z * ((p * (1.0 - p) / n) + z * z / (4.0 * n * n)).sqrt() / denom;
    (p, (centre - half).max(0.0), (centre + half).min(1.0))
}

/// 비율 + 구간
#[derive(Debug, Clone, Default, Serialize)]
pub struct Rate {
    pub k: usize,
    pub n: usize,
    pub p: f64,
    pub lo: f64,
    pub hi: f64,
}

impl Rate {
    pub fn new(k: usize, n: usize) -> Self {
        let (p, lo, hi) = wilson(k, n);
        Rate { k, n, p, lo, hi }
    }

    pub fn fmt_pct(&self) -> String {
        if self.n == 0 {
            "-".into()
        } else {
            format!(
                "{:.1}% ({}/{}, {:.1}~{:.1})",
                self.p * 100.0,
                self.k,
                self.n,
                self.lo * 100.0,
                self.hi * 100.0
            )
        }
    }
}

/// 글자 범위 [s, e)를 탐지들이 덮는 비율 (공백 제외)
pub fn char_coverage(text: &[char], s: usize, e: usize, dets: &[Detection]) -> f64 {
    let idx: Vec<usize> = (s..e.min(text.len()))
        .filter(|&i| !text[i].is_whitespace())
        .collect();
    if idx.is_empty() {
        return 1.0;
    }
    let covered = idx
        .iter()
        .filter(|&&i| dets.iter().any(|d| d.start.0 <= i && i < d.end.0))
        .count();
    covered as f64 / idx.len() as f64
}

/// 하드네거티브 종류 (source 접미사, 예: `n065:PUBLIC_REP` → PUBLIC_REP)
pub fn negative_kind(source: &str) -> String {
    source.rsplit(':').next().unwrap_or(source).to_string()
}

/// 13자리 하드네거티브 (주민번호 모양)인가
pub fn is_13digit_negative(kind: &str) -> bool {
    matches!(kind, "RRN_INVALID" | "DOC_NO13" | "CORP_NO")
}

/// 텍스트 평가 누적
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct TextEval {
    pub docs: usize,
    /// 라벨 → (찾음, 전체)
    pub recall: BTreeMap<String, (usize, usize)>,
    /// 라벨 → 그 라벨로 맞게 찾은 수 (라벨까지 맞음)
    pub label_correct: BTreeMap<String, usize>,
    /// 하드네거티브 종류 → (가림, 전체)
    pub negatives: BTreeMap<String, (usize, usize)>,
    /// 탐지 수: 정답과 겹침 / 참고 항목만 / 그 밖 (오탐)
    pub det_true: usize,
    pub det_neutral: usize,
    pub det_false: usize,
    /// 오탐 탐지기별
    pub false_by_detector: BTreeMap<String, usize>,
    pub seconds_total: f64,
    pub timing: DetectTiming,
}

impl TextEval {
    pub fn add(
        &mut self,
        label: &DocLabel,
        dets: &[Detection],
        pack: &RulePack,
        t: &DetectTiming,
        secs: f64,
    ) {
        self.docs += 1;
        self.seconds_total += secs;
        self.timing.rules += t.rules;
        self.timing.ner += t.ner;
        self.timing.slm += t.slm;
        self.timing.slm_calls += t.slm_calls;
        self.timing.slm_dropped += t.slm_dropped;
        self.timing.slm_multi += t.slm_multi;
        let text: Vec<char> = label.text.chars().collect();
        for s in &label.spans {
            if !pack.masks(s.label) {
                continue;
            }
            let name = label_key(s.label);
            let hit = char_coverage(&text, s.start.0, s.end.0, dets) >= 0.9;
            let e = self.recall.entry(name.clone()).or_default();
            e.0 += usize::from(hit);
            e.1 += 1;
            if dets
                .iter()
                .any(|d| d.label == s.label && d.overlaps(s.start.0, s.end.0))
            {
                *self.label_correct.entry(name).or_default() += 1;
            }
        }
        for n in &label.negatives {
            let kind = negative_kind(&n.source);
            let hit = dets.iter().any(|d| d.overlaps(n.start.0, n.end.0));
            let e = self.negatives.entry(kind).or_default();
            e.0 += usize::from(hit);
            e.1 += 1;
        }
        for d in dets {
            let gold = label.spans.iter().any(|s| d.overlaps(s.start.0, s.end.0));
            let attr = label
                .attributes
                .iter()
                .any(|a| d.overlaps(a.start.0, a.end.0));
            if gold {
                self.det_true += 1;
            } else if attr {
                self.det_neutral += 1;
            } else {
                self.det_false += 1;
                *self
                    .false_by_detector
                    .entry(format!("{:?}", d.detector).to_lowercase())
                    .or_default() += 1;
            }
        }
    }

    pub fn label_rate(&self, l: Label) -> Rate {
        let (k, n) = self.recall.get(&label_key(l)).copied().unwrap_or_default();
        Rate::new(k, n)
    }

    pub fn group_rate(&self, labels: &[Label]) -> Rate {
        let (mut k, mut n) = (0, 0);
        for l in labels {
            let (a, b) = self.recall.get(&label_key(*l)).copied().unwrap_or_default();
            k += a;
            n += b;
        }
        Rate::new(k, n)
    }

    pub fn negative_rate(&self) -> Rate {
        let (k, n) = self
            .negatives
            .values()
            .fold((0, 0), |a, (k, n)| (a.0 + k, a.1 + n));
        Rate::new(k, n)
    }

    pub fn negative_rate_13(&self) -> Rate {
        let (k, n) = self
            .negatives
            .iter()
            .filter(|(kind, _)| is_13digit_negative(kind))
            .fold((0, 0), |a, (_, (k, n))| (a.0 + k, a.1 + n));
        Rate::new(k, n)
    }

    pub fn precision(&self) -> Rate {
        Rate::new(self.det_true, self.det_true + self.det_false)
    }
}

pub fn label_key(l: Label) -> String {
    serde_json::to_value(l)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{l:?}"))
}

/// 판정 기준의 라벨 묶음
pub const STRUCTURED: [Label; 7] = [
    Label::Rrn,
    Label::Frn,
    Label::Passport,
    Label::DriverLicense,
    Label::Phone,
    Label::Email,
    Label::Account,
];
pub const NAME_ADDRESS: [Label; 2] = [Label::Name, Label::Address];
pub const SENSITIVE: [Label; 5] = [
    Label::Health,
    Label::Union,
    Label::Criminal,
    Label::Religion,
    Label::Political,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_matches_known_value() {
        // 990/1000 → 약 98.2%~99.5%
        let (p, lo, hi) = wilson(990, 1000);
        assert!((p - 0.99).abs() < 1e-9);
        assert!(
            (lo - 0.9817).abs() < 0.001 && (hi - 0.9946).abs() < 0.001,
            "{lo} {hi}"
        );
    }
}
