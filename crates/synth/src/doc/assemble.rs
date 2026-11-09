//! 템플릿 블록 → 채운 블록. 특약(일반·민감·하드네거티브), 회사 추가 항목 표, 서명 계획을 붙인다.

use pii_core::{AttributeKind, Label, MarkKind, Owner};
use rand::RngExt;

use super::values::{Ctx, comma};
use super::{
    DocRequest, Field, FieldKind, RBlock, RCell, RText, ResolvedDoc, SignMethod, SignPlan,
    SynthConfig, SynthError,
};
use crate::clauses::{DOC_SLOTS, HARD_NEGATIVE_SLOTS, render_fields, render_text};
use crate::fake::ids::{self, BizKind};
use crate::fake::phone;
use crate::fake::{NumberText, pick, seeded};
use crate::template::{Align, Block, EmployerKind, Layout, Template, placeholders};

/// 텍스트 조각을 만들며 필드 위치를 기록한다
#[derive(Default)]
struct Builder {
    text: String,
    chars: usize,
    fields: Vec<Field>,
}

impl Builder {
    fn fixed(&mut self, s: &str) {
        self.text.push_str(s);
        self.chars += s.chars().count();
    }

    fn field(&mut self, s: &str, kind: FieldKind, source: &str, hand: bool) {
        let start = self.chars;
        self.fixed(s);
        if !s.is_empty() {
            self.fields.push(Field {
                start,
                end: self.chars,
                kind,
                source: source.to_string(),
                hand,
            });
        }
    }

    fn done(self) -> RText {
        RText {
            text: self.text,
            fields: self.fields,
        }
    }
}

/// 손글씨 기입 여부 (손글씨 문서면 필드마다 문서의 손글씨 비율로)
fn hand(ctx: &mut Ctx) -> bool {
    match ctx.hand_share {
        Some(share) => ctx.rng().random_bool(share),
        None => false,
    }
}

/// 템플릿 문구의 `{slot}`을 채운다
fn fill(ctx: &mut Ctx, tpl: &Template, text: &str) -> Result<RText, SynthError> {
    let names = placeholders(text).map_err(SynthError::Assemble)?;
    let mut b = Builder::default();
    let mut rest = text;
    for name in names {
        let pat = format!("{{{name}}}");
        let pos = rest
            .find(&pat)
            .ok_or_else(|| SynthError::Assemble(format!("자리표시 `{pat}` 위치")))?;
        b.fixed(&rest[..pos]);
        let v = ctx.slot(tpl, name)?;
        let h = match v.kind {
            FieldKind::Check { .. } => ctx.hand_share.is_some(),
            _ => hand(ctx),
        };
        b.field(&v.text, v.kind, name, h);
        rest = &rest[pos + pat.len()..];
    }
    b.fixed(rest);
    Ok(b.done())
}

fn plain(s: &str) -> RText {
    RText {
        text: s.to_string(),
        fields: Vec::new(),
    }
}

