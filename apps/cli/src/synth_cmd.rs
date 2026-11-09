//! `pii-mask synth` — 5단계 데이터 생성
//!
//! - `--mode image`: 문서마다 라벨 JSON + (D0) PDF 또는 (D1~D4) 열화 쪽 이미지, `manifest.jsonl`, `ratio_tally.json`
//! - `--mode text`: `labels.jsonl`(문서마다 DocLabel 한 줄), `manifest.jsonl`, `ratio_tally.json`
//!
//! 화면 출력은 통계만 (개인정보 값은 찍지 않는다).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use pii_core::Label;
use synth::dataset::{
    BuiltDoc, ManifestEntry, TEXT_TARGET, TEXT_TARGET_LABELS, build_image_doc,
    generate_holdout_text_set, generate_text_set, plan_image_set,
};
use synth::doc::SynthConfig;
use synth::render::Fonts;

pub struct SynthOptions {
    pub image: bool,
    pub n: usize,
    pub seed: u64,
    pub out: PathBuf,
    pub threads: usize,
    /// 처음 보는 서식 시험 템플릿 폴더 (텍스트 세트만)
    pub templates: Option<PathBuf>,
}

/// 출력 폴더: 없으면 만들고, 비어 있지 않으면 거부 (다른 세트를 덮어쓰지 않게)
fn prepare_out(out: &Path) -> Result<()> {
    if out.exists() {
        let mut entries = fs::read_dir(out).with_context(|| format!("{} 읽기", out.display()))?;
        if entries.next().is_some() {
            bail!(
                "출력 폴더가 비어 있지 않습니다: {} (다른 폴더를 주거나 직접 지운 뒤 다시 실행)",
                out.display()
            );
        }
    }
    fs::create_dir_all(out).with_context(|| format!("{} 만들기", out.display()))
}

fn write_json<T: serde::Serialize>(path: &Path, v: &T) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(v)?)
        .with_context(|| format!("{} 쓰기", path.display()))
}

pub fn run(opt: &SynthOptions) -> Result<()> {
    let root = PathBuf::from(".");
    if opt.templates.is_some() && opt.image {
        bail!("--templates는 --mode text에서만 씁니다");
    }
    let cfg = match &opt.templates {
        Some(dir) => SynthConfig::load_with_templates(&root, dir),
        None => SynthConfig::load(&root),
    }
    .context("합성 설정 읽기 (작업 폴더에서 실행하세요)")?;
    prepare_out(&opt.out)?;
    // 이 실행의 비율 집계만 남기게 비운다
    let _ = cfg.ratios.tally.take();
    if opt.image {
        image_set(&cfg, opt)?;
    } else {
        text_set(&cfg, opt)?;
    }
    write_json(&opt.out.join("ratio_tally.json"), &cfg.ratios.tally.take())?;
    println!("완료: {}", opt.out.display());
    Ok(())
}

