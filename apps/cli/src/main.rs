//! `pii-mask` — 근로계약서 개인정보 마스킹 PoC CLI.
//!
//! 각 단계 구현은 PLAN.md 4장 순서대로 채운다. 구현된 것: `forms extract` (1-1), `synth` (5단계), `data-report` (6단계).

mod eval_cmd;
mod eval_text;
mod final_report;
mod forms;
mod report;
mod run_images;
mod synth_cmd;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "pii-mask", version, about = "근로계약서 개인정보 마스킹 PoC")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 표준근로계약서 양식 처리 (1단계)
    Forms {
        #[command(subcommand)]
        action: FormsAction,
    },
    /// 합성 데이터 생성 (3~5단계)
    Synth(SynthArgs),
    /// 데이터 점검 리포트 (6단계): 라벨 통계·분포·임의 비율
    DataReport(ReportArgs),
    /// 텍스트 세트 탐지 평가 (E1·E2·E4)
    EvalText(EvalTextArgs),
    /// 이미지·PDF 세트 엔드투엔드: OCR → 탐지 → 마스킹 → 채점 (E5)
    RunImages(RunImagesArgs),
    /// 13단계 판정 리포트 (results/report.md)
    FinalReport {
        #[arg(long, default_value = "results/report.md")]
        out: PathBuf,
    },
    /// OCR 실행 (8단계)
    Ocr,
    /// 개인정보 탐지: 규칙 팩·NER·SLM (9~11단계)
    Detect,
    /// 마스킹 (12단계)
    Mask,
    /// 개선 기록: 이슈·실행 로그 갱신 (results/runs → docs/issues.md, docs/run_log.md)
    Eval {
        #[command(subcommand)]
        action: EvalAction,
    },
    /// 규칙 팩 lint·서명·검증 (9단계)
    Pack,
}

#[derive(Debug, Subcommand)]
enum FormsAction {
    /// 원본 PDF에서 쪽별 텍스트(글자 위치 포함)·PNG 추출, 양식별로 나누기 (1-1)
    Extract(ExtractArgs),
}

#[derive(Debug, clap::Args)]
struct ExtractArgs {
    /// 입력 PDF (한컴 뷰어에서 뽑은 표준근로계약서 원본)
    #[arg(long, default_value = forms::DEFAULT_INPUT)]
    input: PathBuf,
    /// 출력 폴더
    #[arg(long, default_value = forms::DEFAULT_OUT)]
    out: PathBuf,
    /// PNG 해상도
    #[arg(long, default_value_t = 200.0)]
    dpi: f32,
    /// 글자 박스를 그린 확인용 PNG(page_XX.overlay.png)도 저장
    #[arg(long)]
    overlay: bool,
}