/// 문서 하나를 조립한다 (같은 시드면 같은 문서)
pub fn generate(cfg: &SynthConfig, req: &DocRequest) -> Result<ResolvedDoc, SynthError> {
    let mut ctx = Ctx::new(cfg, seeded(req.seed), req.layout, req.forms.clone())?;
    if req.digital {
        // D0 디지털 문서는 인쇄체만 (처음부터 디지털로 작성된 문서 — 사용자 결정 2026-11-08)
        ctx.hand_share = None;
    }
    ctx.foreign_worker = req.extra.foreign_worker && !matches!(req.layout, Layout::D);
    ctx.force_license = req.extra.driver_license;
    let company_added = matches!(req.layout, Layout::B | Layout::C) || req.extra.company_extra;
    let mut blocks = Vec::new();
    // 레이아웃 C: 원본 앞면이 한 쪽을 꽉 채우므로 회사 추가 항목은 서명란 뒤 별지로 붙인다
    let mut attachment: Vec<RBlock> = Vec::new();
    for (fi, form) in req.forms.iter().enumerate() {
        let tpl = cfg.template(form)?;
        if fi > 0 {
            blocks.push(RBlock::PageBreak);
        }
        for block in &tpl.blocks {
            match block {
                Block::Title { text } => blocks.push(RBlock::Title(fill(&mut ctx, tpl, text)?)),
                Block::Paragraph {
                    text,
                    indent,
                    align,
                } => blocks.push(RBlock::Paragraph {
                    text: fill(&mut ctx, tpl, text)?,
                    indent: *indent,
                    align: *align,
                }),
                Block::Table { widths, rows } => {
                    let mut out = Vec::with_capacity(rows.len());
                    for row in rows {
                        let mut cells = Vec::with_capacity(row.len());
                        for cell in row {
                            let (rowspan, colspan) = match cell {
                                crate::template::Cell::Text(_) => (1, 1),
                                crate::template::Cell::Spanned {
                                    rowspan, colspan, ..
                                } => (*rowspan, *colspan),
                            };
                            cells.push(RCell {
                                text: fill(&mut ctx, tpl, cell.text())?,
                                rowspan,
                                colspan,
                            });
                        }
                        out.push(cells);
                    }
                    blocks.push(RBlock::Table {
                        widths: widths.clone(),
                        rows: out,
                    });
                }
                Block::Signature {
                    owner,
                    anchor,
                    lines,
                } => {
                    let mut filled = Vec::with_capacity(lines.len());
                    for l in lines {
                        filled.push(fill(&mut ctx, tpl, l)?);
                    }
                    let sign = sign_plan(&mut ctx, *owner)?;
                    blocks.push(RBlock::Signature {
                        owner: *owner,
                        anchor: anchor.clone(),
                        lines: filled,
                        sign,
                    });
                }
                Block::CompanyExtra => {
                    if company_added {
                        let extra = company_extra(&mut ctx)?;
                        if req.layout == Layout::C {
                            attachment = extra;
                        } else {
                            blocks.extend(extra);
                        }
                    }
                }
                Block::SpecialTerms => blocks.extend(special_terms(&mut ctx)?),
                Block::PageBreak => blocks.push(RBlock::PageBreak),
            }
        }
    }
    if !attachment.is_empty() {
        blocks.push(RBlock::PageBreak);
        blocks.extend(attachment);
    }
    Ok(ResolvedDoc {
        doc_id: req.doc_id.clone(),
        form: req.forms.join("+"),
        layout: req.layout,
        variant: if company_added {
            "company_added"
        } else {
            "standard"
        }
        .to_string(),
        employer_kind: match ctx.employer_kind {
            EmployerKind::Individual => "individual",
            EmployerKind::Corporation => "corporation",
        }
        .to_string(),
        industry: ctx.industry.clone(),
        handwriting_doc: ctx.hand_share.is_some(),
        seed: req.seed,
        blocks,
    })
}

/// 서명란 계획 (축 1 서명 방식 × 축 2 도장, 독립)
fn sign_plan(ctx: &mut Ctx, owner: Owner) -> Result<SignPlan, SynthError> {
    let methods = [
        SignMethod::NameSign,
        SignMethod::Scribble,
        SignMethod::PrintSign,
        SignMethod::Fingerprint,
        SignMethod::None,
    ];
    let method = methods[ctx.cfg.choose(
        ctx.rng(),
        "sign_method",
        ["name_sign", "scribble", "print_sign", "fingerprint", "none"],
    )?];
    let seal = match owner {
        Owner::Employer => match ctx.employer_kind {
            EmployerKind::Corporation => Some(MarkKind::CorporateSeal),
            EmployerKind::Individual => {
                (ctx.cfg
                    .choose(ctx.rng(), "employer_seal_individual", ["personal", "none"])?
                    == 0)
                    .then_some(MarkKind::PersonalSeal)
            }
        },
        _ => (ctx
            .cfg
            .choose(ctx.rng(), "worker_seal", ["personal", "none"])?
            == 0)
            .then_some(MarkKind::PersonalSeal),
    };
    Ok(SignPlan {
        method,
        seal,
        signer: ctx.person(owner)?.name,
        company: ctx.company()?.name,
    })
}

