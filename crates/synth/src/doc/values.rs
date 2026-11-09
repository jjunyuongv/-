//! 문서 하나의 값 문맥: 사람·회사·날짜·금액을 문서 안에서 일관되게 만들고, 템플릿 슬롯 값을 채운다.
//!
//! - 같은 슬롯 이름 → 같은 값 (캐시). 레이아웃 D(연소근로자 + 친권자 동의서)도 한 문맥을 공유
//! - 날짜: 계약일 ≤ 시작일 < 종료일. 금액: 건설 제수당 합계 = 각 수당 합, 외국인 월 통상임금 = 기본급 + 고정 수당
//! - 사람: 근로자(레이아웃 C는 외국인, D는 만 15~17세), 친권자(만 38~55세, 주소 70%는 근로자와 같음), 대표자

use std::collections::BTreeMap;

use pii_core::{AttributeKind, Owner};
use rand::RngExt;

use super::{FieldKind, SynthConfig, SynthError};
use crate::fake::address::{
    AddressStyle, KoreanAddress, Region, foreign_address, korean_address, korean_address_where,
};
use crate::fake::data::{Gender, Generation};
use crate::fake::finance::{
    Account, KoreanPassport, account, driver_license, foreign_passport, korean_passport,
    license_text,
};
use crate::fake::ids::{BizKind, bizno, frn, rrn, rrn_kind};
use crate::fake::person::{ForeignName, email, foreign_name_in, korean_name};
use crate::fake::phone::{PhoneGen, mobile};
use crate::fake::{NumberStyle, NumberText, Separator, SynthRng, Ymd, digits, pick};
use crate::template::{EmployerKind, Layout, Slot, SlotKind, Template};

/// 외국인 근로자 정보
#[derive(Debug, Clone)]
pub struct Foreigner {
    pub country: usize,
    pub name: ForeignName,
    pub passport: String,
    pub frn: NumberText,
    pub home: String,
}

/// 사람 하나
#[derive(Debug, Clone)]
pub struct Person {
    pub name: String,
    pub gender: Gender,
    pub birth: Ymd,
    pub phone: NumberText,
    pub address: KoreanAddress,
    pub rrn: NumberText,
    pub foreign: Option<Foreigner>,
}

/// 회사
#[derive(Debug, Clone)]
pub struct Company {
    pub name: String,
    pub address: KoreanAddress,
    pub phone: NumberText,
    pub bizno: NumberText,
}

/// 슬롯 값
#[derive(Debug, Clone, PartialEq)]
pub struct SlotValue {
    pub text: String,
    pub kind: FieldKind,
}

const WEEKDAYS: [&str; 7] = ["월", "화", "수", "목", "금", "토", "일"];

/// 친권자 동의서 날짜가 계약일보다 앞설 수 있는 최대 일수 (레이아웃 D)
pub const CONSENT_LEAD_DAYS: i64 = 7;

/// 사회보험 묶음의 산재보험 칸 (일부 체크여도 항상 체크)
const INSURANCE_ACCIDENT: &str = "ins_accident";

/// 단시간 근로자 요일별 휴게시간 (분)
pub const PART_TIME_REST_MIN: u32 = 30;

/// 연소근로자 법정 한도 (근로기준법 제69조): 하루 7시간, 주 35시간
pub const MINOR_DAILY_MAX: u32 = 7;
pub const MINOR_WEEKLY_MAX: u32 = 35;

/// (시, 분)에 분을 더한다
fn add_minutes((h, m): (u8, u8), minutes: u32) -> (u8, u8) {
    let t = u32::from(h) * 60 + u32::from(m) + minutes;
    ((t / 60).min(23) as u8, (t % 60) as u8)
}

/// 업종별 회사 이름 끝말
fn company_suffixes(industry: &str) -> &'static [&'static str] {
    match industry {
        "제조" => &["산업", "정밀", "테크", "화학"],
        "물류" => &["물류", "로지스", "운수"],
        "건설" => &["건설", "종합건설", "개발"],
        "병원" => &["병원", "요양병원", "의원"],
        "유통" => &["마트", "유통", "상사"],
        "사무" => &["컨설팅", "솔루션", "기획"],
        "교육" => &["학원", "교육", "에듀"],
        "농축수산" => &["농장", "영농조합법인", "수산", "축산"],
        _ => &["돌봄센터", "케어"],
    }
}

