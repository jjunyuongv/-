//! 전화번호 생성기 (PLAN.md 3단계, 1-2 결정)
//!
//! | 종류 | 형식 (묶음 자릿수) | 쓰는 곳 |
//! |---|---|---|
//! | 휴대전화 [`mobile`] | 3/4/4 (`010`) | 근로자·친권자 연락처, 개인사업자 사업주 → `PHONE` |
//! | 유선 [`landline`] | 2~3/3~4/4 (`02`, `031`~`064`) | 회사 대표번호(하드네거티브), 팩스 |
//! | 인터넷 전화 [`internet`] | 3/4/4 (`070`) | 회사 직통번호(하드네거티브) |
//! | 전국 대표번호 [`national_rep`] | 4/4 (`15xx`·`16xx`·`18xx`) | 회사 대표번호(하드네거티브) |
//! | 4자리 특수번호 [`special_number`] | 4 (`13xx` 공공 상담번호) | 하드네거티브 |
//! | 회사 대표번호 [`PhoneGen::company_at`] | 유선(회사 주소 시·도 지역번호 [`area_code_for`]) + 전국 대표번호 (`company_phone_kind`) | 하드네거티브 |
//! | 공공기관 대표번호 [`PhoneGen::public_rep`] | 전국 대표번호 + 정부청사 지역번호 유선 (`public_rep_kind`) | 하드네거티브 |
//!
//! 형식은 실제와 같고 값은 무작위다. 실제 번호와 우연히 같을 수는 있지만, 사람·문맥과 연결되지 않는 합성 값이다.

use rand::{Rng, RngExt};

use super::{FakeError, NumberText, digits, pick, pick_weighted, ratio_weights};
use crate::ratios::{RatioConfig, Tally};

/// 지역번호 (서울 02 + 시도 3자리)
pub const AREA_CODES: [&str; 17] = [
    "02", "031", "032", "033", "041", "042", "043", "044", "051", "052", "053", "054", "055",
    "061", "062", "063", "064",
];

/// 정부청사가 있는 지역번호 (세종 044, 서울 02, 대전 042) — 공공기관 대표번호 유선용
pub const GOV_AREA_CODES: [&str; 3] = ["044", "02", "042"];

/// 전국 대표번호 앞 4자리
pub const NATIONAL_REP_PREFIXES: [&str; 17] = [
    "1544", "1566", "1577", "1588", "1599", "1600", "1644", "1661", "1666", "1670", "1688", "1800",
    "1811", "1833", "1855", "1877", "1899",
];

/// 4자리 공공 특수번호 (고용노동부 1350, 국민연금 1355, 외국인종합안내 1345, 관광 1330, 청소년 1388, 여성긴급 1366)
pub const SPECIAL_NUMBERS: [&str; 6] = ["1350", "1355", "1345", "1330", "1388", "1366"];

/// 휴대전화 010-XXXX-XXXX
pub fn mobile<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    NumberText::new(vec![
        "010".into(),
        digits(rng, 4, 2..=9),
        digits(rng, 4, 0..=9),
    ])
}

fn landline_in<R: Rng + ?Sized>(rng: &mut R, area_codes: &[&str]) -> NumberText {
    let area = pick(rng, area_codes).copied().unwrap_or("02");
    let mid_len = if rng.random_bool(0.5) { 3 } else { 4 };
    NumberText::new(vec![
        area.into(),
        digits(rng, mid_len, 2..=9),
        digits(rng, 4, 0..=9),
    ])
}

/// 유선 2~3/3~4/4
pub fn landline<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    landline_in(rng, &AREA_CODES)
}

