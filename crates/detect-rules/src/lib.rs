//! 규칙 팩 탐지 (PLAN.md 9단계, 실험 E1): 정규화 → 정규식 → 문맥 키워드 → 검증기 → 원문 위치
//!
//! - 정규식은 정규화 텍스트(`pii_core::Normalized`)에 적용하고, 결과는 대응표로 원문 글자 위치로 되돌린다
//! - `regex`는 바이트 오프셋을 주므로 글자 오프셋으로 바꾼다 (CLAUDE.md 오프셋 규칙)
//! - 검증기는 앱에 내장 (팩은 이름만). 고유식별정보 13자리는 생년월일·성별코드가 유효하면 마스킹 (체크섬 무관 — 2020.10 이후형)
//! - [`propagate`]: 찾은 값과 같은 문자열을 문서 안에서 모두 가린다 (유출 방지 우선)

use pii_core::{CharOffset, Detection, Detector, Label, Normalized};
use rulepack::{Pattern, RulePack};

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// 주민·외국인등록번호 13자리 (뒤 6자리는 `*`로 가려져 있을 수 있음) → RRN/FRN, 아니면 None
pub fn validate_id13(v: &str) -> Option<Label> {
    let c: Vec<char> = v
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '*')
        .collect();
    if c.len() != 13 || c[..7].contains(&'*') {
        return None;
    }
    let d = |i: usize| c[i].to_digit(10).unwrap_or(0);
    let g = d(6);
    let century = match g {
        1 | 2 | 5 | 6 => 1900,
        3 | 4 | 7 | 8 => 2000,
        9 | 0 => 1800,
        _ => return None,
    };
    let (yy, mm, dd) = (d(0) * 10 + d(1), d(2) * 10 + d(3), d(4) * 10 + d(5));
    if mm == 0 || mm > 12 || dd == 0 || dd > days_in_month(century + yy, mm) {
        return None;
    }
    Some(if (5..=8).contains(&g) {
        Label::Frn
    } else {
        Label::Rrn
    })
}

/// 사업자등록번호 체크섬 + 가운데 유형 코드가 개인(01~79, 90~99)이면 BIZ_NO. 법인(81~88 등)은 하드네거티브라 버림
pub fn validate_bizno_individual(v: &str) -> Option<Label> {
    let d: Vec<u32> = v.chars().filter_map(|c| c.to_digit(10)).collect();
    if d.len() != 10 {
        return None;
    }
    const W: [u32; 9] = [1, 3, 7, 1, 3, 7, 1, 3, 5];
    let mut sum: u32 = d[..9].iter().zip(W).map(|(a, w)| a * w).sum();
    sum += d[8] * 5 / 10;
    if (10 - sum % 10) % 10 != d[9] {
        return None;
    }
    let code = d[3] * 10 + d[4];
    ((1..=79).contains(&code) || (90..=99).contains(&code)).then_some(Label::BizNo)
}

/// 운전면허: 지역명이거나 숫자 지역코드 11~28
pub fn validate_license(v: &str) -> Option<Label> {
    let d: Vec<u32> = v.chars().filter_map(|c| c.to_digit(10)).collect();
    let has_name = v.chars().next().is_some_and(|c| ('가'..='힣').contains(&c));
    let ok = if has_name {
        d.len() == 10
    } else {
        d.len() == 12 && (11..=28).contains(&(d[0] * 10 + d[1]))
    };
    ok.then_some(Label::DriverLicense)
}

/// 계좌: 숫자 10~16자리, 휴대전화 모양(01X로 시작 10~11자리)은 아님
pub fn validate_account(v: &str) -> Option<Label> {
    let d: String = v.chars().filter(char::is_ascii_digit).collect();
    let n = d.len();
    let mobile = d.starts_with("01") && (10..=11).contains(&n);
    ((10..=16).contains(&n) && !mobile).then_some(Label::Account)
}

fn validate(name: &str, v: &str, label: Label) -> Option<Label> {
    match name {
        "id13" => validate_id13(v),
        "bizno_individual" => validate_bizno_individual(v),
        "license" => validate_license(v),
        "account" => validate_account(v),
        "not_all_same" => {
            let mut it = v.chars().filter(char::is_ascii_digit);
            let first = it.next()?;
            it.any(|c| c != first).then_some(label)
        }
        _ => Some(label),
    }
}

/// 정규화 텍스트에서 바이트 → 글자 위치
fn byte_to_char_table(s: &str) -> Vec<usize> {
    let mut t = vec![0; s.len() + 1];
    let mut ci = 0;
    for (bi, ch) in s.char_indices() {
        for k in 0..ch.len_utf8() {
            t[bi + k] = ci;
        }
        ci += 1;
    }
    t[s.len()] = ci;
    t
}