/// 업종별 직무
fn job_titles(industry: &str) -> &'static [&'static str] {
    match industry {
        "제조" => &["생산직", "품질검사", "설비보전", "포장"],
        "물류" => &["상하차", "배송기사", "재고관리", "지게차 운전"],
        "건설" => &["철근공", "형틀목공", "보통인부", "전기공"],
        "병원" => &["간호조무사", "원무", "간병", "조리"],
        "유통" => &["판매", "계산원", "진열", "물류"],
        "사무" => &["총무", "회계", "고객상담", "디자인"],
        "교육" => &["강사", "행정", "상담"],
        "농축수산" => &["재배", "수확", "사육", "선별 작업"],
        _ => &["가사도우미", "요양보호사"],
    }
}

fn industry_desc(
    industry: &str,
    agri: bool,
) -> (
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
) {
    if agri || industry == "농축수산" {
        return (
            &["농업(시설원예)", "축산업", "어업(양식)"],
            &["딸기 재배", "돼지 사육", "굴 양식", "버섯 재배"],
            &["작물 재배 및 수확", "축사 관리", "어로 작업", "선별·포장"],
        );
    }
    match industry {
        "건설" => (&["건설업"], &["건축 공사"], &["형틀 작업", "철근 작업"]),
        "물류" => (&["운수업"], &["물류 창고 운영"], &["상하차", "분류"]),
        _ => (
            &["제조업"],
            &["자동차 부품 제조", "식품 제조", "금속 가공"],
            &["생산 라인 작업", "검사", "포장"],
        ),
    }
}

/// 숫자에 천 단위 쉼표
pub fn comma(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 문서 하나의 값 문맥
pub struct Ctx<'a> {
    pub cfg: &'a SynthConfig,
    pub rng: SynthRng,
    pub phones: PhoneGen,
    pub layout: Layout,
    pub forms: Vec<String>,
    pub employer_kind: EmployerKind,
    pub industry: String,
    pub contract: Ymd,
    pub hand_share: Option<f64>,
    /// 국내 양식을 쓰는 외국인 근로자 (텍스트 1,000건 추가 항목)
    pub foreign_worker: bool,
    /// 업종과 관계없이 추가 항목에 운전면허번호
    pub force_license: bool,
    persons: BTreeMap<u8, Person>,
    company: Option<Company>,
    dates: BTreeMap<String, Ymd>,
    times: BTreeMap<String, (u8, u8)>,
    wons: BTreeMap<String, u64>,
    numbers: BTreeMap<String, u32>,
    checks: BTreeMap<String, bool>,
    cache: BTreeMap<String, SlotValue>,
    guardian_relation: Option<String>,
}

fn owner_key(o: Owner) -> u8 {
    match o {
        Owner::Worker => 0,
        Owner::Employer => 1,
        Owner::Guardian => 2,
    }
}

impl<'a> Ctx<'a> {
    pub fn new(
        cfg: &'a SynthConfig,
        mut rng: SynthRng,
        layout: Layout,
        forms: Vec<String>,
    ) -> Result<Self, SynthError> {
        let phones = PhoneGen::new(&cfg.ratios)?;
        let employer_kind =
            if cfg.choose(&mut rng, "employer_kind", ["individual", "corporation"])? == 0 {
                EmployerKind::Individual
            } else {
                EmployerKind::Corporation
            };
        let industry = Self::pick_industry(cfg, &mut rng, &forms)?;
        let hand_share = if cfg.choose(&mut rng, "handwriting_doc", ["yes", "no"])? == 0 {
            // PLAN.md 4단계: 손글씨 문서 안 기입 필드의 50~100%
            Some(rng.random_range(0.5..=1.0))
        } else {
            None
        };
        let contract = Ymd {
            year: 2025,
            month: 1,
            day: 1,
        }
        .add_days(rng.random_range(0..=660));
        Ok(Ctx {
            cfg,
            rng,
            phones,
            layout,
            forms,
            employer_kind,
            industry,
            contract,
            hand_share,
            foreign_worker: false,
            force_license: false,
            persons: BTreeMap::new(),
            company: None,
            dates: BTreeMap::new(),
            times: BTreeMap::new(),
            wons: BTreeMap::new(),
            numbers: BTreeMap::new(),
            checks: BTreeMap::new(),
            cache: BTreeMap::new(),
            guardian_relation: None,
        })
    }

