//! 5단계 데이터 세트: 이미지 세트(개발용·평가용)와 텍스트 세트의 문서 계획·생성
//!
//! - 이미지 세트: 레이아웃(`layout_mix`)·난이도(`difficulty_mix`)를 **건수로 정확히** 나눈 뒤 시드로 섞어 짝짓는다
//!   (작은 세트에서도 배분이 흔들리지 않게). D0는 텍스트 레이어 PDF, D1~D4는 열화 쪽 이미지
//! - 텍스트 세트: 레이아웃은 `layout_mix` 비율로 뽑고, 정형 라벨(`TEXT_TARGET_LABELS`)이 진도표보다 뒤처지면
//!   그 라벨이 나오는 회사 추가 항목을 붙인다 (PLAN.md 1-2: 정형 라벨별 span 최소 500개)
//! - 문서 시드는 세트 시드로 만든 난수열에서 뽑는다 → 같은 세트 시드면 같은 데이터

use std::collections::BTreeMap;

use pii_core::{DocLabel, Label};
use rand::RngExt;

use crate::degrade::{Difficulty, degrade_page};
use crate::doc::{
    DocRequest, ExtraOptions, SignMethod, SynthConfig, SynthError, generate, text_label,
};
use crate::fake::{SynthRng, seeded};
use crate::render::{Fonts, layout, pdf::write_pdf, render};
use crate::template::Layout;

/// 텍스트 세트 목표: 정형 라벨별 정답 span 최소 개수 (PLAN.md 1-2)
pub const TEXT_TARGET: u32 = 500;
pub const TEXT_TARGET_LABELS: [Label; 7] = [
    Label::Rrn,
    Label::Frn,
    Label::Passport,
    Label::DriverLicense,
    Label::Phone,
    Label::Email,
    Label::Account,
];

/// `layout_mix` 키 → (레이아웃, 양식)
const LAYOUT_KEYS: [(&str, Layout, &[&str]); 9] = [
    ("A_indefinite", Layout::A, &["standard_indefinite"]),
    ("A_fixed_term", Layout::A, &["standard_fixed_term"]),
    ("B_indefinite", Layout::B, &["standard_indefinite"]),
    ("B_fixed_term", Layout::B, &["standard_fixed_term"]),
    ("C_general", Layout::C, &["foreign_worker"]),
    ("C_agri", Layout::C, &["foreign_worker_agri"]),
    ("D_minor", Layout::D, &["minor", "guardian_consent"]),
    ("E_part_time", Layout::E, &["part_time"]),
    ("F_construction", Layout::F, &["construction_daily"]),
];

/// 이미지 세트 문서 하나의 계획
#[derive(Debug, Clone)]
pub struct DocPlan {
    pub doc_id: String,
    pub layout: Layout,
    pub forms: Vec<String>,
    pub difficulty: Difficulty,
    pub seed: u64,
}

/// 만든 파일 하나 (이름, 바이트)
pub struct OutFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// 쪽 파일 정보 (매니페스트용)
#[derive(Debug, Clone, serde::Serialize)]
pub struct PageInfo {
    pub file: String,
    pub width: u32,
    pub height: u32,
}

/// 서명란 하나의 실제 계획 (6단계 리포트: 축1·축2 분포)
#[derive(Debug, Clone, serde::Serialize)]
pub struct SignInfo {
    pub owner: String,
    pub method: String,
    pub seal: String,
}

/// 매니페스트 한 줄
#[derive(Debug, Clone, serde::Serialize)]
pub struct ManifestEntry {
    pub doc_id: String,
    pub layout: String,
    pub form: String,
    pub difficulty: String,
    pub seed: u64,
    pub handwriting_doc: bool,
    pub label_file: String,
    /// D0: PDF 파일 (라벨 박스는 200dpi 픽셀, PDF pt = 픽셀 × 72/200)
    pub pdf: Option<String>,
    /// D1~D4: 쪽 이미지
    pub pages: Vec<PageInfo>,
    pub signatures: Vec<SignInfo>,
}