/// 2열 표 한 행: 항목명 | 값 (값은 필드 여러 개일 수 있음)
fn row(label: &str, value: RText) -> Vec<RCell> {
    vec![
        RCell {
            text: plain(label),
            rowspan: 1,
            colspan: 1,
        },
        RCell {
            text: value,
            rowspan: 1,
            colspan: 1,
        },
    ]
}

fn one_field(ctx: &mut Ctx, text: &str, kind: FieldKind, source: &str) -> RText {
    let mut b = Builder::default();
    let h = hand(ctx);
    b.field(text, kind, source, h);
    b.done()
}

fn label(l: Label, owner: Option<Owner>) -> FieldKind {
    FieldKind::Label { label: l, owner }
}

/// 회사 추가 항목 (레이아웃 B: 근로자 인적사항 표, C: 외국인 추가 정보 표)
fn company_extra(ctx: &mut Ctx) -> Result<Vec<RBlock>, SynthError> {
    let worker = ctx.person(Owner::Worker)?;
    let w = Some(Owner::Worker);
    let mut rows = Vec::new();
    let title;
    if let Some(f) = &worker.foreign {
        title = "추가 정보 (Additional Information)";
        let frn = ctx.styled(&f.frn, true)?;
        rows.push(row(
            "외국인등록번호 Alien Registration No.",
            one_field(ctx, &frn, label(Label::Frn, w), "extra:frn"),
        ));
        rows.push(row(
            "여권번호 Passport No.",
            one_field(
                ctx,
                &f.passport,
                label(Label::Passport, w),
                "extra:passport",
            ),
        ));
        let nationality = ctx
            .cfg
            .data
            .foreign
            .country
            .get(f.country)
            .map(|c| c.nationality.clone())
            .unwrap_or_default();
        rows.push(row(
            "국적 Nationality",
            one_field(
                ctx,
                &nationality,
                FieldKind::Attribute(AttributeKind::Nationality),
                "extra:nationality",
            ),
        ));
        let phone = ctx.styled(&worker.phone, false)?;
        rows.push(row(
            "국내 연락처 Phone in Korea",
            one_field(ctx, &phone, label(Label::Phone, w), "extra:phone"),
        ));
        let style = ctx.address_style()?;
        let addr = worker.address.render(style);
        rows.push(row(
            "국내 주소 Address in Korea",
            one_field(ctx, &addr, label(Label::Address, w), "extra:address"),
        ));
    } else {
        title = "근로자 인적사항";
        let rrn = ctx.styled(&worker.rrn, true)?;
        rows.push(row(
            "주민등록번호",
            one_field(ctx, &rrn, label(Label::Rrn, w), "extra:rrn"),
        ));
        let birth = format!(
            "{}.{:02}.{:02}",
            worker.birth.year, worker.birth.month, worker.birth.day
        );
        rows.push(row(
            "생년월일",
            one_field(ctx, &birth, label(Label::BirthDate, w), "extra:birth"),
        ));
        let email = ctx.email();
        rows.push(row(
            "이메일",
            one_field(ctx, &email, label(Label::Email, w), "extra:email"),
        ));
        // 비상연락처: 이름 (관계) 전화 — 제3자라 owner 없음, 관계는 참고 항목
        let gender = if ctx.rng().random_bool(0.5) {
            crate::fake::data::Gender::Male
        } else {
            crate::fake::data::Gender::Female
        };
        let cfg = ctx.cfg;
        let em_name = crate::fake::person::korean_name(
            ctx.rng(),
            &cfg.data.names,
            crate::fake::data::Generation::G7080,
            gender,
        )
        .full();
        let relation = pick(ctx.rng(), &["배우자", "부", "모", "형제", "자매"])
            .copied()
            .unwrap_or("배우자");
        let em_phone_n = phone::mobile(ctx.rng());
        let em_phone = ctx.styled(&em_phone_n, false)?;
        let mut b = Builder::default();
        let h = hand(ctx);
        b.field(
            &em_name,
            label(Label::Name, None),
            "extra:emergency_name",
            h,
        );
        b.fixed(" (");
        b.field(
            relation,
            FieldKind::Attribute(AttributeKind::EmergencyRelation),
            "extra:emergency_relation",
            h,
        );
        b.fixed(") ");
        b.field(
            &em_phone,
            label(Label::Phone, None),
            "extra:emergency_phone",
            h,
        );
        rows.push(row("비상연락처", b.done()));
        let emp = ctx.emp_id();
        rows.push(row(
            "사번",
            one_field(ctx, &emp, label(Label::EmpId, w), "extra:emp_id"),
        ));
        // 운전·배송이 많은 업종에서만 운전면허
        if ctx.force_license || matches!(ctx.industry.as_str(), "물류" | "건설") {
            let lic = ctx.driver_license()?;
            rows.push(row(
                "운전면허번호",
                one_field(
                    ctx,
                    &lic,
                    label(Label::DriverLicense, w),
                    "extra:driver_license",
                ),
            ));
        }
    }
    // 급여 계좌 (공통): 은행명 계좌번호 / 예금주
    let acc = ctx.account()?;
    let number = ctx.styled(&acc.number, false)?;
    let mut b = Builder::default();
    let h = hand(ctx);
    b.fixed(&format!("{} ", acc.bank));
    b.field(&number, label(Label::Account, w), "extra:account", h);
    b.fixed(" / 예금주 ");
    b.field(
        &worker.name,
        label(Label::Name, w),
        "extra:account_holder",
        h,
    );
    rows.push(row("급여 계좌", b.done()));

    Ok(vec![
        RBlock::Paragraph {
            text: plain(title),
            indent: 0,
            align: Align::Left,
        },
        RBlock::Table {
            widths: vec![0.3, 0.7],
            rows,
        },
    ])
}

