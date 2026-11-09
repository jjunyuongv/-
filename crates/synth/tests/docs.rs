//! 문서 조립(PLAN.md 4단계) 검증: 모든 레이아웃 × 여러 시드
//!
//! - 오프셋: span·참고 항목·네거티브의 값이 텍스트 그 위치 글자와 같음 (`DocLabel::offset_problems`)
//! - 시드 고정: 같은 시드 → 같은 라벨
//! - 자리표시가 남지 않음, 레이아웃별 필수 라벨, 레이아웃 D 연소근로자 나이·이름 공유

use std::collections::BTreeSet;
use std::path::PathBuf;

use pii_core::{AttributeKind, DocLabel, Label, Owner};
use synth::doc::values::CONSENT_LEAD_DAYS;
use synth::doc::{DocRequest, SynthConfig, generate, text_label};
use synth::fake::Ymd;
use synth::fake::data::is_eup_myeon;
use synth::fake::phone::area_code_for;
use synth::template::Layout;

fn cfg() -> SynthConfig {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    match SynthConfig::load(&root) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    }
}

fn cases() -> Vec<(Layout, Vec<&'static str>)> {
    vec![
        (Layout::A, vec!["standard_indefinite"]),
        (Layout::A, vec!["standard_fixed_term"]),
        (Layout::B, vec!["standard_indefinite"]),
        (Layout::C, vec!["foreign_worker"]),
        (Layout::C, vec!["foreign_worker_agri"]),
        (Layout::D, vec!["minor", "guardian_consent"]),
        (Layout::E, vec!["part_time"]),
        (Layout::F, vec!["construction_daily"]),
    ]
}

fn make(cfg: &SynthConfig, layout: Layout, forms: &[&str], seed: u64) -> DocLabel {
    let req = DocRequest {
        doc_id: format!("{layout:?}_{seed:04}"),
        layout,
        forms: forms.iter().map(|s| s.to_string()).collect(),
        seed,
        digital: false,
        extra: Default::default(),
    };
    match generate(cfg, &req) {
        Ok(d) => text_label(&d),
        Err(e) => panic!("{layout:?} {forms:?} seed {seed}: {e}"),
    }
}

fn labels(l: &DocLabel) -> BTreeSet<Label> {
    l.spans.iter().map(|s| s.label).collect()
}

