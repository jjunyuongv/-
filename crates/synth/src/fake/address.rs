//! 주소: 국내(시·도 + 시·군·구 + [읍·면] + 그 지역 도로명 + 건물번호 + 상세·참고항목), 외국인 본국 주소
//!
//! 도로와 동은 공식 도로명주소 자료에서 짝으로 뽑은 것만 쓴다 (`assets/synth/addresses_ko.toml`).
//! 동 지역은 동을 참고항목 괄호에, 읍·면 지역은 읍·면을 도로명 앞에 쓴다.

use rand::{Rng, RngExt};

use super::data::{ApartmentParts, Country, District, KoreanAddresses, is_eup_myeon};
use super::pick;

/// 국내 주소 (구성 요소를 따로 들고 있어 검증·표기 변형에 쓴다)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KoreanAddress {
    pub sido: String,
    /// 약식 표기용 시·도 (확정된 약칭이 없으면 정식 명칭과 같음)
    pub sido_short: String,
    pub sigungu: String,
    pub road: String,
    pub building: String,
    /// 동·호수 또는 층 (없을 수 있음)
    pub detail: Option<String>,
    /// 그 도로에 실제로 있는 법정동·읍·면
    pub area: String,
    pub apartment: Option<String>,
}

/// 주소 표기 방식
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressStyle {
    /// 서울특별시 강남구 테헤란로 123, 101동 1203호 (역삼동, 한빛마을아파트)
    Full,
    /// 서울 강남구 테헤란로 123, 101동 1203호
    Short,
    /// 서울특별시 강남구 테헤란로 123 (참고항목·상세 없음)
    RoadOnly,
}

impl AddressStyle {
    pub const ALL: [AddressStyle; 3] = [
        AddressStyle::Full,
        AddressStyle::Short,
        AddressStyle::RoadOnly,
    ];
}

/// 주소를 고를 지역 (업종별 근무장소)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// 제한 없음
    Any,
    /// 특별·광역시 자치구 (사무직)
    CityCenter,
    /// 시·군 (자치구가 아닌 곳: 도의 시, 시의 일반구, 세종) — 건설
    SiGun,
    /// 읍·면 지역 도로 — 농업·축산업·어업
    Rural,
}

impl KoreanAddress {
    /// 읍·면 지역 주소인가
    pub fn is_rural(&self) -> bool {
        is_eup_myeon(&self.area)
    }

    pub fn render(&self, style: AddressStyle) -> String {
        let sido = match style {
            AddressStyle::Short => &self.sido_short,
            _ => &self.sido,
        };
        let mut s = sido.clone();
        if !self.sigungu.is_empty() {
            s.push(' ');
            s.push_str(&self.sigungu);
        }
        if self.is_rural() {
            s.push(' ');
            s.push_str(&self.area);
        }
        s.push_str(&format!(" {} {}", self.road, self.building));
        if style != AddressStyle::RoadOnly
            && let Some(d) = &self.detail
        {
            s.push_str(&format!(", {d}"));
        }
        if style == AddressStyle::Full {
            // 참고항목: 동 지역은 (동[, 아파트]), 읍·면 지역은 (아파트)만
            let mut refs = Vec::new();
            if !self.is_rural() {
                refs.push(self.area.as_str());
            }
            if let Some(a) = &self.apartment {
                refs.push(a.as_str());
            }
            if !refs.is_empty() {
                s.push_str(&format!(" ({})", refs.join(", ")));
            }
        }
        s
    }
}

