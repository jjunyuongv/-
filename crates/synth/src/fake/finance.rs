//! 계좌번호, 여권번호, 운전면허번호
//!
//! - 계좌번호: 은행별 묶음 형식(`banks.toml`). **체크섬이 공개되어 있지 않아** 탐지는 형식·문맥으로만 가능
//! - 여권번호: 한국 구형 `M` + 8자리, 신형(2021~) `M` + 3자리 + 영문 1자 + 4자리 / 외국 여권은 나라별 형식(`foreign.toml`)
//! - 운전면허번호: 지역코드 2 + 발급연도 2 + 일련번호 6 + 2자리 (지역코드는 숫자형, 2020년 이후 표기)

use rand::{Rng, RngExt};

use super::data::{Bank, Banks, Country};
use super::{NumberText, digits, pick};

/// 계좌 (은행 이름 + 번호)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub bank: String,
    pub number: NumberText,
}

/// 은행 하나에서 계좌번호를 만든다
pub fn account_in<R: Rng + ?Sized>(rng: &mut R, bank: &Bank) -> Account {
    let mut groups = Vec::with_capacity(bank.groups.len());
    for (i, len) in bank.groups.iter().enumerate() {
        let g = match (i, pick(rng, &bank.prefixes)) {
            (0, Some(prefix)) => prefix.clone(),
            _ => digits(rng, *len, if i == 0 { 1..=9 } else { 0..=9 }),
        };
        groups.push(g);
    }
    Account {
        bank: bank.name.clone(),
        number: NumberText::new(groups),
    }
}

/// 은행을 균등하게 골라 계좌번호를 만든다 (근로자 급여 계좌·회사 법인 계좌 모두)
pub fn account<R: Rng + ?Sized>(rng: &mut R, banks: &Banks) -> Option<Account> {
    let bank = pick(rng, &banks.bank)?;
    Some(account_in(rng, bank))
}

/// 한국 여권 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KoreanPassport {
    /// 구형: M + 8자리
    Old,
    /// 신형(2021~): M + 3자리 + 영문 1자 + 4자리
    New,
}

fn upper_letter<R: Rng + ?Sized>(rng: &mut R) -> char {
    char::from(b'A' + rng.random_range(0..26u8))
}

/// 한국 여권번호
pub fn korean_passport<R: Rng + ?Sized>(rng: &mut R, kind: KoreanPassport) -> String {
    match kind {
        KoreanPassport::Old => format!("M{}", digits(rng, 8, 0..=9)),
        KoreanPassport::New => format!(
            "M{}{}{}",
            digits(rng, 3, 0..=9),
            upper_letter(rng),
            digits(rng, 4, 0..=9)
        ),
    }
}

/// 외국 여권번호 (나라별 형식: L = 영문 대문자, # = 숫자)
pub fn foreign_passport<R: Rng + ?Sized>(rng: &mut R, c: &Country) -> String {
    c.passport
        .chars()
        .map(|ch| match ch {
            'L' => upper_letter(rng),
            _ => char::from(b'0' + rng.random_range(0..=9u8)),
        })
        .collect()
}

/// 운전면허 지역코드 (숫자형): 서울 11, 부산 12, 경기 13, 강원 14, 충북 15, 충남 16, 전북 17, 전남 18,
/// 경북 19, 경남 20, 제주 21, 대구 22, 인천 23, 광주 24, 대전 25, 울산 26, 경기북부 28
pub const LICENSE_REGION_CODES: [&str; 17] = [
    "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23", "24", "25", "26",
    "28",
];

/// 지역코드와 같은 순서의 지역명 (지역명 표기용, 28 경기북부는 "경기")
pub const LICENSE_REGION_NAMES: [&str; 17] = [
    "서울", "부산", "경기", "강원", "충북", "충남", "전북", "전남", "경북", "경남", "제주", "대구",
    "인천", "광주", "대전", "울산", "경기",
];

