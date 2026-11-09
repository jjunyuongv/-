//! `pii-mask run-images` — 이미지·PDF 세트 엔드투엔드 (8~12단계 개발 확인, 13단계 E5a·E5b)
//!
//! D0: PDF 텍스트 레이어 + 래스터(200dpi) → 탐지 → 마스킹 → 이미지만으로 새 PDF
//! D1~D4: 쪽 이미지 OCR → 탐지 → 서명란 → 마스킹
//! 결과 JSON(통계만)과, 원하면 마스킹 결과 파일을 쓴다. 쪽당 시간은 순차 처리로 잰다 (기기 조건과 같게)

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use eval::Pipeline;
use eval::image::{DiffStats, ocr_image, process_page, score_page};
use pii_core::DocLabel;

pub struct Options {
    pub set: PathBuf,
    pub out: PathBuf,
    pub masked_out: Option<PathBuf>,
    pub slm_url: Option<String>,
    pub note: String,
    pub slm_section: bool,
}

#[derive(serde::Serialize)]
struct Output {
    set: String,
    conditions: BTreeMap<String, String>,
    model_load_secs: f64,
    peak_working_set_mb_self: Option<f64>,
    peak_working_set_mb_llama_server: Option<f64>,
    by_difficulty: BTreeMap<String, DiffStats>,
    /// 레이아웃별 (관리자 페이지 표용, 채점 항목만)
    by_layout: BTreeMap<String, DiffStats>,
}

/// Windows 프로세스 최대 작업 집합 (PowerShell로 조회, MB)
fn peak_mb(pid: u32) -> Option<f64> {
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .map(|b| b / 1048576.0)
}

fn llama_pid() -> Option<u32> {
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-Process llama-server -ErrorAction SilentlyContinue | Select-Object -First 1).Id",
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

