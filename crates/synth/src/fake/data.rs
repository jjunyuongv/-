//! 가짜 PII 자료 표 (`assets/synth/*.toml`) 스키마·로딩·검증.
//! 출처·근거는 각 TOML 파일 머리에 적는다.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("자료 파일을 읽을 수 없습니다: {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("자료 파일 TOML 형식 오류: {path}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("자료 파일 검증 실패: {path}\n  - {}", problems.join("\n  - "))]
    Invalid { path: String, problems: Vec<String> },
}

/// 이름 세대 (1-2 결정 이후 3단계: 1970~80년대생 / 2000년대생)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Generation {
    G7080,
    G2000,
}

impl Generation {
    /// 생년으로 세대를 정한다 (1995년 미만 → 1970~80년대식, 이후 → 2000년대식)
    pub fn for_birth_year(year: u16) -> Self {
        if year < 1995 {
            Generation::G7080
        } else {
            Generation::G2000
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gender {
    Male,
    Female,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surname {
    pub name: String,
    pub pct: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GivenSyllables {
    pub generation: Generation,
    pub gender: Gender,
    pub first: Vec<String>,
    pub second: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KoreanNames {
    pub surnames: Vec<Surname>,
    pub given: Vec<GivenSyllables>,
}

/// 도로 하나 + 그 도로에 실제 건물이 있는 법정동·읍·면 (공식 자료에서 짝으로 뽑음)
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Road {
    pub name: String,
    /// 지상 건물본번 최댓값 (건물번호는 1..=max_no)
    pub max_no: u32,
    pub areas: Vec<String>,
}

impl Road {
    /// 읍·면 지역에 걸친 도로인가 (농업·건설 업종 근무장소용)
    pub fn is_rural(&self) -> bool {
        self.areas.iter().any(|a| is_eup_myeon(a))
    }
}

/// 읍·면 이름인가 (주소에서 도로명 앞에 쓴다)
pub fn is_eup_myeon(area: &str) -> bool {
    area.ends_with('읍') || area.ends_with('면')
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct District {
    pub sido: String,
    /// 약칭 (서울·경기 등). 확정된 약칭이 없는 시·도(전남광주통합특별시)는 비워 두고 정식 명칭만 쓴다
    #[serde(default)]
    pub sido_short: Option<String>,
    /// 세종특별자치시처럼 시·군·구가 없으면 빈 문자열
    pub sigungu: String,
    pub roads: Vec<Road>,
}

impl District {
    /// 약식 표기에 쓸 시·도 이름 (약칭이 없으면 정식 명칭)
    pub fn sido_short_or_full(&self) -> &str {
        self.sido_short.as_deref().unwrap_or(&self.sido)
    }

    /// 특별·광역시의 자치구 (도심) — "강남구"처럼 한 단어로 끝나는 구
    pub fn is_city_center(&self) -> bool {
        self.sigungu.ends_with('구') && !self.sigungu.contains(' ')
    }

    pub fn road(&self, name: &str) -> Option<&Road> {
        self.roads.iter().find(|r| r.name == name)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApartmentParts {
    pub prefix: Vec<String>,
    pub suffix: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KoreanAddresses {
    pub district: Vec<District>,
    pub apartment: ApartmentParts,
}

/// 이름 조각 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamePart {
    Family,
    Middle,
    Given,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Country {
    pub code: String,
    pub nationality: String,
    /// 그 나라의 이름 순서
    pub order: Vec<NamePart>,
    #[serde(default)]
    pub family: Vec<String>,
    /// 여성 성이 따로 있는 나라 (우즈베키스탄 -OVA 등). 없으면 `family`
    #[serde(default)]
    pub family_female: Vec<String>,
    #[serde(default)]
    pub middle_male: Vec<String>,
    #[serde(default)]
    pub middle_female: Vec<String>,
    pub given_male: Vec<String>,
    pub given_female: Vec<String>,
    /// L = 영문 대문자, # = 숫자
    pub passport: String,
    pub address: String,
    pub provinces: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForeignData {
    pub country: Vec<Country>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bank {
    pub name: String,
    pub groups: Vec<usize>,
    #[serde(default)]
    pub prefixes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Banks {
    pub bank: Vec<Bank>,
}

/// 자료 표 전체
#[derive(Debug, Clone)]
pub struct FakeData {
    pub names: KoreanNames,
    pub addresses: KoreanAddresses,
    pub foreign: ForeignData,
    pub banks: Banks,
}

fn load_toml<T: DeserializeOwned>(path: &Path) -> Result<T, DataError> {
    let origin = path.display().to_string();
    let src = fs::read_to_string(path).map_err(|source| DataError::Io {
        path: origin.clone(),
        source,
    })?;
    toml::from_str(&src).map_err(|source| DataError::Parse {
        path: origin,
        source,
    })
}

fn check(path: &Path, problems: Vec<String>) -> Result<(), DataError> {
    if problems.is_empty() {
        Ok(())
    } else {
        Err(DataError::Invalid {
            path: path.display().to_string(),
            problems,
        })
    }
}

impl FakeData {
    /// `assets/synth` 폴더에서 읽고 검증한다
    pub fn load(dir: &Path) -> Result<Self, DataError> {
        let names_path = dir.join("names_ko.toml");
        let names: KoreanNames = load_toml(&names_path)?;
        check(&names_path, names.validate())?;
        let addr_path = dir.join("addresses_ko.toml");
        let addresses: KoreanAddresses = load_toml(&addr_path)?;
        check(&addr_path, addresses.validate())?;
        let foreign_path = dir.join("foreign.toml");
        let foreign: ForeignData = load_toml(&foreign_path)?;
        check(&foreign_path, foreign.validate())?;
        let banks_path = dir.join("banks.toml");
        let banks: Banks = load_toml(&banks_path)?;
        check(&banks_path, banks.validate())?;
        Ok(FakeData {
            names,
            addresses,
            foreign,
            banks,
        })
    }
}

fn is_hangul_syllable(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

impl KoreanNames {
    pub fn validate(&self) -> Vec<String> {
        let mut p = Vec::new();
        let mut seen = BTreeSet::new();
        for s in &self.surnames {
            if !(1..=2).contains(&s.name.chars().count()) || !s.name.chars().all(is_hangul_syllable)
            {
                p.push(format!("성 `{}`: 한글 1~2음절", s.name));
            }
            if s.pct.is_nan() || s.pct <= 0.0 {
                p.push(format!("성 `{}`: 비율은 0보다 커야 함", s.name));
            }
            if !seen.insert(&s.name) {
                p.push(format!("성 `{}` 중복", s.name));
            }
        }
        for generation in [Generation::G7080, Generation::G2000] {
            for gender in [Gender::Male, Gender::Female] {
                let n = self
                    .given
                    .iter()
                    .filter(|g| g.generation == generation && g.gender == gender)
                    .count();
                if n != 1 {
                    p.push(format!(
                        "이름 음절 {generation:?}/{gender:?}: 표가 {n}개 (1개여야 함)"
                    ));
                }
            }
        }
        for g in &self.given {
            for syl in g.first.iter().chain(&g.second) {
                if syl.chars().count() != 1 || !syl.chars().all(is_hangul_syllable) {
                    p.push(format!("이름 음절 `{syl}`: 한글 1음절"));
                }
            }
            if g.first.len() < 5 || g.second.len() < 5 {
                p.push(format!(
                    "이름 음절 {:?}/{:?}: 각 5개 이상",
                    g.generation, g.gender
                ));
            }
        }
        p
    }
}

impl KoreanAddresses {
    pub fn validate(&self) -> Vec<String> {
        // 공식 도로명주소 자료(2026-11-08) 기준 시·도: 광주광역시·전라남도 → 전남광주통합특별시
        const SIDO: [&str; 16] = [
            "서울특별시",
            "부산광역시",
            "대구광역시",
            "인천광역시",
            "대전광역시",
            "울산광역시",
            "세종특별자치시",
            "경기도",
            "강원특별자치도",
            "충청북도",
            "충청남도",
            "전북특별자치도",
            "경상북도",
            "경상남도",
            "제주특별자치도",
            "전남광주통합특별시",
        ];
        let mut p = Vec::new();
        let mut keys = BTreeSet::new();
        for d in &self.district {
            let key = format!("{} {}", d.sido, d.sigungu);
            if !SIDO.contains(&d.sido.as_str()) {
                p.push(format!("`{key}`: 모르는 시·도"));
            }
            if let Some(short) = &d.sido_short
                && !d
                    .sido
                    .starts_with(&short[..short.chars().next().map_or(0, char::len_utf8)])
            {
                p.push(format!("`{key}`: 약칭 `{short}`가 시·도와 맞지 않음"));
            }
            if d.sigungu.is_empty() != (d.sido == "세종특별자치시") {
                p.push(format!("`{key}`: 시·군·구는 세종특별자치시만 비울 수 있음"));
            }
            if !keys.insert(key.clone()) {
                p.push(format!("`{key}` 중복"));
            }
            if d.roads.is_empty() {
                p.push(format!("`{key}`: 도로가 비어 있음"));
            }
            let mut roads = BTreeSet::new();
            for r in &d.roads {
                let name = &r.name;
                if !(name.ends_with('로') || name.ends_with('길')) {
                    p.push(format!("`{key}` 도로 `{name}`: 도로명은 ~로/~길"));
                }
                if !roads.insert(name) {
                    p.push(format!("`{key}` 도로 `{name}` 중복"));
                }
                if r.max_no == 0 || r.areas.is_empty() {
                    p.push(format!("`{key}` 도로 `{name}`: 건물번호·동이 비어 있음"));
                }
                for area in &r.areas {
                    // 법정동은 ~동·~가가 대부분이고, 대구 중구 '시장북로'처럼 ~로로 끝나는 법정동도 있다
                    if !["동", "가", "읍", "면", "로"]
                        .iter()
                        .any(|e| area.ends_with(e))
                    {
                        p.push(format!(
                            "`{key}` 도로 `{name}` 동 `{area}`: ~동/가/읍/면/로"
                        ));
                    }
                }
            }
        }
        if self.apartment.prefix.is_empty() || self.apartment.suffix.is_empty() {
            p.push("아파트 이름 조각이 비어 있음".to_string());
        }
        p
    }

    /// 시·도, 시·군·구로 구역을 찾는다
    pub fn find(&self, sido: &str, sigungu: &str) -> Option<&District> {
        self.district.iter().find(|d| {
            (d.sido == sido || d.sido_short.as_deref() == Some(sido)) && d.sigungu == sigungu
        })
    }
}

impl ForeignData {
    pub fn validate(&self) -> Vec<String> {
        let mut p = Vec::new();
        let mut codes = BTreeSet::new();
        for c in &self.country {
            let id = &c.code;
            if !codes.insert(id) {
                p.push(format!("{id}: 중복"));
            }
            if c.order.is_empty() || !c.order.contains(&NamePart::Given) {
                p.push(format!("{id}: 이름 순서에 given이 있어야 함"));
            }
            let needs = |part: NamePart| c.order.contains(&part);
            if needs(NamePart::Family) && c.family.is_empty() {
                p.push(format!("{id}: family 목록이 비어 있음"));
            }
            if needs(NamePart::Middle) && (c.middle_male.is_empty() || c.middle_female.is_empty()) {
                p.push(format!("{id}: middle 목록이 비어 있음"));
            }
            if !c.family_female.is_empty() && c.family_female.len() != c.family.len() {
                p.push(format!("{id}: family_female은 family와 같은 개수"));
            }
            let all = c
                .family
                .iter()
                .chain(&c.family_female)
                .chain(&c.middle_male)
                .chain(&c.middle_female)
                .chain(&c.given_male)
                .chain(&c.given_female);
            for part in all {
                if part.chars().any(|ch| ch.is_ascii_lowercase()) || !part.is_ascii() {
                    p.push(format!("{id}: 이름 조각 `{part}`는 영문 대문자"));
                }
            }
            if c.passport.is_empty() || !c.passport.chars().all(|ch| ch == 'L' || ch == '#') {
                p.push(format!("{id}: 여권 형식은 L·#만"));
            }
            if !c.address.contains("{province}") || c.provinces.is_empty() {
                p.push(format!("{id}: 주소 형식에 {{province}}와 provinces 필요"));
            }
        }
        p
    }
}

impl Banks {
    pub fn validate(&self) -> Vec<String> {
        let mut p = Vec::new();
        for b in &self.bank {
            if b.groups.is_empty() || b.groups.contains(&0) {
                p.push(format!("{}: 묶음 자릿수가 비었거나 0", b.name));
            }
            let first = b.groups.first().copied().unwrap_or(0);
            for pre in &b.prefixes {
                if pre.len() != first || !pre.bytes().all(|x| x.is_ascii_digit()) {
                    p.push(format!(
                        "{}: 접두 `{pre}`는 첫 묶음 자릿수({first})의 숫자",
                        b.name
                    ));
                }
            }
            let total: usize = b.groups.iter().sum();
            if !(10..=14).contains(&total) {
                p.push(format!("{}: 계좌번호 전체 자릿수 {total} (10~14)", b.name));
            }
        }
        p
    }
}