/// 만든 문서 하나
pub struct BuiltDoc {
    pub label: DocLabel,
    pub files: Vec<OutFile>,
    pub manifest: ManifestEntry,
}

fn layout_name(l: Layout) -> &'static str {
    match l {
        Layout::A => "A",
        Layout::B => "B",
        Layout::C => "C",
        Layout::D => "D",
        Layout::E => "E",
        Layout::F => "F",
    }
}

/// 비율 표를 n건으로 나눈다 (최대 나머지 방식, 합 = n)
fn allocate(
    cfg: &SynthConfig,
    table: &str,
    keys: &[&str],
    n: usize,
) -> Result<Vec<usize>, SynthError> {
    let t = cfg
        .ratios
        .ratio
        .get(table)
        .ok_or_else(|| SynthError::Assemble(format!("비율 표 `{table}` 없음")))?;
    let mut w = Vec::with_capacity(keys.len());
    for k in keys {
        w.push(
            *t.weights
                .get(*k)
                .ok_or_else(|| SynthError::Assemble(format!("비율 `{table}.{k}` 없음")))?,
        );
    }
    let exact: Vec<f64> = w.iter().map(|x| x * n as f64).collect();
    let mut counts: Vec<usize> = exact.iter().map(|x| x.floor() as usize).collect();
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by(|&a, &b| {
        let ra = exact[a] - exact[a].floor();
        let rb = exact[b] - exact[b].floor();
        rb.total_cmp(&ra).then(a.cmp(&b))
    });
    let mut left = n - counts.iter().sum::<usize>();
    for i in order {
        if left == 0 {
            break;
        }
        counts[i] += 1;
        left -= 1;
    }
    Ok(counts)
}

fn shuffle<T>(rng: &mut SynthRng, v: &mut [T]) {
    for i in (1..v.len()).rev() {
        let j = rng.random_range(0..=i);
        v.swap(i, j);
    }
}

/// 이미지 세트 계획: 레이아웃·난이도를 건수로 나누고 섞어 짝짓는다
pub fn plan_image_set(cfg: &SynthConfig, n: usize, seed: u64) -> Result<Vec<DocPlan>, SynthError> {
    let mut rng = seeded(seed);
    let keys: Vec<&str> = LAYOUT_KEYS.iter().map(|k| k.0).collect();
    let lc = allocate(cfg, "layout_mix", &keys, n)?;
    let mut layouts: Vec<usize> = lc
        .iter()
        .enumerate()
        .flat_map(|(i, c)| std::iter::repeat_n(i, *c))
        .collect();
    let dkeys = ["D0", "D1", "D2", "D3", "D4"];
    let dc = allocate(cfg, "difficulty_mix", &dkeys, n)?;
    let mut diffs: Vec<Difficulty> = dc
        .iter()
        .enumerate()
        .flat_map(|(i, c)| std::iter::repeat_n(Difficulty::ALL[i], *c))
        .collect();
    shuffle(&mut rng, &mut layouts);
    shuffle(&mut rng, &mut diffs);
    // 문서 번호 순서는 난이도·레이아웃 순으로 정렬 (사람이 폴더를 볼 때 찾기 쉽게)
    let mut pairs: Vec<(Difficulty, usize, u64)> = diffs
        .into_iter()
        .zip(layouts)
        .map(|(d, l)| (d, l, rng.random::<u64>()))
        .collect();
    pairs.sort_by_key(|(d, l, _)| (*d, *l));
    Ok(pairs
        .into_iter()
        .enumerate()
        .map(|(i, (difficulty, li, doc_seed))| {
            let (_, layout, forms) = LAYOUT_KEYS[li];
            DocPlan {
                doc_id: format!("s{seed}_{:04}", i + 1),
                layout,
                forms: forms.iter().map(|s| s.to_string()).collect(),
                difficulty,
                seed: doc_seed,
            }
        })
        .collect())
}

