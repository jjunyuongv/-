//! 식별번호 생성기 (체크섬 포함) + 하드네거티브 번호.
//!
//! | 번호 | 형식 | 검증 |
//! |---|---|---|
//! | 주민등록번호 | `YYMMDD-GXXXXXC` (G: 1·2 1900년대, 3·4 2000년대) | 2020.10 이전형: 가중치 2~9,2~5 / `(11 - 합%11) % 10`. 이후형: 뒤 6자리 무작위(체크섬 없음) |
//! | 외국인등록번호 | `YYMMDD-GXXXXXC` (G: 5·6 1900년대, 7·8 2000년대) | 등록기관(8~9번째) 짝수, 12번째 6~9, `(13 - 합%11) % 10` |
//! | 사업자등록번호 | `XXX-XX-XXXXX` | 가운데 두 자리 = 유형 코드(개인 01~79·90~99, 법인 81·82·85~88), 가중치 1,3,7,1,3,7,1,3,5 |
//! | 법인등록번호 | `XXXXXX-XXXXXXX` | 앞 4자리 등기관서, 5~6번째 법인 종류, 가중치 1,2 반복 / `(10 - 합%10) % 10` |

use rand::{Rng, RngExt};

use super::date::{Ymd, birth_date_for_age, days_in_month};
use super::{FakeError, NumberText, digits, pick, pick_weighted, ratio_weights};
use crate::ratios::RatioConfig;

/// 주민등록번호 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RrnKind {
    /// 2020.10 이전형 (지역번호·체크섬 있음)
    Legacy,
    /// 2020.10 이후형 (성별 뒤 6자리 무작위, 체크섬 없음)
    Post2020,
}

/// 사업자 종류 (사업자등록번호 가운데 두 자리 유형 코드를 가른다)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BizKind {
    Individual,
    Corporation,
}

/// 개인사업자 유형 코드: 01~79 과세, 90~99 면세
pub fn individual_biz_codes() -> impl Iterator<Item = u8> {
    (1..=79).chain(90..=99)
}

/// 법인 유형 코드: 81·86·87·88 영리법인 본점, 85 영리법인 지점, 82 비영리법인
pub const CORPORATION_BIZ_CODES: [u8; 6] = [81, 82, 85, 86, 87, 88];

/// 법인등록번호 앞 4자리 등기관서 코드 (예시 목록, 균등 선택)
const CORP_REGISTRY_OFFICES: [&str; 12] = [
    "1101", "1201", "1301", "1311", "1345", "1358", "1401", "1501", "1601", "1701", "2001", "2401",
];
/// 법인등록번호 5~6번째 법인 종류 (11 주식회사, 12 합명, 13 합자, 14 유한, 21 사단, 22 재단)
const CORP_KINDS: [&str; 6] = ["11", "12", "13", "14", "21", "22"];

fn to_digits(s: &str) -> Vec<u8> {
    s.bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect()
}

fn digit_char(d: u8) -> char {
    char::from(b'0' + d % 10)
}

/// 주민·외국인등록번호 앞 12자리의 가중합
fn rrn_weighted_sum(d: &[u8]) -> u32 {
    const W: [u32; 12] = [2, 3, 4, 5, 6, 7, 8, 9, 2, 3, 4, 5];
    d.iter().zip(W).map(|(a, w)| u32::from(*a) * w).sum()
}

/// 주민등록번호 체크섬 (2020.10 이전형)
pub fn rrn_check_digit(first12: &[u8]) -> u8 {
    ((11 - rrn_weighted_sum(first12) % 11) % 10) as u8
}

/// 외국인등록번호 체크섬
pub fn frn_check_digit(first12: &[u8]) -> u8 {
    ((13 - rrn_weighted_sum(first12) % 11) % 10) as u8
}

/// 생년월일 연도와 내외국인으로 성별 자리 후보
fn gender_digits(year: u16, foreigner: bool) -> [u8; 2] {
    match (year >= 2000, foreigner) {
        (false, false) => [1, 2],
        (true, false) => [3, 4],
        (false, true) => [5, 6],
        (true, true) => [7, 8],
    }
}