    fn pick_industry(
        cfg: &SynthConfig,
        rng: &mut SynthRng,
        forms: &[String],
    ) -> Result<String, SynthError> {
        if forms.iter().any(|f| f == "construction_daily") {
            return Ok("건설".into());
        }
        if forms.iter().any(|f| f == "foreign_worker_agri") {
            return Ok("농축수산".into());
        }
        let keys = [
            "제조",
            "물류",
            "건설",
            "병원",
            "유통",
            "사무",
            "교육",
            "농축수산",
            "가사돌봄",
        ];
        loop {
            let i = cfg.choose(rng, "industry", keys)?;
            let ind = keys[i];
            // 외국인 일반(E-9) 문서는 제조·건설·물류 중에서
            if forms.iter().any(|f| f == "foreign_worker")
                && !matches!(ind, "제조" | "건설" | "물류")
            {
                continue;
            }
            return Ok(ind.into());
        }
    }

    /// 근로자가 외국인 (레이아웃 C, 또는 국내 양식의 외국인 근로자)
    fn is_foreign_worker(&self) -> bool {
        self.layout == Layout::C || self.foreign_worker
    }

    fn is_minor_doc(&self) -> bool {
        self.forms
            .iter()
            .any(|f| f == "minor" || f == "guardian_consent")
    }

    fn gender(&mut self) -> Result<Gender, SynthError> {
        Ok(
            if self
                .cfg
                .choose(&mut self.rng, "gender", ["male", "female"])?
                == 0
            {
                Gender::Male
            } else {
                Gender::Female
            },
        )
    }

    /// 번호 표기 방식 (구분자·전각 비율)
    pub fn number_style(&mut self, maskable: bool) -> Result<NumberStyle, SynthError> {
        let sep = [
            Separator::Hyphen,
            Separator::None,
            Separator::Space,
            Separator::Dot,
        ][self.cfg.choose(
            &mut self.rng,
            "number_separator",
            ["hyphen", "none", "space", "dot"],
        )?];
        let fullwidth = self
            .cfg
            .choose(&mut self.rng, "fullwidth", ["fullwidth", "halfwidth"])?
            == 0;
        let mask_tail = if maskable
            && self
                .cfg
                .choose(&mut self.rng, "rrn_mask", ["masked", "full"])?
                == 0
        {
            6
        } else {
            0
        };
        Ok(NumberStyle {
            sep,
            fullwidth,
            mask_tail,
        })
    }

    pub fn address_style(&mut self) -> Result<AddressStyle, SynthError> {
        Ok(AddressStyle::ALL[self.cfg.choose(
            &mut self.rng,
            "address_style",
            ["full", "short", "road_only"],
        )?])
    }

    fn new_address(&mut self) -> Result<KoreanAddress, SynthError> {
        korean_address(&mut self.rng, &self.cfg.data.addresses)
            .ok_or_else(|| SynthError::Assemble("주소 자료가 비어 있음".into()))
    }

    /// 소속별 사람 (처음 쓸 때 만든다)
    pub fn person(&mut self, owner: Owner) -> Result<Person, SynthError> {
        if let Some(p) = self.persons.get(&owner_key(owner)) {
            return Ok(p.clone());
        }
        let gender = self.gender()?;
        let (min_age, max_age) = match owner {
            Owner::Worker if self.is_minor_doc() => (15, 17),
            // 외국인(고용허가제): 응시 요건 만 18~39세 + 체류 연장·재입국 → 만 20~45세
            Owner::Worker if self.is_foreign_worker() => (20, 45),
            Owner::Worker => (19, 60),
            Owner::Guardian => (38, 55),
            Owner::Employer => (35, 65),
        };
        let birth =
            crate::fake::date::birth_date_for_age(&mut self.rng, self.contract, min_age, max_age);
        let foreign = if owner == Owner::Worker && self.is_foreign_worker() {
            let countries = &self.cfg.data.foreign.country;
            let ci = self.rng.random_range(0..countries.len());
            let c = &countries[ci];
            Some(Foreigner {
                country: ci,
                name: foreign_name_in(&mut self.rng, c, gender),
                passport: foreign_passport(&mut self.rng, c),
                frn: frn(&mut self.rng, birth),
                home: foreign_address(&mut self.rng, c),
            })
        } else {
            None
        };
        let name = match &foreign {
            Some(f) => f.name.full(),
            None => korean_name(
                &mut self.rng,
                &self.cfg.data.names,
                Generation::for_birth_year(birth.year),
                gender,
            )
            .full(),
        };
        let address = if owner == Owner::Guardian
            && self.cfg.choose(
                &mut self.rng,
                "guardian_same_address",
                ["same", "different"],
            )? == 0
        {
            self.person(Owner::Worker)?.address
        } else {
            self.new_address()?
        };
        let kind = rrn_kind(&mut self.rng, &self.cfg.ratios)?;
        let p = Person {
            name,
            gender,
            birth,
            phone: mobile(&mut self.rng),
            address,
            rrn: rrn(&mut self.rng, birth, kind),
            foreign,
        };
        self.persons.insert(owner_key(owner), p.clone());
        Ok(p)
    }

