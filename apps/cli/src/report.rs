//! `pii-mask data-report` — 6단계 데이터 점검 리포트 (`results/data_report.md`)
//!
//! 세트 폴더의 라벨 JSON·`manifest.jsonl`·`ratio_tally.json`만 읽는다 (이미지는 열지 않음).
//! 리포트에는 통계만 쓰고 개인정보 값은 쓰지 않는다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pii_core::{DocLabel, Label};
use synth::ratios::{RatioConfig, TallyCounts};

/// 세트 하나
struct Set {
    name: String,
    labels: Vec<DocLabel>,
    manifest: Vec<serde_json::Value>,
    tally: TallyCounts,
    image: bool,
}

fn read_set(dir: &Path) -> Result<Set> {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| dir.display().to_string());
    let manifest: Vec<serde_json::Value> = fs::read_to_string(dir.join("manifest.jsonl"))
        .with_context(|| format!("{}/manifest.jsonl", dir.display()))?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let text_labels = dir.join("labels.jsonl");
    let (labels, image) = if text_labels.is_file() {
        let labels = fs::read_to_string(&text_labels)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<Vec<DocLabel>, _>>()?;
        (labels, false)
    } else {
        let mut labels = Vec::new();
        for m in &manifest {
            let f = m["label_file"].as_str().unwrap_or_default();
            let src = fs::read_to_string(dir.join(f)).with_context(|| f.to_string())?;
            labels.push(serde_json::from_str(&src)?);
        }
        (labels, true)
    };
    let tally = serde_json::from_str(&fs::read_to_string(dir.join("ratio_tally.json"))?)?;
    Ok(Set {
        name,
        labels,
        manifest,
        tally,
        image,
    })
}

fn label_name(l: Label) -> String {
    serde_json::to_value(l)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{l:?}"))
}

fn pct(a: f64, b: f64) -> String {
    if b == 0.0 {
        "-".into()
    } else {
        format!("{:.1}%", a / b * 100.0)
    }
}

/// 재현율 p, 표본 n일 때 Wilson 95% 신뢰구간 폭 (%p)
fn wilson_width(p: f64, n: f64) -> f64 {
    if n == 0.0 {
        return f64::NAN;
    }
    let z = 1.96f64;
    let denom = 1.0 + z * z / n;
    let half = z * ((p * (1.0 - p) / n) + z * z / (4.0 * n * n)).sqrt() / denom;
    2.0 * half * 100.0
}