/// 주민등록번호. `kind`가 `Legacy`면 체크섬이 맞고, `Post2020`이면 뒤 6자리가 무작위다.
pub fn rrn<R: Rng + ?Sized>(rng: &mut R, birth: Ymd, kind: RrnKind) -> NumberText {
    let g = *pick(rng, &gender_digits(birth.year, false)).unwrap_or(&1);
    let mut back = String::from(digit_char(g));
    match kind {
        RrnKind::Legacy => {
            back.push_str(&digits(rng, 5, 0..=9));
            let first12 = to_digits(&format!("{}{}", birth.yymmdd(), back));
            back.push(digit_char(rrn_check_digit(&first12)));
        }
        RrnKind::Post2020 => back.push_str(&digits(rng, 6, 0..=9)),
    }
    NumberText::new(vec![birth.yymmdd(), back])
}

/// 비율 설정(`rrn_kind`)에 따라 이전형/이후형을 고른다
pub fn rrn_kind<R: Rng + ?Sized>(rng: &mut R, config: &RatioConfig) -> Result<RrnKind, FakeError> {
    let w = ratio_weights(config, "rrn_kind", ["legacy", "post2020"])?;
    let i = pick_weighted(rng, &w);
    config.tally.record("rrn_kind", ["legacy", "post2020"][i]);
    Ok(if i == 0 {
        RrnKind::Legacy
    } else {
        RrnKind::Post2020
    })
}

/// 외국인등록번호 (등록기관 짝수, 12번째 6~9, 체크섬)
pub fn frn<R: Rng + ?Sized>(rng: &mut R, birth: Ymd) -> NumberText {
    let g = *pick(rng, &gender_digits(birth.year, true)).unwrap_or(&5);
    let office = rng.random_range(0..=49u8) * 2; // 짝수 00~98
    let mid = digits(rng, 2, 0..=9);
    let twelfth = rng.random_range(6..=9u8);
    let mut back = format!(
        "{}{:02}{}{}",
        digit_char(g),
        office,
        mid,
        digit_char(twelfth)
    );
    let first12 = to_digits(&format!("{}{}", birth.yymmdd(), back));
    back.push(digit_char(frn_check_digit(&first12)));
    NumberText::new(vec![birth.yymmdd(), back])
}

/// 주민번호 형식이지만 생년월일이 무효한 13자리 (하드네거티브). 성별 자리는 유효(1~4),
/// 체크섬은 절반 확률로 맞춘다 → 날짜 검사만으로 걸러지는지 측정
pub fn rrn_invalid_date<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    let yy = rng.random_range(0..=99u8);
    let (mm, dd) = match rng.random_range(0..3u8) {
        // 월이 13~99 또는 00
        0 => (
            if rng.random_bool(0.5) {
                rng.random_range(13..=99u8)
            } else {
                0
            },
            rng.random_range(1..=28u8),
        ),
        // 일이 그 달보다 큼 (2월 30·31일, 4월 31일, 1월 32일 등).
        // 2월은 윤년에 29일이 유효하므로 30일부터
        1 => {
            let m = rng.random_range(1..=12u8);
            let lo = if m == 2 {
                30
            } else {
                days_in_month(2001, m) + 1
            };
            (m, rng.random_range(lo..=lo.max(31)))
        }
        // 일이 32~99 또는 00
        _ => (
            rng.random_range(1..=12u8),
            if rng.random_bool(0.5) {
                rng.random_range(32..=99u8)
            } else {
                0
            },
        ),
    };
    let front = format!("{yy:02}{mm:02}{dd:02}");
    let g = rng.random_range(1..=4u8);
    let mut back = format!("{}{}", digit_char(g), digits(rng, 5, 0..=9));
    if rng.random_bool(0.5) {
        let first12 = to_digits(&format!("{front}{back}"));
        back.push(digit_char(rrn_check_digit(&first12)));
    } else {
        back.push_str(&digits(rng, 1, 0..=9));
    }
    NumberText::new(vec![front, back])
}

