//! 사람: 한국인 이름(성씨 빈도·세대별 음절), 외국인 이름(국가별 순서·영문 대문자), 국적, 이메일

use rand::{Rng, RngExt};

use super::data::{Country, ForeignData, Gender, Generation, KoreanNames, NamePart};
use super::{digits, pick, pick_weighted};

/// 한국인 이름
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KoreanName {
    pub surname: String,
    pub given: String,
}

impl KoreanName {
    pub fn full(&self) -> String {
        format!("{}{}", self.surname, self.given)
    }
}

/// 성씨 빈도와 세대·성별 음절표로 이름을 만든다 (첫·둘째 음절이 같으면 다시 뽑음)
pub fn korean_name<R: Rng + ?Sized>(
    rng: &mut R,
    names: &KoreanNames,
    generation: Generation,
    gender: Gender,
) -> KoreanName {
    let weights: Vec<f64> = names.surnames.iter().map(|s| s.pct).collect();
    let surname = names
        .surnames
        .get(pick_weighted(rng, &weights))
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let table = names
        .given
        .iter()
        .find(|g| g.generation == generation && g.gender == gender);
    let given = match table {
        Some(t) => loop {
            let a = pick(rng, &t.first).cloned().unwrap_or_default();
            let b = pick(rng, &t.second).cloned().unwrap_or_default();
            if a != b {
                break format!("{a}{b}");
            }
        },
        None => String::new(),
    };
    KoreanName { surname, given }
}

/// 외국인 이름 (그 나라 이름 순서, 영문 대문자)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignName {
    pub country: String,
    pub nationality: String,
    /// 이름 순서대로의 조각
    pub parts: Vec<(NamePart, String)>,
}

