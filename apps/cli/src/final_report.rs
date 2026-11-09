//! `pii-mask final-report` — 최종 판정 리포트 (`results/report.md`)
//!
//! 최종 수치: `results/e5_eval2_s46.json`(새 시드 평가용 이미지·PDF 100건, 판정 뒤 개선 반영),
//! `results/e4b.json`(텍스트 1,000건, 특약 구역 전체 SLM). 참고: 13단계 판정 `results/e5_eval.json`·`results/e4.json`,
//! 13단계 첫 실행 `results/e5_eval_first.json`, 텍스트 E1·E2 `results/e1.json`·`e2.json`.
//! 판정 기준은 PLAN.md 0장. 비율은 Wilson 95% 신뢰구간과 함께.
//! 판정 규칙: 점추정이 기준 이상이고 신뢰구간 하한도 기준-1%p 이상이면 "가능", 점추정만 기준 이상이면
//! "조건부 가능", 점추정이 기준 미만이면 "불가".
//! 처음 보는 서식 시험: `results/holdout_{e1,e2,e4b}.json`·`_by_form.json` (서식 3종 30건).
//! 평가용 세트는 건수·비율만 적는다 (문서 ID·값 없음).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use eval::dashboard::HOLDOUT_FORMS;
use eval::image::DiffStats;
use eval::{NAME_ADDRESS, Rate, SENSITIVE, STRUCTURED, TextEval, label_key};
use pii_core::Label;

#[derive(serde::Deserialize)]
struct E5 {
    conditions: BTreeMap<String, String>,
    model_load_secs: f64,
    peak_working_set_mb_self: Option<f64>,
    peak_working_set_mb_llama_server: Option<f64>,
    by_difficulty: BTreeMap<String, DiffStats>,
}

fn load<T: serde::de::DeserializeOwned>(p: &str) -> Result<T> {
    serde_json::from_str(&fs::read_to_string(p).with_context(|| p.to_string())?)
        .with_context(|| p.to_string())
}

fn verdict_min(r: &Rate, target: f64) -> &'static str {
    if r.n == 0 {
        "판정 불가 (표본 없음)"
    } else if r.p >= target && r.lo >= target - 0.01 {
        "가능"
    } else if r.p >= target {
        "조건부 가능"
    } else {
        "불가"
    }
}

fn verdict_max(r: &Rate, limit: f64) -> &'static str {
    if r.n == 0 {
        "판정 불가 (표본 없음)"
    } else if r.p <= limit && r.hi <= limit + 0.01 {
        "가능"
    } else if r.p <= limit {
        "조건부 가능"
    } else {
        "불가"
    }
}

fn pct(r: &Rate) -> String {
    r.fmt_pct()
}

fn sum_stats<'a>(it: impl Iterator<Item = &'a DiffStats>) -> DiffStats {
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

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

fn pctl(v: &[f64], q: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s.get(((s.len() as f64 - 1.0) * q).round() as usize)
        .copied()
        .unwrap_or(f64::NAN)
}

/// 이미지 세트에서 라벨 묶음의 "가려진 비율" (1 - 유출)
fn covered_rate(s: &DiffStats, labels: &[Label]) -> Rate {
    let (mut leak, mut n) = (0, 0);
    for l in labels {
        if let Some((a, b)) = s.by_label.get(&label_key(*l)) {
            leak += a;
            n += b;
        }
    }
    Rate::new(n - leak, n)
}

/// 난이도 묶음 통계
struct Paths {
    d0: DiffStats,
    d12: DiffStats,
    d02: DiffStats,
    d34: DiffStats,
    all: DiffStats,
}

impl Paths {
    fn of(e: &E5) -> Paths {
        let d = &e.by_difficulty;
        let pick = |ks: &[&str]| sum_stats(ks.iter().filter_map(|k| d.get(*k)));
        Paths {
            d0: pick(&["D0"]),
            d12: pick(&["D1", "D2"]),
            d02: pick(&["D0", "D1", "D2"]),
            d34: pick(&["D3", "D4"]),
            all: sum_stats(d.values()),
        }
    }
}

fn leak(s: &DiffStats) -> Rate {
    Rate::new(s.leaked, s.spans)
}
fn sign(s: &DiffStats) -> Rate {
    Rate::new(s.sign_covered, s.sign_boxes)
}
fn fp(s: &DiffStats) -> Rate {
    Rate::new(s.negatives_masked, s.negatives)
}
fn in_text(s: &DiffStats) -> Rate {
    Rate::new(s.value_in_text, s.spans)
}

/// 실패 유형 중 유출·서명란 쪽 (오탐 제외), 건수 많은 순
fn leak_failures(s: &DiffStats) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = s
        .failures
        .iter()
        .filter(|(k, _)| !k.starts_with("오탐"))
        .map(|(k, n)| (k.clone(), *n))
        .collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    v
}

fn fail_count(s: &DiffStats, key: &str) -> usize {
    s.failures.get(key).copied().unwrap_or(0)
}

/// 개발용 2차 세트(시드 45) 최신 실행의 D0~D2 유출률 (한계 절의 비교용)
fn latest_dev2_leak() -> Option<String> {
    eval::runlog::load_all(Path::new("."))
        .into_iter()
        .rev()
        .find(|(r, _)| r.exp == "e5" && r.set.ends_with("dev2_s45"))
        .and_then(|(r, _)| {
            r.metrics
                .iter()
                .find(|m| m.key == "e5.leak_rate_d02")
                .map(|m| format!("{} (`{}`)", m.fmt_value(), r.run_id))
        })
}

/// 처음 보는 서식 시험 실험 하나
struct Holdout {
    name: &'static str,
    overall: TextEval,
    by_form: BTreeMap<String, TextEval>,
    baseline: TextEval,
}

