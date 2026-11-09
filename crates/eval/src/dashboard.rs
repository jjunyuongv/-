//! 로컬 관리자 페이지 (`pii-mask eval dashboard` → `results/dashboard/index.html`)
//!
//! - 서버 없이 파일 하나로 연다. 외부 요청 금지 (CSP `default-src 'none'`, 글꼴·스크립트 없음, 차트는 인라인 SVG)
//! - 입력은 실행 기록(`results/runs/`)·결과 JSON의 지표·건수와 `docs/issues.md`의 제목·상태뿐이다.
//!   평가용 세트의 문서 ID·합성 값은 입력에 없으므로 페이지에도 나올 수 없다
//! - 문서 이미지는 개발용 세트의 확인용 오버레이(`data/masked_dev*/`)만, 상대 경로로 건다. 평가용은 넣지 않는다
//! - 처음 보는 서식 시험 실행(세트 이름에 `holdout`)은 본 판정 카드·추이에 섞지 않고 전용 칸에만 표시한다

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::image::DiffStats;
use crate::runlog::{Goal, Metric, RunRecord, load_all, now_kst};
use crate::{NAME_ADDRESS, Rate, SENSITIVE, STRUCTURED, TextEval};

type Run = (RunRecord, BTreeMap<String, usize>);
type Counts = Vec<(String, usize)>;

/// 13단계 첫 실행·재측정 (추이 표시 이름과 카드의 참고값 기준)
const RUN_FIRST: &str = "20261108-190000-e5";
const RUN_13: &str = "20261108-193000-e5";
const VERDICT_DATE: &str = "2026-11-09";

/// 처음 보는 서식 (템플릿 id, 표시 이름)
pub const HOLDOUT_FORMS: [(&str, &str); 3] = [
    ("holdout_table_altkeys", "키 이름 변경"),
    ("holdout_prose_gap_eul", "문장형"),
    ("holdout_shuffled_identity_last", "순서 섞임"),
];

/// 이미지 결과 JSON (`run-images --out`)
#[derive(Debug, Default, serde::Deserialize)]
pub struct E5Data {
    #[serde(default)]
    pub conditions: BTreeMap<String, String>,
    #[serde(default)]
    pub model_load_secs: f64,
    pub peak_working_set_mb_self: Option<f64>,
    pub peak_working_set_mb_llama_server: Option<f64>,
    #[serde(default)]
    pub by_difficulty: BTreeMap<String, DiffStats>,
    #[serde(default)]
    pub by_layout: BTreeMap<String, DiffStats>,
}

/// 처음 보는 서식 시험 실험 하나
pub struct HoldoutExp {
    pub exp: String,
    pub overall: TextEval,
    pub by_form: BTreeMap<String, TextEval>,
    /// 같은 실험의 기존 양식 결과 (`text_s43`)
    pub baseline: Option<TextEval>,
}

/// 실행 기록 밖의 자료 (결과 JSON·오버레이 목록)
#[derive(Default)]
pub struct Extras {
    pub e5: Option<E5Data>,
    pub layouts: BTreeMap<String, DiffStats>,
    pub text: Option<TextEval>,
    pub holdout: Vec<HoldoutExp>,
    /// (난이도, 페이지 기준 상대 경로들)
    pub gallery: Vec<(String, Vec<String>)>,
}

const CSS: &str = r#"
:root { color-scheme: light; --surface: #f4f3ef; --card: #fcfcfb; --line: #e4e3dd; --text: #0b0b0b;
  --text-2: #52514e; --s1: #2a78d6; --s2: #eb6834; --s3: #1baf7a; --good: #0ca30c; --warn: #fab219; --crit: #d03b3b; }
@media (prefers-color-scheme: dark) { :root { color-scheme: dark; --surface: #121211; --card: #1a1a19; --line: #33322f;
  --text: #f0efec; --text-2: #c3c2b7; --s1: #3987e5; --s2: #d95926; --s3: #199e70; } }
* { box-sizing: border-box; }
body { margin: 0; background: var(--surface); color: var(--text);
  font: 15px/1.55 "Malgun Gothic", "Apple SD Gothic Neo", "Noto Sans KR", sans-serif; }
main { max-width: 1180px; margin: 0 auto; padding: 24px 16px 56px; }
h1 { font-size: 24px; margin: 0 0 4px; }
h2 { font-size: 18px; margin: 36px 0 12px; }
p.sub, .muted { color: var(--text-2); }
p.sub { margin: 0; font-size: 14px; }
.cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 250px), 1fr)); gap: 12px; margin-top: 18px; }
.card { background: var(--card); border: 1px solid var(--line); border-radius: 8px; padding: 16px; min-width: 0; }
.card h3 { font-size: 15px; margin: 0 0 8px; }
.chip { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; font-weight: 700; }
.chip i { font-style: normal; display: inline-grid; place-items: center; width: 18px; height: 18px; border-radius: 50%;
  color: #fff; font-size: 12px; line-height: 1; }
.good i { background: var(--good); } .warn i { background: var(--warn); color: #0b0b0b; } .crit i { background: var(--crit); }
.pend i { background: var(--text-2); }
.hero { font-size: 32px; font-weight: 700; margin: 10px 0 0; line-height: 1.2; }
.hero small { font-size: 13px; font-weight: 400; color: var(--text-2); }
.card p { margin: 6px 0 0; font-size: 13px; color: var(--text-2); }
.card ul { margin: 8px 0 0; padding: 0; list-style: none; font-size: 13px; color: var(--text-2); }
.card li { display: flex; justify-content: space-between; gap: 8px; }
.card li b { color: var(--text); font-weight: 600; white-space: nowrap; }
.legend { display: flex; flex-wrap: wrap; gap: 16px; font-size: 13px; color: var(--text-2); margin: 0 0 10px; }
.legend span { display: inline-flex; align-items: center; gap: 6px; }
.charts { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 440px), 1fr)); gap: 12px; }
svg { display: block; width: 100%; height: auto; }
svg text { font-family: inherit; font-size: 12px; fill: var(--text-2); }
svg text.v { fill: var(--text); font-weight: 700; }
.grid { stroke: var(--line); stroke-width: 1; }
.target { stroke: var(--text-2); stroke-width: 1; }
.ln { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
.dot { stroke: var(--card); stroke-width: 2; }
.hit { fill: transparent; }
.scroll { overflow-x: auto; background: var(--card); border: 1px solid var(--line); border-radius: 8px; }
table { border-collapse: collapse; width: 100%; font-size: 13px; }
th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid var(--line); vertical-align: top; }
th { color: var(--text-2); font-weight: 600; white-space: nowrap; }
tr:last-child td { border-bottom: 0; }
td.num { white-space: nowrap; font-variant-numeric: tabular-nums; }
code { font-family: Consolas, monospace; font-size: 12px; }
h3.t { font-size: 14px; margin: 16px 0 8px; color: var(--text-2); font-weight: 600; }
.bars { display: grid; gap: 8px; }
.bar-row { display: grid; grid-template-columns: minmax(0, 11em) 1fr 3em; align-items: center; gap: 8px; font-size: 13px; }
.bar-row span.l { color: var(--text-2); overflow-wrap: anywhere; }
.bar-track { display: block; height: 14px; }
.bar { display: block; height: 14px; min-width: 2px; background: var(--s1); border-radius: 0 4px 4px 0; }
.bar-row b { font-weight: 600; text-align: right; font-variant-numeric: tabular-nums; }
.gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 170px), 1fr)); gap: 10px; }
.gallery a { display: block; color: var(--text-2); font-size: 12px; text-decoration: none; }
.gallery img { display: block; width: 100%; height: 230px; object-fit: cover; object-position: top; background: var(--line);
  border: 1px solid var(--line); border-radius: 6px; }