    pub fn company(&mut self) -> Result<Company, SynthError> {
        if let Some(c) = &self.company {
            return Ok(c.clone());
        }
        const PREFIX: [&str; 10] = [
            "가람",
            "한빛",
            "새솔",
            "늘해",
            "다온",
            "솔내",
            "푸른",
            "햇살",
            "온누리",
            "바른",
        ];
        let prefix = pick(&mut self.rng, &PREFIX).copied().unwrap_or("가람");
        let suffix = pick(&mut self.rng, company_suffixes(&self.industry))
            .copied()
            .unwrap_or("산업");
        let mut name = format!("{prefix}{suffix}");
        if self.employer_kind == EmployerKind::Corporation
            && !name.ends_with("법인")
            && self.rng.random_bool(0.5)
        {
            name = if self.rng.random_bool(0.5) {
                format!("(주){name}")
            } else {
                format!("주식회사 {name}")
            };
        }
        let kind = match self.employer_kind {
            EmployerKind::Individual => BizKind::Individual,
            EmployerKind::Corporation => BizKind::Corporation,
        };
        // 업종별 근무장소 지역: 농축수산 → 읍·면, 건설 → 시·군, 사무 → 특별·광역시 자치구
        let region = match self.industry.as_str() {
            "농축수산" => Region::Rural,
            "건설" => Region::SiGun,
            "사무" => Region::CityCenter,
            _ => Region::Any,
        };
        let address = korean_address_where(&mut self.rng, &self.cfg.data.addresses, region)
            .ok_or_else(|| SynthError::Assemble("주소 자료가 비어 있음".into()))?;
        let phone = self
            .phones
            .company_at(&mut self.rng, &address.sido, &address.sigungu);
        let c = Company {
            name,
            address,
            phone,
            bizno: bizno(&mut self.rng, kind),
        };
        self.company = Some(c.clone());
        Ok(c)
    }

    pub fn guardian_relation(&mut self) -> Result<String, SynthError> {
        if let Some(r) = &self.guardian_relation {
            return Ok(r.clone());
        }
        let g = self.person(Owner::Guardian)?.gender;
        let i = self.cfg.choose(
            &mut self.rng,
            "guardian_relation",
            ["parent", "grandparent", "guardian"],
        )?;
        let r = match (i, g) {
            (0, Gender::Male) => "부",
            (0, Gender::Female) => "모",
            (1, Gender::Male) => "조부",
            (1, Gender::Female) => "조모",
            _ => "후견인",
        }
        .to_string();
        self.guardian_relation = Some(r.clone());
        Ok(r)
    }

    fn date(&mut self, name: &str) -> Ymd {
        if let Some(d) = self.dates.get(name) {
            return *d;
        }
        let d = match name {
            "contract" => self.contract,
            // 친권자 동의서는 계약 당일이나 며칠 전에 받는다 (근로기준법 제66조: 동의서를 사업장에 갖춰야 함)
            "consent" => self
                .contract
                .add_days(-self.rng.random_range(0..=CONSENT_LEAD_DAYS)),
            "start" | "change_start" => self.contract.add_days(self.rng.random_range(0..=14)),
            // 외국인 양식: 사업장변경자 기간 = 계약 개월 수 (시작일 + n개월 - 1일)
            "change_end" => {
                let start = self.date("change_start");
                let months = self.number("contract_months");
                start.add_months(months).add_days(-1)
            }
            "end" => {
                let start = self.date("start");
                let days = if self.industry == "건설" {
                    self.rng.random_range(30..=180)
                } else {
                    364
                };
                start.add_days(days)
            }
            _ => self.contract.add_days(-self.rng.random_range(30..=300)),
        };
        self.dates.insert(name.to_string(), d);
        d
    }