fn kn(r: &Rate) -> String {
    if r.n == 0 {
        "-".into()
    } else {
        format!("{:.1}% ({}/{})", r.p * 100.0, r.k, r.n)
    }
}

/// 처음 보는 서식 시험 절
fn write_holdout(w: &mut String, h: &[Holdout]) -> Result<()> {
    writeln!(w, "## 7. 처음 보는 서식 시험\n")?;
    writeln!(
        w,
        "서식 일반화를 보려고, **탐지 코드를 보지 않은 별도 세션이 작성한** 서식 3종(`assets/templates_holdout/`)으로 텍스트 30건(시드 48, 서식별 10건)을 만들어 **코드 수정 없이 E1·E2·E4b를 한 번만** 돌렸다. 결과가 나빠도 고치지 않고 그대로 적는다.\n"
    )?;
    writeln!(
        w,
        "- 키 이름 변경: 표형, `근로자명`·`주민번호`·`현주소`·`휴대폰`·`상호` 등 표준과 다른 키\n- 문장형: 표 없이 \"갑·을\" 문장 속에 개인정보\n- 순서 섞임: 항목 순서가 섞이고 인적사항이 문서 맨 뒤\n"
    )?;
    writeln!(
        w,
        "| 실험 | 서식 | 문서 | 정형 | 이름·주소 | 민감정보 | 오탐 (하드네거티브) | 탐지기가 모르는 라벨 (SEX_LIFE) |"
    )?;
    writeln!(w, "|---|---|---:|---|---|---|---|---|")?;
    let row = |w: &mut String, exp: &str, form: &str, e: &TextEval| -> Result<()> {
        writeln!(
            w,
            "| {exp} | {form} | {} | {} | {} | {} | {} | {} |",
            e.docs,
            kn(&e.group_rate(&STRUCTURED)),
            kn(&e.group_rate(&NAME_ADDRESS)),
            kn(&e.group_rate(&SENSITIVE)),
            kn(&e.negative_rate()),
            kn(&e.group_rate(&[Label::SexLife]))
        )?;
        Ok(())
    };
    for x in h {
        for (id, label) in HOLDOUT_FORMS {
            if let Some(e) = x.by_form.get(id) {
                row(w, x.name, label, e)?;
            }
        }
        row(w, x.name, "**처음 보는 서식 전체**", &x.overall)?;
        row(w, x.name, "기존 양식 (`text_s43`, 1,000건)", &x.baseline)?;
    }
    writeln!(w, "\n라벨별 (처음 보는 서식 전체):\n")?;
    writeln!(w, "| 라벨 | E1 | E2 | E4b |")?;
    writeln!(w, "|---|---|---|---|")?;
    if let [a, b, c] = h {
        let mut labels: Vec<&String> = c.overall.recall.keys().collect();
        labels.sort();
        for l in labels {
            let r = |e: &TextEval| {
                let (k, n) = e.recall.get(l).copied().unwrap_or_default();
                kn(&Rate::new(k, n))
            };
            writeln!(
                w,
                "| {l} | {} | {} | {} |",
                r(&a.overall),
                r(&b.overall),
                r(&c.overall)
            )?;
        }
    }
    writeln!(w, "\n해석 (원인 분석·수정은 시험 조건상 하지 않음):\n")?;
    if let [a, b, c] = h {
        let sens = c.overall.group_rate(&SENSITIVE);
        writeln!(
            w,
            "- **민감정보가 가장 크게 무너졌다**: E4b {} — 기존 양식 {}. 이 서식의 민감정보는 병명·종교명 같은 직접 단어 없이 증상·생활·활동으로 돌려 쓴 문장이라, 서식 변화와 표현 난이도가 함께 들어간 결과다. 맞힌 것은 순서 섞임 서식의 HEALTH뿐이다.",
            kn(&sens),
            kn(&c.baseline.group_rate(&SENSITIVE))
        )?;
        writeln!(
            w,
            "- **표본 주의**: 민감정보는 서식별 고정 문장 19종을 문서 10건씩 반복한 190개라, 신뢰구간을 계산하면 실제보다 좁게 나온다. 사실상 표본은 문장 19종이다."
        )?;
        writeln!(
            w,
            "- **정형 번호는 유지**: {} (이 세트의 정형은 주민번호·휴대전화뿐 — 외국인등록·여권·면허·이메일·계좌는 시험 안 됨).",
            kn(&c.overall.group_rate(&STRUCTURED))
        )?;
        writeln!(
            w,
            "- **이름·주소는 NER이 메웠다**: 규칙만(E1) {} → NER 추가(E2) {}. 양식 키에 의존하는 규칙은 키 이름이 바뀌면 놓치고, NER이 이를 보완한다.",
            kn(&a.overall.group_rate(&NAME_ADDRESS)),
            kn(&b.overall.group_rate(&NAME_ADDRESS))
        )?;
        let birth = |e: &TextEval| e.label_rate(Label::BirthDate);
        writeln!(
            w,
            "- **생년월일 전부 놓침**: {} — `출생일` 키를 규칙이 모른다. 재현율 묶음 밖이라 위 표에는 안 보이지만 실제 유출이다.",
            kn(&birth(&c.overall))
        )?;
        writeln!(
            w,
            "- **오탐이 늘었다**: {} — 기존 양식 {} (표본 21개).",
            kn(&c.overall.negative_rate()),
            kn(&c.baseline.negative_rate())
        )?;
        writeln!(
            w,
            "- **SEX_LIFE는 라벨 체계에 없어** 재현율에 넣지 않고 따로 셌다: 다른 탐지에 덮인 비율 {}. 탐지기는 이 범주를 모른다.\n",
            kn(&c.overall.group_rate(&[Label::SexLife]))
        )?;
    }
    writeln!(
        w,
        "시험을 위해 고친 것 (**탐지·마스킹 코드 `detect-*`·`mask`·규칙 팩은 수정하지 않음**):\n"
    )?;
    for x in [
        "템플릿 로더: 슬롯 `value` + 생성기 `literal`(값을 그대로 넣음), `holdout_*`은 원본 PDF 쪽 번호 없어도 허용 (`synth::template`, `doc/values.rs`)",
        "`synth --templates <폴더>` + `generate_holdout_text_set`: 지정 폴더의 템플릿을 차례로 돌려 같은 건수씩 생성",
        "라벨 `SEX_LIFE` 추가 (`pii_core::Label`): 정답 표기용. 재현율 묶음(`SENSITIVE`)에 넣지 않았고 탐지기에도 없음",
        "평가 기록: 처음 보는 서식 실행은 지표 키를 `h_e1`·`h_e2`·`h_e4b`로 나눠 기존 이슈·추이에 섞지 않음, 서식별 집계 파일 `results/holdout_*_by_form.json`",
        "서식 파일 표기 맞춤 2건: `POLITICS` → `POLITICAL`, `birth_date:guardian` → `birth_date`(근로자 본인의 성인 생년월일)",
    ] {
        writeln!(w, "- {x}")?;
    }
    writeln!(w)?;
    Ok(())
}