fn image_set(cfg: &SynthConfig, opt: &SynthOptions) -> Result<()> {
    let fonts = Fonts::load(Path::new("assets/fonts")).context("글꼴 읽기")?;
    let plan = plan_image_set(cfg, opt.n, opt.seed)?;
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, ManifestEntry)>> = Mutex::new(Vec::new());
    let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let threads = opt.threads.max(1).min(plan.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(p) = plan.get(i) else { break };
                    let r = build_image_doc(cfg, &fonts, p)
                        .map_err(anyhow::Error::from)
                        .and_then(|d| save_doc(&opt.out, d));
                    match r {
                        Ok(m) => {
                            if let Ok(mut v) = results.lock() {
                                v.push((i, m));
                            }
                        }
                        Err(e) => {
                            if let Ok(mut v) = errors.lock() {
                                v.push(format!("{}: {e:#}", p.doc_id));
                            }
                        }
                    }
                }
            });
        }
    });
    let errors = errors.into_inner().unwrap_or_default();
    if !errors.is_empty() {
        bail!("문서 생성 실패 {}건:\n{}", errors.len(), errors.join("\n"));
    }
    let mut results = results.into_inner().unwrap_or_default();
    results.sort_by_key(|(i, _)| *i);
    let mut f = fs::File::create(opt.out.join("manifest.jsonl"))?;
    for (_, m) in &results {
        writeln!(f, "{}", serde_json::to_string(m)?)?;
    }
    // 통계
    let mut by_diff: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut by_layout: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, m) in &results {
        let e = by_diff.entry(m.difficulty.as_str()).or_default();
        e.0 += 1;
        e.1 += if m.pdf.is_some() { 1 } else { m.pages.len() };
        *by_layout.entry(m.layout.as_str()).or_default() += 1;
    }
    println!("문서 {}건", results.len());
    for (d, (docs, files)) in &by_diff {
        println!("  {d}: 문서 {docs}건, 파일 {files}개");
    }
    let layouts: Vec<String> = by_layout.iter().map(|(l, n)| format!("{l} {n}")).collect();
    println!("  레이아웃: {}", layouts.join(", "));
    Ok(())
}

fn save_doc(out: &Path, d: BuiltDoc) -> Result<ManifestEntry> {
    for f in &d.files {
        fs::write(out.join(&f.name), &f.bytes).with_context(|| format!("{} 쓰기", f.name))?;
    }
    write_json(&out.join(&d.manifest.label_file), &d.label)?;
    Ok(d.manifest)
}

#[derive(serde::Serialize)]
struct TextManifest<'a> {
    doc_id: &'a str,
    layout: &'a str,
    form: &'a str,
    company_extra: bool,
    foreign_worker: bool,
    driver_license: bool,
    signatures: &'a [synth::dataset::SignInfo],
}

fn text_set(cfg: &SynthConfig, opt: &SynthOptions) -> Result<()> {
    let set = if opt.templates.is_some() {
        generate_holdout_text_set(cfg, opt.n, opt.seed)?
    } else {
        generate_text_set(cfg, opt.n, opt.seed)?
    };
    let mut labels = fs::File::create(opt.out.join("labels.jsonl"))?;
    let mut manifest = fs::File::create(opt.out.join("manifest.jsonl"))?;
    let mut spans: BTreeMap<Label, (u32, u32)> = BTreeMap::new();
    for ((l, sig), x) in set.labels.iter().zip(&set.signatures).zip(&set.extras) {
        writeln!(labels, "{}", serde_json::to_string(l)?)?;
        writeln!(
            manifest,
            "{}",
            serde_json::to_string(&TextManifest {
                doc_id: &l.doc_id,
                layout: &l.layout,
                form: &l.form,
                company_extra: x.company_extra,
                foreign_worker: x.foreign_worker,
                driver_license: x.driver_license,
                signatures: sig,
            })?
        )?;
        let mut seen = std::collections::BTreeSet::new();
        for s in &l.spans {
            let e = spans.entry(s.label).or_default();
            e.0 += 1;
            if seen.insert(s.label) {
                e.1 += 1;
            }
        }
    }
    println!(
        "문서 {}건 — 정형 라벨 (목표 span {TEXT_TARGET}개 이상)",
        set.labels.len()
    );
    let mut short = Vec::new();
    for l in TEXT_TARGET_LABELS {
        let (n, docs) = spans.get(&l).copied().unwrap_or_default();
        println!(
            "  {l:?}: span {n}개, 문서 {docs}건{}",
            if n < TEXT_TARGET { "  ← 미달" } else { "" }
        );
        if n < TEXT_TARGET {
            short.push(format!("{l:?}"));
        }
    }
    // 처음 보는 서식 시험은 건수가 적고 정형 라벨 목표가 없다
    if !short.is_empty() && opt.templates.is_none() {
        bail!("목표 미달 라벨: {}", short.join(", "));
    }
    Ok(())
}