    fn time(&mut self, name: &str) -> (u8, u8) {
        if let Some(t) = self.times.get(name) {
            return *t;
        }
        let t = if let Some(rest) = name.strip_prefix("day") {
            // day{i}_start / day{i}_end / day{i}_rest_start / day{i}_rest_end (단시간 요일별)
            let i: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let start = format!("day{i}_start");
            match &rest[i.len()..] {
                "_start" => (
                    self.rng.random_range(9..=14u8),
                    if self.rng.random_bool(0.5) { 0 } else { 30 },
                ),
                // 근로시간 = 종업 - 시업 - 휴게(30분) → 종업 = 시업 + 근로시간 + 30분
                "_end" => {
                    let start = self.time(&start);
                    let hours = self.number(&format!("daily_hours:{i}"));
                    add_minutes(start, hours * 60 + PART_TIME_REST_MIN)
                }
                "_rest_start" => {
                    let start = self.time(&start);
                    add_minutes(start, 120)
                }
                _ => {
                    let rest = self.time(&format!("day{i}_rest_start"));
                    add_minutes(rest, PART_TIME_REST_MIN)
                }
            }
        } else {
            match name {
                "work_start" => (
                    self.rng.random_range(8..=10u8),
                    if self.rng.random_bool(0.7) { 0 } else { 30 },
                ),
                // 휴게 12~13시(1시간) 포함. 연소근로자(레이아웃 D)는 근로기준법 제69조 한도 안:
                // 하루 7시간·주 35시간 이하 → 하루 근로시간 ≤ min(7, 35 ÷ 주 근무일)
                "work_end" => {
                    let start = self.time("work_start");
                    let daily = if self.is_minor_doc() {
                        self.number("minor_daily_hours")
                    } else {
                        8
                    };
                    add_minutes(start, (daily + 1) * 60)
                }
                "rest_start" => (12, 0),
                _ => (13, 0),
            }
        };
        self.times.insert(name.to_string(), t);
        t
    }

    fn number(&mut self, kind: &str) -> u32 {
        if let Some(n) = self.numbers.get(kind) {
            return *n;
        }
        if kind == "minor_daily_hours" {
            let days = self.number("workdays_per_week");
            let max = MINOR_DAILY_MAX.min(MINOR_WEEKLY_MAX / days.max(1));
            let n = self.rng.random_range(4.min(max)..=max);
            self.numbers.insert(kind.to_string(), n);
            return n;
        }
        let r = &mut self.rng;
        let construction = self.forms.iter().any(|f| f == "construction_daily");
        let n = match kind.split(':').next().unwrap_or_default() {
            "workdays_per_week" => *pick(r, &[5, 5, 5, 6]).unwrap_or(&5),
            "day_of_month" => *pick(r, &[5, 10, 15, 20, 25]).unwrap_or(&10),
            "monthly_hours" if construction => r.random_range(4..=30),
            "monthly_hours" => *pick(r, &[209, 226, 243]).unwrap_or(&209),
            "daily_hours" => r.random_range(3..=6),
            "contract_months" => *pick(r, &[12, 24, 36]).unwrap_or(&12),
            "probation_months" => r.random_range(1..=3),
            "daily_overtime_hours" => r.random_range(1..=2),
            "max_overtime_hours" => r.random_range(2..=4),
            "rest_minutes" => *pick(r, &[30, 60]).unwrap_or(&60),
            "rest_count" => r.random_range(1..=2),
            "rest_hours" => 1,
            "overtime_rate_percent" => 50,
            "seasonal_variation_hours" => r.random_range(1..=3),
            "extension" => r.random_range(100..=9999),
            _ => r.random_range(1..=9),
        };
        self.numbers.insert(kind.to_string(), n);
        n
    }

    fn won(&mut self, name: &str) -> u64 {
        if let Some(w) = self.wons.get(name) {
            return *w;
        }
        let step = |r: &mut SynthRng, lo: u64, hi: u64, unit: u64| {
            r.random_range(lo / unit..=hi / unit) * unit
        };
        let w = match name {
            "wage" if self.forms.iter().any(|f| f == "part_time") => {
                step(&mut self.rng, 10_320, 15_000, 10)
            }
            "wage" if self.forms.iter().any(|f| f == "construction_daily") => {
                step(&mut self.rng, 150_000, 260_000, 1_000)
            }
            "wage" if self.layout == Layout::C => step(&mut self.rng, 2_100_000, 2_600_000, 10_000),
            "wage" => step(&mut self.rng, 2_200_000, 4_200_000, 10_000),
            "allowance_total" => self.won("overtime") + self.won("night") + self.won("holiday"),
            "monthly_normal" => self.won("base") + self.won("fixed1") + self.won("fixed2"),
            "base" => step(&mut self.rng, 2_100_000, 2_400_000, 10_000),
            "bonus" => step(&mut self.rng, 100_000, 1_000_000, 50_000),
            "probation" | "probation_3m" => {
                let base = if self.layout == Layout::C {
                    self.won("monthly_normal")
                } else {
                    self.won("wage")
                };
                base * 9 / 10 / 10 * 10
            }
            "overtime_hourly" => step(&mut self.rng, 15_000, 20_000, 10),
            "accommodation_cost" => step(&mut self.rng, 100_000, 300_000, 10_000),
            "meal_cost" => step(&mut self.rng, 50_000, 150_000, 10_000),
            "min_wage_hourly" => *pick(&mut self.rng, &[9_860, 10_030, 10_320]).unwrap_or(&10_320),
            _ => step(&mut self.rng, 50_000, 300_000, 10_000),
        };
        self.wons.insert(name.to_string(), w);
        w
    }