fn run_pattern(
    pack: &RulePack,
    p: &Pattern,
    norm: &Normalized,
    chars: &[char],
    b2c: &[usize],
    out: &mut Vec<Detection>,
) {
    let text = &norm.text;
    for caps in p.regex.captures_iter(text) {
        let Some(whole) = caps.get(0) else { continue };
        let m = match &p.def.group {
            Some(g) => match caps.name(g) {
                Some(m) => m,
                None => continue,
            },
            None => whole,
        };
        let (ws, we) = (b2c[whole.start()], b2c[whole.end()]);
        let (mut s, mut e) = (b2c[m.start()], b2c[m.end()]);
        while s < e && chars[s].is_whitespace() {
            s += 1;
        }
        while e > s && (chars[e - 1].is_whitespace() || chars[e - 1] == ',') {
            e -= 1;
        }
        if s >= e {
            continue;
        }
        if p.def.digit_boundary {
            let digit = |i: Option<usize>| {
                i.and_then(|i| chars.get(i))
                    .is_some_and(char::is_ascii_digit)
            };
            let before = digit(ws.checked_sub(1))
                || (ws >= 2 && chars[ws - 1] == '-' && digit(Some(ws - 2)));
            let after = digit(Some(we)) || (chars.get(we) == Some(&'-') && digit(Some(we + 1)));
            if before || after {
                continue;
            }
        }
        let ctx_start = ws.saturating_sub(p.def.window);
        let ctx: String = chars[ctx_start..ws].iter().collect();
        if !p.def.need_context.is_empty()
            && !p.def.need_context.iter().any(|k| ctx.contains(k.as_str()))
        {
            continue;
        }
        if p.def.deny_context.iter().any(|k| ctx.contains(k.as_str())) {
            continue;
        }
        let value: String = chars[s..e].iter().collect();
        let label = match &p.def.validator {
            Some(v) => match validate(v, &value, p.def.label) {
                Some(l) => l,
                None => continue,
            },
            None => p.def.label,
        };
        if label == Label::Name && pack.is_not_a_name(&value) {
            continue;
        }
        let (os, oe) = norm.to_original(CharOffset(s), CharOffset(e));
        out.push(Detection {
            label,
            start: os,
            end: oe,
            detector: Detector::Rule,
        });
    }
}

/// 규칙 팩으로 탐지 (원문 글자 위치)
pub fn detect(pack: &RulePack, text: &str) -> Vec<Detection> {
    let norm = Normalized::new(text);
    let chars: Vec<char> = norm.text.chars().collect();
    let b2c = byte_to_char_table(&norm.text);
    let mut out = Vec::new();
    for p in &pack.patterns {
        run_pattern(pack, p, &norm, &chars, &b2c, &mut out);
    }
    out
}

/// 찾은 값과 같은 문자열(3글자 이상)을 문서 안 다른 위치에서도 찾아 더한다
pub fn propagate(text: &str, found: &[Detection]) -> Vec<Detection> {
    let chars: Vec<char> = text.chars().collect();
    let mut values: Vec<(Vec<char>, Label)> = Vec::new();
    for d in found {
        let v: Vec<char> = chars[d.start.0.min(chars.len())..d.end.0.min(chars.len())].to_vec();
        let core = v.iter().filter(|c| !c.is_whitespace()).count();
        if core >= 3 && !values.iter().any(|(x, _)| *x == v) {
            values.push((v, d.label));
        }
    }
    let mut out = Vec::new();
    for (v, label) in values {
        let n = v.len();
        let mut i = 0;
        while i + n <= chars.len() {
            if chars[i..i + n] == v[..] {
                if !found.iter().any(|d| d.overlaps(i, i + n)) {
                    out.push(Detection {
                        label,
                        start: CharOffset(i),
                        end: CharOffset(i + n),
                        detector: Detector::Propagated,
                    });
                }
                i += n;
            } else {
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> RulePack {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/rulepacks/kr-contract-0.1.0.toml");
        RulePack::load(&p).unwrap_or_else(|e| panic!("{e}"))
    }

    fn found(text: &str) -> Vec<(Label, String)> {
        let chars: Vec<char> = text.chars().collect();
        detect(&pack(), text)
            .into_iter()
            .map(|d| (d.label, chars[d.start.0..d.end.0].iter().collect()))
            .collect()
    }

    #[test]
    fn structured_ids_and_hard_negatives() {
        // 합성 규칙으로 만든 가짜 번호 (실제 사람과 무관)
        let f = found("주민등록번호: 900101-1234568, 외국인등록번호 ８９０２１５－５１２３４５６");
        assert!(f.contains(&(Label::Rrn, "900101-1234568".into())), "{f:?}");
        assert!(
            f.iter()
                .any(|(l, v)| *l == Label::Frn && v == "８９０２１５－５１２３４５６"),
            "{f:?}"
        );
        // 무효 날짜·법인등록번호 문맥은 버림
        assert!(found("자산번호: 901341-1234567").is_empty());
        assert!(found("법인등록번호 110111-1234567").is_empty());
        // 더 긴 번호의 일부는 잡지 않음
        assert!(
            found("번호 9001011234567890")
                .iter()
                .all(|(l, _)| *l != Label::Rrn)
        );
    }

    #[test]
    fn phones_and_negatives() {
        let f = found("연 락 처 : 010-1234-5678, 대표번호 1588-1234, 팩스 031-123-4567");
        assert_eq!(f, vec![(Label::Phone, "010-1234-5678".into())]);
    }

    #[test]
    fn names_addresses_by_form_keys() {
        let f = found(
            "성 명 : 홍길동 (서명)\n주 소 : 서울특별시 강남구 테헤란로 123, 101동 1203호 (역삼동)\n",
        );
        assert!(f.contains(&(Label::Name, "홍길동".into())), "{f:?}");
        assert!(
            f.contains(&(
                Label::Address,
                "서울특별시 강남구 테헤란로 123, 101동 1203호 (역삼동)".into()
            )),
            "{f:?}"
        );
    }

    #[test]
    fn propagation_finds_other_occurrences() {
        let text = "성 명 : 홍길동\n본인은 위 연소근로자 홍길동가 근로를 하는 것에 동의합니다.";
        let d = detect(&pack(), text);
        let p = propagate(text, &d);
        assert!(d.len() + p.len() >= 2);
    }
}