/// 사업자등록번호 체크 숫자 (앞 9자리)
pub fn bizno_check_digit(first9: &[u8]) -> u8 {
    const W: [u32; 9] = [1, 3, 7, 1, 3, 7, 1, 3, 5];
    let mut sum: u32 = first9.iter().zip(W).map(|(a, w)| u32::from(*a) * w).sum();
    sum += u32::from(first9.get(8).copied().unwrap_or(0)) * 5 / 10;
    ((10 - sum % 10) % 10) as u8
}

/// 사업자등록번호. 가운데 두 자리 유형 코드로 개인·법인을 구분한다
pub fn bizno<R: Rng + ?Sized>(rng: &mut R, kind: BizKind) -> NumberText {
    let office = rng.random_range(101..=899u16);
    let code = match kind {
        BizKind::Individual => {
            let codes: Vec<u8> = individual_biz_codes().collect();
            *pick(rng, &codes).unwrap_or(&1)
        }
        BizKind::Corporation => *pick(rng, &CORPORATION_BIZ_CODES).unwrap_or(&81),
    };
    let serial = digits(rng, 4, 0..=9);
    let first9 = to_digits(&format!("{office:03}{code:02}{serial}"));
    let check = bizno_check_digit(&first9);
    NumberText::new(vec![
        format!("{office:03}"),
        format!("{code:02}"),
        format!("{serial}{}", digit_char(check)),
    ])
}

/// 사업자등록번호 유형 코드로 사업자 종류 판별 (해당 없음이면 `None`)
pub fn biz_kind_of(number: &NumberText) -> Option<BizKind> {
    let d = to_digits(&number.digits());
    let code = d.get(3).copied().unwrap_or(0) * 10 + d.get(4).copied().unwrap_or(0);
    if individual_biz_codes().any(|c| c == code) {
        Some(BizKind::Individual)
    } else if CORPORATION_BIZ_CODES.contains(&code) {
        Some(BizKind::Corporation)
    } else {
        None
    }
}

/// 법인등록번호 체크 숫자 (앞 12자리, 가중치 1·2 반복)
pub fn corp_reg_check_digit(first12: &[u8]) -> u8 {
    let sum: u32 = first12
        .iter()
        .enumerate()
        .map(|(i, d)| u32::from(*d) * if i % 2 == 0 { 1 } else { 2 })
        .sum();
    ((10 - sum % 10) % 10) as u8
}

/// 법인등록번호 (13자리, 6-7 형식)
pub fn corp_reg_no<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    let office = pick(rng, &CORP_REGISTRY_OFFICES).copied().unwrap_or("1101");
    let kind = pick(rng, &CORP_KINDS).copied().unwrap_or("11");
    let serial = digits(rng, 6, 0..=9);
    let first12 = format!("{office}{kind}{serial}");
    let check = corp_reg_check_digit(&to_digits(&first12));
    NumberText::new(vec![
        first12[..6].to_string(),
        format!("{}{}", &first12[6..], digit_char(check)),
    ])
}

/// 13자리 문서·관리번호 (하드네거티브). 붙여 쓴 13자리 하나
pub fn doc_no13<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    NumberText::new(vec![digits(rng, 13, 1..=9)])
}

/// 사내 문서번호 (예: 인사-2026-0153)
pub fn doc_no<R: Rng + ?Sized>(rng: &mut R, year: u16) -> NumberText {
    const DEPTS: [&str; 5] = ["인사", "총무", "노무", "경영", "안전"];
    let dept = pick(rng, &DEPTS).copied().unwrap_or("인사");
    NumberText::new(vec![
        dept.to_string(),
        year.to_string(),
        digits(rng, 4, 0..=9),
    ])
}

/// 사업장관리번호 (11자리 = 사업자등록번호 10자리 + 단위사업장 1자리)
pub fn workplace_mgmt_no<R: Rng + ?Sized>(rng: &mut R, kind: BizKind) -> NumberText {
    let mut b = bizno(rng, kind);
    b.groups.push(rng.random_range(0..=9u8).to_string());
    b
}

/// 우편번호 (5자리, 01000~63999)
pub fn zipcode<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    NumberText::new(vec![format!("{:05}", rng.random_range(1000..=63999u32))])
}