footer { margin-top: 32px; font-size: 13px; color: var(--text-2); }
"#;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn is_holdout(r: &RunRecord) -> bool {
    r.set.contains("holdout")
}

fn metric<'a>(r: &'a RunRecord, suffix: &str) -> Option<&'a Metric> {
    let key = format!("{}.{suffix}", r.exp);
    r.metrics.iter().find(|m| m.key == key)
}

fn fmt_rate(r: &Rate) -> String {
    if r.n == 0 {
        "-".into()
    } else {
        format!("{:.1}%", r.p * 100.0)
    }
}

fn short_pct(m: Option<&Metric>) -> String {
    m.filter(|x| x.n > 0)
        .map_or("-".into(), |x| format!("{:.1}%", x.value * 100.0))
}

/// 난이도들을 합친 유출률
fn leak_of(r: &RunRecord, ds: &[&str]) -> Rate {
    let (mut k, mut n) = (0, 0);
    for d in ds {
        if let Some(m) = metric(r, &format!("leak_rate_{d}")) {
            k += m.k;
            n += m.n;
        }
    }
    Rate::new(k, n)
}

/// 처리 경로의 유출·서명란 실패 유형 상위 3 (건수만, 오탐 제외)
fn top_failures(f: &BTreeMap<String, usize>, ds: &[&str]) -> Vec<(String, usize)> {
    let mut sum: BTreeMap<String, usize> = BTreeMap::new();
    for (k, n) in f {
        let Some((d, kind)) = k.split_once(':') else {
            continue;
        };
        if ds.iter().any(|x| x.eq_ignore_ascii_case(d)) && !kind.starts_with("오탐") {
            *sum.entry(kind.to_string()).or_default() += n;
        }
    }
    let mut v: Vec<_> = sum.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    v.truncate(3);
    v
}

fn chip(class: &str, icon: &str, text: &str) -> String {
    format!(
        r#"<span class="chip {class}"><i aria-hidden="true">{icon}</i>{}</span>"#,
        esc(text)
    )
}

fn verdict_chip(v: &str) -> String {
    match v {
        "가능" => chip("good", "✓", v),
        "조건부 가능" => chip("warn", "!", v),
        "불가" => chip("crit", "✕", v),
        _ => chip("pend", "–", v),
    }
}

fn path_card(
    w: &mut String,
    title: &str,
    badge: &str,
    ds: &[&str],
    latest: &Run,
    baseline: Option<&Run>,
    condition: &str,
) {
    let rate = leak_of(&latest.0, ds);
    let _ = write!(
        w,
        r#"<article class="card"><h3>{}</h3>{badge}<div class="hero">{}<small> 유출률</small></div><p>{}/{}건 · 95% 구간 {:.1}~{:.1}% · 목표 ≤ 1%</p>"#,
        esc(title),
        fmt_rate(&rate),
        rate.k,
        rate.n,
        rate.lo * 100.0,
        rate.hi * 100.0
    );
    if let Some(b) = baseline {
        let _ = write!(
            w,
            "<p>13단계 판정: {} (다른 세트, 참고)</p>",
            fmt_rate(&leak_of(&b.0, ds))
        );
    }
    let _ = write!(w, "<p>{}</p>", esc(condition));
    let tops = top_failures(&latest.1, ds);
    if !tops.is_empty() {
        let _ = write!(w, "<ul>");
        for (k, n) in tops {
            let _ = write!(w, "<li><span>{}</span><b>{n}건</b></li>", esc(&k));
        }
        let _ = write!(w, "</ul>");
    }
    let _ = write!(w, "</article>");
}

fn text_card(w: &mut String, r: &RunRecord) {
    let _ = write!(
        w,
        r#"<article class="card"><h3>텍스트 탐지 (E4b)</h3>{}<div class="hero">{}<small> 민감정보</small></div><p>목표 ≥ 90% · 텍스트 {}건 · <code>{}</code></p><ul>"#,
        chip("good", "✓", "가능"),
        short_pct(metric(r, "sensitive_recall")),
        r.docs,
        esc(&r.run_id)
    );
    for (name, s) in [
        ("정형 번호", "structured_recall"),
        ("이름·주소", "name_address_recall"),
        ("오탐 (하드네거티브)", "fp_rate"),
    ] {
        let _ = write!(
            w,
            "<li><span>{name}</span><b>{}</b></li>",
            short_pct(metric(r, s))
        );
    }
    let _ = write!(w, "</ul></article>");
}

/// 추이 x축에 쓸 평가용 실행 이름
fn eval_tick(r: &RunRecord) -> String {
    match r.run_id.as_str() {
        RUN_FIRST => "첫 실행".into(),
        RUN_13 => "재측정".into(),
        _ => r.set.rsplit('/').next().unwrap_or(&r.set).to_string(),
    }
}

fn series_of(r: &RunRecord) -> usize {
    if r.set_kind == "평가용" {
        0
    } else if r.set.contains("dev2") {
        2
    } else {
        1
    }
}