    /// `requires` 조건 (`!` 접두사면 체크 안 됐을 때)
    fn requires_met(&mut self, tpl: &Template, r: &str) -> Result<bool, SynthError> {
        match r.strip_prefix('!') {
            Some(slot) => Ok(!self.check(tpl, slot)?),
            None => self.check(tpl, r),
        }
    }

    /// 체크 칸 상태 (묶음 단위로 정함)
    fn check(&mut self, tpl: &Template, slot_id: &str) -> Result<bool, SynthError> {
        if let Some(c) = self.checks.get(slot_id) {
            return Ok(*c);
        }
        let Some(slot) = tpl.slots.get(slot_id) else {
            return Ok(false);
        };
        let group = slot.group.clone().unwrap_or_default();
        let members: Vec<(String, Slot)> = tpl
            .slots
            .iter()
            .filter(|(_, s)| {
                s.kind == SlotKind::Check && s.group.as_deref() == Some(group.as_str())
            })
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect();
        // requires가 꺼져 있으면 묶음 전체가 빈칸
        let enabled = match &slot.requires {
            Some(r) => self.requires_met(tpl, r)?,
            None => true,
        };
        if slot.multi && group == "insurance" {
            // 사회보험: 대부분 4개 모두 체크, 일부만 체크한 조합은 낮은 비율 (insurance_checks)
            let all = self
                .cfg
                .choose(&mut self.rng, "insurance_checks", ["all", "partial"])?
                == 0;
            // 일부 체크여도 산재보험은 항상 체크 (모든 사업장 당연 적용 — 사용자 결정 2026-11-08)
            let accident: Vec<bool> = members
                .iter()
                .map(|(k, _)| k == INSURANCE_ACCIDENT)
                .collect();
            let mut on = Vec::with_capacity(members.len());
            for is_accident in &accident {
                on.push(
                    all || *is_accident
                        || self.cfg.choose(
                            &mut self.rng,
                            "check_multi",
                            ["checked", "unchecked"],
                        )? == 0,
                );
            }
            let others: Vec<usize> = (0..on.len()).filter(|i| !accident[*i]).collect();
            if !all && on.iter().all(|x| *x) && !others.is_empty() {
                // '일부'인데 모두 켜졌으면 산재보험이 아닌 것 하나를 끈다
                let i = others[self.rng.random_range(0..others.len())];
                on[i] = false;
            }
            for ((k, _), v) in members.iter().zip(on) {
                self.checks.insert(k.clone(), enabled && v);
            }
        } else if slot.multi {
            for (k, _) in &members {
                let on = enabled
                    && self
                        .cfg
                        .choose(&mut self.rng, "check_multi", ["checked", "unchecked"])?
                        == 0;
                self.checks.insert(k.clone(), on);
            }
        } else {
            // 칸이 하나뿐인 묶음(단시간 양식 기타급여 "없음" 등)은 "체크 안 함"도 한 선택지로 균등하게
            let options = if members.len() == 1 {
                2
            } else {
                members.len().max(1)
            };
            let chosen = if enabled {
                Some(self.rng.random_range(0..options))
            } else {
                None
            };
            for (i, (k, _)) in members.iter().enumerate() {
                self.checks.insert(k.clone(), chosen == Some(i));
            }
        }
        Ok(self.checks.get(slot_id).copied().unwrap_or(false))
    }

    fn birth_text(&mut self, birth: Ymd, foreign: bool) -> String {
        let Ymd { year, month, day } = birth;
        let choices: Vec<String> = if foreign {
            vec![
                format!("{year}/{month:02}/{day:02}"),
                format!("{day:02}/{month:02}/{year}"),
                format!("{year}.{month:02}.{day:02}"),
            ]
        } else {
            vec![
                format!("{year}.{month:02}.{day:02}"),
                format!("{year}년 {month}월 {day}일"),
                format!("{year}-{month:02}-{day:02}"),
                birth.yymmdd(),
            ]
        };
        pick(&mut self.rng, &choices).cloned().unwrap_or_default()
    }