/// 구역 하나 안에서 주소를 만든다 (도로는 그 구역 목록에서, 동·읍·면은 그 도로의 짝에서만).
/// `rural_only`면 읍·면 지역 도로와 그 읍·면만 쓴다.
pub fn korean_address_in<R: Rng + ?Sized>(
    rng: &mut R,
    d: &District,
    apt: &ApartmentParts,
    rural_only: bool,
) -> KoreanAddress {
    let roads: Vec<_> = d
        .roads
        .iter()
        .filter(|r| !rural_only || r.is_rural())
        .collect();
    let road = pick(rng, &roads).copied().or_else(|| d.roads.first());
    let (road_name, max_no, area) = match road {
        Some(r) => {
            let areas: Vec<&String> = r
                .areas
                .iter()
                .filter(|a| !rural_only || is_eup_myeon(a))
                .collect();
            let area = pick(rng, &areas).copied().or_else(|| r.areas.first());
            (
                r.name.clone(),
                r.max_no.max(1),
                area.cloned().unwrap_or_default(),
            )
        }
        None => (String::new(), 1, String::new()),
    };
    let main = rng.random_range(1..=max_no);
    let building = if rng.random_bool(0.2) {
        format!("{main}-{}", rng.random_range(1..=30u8))
    } else {
        main.to_string()
    };
    let (detail, apartment) = match rng.random_range(0..3u8) {
        // 아파트: 동·호수 + 아파트 이름
        0 => {
            let name = format!(
                "{}{}아파트",
                pick(rng, &apt.prefix).cloned().unwrap_or_default(),
                pick(rng, &apt.suffix).cloned().unwrap_or_default()
            );
            let floor = rng.random_range(1..=25u8);
            let unit = rng.random_range(1..=6u8);
            (
                Some(format!(
                    "{}동 {floor}{unit:02}호",
                    rng.random_range(101..=120u16)
                )),
                Some(name),
            )
        }
        // 다세대·오피스텔 호수
        1 => (Some(format!("{}호", rng.random_range(101..=605u16))), None),
        // 단독·상가 층
        _ => (Some(format!("{}층", rng.random_range(1..=5u8))), None),
    };
    KoreanAddress {
        sido: d.sido.clone(),
        sido_short: d.sido_short_or_full().to_string(),
        sigungu: d.sigungu.clone(),
        road: road_name,
        building,
        detail,
        area,
        apartment,
    }
}

/// 구역을 균등하게 골라 주소를 만든다
pub fn korean_address<R: Rng + ?Sized>(
    rng: &mut R,
    data: &KoreanAddresses,
) -> Option<KoreanAddress> {
    korean_address_where(rng, data, Region::Any)
}

/// 지역 조건에 맞는 구역에서 주소를 만든다 (맞는 구역이 없으면 전체에서)
pub fn korean_address_where<R: Rng + ?Sized>(
    rng: &mut R,
    data: &KoreanAddresses,
    region: Region,
) -> Option<KoreanAddress> {
    let fits = |d: &&District| match region {
        Region::Any => true,
        Region::CityCenter => d.is_city_center(),
        Region::SiGun => !d.is_city_center(),
        Region::Rural => d.roads.iter().any(|r| r.is_rural()),
    };
    let candidates: Vec<&District> = data.district.iter().filter(fits).collect();
    let d = match pick(rng, &candidates) {
        Some(d) => *d,
        None => pick(rng, &data.district)?,
    };
    Some(korean_address_in(
        rng,
        d,
        &data.apartment,
        region == Region::Rural,
    ))
}