const SERIES: [(&str, &str); 3] = [
    ("평가용 세트", "var(--s1)"),
    ("개발용 1차 (시드 41)", "var(--s2)"),
    ("개발용 2차 (시드 45)", "var(--s3)"),
];

fn marker(series: usize, x: f64, y: f64, color: &str) -> String {
    if series == 2 {
        format!(
            r#"<rect class="dot" x="{:.1}" y="{:.1}" width="9" height="9" fill="{color}"/>"#,
            x - 4.5,
            y - 4.5
        )
    } else {
        format!(r#"<circle class="dot" cx="{x:.1}" cy="{y:.1}" r="4.5" fill="{color}"/>"#)
    }
}

/// 지표 하나의 추이 (실행 순서가 x축, 비율 y축 하나)
fn chart(w: &mut String, title: &str, suffix: &str, runs: &[&Run]) {
    const W: f64 = 540.0;
    const H: f64 = 230.0;
    const L: f64 = 44.0;
    const R: f64 = 78.0;
    const T: f64 = 22.0;
    const B: f64 = 30.0;
    let values: Vec<f64> = runs
        .iter()
        .filter_map(|(r, _)| metric(r, suffix))
        .filter(|m| m.n > 0)
        .map(|m| m.value)
        .collect();
    let target = runs
        .iter()
        .filter_map(|(r, _)| metric(r, suffix))
        .find(|m| m.goal != Goal::Report);
    let top = values
        .iter()
        .copied()
        .chain(target.map(|m| m.target))
        .fold(0.0_f64, f64::max);
    // 5%p 단위로 올림, 비율이라 100%를 넘지 않게
    let ymax = (((top * 1.2) * 20.0).ceil() / 20.0).clamp(0.05, 1.0);
    let n = runs.len().max(2);
    let px = |i: usize| L + (W - L - R) * i as f64 / (n - 1) as f64;
    let py = |v: f64| T + (H - T - B) * (1.0 - v / ymax);
    let _ = write!(
        w,
        r#"<figure class="card" style="margin:0"><h3>{}</h3><svg viewBox="0 0 {W} {H}" role="img" aria-label="{}">"#,
        esc(title),
        esc(title)
    );
    // 눈금 간격: 칸이 4개를 넘지 않는 가장 작은 값
    let step = [0.05, 0.1, 0.2, 0.25]
        .into_iter()
        .find(|s| ymax / s <= 4.001)
        .unwrap_or(0.25);
    let ticks = (ymax / step).round() as u32;
    for i in 0..=ticks {
        let v = step * f64::from(i);
        let y = py(v);
        let _ = write!(
            w,
            r#"<line class="grid" x1="{L}" x2="{:.1}" y1="{y:.1}" y2="{y:.1}"/><text x="{:.1}" y="{:.1}" text-anchor="end">{:.0}%</text>"#,
            W - R,
            L - 6.0,
            y + 4.0,
            v * 100.0
        );
    }
    if let Some(m) = target {
        let y = py(m.target);
        let _ = write!(
            w,
            r#"<line class="target" x1="{L}" x2="{:.1}" y1="{y:.1}" y2="{y:.1}"/><text x="{:.1}" y="{:.1}">목표 {}</text>"#,
            W - R,
            W - R + 6.0,
            y + 4.0,
            esc(&m.fmt_target())
        );
    }
    // 뒤에 그린 것이 위에 오므로 평가용을 마지막에
    for s in [1usize, 2, 0] {
        let (_, color) = SERIES[s];
        let pts: Vec<(usize, &Run, &Metric)> = runs
            .iter()
            .enumerate()
            .filter(|(_, run)| series_of(&run.0) == s)
            .filter_map(|(i, run)| {
                metric(&run.0, suffix)
                    .filter(|m| m.n > 0)
                    .map(|m| (i, *run, m))
            })
            .collect();
        let path: Vec<String> = pts
            .iter()
            .map(|(i, _, m)| format!("{:.1},{:.1}", px(*i), py(m.value)))
            .collect();
        let _ = write!(
            w,
            r#"<polyline class="ln" stroke="{color}" points="{}"/>"#,
            path.join(" ")
        );
        for (i, run, m) in &pts {
            let (x, y) = (px(*i), py(m.value));
            let _ = write!(w, "{}", marker(s, x, y, color));
            if s == 0 {
                let anchor = if *i + 1 == n { "end" } else { "middle" };
                // 값 글자가 목표선에 걸리면 점 아래로
                let above = (y - 10.0).max(12.0);
                let hits_target =
                    target.is_some_and(|t| (py(t.target) - (above - 4.0)).abs() < 9.0);
                let label_y = if hits_target { y + 19.0 } else { above };
                let _ = write!(
                    w,
                    r#"<text class="v" x="{x:.1}" y="{:.1}" text-anchor="{anchor}">{:.1}%</text><text x="{x:.1}" y="{:.1}" text-anchor="{anchor}">{}</text>"#,
                    label_y,
                    m.value * 100.0,
                    H - 10.0,
                    esc(&eval_tick(&run.0))
                );
            }
            // 표시 영역보다 넓은 투명 원 — 마우스를 올리면 실행 정보
            let _ = write!(
                w,
                r#"<circle class="hit" cx="{x:.1}" cy="{y:.1}" r="11"><title>{} · {} · {}</title></circle>"#,
                esc(&run.0.run_id),
                esc(SERIES[s].0),
                esc(&m.fmt_value())
            );
        }
    }
    let _ = write!(w, "</svg></figure>");
}

/// `docs/issues.md`의 (제목, 상태)
fn parse_issue_status(md: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in md.lines() {
        if let Some(t) = line.strip_prefix("## ") {
            if t.starts_with("ISSUE-") {
                out.push((t.to_string(), String::new()));
            }
        } else if let Some(s) = line.strip_prefix("- 상태:")
            && let Some(last) = out.last_mut()
            && last.1.is_empty()
        {
            last.1 = s.trim().replace("**", "").replace('`', "");
        }
    }
    out
}

fn kn(r: &Rate) -> String {
    if r.n == 0 {
        "-".into()
    } else {
        format!("{:.1}% ({}/{})", r.p * 100.0, r.k, r.n)
    }
}

fn holdout_row(w: &mut String, exp: &str, form: &str, e: &TextEval) {
    let _ = write!(
        w,
        r#"<tr><td>{}</td><td>{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td></tr>"#,
        esc(exp),
        esc(form),
        e.docs,
        kn(&e.group_rate(&STRUCTURED)),
        kn(&e.group_rate(&NAME_ADDRESS)),
        kn(&e.group_rate(&SENSITIVE)),
        kn(&e.negative_rate()),
        kn(&e.group_rate(&[pii_core::Label::SexLife]))
    );
}

fn holdout_section(w: &mut String, x: &Extras) {
    let _ = write!(w, "<h2>처음 보는 서식 시험</h2>");
    if x.holdout.is_empty() {
        let _ = write!(
            w,
            r#"<article class="card">{}<p>시험을 아직 하지 않았다.</p></article>"#,
            chip("pend", "–", "시험 진행 중")
        );
        return;
    }
    let _ = write!(
        w,
        r#"<p class="sub" style="margin-bottom:10px">{} 탐지 코드를 보지 않은 별도 세션이 쓴 서식 3종 · 텍스트 30건(시드 48) · 탐지 코드 수정 없이 한 번 실행 · 표본이 작아 참고용. 맨 오른쪽 열은 라벨 체계에 없어 재현율에 넣지 않은 값이다.</p><div class="scroll"><table><thead><tr><th>실험</th><th>서식</th><th>문서</th><th>정형</th><th>이름·주소</th><th>민감정보</th><th>오탐</th><th>탐지기가 모르는 라벨</th></tr></thead><tbody>"#,
        chip("pend", "–", "시험 완료")
    );
    for h in &x.holdout {
        let name = h.exp.to_uppercase();
        for (id, label) in HOLDOUT_FORMS {
            if let Some(e) = h.by_form.get(id) {
                holdout_row(w, &name, label, e);
            }
        }
        holdout_row(w, &name, "처음 보는 서식 전체", &h.overall);
        if let Some(b) = &h.baseline {
            holdout_row(w, &name, "기존 양식 (text_s43)", b);
        }
    }
    let _ = write!(w, "</tbody></table></div>");
}

fn targets_table(w: &mut String, title_runs: &[&RunRecord]) {
    let _ = write!(
        w,
        r#"<div class="scroll"><table><thead><tr><th>지표</th><th>값 (95% 구간)</th><th>목표</th><th>판정</th><th>실행 ID</th><th>세트</th></tr></thead><tbody>"#
    );
    for r in title_runs {
        for m in r.metrics.iter().filter(|m| m.goal != Goal::Report) {
            let _ = write!(
                w,
                r#"<tr><td>{}</td><td class="num">{}</td><td class="num">{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>"#,
                esc(&m.name),
                esc(&m.fmt_value()),
                esc(&m.fmt_target()),
                verdict_chip(&m.verdict),
                esc(&r.run_id),
                esc(&r.set)
            );
        }
    }
    let _ = write!(w, "</tbody></table></div>");
}

fn run_summary(r: &RunRecord) -> String {
    if r.exp == "e5" {
        format!(
            "유출 D0~D2 {} · D3~D4 {} · 서명란 {} · 오탐 {}",
            short_pct(metric(r, "leak_rate_d02")),
            short_pct(metric(r, "leak_rate_d34")),
            short_pct(metric(r, "signature_coverage")),
            short_pct(metric(r, "fp_rate"))
        )
    } else {
        format!(
            "정형 {} · 이름·주소 {} · 민감 {} · 오탐 {}",
            short_pct(metric(r, "structured_recall")),
            short_pct(metric(r, "name_address_recall")),
            short_pct(metric(r, "sensitive_recall")),
            short_pct(metric(r, "fp_rate"))
        )
    }
}

fn merged<'a>(it: impl Iterator<Item = &'a DiffStats>) -> DiffStats {
    let mut s = DiffStats::default();
    for d in it {
        s.docs += d.docs;
        s.pages += d.pages;
        s.spans += d.spans;
        s.leaked += d.leaked;
        s.value_in_text += d.value_in_text;
        s.sign_boxes += d.sign_boxes;
        s.sign_covered += d.sign_covered;
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
    }
    s
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn secs(v: &[f64]) -> String {
    mean(v).map_or("-".into(), |x| format!("{x:.1}초"))
}

fn stats_table(w: &mut String, head: &str, rows: &[(String, &DiffStats)]) {
    let _ = write!(
        w,
        r#"<div class="scroll"><table><thead><tr><th>{}</th><th>문서</th><th>쪽</th><th>유출률</th><th>PII 값 인식</th><th>서명란 커버리지</th><th>오탐</th><th>쪽당 시간</th></tr></thead><tbody>"#,
        esc(head)
    );
    for (name, s) in rows {
        let _ = write!(
            w,
            r#"<tr><td>{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td></tr>"#,
            esc(name),
            s.docs,
            s.pages,
            kn(&Rate::new(s.leaked, s.spans)),
            kn(&Rate::new(s.value_in_text, s.spans)),
            kn(&Rate::new(s.sign_covered, s.sign_boxes)),
            kn(&Rate::new(s.negatives_masked, s.negatives)),
            secs(&s.page_secs)
        );
    }
    let _ = write!(w, "</tbody></table></div>");
}

fn layout_name(l: &str) -> String {
    let d = match l {
        "A" => "표준",
        "B" => "표준 + 회사 항목",
        "C" => "외국인",
        "D" => "연소근로자",
        "E" => "단시간",
        "F" => "건설일용",
        _ => "",
    };
    format!("{l} {d}")
}

fn results_section(w: &mut String, x: &Extras) {
    let Some(e) = &x.e5 else { return };
    let _ = write!(w, "<h2>처리 경로·레이아웃·라벨별 결과</h2>");
    let d = &e.by_difficulty;
    let pick = |ks: &[&str]| merged(ks.iter().filter_map(|k| d.get(*k)));
    let (d12, d34, all) = (pick(&["D1", "D2"]), pick(&["D3", "D4"]), merged(d.values()));
    let mut rows: Vec<(String, &DiffStats)> = d.iter().map(|(k, s)| (k.clone(), s)).collect();
    rows.push(("D1~D2 스캔".into(), &d12));
    rows.push(("D3~D4 사진·팩스".into(), &d34));
    rows.push(("전체".into(), &all));
    let _ = write!(w, r#"<h3 class="t">처리 경로</h3>"#);
    stats_table(w, "경로", &rows);
    let _ = write!(w, r#"<h3 class="t">레이아웃</h3>"#);
    if x.layouts.is_empty() {
        let _ = write!(w, r#"<p class="sub">레이아웃별 집계가 없다.</p>"#);
    } else {
        let rows: Vec<(String, &DiffStats)> =
            x.layouts.iter().map(|(k, s)| (layout_name(k), s)).collect();
        stats_table(w, "레이아웃", &rows);
    }
    let _ = write!(
        w,
        r#"<h3 class="t">라벨</h3><div class="scroll"><table><thead><tr><th>라벨</th><th>이미지·PDF 유출</th><th>텍스트 재현율 (E4b)</th></tr></thead><tbody>"#
    );
    let mut labels: Vec<&String> = all.by_label.keys().collect();
    if let Some(t) = &x.text {
        labels.extend(t.recall.keys());
    }
    labels.sort();
    labels.dedup();
    for l in labels {
        let img = all
            .by_label
            .get(l)
            .map_or("-".into(), |(a, b)| kn(&Rate::new(*a, *b)));
        let txt = x
            .text
            .as_ref()
            .and_then(|t| t.recall.get(l))
            .map_or("-".into(), |(a, b)| kn(&Rate::new(*a, *b)));
        let _ = write!(
            w,
            r#"<tr><td>{}</td><td class="num">{img}</td><td class="num">{txt}</td></tr>"#,
            esc(l)
        );
    }
    let _ = write!(w, "</tbody></table></div>");
}

fn bar_panel(w: &mut String, title: &str, items: &[(String, usize)]) {
    let total: usize = items.iter().map(|x| x.1).sum();
    let max = items.iter().map(|x| x.1).max().unwrap_or(1).max(1);
    let _ = write!(
        w,
        r#"<figure class="card" style="margin:0"><h3>{} · {total}건</h3><div class="bars">"#,
        esc(title)
    );
    if items.is_empty() {
        let _ = write!(w, r#"<p>없음</p>"#);
    }
    for (k, n) in items {
        let _ = write!(
            w,
            r#"<div class="bar-row"><span class="l">{}</span><span class="bar-track"><span class="bar" style="width:{:.1}%"></span></span><b>{n}</b></div>"#,
            esc(k),
            *n as f64 * 100.0 / max as f64
        );
    }
    let _ = write!(w, "</div></figure>");
}

/// 실패 유형을 유출·서명란·오탐 종류·오탐 영역으로 나눈다 (건수 많은 순)
fn failure_groups(f: &BTreeMap<String, usize>) -> (Counts, Counts, Counts, Counts) {
    let (mut leak, mut sign, mut fp, mut src) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (k, n) in f {
        if let Some(s) = k.strip_prefix("오탐:") {
            src.push((s.to_string(), *n));
        } else if let Some(s) = k.strip_prefix("오탐 ") {
            fp.push((s.to_string(), *n));
        } else if k.starts_with("서명란") {
            sign.push((k.clone(), *n));
        } else {
            leak.push((k.clone(), *n));
        }
    }
    for v in [&mut leak, &mut sign, &mut fp, &mut src] {
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
    }
    (leak, sign, fp, src)
}

fn failures_section(w: &mut String, x: &Extras) {
    let Some(e) = &x.e5 else { return };
    let all = merged(e.by_difficulty.values());
    let (leak, sign, fp, src) = failure_groups(&all.failures);
    let _ = write!(
        w,
        r#"<h2>실패 유형 (최신 평가, 건수만)</h2><div class="charts">"#
    );
    bar_panel(w, "유출", &leak);
    bar_panel(w, "서명란", &sign);
    bar_panel(w, "오탐 종류", &fp);
    bar_panel(w, "오탐을 덮은 영역", &src);
    let _ = write!(w, "</div>");
}

/// 지표 변화의 좋고 나쁨 (낮을수록 좋은 지표인지)
fn lower_is_better(m: &Metric) -> Option<bool> {
    match m.goal {
        Goal::AtMost => Some(true),
        Goal::AtLeast => Some(false),
        Goal::Report if m.key.contains("leak_rate") || m.key.contains("secs_per") => Some(true),
        Goal::Report => None,
    }
}

fn diff_rows(w: &mut String, title: &str, prev: &RunRecord, cur: &RunRecord) {
    for m in cur.metrics.iter().filter(|m| {
        m.goal != Goal::Report || m.key.ends_with("leak_rate_d34") || m.key.contains("secs_per")
    }) {
        let suffix = m.key.split_once('.').map_or(m.key.as_str(), |x| x.1);
        let Some(o) = metric(prev, suffix) else {
            continue;
        };
        let is_secs = m.key.contains("secs_per");
        let (a, b, delta) = if is_secs {
            (
                format!("{:.1}초", o.value),
                format!("{:.1}초", m.value),
                format!("{:+.1}초", m.value - o.value),
            )
        } else {
            (
                short_pct(Some(o)),
                short_pct(Some(m)),
                format!("{:+.1}%p", (m.value - o.value) * 100.0),
            )
        };
        let eps = if is_secs { 0.05 } else { 0.0005 };
        let change = if (m.value - o.value).abs() < eps {
            chip("pend", "–", "같음")
        } else {
            match lower_is_better(m).map(|low| (m.value < o.value) == low) {
                Some(true) => chip("good", "✓", "개선"),
                Some(false) => chip("crit", "✕", "악화"),
                None => chip("pend", "–", "변화"),
            }
        };
        let _ = write!(
            w,
            r#"<tr><td>{}</td><td>{}</td><td class="num">{a}</td><td class="num">{b}</td><td class="num">{delta}</td><td>{change}</td></tr>"#,
            esc(title),
            esc(&m.name)
        );
    }
}

fn diff_section(w: &mut String, runs: &[&Run]) {
    let _ = write!(
        w,
        r#"<h2>최신 실행과 이전 실행의 차이</h2><div class="scroll"><table><thead><tr><th>비교</th><th>지표</th><th>이전</th><th>최신</th><th>차이</th><th>변화</th></tr></thead><tbody>"#
    );
    let of = |exp: &str, pred: &dyn Fn(&RunRecord) -> bool| -> Vec<&RunRecord> {
        runs.iter()
            .map(|(r, _)| r)
            .filter(|r| r.exp == exp && pred(r))
            .collect()
    };
    let last_two = |v: &[&'_ RunRecord]| -> Option<(RunRecord, RunRecord)> {
        match v {
            [.., a, b] => Some(((*a).clone(), (*b).clone())),
            _ => None,
        }
    };
    if let Some((a, b)) = last_two(&of("e5", &|r| r.set_kind == "평가용")) {
        let title = format!(
            "이미지 평가용: {} → {} (세트 다르면 참고용)",
            a.set.rsplit('/').next().unwrap_or(&a.set),
            b.set.rsplit('/').next().unwrap_or(&b.set)
        );
        diff_rows(w, &title, &a, &b);
    }
    for (name, set) in [("개발용 1차", "dev_s41"), ("개발용 2차", "dev2_s45")] {
        if let Some((a, b)) = last_two(&of("e5", &|r| r.set.ends_with(set))) {
            diff_rows(w, &format!("{name} (같은 세트)"), &a, &b);
        }
    }
    let full = |r: &RunRecord| r.docs >= 1000;
    if let (Some(a), Some(b)) = (of("e4", &full).last(), of("e4b", &full).last()) {
        diff_rows(w, "텍스트 1,000건: E4 → E4b", a, b);
    }
    let _ = write!(w, "</tbody></table></div>");
}

fn run_info_section(
    w: &mut String,
    latest: Option<&RunRecord>,
    x: &Extras,
    text: Option<&RunRecord>,
) {
    let _ = write!(
        w,
        r#"<h2>실행 정보</h2><div class="scroll"><table><thead><tr><th>항목</th><th>값</th></tr></thead><tbody>"#
    );
    let mut row = |k: &str, v: String| {
        let _ = write!(w, "<tr><td>{}</td><td>{}</td></tr>", esc(k), esc(&v));
    };
    if let Some(r) = latest {
        row(
            "이미지 세트",
            format!(
                "{} ({}, 시드 {}, 문서 {}건)",
                r.set,
                r.set_kind,
                r.seed.map_or("-".into(), |s| s.to_string()),
                r.docs
            ),
        );
        row("이미지 실행 ID", format!("{} ({})", r.run_id, r.date));
        row("설정 메모", r.note.clone());
    }
    if let Some(e) = &x.e5 {
        let all = merged(e.by_difficulty.values());
        row("쪽 수", format!("{}쪽", all.pages));
        row("쪽당 평균 시간", secs(&all.page_secs));
        row("문서당 평균 시간", secs(&all.doc_secs));
        row(
            "최대 메모리",
            format!(
                "앱 {:.1}GB + SLM {:.1}GB",
                e.peak_working_set_mb_self.unwrap_or(f64::NAN) / 1024.0,
                e.peak_working_set_mb_llama_server.unwrap_or(f64::NAN) / 1024.0
            ),
        );
        row("모델 로딩", format!("{:.1}초 (OCR·NER)", e.model_load_secs));
        for (k, v) in &e.conditions {
            row(
                &format!("설정: {k}"),
                v.replace("http://127.0.0.1:8089", "로컬 서버"),
            );
        }
    }
    if let Some(r) = text {
        row(
            "텍스트 세트",
            format!(
                "{} ({}, 시드 {}, 문서 {}건)",
                r.set,
                r.set_kind,
                r.seed.map_or("-".into(), |s| s.to_string()),
                r.docs
            ),
        );
        row("텍스트 실행 ID", format!("{} ({})", r.run_id, r.date));
        if let Some(m) = metric(r, "secs_per_doc") {
            row("텍스트 문서당 시간", format!("{:.1}초", m.value));
        }
    }
    let _ = write!(w, "</tbody></table></div>");
}

fn gallery_section(w: &mut String, x: &Extras) {
    let _ = write!(
        w,
        r#"<h2>개발용 세트 오버레이</h2><p class="sub" style="margin-bottom:10px">개발용 합성 데이터만 표시한다 (평가용 제외). 정답 박스 색: 가려짐 초록 · 유출 빨강 · 서명란 파랑/주황 · 오탐 보라. 그림을 누르면 원본 크기로 열린다.</p>"#
    );
    if x.gallery.is_empty() {
        let _ = write!(w, r#"<p class="sub">오버레이가 없다.</p>"#);
    }
    for (diff, files) in &x.gallery {
        let _ = write!(
            w,
            r#"<h3 class="t">{}</h3><div class="gallery">"#,
            esc(diff)
        );
        for f in files {
            let name = f.rsplit('/').next().unwrap_or(f);
            let _ = write!(
                w,
                r#"<a href="{0}"><img loading="lazy" src="{0}" alt="{1}">{1}</a>"#,
                esc(f),
                esc(name.trim_end_matches("_check.png"))
            );
        }
        let _ = write!(w, "</div>");
    }
}

/// 페이지 HTML
pub fn render(runs: &[Run], x: &Extras, issues_md: &str, generated: &str) -> String {
    let mut w = String::new();
    let _ = write!(
        w,
        r#"<!doctype html><html lang="ko"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src file:"><title>가려냄 관리자</title><style>{CSS}</style></head><body><main><h1>가려냄 관리자</h1><p class="sub">합성 데이터 평가 결과 · 숫자만 표시 · 이 기기에서만 열람 · 생성 {}</p>"#,
        esc(generated)
    );

    let main_runs: Vec<&Run> = runs.iter().filter(|(r, _)| !is_holdout(r)).collect();
    let latest_image = main_runs
        .iter()
        .rev()
        .find(|(r, _)| r.exp == "e5" && r.set_kind == "평가용")
        .copied();
    let latest_text = main_runs
        .iter()
        .rev()
        .find(|(r, _)| r.exp == "e4b" && r.set_kind == "평가용" && r.docs >= 1000)
        .map(|(r, _)| r);
    let baseline = main_runs.iter().find(|(r, _)| r.run_id == RUN_13).copied();

    let _ = write!(w, r#"<section class="cards">"#);
    if let Some(latest) = latest_image {
        path_card(
            &mut w,
            "D0 디지털 PDF",
            &chip("good", "✓", "자동 마스킹 가능"),
            &["d0"],
            latest,
            baseline,
            "서명란은 가린 결과를 한 번 확인 권장",
        );
        path_card(
            &mut w,
            "D1~D2 스캔",
            &chip("warn", "!", "자동 + 사람 검토 시 가능"),
            &["d1", "d2"],
            latest,
            baseline,
            "자동으로 가린 뒤 사람이 결과를 확인·보완",
        );
        path_card(
            &mut w,
            "D3~D4 사진·팩스",
            &chip("crit", "✕", "현재 불가"),
            &["d3", "d4"],
            latest,
            baseline,
            "재촬영·원본 요청 또는 사람이 직접 처리",
        );
    }
    if let Some(r) = latest_text {
        text_card(&mut w, r);
    }
    let _ = write!(w, "</section>");
    if let Some((r, _)) = latest_image {
        let _ = write!(
            w,
            r#"<p class="sub" style="margin-top:10px">처리 경로 판정은 {VERDICT_DATE} 결정 · 근거 실행 <code>{}</code> (세트 <code>{}</code>, 문서 {}건)</p>"#,
            esc(&r.run_id),
            esc(&r.set),
            r.docs
        );
    }

    if !x.holdout.is_empty() {
        let _ = write!(
            w,
            r#"<p class="sub" style="margin-top:4px">위 판정은 기존 양식·문장 풀 범위. 처음 보는 서식 결과는 바로 아래를 본다.</p>"#
        );
    }
    holdout_section(&mut w, x);

    // 추이
    let e5: Vec<&Run> = main_runs
        .iter()
        .filter(|(r, _)| r.exp == "e5")
        .copied()
        .collect();
    let _ = write!(
        w,
        r#"<h2>이미지·PDF 추이</h2><p class="sub" style="margin-bottom:10px">가로축은 실행 순서. 평가용 세트는 13단계 두 번(<code>eval_s42</code>)과 새 시드(<code>eval2_s46</code>)로 세트가 달라 직접 비교는 참고용. 점에 마우스를 올리면 실행 정보가 보인다.</p><div class="legend">"#
    );
    for (i, (name, color)) in SERIES.iter().enumerate() {
        let key = if i == 2 {
            format!(
                r#"<svg width="12" height="12" style="width:12px"><rect x="1" y="1" width="10" height="10" fill="{color}"/></svg>"#
            )
        } else {
            format!(
                r#"<svg width="12" height="12" style="width:12px"><circle cx="6" cy="6" r="5" fill="{color}"/></svg>"#
            )
        };
        let _ = write!(w, "<span>{key}{}</span>", esc(name));
    }
    let _ = write!(w, r#"</div><div class="charts">"#);
    for (title, suffix) in [
        ("유출률 D0~D2 (주 지표)", "leak_rate_d02"),
        ("유출률 D3~D4", "leak_rate_d34"),
        ("서명란 커버리지", "signature_coverage"),
        ("이미지·PDF 오탐률", "fp_rate"),
    ] {
        chart(&mut w, title, suffix, &e5);
    }
    let _ = write!(w, "</div>");

    // 목표 대비
    let _ = write!(w, "<h2>최신 평가 — 목표 대비</h2>");
    let mut latest: Vec<&RunRecord> = Vec::new();
    latest.extend(latest_image.map(|(r, _)| r));
    latest.extend(latest_text);
    targets_table(&mut w, &latest);

    results_section(&mut w, x);
    failures_section(&mut w, x);
    diff_section(&mut w, &main_runs);
    run_info_section(&mut w, latest_image.map(|(r, _)| r), x, latest_text);

    // 이슈
    let _ = write!(
        w,
        r#"<h2>이슈</h2><div class="scroll"><table><thead><tr><th>이슈</th><th>상태</th></tr></thead><tbody>"#
    );
    for (t, s) in parse_issue_status(issues_md) {
        let _ = write!(w, "<tr><td>{}</td><td>{}</td></tr>", esc(&t), esc(&s));
    }
    let _ = write!(w, "</tbody></table></div>");

    // 실행 기록 (추이의 표 보기를 겸함)
    let _ = write!(
        w,
        r#"<h2>실행 기록</h2><div class="scroll"><table><thead><tr><th>실행 ID</th><th>구분</th><th>세트</th><th>시드</th><th>건수</th><th>주요 지표</th><th>시간</th><th>변경 사항</th></tr></thead><tbody>"#
    );
    for (r, _) in runs.iter().rev() {
        let time = metric(r, "secs_per_page")
            .map(|m| format!("쪽당 {:.1}초", m.value))
            .or_else(|| metric(r, "secs_per_doc").map(|m| format!("문서당 {:.2}초", m.value)))
            .unwrap_or_else(|| "-".into());
        let _ = write!(
            w,
            r#"<tr><td><code>{}</code></td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{}</td><td>{}</td></tr>"#,
            esc(&r.run_id),
            esc(&r.set_kind),
            esc(r.set.rsplit('/').next().unwrap_or(&r.set)),
            r.seed.map_or("-".into(), |s| s.to_string()),
            r.docs,
            esc(&run_summary(r)),
            esc(&time),
            esc(&r.note)
        );
    }
    let _ = write!(w, "</tbody></table></div>");
    gallery_section(&mut w, x);
    let _ = write!(
        w,
        "<footer>생성: <code>pii-mask eval dashboard</code> · 평가용 세트는 숫자만 표시 · 문서 이미지는 개발용 오버레이만 · 외부 요청 없음</footer></main></body></html>"
    );
    w
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(p).ok()?).ok()
}

/// 개발용 오버레이 폴더인가 (평가용 세트는 절대 넣지 않는다)
fn is_dev_overlay_dir(name: &str) -> bool {
    name.starts_with("masked_dev") && !name.contains("eval") && !name.contains("text_")
}

/// 개발용 오버레이 목록을 난이도별로 (페이지 기준 상대 경로)
fn load_gallery(root: &Path) -> Vec<(String, Vec<String>)> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Ok(dirs) = fs::read_dir(root.join("data")) else {
        return Vec::new();
    };
    for dir in dirs.flatten() {
        let name = dir.file_name().to_string_lossy().to_string();
        if !is_dev_overlay_dir(&name) {
            continue;
        }
        let set = name.trim_start_matches("masked_");
        let manifest = fs::read_to_string(root.join("data").join(set).join("manifest.jsonl"))
            .unwrap_or_default();
        let diff_of: BTreeMap<String, String> = manifest
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| {
                Some((
                    v["doc_id"].as_str()?.to_string(),
                    v["difficulty"].as_str()?.to_string(),
                ))
            })
            .collect();
        let Ok(files) = fs::read_dir(dir.path()) else {
            continue;
        };
        for f in files.flatten() {
            let file = f.file_name().to_string_lossy().to_string();
            if !file.ends_with("_check.png") {
                continue;
            }
            let doc: String = file.split('_').take(2).collect::<Vec<_>>().join("_");
            let diff = diff_of.get(&doc).cloned().unwrap_or_else(|| "기타".into());
            by.entry(diff)
                .or_default()
                .push(format!("../../data/{name}/{file}"));
        }
    }
    let mut out: Vec<_> = by.into_iter().collect();
    for (_, v) in &mut out {
        v.sort();
    }
    out
}

fn load_extras(root: &Path, runs: &[Run]) -> Extras {
    let results = root.join("results");
    let mut x = Extras::default();
    let latest = runs
        .iter()
        .rev()
        .map(|(r, _)| r)
        .find(|r| r.exp == "e5" && r.set_kind == "평가용" && !is_holdout(r));
    if let Some(r) = latest {
        let name = r.set.rsplit('/').next().unwrap_or(&r.set);
        let file = if name == "eval_s42" {
            "e5_eval".to_string()
        } else {
            format!("e5_{name}")
        };
        let data: Option<E5Data> = read_json(&results.join(format!("{file}.json")));
        let layout: Option<E5Data> = read_json(&results.join(format!("{file}_layout.json")));
        x.layouts = layout
            .map(|l| l.by_layout)
            .filter(|l| !l.is_empty())
            .or_else(|| data.as_ref().map(|d| d.by_layout.clone()))
            .unwrap_or_default();
        x.e5 = data;
    }
    x.text = read_json(&results.join("e4b.json"));
    for exp in ["e1", "e2", "e4b"] {
        let overall = read_json(&results.join(format!("holdout_{exp}.json")));
        let by_form = read_json(&results.join(format!("holdout_{exp}_by_form.json")));
        if let (Some(overall), Some(by_form)) = (overall, by_form) {
            x.holdout.push(HoldoutExp {
                exp: exp.to_string(),
                overall,
                by_form,
                baseline: read_json(&results.join(format!("{exp}.json"))),
            });
        }
    }
    x.gallery = load_gallery(root);
    x
}

/// `results/dashboard/index.html`을 쓴다
pub fn write(root: &Path) -> std::io::Result<PathBuf> {
    let runs = load_all(root);
    let extras = load_extras(root, &runs);
    let issues = fs::read_to_string(root.join("docs/issues.md")).unwrap_or_default();
    let (date, time) = now_kst();
    let hm = time
        .get(..4)
        .map_or(time.clone(), |x| format!("{}:{}", &x[..2], &x[2..]));
    let html = render(&runs, &extras, &issues, &format!("{date} {hm}"));
    let dir = root.join("results/dashboard");
    fs::create_dir_all(&dir)?;
    let path = dir.join("index.html");
    fs::write(&path, html)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric_of(key: &str, k: usize, n: usize, goal: Goal, target: f64) -> Metric {
        let r = Rate::new(k, n);
        Metric {
            key: key.into(),
            name: key.into(),
            k,
            n,
            value: r.p,
            lo: r.lo,
            hi: r.hi,
            goal,
            target,
            verdict: "불가".into(),
        }
    }

    fn run(id: &str, set: &str, kind: &str, note: &str) -> Run {
        let rec = RunRecord {
            run_id: id.into(),
            date: "2026-11-09".into(),
            exp: "e5".into(),
            set: set.into(),
            seed: Some(46),
            set_kind: kind.into(),
            docs: 100,
            note: note.into(),
            metrics: vec![
                metric_of("e5.leak_rate_d0", 0, 200, Goal::Report, 0.0),
                metric_of("e5.leak_rate_d1", 14, 200, Goal::Report, 0.0),
                metric_of("e5.leak_rate_d02", 14, 400, Goal::AtMost, 0.01),
            ],
        };
        let mut f = BTreeMap::new();
        f.insert("D1:OCR 오인식".to_string(), 3);
        f.insert("D1:오탐 ZIPCODE".to_string(), 9);
        (rec, f)
    }

    #[test]
    fn page_makes_no_external_requests_and_escapes_notes() {
        let html = render(
            &[run("r1", "data/eval2_s46", "평가용", "<script>x</script>")],
            &Extras::default(),
            "## ISSUE-001: a\n- 상태: 열림\n",
            "2026-11-09 01:00",
        );
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("ISSUE-001"));
    }

    #[test]
    fn holdout_runs_stay_out_of_main_cards() {
        let runs = vec![
            run("r1", "data/eval2_s46", "평가용", ""),
            run("r2", "data/text_holdout_s48", "평가용", ""),
        ];
        let html = render(&runs, &Extras::default(), "", "t");
        assert!(html.contains("근거 실행 <code>r1</code>"));
    }

    #[test]
    fn evaluation_sets_never_enter_the_gallery() {
        assert!(is_dev_overlay_dir("masked_dev_s41"));
        assert!(is_dev_overlay_dir("masked_dev2_s45"));
        assert!(!is_dev_overlay_dir("masked_eval_s42"));
        assert!(!is_dev_overlay_dir("masked_dev_eval"));
        assert!(!is_dev_overlay_dir("eval2_s46"));
    }

    #[test]
    fn failures_split_into_groups() {
        let mut f = BTreeMap::new();
        for (k, n) in [
            ("OCR 오인식", 3),
            ("서명란 앵커 미검출", 2),
            ("오탐 ZIPCODE", 1),
            ("오탐:필드", 1),
        ] {
            f.insert(k.to_string(), n);
        }
        let (leak, sign, fp, src) = failure_groups(&f);
        assert_eq!(leak, vec![("OCR 오인식".to_string(), 3)]);
        assert_eq!(sign.len(), 1);
        assert_eq!(fp, vec![("ZIPCODE".to_string(), 1)]);
        assert_eq!(src, vec![("필드".to_string(), 1)]);
    }

    #[test]
    fn path_failures_count_only_that_path_and_skip_false_positives() {
        let (_, f) = run("r", "s", "평가용", "");
        assert_eq!(
            top_failures(&f, &["d1", "d2"]),
            vec![("OCR 오인식".to_string(), 3)]
        );
        assert!(top_failures(&f, &["d3"]).is_empty());
    }
}