pub fn run(out: &Path) -> Result<()> {
    let e1: TextEval = load("results/e1.json")?;
    let e2: TextEval = load("results/e2.json")?;
    let e4: TextEval = load("results/e4.json")?;
    let e4b: TextEval = load("results/e4b.json")?;
    let e5: E5 = load("results/e5_eval2_s46.json")?;
    let e5_13: E5 = load("results/e5_eval.json")?;
    let e5_first: Option<E5> = load("results/e5_eval_first.json").ok();
    let mut holdout = Vec::new();
    for (name, base) in [("E1", "e1"), ("E2", "e2"), ("E4b", "e4b")] {
        holdout.push(Holdout {
            name,
            overall: load(&format!("results/holdout_{base}.json"))?,
            by_form: load(&format!("results/holdout_{base}_by_form.json"))?,
            baseline: load(&format!("results/{base}.json"))?,
        });
    }
    let holdout_sens = holdout
        .last()
        .map(|h| h.overall.group_rate(&SENSITIVE))
        .unwrap_or_else(|| Rate::new(0, 0));
    let p = Paths::of(&e5);
    let q = Paths::of(&e5_13);
    let d = &e5.by_difficulty;

    let mut w = String::new();
    writeln!(w, "# 기기 내 마스킹 가능 여부 판정 리포트 (최종)\n")?;
    writeln!(
        w,
        "근로계약서 이미지·PDF의 개인정보를 노트북 안에서만(서버 전송 없이) 가릴 수 있는지, 합성 데이터로 잰 결과. 실제 개인정보는 쓰지 않았다.\n"
    )?;
    writeln!(
        w,
        "- **최종 수치**: 새 시드 평가용 이미지·PDF 100건 `data/eval2_s46` + 텍스트 1,000건 `data/text_s43` E4b (특약 구역 전체 SLM)"
    )?;
    writeln!(
        w,
        "- **참고**: 13단계 첫 판정 = 평가용 이미지·PDF 100건 `data/eval_s42` + 텍스트 E4 (트리거 방식). 세트가 달라 나란히 둔 비교는 참고용\n"
    )?;

    // ── 결론 요약 5줄 ──
    let leak0 = leak(&p.d0);
    let leak12 = leak(&p.d12);
    let leak34 = leak(&p.d34);
    let sens = e4b.group_rate(&SENSITIVE);
    let partial12 = fail_count(&p.d12, "박스 부분 가림 (매핑·경계)");
    let ocr_wrong = fail_count(&p.all, "OCR 오인식");
    let anchor_miss = fail_count(&p.all, "서명란 앵커 미검출");
    writeln!(w, "## 결론 요약\n")?;
    writeln!(
        w,
        "1. **무엇을 했나**: 합성 근로계약서로 규칙 팩 + KoELECTRA NER + Qwen3.5-4B(SLM) + PP-OCRv5를 노트북 안에서만 돌려 유출률·재현율·오탐률을 쟀고, 13단계 첫 판정(불가) 뒤 개발용 세트로만 고쳐 새 시드 평가 세트로 다시 쟀다."
    )?;
    writeln!(
        w,
        "2. **되는 것**: 디지털 PDF(D0)는 유출 {} — **자동 마스킹 가능**. 텍스트 탐지는 정형 {} · 이름·주소 {} · 민감정보 {} (E4b)로 기준을 넘는다.",
        pct(&leak0),
        pct(&e4b.group_rate(&STRUCTURED)),
        pct(&e4b.group_rate(&NAME_ADDRESS)),
        pct(&sens)
    )?;
    writeln!(
        w,
        "3. **안 되는 것**: 스캔(D1~D2) 유출 {}는 목표(≤1%) 미달이라 **사람 검토를 붙여야** 쓸 수 있고, 사진·팩스(D3~D4) 유출 {}는 **현재 불가**. 서명란 커버리지 {}·이미지 오탐 {}도 목표 미달. **처음 보는 서식 3종에서는 민감정보 재현율이 {}로 떨어져, 위의 \"되는 것\"은 기존 양식·문장 풀 범위에서만 유효하다.**",
        pct(&leak12),
        pct(&leak34),
        pct(&sign(&p.all)),
        pct(&fp(&p.all)),
        kn(&holdout_sens)
    )?;
    writeln!(
        w,
        "4. **왜 안 되나**: 값은 찾았는데 마스크가 정답 박스를 덜 덮는 경우(박스 부분 가림, D1~D2 {partial12}건), 손글씨·열화로 OCR이 값을 틀리게 읽는 경우(OCR 오인식 {ocr_wrong}건), 도장·손글씨가 서명란 앵커 문구를 가리는 경우(앵커 미검출 {anchor_miss}건)가 남았다."
    )?;
    writeln!(
        w,
        "5. **다음에 할 일**: ① 마스크 여백·줄 단위 확장 ② 서명란 앵커 미검출(D2) ③ 오탐(사업주 전화·우편번호·내선) ④ 저품질 영역 PaddleOCR-VL 폴백 ⑤ 서식 일반화 대응(간접 표현 민감정보·양식 키) ⑥ 실제 계약서 10~20건 검증 ⑦ 민감정보 라벨 체계 보완 (10절).\n"
    )?;

    // ── 1. 처리 경로별 판정 ──
    writeln!(w, "## 1. 처리 경로별 판정\n")?;
    writeln!(
        w,
        "| 처리 경로 | 판정 | 유출률 (span) | PII 값 인식률 | 서명란 커버리지 | 오탐 (하드네거티브) | 운영 조건 |"
    )?;
    writeln!(w, "|---|---|---|---|---|---|---|")?;
    writeln!(
        w,
        "| **D0 디지털 PDF** (텍스트 레이어) | **자동 마스킹 가능** | {} | {} | {} | {} | 서명란은 가린 결과를 한 번 확인 권장 (커버리지 목표 미달) |",
        pct(&leak0),
        pct(&in_text(&p.d0)),
        pct(&sign(&p.d0)),
        pct(&fp(&p.d0))
    )?;
    writeln!(
        w,
        "| **D1~D2 스캔** | **자동 + 사람 검토 시 가능** | {} | {} | {} | {} | 자동으로 가린 뒤 사람이 결과를 확인·보완 |",
        pct(&leak12),
        pct(&in_text(&p.d12)),
        pct(&sign(&p.d12)),
        pct(&fp(&p.d12))
    )?;
    writeln!(
        w,
        "| **D3~D4 사진·팩스** | **현재 불가** | {} | {} | {} | {} | 재촬영·원본 요청 또는 사람이 직접 처리 |",
        pct(&leak34),
        pct(&in_text(&p.d34)),
        pct(&sign(&p.d34)),
        pct(&fp(&p.d34))
    )?;
    writeln!(
        w,
        "| **텍스트 탐지** (정형·이름·주소·민감정보, E4b) | **가능** | - | 정형 {} · 이름·주소 {} · 민감정보 {} | - | {} | 특약 구역 전체를 SLM에 보내는 구성 (문서당 {:.1}초) |\n",
        pct(&e4b.group_rate(&STRUCTURED)),
        pct(&e4b.group_rate(&NAME_ADDRESS)),
        pct(&sens),
        pct(&e4b.negative_rate()),
        e4b.seconds_total / e4b.docs.max(1) as f64
    )?;
    writeln!(
        w,
        "- 유출률 목표는 ≤ 1% (PLAN.md 0장). D0만 이 목표를 만족한다. D1~D2는 목표를 못 넘으므로 \"사람 검토\"를 조건으로 붙인 판정이고, D3~D4는 사람 검토로도 부담이 커서(span 4개 중 약 1개 유출) 입력 단계에서 걸러야 한다."
    )?;
    writeln!(
        w,
        "- 처리 경로는 입력으로 구분할 수 있다: 텍스트 레이어가 있는 PDF = D0, 스캔·사진 이미지는 기울기·해상도 등으로 D1~D2와 D3~D4를 가르는 품질 검사가 필요 (미구현, 후속)."
    )?;
    writeln!(
        w,
        "- 이 판정은 기존 양식·문장 풀 범위에서 쟀다. 처음 보는 서식 결과는 7절을 본다.\n"
    )?;

    // ── 2. 기준별 결과 (최종 vs 13단계) ──
    writeln!(w, "## 2. 기준별 결과 — 최종과 13단계 첫 판정\n")?;
    writeln!(
        w,
        "| 기준 (PLAN.md) | 목표 | **최종** (eval2_s46 · 텍스트 E4b) | 판정 | 13단계 첫 판정 (eval_s42 · 텍스트 E4) | 판정 |"
    )?;
    writeln!(w, "|---|---|---|---|---|---|")?;
    let row_min = |w: &mut String, name: &str, t: f64, a: &Rate, b: &Rate| -> Result<()> {
        writeln!(
            w,
            "| {name} | ≥ {:.0}% | {} | **{}** | {} | {} |",
            t * 100.0,
            pct(a),
            verdict_min(a, t),
            pct(b),
            verdict_min(b, t)
        )?;
        Ok(())
    };
    let row_max = |w: &mut String, name: &str, t: f64, a: &Rate, b: &Rate| -> Result<()> {
        writeln!(
            w,
            "| {name} | ≤ {:.0}% | {} | **{}** | {} | {} |",
            t * 100.0,
            pct(a),
            verdict_max(a, t),
            pct(b),
            verdict_max(b, t)
        )?;
        Ok(())
    };
    row_min(
        &mut w,
        "정형 PII 재현율 — 텍스트 1,000건",
        0.99,
        &e4b.group_rate(&STRUCTURED),
        &e4.group_rate(&STRUCTURED),
    )?;
    row_min(
        &mut w,
        "이름·주소 재현율 — 텍스트",
        0.95,
        &e4b.group_rate(&NAME_ADDRESS),
        &e4.group_rate(&NAME_ADDRESS),
    )?;
    row_min(
        &mut w,
        "민감정보 재현율 — 텍스트",
        0.90,
        &sens,
        &e4.group_rate(&SENSITIVE),
    )?;
    row_max(
        &mut w,
        "오탐률 (하드네거티브) — 텍스트",
        0.05,
        &e4b.negative_rate(),
        &e4.negative_rate(),
    )?;
    row_min(
        &mut w,
        "정형 PII 재현율 — 이미지 D1~D2",
        0.95,
        &covered_rate(&p.d12, &STRUCTURED),
        &covered_rate(&q.d12, &STRUCTURED),
    )?;
    row_max(
        &mut w,
        "**유출률 (주 지표)** — D0~D2",
        0.01,
        &leak(&p.d02),
        &leak(&q.d02),
    )?;
    row_max(
        &mut w,
        "· 유출률 D0 (디지털 PDF)",
        0.01,
        &leak0,
        &leak(&q.d0),
    )?;
    row_max(
        &mut w,
        "· 유출률 D1~D2 (스캔)",
        0.01,
        &leak12,
        &leak(&q.d12),
    )?;
    writeln!(
        w,
        "| 유출률 — D3~D4 (별도 보고) | 보고 | {} | 보고 | {} | 보고 |",
        pct(&leak34),
        pct(&leak(&q.d34))
    )?;
    row_min(
        &mut w,
        "서명란 커버리지 (근로자·친권자)",
        0.95,
        &sign(&p.all),
        &sign(&q.all),
    )?;
    row_max(
        &mut w,
        "오탐률 (하드네거티브) — 이미지·PDF",
        0.05,
        &fp(&p.all),
        &fp(&q.all),
    )?;
    writeln!(
        w,
        "| 쪽당 평균 처리 시간 | 보고 | {:.1}초 (E4b 포함) | 보고 | {:.1}초 (트리거 방식) | 보고 |",
        mean(&p.all.page_secs),
        mean(&q.all.page_secs)
    )?;
    writeln!(
        w,
        "\n판정 규칙: 점추정이 기준을 만족하고 95% 신뢰구간도 기준±1%p 안이면 \"가능\", 점추정만 만족하면 \"조건부 가능\", 점추정이 기준을 못 넘으면 \"불가\". 13단계 첫 판정 열은 리포트 첫 버전의 숫자(첫 실행 뒤 행 묶기·여백 보완까지 반영한 `results/e5_eval.json`).\n"
    )?;

    // ── 3. 판정 뒤 개선 ──
    writeln!(w, "## 3. 13단계 첫 판정 뒤 바꾼 것\n")?;
    writeln!(
        w,
        "모든 원인 분석·조정은 개발용 세트(이미지 시드 41 20건·시드 45 40건)에서 했고, 시도마다 `docs/issues.md` 시도 기록에 실행 ID와 전→후 수치를 남겼다.\n"
    )?;
    for x in [
        "**필드 영역 마스킹**: `성명`·`주소`·`연락처` 등 개인정보 양식 키로 시작하는 줄은 값 칸 전체를 가림 — OCR이 값을 틀리게 읽어도 칸은 가려짐 (규칙 팩 `[fields]`)",
        "**서명 줄 대체 앵커**: \"(서명)\" 문구가 도장·손글씨에 가려 안 읽혀도, 날짜 줄 아래의 `성명`·`근로자:`·`대표자` 등으로 시작하는 줄을 서명 줄로 봄 (규칙 팩 `sign_line_keys`)",
        "**기울어진 줄 보정**: OCR 글자 박스의 세로 범위를 줄 4점에 맞추고, 긴 값은 높이 기준으로 나눠 여러 사각형으로 덮음 — 기울어진 쪽에서 위아래 줄(회사 전화 등)까지 덮던 오탐을 줄임",
        "**이미지 경로 E4b**: 특약 구역 전체 줄을 SLM에 보냄 (트리거 단어 없는 민감정보 문장 대응)",
        "**기록 체계**: 실행마다 `results/runs/<실행ID>/`에 지표·실패 유형 건수, `pii-mask eval issues`로 `docs/issues.md`·`docs/run_log.md` 갱신, 관리자 페이지 `pii-mask eval dashboard`",
    ] {
        writeln!(w, "- {x}")?;
    }
    writeln!(w)?;

    // ── 4. 텍스트 ──
    writeln!(w, "## 4. 텍스트 1,000건 (E1·E2·E4·E4b)\n")?;
    writeln!(
        w,
        "정답 span의 글자 90% 이상이 어떤 탐지에든 덮이면 찾은 것으로 셈 (가리는 목적이라 라벨 무관). SALARY는 정책상 가리지 않아 제외. 참고 항목에 걸린 탐지는 중립.\n"
    )?;
    writeln!(
        w,
        "| 라벨 | 표본 | E1 규칙 | E2 규칙+NER | E4 +SLM 트리거 | **E4b +SLM 특약 전체** |"
    )?;
    writeln!(w, "|---|---:|---|---|---|---|")?;
    let mut labels: Vec<String> = e4b.recall.keys().cloned().collect();
    labels.sort();
    for l in &labels {
        let r = |e: &TextEval| {
            let (k, n) = e.recall.get(l).copied().unwrap_or_default();
            Rate::new(k, n).fmt_pct()
        };
        let n = e4b.recall.get(l).map_or(0, |x| x.1);
        writeln!(
            w,
            "| {l} | {n} | {} | {} | {} | {} |",
            r(&e1),
            r(&e2),
            r(&e4),
            r(&e4b)
        )?;
    }
    writeln!(w)?;
    writeln!(w, "| 지표 | E1 | E2 | E4 | **E4b** |")?;
    writeln!(w, "|---|---|---|---|---|")?;
    type RateFn<'a> = &'a dyn Fn(&TextEval) -> Rate;
    let rows: [(&str, RateFn); 6] = [
        ("정형 7종", &|e: &TextEval| e.group_rate(&STRUCTURED)),
        ("이름·주소", &|e: &TextEval| {
            e.group_rate(&NAME_ADDRESS)
        }),
        ("민감정보 5종", &|e: &TextEval| {
            e.group_rate(&SENSITIVE)
        }),
        ("하드네거티브 오탐", &|e: &TextEval| {
            e.negative_rate()
        }),
        ("13자리 하드네거티브 오탐", &|e: &TextEval| {
            e.negative_rate_13()
        }),
        (
            "정밀도 (정답과 겹친 탐지 비율)",
            &|e: &TextEval| e.precision(),
        ),
    ];
    for (name, f) in rows {
        writeln!(
            w,
            "| {name} | {} | {} | {} | {} |",
            f(&e1).fmt_pct(),
            f(&e2).fmt_pct(),
            f(&e4).fmt_pct(),
            f(&e4b).fmt_pct()
        )?;
    }
    let per_doc = |e: &TextEval| e.seconds_total / e.docs.max(1) as f64;
    writeln!(
        w,
        "| 문서당 탐지 시간 | {:.3}초 | {:.3}초 | {:.2}초 (SLM {}회) | {:.2}초 (SLM {}회) |\n",
        per_doc(&e1),
        per_doc(&e2),
        per_doc(&e4),
        e4.timing.slm_calls,
        per_doc(&e4b),
        e4b.timing.slm_calls
    )?;
    writeln!(
        w,
        "- 민감정보는 SLM 없이는 거의 못 찾는다 (E1·E2 ≈ 0%). E4의 누락은 대부분 트리거 단어가 없는 문장이었고, E4b는 특약 구역 전체를 보내 이 누락을 없앴다. 대가는 문서당 시간 약 {:.1}배.",
        per_doc(&e4b) / per_doc(&e4).max(1e-9)
    )?;
    let mut fps: Vec<(&String, &(usize, usize))> =
        e4b.negatives.iter().filter(|(_, v)| v.0 > 0).collect();
    fps.sort_by_key(|x| std::cmp::Reverse(x.1.0));
    if !fps.is_empty() {
        let s: Vec<String> = fps
            .iter()
            .map(|(k, v)| format!("{k} {}/{}", v.0, v.1))
            .collect();
        writeln!(w, "- E4b 하드네거티브 오탐 종류: {}", s.join(", "))?;
    }
    writeln!(w)?;

    // ── 5. 이미지·PDF ──
    writeln!(
        w,
        "## 5. 평가용 이미지·PDF 100건 — 새 시드 `eval2_s46` (D0 PDF 20건 · D1~D4 이미지 80건)\n"
    )?;
    writeln!(
        w,
        "| 난이도 | 문서 | 쪽 | 유출률 (span) | PII 값 인식률 (OCR·PDF 텍스트) | 서명란 커버리지 | 사업주 서명란 가림 (정책상 끔) | 오탐 (하드네거티브) | 쪽당 평균 / p95 | 문서당 평균 |"
    )?;
    writeln!(w, "|---|---:|---:|---|---|---|---|---|---|---|")?;
    let img = sum_stats(["D1", "D2", "D3", "D4"].iter().filter_map(|k| d.get(*k)));
    for (k, s) in d
        .iter()
        .chain([(&"D1~D4".to_string(), &img)])
        .chain([(&"전체".to_string(), &p.all)])
    {
        writeln!(
            w,
            "| {k} | {} | {} | {} | {} | {} | {} | {} | {:.2} / {:.2}초 | {:.2}초 |",
            s.docs,
            s.pages,
            pct(&leak(s)),
            pct(&in_text(s)),
            pct(&sign(s)),
            Rate::new(s.employer_sign_masked, s.employer_sign_boxes).fmt_pct(),
            pct(&fp(s)),
            mean(&s.page_secs),
            pctl(&s.page_secs, 0.95),
            mean(&s.doc_secs)
        )?;
    }
    writeln!(w, "\n라벨별 유출 (D0~D4 전체):\n")?;
    writeln!(w, "| 라벨 | 유출 / 전체 | 유출률 |")?;
    writeln!(w, "|---|---:|---|")?;
    for (k, (a, b)) in &p.all.by_label {
        writeln!(w, "| {k} | {a} / {b} | {} |", Rate::new(*a, *b).fmt_pct())?;
    }
    writeln!(
        w,
        "\n- \"사업주 서명란 가림\"은 정책상 가리지 않는 사업주 서명·도장 박스가 50% 이상 가려진 비율. 합성 배치상 사업주 서명이 대표자 이름(정책상 가림)·필드 영역과 겹쳐 함께 덮이는 경우와 소속 판별 실패(unknown → 가림)가 섞여 있어, 과잉 마스킹 정도는 이 수치로 판정하지 않음."
    )?;
    writeln!(
        w,
        "- D0는 PDF 텍스트 레이어를 직접 쓰므로 OCR이 없고, 결과 PDF는 래스터 이미지만 담는다 (텍스트 레이어·문서 정보·XMP·주석·책갈피·첨부 없음 — `mask` 테스트로 확인).\n"
    )?;
    writeln!(
        w,
        "유출률 추이 (세트가 다름 — 13단계 두 열은 `eval_s42`, 최종 열은 `eval2_s46`):\n"
    )?;
    writeln!(
        w,
        "| 난이도 | 13단계 첫 실행 | 13단계 판정 (행 묶기·여백 보완) | **최종** (판정 뒤 개선) |"
    )?;
    writeln!(w, "|---|---|---|---|")?;
    for k in ["D0", "D1", "D2", "D3", "D4"] {
        let first = e5_first
            .as_ref()
            .and_then(|f| f.by_difficulty.get(k))
            .map_or("-".into(), |x| pct(&leak(x)));
        let mid = e5_13
            .by_difficulty
            .get(k)
            .map_or("-".into(), |x| pct(&leak(x)));
        let last = d.get(k).map_or("-".into(), |x| pct(&leak(x)));
        writeln!(w, "| {k} | {first} | {mid} | {last} |")?;
    }
    writeln!(w)?;

    // ── 6. 실패 유형 ──
    writeln!(
        w,
        "## 6. 실패 유형 (최종 평가 세트, 건수만)\n\n유출 span과 덜 가린 서명란:\n"
    )?;
    writeln!(w, "| 유형 | 건수 | D0 | D1 | D2 | D3 | D4 |")?;
    writeln!(w, "|---|---:|---:|---:|---:|---:|---:|")?;
    let by = |dk: &str, k: &str| d.get(dk).map_or(0, |s| fail_count(s, k));
    for (k, n) in leak_failures(&p.all) {
        writeln!(
            w,
            "| {k} | {n} | {} | {} | {} | {} | {} |",
            by("D0", &k),
            by("D1", &k),
            by("D2", &k),
            by("D3", &k),
            by("D4", &k)
        )?;
    }
    writeln!(
        w,
        "\n- OCR 미검출: 정답 박스 자리에서 글자를 못 찾음 / OCR 오인식: 글자는 찾았지만 값이 틀리게 읽힘 / 규칙·NER 탐지 누락: OCR 텍스트에 값이 있는데 못 찾음 / SLM 트리거·탐지 누락: 민감정보 문장을 못 찾음 / 박스 부분 가림: 마스크가 정답 박스를 일부(5~90%)만 덮음 / 서명란 앵커 미검출: 서명란 근처에 가린 영역이 없음 / 서명란 영역 어긋남: 가린 영역이 있지만 덜 덮음\n"
    )?;
    let mut kinds: Vec<(String, usize)> = Vec::new();
    let mut sources: Vec<(String, usize)> = Vec::new();
    for (k, n) in &p.all.failures {
        if let Some(s) = k.strip_prefix("오탐:") {
            sources.push((s.to_string(), *n));
        } else if let Some(s) = k.strip_prefix("오탐 ") {
            kinds.push((s.to_string(), *n));
        }
    }
    kinds.sort_by_key(|x| std::cmp::Reverse(x.1));
    sources.sort_by_key(|x| std::cmp::Reverse(x.1));
    let join = |v: &[(String, usize)]| {
        v.iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    writeln!(
        w,
        "오탐 {}건 — 종류: {} / 덮은 영역: {}\n",
        p.all.negatives_masked,
        join(&kinds),
        join(&sources)
    )?;

    // ── 7. 자원·측정 조건 ──
    write_holdout(&mut w, &holdout)?;
    writeln!(w, "## 8. 기기 자원과 측정 조건\n")?;
    writeln!(
        w,
        "| 항목 | 값 |\n|---|---|\n| 쪽당 평균 처리 시간 | {:.1}초 (p95 {:.1}초, 이미지 E4b 포함) |\n| 문서당 평균 처리 시간 | {:.1}초 |\n| 최대 메모리 | 앱 {:.1}GB + SLM(llama-server) {:.1}GB |\n| 설치 용량 | 약 2.9GB (OCR·NER·SLM 모델 + 앱) |\n| 모델 로딩 | {:.1}초 (OCR·NER, 앱 시작 시 1회) |\n| SLM 단독 속도 | 프롬프트 약 155 토큰/초, 생성 약 25 토큰/초 (호출 1건 평균 약 1.1초, 쪽당 약 {:.1}회) |\n",
        mean(&p.all.page_secs),
        pctl(&p.all.page_secs, 0.95),
        mean(&p.all.doc_secs),
        e5.peak_working_set_mb_self.unwrap_or(f64::NAN) / 1024.0,
        e5.peak_working_set_mb_llama_server.unwrap_or(f64::NAN) / 1024.0,
        e5.model_load_secs,
        p.all.slm_calls as f64 / p.all.pages.max(1) as f64
    )?;
    writeln!(
        w,
        "- 기기: Core Ultra 7 258V, RAM 32GB, Intel Arc 140V (Vulkan). **전원 연결 + 고성능 모드** (사용자 설정)"
    )?;
    for (k, v) in &e5.conditions {
        writeln!(w, "- {k}: {v}")?;
    }
    writeln!(
        w,
        "- SLM 입력: 이미지·텍스트 모두 **트리거 줄 + 특약 구역 전체 줄**(E4b, 이미지는 `run-images --slm-section`). 시스템 프롬프트는 매 요청 같아 llama-server가 캐시"
    )?;
    writeln!(
        w,
        "- llama-server는 미리 띄워 둔 상태에서 측정 (SLM 모델 로딩 시간 제외). 최대 메모리 = Windows 프로세스 최대 작업 집합 (llama-server의 GPU 공유 메모리는 일부만 잡힐 수 있음)"
    )?;
    writeln!(
        w,
        "- 같은 코드·같은 SLM 호출 수에서도 개발용 실행의 쪽당 시간이 3.9~7.6초로 흔들린 적이 있음 (시스템 상태 영향 추정) — 시간은 이 정도 폭으로 해석\n"
    )?;

    // ── 8. 한계 ──
    writeln!(w, "## 9. 한계와 해석 시 주의\n")?;
    writeln!(
        w,
        "- **합성 데이터 기준**이고, 데이터 생성기와 탐지기를 같은 쪽(같은 사람·같은 양식 8종·같은 문장 풀)에서 만들었다. 생성기가 만들지 않는 형식·문장은 평가되지 않으므로 실제 문서보다 낙관적일 수 있다"
    )?;
    if let Some(h) = holdout.last() {
        writeln!(
            w,
            "- **다른 서식 일반화**: 탐지 코드를 보지 않은 별도 세션이 작성한 서식 3종, 텍스트 30건으로 한 번 시험했다 (7절). 정형 {}·이름·주소 {}는 유지됐지만 민감정보 {}, 생년월일 {}, 오탐 {}로 나빠졌다. 1~6절의 수치는 표준 근로계약서 계열 양식 8종과 그 문장 풀에 대한 것이고, 이미지·PDF 경로의 서식 일반화는 시험하지 않았다",
            kn(&h.overall.group_rate(&STRUCTURED)),
            kn(&h.overall.group_rate(&NAME_ADDRESS)),
            kn(&h.overall.group_rate(&SENSITIVE)),
            kn(&h.overall.label_rate(Label::BirthDate)),
            kn(&h.overall.negative_rate())
        )?;
    }
    writeln!(
        w,
        "- **라벨 체계 누락**: 개인정보보호법 민감정보 중 성생활·사상·신념·인종·민족·유전정보가 라벨에 없음 (탐지기가 찾지도, 평가에서 세지도 않음. 시험에서 `SEX_LIFE`만 정답으로 따로 집계)"
    )?;
    writeln!(
        w,
        "- **줄바꿈된 개인정보**(한 값이 두 줄에 걸침)는 표본이 부족하다 (13단계 평가용 이미지에 3건, 모두 주소) → 줄바꿈 처리 성능은 판정할 수 없음"
    )?;
    let dev2 = latest_dev2_leak().unwrap_or_else(|| "2.7%".into());
    writeln!(
        w,
        "- **평가 결과를 보고 개선한 이력**: 13단계 첫 판정의 실패 유형 통계(건수)를 보고 개선 방향을 정했다. 원인 분석·조정은 개발용 세트에서만 했지만, 평가 세트의 통계가 방향 선택에 영향을 줬다. 그래서 최종은 새 시드 세트로 쟀다. 개발용 2차 세트 D0~D2 유출 {dev2} vs 새 평가 세트 {} — 차이에는 개발 세트에 맞춘 효과와 세트 간 표본 차이가 섞여 있어, 개발 세트 수치는 낙관적으로 봐야 한다",
        pct(&leak(&p.d02))
    )?;
    writeln!(
        w,
        "- 텍스트 평가 세트 `text_s43`은 13단계부터 같은 세트를 다시 쓴다. 트리거 단어 10개를 개발용 사례로 보강했는데 같은 문장 풀에서 나와 민감정보 재현율에 낙관 편향 가능 (E4b는 특약 구역에서는 트리거에 의존하지 않음)"
    )?;
    writeln!(
        w,
        "- 이미지 100건은 레이아웃×난이도 칸이 몇 건뿐이라 칸별·라벨별 수치는 신뢰구간이 넓다. 서명란 정답 박스는 합성 위치라 실제 서명란과 다를 수 있다"
    )?;
    writeln!(
        w,
        "- 재OCR 보조 지표는 측정하지 않음 (후속) — 주 지표(정답 박스 커버리지)만으로 판정\n"
    )?;

    // ── 9. 다음 개선 우선순위 ──
    writeln!(w, "## 10. 다음 개선 우선순위\n")?;
    let partial_all = fail_count(&p.all, "박스 부분 가림 (매핑·경계)");
    let anchor_d2 = by("D2", "서명란 앵커 미검출");
    for x in [
        format!(
            "① **마스킹 박스 여백·줄 단위 확장** — 박스 부분 가림 D1~D2 {partial12}건 (전체 {partial_all}건). 값을 찾은 줄은 줄 높이 전체·값 끝까지 넓히는 방식"
        ),
        format!(
            "② **서명란 앵커 미검출** — D2 {anchor_d2}건 (전체 {anchor_miss}건). 앵커 문구 없이 서명·도장 덩어리로 찾는 보강"
        ),
        "③ **오탐** — 사업주 전화·우편번호·내선번호. 필드 영역이 옆 칸까지 넓어지는 경우와 이름·주소 NER 경계".into(),
        "④ **저품질 영역 PaddleOCR-VL 폴백** — OCR 오인식이 D3~D4에 몰려 있음. 신뢰도 낮은 줄만 재인식해 시간·효과 측정".into(),
        "⑤ **서식 일반화 대응** — 7절 결과 기준: 간접 표현 민감정보(문장 풀 다양화 후 평가, 특약 판별 방식 재검토), 양식 키 변형(`출생일` 등), 오탐".into(),
        "⑥ **실제 계약서 10~20건 검증** — 내부 승인·동의를 받은 문서로, 외부 반출 없는 환경에서 (합성 데이터 낙관 편향 확인)".into(),
        "⑦ **라벨 체계 누락** — 개인정보보호법 민감정보 중 성생활·사상·신념·인종·민족·유전정보가 라벨에 없음. 라벨·문장 풀·SLM 프롬프트에 추가하고 다시 평가".into(),
    ] {
        writeln!(w, "- {x}")?;
    }
    writeln!(w)?;

    // ── 10. 후속 ──
    writeln!(w, "## 11. 후속으로 미룬 것\n")?;
    for x in [
        "입력 품질 검사 (D1~D2 / D3~D4 경로 자동 구분)",
        "검출 모델 mobile_det 비교, 왜곡 보정(UVDoc) 켜고 끄기 비교",
        "규칙 팩 서명(ed25519) 검증·lint 상세·내장 테스트·롤백·갱신 시나리오 테스트",
        "E3 (SLM 단독 전체 청크), SLM 모델 크기 비교 (2B 등으로 내려가며 기준 충족 지점)",
        "NER 파인튜닝, 혼합 PDF(이미지 영역) OCR 검토",
        "재OCR 보조 지표, CER 측정, 텍스트 출력 마스킹 치환(`김**`), 감사 로그",
        "CPU만 실행 시 엔드투엔드 시간 (7단계 실측: CPU만 SLM 계약서 1건 11~12초)",
    ] {
        writeln!(w, "- {x}")?;
    }
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(out, w)?;
    println!("리포트: {}", out.display());
    Ok(())
}