fn sign_infos(doc: &crate::doc::ResolvedDoc) -> Vec<SignInfo> {
    doc.blocks
        .iter()
        .filter_map(|b| match b {
            crate::doc::RBlock::Signature { owner, sign, .. } => Some(SignInfo {
                owner: format!("{owner:?}").to_lowercase(),
                method: match sign.method {
                    SignMethod::NameSign => "name_sign",
                    SignMethod::Scribble => "scribble",
                    SignMethod::PrintSign => "print_sign",
                    SignMethod::Fingerprint => "fingerprint",
                    SignMethod::None => "none",
                }
                .to_string(),
                seal: match sign.seal {
                    Some(pii_core::MarkKind::PersonalSeal) => "personal",
                    Some(pii_core::MarkKind::CorporateSeal) => "corporate",
                    Some(_) | None => "none",
                }
                .to_string(),
            }),
            _ => None,
        })
        .collect()
}

/// 이미지 세트 문서 하나를 만든다 (D0 → PDF, D1~D4 → 열화 쪽 이미지)
pub fn build_image_doc(
    cfg: &SynthConfig,
    fonts: &Fonts,
    plan: &DocPlan,
) -> Result<BuiltDoc, SynthError> {
    let req = DocRequest {
        doc_id: plan.doc_id.clone(),
        layout: plan.layout,
        forms: plan.forms.clone(),
        seed: plan.seed,
        digital: plan.difficulty == Difficulty::D0,
        extra: ExtraOptions::default(),
    };
    let doc = generate(cfg, &req)?;
    let id = &plan.doc_id;
    let mut files = Vec::new();
    let mut pages = Vec::new();
    let mut pdf = None;
    let mut label = if plan.difficulty == Difficulty::D0 {
        let laid = layout(&doc, fonts);
        let bytes = write_pdf(&laid, fonts).map_err(|e| SynthError::Assemble(e.to_string()))?;
        let name = format!("{id}.pdf");
        files.push(OutFile {
            name: name.clone(),
            bytes,
        });
        pdf = Some(name);
        laid.label
    } else {
        let r = render(&doc, fonts).map_err(|e| SynthError::Assemble(e.to_string()))?;
        let mut label = r.label;
        for (i, page) in r.pages.iter().enumerate() {
            let mut rng = seeded(plan.seed ^ 0xD15C_0000 ^ (i as u64) << 8);
            let img = degrade_page(
                cfg,
                fonts,
                page,
                i as u32,
                &mut label,
                plan.difficulty,
                &mut rng,
            )?;
            let (width, height) = img.size();
            let name = format!("{id}_p{}.{}", i + 1, img.ext());
            files.push(OutFile {
                name: name.clone(),
                bytes: img.encode()?,
            });
            pages.push(PageInfo {
                file: name,
                width,
                height,
            });
        }
        label
    };
    label.difficulty = plan.difficulty.name().to_string();
    let label_file = format!("{id}.json");
    let manifest = ManifestEntry {
        doc_id: id.clone(),
        layout: layout_name(plan.layout).to_string(),
        form: label.form.clone(),
        difficulty: label.difficulty.clone(),
        seed: plan.seed,
        handwriting_doc: label.handwriting_doc,
        label_file,
        pdf,
        pages,
        signatures: sign_infos(&doc),
    };
    Ok(BuiltDoc {
        label,
        files,
        manifest,
    })
}

/// 텍스트 세트 결과
pub struct TextSet {
    pub labels: Vec<DocLabel>,
    pub signatures: Vec<Vec<SignInfo>>,
    /// 문서별 추가 항목 (리포트용)
    pub extras: Vec<ExtraOptions>,
}

