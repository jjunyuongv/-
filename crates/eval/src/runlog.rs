//! 실행 기록 (개선 기록 체계): 실행마다 `results/runs/<실행ID>/metrics.json`·`failures.json`
//!
//! - metrics.json: 지표 키·값·Wilson 95% 신뢰구간·목표·목표 대비 판정 + 실행 정보(실험·세트·시드·변경 사항)
//! - failures.json: 실패 유형별 건수 (값 없음)
//! - `pii-mask eval issues`가 이 기록으로 `docs/issues.md` 자동 영역과 `docs/run_log.md`를 갱신한다
//!
//! 실행 ID = `YYYYMMDD-HHMMSS-<실험>` (한국 시간). 지표 키 = `<실험>.<지표>` (예: `e4.sensitive_recall`)

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::image::DiffStats;
use crate::{NAME_ADDRESS, Rate, SENSITIVE, STRUCTURED, TextEval, label_key};

/// 목표 방향
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Goal {
    /// 이 값 이상
    AtLeast,
    /// 이 값 이하
    AtMost,
    /// 목표 없음 (보고만)
    Report,
}

/// 지표 하나
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub key: String,
    pub name: String,
    pub k: usize,
    pub n: usize,
    pub value: f64,
    pub lo: f64,
    pub hi: f64,
    pub goal: Goal,
    pub target: f64,
    /// 가능 / 조건부 가능 / 불가 / 보고
    pub verdict: String,
}

impl Metric {
    pub fn rate(key: &str, name: &str, r: &Rate, goal: Goal, target: f64) -> Self {
        Metric {
            key: key.into(),
            name: name.into(),
            k: r.k,
            n: r.n,
            // JSON에 NaN을 쓰지 않는다 (null로 바뀌어 다시 읽을 수 없음) — 표본 없음은 n = 0으로 구분
            value: if r.n == 0 { 0.0 } else { r.p },
            lo: if r.n == 0 { 0.0 } else { r.lo },
            hi: if r.n == 0 { 0.0 } else { r.hi },
            goal,
            target,
            verdict: verdict(r, goal, target).into(),
        }
    }

    /// 값만 있는 지표 (시간 등)
    pub fn plain(key: &str, name: &str, value: f64) -> Self {
        Metric {
            key: key.into(),
            name: name.into(),
            k: 0,
            n: 0,
            value: if value.is_finite() { value } else { 0.0 },
            lo: 0.0,
            hi: 0.0,
            goal: Goal::Report,
            target: 0.0,
            verdict: "보고".into(),
        }
    }

    /// 목표 미달인가
    pub fn failing(&self) -> bool {
        self.verdict == "불가"
    }

    pub fn fmt_value(&self) -> String {
        if self.n == 0 {
            // 값만 있는 지표(시간 등)는 그대로, 비율인데 표본이 없으면 -
            return if self.verdict == "보고" && self.value != 0.0 {
                format!("{:.2}", self.value)
            } else {
                "-".into()
            };
        }
        format!(
            "{:.1}% ({}/{}, {:.1}~{:.1})",
            self.value * 100.0,
            self.k,
            self.n,
            self.lo * 100.0,
            self.hi * 100.0
        )
    }

    pub fn fmt_target(&self) -> String {
        match self.goal {
            Goal::AtLeast => format!("≥ {:.0}%", self.target * 100.0),
            Goal::AtMost => format!("≤ {:.0}%", self.target * 100.0),
            Goal::Report => "보고".into(),
        }
    }
}

/// 판정 규칙 (리포트와 같음): 점추정이 기준 + 구간도 기준±1%p 안 → 가능, 점추정만 → 조건부 가능, 아니면 불가
pub fn verdict(r: &Rate, goal: Goal, target: f64) -> &'static str {
    if goal == Goal::Report {
        return "보고";
    }
    if r.n == 0 {
        return "표본 없음";
    }
    match goal {
        Goal::AtLeast if r.p >= target && r.lo >= target - 0.01 => "가능",
        Goal::AtLeast if r.p >= target => "조건부 가능",
        Goal::AtMost if r.p <= target && r.hi <= target + 0.01 => "가능",
        Goal::AtMost if r.p <= target => "조건부 가능",
        _ => "불가",
    }
}

/// 실행 기록
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    /// YYYY-MM-DD (한국 시간)
    pub date: String,
    /// e1 / e2 / e4 / e4b / e5 …
    pub exp: String,
    /// 데이터 세트 폴더
    pub set: String,
    /// 데이터 세트 시드 (문서 ID `s<시드>_…`에서)
    pub seed: Option<u64>,
    /// 평가용 / 개발용 (세트 이름으로 판단)
    pub set_kind: String,
    pub docs: usize,
    /// 변경 사항 (사람이 적음)
    pub note: String,
    pub metrics: Vec<Metric>,
}

/// 한국 시간 (UTC+9) 날짜·시각
pub fn now_kst() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        + 9 * 3600;
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    // 날짜 계산 (Howard Hinnant civil_from_days)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        format!("{y:04}-{m:02}-{d:02}"),
        format!("{:02}{:02}{:02}", tod / 3600, (tod % 3600) / 60, tod % 60),
    )
}