#[test]
fn all_layouts_assemble_with_correct_offsets() {
    let cfg = cfg();
    let mut problems = Vec::new();
    for (layout, forms) in cases() {
        for seed in 0..40u64 {
            let l = make(&cfg, layout, &forms, seed);
            for p in l.offset_problems() {
                problems.push(format!("{} : {p}", l.doc_id));
            }
            if l.text.contains('{') || l.text.contains('}') {
                problems.push(format!("{}: 자리표시가 남음", l.doc_id));
            }
            if l.negatives.is_empty() {
                problems.push(format!("{}: 하드네거티브 없음 (특약에 최소 1개)", l.doc_id));
            }
            let ls = labels(&l);
            if !ls.contains(&Label::Name) {
                problems.push(format!("{}: NAME span 없음", l.doc_id));
            }
            match layout {
                Layout::B => {
                    for need in [Label::Rrn, Label::Email, Label::Account, Label::EmpId] {
                        if !ls.contains(&need) {
                            problems.push(format!("{}: B에 {need:?} 없음", l.doc_id));
                        }
                    }
                }
                Layout::C => {
                    for need in [
                        Label::Frn,
                        Label::Passport,
                        Label::Account,
                        Label::BirthDate,
                    ] {
                        if !ls.contains(&need) {
                            problems.push(format!("{}: C에 {need:?} 없음", l.doc_id));
                        }
                    }
                    if !l
                        .attributes
                        .iter()
                        .any(|a| a.kind == AttributeKind::Nationality)
                    {
                        problems.push(format!("{}: C에 국적 참고 항목 없음", l.doc_id));
                    }
                }
                Layout::A | Layout::E | Layout::F => {
                    if ls.contains(&Label::Rrn) || ls.contains(&Label::Email) {
                        problems.push(format!(
                            "{}: 회사 추가 항목이 없는 레이아웃에 RRN/EMAIL",
                            l.doc_id
                        ));
                    }
                }
                Layout::D => {}
            }
            // 사업주 쪽 라벨은 owner = employer
            for s in &l.spans {
                if s.source.starts_with("employer_") && s.owner != Some(Owner::Employer) {
                    problems.push(format!("{}: {} owner {:?}", l.doc_id, s.source, s.owner));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{}건:\n{}",
        problems.len(),
        problems[..problems.len().min(30)].join("\n")
    );
}

#[test]
fn same_seed_same_document() {
    let cfg = cfg();
    for (layout, forms) in cases() {
        let a = make(&cfg, layout, &forms, 7);
        let b = make(&cfg, layout, &forms, 7);
        let c = make(&cfg, layout, &forms, 8);
        assert_eq!(a, b, "{layout:?}");
        assert_ne!(a.text, c.text, "{layout:?}");
    }
}

#[test]
fn layout_d_minor_and_guardian_share_values() {
    let cfg = cfg();
    for seed in 0..40u64 {
        let l = make(&cfg, Layout::D, &["minor", "guardian_consent"], seed);
        // 근로자 이름은 계약서 머리말·서명란과 동의서 여러 곳에서 같은 값
        let names: BTreeSet<&str> = l
            .spans
            .iter()
            .filter(|s| s.label == Label::Name && s.owner == Some(Owner::Worker))
            .map(|s| s.value.as_str())
            .collect();
        assert_eq!(names.len(), 1, "seed {seed}: {names:?}");
        // 만 나이 참고 항목은 15~17
        let age: u32 = l
            .attributes
            .iter()
            .find(|a| a.kind == AttributeKind::Age)
            .map(|a| a.value.parse().unwrap())
            .unwrap();
        assert!((15..=17).contains(&age), "seed {seed}: {age}");
        assert!(
            l.attributes
                .iter()
                .any(|a| a.kind == AttributeKind::Relation)
        );
        assert_eq!(l.pages, 2);
    }
}

/// 텍스트에서 "YYYY년 M월 D일"을 모두 찾는다
fn dates_in(text: &str) -> Vec<Ymd> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let num = |i: &mut usize| {
        let s = *i;
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            *i += 1;
        }
        chars[s..*i].iter().collect::<String>().parse::<u32>().ok()
    };
    let mut i = 0;
    while i < chars.len() {
        let mut j = i;
        let parsed = (|| {
            let y = num(&mut j)?;
            (chars.get(j..j + 2)? == ['년', ' ']).then_some(())?;
            j += 2;
            let m = num(&mut j)?;
            (chars.get(j..j + 2)? == ['월', ' ']).then_some(())?;
            j += 2;
            let d = num(&mut j)?;
            (chars.get(j)? == &'일').then_some(())?;
            (y >= 1000).then_some((y, m, d))
        })();
        match parsed {
            Some((y, m, d)) => {
                out.push(Ymd {
                    year: y as u16,
                    month: m as u8,
                    day: d as u8,
                });
                i = j;
            }
            None => i += 1,
        }
    }
    out
}

#[test]
fn layout_d_consent_date_is_same_day_or_few_days_before_contract() {
    let cfg = cfg();
    let mut earlier = 0;
    for seed in 0..60u64 {
        let l = make(&cfg, Layout::D, &["minor", "guardian_consent"], seed);
        let split = l.text.find("친권자(후견인) 동의서").expect("동의서 제목");
        // 계약서 쪽 마지막 날짜 = 계약일, 동의서 쪽 마지막 날짜 = 동의일
        let contract = *dates_in(&l.text[..split]).last().expect("계약일");
        let consent = *dates_in(&l.text[split..]).last().expect("동의일");
        let lead = contract.to_days() - consent.to_days();
        assert!(
            (0..=CONSENT_LEAD_DAYS).contains(&lead),
            "seed {seed}: 계약 {contract:?} 동의 {consent:?}"
        );
        earlier += usize::from(lead > 0);
    }
    assert!(earlier > 0, "동의서가 계약일보다 앞선 경우가 한 번도 없음");
}

/// 전각 숫자를 반각으로 바꾸고 숫자 묶음만 꺼낸다
fn digit_groups(s: &str) -> Vec<String> {
    let half: String = s
        .chars()
        .map(|c| match c {
            '０'..='９' => char::from_u32(c as u32 - '０' as u32 + '0' as u32).unwrap_or(c),
            _ => c,
        })
        .collect();
    half.split(|c: char| !c.is_ascii_digit())
        .filter(|g| !g.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn company_landline_matches_workplace_sido_and_industry_region() {
    let cfg = cfg();
    let districts = &cfg.data.addresses.district;
    let mut checked = 0;
    for (layout, forms) in cases() {
        for seed in 0..40u64 {
            let l = make(&cfg, layout, &forms, seed);
            let Some(work) = l
                .attributes
                .iter()
                .find(|a| a.kind == AttributeKind::WorkplaceAddress)
            else {
                continue;
            };
            let d = districts
                .iter()
                .filter(|d| {
                    [d.sido.as_str(), d.sido_short_or_full()].iter().any(|s| {
                        let head = if d.sigungu.is_empty() {
                            format!("{s} ")
                        } else {
                            format!("{s} {} ", d.sigungu)
                        };
                        work.value.starts_with(&head)
                    })
                })
                .max_by_key(|d| d.sigungu.len())
                .unwrap_or_else(|| panic!("{}: 모르는 근무지 주소", l.doc_id));
            // 농업·축산업·어업 → 읍·면, 사무 → 특별·광역시 자치구, 건설 → 시·군
            match l.industry.as_str() {
                "농축수산" => assert!(
                    work.value.split(' ').any(is_eup_myeon),
                    "{}: {}",
                    l.doc_id,
                    work.value
                ),
                "사무" => assert!(d.is_city_center(), "{}: {}", l.doc_id, work.value),
                "건설" => assert!(!d.is_city_center(), "{}: {}", l.doc_id, work.value),
                _ => {}
            }
            // 회사 유선 대표번호의 지역번호 = 회사 주소 시·도
            for n in l.negatives.iter().filter(|n| n.source == "employer_phone") {
                let g = digit_groups(&n.value);
                if g.len() == 3 && g[0] != "070" {
                    assert_eq!(
                        Some(g[0].as_str()),
                        area_code_for(&d.sido, &d.sigungu),
                        "{}: {}",
                        l.doc_id,
                        work.value
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 10, "회사 유선 번호 검사 {checked}건");
}

#[test]
fn insurance_checks_are_mostly_all_four() {
    let cfg = cfg();
    let n = 300u64;
    let mut all = 0;
    for seed in 0..n {
        let l = make(&cfg, Layout::D, &["minor", "guardian_consent"], seed);
        let line = l
            .text
            .lines()
            .find(|t| t.contains("고용보험") && t.contains("건강보험"))
            .expect("사회보험 줄");
        all += usize::from(line.matches('■').count() == 4);
    }
    let p = all as f64 / n as f64;
    assert!((p - 0.9).abs() < 0.06, "4개 모두 체크 {p}");
}

#[test]
fn foreign_workers_are_20_to_45() {
    let cfg = cfg();
    let mut checked = 0;
    for seed in 0..80u64 {
        let l = make(&cfg, Layout::C, &["foreign_worker"], seed);
        // 외국인등록번호 앞 6자리 = 생년월일, 7번째 5·6 → 1900년대, 7·8 → 2000년대
        let Some(frn) = l.spans.iter().find(|s| s.label == Label::Frn) else {
            continue;
        };
        let g = digit_groups(&frn.value).concat();
        let century = if matches!(&g[6..7], "5" | "6") {
            1900
        } else {
            2000
        };
        let birth = Ymd {
            year: century + g[0..2].parse::<u16>().unwrap(),
            month: g[2..4].parse().unwrap(),
            day: g[4..6].parse().unwrap(),
        };
        // 계약일 = 문서 끝 서명 날짜
        let contract = *dates_in(&l.text).last().expect("계약일");
        let age = birth.age_at(contract);
        assert!((20..=45).contains(&age), "seed {seed}: 만 {age}세");
        checked += 1;
    }
    assert!(checked > 10, "외국인등록번호가 있는 문서 {checked}건");
}

#[test]
fn ratios_drive_handwriting_and_employer_kind() {
    let cfg = cfg();
    let n = 400u64;
    let mut hand_docs = 0;
    let mut corp = 0;
    for seed in 0..n {
        let l = make(&cfg, Layout::A, &["standard_indefinite"], seed);
        hand_docs += usize::from(l.handwriting_doc);
        corp += usize::from(l.employer_kind == "corporation");
        if !l.handwriting_doc {
            assert!(l.spans.iter().all(|s| !s.hand), "{}", l.doc_id);
        }
    }
    let ph = hand_docs as f64 / n as f64;
    let pc = corp as f64 / n as f64;
    assert!((ph - 0.4).abs() < 0.07, "손글씨 문서 {ph}");
    assert!((pc - 0.6).abs() < 0.07, "법인 {pc}");
}

/// 확인용: 레이아웃별 예시 라벨 JSON 출력 (cargo test -p synth --test docs -- --ignored --nocapture)
#[test]
#[ignore = "확인용 출력"]
fn print_samples() {
    let cfg = cfg();
    for (layout, forms) in [
        (Layout::D, vec!["minor", "guardian_consent"]),
        (Layout::C, vec!["foreign_worker"]),
    ] {
        let l = make(&cfg, layout, &forms, 3);
        println!("{}", serde_json::to_string_pretty(&l).unwrap());
    }
}

/// 텍스트에서 `숫자 + suffix`를 모두 찾는다 (예: "시간")
fn nums_before(text: &str, suffix: &str) -> Vec<u32> {
    let chars: Vec<char> = text.chars().collect();
    let suf: Vec<char> = suffix.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_digit()) {
            let s = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if chars.get(i..i + suf.len()) == Some(&suf[..]) {
                let n: String = chars[s..i].iter().collect();
                out.push(n.parse().unwrap_or_else(|e| panic!("{e}")));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// "N시 M분" 시각을 모두 분 단위로
fn times_in(text: &str) -> Vec<u32> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let num = |i: &mut usize| {
        let s = *i;
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            *i += 1;
        }
        chars[s..*i].iter().collect::<String>().parse::<u32>().ok()
    };
    while i < chars.len() {
        if !chars[i].is_ascii_digit() || (i > 0 && chars[i - 1].is_ascii_digit()) {
            i += 1;
            continue;
        }
        let mut j = i;
        let t = (|| {
            let h = num(&mut j)?;
            (chars.get(j..j + 2)? == ['시', ' ']).then_some(())?;
            j += 2;
            let m = num(&mut j)?;
            (chars.get(j)? == &'분').then_some(h * 60 + m)
        })();
        match t {
            Some(t) => {
                out.push(t);
                i = j;
            }
            None => i += 1,
        }
    }
    out
}

#[test]
fn minor_work_hours_within_legal_limits() {
    // 근로기준법 제69조: 연소근로자 하루 7시간·주 35시간 이하
    let cfg = cfg();
    for seed in 0..80u64 {
        let l = make(&cfg, Layout::D, &["minor", "guardian_consent"], seed);
        let line = l
            .text
            .lines()
            .find(|t| t.contains("4. 소정근로시간"))
            .unwrap();
        let t = times_in(line);
        assert_eq!(t.len(), 4, "seed {seed}: {line}");
        let daily = t[1] - t[0] - (t[3] - t[2]);
        let days_line = l.text.lines().find(|t| t.contains("근무일/휴일")).unwrap();
        let days = nums_before(days_line, "일(")[0];
        assert!(daily <= 7 * 60, "seed {seed}: 하루 {daily}분");
        assert!(
            days * daily <= 35 * 60,
            "seed {seed}: 주 {days}일 × {daily}분"
        );
    }
}

#[test]
fn part_time_hours_equal_end_minus_start_minus_rest() {
    let cfg = cfg();
    for seed in 0..60u64 {
        let l = make(&cfg, Layout::E, &["part_time"], seed);
        let a = l.text.find("근로시간\t").expect("근로시간 표");
        let b = a + l.text[a..].find("주휴일").expect("주휴일");
        let seg = &l.text[a..b];
        let hours = nums_before(seg, "시간");
        let t = times_in(seg);
        let n = hours.len();
        assert_eq!(t.len(), n * 4, "seed {seed}");
        for d in 0..n {
            let (s, e) = (t[d], t[n + d]);
            let (rs, re) = (t[2 * n + 2 * d], t[2 * n + 2 * d + 1]);
            assert_eq!(e - s - (re - rs), hours[d] * 60, "seed {seed} {d}번째 요일");
            assert!(s < rs && re <= e, "seed {seed}: 휴게가 근로 시간 안");
        }
    }
}

/// 줄에서 `from` 다음 처음 나오는 `to`까지에 숫자가 있는가
fn has_digit_between(line: &str, from: &str, to: &str) -> bool {
    let Some(a) = line.find(from) else {
        return false;
    };
    let rest = &line[a + from.len()..];
    let end = rest.find(to).unwrap_or(rest.len());
    rest[..end].chars().any(|c| c.is_ascii_digit())
}

#[test]
fn bonus_and_allowance_amounts_follow_checks() {
    let cfg = cfg();
    let mut checked = 0;
    for (layout, forms) in cases() {
        for seed in 0..40u64 {
            let l = make(&cfg, layout, &forms, seed);
            let lines: Vec<&str> = l.text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                // 상여금 : 있음 (V) 100,000원, 없음 ( )
                if line.contains("상여금 : 있음") {
                    let no = line.trim_end().ends_with("(V)");
                    assert_eq!(
                        has_digit_between(line, "있음", "원"),
                        !no,
                        "{}: {line}",
                        l.doc_id
                    );
                    checked += 1;
                }
                // 기타급여(제수당 등) : 있음 (V), 없음 ( ) + 다음 줄 "· 수당 금액원, …"
                if line.contains("기타급여(제수당 등) : 있음 (") {
                    let yes = line.contains("있음 (V)");
                    let next = lines.get(i + 1).copied().unwrap_or("");
                    assert_eq!(
                        next.chars().any(|c| c.is_ascii_digit()),
                        yes,
                        "{}: {next}",
                        l.doc_id
                    );
                    checked += 1;
                }
                // 단시간: 기타급여(제수당 등) : 있음 : 50,000원(내역별 기재), 없음 (V),
                if line.contains("기타급여(제수당 등) : 있음 :") {
                    let no = line.contains("없음 (V)");
                    assert_eq!(
                        has_digit_between(line, "있음 :", "원"),
                        !no,
                        "{}: {line}",
                        l.doc_id
                    );
                    checked += 1;
                }
                // 외국인 농업: [V]지급(상여금: …원, 수당: …원), [ ]미지급
                if line.contains("상여금 및 수당:") {
                    let no = line.contains("[V]미지급");
                    assert_eq!(
                        has_digit_between(line, "지급(", ")"),
                        !no,
                        "{}: {line}",
                        l.doc_id
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 200, "검사 {checked}건");
}

#[test]
fn partial_insurance_always_keeps_industrial_accident() {
    let cfg = cfg();
    let mut partial = 0;
    for (layout, forms) in [
        (Layout::D, vec!["minor", "guardian_consent"]),
        (Layout::E, vec!["part_time"]),
        (Layout::F, vec!["construction_daily"]),
    ] {
        for seed in 0..200u64 {
            let l = make(&cfg, layout, &forms, seed);
            let Some(line) = l
                .text
                .lines()
                .find(|t| t.contains("고용보험") && t.contains("산재보험"))
            else {
                continue;
            };
            if line.matches('■').count() < 4 {
                partial += 1;
                assert!(line.contains("■ 산재보험"), "{}: {line}", l.doc_id);
            }
        }
    }
    assert!(partial > 10, "일부 체크 {partial}건");
}

#[test]
fn foreign_contract_months_match_change_dates() {
    let cfg = cfg();
    for forms in [vec!["foreign_worker"], vec!["foreign_worker_agri"]] {
        for seed in 0..40u64 {
            let l = make(&cfg, Layout::C, &forms, seed);
            let a = l.text.find("신규 또는 재입국자: (").expect("개월 칸");
            let months = nums_before(&l.text[a..], ")")[0];
            let b = l.text.find("사업장변경자:").expect("사업장변경자");
            let d = dates_in(&l.text[b..]);
            let (start, end) = (d[0], d[1]);
            assert_eq!(
                end.to_days(),
                start.add_months(months).add_days(-1).to_days(),
                "seed {seed}: {months}개월 {start:?} ~ {end:?}"
            );
        }
    }
}