    /// 템플릿 슬롯 하나의 값
    pub fn slot(&mut self, tpl: &Template, slot_id: &str) -> Result<SlotValue, SynthError> {
        if let Some(v) = self.cache.get(slot_id) {
            return Ok(v.clone());
        }
        let slot = tpl.slots.get(slot_id).cloned().ok_or_else(|| {
            SynthError::Assemble(format!("{}: 슬롯 `{slot_id}` 없음", tpl.meta.id))
        })?;
        let v = self.compute(tpl, slot_id, &slot)?;
        self.cache.insert(slot_id.to_string(), v.clone());
        Ok(v)
    }

    fn compute(
        &mut self,
        tpl: &Template,
        slot_id: &str,
        slot: &Slot,
    ) -> Result<SlotValue, SynthError> {
        if slot.kind == SlotKind::Check {
            let checked = self.check(tpl, slot_id)?;
            let text = match (slot.style, checked) {
                (Some(crate::template::CheckStyle::Box), true) => "■",
                (Some(crate::template::CheckStyle::Box), false) => "□",
                (Some(crate::template::CheckStyle::Paren), true) => "(V)",
                (Some(crate::template::CheckStyle::Paren), false) => "( )",
                (_, true) => "[V]",
                (_, false) => "[ ]",
            };
            return Ok(SlotValue {
                text: text.into(),
                kind: FieldKind::Check { checked },
            });
        }
        // requires가 꺼져 있으면 빈칸
        if let Some(r) = &slot.requires
            && !self.requires_met(tpl, r)?
        {
            return Ok(SlotValue {
                text: String::new(),
                kind: FieldKind::Plain,
            });
        }
        // 생성기와 라벨 (choices면 사업주 종류에 맞는 후보에서 비율로)
        let (generator, label, hard_negative) = if slot.choices.is_empty() {
            (
                slot.generator.clone().unwrap_or_default(),
                slot.label,
                slot.hard_negative,
            )
        } else {
            let cands: Vec<&crate::template::Choice> = slot
                .choices
                .iter()
                .filter(|c| c.when.is_none_or(|w| w == self.employer_kind))
                .collect();
            let weights: Vec<f64> = cands
                .iter()
                .map(|c| {
                    c.ratio
                        .as_deref()
                        .and_then(|r| self.cfg.ratios.get(r))
                        .unwrap_or(1.0)
                })
                .collect();
            let c = cands
                .get(crate::fake::pick_weighted(&mut self.rng, &weights))
                .ok_or_else(|| {
                    SynthError::Assemble(format!("`{slot_id}`: 사업주 종류에 맞는 선택지 없음"))
                })?;
            if let Some(r) = &c.ratio {
                self.cfg.ratios.record_ref(r);
            }
            (c.generator.clone(), c.label, c.hard_negative)
        };
        let owner = slot.owner;
        let text = self.generate(&generator, owner, slot)?;
        let kind = if text.is_empty() {
            FieldKind::Plain
        } else if let Some(label) = label {
            FieldKind::Label { label, owner }
        } else if let Some(a) = slot.attribute {
            FieldKind::Attribute(a)
        } else if hard_negative {
            FieldKind::Negative
        } else {
            FieldKind::Plain
        };
        Ok(SlotValue { text, kind })
    }