/// 새 실행 ID
pub fn new_run_id(exp: &str) -> (String, String) {
    let (date, time) = now_kst();
    (format!("{}-{time}-{exp}", date.replace('-', "")), date)
}

/// 세트 폴더 이름 → (시드, 종류)
/// 처음 보는 서식 시험 세트인가 (본 판정·추이에 섞지 않는다)
pub fn is_holdout_set(set: &str) -> bool {
    set.contains("holdout")
}

pub fn set_info(set: &Path) -> (Option<u64>, String) {
    let name = set
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let seed = name.rsplit_once("_s").and_then(|(_, s)| s.parse().ok());
    // 평가용: eval*·text_* (개발용 텍스트는 devtext_*)
    let kind = if name.starts_with("eval") || name.starts_with("text_") {
        "평가용"
    } else {
        "개발용"
    };
    (seed, kind.into())
}

/// 텍스트 실험 지표
pub fn text_metrics(exp: &str, e: &TextEval) -> Vec<Metric> {
    // 민감정보 목표는 SLM을 쓰는 실험(e4·e4b)만
    let sens_goal = if exp.contains("e4") {
        Goal::AtLeast
    } else {
        Goal::Report
    };
    let mut out = vec![
        Metric::rate(
            &format!("{exp}.structured_recall"),
            "정형 PII 재현율 (텍스트)",
            &e.group_rate(&STRUCTURED),
            Goal::AtLeast,
            0.99,
        ),
        Metric::rate(
            &format!("{exp}.name_address_recall"),
            "이름·주소 재현율 (텍스트)",
            &e.group_rate(&NAME_ADDRESS),
            Goal::AtLeast,
            0.95,
        ),
        Metric::rate(
            &format!("{exp}.sensitive_recall"),
            "민감정보 재현율 (텍스트)",
            &e.group_rate(&SENSITIVE),
            sens_goal,
            0.90,
        ),
        Metric::rate(
            &format!("{exp}.fp_rate"),
            "하드네거티브 오탐률 (텍스트)",
            &e.negative_rate(),
            Goal::AtMost,
            0.05,
        ),
        Metric::rate(
            &format!("{exp}.precision"),
            "정밀도 (텍스트)",
            &e.precision(),
            Goal::Report,
            0.0,
        ),
        Metric::plain(
            &format!("{exp}.secs_per_doc"),
            "문서당 탐지 시간(초)",
            e.seconds_total / e.docs.max(1) as f64,
        ),
    ];
    // 탐지기가 모르는 정답 라벨 — 재현율 묶음에 넣지 않고 참고로만
    let unknown = e.group_rate(&[pii_core::Label::SexLife]);
    if unknown.n > 0 {
        out.push(Metric::rate(
            &format!("{exp}.unknown_label_covered"),
            "탐지기가 모르는 라벨(SEX_LIFE)이 다른 탐지에 덮인 비율",
            &unknown,
            Goal::Report,
            0.0,
        ));
    }
    out
}

/// 텍스트 실험 실패 유형: 라벨별 누락, 하드네거티브 종류별 오탐
pub fn text_failures(e: &TextEval) -> BTreeMap<String, usize> {
    let mut f = BTreeMap::new();
    for (l, (k, n)) in &e.recall {
        if n > k {
            f.insert(format!("누락:{l}"), n - k);
        }
    }
    for (kind, (k, _)) in &e.negatives {
        if *k > 0 {
            f.insert(format!("오탐:{kind}"), *k);
        }
    }
    f
}

/// 이미지 실험 합계
pub fn sum_stats<'a>(it: impl Iterator<Item = &'a DiffStats>) -> DiffStats {
    let mut s = DiffStats::default();
    for d in it {
        s.docs += d.docs;
        s.pages += d.pages;
        s.spans += d.spans;
        s.leaked += d.leaked;
        s.value_in_text += d.value_in_text;
        s.sign_boxes += d.sign_boxes;
        s.sign_covered += d.sign_covered;
        s.employer_sign_boxes += d.employer_sign_boxes;
        s.employer_sign_masked += d.employer_sign_masked;
        s.negatives += d.negatives;
        s.negatives_masked += d.negatives_masked;
        for (k, v) in &d.by_label {
            let e = s.by_label.entry(k.clone()).or_default();
            e.0 += v.0;
            e.1 += v.1;
        }
        for (k, v) in &d.failures {
            *s.failures.entry(k.clone()).or_default() += v;
        }
        s.page_secs.extend(&d.page_secs);
        s.doc_secs.extend(&d.doc_secs);
        s.extract_secs += d.extract_secs;
        s.detect_secs += d.detect_secs;
        s.mask_secs += d.mask_secs;
        s.slm_calls += d.slm_calls;
    }
    s
}