/// 특약사항: 일반 1~3개 + 민감 문장(비율) + 하드네거티브 1~2개, 순서 섞음
fn special_terms(ctx: &mut Ctx) -> Result<Vec<RBlock>, SynthError> {
    let foreign = ctx.layout == Layout::C;
    let worker = ctx.person(Owner::Worker)?;
    // 외국인 이름(영문)에는 조사를 붙일 수 없으므로 이름이 들어가는 문장은 뺀다 (국내 양식의 외국인 근로자 포함)
    let foreign_name = worker.foreign.is_some();
    let company = ctx.company()?;
    let mut items: Vec<RText> = Vec::new();

    let doc_lookup = |name: &str| -> Option<String> {
        match name {
            "WORKER_NAME" => Some(worker.name.clone()),
            "COMPANY" => Some(company.name.clone()),
            _ => None,
        }
    };
    let doc_kind = |name: &str| -> FieldKind {
        match name {
            "WORKER_NAME" => label(Label::Name, Some(Owner::Worker)),
            _ => FieldKind::Plain,
        }
    };
    let to_rtext =
        |r: crate::clauses::Rendered, kind_of: &dyn Fn(&str) -> FieldKind, source: &str| RText {
            text: r.text,
            fields: r
                .fields
                .into_iter()
                .map(|(name, s, e)| Field {
                    start: s.0,
                    end: e.0,
                    kind: kind_of(&name),
                    source: format!("{source}:{name}"),
                    hand: false,
                })
                .collect(),
        };

    // 일반 특약
    let n_general = ctx.rng().random_range(1..=3usize);
    for _ in 0..n_general {
        let cands: Vec<&crate::clauses::GeneralClause> = ctx
            .cfg
            .general
            .iter()
            .filter(|g| !(foreign_name && g.text.contains("{WORKER_NAME}")))
            .collect();
        if let Some(g) = pick(ctx.rng(), &cands) {
            let r = render_fields(&g.text, doc_lookup)?;
            items.push(to_rtext(r, &doc_kind, &g.id));
        }
    }

    // 민감 문장
    if ctx
        .cfg
        .choose(ctx.rng(), "sensitive_clause", ["yes", "no"])?
        == 0
    {
        let industry = ctx.industry.clone();
        let cands: Vec<&crate::clauses::SensitiveClause> = ctx
            .cfg
            .sensitive
            .iter()
            .filter(|c| !(foreign_name && c.template.contains("{WORKER_NAME}")))
            .filter(|c| {
                ctx.cfg
                    .values
                    .iter()
                    .any(|v| c.accepts(v) && v.fits_industry(&industry))
            })
            .collect();
        if let Some(c) = pick(ctx.rng(), &cands) {
            let vals: Vec<&crate::clauses::SensitiveValue> = ctx
                .cfg
                .values
                .iter()
                .filter(|v| c.accepts(v) && v.fits_industry(&industry))
                .collect();
            if let Some(v) = pick(ctx.rng(), &vals) {
                let value = render_text(&v.value, doc_lookup)?;
                let label_name = match serde_json::to_value(c.label) {
                    Ok(serde_json::Value::String(s)) => s,
                    _ => String::new(),
                };
                let r = render_fields(&c.template, |n| {
                    if n == label_name {
                        Some(value.clone())
                    } else {
                        doc_lookup(n)
                    }
                })?;
                let sensitive_label = c.label;
                let kind_of = |n: &str| {
                    if n == label_name {
                        label(sensitive_label, Some(Owner::Worker))
                    } else {
                        doc_kind(n)
                    }
                };
                items.push(to_rtext(r, &kind_of, &format!("{}+{}", c.id, v.id)));
            }
        }
    }

    // 하드네거티브
    let n_neg = ctx.rng().random_range(1..=2usize);
    for _ in 0..n_neg {
        let cfg = ctx.cfg;
        let Some(n) = pick(ctx.rng(), &cfg.negatives).cloned() else {
            break;
        };
        let names = placeholders(&n.template).map_err(SynthError::Assemble)?;
        let mut values = std::collections::BTreeMap::new();
        for name in names {
            let doc_slot = DOC_SLOTS.iter().any(|(d, _)| *d == name);
            if !doc_slot && !values.contains_key(name) {
                values.insert(name.to_string(), negative_value(ctx, name)?);
            }
        }
        let r = render_fields(&n.template, |name| {
            values.get(name).cloned().or_else(|| doc_lookup(name))
        })?;
        let kind_of = |name: &str| {
            if HARD_NEGATIVE_SLOTS.iter().any(|(s, _)| *s == name) {
                FieldKind::Negative
            } else {
                doc_kind(name)
            }
        };
        let mut t = to_rtext(r, &kind_of, &n.id);
        if n.kind == crate::clauses::HardNegativeKind::LawArticle {
            for (s, e) in law_article_ranges(&t.text) {
                t.fields.push(Field {
                    start: s,
                    end: e,
                    kind: FieldKind::Negative,
                    source: format!("{}:law_article", n.id),
                    hand: false,
                });
            }
            t.fields.sort_by_key(|f| f.start);
        }
        items.push(t);
    }

    // 순서 섞기 (Fisher–Yates)
    for i in (1..items.len()).rev() {
        let j = ctx.rng().random_range(0..=i);
        items.swap(i, j);
    }
    let heading = if foreign {
        "특약사항 (Special terms)"
    } else {
        "특약사항"
    };
    let mut out = vec![RBlock::Paragraph {
        text: plain(heading),
        indent: 0,
        align: Align::Left,
    }];
    for (i, mut t) in items.into_iter().enumerate() {
        let prefix = format!("{}. ", i + 1);
        let shift = prefix.chars().count();
        for f in &mut t.fields {
            f.start += shift;
            f.end += shift;
        }
        t.text = format!("{prefix}{}", t.text);
        out.push(RBlock::Paragraph {
            text: t,
            indent: 1,
            align: Align::Left,
        });
    }
    Ok(out)
}