/// 생년월일: 기준일에 만 나이 범위 (재수출, 주민번호와 같이 쓰기 편하게)
pub fn birth_date<R: Rng + ?Sized>(rng: &mut R, at: Ymd, min_age: u16, max_age: u16) -> Ymd {
    birth_date_for_age(rng, at, min_age, max_age)
}

// ───────── 검증 (테스트·리포트용. 9단계 규칙 팩 검증기는 detect-rules에 따로 구현) ─────────

/// 주민·외국인등록번호 앞 6자리 생년월일 + 성별 자리로 날짜가 유효한지
pub fn rrn_date_valid(number: &NumberText) -> bool {
    let d = to_digits(&number.digits());
    if d.len() != 13 {
        return false;
    }
    let century: u16 = match d[6] {
        1 | 2 | 5 | 6 => 1900,
        3 | 4 | 7 | 8 => 2000,
        9 | 0 => 1800,
        _ => return false,
    };
    let year = century + u16::from(d[0]) * 10 + u16::from(d[1]);
    Ymd {
        year,
        month: d[2] * 10 + d[3],
        day: d[4] * 10 + d[5],
    }
    .is_valid()
}

pub fn rrn_checksum_ok(number: &NumberText) -> bool {
    let d = to_digits(&number.digits());
    d.len() == 13 && rrn_check_digit(&d[..12]) == d[12]
}

pub fn frn_valid(number: &NumberText) -> bool {
    let d = to_digits(&number.digits());
    d.len() == 13
        && (5..=8).contains(&d[6])
        && (d[7] * 10 + d[8]) % 2 == 0
        && (6..=9).contains(&d[11])
        && frn_check_digit(&d[..12]) == d[12]
        && rrn_date_valid(number)
}

pub fn bizno_valid(number: &NumberText) -> bool {
    let d = to_digits(&number.digits());
    d.len() == 10 && bizno_check_digit(&d[..9]) == d[9]
}