impl ForeignName {
    /// 기본 표기: 그 나라 순서, 영문 대문자, 공백으로 이음
    pub fn full(&self) -> String {
        self.parts
            .iter()
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// 성이 있으면 성 (없는 나라는 `None`, 예: 미얀마)
    pub fn family(&self) -> Option<&str> {
        self.parts
            .iter()
            .find(|(p, _)| *p == NamePart::Family)
            .map(|(_, s)| s.as_str())
    }
}

/// 나라 하나에서 이름을 만든다
pub fn foreign_name_in<R: Rng + ?Sized>(rng: &mut R, c: &Country, gender: Gender) -> ForeignName {
    let female = gender == Gender::Female;
    let mut parts = Vec::new();
    for part in &c.order {
        let list = match (part, female) {
            (NamePart::Family, true) if !c.family_female.is_empty() => &c.family_female,
            (NamePart::Family, _) => &c.family,
            (NamePart::Middle, false) => &c.middle_male,
            (NamePart::Middle, true) => &c.middle_female,
            (NamePart::Given, false) => &c.given_male,
            (NamePart::Given, true) => &c.given_female,
        };
        if let Some(s) = pick(rng, list) {
            parts.push((*part, s.clone()));
        }
    }
    ForeignName {
        country: c.code.clone(),
        nationality: c.nationality.clone(),
        parts,
    }
}

/// 나라를 균등하게 골라 이름을 만든다
pub fn foreign_name<R: Rng + ?Sized>(
    rng: &mut R,
    data: &ForeignData,
    gender: Gender,
) -> Option<ForeignName> {
    let c = pick(rng, &data.country)?;
    Some(foreign_name_in(rng, c, gender))
}

/// 이메일 주소 (로컬 부분은 지어낸 단어 + 숫자, 도메인은 흔한 메일 서비스)
pub fn email<R: Rng + ?Sized>(rng: &mut R) -> String {
    const WORDS: [&str; 12] = [
        "haneul", "garam", "sunny", "byeol", "dain", "nuri", "sori", "maru", "solbit", "hana",
        "areum", "jiwoo",
    ];
    const DOMAINS: [&str; 6] = [
        "naver.com",
        "gmail.com",
        "daum.net",
        "kakao.com",
        "hanmail.net",
        "nate.com",
    ];
    let word = pick(rng, &WORDS).copied().unwrap_or("hana");
    let n = rng.random_range(2..=4usize);
    let sep = if rng.random_bool(0.3) { "." } else { "" };
    let domain = pick(rng, &DOMAINS).copied().unwrap_or("naver.com");
    format!("{word}{sep}{}@{domain}", digits(rng, n, 0..=9))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::data::FakeData;
    use crate::fake::seeded;
    use std::path::PathBuf;

    pub(crate) fn data() -> FakeData {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/synth");
        FakeData::load(&dir).unwrap()
    }

    #[test]
    fn korean_names_follow_generation_tables() {
        let d = data();
        let mut r = seeded(31);
        for (generation, gender) in [
            (Generation::G7080, Gender::Male),
            (Generation::G7080, Gender::Female),
            (Generation::G2000, Gender::Male),
            (Generation::G2000, Gender::Female),
        ] {
            let table = d
                .names
                .given
                .iter()
                .find(|g| g.generation == generation && g.gender == gender)
                .unwrap();
            for _ in 0..300 {
                let n = korean_name(&mut r, &d.names, generation, gender);
                let syl: Vec<String> = n.given.chars().map(String::from).collect();
                assert_eq!(syl.len(), 2, "{n:?}");
                assert!(table.first.contains(&syl[0]) && table.second.contains(&syl[1]));
                assert_ne!(syl[0], syl[1]);
                assert!(d.names.surnames.iter().any(|s| s.name == n.surname));
            }
        }
    }

    #[test]
    fn surname_frequency_is_weighted() {
        let d = data();
        let mut r = seeded(32);
        let n = 20_000;
        let kim = (0..n)
            .filter(|_| {
                korean_name(&mut r, &d.names, Generation::G2000, Gender::Male).surname == "김"
            })
            .count();
        let total: f64 = d.names.surnames.iter().map(|s| s.pct).sum();
        let expected = 21.5 / total;
        let p = kim as f64 / n as f64;
        assert!((p - expected).abs() < 0.015, "{p} vs {expected}");
    }

    #[test]
    fn generation_follows_birth_year_for_layout_d() {
        // 레이아웃 D: 친권자(1970~80년대생) / 연소근로자(2009~2011년생)
        assert_eq!(Generation::for_birth_year(1978), Generation::G7080);
        assert_eq!(Generation::for_birth_year(1986), Generation::G7080);
        assert_eq!(Generation::for_birth_year(2010), Generation::G2000);
    }

    #[test]
    fn foreign_names_follow_country_order_and_uppercase() {
        let d = data();
        let mut r = seeded(33);
        for c in &d.foreign.country {
            for gender in [Gender::Male, Gender::Female] {
                let n = foreign_name_in(&mut r, c, gender);
                let order: Vec<NamePart> = n.parts.iter().map(|(p, _)| *p).collect();
                assert_eq!(order, c.order, "{}", c.code);
                let full = n.full();
                assert_eq!(full, full.to_uppercase(), "{full}");
                if c.code == "VN" {
                    let middle = &n.parts[1].1;
                    let list = if gender == Gender::Male {
                        &c.middle_male
                    } else {
                        &c.middle_female
                    };
                    assert!(list.contains(middle));
                    assert_eq!(n.parts[0].0, NamePart::Family, "베트남은 성이 앞");
                }
                if c.code == "UZ" && gender == Gender::Female {
                    assert!(n.family().unwrap().ends_with("OVA"));
                }
                if c.code == "MM" {
                    assert!(n.family().is_none(), "미얀마는 성 없음");
                }
            }
        }
        let ph = d.foreign.country.iter().find(|c| c.code == "PH").unwrap();
        let n = foreign_name_in(&mut r, ph, Gender::Female);
        assert_eq!(
            n.parts.last().unwrap().0,
            NamePart::Family,
            "필리핀은 성이 뒤"
        );
    }

    #[test]
    fn emails_have_local_and_domain() {
        let mut r = seeded(34);
        for _ in 0..200 {
            let e = email(&mut r);
            let (local, domain) = e.split_once('@').unwrap();
            assert!(!local.is_empty() && domain.contains('.'));
            assert!(e.is_ascii());
        }
    }
}