pub fn run(dirs: &[PathBuf], out: &Path) -> Result<()> {
    let sets: Vec<Set> = dirs.iter().map(|d| read_set(d)).collect::<Result<_>>()?;
    let ratios = RatioConfig::load(Path::new("assets/synth/ratios.toml"))?;
    let mut md = String::new();
    let w = &mut md;
    writeln!(w, "# 데이터 점검 리포트 (6단계)\n")?;
    writeln!(
        w,
        "`pii-mask data-report`로 생성. 라벨 JSON·매니페스트·비율 집계만 읽음 (이미지는 열지 않음). 개인정보 값은 싣지 않는다.\n"
    )?;

    // 1. 세트 개요
    writeln!(w, "## 1. 세트 개요\n")?;
    writeln!(w, "| 세트 | 문서 | 쪽 이미지 | PDF | 쪽 수 합 | 용도 |")?;
    writeln!(w, "|---|---:|---:|---:|---:|---|")?;
    for s in &sets {
        let imgs: usize = s
            .manifest
            .iter()
            .map(|m| m["pages"].as_array().map_or(0, Vec::len))
            .sum();
        let pdfs = s.manifest.iter().filter(|m| m["pdf"].is_string()).count();
        let pages: u32 = s.labels.iter().map(|l| l.pages).sum();
        let usage = if !s.image {
            "텍스트 탐지기(E1~E4) 평가"
        } else if s.labels.len() <= 20 {
            "8~12단계 개발·조정 전용"
        } else {
            "13단계 판정 전용 (개발 중 열람 금지)"
        };
        writeln!(
            w,
            "| `{}` | {} | {imgs} | {pdfs} | {pages} | {usage} |",
            s.name,
            s.labels.len()
        )?;
    }

    // 2. 레이아웃·난이도 분포
    writeln!(w, "\n## 2. 레이아웃·난이도 분포\n")?;
    for s in &sets {
        let mut grid: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut forms: BTreeMap<String, usize> = BTreeMap::new();
        for l in &s.labels {
            *grid
                .entry((l.layout.clone(), l.difficulty.clone()))
                .or_default() += 1;
            *forms.entry(l.form.clone()).or_default() += 1;
        }
        let diffs: BTreeSet<&String> = grid.keys().map(|(_, d)| d).collect();
        let layouts: BTreeSet<&String> = grid.keys().map(|(l, _)| l).collect();
        writeln!(w, "### `{}`\n", s.name)?;
        write!(w, "| 레이아웃 |")?;
        for d in &diffs {
            write!(w, " {d} |")?;
        }
        writeln!(w, " 합 |")?;
        write!(w, "|---|")?;
        for _ in &diffs {
            write!(w, "---:|")?;
        }
        writeln!(w, "---:|")?;
        for l in &layouts {
            write!(w, "| {l} |")?;
            let mut sum = 0;
            for d in &diffs {
                let n = grid
                    .get(&((*l).clone(), (*d).clone()))
                    .copied()
                    .unwrap_or(0);
                sum += n;
                write!(w, " {n} |")?;
            }
            writeln!(w, " {sum} |")?;
        }
        let f: Vec<String> = forms.iter().map(|(k, v)| format!("{k} {v}")).collect();
        writeln!(w, "\n양식: {}\n", f.join(", "))?;
    }

    // 3. 라벨별 개수
    writeln!(w, "## 3. 라벨별 개수 (span 수 / 그 라벨이 있는 문서 수)\n")?;
    let mut all_labels: BTreeSet<Label> = BTreeSet::new();
    let mut per_set: Vec<BTreeMap<Label, (usize, usize, usize)>> = Vec::new();
    for s in &sets {
        let mut m: BTreeMap<Label, (usize, usize, usize)> = BTreeMap::new();
        for l in &s.labels {
            let mut seen = BTreeSet::new();
            for sp in &l.spans {
                let e = m.entry(sp.label).or_default();
                e.0 += 1;
                e.2 += usize::from(sp.hand);
                if seen.insert(sp.label) {
                    e.1 += 1;
                }
            }
            // 이미지 전용 라벨(서명·도장·지장)은 박스로 센다
            let mut seen_box = BTreeSet::new();
            for b in &l.boxes {
                if let (Some(lb), Some(_)) = (b.label, b.kind)
                    && lb.is_image_only()
                {
                    let e = m.entry(lb).or_default();
                    e.0 += 1;
                    if seen_box.insert(lb) {
                        e.1 += 1;
                    }
                }
            }
        }
        all_labels.extend(m.keys().copied());
        per_set.push(m);
    }
    write!(w, "| 라벨 | 항상 가림 |")?;
    for s in &sets {
        write!(w, " `{}` |", s.name)?;
    }
    writeln!(w)?;
    write!(w, "|---|:---:|")?;
    for _ in &sets {
        write!(w, "---:|")?;
    }
    writeln!(w)?;
    for l in &all_labels {
        write!(
            w,
            "| {} | {} |",
            label_name(*l),
            if Label::ALWAYS_MASKED.contains(l) {
                "✔"
            } else {
                ""
            }
        )?;
        for m in &per_set {
            let (n, d, _) = m.get(l).copied().unwrap_or_default();
            write!(w, " {n} / {d} |")?;
        }
        writeln!(w)?;
    }
    writeln!(
        w,
        "\n서명·도장·지장(`SIGNATURE`·`SEAL`·`FINGERPRINT`)은 이미지 박스 수. 텍스트 세트에는 박스가 없어 0.\n"
    )?;
    write!(w, "참고 항목(attributes)·하드네거티브(negatives) 수:")?;
    for s in &sets {
        let a: usize = s.labels.iter().map(|l| l.attributes.len()).sum();
        let n: usize = s.labels.iter().map(|l| l.negatives.len()).sum();
        write!(w, " `{}` 참고 {a} · 네거티브 {n};", s.name)?;
    }
    writeln!(w, "\n")?;

    // 4. 정형 라벨 표본과 신뢰구간
    writeln!(w, "## 4. 정형 라벨 표본 수와 신뢰구간\n")?;
    writeln!(
        w,
        "목표: 텍스트 세트에서 정형 라벨별 span 500개 이상 (PLAN.md 1-2). 신뢰구간 폭 = 재현율 99%가 나왔을 때 Wilson 95% 구간의 폭.\n"
    )?;
    write!(w, "| 라벨 |")?;
    for s in &sets {
        write!(w, " `{}` span | 문서 | 구간 폭 |", s.name)?;
    }
    writeln!(w)?;
    write!(w, "|---|")?;
    for _ in &sets {
        write!(w, "---:|---:|---:|")?;
    }
    writeln!(w)?;
    for l in synth::dataset::TEXT_TARGET_LABELS {
        write!(w, "| {} |", label_name(l))?;
        for m in &per_set {
            let (n, d, _) = m.get(&l).copied().unwrap_or_default();
            if n == 0 {
                write!(w, " 0 | 0 | - |")?;
            } else {
                write!(
                    w,
                    " {n} | {d} | ±{:.1}%p |",
                    wilson_width(0.99, n as f64) / 2.0
                )?;
            }
        }
        writeln!(w)?;
    }
    writeln!(w, "\n표본이 적은 라벨 (평가용 이미지 세트 span 30개 미만):")?;
    if let Some((i, s)) = sets
        .iter()
        .enumerate()
        .find(|(_, s)| s.image && s.labels.len() > 20)
    {
        let few: Vec<String> = per_set[i]
            .iter()
            .filter(|(_, v)| v.0 < 30)
            .map(|(l, v)| format!("{} {}", label_name(*l), v.0))
            .collect();
        writeln!(
            w,
            "`{}` — {}\n",
            s.name,
            if few.is_empty() {
                "없음".into()
            } else {
                few.join(", ")
            }
        )?;
    }

    // 5. 문서당 평균 개인정보 수
    writeln!(
        w,
        "## 5. 문서당 평균 개인정보 수 (span, 이미지 전용 라벨 제외)\n"
    )?;
    write!(w, "| 레이아웃 |")?;
    for s in &sets {
        write!(w, " `{}` |", s.name)?;
    }
    writeln!(w)?;
    write!(w, "|---|")?;
    for _ in &sets {
        write!(w, "---:|")?;
    }
    writeln!(w)?;
    let layouts: BTreeSet<String> = sets
        .iter()
        .flat_map(|s| s.labels.iter().map(|l| l.layout.clone()))
        .collect();
    for lay in layouts.iter().map(Some).chain([None]) {
        write!(w, "| {} |", lay.map_or("전체".to_string(), |l| l.clone()))?;
        for s in &sets {
            let docs: Vec<&DocLabel> = s
                .labels
                .iter()
                .filter(|l| lay.is_none_or(|x| &l.layout == x))
                .collect();
            let n: usize = docs.iter().map(|l| l.spans.len()).sum();
            if docs.is_empty() {
                write!(w, " - |")?;
            } else {
                write!(w, " {:.1} |", n as f64 / docs.len() as f64)?;
            }
        }
        writeln!(w)?;
    }

    // 6. 손글씨
    writeln!(w, "\n## 6. 손글씨\n")?;
    writeln!(
        w,
        "| 세트 | 손글씨 문서 | 손글씨 문서 안 개인정보 span 중 손글씨 | 전체 span 중 손글씨 |"
    )?;
    writeln!(w, "|---|---:|---:|---:|")?;
    for s in &sets {
        let hd = s.labels.iter().filter(|l| l.handwriting_doc).count();
        let (mut hs, mut hn) = (0usize, 0usize);
        for l in s.labels.iter().filter(|l| l.handwriting_doc) {
            hs += l.spans.iter().filter(|x| x.hand).count();
            hn += l.spans.len();
        }
        let total: usize = s.labels.iter().map(|l| l.spans.len()).sum();
        writeln!(
            w,
            "| `{}` | {hd} / {} ({}) | {} | {} |",
            s.name,
            s.labels.len(),
            pct(hd as f64, s.labels.len() as f64),
            pct(hs as f64, hn as f64),
            pct(hs as f64, total as f64)
        )?;
    }
    writeln!(
        w,
        "\n설정: 손글씨 문서 0.4 (`handwriting_doc`), 그 안 기입 필드 50~100%. D0(디지털)은 손글씨 없음 (사용자 결정).\n"
    )?;

    // 7. 서명란
    writeln!(w, "## 7. 서명란 (축 1 서명 방식 × 축 2 도장)\n")?;
    for s in &sets {
        let mut axis1: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut axis2: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut blank = 0usize;
        let mut worker_lines = 0usize;
        for m in &s.manifest {
            for sig in m["signatures"].as_array().into_iter().flatten() {
                let owner = sig["owner"].as_str().unwrap_or("?").to_string();
                let method = sig["method"].as_str().unwrap_or("?").to_string();
                let seal = sig["seal"].as_str().unwrap_or("?").to_string();
                if owner == "worker" {
                    worker_lines += 1;
                    blank += usize::from(method == "none" && seal == "none");
                }
                *axis1.entry((owner.clone(), method)).or_default() += 1;
                *axis2.entry((owner, seal)).or_default() += 1;
            }
        }
        writeln!(w, "### `{}`\n", s.name)?;
        writeln!(w, "| 소속 | 축 1 서명 방식 (건수) | 축 2 도장 (건수) |")?;
        writeln!(w, "|---|---|---|")?;
        let owners: BTreeSet<&String> = axis1.keys().map(|(o, _)| o).collect();
        for o in owners {
            let total: usize = axis1
                .iter()
                .filter(|((x, _), _)| x == o)
                .map(|(_, n)| n)
                .sum();
            let a1: Vec<String> = axis1
                .iter()
                .filter(|((x, _), _)| x == o)
                .map(|((_, k), n)| format!("{k} {n} ({})", pct(*n as f64, total as f64)))
                .collect();
            let a2: Vec<String> = axis2
                .iter()
                .filter(|((x, _), _)| x == o)
                .map(|((_, k), n)| format!("{k} {n} ({})", pct(*n as f64, total as f64)))
                .collect();
            writeln!(w, "| {o} | {} | {} |", a1.join(", "), a2.join(", "))?;
        }
        writeln!(
            w,
            "\n근로자 서명란 중 서명·도장 모두 없음: {blank} / {worker_lines} ({}) — 설정상 기대 16%\n",
            pct(blank as f64, worker_lines as f64)
        )?;
    }

    // 8. 임의로 정한 비율
    writeln!(w, "## 8. 임의로 정한 비율 (`arbitrary = true`)\n")?;
    writeln!(
        w,
        "`assets/synth/ratios.toml`에서 근거 없이 정한 초기값. 실제 = 이 세트를 만들 때 뽑힌 비율 (뽑힌 횟수 N).\n"
    )?;
    write!(w, "| 표 | 무엇 | 키 | 설정 |")?;
    for s in &sets {
        write!(w, " `{}` 실제 |", s.name)?;
    }
    writeln!(w)?;
    write!(w, "|---|---|---|---:|")?;
    for _ in &sets {
        write!(w, "---:|")?;
    }
    writeln!(w)?;
    for (name, t) in ratios.arbitrary() {
        for (i, (key, wgt)) in t.weights.iter().enumerate() {
            let note = if i == 0 { t.note.as_str() } else { "" };
            let tname = if i == 0 {
                format!("`{name}`")
            } else {
                String::new()
            };
            write!(w, "| {tname} | {note} | {key} | {wgt:.2} |")?;
            for s in &sets {
                let counts = s.tally.get(name);
                let total: u64 = counts.map_or(0, |c| c.values().sum());
                let n = counts.and_then(|c| c.get(key)).copied().unwrap_or(0);
                if total == 0 {
                    write!(w, " - |")?;
                } else {
                    write!(w, " {:.2} (N={total}) |", n as f64 / total as f64)?;
                }
            }
            writeln!(w)?;
        }
    }
    writeln!(w, "\n\"-\"는 그 세트에서 쓰이지 않은 표.")?;

    if let Some(p) = out.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(out, md).with_context(|| format!("{} 쓰기", out.display()))?;
    println!("리포트: {}", out.display());
    Ok(())
}