    fn generate(
        &mut self,
        generator: &str,
        owner: Option<Owner>,
        slot: &Slot,
    ) -> Result<String, SynthError> {
        let mut parts = generator.split(':');
        let base = parts.next().unwrap_or_default();
        let a1 = parts.next().unwrap_or_default().to_string();
        let a2 = parts.next().unwrap_or_default().to_string();
        let who = owner.unwrap_or(Owner::Worker);
        Ok(match base {
            "person_name_ko" | "person_name_en" => self.person(who)?.name,
            "address_ko" if slot.attribute == Some(AttributeKind::WorkplaceAddress) => {
                let style = self.address_style()?;
                self.company()?.address.render(style)
            }
            "address_ko" => {
                let style = self.address_style()?;
                self.person(who)?.address.render(style)
            }
            "address_foreign" => self
                .person(Owner::Worker)?
                .foreign
                .map(|f| f.home)
                .unwrap_or_default(),
            "mobile_phone" => {
                let style = self.number_style(false)?;
                self.person(who)?.phone.render(style)
            }
            "company_phone" => {
                let style = self.number_style(false)?;
                self.company()?.phone.render(style)
            }
            "birth_date" => {
                let p = self.person(who)?;
                self.birth_text(p.birth, p.foreign.is_some())
            }
            "rrn" => {
                let style = self.number_style(true)?;
                self.person(who)?.rrn.render(style)
            }
            "bizno" => {
                let style = self.number_style(false)?;
                self.company()?.bizno.render(style)
            }
            "company_name" => self.company()?.name,
            "literal" => slot.value.clone().unwrap_or_default(),
            "relation_guardian" => self.guardian_relation()?,
            "age_minor" => self
                .person(Owner::Worker)?
                .birth
                .age_at(self.contract)
                .to_string(),
            "date" => {
                let d = self.date(&a1);
                match a2.as_str() {
                    "year" => d.year.to_string(),
                    "month" => d.month.to_string(),
                    "day" => d.day.to_string(),
                    _ => format!("{:02}/{:02}/{:02}", d.year % 100, d.month, d.day),
                }
            }
            "time" => {
                let (h, m) = self.time(&a1);
                match a2.as_str() {
                    "hour" => h.to_string(),
                    "minute" => format!("{m:02}"),
                    _ => format!("{h:02}:{m:02}"),
                }
            }
            "won" => comma(self.won(&a1)),
            "number" => {
                let key = if a2.is_empty() {
                    a1.clone()
                } else {
                    format!("{a1}:{a2}")
                };
                self.number(&key).to_string()
            }
            "weekday" if a1.is_empty() => {
                if slot.owner.is_none() && self.forms.iter().any(|f| f.starts_with("foreign")) {
                    pick(&mut self.rng, &WEEKDAYS[..5])
                        .copied()
                        .unwrap_or("금")
                        .to_string()
                } else {
                    "일".to_string()
                }
            }
            "weekday" => {
                let i: usize = a1.parse().unwrap_or(1);
                WEEKDAYS
                    .get(i.saturating_sub(1))
                    .copied()
                    .unwrap_or("월")
                    .to_string()
            }
            "workplace" => {
                let style = self.address_style()?;
                self.company()?.address.render(style)
            }
            "job_title" => pick(&mut self.rng, job_titles(&self.industry))
                .copied()
                .unwrap_or("생산직")
                .to_string(),
            "industry" | "business_desc" | "job_desc" => {
                let (ind, biz, job) = industry_desc(&self.industry, a1 == "agri");
                let list = match base {
                    "industry" => ind,
                    "business_desc" => biz,
                    _ => job,
                };
                pick(&mut self.rng, list)
                    .copied()
                    .unwrap_or_default()
                    .to_string()
            }
            "allowance_name" => pick(
                &mut self.rng,
                &["식대", "교통비", "가족수당", "직책수당", "자격수당"],
            )
            .copied()
            .unwrap_or("식대")
            .to_string(),
            "consent_status" => if a1 == "family_cert" {
                "제출"
            } else {
                "구비"
            }
            .to_string(),
            "other_note" => pick(
                &mut self.rng,
                &["격주 토요일", "명절 연휴", "회사 지정일", "기숙사 별도"],
            )
            .copied()
            .unwrap_or_default()
            .to_string(),
            other => return Err(SynthError::Assemble(format!("생성기 `{other}` 미구현"))),
        })
    }

    // ───────── 회사 추가 항목·하드네거티브용 값 ─────────

    pub fn styled(&mut self, n: &NumberText, maskable: bool) -> Result<String, SynthError> {
        let style = self.number_style(maskable)?;
        Ok(n.render(style))
    }

    pub fn account(&mut self) -> Result<Account, SynthError> {
        account(&mut self.rng, &self.cfg.data.banks)
            .ok_or_else(|| SynthError::Assemble("은행 자료가 비어 있음".into()))
    }

    pub fn email(&mut self) -> String {
        email(&mut self.rng)
    }

    pub fn emp_id(&mut self) -> String {
        format!("{}-{}", self.contract.year, digits(&mut self.rng, 4, 0..=9))
    }

    pub fn korean_passport(&mut self) -> Result<String, SynthError> {
        let kind = if self
            .cfg
            .choose(&mut self.rng, "korean_passport_kind", ["old", "new"])?
            == 0
        {
            KoreanPassport::Old
        } else {
            KoreanPassport::New
        };
        Ok(korean_passport(&mut self.rng, kind))
    }

    /// 운전면허번호 (숫자 지역코드 / 지역명 표기를 비율로 섞음)
    pub fn driver_license(&mut self) -> Result<String, SynthError> {
        let issue_year = self.rng.random_range(2005..=2024u16);
        let n = driver_license(&mut self.rng, issue_year);
        let region_name =
            self.cfg
                .choose(&mut self.rng, "license_style", ["numeric", "region_name"])?
                == 1;
        Ok(license_text(&n, region_name, "-"))
    }

    pub fn rng(&mut self) -> &mut SynthRng {
        &mut self.rng
    }
}