pub fn run(o: &Options) -> Result<()> {
    let root = Path::new(".");
    let t_load = Instant::now();
    let mut pipe = Pipeline::new(
        &root.join("assets/rulepacks/kr-contract-0.1.0.toml"),
        Some(&root.join("models/ner/koelectra-ko-pii-ner")),
        o.slm_url.as_deref(),
    )?;
    if o.slm_section
        && let Some(slm) = pipe.slm.take()
    {
        // E4b 구성: 특약 구역 전체 줄을 SLM에
        pipe.slm = Some(slm.with_section("특약"));
    }
    let engine = ocr::OcrEngine::new(&ocr::OcrModels::in_dir(&root.join("models/ocr/monkt")))?;
    let pdfium = mask::bind_pdfium(root)?;
    let model_load_secs = t_load.elapsed().as_secs_f64();
    if let Some(d) = &o.masked_out {
        fs::create_dir_all(d)?;
    }
    let manifest = fs::read_to_string(o.set.join("manifest.jsonl")).context("manifest.jsonl")?;
    let mut by: BTreeMap<String, DiffStats> = BTreeMap::new();
    let mut by_layout: BTreeMap<String, DiffStats> = BTreeMap::new();
    let t_all = Instant::now();
    for (n, line) in manifest
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
    {
        let m: serde_json::Value = serde_json::from_str(line)?;
        let label_file = m["label_file"].as_str().unwrap_or_default();
        let label: DocLabel = serde_json::from_str(&fs::read_to_string(o.set.join(label_file))?)?;
        let diff = label.difficulty.clone();
        let stats = by.entry(diff.clone()).or_default();
        stats.docs += 1;
        let layout_stats = by_layout.entry(label.layout.clone()).or_default();
        layout_stats.docs += 1;
        let mut doc_secs = 0.0;
        if let Some(pdf) = m["pdf"].as_str() {
            let bytes = fs::read(o.set.join(pdf))?;
            let t = Instant::now();
            let pages = mask::read_pdf(&pdfium, &bytes, 200.0)?;
            let read = t.elapsed().as_secs_f64();
            stats.extract_secs += read;
            let mut masked_pages = Vec::new();
            let per_read = read / pages.len().max(1) as f64;
            for (i, p) in pages.into_iter().enumerate() {
                let t = Instant::now();
                let run = process_page(&mut pipe, &p.image, p.text, stats)?;
                let secs = t.elapsed().as_secs_f64() + per_read;
                stats.page_secs.push(secs);
                stats.pages += 1;
                doc_secs += secs;
                score_page(&label, i as u32, &run, &pipe.pack, stats);
                layout_stats.pages += 1;
                score_page(&label, i as u32, &run, &pipe.pack, layout_stats);
                if let Some(d) = &o.masked_out {
                    let stem = pdf.trim_end_matches(".pdf");
                    eval::image::overlay(&run.masked, &label, i as u32, &run.rects, &pipe.pack)
                        .save(d.join(format!("{stem}_p{}_check.png", i + 1)))?;
                }
                masked_pages.push(run.masked);
            }
            // 마스킹 결과 PDF (래스터 이미지만) — 문서 처리 시간에 포함
            let t = Instant::now();
            let out_pdf = mask::image_pdf(&masked_pages, 200.0)?;
            doc_secs += t.elapsed().as_secs_f64();
            if let Some(d) = &o.masked_out {
                fs::write(d.join(pdf), out_pdf)?;
            }
        } else {
            for (i, pg) in m["pages"].as_array().into_iter().flatten().enumerate() {
                let file = pg["file"].as_str().unwrap_or_default();
                let t = Instant::now();
                let (img, text) = ocr_image(&engine, &o.set.join(file), stats)?;
                let run = process_page(&mut pipe, &img, text, stats)?;
                // 개발 확인용 (합성 데이터): 큰 마스크 사각형과 그 탐지
                if std::env::var("PII_DEBUG").is_ok_and(|v| file.starts_with(&v)) {
                    for l in &run.text.lines {
                        let r = l.poly;
                        eprintln!(
                            "LINE h={:.0} w={:.0} {:?}",
                            r[2][1] - r[0][1],
                            r[1][0] - r[0][0],
                            l.text
                        );
                    }
                    let pt = mask::page_text(&run.text);
                    let chars: Vec<char> = pt.text.chars().collect();
                    for d in &run.detections {
                        let v: String = chars[d.start.0.min(chars.len())..d.end.0.min(chars.len())]
                            .iter()
                            .collect();
                        eprintln!("DET {:?} {:?} {:?}", d.detector, d.label, v);
                    }
                    for r in &run.rects {
                        eprintln!(
                            "RECT {:.0}x{:.0} at {:.0},{:.0}",
                            r[2] - r[0],
                            r[3] - r[1],
                            r[0],
                            r[1]
                        );
                    }
                }
                let secs = t.elapsed().as_secs_f64();
                stats.page_secs.push(secs);
                stats.pages += 1;
                doc_secs += secs;
                score_page(&label, i as u32, &run, &pipe.pack, stats);
                layout_stats.pages += 1;
                score_page(&label, i as u32, &run, &pipe.pack, layout_stats);
                if let Some(d) = &o.masked_out {
                    let stem = file.rsplit_once('.').map_or(file, |x| x.0);
                    run.masked.save(d.join(format!("{stem}.png")))?;
                    eval::image::overlay(&run.masked, &label, i as u32, &run.rects, &pipe.pack)
                        .save(d.join(format!("{stem}_check.png")))?;
                }
            }
        }
        stats.doc_secs.push(doc_secs);
        if (n + 1) % 10 == 0 {
            eprintln!("{}건 ({:.0}초)", n + 1, t_all.elapsed().as_secs_f64());
        }
    }
    let mut conditions = BTreeMap::new();
    conditions.insert(
        "ocr".into(),
        "oar-ocr 0.10.2, PP-OCRv5 server_det + 한국어 rec (monkt/paddleocr-onnx), CPU".into(),
    );
    conditions.insert("ner".into(), "KoELECTRA INT8 ONNX, CPU 4스레드".into());
    conditions.insert(
        "slm".into(),
        o.slm_url
            .as_deref()
            .map_or("끔".into(), |u| format!("llama-server {u} (Qwen3.5-4B Q4_K_M, Vulkan, 프롬프트 캐시 켬: 시스템 프롬프트 공통·문서 본문 매번 다름)")),
    );
    conditions.insert(
        "처리".into(),
        "문서·쪽 순차 처리, 파일 저장·채점 시간 제외 (D0는 결과 PDF 생성 포함)".into(),
    );
    let out = Output {
        set: o.set.display().to_string(),
        conditions,
        model_load_secs,
        peak_working_set_mb_self: peak_mb(std::process::id()),
        peak_working_set_mb_llama_server: llama_pid().and_then(peak_mb),
        by_difficulty: by,
        by_layout,
    };
    if let Some(d) = o.out.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(&o.out, serde_json::to_vec_pretty(&out)?)?;
    // 구성 변화는 지표 키(e5.*)를 바꾸지 않고 변경 사항에 남긴다 (이슈 추적이 끊기지 않게)
    let note = if o.slm_section {
        format!("{} [SLM: 특약 구역 전체]", o.note)
    } else {
        o.note.clone()
    };
    crate::eval_cmd::save_image("e5", &o.set, &note, &out.by_difficulty, None)?;
    for (d, s) in &out.by_difficulty {
        let mean = |v: &Vec<f64>| v.iter().sum::<f64>() / v.len().max(1) as f64;
        println!(
            "{d}: 문서 {} 쪽 {} | 유출 {}/{} | PII 값 인식 {}/{} | 서명란 {}/{} (사업주 과잉 {}/{}) | 오탐 {}/{} | 쪽당 {:.2}초 문서당 {:.2}초",
            s.docs,
            s.pages,
            s.leaked,
            s.spans,
            s.value_in_text,
            s.spans,
            s.sign_covered,
            s.sign_boxes,
            s.employer_sign_masked,
            s.employer_sign_boxes,
            s.negatives_masked,
            s.negatives,
            mean(&s.page_secs),
            mean(&s.doc_secs)
        );
        if !s.failures.is_empty() {
            println!("    실패: {:?}", s.failures);
        }
    }
    println!(
        "모델 로딩 {:.1}초, 최대 메모리 앱 {:.0}MB / llama-server {:.0}MB",
        out.model_load_secs,
        out.peak_working_set_mb_self.unwrap_or(f64::NAN),
        out.peak_working_set_mb_llama_server.unwrap_or(f64::NAN)
    );
    Ok(())
}