/// 시·도(공식 이름)의 지역번호. 전남광주통합특별시는 옛 광주광역시 자치구면 062, 그 밖(옛 전라남도)은 061
pub fn area_code_for(sido: &str, sigungu: &str) -> Option<&'static str> {
    const GWANGJU_GU: [&str; 5] = ["동구", "서구", "남구", "북구", "광산구"];
    Some(match sido {
        "서울특별시" => "02",
        "부산광역시" => "051",
        "대구광역시" => "053",
        "인천광역시" => "032",
        "대전광역시" => "042",
        "울산광역시" => "052",
        "세종특별자치시" => "044",
        "경기도" => "031",
        "강원특별자치도" => "033",
        "충청북도" => "043",
        "충청남도" => "041",
        "전북특별자치도" => "063",
        "경상북도" => "054",
        "경상남도" => "055",
        "제주특별자치도" => "064",
        "전남광주통합특별시" if GWANGJU_GU.contains(&sigungu) => "062",
        "전남광주통합특별시" => "061",
        _ => return None,
    })
}

/// 그 지역의 유선 번호 (모르는 시·도면 전국 지역번호 중 무작위)
pub fn landline_for<R: Rng + ?Sized>(rng: &mut R, sido: &str, sigungu: &str) -> NumberText {
    match area_code_for(sido, sigungu) {
        Some(code) => landline_in(rng, &[code]),
        None => landline(rng),
    }
}

/// 인터넷 전화 070-XXXX-XXXX
pub fn internet<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    NumberText::new(vec![
        "070".into(),
        digits(rng, 4, 2..=9),
        digits(rng, 4, 0..=9),
    ])
}

/// 전국 대표번호 15xx·16xx·18xx-XXXX
pub fn national_rep<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    let prefix = pick(rng, &NATIONAL_REP_PREFIXES).copied().unwrap_or("1588");
    NumberText::new(vec![prefix.into(), digits(rng, 4, 0..=9)])
}

/// 4자리 공공 특수번호
pub fn special_number<R: Rng + ?Sized>(rng: &mut R) -> NumberText {
    NumberText::new(vec![
        pick(rng, &SPECIAL_NUMBERS)
            .copied()
            .unwrap_or("1350")
            .into(),
    ])
}

/// 비율 설정이 필요한 전화번호 생성기 (회사·공공기관 대표번호)
#[derive(Debug, Clone)]
pub struct PhoneGen {
    /// company_phone_kind: [유선, 전국 대표번호]
    company: [f64; 2],
    /// public_rep_kind: [전국 대표번호, 정부청사 유선]
    public_rep: [f64; 2],
    tally: Tally,
}

impl PhoneGen {
    pub fn new(config: &RatioConfig) -> Result<Self, FakeError> {
        Ok(PhoneGen {
            company: ratio_weights(config, "company_phone_kind", ["landline", "national"])?,
            public_rep: ratio_weights(config, "public_rep_kind", ["national", "landline"])?,
            tally: config.tally.clone(),
        })
    }

    fn pick_company<R: Rng + ?Sized>(&self, rng: &mut R) -> usize {
        let i = pick_weighted(rng, &self.company);
        self.tally
            .record("company_phone_kind", ["landline", "national"][i]);
        i
    }

    /// 회사 대표번호 = 유선 + 전국 대표번호 (지역번호는 무작위)
    pub fn company<R: Rng + ?Sized>(&self, rng: &mut R) -> NumberText {
        if self.pick_company(rng) == 0 {
            landline(rng)
        } else {
            national_rep(rng)
        }
    }

    /// 회사 대표번호 — 유선이면 회사 주소 시·도의 지역번호
    pub fn company_at<R: Rng + ?Sized>(
        &self,
        rng: &mut R,
        sido: &str,
        sigungu: &str,
    ) -> NumberText {
        if self.pick_company(rng) == 0 {
            landline_for(rng, sido, sigungu)
        } else {
            national_rep(rng)
        }
    }