#[derive(Debug, clap::Args)]
struct SynthArgs {
    /// 생성 방식
    #[arg(long, value_enum)]
    mode: SynthMode,
    /// 생성할 문서 수
    #[arg(long)]
    n: u32,
    /// 랜덤 시드 (같은 시드면 같은 데이터)
    #[arg(long)]
    seed: u64,
    /// 출력 폴더 (비어 있어야 함). 기본: data/{mode}_s{seed}
    #[arg(long)]
    out: Option<PathBuf>,
    /// 이미지 생성 병렬 스레드 수 (결과는 스레드 수와 관계없이 같음)
    #[arg(long, default_value_t = 4)]
    threads: usize,
    /// 처음 보는 서식 시험: 이 폴더의 템플릿으로 텍스트 세트를 만든다 (--mode text만)
    #[arg(long)]
    templates: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum EvalAction {
    /// docs/issues.md 자동 영역 갱신 + 목표 미달 이슈 자동 등록 + docs/run_log.md에 실행 추가
    Issues,
    /// 지금까지의 results/*.json을 실행 기록(results/runs)으로 등록 (한 번만)
    Backfill,
    /// 로컬 관리자 페이지 생성 (results/dashboard/index.html)
    Dashboard,
}

#[derive(Debug, clap::Args)]
struct RunImagesArgs {
    /// 이미지·PDF 세트 폴더 (synth --mode image 출력)
    #[arg(long)]
    set: PathBuf,
    /// 결과 JSON
    #[arg(long)]
    out: PathBuf,
    /// 마스킹 결과 파일을 쓸 폴더 (없으면 안 씀)
    #[arg(long)]
    masked_out: Option<PathBuf>,
    /// llama-server 주소
    #[arg(long, default_value = "http://127.0.0.1:8089")]
    slm_url: String,
    /// SLM 끄기
    #[arg(long)]
    no_slm: bool,
    /// 변경 사항 (실행 기록·docs/run_log.md에 남김)
    #[arg(long, default_value = "")]
    note: String,
    /// 이미지 경로도 특약 구역 전체를 SLM에 보냄 (E4b 구성)
    #[arg(long)]
    slm_section: bool,
}

#[derive(Debug, clap::Args)]
struct EvalTextArgs {
    /// 실험: e1(규칙) / e2(규칙+NER) / e4(규칙+NER+SLM)
    #[arg(long)]
    exp: String,
    /// 텍스트 세트 폴더 (labels.jsonl)
    #[arg(long, default_value = "data/text_s43")]
    set: PathBuf,
    /// 결과 JSON (기본 results/{exp}.json)
    #[arg(long)]
    out: Option<PathBuf>,
    /// llama-server 주소 (e4)
    #[arg(long, default_value = "http://127.0.0.1:8089")]
    slm_url: String,
    /// 앞에서 n건만 (빠른 확인용)
    #[arg(long)]
    limit: Option<usize>,
    /// 변경 사항 (실행 기록·docs/run_log.md에 남김)
    #[arg(long, default_value = "")]
    note: String,
}

#[derive(Debug, clap::Args)]
struct ReportArgs {
    /// 데이터 세트 폴더 (synth 출력, 여러 번 지정)
    #[arg(long = "set", required = true)]
    sets: Vec<PathBuf>,
    /// 리포트 파일
    #[arg(long, default_value = "results/data_report.md")]
    out: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SynthMode {
    /// 이미지·PDF 렌더링 + 열화
    Image,
    /// 텍스트 + spans만 (렌더링 없음)
    Text,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Forms {
            action: FormsAction::Extract(args),
        } => forms::extract(&forms::ExtractOptions {
            input: args.input,
            out: args.out,
            dpi: args.dpi,
            overlay: args.overlay,
        }),
        Command::Synth(args) => synth_cmd::run(&synth_cmd::SynthOptions {
            image: matches!(args.mode, SynthMode::Image),
            n: args.n as usize,
            seed: args.seed,
            templates: args.templates,
            out: args.out.unwrap_or_else(|| {
                let mode = match args.mode {
                    SynthMode::Image => "image",
                    SynthMode::Text => "text",
                };
                PathBuf::from(format!("data/{mode}_s{}", args.seed))
            }),
            threads: args.threads,
        }),
        Command::DataReport(args) => report::run(&args.sets, &args.out),
        Command::FinalReport { out } => final_report::run(&out),
        Command::RunImages(a) => run_images::run(&run_images::Options {
            set: a.set,
            out: a.out,
            masked_out: a.masked_out,
            slm_url: (!a.no_slm).then_some(a.slm_url),
            note: a.note,
            slm_section: a.slm_section,
        }),
        Command::EvalText(a) => eval_text::run(&eval_text::Options {
            out: a
                .out
                .unwrap_or_else(|| PathBuf::from(format!("results/{}.json", a.exp))),
            set: a.set,
            exp: a.exp,
            slm_url: a.slm_url,
            limit: a.limit,
            note: a.note,
        }),
        Command::Ocr => not_yet("ocr", "8"),
        Command::Detect => not_yet("detect", "9~11"),
        Command::Mask => not_yet("mask", "12"),
        Command::Eval { action } => match action {
            EvalAction::Issues => eval_cmd::issues(),
            EvalAction::Backfill => eval_cmd::backfill(),
            EvalAction::Dashboard => eval_cmd::dashboard(),
        },
        Command::Pack => not_yet("pack", "9"),
    }
}

fn not_yet(name: &str, stage: &str) -> Result<()> {
    bail!("`{name}`은(는) 아직 구현되지 않았습니다 (PLAN.md {stage}단계)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_documented_commands() {
        let cli = Cli::try_parse_from(["pii-mask", "forms", "extract"]).unwrap();
        match cli.command {
            Command::Forms {
                action: FormsAction::Extract(args),
            } => {
                assert_eq!(args.input, PathBuf::from(forms::DEFAULT_INPUT));
                assert_eq!(args.out, PathBuf::from(forms::DEFAULT_OUT));
                assert_eq!(args.dpi, 200.0);
                assert!(!args.overlay);
            }
            other => panic!("forms extract로 파싱되어야 함: {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "pii-mask", "synth", "--mode", "image", "--n", "100", "--seed", "42",
        ])
        .unwrap();
        match cli.command {
            Command::Synth(args) => {
                assert!(matches!(args.mode, SynthMode::Image));
                assert_eq!(args.n, 100);
                assert_eq!(args.seed, 42);
            }
            other => panic!("synth로 파싱되어야 함: {other:?}"),
        }

        assert!(Cli::try_parse_from(["pii-mask", "eval", "issues"]).is_ok());
        for sub in ["ocr", "detect", "mask", "pack"] {
            assert!(Cli::try_parse_from(["pii-mask", sub]).is_ok(), "{sub}");
        }
    }
}