/// 운전면허번호 표기. `region_name`이면 지역코드 대신 지역명 (예: `경북 18-983209-40`)
pub fn license_text(number: &NumberText, region_name: bool, sep: &str) -> String {
    let mut groups = number.groups.clone();
    if region_name
        && let Some(first) = groups.first()
        && let Some(i) = LICENSE_REGION_CODES.iter().position(|c| c == first)
    {
        let name = LICENSE_REGION_NAMES.get(i).copied().unwrap_or("서울");
        groups.remove(0);
        return format!("{name} {}", groups.join(sep));
    }
    groups.join(sep)
}

/// 운전면허번호 `RR-YY-NNNNNN-CC`
pub fn driver_license<R: Rng + ?Sized>(rng: &mut R, issue_year: u16) -> NumberText {
    let region = pick(rng, &LICENSE_REGION_CODES).copied().unwrap_or("11");
    NumberText::new(vec![
        region.to_string(),
        format!("{:02}", issue_year % 100),
        digits(rng, 6, 0..=9),
        digits(rng, 2, 0..=9),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::data::FakeData;
    use crate::fake::seeded;
    use std::path::PathBuf;

    fn data() -> FakeData {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/synth");
        FakeData::load(&dir).unwrap()
    }

    #[test]
    fn accounts_follow_bank_formats() {
        let d = data();
        let mut r = seeded(51);
        for bank in &d.banks.bank {
            for _ in 0..100 {
                let a = account_in(&mut r, bank);
                let lens: Vec<usize> = a.number.groups.iter().map(String::len).collect();
                assert_eq!(lens, bank.groups, "{}", bank.name);
                if !bank.prefixes.is_empty() {
                    assert!(bank.prefixes.contains(&a.number.groups[0]));
                }
                assert!(a.number.digits().bytes().all(|b| b.is_ascii_digit()));
            }
        }
    }

    #[test]
    fn passports_match_patterns() {
        let d = data();
        let mut r = seeded(52);
        for _ in 0..200 {
            let old = korean_passport(&mut r, KoreanPassport::Old);
            assert_eq!(old.len(), 9);
            assert!(old.starts_with('M') && old[1..].bytes().all(|b| b.is_ascii_digit()));
            let new = korean_passport(&mut r, KoreanPassport::New);
            let b = new.as_bytes();
            assert_eq!(b.len(), 9);
            assert!(b[0] == b'M' && b[4].is_ascii_uppercase());
            assert!(b[1..4].iter().chain(&b[5..]).all(u8::is_ascii_digit));
        }
        for c in &d.foreign.country {
            let p = foreign_passport(&mut r, c);
            assert_eq!(p.len(), c.passport.len());
            for (ch, pat) in p.chars().zip(c.passport.chars()) {
                match pat {
                    'L' => assert!(ch.is_ascii_uppercase()),
                    _ => assert!(ch.is_ascii_digit()),
                }
            }
        }
    }

    #[test]
    fn license_region_name_style() {
        let n = NumberText::new(vec!["19".into(), "18".into(), "983209".into(), "40".into()]);
        assert_eq!(license_text(&n, true, "-"), "경북 18-983209-40");
        assert_eq!(license_text(&n, false, "-"), "19-18-983209-40");
        assert_eq!(LICENSE_REGION_NAMES.len(), LICENSE_REGION_CODES.len());
    }

    #[test]
    fn driver_licenses_have_region_and_year() {
        let mut r = seeded(53);
        for _ in 0..200 {
            let l = driver_license(&mut r, 2019);
            let lens: Vec<usize> = l.groups.iter().map(String::len).collect();
            assert_eq!(lens, [2, 2, 6, 2]);
            assert!(LICENSE_REGION_CODES.contains(&l.groups[0].as_str()));
            assert_eq!(l.groups[1], "19");
            assert_eq!(l.canonical().len(), 15);
        }
    }
}