/// 외국인 본국 주소 (나라별 형식)
pub fn foreign_address<R: Rng + ?Sized>(rng: &mut R, c: &Country) -> String {
    let province = pick(rng, &c.provinces).cloned().unwrap_or_default();
    let mut out = c.address.replace("{province}", &province);
    while out.contains("{num}") {
        out = out.replacen("{num}", &rng.random_range(1..=300u16).to_string(), 1);
    }
    while out.contains("{n}") {
        out = out.replacen("{n}", &rng.random_range(1..=15u8).to_string(), 1);
    }
    out
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

    /// 표기한 주소에서 시·도/시·군·구/[읍·면]/도로명을 다시 꺼낸다 (RoadOnly 표기 기준)
    fn parse_road_only(s: &str) -> (String, String, Option<String>, String) {
        let words: Vec<&str> = s.split(' ').collect();
        let sido = words[0].to_string();
        // 도로명 = "로"/"길"로 끝나는 첫 단어. 바로 앞 단어가 읍·면이면 읍·면, 나머지가 시·군·구 (시+구 두 단어일 수 있음)
        let road_idx = words
            .iter()
            .position(|w| w.ends_with('로') || w.ends_with('길'))
            .unwrap();
        let (end, eup) = if road_idx > 1 && is_eup_myeon(words[road_idx - 1]) {
            (road_idx - 1, Some(words[road_idx - 1].to_string()))
        } else {
            (road_idx, None)
        };
        let sigungu = words[1..end].join(" ");
        (sido, sigungu, eup, words[road_idx].to_string())
    }

    #[test]
    fn roads_and_areas_are_official_pairs() {
        let d = data();
        let mut r = seeded(41);
        let mut rural = 0;
        for _ in 0..3000 {
            let a = korean_address(&mut r, &d.addresses).unwrap();
            let district = d.addresses.find(&a.sido, &a.sigungu).unwrap();
            let road = district.road(&a.road).expect("구역에 없는 도로");
            // 동·읍·면은 그 도로의 짝에서만
            assert!(road.areas.contains(&a.area), "{a:?}");
            let main: u32 = a.building.split('-').next().unwrap().parse().unwrap();
            assert!((1..=road.max_no).contains(&main), "{a:?}");

            // 표기한 문자열을 다시 분해해도 같은 구역의 도로·읍면
            let (sido, sigungu, eup, road_name) =
                parse_road_only(&a.render(AddressStyle::RoadOnly));
            let back = d.addresses.find(&sido, &sigungu).unwrap();
            let back_road = back.road(&road_name).unwrap();
            match eup {
                Some(e) => {
                    rural += 1;
                    assert!(back_road.areas.contains(&e), "{e} {road_name}");
                }
                None => assert!(!a.is_rural(), "{a:?}"),
            }
        }
        assert!(rural > 0, "읍·면 주소가 한 번도 안 나옴");
    }

    #[test]
    fn district_table_is_valid_and_all_used() {
        // 같은 이름의 도로가 여러 구역에 있을 수 있다 (예: 중앙로) — 구역별로 공식 자료에서 뽑았으므로 구역 안에서만 본다
        let d = data();
        assert!(d.addresses.validate().is_empty());
        let mut r = seeded(42);
        let mut used = std::collections::BTreeSet::new();
        for _ in 0..5000 {
            let a = korean_address(&mut r, &d.addresses).unwrap();
            used.insert(format!("{} {}", a.sido, a.sigungu));
        }
        assert_eq!(used.len(), d.addresses.district.len());
    }

    #[test]
    fn regions_pick_matching_districts() {
        let d = data();
        let mut r = seeded(45);
        for _ in 0..500 {
            let a = korean_address_where(&mut r, &d.addresses, Region::CityCenter).unwrap();
            assert!(
                d.addresses
                    .find(&a.sido, &a.sigungu)
                    .unwrap()
                    .is_city_center()
            );
            let a = korean_address_where(&mut r, &d.addresses, Region::SiGun).unwrap();
            assert!(
                !d.addresses
                    .find(&a.sido, &a.sigungu)
                    .unwrap()
                    .is_city_center()
            );
            let a = korean_address_where(&mut r, &d.addresses, Region::Rural).unwrap();
            assert!(a.is_rural(), "{a:?}");
            let full = a.render(AddressStyle::Full);
            // 읍·면 주소: 읍·면이 도로명 바로 앞, 참고항목에 읍·면 없음
            assert!(full.contains(&format!(" {} {} ", a.area, a.road)), "{full}");
            assert!(!full.contains(&format!("({}", a.area)), "{full}");
        }
    }

    #[test]
    fn styles_render() {
        let d = data();
        let gangnam = d.addresses.find("서울특별시", "강남구").unwrap();
        let mut r = seeded(43);
        let a = korean_address_in(&mut r, gangnam, &d.addresses.apartment, false);
        let full = a.render(AddressStyle::Full);
        let short = a.render(AddressStyle::Short);
        assert!(full.starts_with("서울특별시 강남구 "), "{full}");
        assert!(short.starts_with("서울 강남구 "), "{short}");
        assert!(full.contains(&format!("({}", a.area)));
        // 약칭이 확정되지 않은 시·도는 약식 표기에서도 정식 명칭
        let mokpo = d.addresses.find("전남광주통합특별시", "목포시").unwrap();
        assert!(mokpo.sido_short.is_none());
        let s = korean_address_in(&mut r, mokpo, &d.addresses.apartment, false)
            .render(AddressStyle::Short);
        assert!(s.starts_with("전남광주통합특별시 목포시 "), "{s}");
        let sejong = d.addresses.find("세종특별자치시", "").unwrap();
        for _ in 0..50 {
            let s = korean_address_in(&mut r, sejong, &d.addresses.apartment, false)
                .render(AddressStyle::RoadOnly);
            assert!(s.starts_with("세종특별자치시 ") && !s.contains("  "), "{s}");
        }
    }

    #[test]
    fn foreign_addresses_fill_all_placeholders() {
        let d = data();
        let mut r = seeded(44);
        for c in &d.foreign.country {
            let a = foreign_address(&mut r, c);
            assert!(!a.contains('{'), "{a}");
            assert!(c.provinces.iter().any(|p| a.contains(p.as_str())));
        }
    }
}
