//! `pii-mask eval-text` — 텍스트 1,000건 탐지 평가 (E1 규칙 / E2 규칙+NER / E4 규칙+NER+SLM)
//!
//! 결과 JSON만 쓴다 (값 없음, 통계만).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use eval::{Pipeline, TextEval};
use pii_core::DocLabel;

pub struct Options {
    pub set: PathBuf,
    pub exp: String,
    pub out: PathBuf,
    pub slm_url: String,
    pub limit: Option<usize>,
    pub note: String,
}

pub fn run(o: &Options) -> Result<()> {
    let ner = Path::new("models/ner/koelectra-ko-pii-ner");
    let pack = Path::new("assets/rulepacks/kr-contract-0.1.0.toml");
    let (use_ner, use_slm) = match o.exp.as_str() {
        "e1" => (false, false),
        "e2" => (true, false),
        "e4" | "e4b" => (true, true),
        other => anyhow::bail!("모르는 실험: {other} (e1, e2, e4, e4b)"),
    };
    let mut p = Pipeline::new(
        pack,
        use_ner.then_some(ner),
        use_slm.then_some(o.slm_url.as_str()),
    )?;
    if o.exp == "e4b"
        && let Some(slm) = p.slm.take()
    {
        // E4b (추가 실험): 특약사항 구역의 모든 줄을 SLM에 보냄 + 트리거 줄
        p.slm = Some(slm.with_section("특약"));
    }
    let src = fs::read_to_string(o.set.join("labels.jsonl")).context("labels.jsonl")?;
    let mut ev = TextEval::default();
    // 처음 보는 서식 시험: 서식별로도 집계
    let by_form_on = eval::runlog::is_holdout_set(&o.set.display().to_string());
    let mut by_form: BTreeMap<String, TextEval> = BTreeMap::new();
    let t_all = Instant::now();
    for (i, line) in src.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        if o.limit.is_some_and(|n| i >= n) {
            break;
        }
        let label: DocLabel = serde_json::from_str(line)?;
        let t = Instant::now();
        let (dets, timing) = p.detect(&label.text)?;
        let secs = t.elapsed().as_secs_f64();
        ev.add(&label, &dets, &p.pack, &timing, secs);
        if by_form_on {
            by_form
                .entry(label.form.clone())
                .or_default()
                .add(&label, &dets, &p.pack, &timing, secs);
        }
        // 개발용 확인: 놓친 정답 span (합성 데이터 전용, 환경변수로만)
        if let Ok(path) = std::env::var("PII_DUMP_MISS") {
            let chars: Vec<char> = label.text.chars().collect();
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            for sp in &label.spans {
                if p.pack.masks(sp.label)
                    && eval::char_coverage(&chars, sp.start.0, sp.end.0, &dets) < 0.9
                {
                    let a = sp.start.0.saturating_sub(25);
                    let ctx: String = chars[a..sp.end.0.min(chars.len())].iter().collect();
                    use std::io::Write as _;
                    let ctx = ctx.replace(['\n', '\t'], " ");
                    writeln!(f, "{:?}\t{}\t{}", sp.label, sp.value, ctx)?;
                }
            }
        }
        // 개발용 확인: 정답·참고 항목과 겹치지 않는 탐지 문자열 (합성 데이터 전용, 환경변수로만)
        if let Ok(path) = std::env::var("PII_DUMP_FALSE") {
            let chars: Vec<char> = label.text.chars().collect();
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            for d in &dets {
                let gold = label.spans.iter().any(|s| d.overlaps(s.start.0, s.end.0));
                let attr = label
                    .attributes
                    .iter()
                    .any(|a| d.overlaps(a.start.0, a.end.0));
                if !gold && !attr {
                    let v: String = chars[d.start.0.min(chars.len())..d.end.0.min(chars.len())]
                        .iter()
                        .collect();
                    use std::io::Write as _;
                    writeln!(f, "{:?}	{:?}	{}", d.detector, d.label, v)?;
                }
            }
        }
        if (i + 1) % 100 == 0 {
            eprintln!(
                "{} {}건 ({:.0}초)",
                o.exp,
                i + 1,
                t_all.elapsed().as_secs_f64()
            );
        }
    }
    if let Some(dir) = o.out.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&o.out, serde_json::to_vec_pretty(&ev)?)?;
    if by_form_on {
        let stem = o
            .out
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().to_string());
        fs::write(
            o.out.with_file_name(format!("{stem}_by_form.json")),
            serde_json::to_vec_pretty(&by_form)?,
        )?;
    }
    crate::eval_cmd::save_text(&o.exp, &o.set, &o.note, &ev, None)?;
    println!(
        "{}: 문서 {}건, 정형 {} / 이름·주소 {} / 민감 {} / 오탐(하드네거티브) {} / 정밀도 {} / 문서당 {:.3}초",
        o.exp,
        ev.docs,
        ev.group_rate(&eval::STRUCTURED).fmt_pct(),
        ev.group_rate(&eval::NAME_ADDRESS).fmt_pct(),
        ev.group_rate(&eval::SENSITIVE).fmt_pct(),
        ev.negative_rate().fmt_pct(),
        ev.precision().fmt_pct(),
        ev.seconds_total / ev.docs.max(1) as f64
    );
    Ok(())
}
