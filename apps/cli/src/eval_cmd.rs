//! `pii-mask eval …` — 개선 기록 (실행 기록·이슈·실행 로그·관리자 페이지)

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use eval::TextEval;
use eval::image::DiffStats;
use eval::runlog::{self, RunRecord};

/// 텍스트 실험 결과를 실행 기록으로 저장
pub fn save_text(
    exp: &str,
    set: &Path,
    note: &str,
    ev: &TextEval,
    run_id: Option<(&str, &str)>,
) -> Result<String> {
    let (seed, set_kind) = runlog::set_info(set);
    // 처음 보는 서식 시험은 지표 키를 나눠 기존 이슈·추이에 섞이지 않게 한다 (h_e1, h_e2, h_e4b)
    let holdout_exp = format!("h_{exp}");
    let exp = if runlog::is_holdout_set(&set.display().to_string()) {
        holdout_exp.as_str()
    } else {
        exp
    };
    let (id, date) = match run_id {
        Some((i, d)) => (i.to_string(), d.to_string()),
        None => runlog::new_run_id(exp),
    };
    let rec = RunRecord {
        run_id: id.clone(),
        date,
        exp: exp.into(),
        set: set.display().to_string().replace('\\', "/"),
        seed,
        set_kind,
        docs: ev.docs,
        note: note.into(),
        metrics: runlog::text_metrics(exp, ev),
    };
    let dir = runlog::save(Path::new("."), &rec, &runlog::text_failures(ev))?;
    println!("실행 기록: {}", dir.display());
    Ok(id)
}

/// 이미지 실험 결과를 실행 기록으로 저장
pub fn save_image(
    exp: &str,
    set: &Path,
    note: &str,
    by: &BTreeMap<String, DiffStats>,
    run_id: Option<(&str, &str)>,
) -> Result<String> {
    let (seed, set_kind) = runlog::set_info(set);
    let (id, date) = match run_id {
        Some((i, d)) => (i.to_string(), d.to_string()),
        None => runlog::new_run_id(exp),
    };
    let rec = RunRecord {
        run_id: id.clone(),
        date,
        exp: exp.into(),
        set: set.display().to_string().replace('\\', "/"),
        seed,
        set_kind,
        docs: by.values().map(|s| s.docs).sum(),
        note: note.into(),
        metrics: runlog::image_metrics(exp, by),
    };
    let dir = runlog::save(Path::new("."), &rec, &runlog::image_failures(by))?;
    println!("실행 기록: {}", dir.display());
    Ok(id)
}

pub fn issues() -> Result<()> {
    let s = eval::issues::update(Path::new("."))?;
    println!(
        "docs/issues.md 자동 영역 {}개 갱신, 자동 등록 {}건{}, docs/run_log.md {}줄 추가",
        s.issues_updated,
        s.issues_registered.len(),
        if s.issues_registered.is_empty() {
            String::new()
        } else {
            format!(" ({})", s.issues_registered.join(", "))
        },
        s.runs_logged
    );
    Ok(())
}

pub fn dashboard() -> Result<()> {
    let p = eval::dashboard::write(Path::new("."))?;
    println!("관리자 페이지: {}", p.display());
    Ok(())
}

#[derive(serde::Deserialize)]
struct E5File {
    by_difficulty: BTreeMap<String, DiffStats>,
}

/// 지금까지의 결과 JSON을 실행 기록으로 등록 (한 번만, 이미 있으면 건너뜀). 시각은 실행 순서를 맞춘 값
pub fn backfill() -> Result<()> {
    // (파일, 실험, 세트, 실행 ID, 변경 사항)
    let items: [(&str, &str, &str, &str, &str); 13] = [
        (
            "results/e1_first.json",
            "e1",
            "data/text_s43",
            "20261108-170000-e1",
            "규칙 팩 첫 버전 (사후 등록)",
        ),
        (
            "results/e2_first.json",
            "e2",
            "data/text_s43",
            "20261108-170100-e2",
            "규칙 팩 첫 버전 + NER 이름·주소 (사후 등록)",
        ),
        (
            "results/e1_pre_final.json",
            "e1",
            "data/text_s43",
            "20261108-173000-e1",
            "점(.)·공백 구분자, 새마을금고 4-2-6-1 형식 보강 — 분석은 개발용 텍스트 시드 44 (사후 등록)",
        ),
        (
            "results/e2_pre_final.json",
            "e2",
            "data/text_s43",
            "20261108-173100-e2",
            "같은 보강 (사후 등록)",
        ),
        (
            "results/e4.json",
            "e4",
            "data/text_s43",
            "20261108-180000-e4",
            "SLM 트리거 방식 (사후 등록)",
        ),
        (
            "results/e5_dev.json",
            "e5",
            "data/dev_s41",
            "20261108-183000-e5",
            "개발용 세트 확인 — 서명란 붉은 덩어리 연결 성분, 앵커 조각, 빈 값 키 (사후 등록)",
        ),
        (
            "results/e5_eval_first.json",
            "e5",
            "data/eval_s42",
            "20261108-190000-e5",
            "E5 첫 실행 (사후 등록)",
        ),
        (
            "results/e5_dev2.json",
            "e5",
            "data/dev2_s45",
            "20261108-191500-e5",
            "개발용 2차 세트 — 쪽 기울기 행 묶기·좌우 여백 30% (사후 등록)",
        ),
        (
            "results/e5_eval.json",
            "e5",
            "data/eval_s42",
            "20261108-193000-e5",
            "쪽 기울기 행 묶기·마스크 좌우 여백 30% (사후 등록)",
        ),
        (
            "results/e4b_300.json",
            "e4b",
            "data/text_s43",
            "20261108-200000-e4b",
            "추가 실험: 특약 구역 전체 줄을 SLM에 (앞 300건, 사후 등록)",
        ),
        (
            "results/e4_300.json",
            "e4",
            "data/text_s43",
            "20261108-201500-e4",
            "E4b 비교용 같은 300건 (사후 등록)",
        ),
        (
            "results/e1.json",
            "e1",
            "data/text_s43",
            "20261108-203000-e1",
            "최종 팩으로 다시 실행 — 숫자 변화 없음 (사후 등록)",
        ),
        (
            "results/e2.json",
            "e2",
            "data/text_s43",
            "20261108-203100-e2",
            "최종 팩으로 다시 실행 — 숫자 변화 없음 (사후 등록)",
        ),
    ];
    for (file, exp, set, id, note) in items {
        if Path::new("results/runs").join(id).exists() {
            continue;
        }
        let Ok(src) = fs::read_to_string(file) else {
            println!("건너뜀 (파일 없음): {file}");
            continue;
        };
        let date = "2026-11-08";
        if exp == "e5" {
            let e: E5File = serde_json::from_str(&src).with_context(|| file.to_string())?;
            save_image(
                exp,
                Path::new(set),
                note,
                &e.by_difficulty,
                Some((id, date)),
            )?;
        } else {
            let e: TextEval = serde_json::from_str(&src).with_context(|| file.to_string())?;
            save_text(exp, Path::new(set), note, &e, Some((id, date)))?;
        }
    }
    Ok(())
}