pub fn corp_reg_no_valid(number: &NumberText) -> bool {
    let d = to_digits(&number.digits());
    d.len() == 13 && corp_reg_check_digit(&d[..12]) == d[12]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::seeded;

    const AT: Ymd = Ymd {
        year: 2026,
        month: 11,
        day: 8,
    };

    #[test]
    fn checksum_digits_match_hand_computed_examples() {
        // 900101-1 23456 ? : 9*2+0*3+0*4+1*5+0*6+1*7+1*8+2*9+3*2+4*3+5*4+6*5 = 124, (11-124%11)%10 = 8
        assert_eq!(rrn_check_digit(&to_digits("900101123456")), 8);
        // 같은 앞 12자리의 외국인등록번호식: (13-124%11)%10 = 0
        assert_eq!(frn_check_digit(&to_digits("900101123456")), 0);
        // 123-45-6789?: 1+6+21+4+15+42+7+24+45 = 165, + 9*5/10=4 → 169, (10-9)%10 = 1
        assert_eq!(bizno_check_digit(&to_digits("123456789")), 1);
        // 110111-123456?: 1+2+0+2+1+2+1+4+3+8+5+12 = 41, (10-1)%10 = 9
        assert_eq!(corp_reg_check_digit(&to_digits("110111123456")), 9);
    }

    #[test]
    fn legacy_rrns_pass_checksum_and_post2020_have_valid_dates() {
        let mut r = seeded(11);
        for _ in 0..500 {
            let birth = birth_date(&mut r, AT, 18, 65);
            let legacy = rrn(&mut r, birth, RrnKind::Legacy);
            assert!(rrn_checksum_ok(&legacy), "{}", legacy.canonical());
            assert!(rrn_date_valid(&legacy));
            assert_eq!(legacy.groups[0].len(), 6);
            assert_eq!(legacy.groups[1].len(), 7);

            let post = rrn(&mut r, birth, RrnKind::Post2020);
            assert!(rrn_date_valid(&post));
            let g = post.groups[1].chars().next().unwrap();
            assert!(matches!(g, '1'..='4'));
        }
    }

    #[test]
    fn gender_digit_follows_century() {
        let mut r = seeded(3);
        let old = rrn(
            &mut r,
            Ymd {
                year: 1985,
                month: 3,
                day: 1,
            },
            RrnKind::Legacy,
        );
        let young = rrn(
            &mut r,
            Ymd {
                year: 2005,
                month: 3,
                day: 1,
            },
            RrnKind::Legacy,
        );
        assert!(matches!(old.groups[1].as_bytes()[0], b'1' | b'2'));
        assert!(matches!(young.groups[1].as_bytes()[0], b'3' | b'4'));
        let f = frn(
            &mut r,
            Ymd {
                year: 2003,
                month: 7,
                day: 9,
            },
        );
        assert!(matches!(f.groups[1].as_bytes()[0], b'7' | b'8'));
    }

    #[test]
    fn frns_are_valid() {
        let mut r = seeded(12);
        for _ in 0..500 {
            let birth = birth_date(&mut r, AT, 20, 50);
            let f = frn(&mut r, birth);
            assert!(frn_valid(&f), "{}", f.canonical());
            // 외국인등록번호는 내국인 주민번호 체크섬과 다르다 (같은 숫자열이 둘 다 맞는 경우는 드묾)
        }
    }

    #[test]
    fn invalid_date_numbers_fail_date_check() {
        let mut r = seeded(13);
        let mut with_checksum = 0;
        for _ in 0..1000 {
            let n = rrn_invalid_date(&mut r);
            assert_eq!(n.digits().len(), 13);
            assert!(!rrn_date_valid(&n), "{}", n.canonical());
            with_checksum += usize::from(rrn_checksum_ok(&n));
        }
        // 절반 정도는 체크섬까지 맞춤 → 날짜 검사가 꼭 필요함을 측정
        assert!((400..=600).contains(&with_checksum), "{with_checksum}");
    }

    #[test]
    fn bizno_type_code_separates_individual_and_corporation() {
        let mut r = seeded(14);
        for _ in 0..500 {
            let i = bizno(&mut r, BizKind::Individual);
            let c = bizno(&mut r, BizKind::Corporation);
            assert!(bizno_valid(&i) && bizno_valid(&c));
            assert_eq!(
                biz_kind_of(&i),
                Some(BizKind::Individual),
                "{}",
                i.canonical()
            );
            assert_eq!(
                biz_kind_of(&c),
                Some(BizKind::Corporation),
                "{}",
                c.canonical()
            );
            assert_eq!(i.canonical().len(), 12); // XXX-XX-XXXXX
        }
    }

    #[test]
    fn corp_reg_numbers_and_other_hard_negatives() {
        let mut r = seeded(15);
        for _ in 0..500 {
            let c = corp_reg_no(&mut r);
            assert!(corp_reg_no_valid(&c), "{}", c.canonical());
            assert_eq!(c.canonical().len(), 14);
            assert_eq!(doc_no13(&mut r).digits().len(), 13);
            let w = workplace_mgmt_no(&mut r, BizKind::Corporation);
            assert_eq!(w.digits().len(), 11);
            let z = zipcode(&mut r).digits();
            assert_eq!(z.len(), 5);
            assert!(z.as_str() >= "01000" && z.as_str() <= "63999");
        }
        let d = doc_no(&mut r, 2026);
        assert_eq!(d.groups.len(), 3);
        assert_eq!(d.groups[1], "2026");
    }

    #[test]
    fn rrn_kind_ratio_comes_from_config() {
        let config = RatioConfig::parse(
            r#"
[ratio.rrn_kind]
note = "t"
arbitrary = false
used_by = ["generator:rrn"]
weights = { legacy = 0.7, post2020 = 0.3 }
"#,
            "t",
        )
        .unwrap();
        let mut r = seeded(16);
        let n = 10_000;
        let post = (0..n)
            .filter(|_| rrn_kind(&mut r, &config).unwrap() == RrnKind::Post2020)
            .count();
        let p = post as f64 / n as f64;
        assert!((p - 0.3).abs() < 0.02, "{p}");

        let empty = RatioConfig::parse("ratio = {}", "e").unwrap();
        assert!(rrn_kind(&mut r, &empty).is_err());
    }
}