/// 텍스트 세트: 정형 라벨이 진도표(목표 × 진행률 × 1.1)보다 뒤처지면 그 라벨이 나오는 추가 항목을 붙인다
pub fn generate_text_set(cfg: &SynthConfig, n: usize, seed: u64) -> Result<TextSet, SynthError> {
    let mut rng = seeded(seed);
    let keys: [&str; 9] = [
        "A_indefinite",
        "A_fixed_term",
        "B_indefinite",
        "B_fixed_term",
        "C_general",
        "C_agri",
        "D_minor",
        "E_part_time",
        "F_construction",
    ];
    let mut counts: BTreeMap<Label, u32> = BTreeMap::new();
    let mut out = TextSet {
        labels: Vec::with_capacity(n),
        signatures: Vec::with_capacity(n),
        extras: Vec::with_capacity(n),
    };
    for i in 0..n {
        let li = cfg.choose(&mut rng, "layout_mix", keys)?;
        let (_, layout, forms) = LAYOUT_KEYS[li];
        let schedule = f64::from(TEXT_TARGET) * (i + 1) as f64 / n as f64 * 1.1;
        // 진도 비율 (1 미만이면 뒤처짐). 한 문서는 외국인·내국인 중 하나라 두 묶음이 경쟁 → 더 뒤처진 쪽을 채운다
        let progress = |l: Label| f64::from(counts.get(&l).copied().unwrap_or(0)) / schedule;
        let foreign_need = progress(Label::Frn).min(progress(Label::Passport));
        let korean_need = [
            Label::Rrn,
            Label::Email,
            Label::DriverLicense,
            Label::Account,
        ]
        .into_iter()
        .map(progress)
        .fold(f64::MAX, f64::min);
        let foreign_ok = matches!(layout, Layout::A | Layout::B | Layout::E | Layout::F);
        let korean_ok = layout != Layout::C;
        let mut extra = ExtraOptions::default();
        let want_foreign =
            foreign_ok && foreign_need < 1.0 && (!korean_ok || foreign_need <= korean_need);
        let want_korean = korean_ok && korean_need < 1.0 && !want_foreign;
        if want_foreign {
            extra.company_extra = true;
            extra.foreign_worker = true;
        } else if want_korean {
            extra.company_extra = true;
            extra.driver_license = progress(Label::DriverLicense) < 1.0;
        } else if cfg.choose(&mut rng, "text_company_extra", ["yes", "no"])? == 0 {
            extra.company_extra = true;
            extra.foreign_worker =
                foreign_ok && cfg.choose(&mut rng, "text_foreign_worker", ["yes", "no"])? == 0;
        }
        let req = DocRequest {
            doc_id: format!("s{seed}_{:05}", i + 1),
            layout,
            forms: forms.iter().map(|s| s.to_string()).collect(),
            seed: rng.random::<u64>(),
            digital: false,
            extra,
        };
        let doc = generate(cfg, &req)?;
        let mut label = text_label(&doc);
        label.difficulty = "text".to_string();
        for s in &label.spans {
            *counts.entry(s.label).or_default() += 1;
        }
        out.signatures.push(sign_infos(&doc));
        out.labels.push(label);
        out.extras.push(extra);
    }
    Ok(out)
}

/// 처음 보는 서식 시험용 텍스트 세트: 불러온 템플릿을 차례로 돌려 같은 건수씩 만든다 (추가 항목 없음)
pub fn generate_holdout_text_set(
    cfg: &SynthConfig,
    n: usize,
    seed: u64,
) -> Result<TextSet, SynthError> {
    let mut rng = seeded(seed);
    let mut out = TextSet {
        labels: Vec::with_capacity(n),
        signatures: Vec::with_capacity(n),
        extras: Vec::with_capacity(n),
    };
    for i in 0..n {
        let t = cfg
            .templates
            .get(i % cfg.templates.len().max(1))
            .ok_or_else(|| SynthError::Assemble("템플릿이 없음".to_string()))?;
        let layout = *t
            .meta
            .layouts
            .first()
            .ok_or_else(|| SynthError::Assemble(format!("`{}` 레이아웃 없음", t.meta.id)))?;
        let req = DocRequest {
            doc_id: format!("s{seed}_{:05}", i + 1),
            layout,
            forms: vec![t.meta.id.clone()],
            seed: rng.random::<u64>(),
            digital: false,
            extra: ExtraOptions::default(),
        };
        let doc = generate(cfg, &req)?;
        let mut label = text_label(&doc);
        label.difficulty = "text".to_string();
        out.signatures.push(sign_infos(&doc));
        out.labels.push(label);
        out.extras.push(ExtraOptions::default());
    }
    Ok(out)
}