/// 법 조항 번호 `제17조`, `제18조의4` 위치 (문자 오프셋)
fn law_article_ranges(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '제' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < chars.len() && chars[j] == '조' {
                let mut end = j + 1;
                // 제18조의4
                if end + 1 < chars.len() && chars[end] == '의' && chars[end + 1].is_ascii_digit() {
                    end += 1;
                    while end < chars.len() && chars[end].is_ascii_digit() {
                        end += 1;
                    }
                }
                out.push((i, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// 하드네거티브 자리표시 값
fn negative_value(ctx: &mut Ctx, name: &str) -> Result<String, SynthError> {
    let kind = match ctx.employer_kind {
        EmployerKind::Individual => BizKind::Individual,
        EmployerKind::Corporation => BizKind::Corporation,
    };
    let n: Option<NumberText> = match name {
        "COMPANY_PHONE" => Some(ctx.company()?.phone),
        "NATIONAL_REP" => Some(phone::national_rep(ctx.rng())),
        "INTERNET_PHONE" => Some(phone::internet(ctx.rng())),
        "FAX" => Some(phone::landline(ctx.rng())),
        "SPECIAL_NUMBER" => Some(phone::special_number(ctx.rng())),
        "PUBLIC_REP" => {
            let p = ctx.phones.clone();
            Some(p.public_rep(ctx.rng()))
        }
        "CORP_NO" => Some(ids::corp_reg_no(ctx.rng())),
        "CORP_BIZ_NO" => Some(ids::bizno(ctx.rng(), BizKind::Corporation)),
        "DOC_NO13" => Some(ids::doc_no13(ctx.rng())),
        "RRN_INVALID" => Some(ids::rrn_invalid_date(ctx.rng())),
        "WORKPLACE_MGMT_NO" => Some(ids::workplace_mgmt_no(ctx.rng(), kind)),
        "ZIPCODE" => Some(ids::zipcode(ctx.rng())),
        "COMPANY_ACCOUNT" => Some(ctx.account()?.number),
        _ => None,
    };
    if let Some(n) = n {
        return ctx.styled(&n, false);
    }
    Ok(match name {
        "DOC_NO" => {
            let year = ctx.contract.year;
            ids::doc_no(ctx.rng(), year).groups.join("-")
        }
        "EXTENSION" => ctx.rng().random_range(100..=9999u32).to_string(),
        "AMOUNT" => comma(u64::from(ctx.rng().random_range(10..=50u32)) * 10_000),
        "MIN_WAGE" => comma(*pick(ctx.rng(), &[9_860u64, 10_030, 10_320]).unwrap_or(&10_320)),
        "DATE" => {
            let back = ctx.rng().random_range(30..=300);
            let d = ctx.contract.add_days(-back);
            format!("{}. {}. {}.", d.year, d.month, d.day)
        }
        other => {
            return Err(SynthError::Assemble(format!(
                "하드네거티브 자리표시 `{other}` 미구현"
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_law_articles() {
        let t = "근로기준법 제17조 및 제18조의4제1항, 제n조, 제조";
        let got: Vec<String> = law_article_ranges(t)
            .into_iter()
            .map(|(s, e)| t.chars().skip(s).take(e - s).collect())
            .collect();
        assert_eq!(got, vec!["제17조", "제18조의4"]);
    }
}