    /// 공공기관 대표번호 = 전국 대표번호 + 정부청사 지역번호 유선
    pub fn public_rep<R: Rng + ?Sized>(&self, rng: &mut R) -> NumberText {
        let i = pick_weighted(rng, &self.public_rep);
        self.tally
            .record("public_rep_kind", ["national", "landline"][i]);
        if i == 0 {
            national_rep(rng)
        } else {
            landline_in(rng, &GOV_AREA_CODES)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::seeded;

    fn lens(n: &NumberText) -> Vec<usize> {
        n.groups.iter().map(String::len).collect()
    }

    #[test]
    fn formats_per_kind() {
        let mut r = seeded(21);
        for _ in 0..500 {
            let m = mobile(&mut r);
            assert_eq!(lens(&m), [3, 4, 4]);
            assert_eq!(m.groups[0], "010");
            assert!(!m.groups[1].starts_with(['0', '1']));

            let l = landline(&mut r);
            let l = lens(&l);
            assert!(
                (2..=3).contains(&l[0]) && (3..=4).contains(&l[1]) && l[2] == 4,
                "{l:?}"
            );

            let i = internet(&mut r);
            assert_eq!(lens(&i), [3, 4, 4]);
            assert_eq!(i.groups[0], "070");

            let n = national_rep(&mut r);
            assert_eq!(lens(&n), [4, 4]);
            assert!(matches!(&n.groups[0][..2], "15" | "16" | "18"));

            let s = special_number(&mut r);
            assert_eq!(lens(&s), [4]);
            assert!(s.groups[0].starts_with("13"));
        }
    }

    #[test]
    fn area_codes_follow_sido() {
        let mut r = seeded(23);
        for (sido, sigungu, code) in [
            ("서울특별시", "강남구", "02"),
            ("경기도", "성남시 분당구", "031"),
            ("세종특별자치시", "", "044"),
            ("전남광주통합특별시", "서구", "062"),
            ("전남광주통합특별시", "목포시", "061"),
            ("제주특별자치도", "제주시", "064"),
        ] {
            assert_eq!(area_code_for(sido, sigungu), Some(code));
            assert_eq!(landline_for(&mut r, sido, sigungu).groups[0], code);
        }
        // 주소 자료의 모든 시·도에 지역번호가 있다
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/synth");
        let data = crate::fake::data::FakeData::load(&dir).unwrap();
        for d in &data.addresses.district {
            let code = area_code_for(&d.sido, &d.sigungu).unwrap();
            assert!(AREA_CODES.contains(&code), "{} {}", d.sido, d.sigungu);
        }
    }

    #[test]
    fn company_and_public_mixes_follow_ratios() {
        let config = RatioConfig::parse(
            r#"
[ratio.company_phone_kind]
note = "t"
arbitrary = true
used_by = ["generator:company_phone"]
weights = { landline = 0.6, national = 0.4 }

[ratio.public_rep_kind]
note = "t"
arbitrary = true
used_by = ["generator:public_rep_number"]
weights = { national = 0.6, landline = 0.4 }
"#,
            "t",
        )
        .unwrap();
        let g = PhoneGen::new(&config).unwrap();
        let mut r = seeded(22);
        let n = 10_000;
        let mut landline_count = 0;
        for _ in 0..n {
            let c = g.company(&mut r);
            assert!(c.groups.len() == 3 || matches!(&c.groups[0][..2], "15" | "16" | "18"));
            landline_count += usize::from(c.groups.len() == 3);
        }
        let p = landline_count as f64 / n as f64;
        assert!((p - 0.6).abs() < 0.02, "{p}");

        for _ in 0..500 {
            let p = g.public_rep(&mut r);
            if p.groups.len() == 3 {
                assert!(GOV_AREA_CODES.contains(&p.groups[0].as_str()));
            } else {
                assert_eq!(p.groups[0].len(), 4);
            }
        }
    }

    #[test]
    fn missing_ratio_is_an_error() {
        let empty = RatioConfig::parse("ratio = {}", "e").unwrap();
        assert!(PhoneGen::new(&empty).is_err());
    }
}