/// 이미지 실험 지표 (난이도별 통계에서)
pub fn image_metrics(exp: &str, d: &BTreeMap<String, DiffStats>) -> Vec<Metric> {
    let pick = |ks: &[&str]| sum_stats(ks.iter().filter_map(|k| d.get(*k)));
    let d02 = pick(&["D0", "D1", "D2"]);
    let d12 = pick(&["D1", "D2"]);
    let d34 = pick(&["D3", "D4"]);
    let all = sum_stats(d.values());
    let covered = |s: &DiffStats| {
        let (mut leak, mut n) = (0, 0);
        for l in STRUCTURED {
            if let Some((a, b)) = s.by_label.get(&label_key(l)) {
                leak += a;
                n += b;
            }
        }
        Rate::new(n - leak, n)
    };
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let mut m = vec![
        Metric::rate(
            &format!("{exp}.leak_rate_d02"),
            "유출률 D0~D2 (주 지표)",
            &Rate::new(d02.leaked, d02.spans),
            Goal::AtMost,
            0.01,
        ),
        Metric::rate(
            &format!("{exp}.leak_rate_d34"),
            "유출률 D3~D4",
            &Rate::new(d34.leaked, d34.spans),
            Goal::Report,
            0.0,
        ),
        Metric::rate(
            &format!("{exp}.structured_recall_d12"),
            "정형 PII 재현율 (이미지 D1~D2)",
            &covered(&d12),
            Goal::AtLeast,
            0.95,
        ),
        Metric::rate(
            &format!("{exp}.signature_coverage"),
            "서명란 커버리지",
            &Rate::new(all.sign_covered, all.sign_boxes),
            Goal::AtLeast,
            0.95,
        ),
        Metric::rate(
            &format!("{exp}.fp_rate"),
            "하드네거티브 오탐률 (이미지·PDF)",
            &Rate::new(all.negatives_masked, all.negatives),
            Goal::AtMost,
            0.05,
        ),
        Metric::rate(
            &format!("{exp}.value_in_text"),
            "PII 값 인식률 (OCR·PDF 텍스트)",
            &Rate::new(all.value_in_text, all.spans),
            Goal::Report,
            0.0,
        ),
        Metric::plain(
            &format!("{exp}.secs_per_page"),
            "쪽당 평균 시간(초)",
            mean(&all.page_secs),
        ),
    ];
    for (k, s) in d {
        m.push(Metric::rate(
            &format!("{exp}.leak_rate_{}", k.to_lowercase()),
            &format!("유출률 {k}"),
            &Rate::new(s.leaked, s.spans),
            Goal::Report,
            0.0,
        ));
    }
    m
}

/// 이미지 실험 실패 유형: 전체 + 난이도별
pub fn image_failures(d: &BTreeMap<String, DiffStats>) -> BTreeMap<String, usize> {
    let mut f = BTreeMap::new();
    for (k, s) in d {
        for (t, n) in &s.failures {
            *f.entry(t.clone()).or_default() += n;
            f.insert(format!("{k}:{t}"), *n);
        }
    }
    f
}

/// `results/runs/<실행ID>/`에 저장
pub fn save(
    root: &Path,
    rec: &RunRecord,
    failures: &BTreeMap<String, usize>,
) -> std::io::Result<PathBuf> {
    let dir = root.join("results/runs").join(&rec.run_id);
    fs::create_dir_all(&dir)?;
    let to_io = |e: serde_json::Error| std::io::Error::other(e.to_string());
    fs::write(
        dir.join("metrics.json"),
        serde_json::to_vec_pretty(rec).map_err(to_io)?,
    )?;
    fs::write(
        dir.join("failures.json"),
        serde_json::to_vec_pretty(failures).map_err(to_io)?,
    )?;
    Ok(dir)
}

/// 저장된 실행 기록 전부 (실행 ID 순)
pub fn load_all(root: &Path) -> Vec<(RunRecord, BTreeMap<String, usize>)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root.join("results/runs")) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        let rec: Option<RunRecord> = fs::read_to_string(p.join("metrics.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        let fails: BTreeMap<String, usize> = fs::read_to_string(p.join("failures.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if let Some(r) = rec {
            out.push((r, fails));
        }
    }
    out.sort_by(|a, b| a.0.run_id.cmp(&b.0.run_id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_follow_report_rule() {
        assert_eq!(verdict(&Rate::new(990, 1000), Goal::AtLeast, 0.99), "가능");
        assert_eq!(
            verdict(&Rate::new(50, 50), Goal::AtLeast, 0.99),
            "조건부 가능"
        );
        assert_eq!(verdict(&Rate::new(900, 1000), Goal::AtLeast, 0.99), "불가");
        assert_eq!(verdict(&Rate::new(68, 599), Goal::AtMost, 0.01), "불가");
    }

    #[test]
    fn kst_date_is_well_formed() {
        let (d, t) = now_kst();
        assert_eq!(d.len(), 10);
        assert_eq!(t.len(), 6);
        assert!(d.starts_with("20"));
    }

    #[test]
    fn set_info_reads_seed() {
        assert_eq!(
            set_info(Path::new("data/eval_s42")),
            (Some(42), "평가용".into())
        );
        assert_eq!(
            set_info(Path::new("data/dev2_s45")),
            (Some(45), "개발용".into())
        );
    }
}
