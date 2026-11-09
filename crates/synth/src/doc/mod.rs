//! 문서 조립 (PLAN.md 4단계): 템플릿 + 값 → 채운 문서 모델 + 정답 라벨.
//!
//! - [`generate`]: 문서 하나를 조립해 [`ResolvedDoc`](블록·필드 위치)을 만든다
//! - [`text_label`]: 텍스트 전용 모드 라벨 (span·참고 항목·네거티브, 박스 없음)
//! - 렌더링(이미지)은 [`ResolvedDoc`]을 [`walk_text`]와 같은 순서로 그려 글자 박스의 문자 오프셋을 맞춘다
//!
//! 비율은 모두 `assets/synth/ratios.toml`에서 꺼낸다 (CLAUDE.md).

pub mod assemble;
pub mod values;

use std::path::Path;

use pii_core::{
    AttributeKind, AttributeSpan, CharOffset, DocLabel, Label, MarkKind, NegativeSpan, Owner, Span,
};
use rand::Rng;

use crate::clauses::{
    ClauseError, GeneralClause, HardNegativeClause, SensitiveClause, SensitiveValue, load_jsonl,
};
use crate::fake::data::{DataError, FakeData};
use crate::fake::{FakeError, pick_weighted, ratio_weights};
use crate::ratios::{RatioConfig, RatioError};
use crate::template::{Align, Layout, Template, TemplateError};

pub use assemble::generate;

#[derive(Debug, thiserror::Error)]
pub enum SynthError {
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error(transparent)]
    Ratio(#[from] RatioError),
    #[error(transparent)]
    Clause(#[from] ClauseError),
    #[error(transparent)]
    Data(#[from] DataError),
    #[error(transparent)]
    Fake(#[from] FakeError),
    #[error("문서 조립 실패: {0}")]
    Assemble(String),
}

/// 합성에 필요한 설정·자료 전체 (한 번 읽어 여러 문서에 씀)
#[derive(Debug, Clone)]
pub struct SynthConfig {
    pub ratios: RatioConfig,
    pub data: FakeData,
    pub templates: Vec<Template>,
    pub general: Vec<GeneralClause>,
    pub sensitive: Vec<SensitiveClause>,
    pub values: Vec<SensitiveValue>,
    pub negatives: Vec<HardNegativeClause>,
}

impl SynthConfig {
    /// 작업 폴더(`assets/` 상위)에서 읽는다
    pub fn load(root: &Path) -> Result<Self, SynthError> {
        Self::load_with_templates(root, &root.join("assets/templates"))
    }

    /// 템플릿 폴더만 바꿔서 읽는다 (처음 보는 서식 시험)
    pub fn load_with_templates(root: &Path, templates: &Path) -> Result<Self, SynthError> {
        let assets = root.join("assets");
        let clauses = assets.join("clauses");
        Ok(SynthConfig {
            ratios: RatioConfig::load(&assets.join("synth/ratios.toml"))?,
            data: FakeData::load(&assets.join("synth"))?,
            templates: Template::load_dir(templates)?,
            general: load_jsonl(&clauses.join("general.jsonl"))?,
            sensitive: load_jsonl(&clauses.join("sensitive.jsonl"))?,
            values: load_jsonl(&clauses.join("sensitive_values.jsonl"))?,
            negatives: load_jsonl(&clauses.join("hard_negatives.jsonl"))?,
        })
    }

    pub fn template(&self, id: &str) -> Result<&Template, SynthError> {
        self.templates
            .iter()
            .find(|t| t.meta.id == id)
            .ok_or_else(|| SynthError::Assemble(format!("템플릿 `{id}` 없음")))
    }

    /// 비율 표에서 키 순서대로 하나를 고른다 (반환: 키 인덱스)
    pub(crate) fn choose<R: Rng + ?Sized, const N: usize>(
        &self,
        rng: &mut R,
        table: &str,
        keys: [&str; N],
    ) -> Result<usize, SynthError> {
        let w = ratio_weights(&self.ratios, table, keys)?;
        let i = pick_weighted(rng, &w);
        self.ratios.tally.record(table, keys[i]);
        Ok(i)
    }
}

/// 문서 요청
#[derive(Debug, Clone)]
pub struct DocRequest {
    pub doc_id: String,
    pub layout: Layout,
    /// 템플릿 id 순서대로 (레이아웃 D는 `["minor", "guardian_consent"]`)
    pub forms: Vec<String>,
    pub seed: u64,
    /// D0 디지털 문서 (처음부터 디지털로 작성 → 손글씨 기입 없음, 서명·도장은 그대로)
    pub digital: bool,
    /// 텍스트 1,000건용 추가 항목 (정형 라벨 목표 채우기 — PLAN.md 5단계)
    pub extra: ExtraOptions,
}

/// 레이아웃 기본 구성 밖의 추가 항목. 이미지 세트는 모두 꺼 둔다 (레이아웃 B·C는 원래 회사 추가 항목이 있음)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtraOptions {
    /// 레이아웃 A·D·E·F에도 회사 추가 항목(근로자 인적사항·급여 계좌) 블록을 붙인다
    pub company_extra: bool,
    /// 국내 양식(A·B·E·F)을 쓰는 외국인 근로자 → 추가 항목에 외국인등록번호·여권번호
    pub foreign_worker: bool,
    /// 추가 항목에 운전면허번호 (원래는 물류·건설 업종만)
    pub driver_license: bool,
}

/// 필드 종류
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// 정답 PII
    Label { label: Label, owner: Option<Owner> },
    /// 참고 항목 (평가에서 중립)
    Attribute(AttributeKind),
    /// 하드네거티브
    Negative,
    /// 채운 값이지만 라벨 없음 (근로조건 등)
    Plain,
    /// 체크 칸
    Check { checked: bool },
}

/// 텍스트 조각 안의 채운 값 위치 (조각 안 문자 오프셋)
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub start: usize,
    pub end: usize,
    pub kind: FieldKind,
    /// 슬롯·문장 풀 id (추적용)
    pub source: String,
    /// 손글씨로 기입
    pub hand: bool,
}

/// 채운 텍스트 조각
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RText {
    pub text: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RCell {
    pub text: RText,
    pub rowspan: u8,
    pub colspan: u8,
}

/// 서명 방식 (PLAN.md 4단계 축 1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignMethod {
    NameSign,
    Scribble,
    PrintSign,
    Fingerprint,
    None,
}

/// 서명란에 그릴 것 (축 1 + 축 2)
#[derive(Debug, Clone, PartialEq)]
pub struct SignPlan {
    pub method: SignMethod,
    /// `PersonalSeal` 또는 `CorporateSeal`
    pub seal: Option<MarkKind>,
    /// 서명·도장에 쓰는 이름
    pub signer: String,
    /// 법인 인감용 회사 이름
    pub company: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RBlock {
    Title(RText),
    Paragraph {
        text: RText,
        indent: u8,
        align: Align,
    },
    Table {
        widths: Vec<f32>,
        rows: Vec<Vec<RCell>>,
    },
    Signature {
        owner: Owner,
        anchor: String,
        lines: Vec<RText>,
        sign: SignPlan,
    },
    PageBreak,
}

/// 조립된 문서
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDoc {
    pub doc_id: String,
    pub form: String,
    pub layout: Layout,
    pub variant: String,
    pub employer_kind: String,
    pub industry: String,
    pub handwriting_doc: bool,
    pub seed: u64,
    pub blocks: Vec<RBlock>,
}

/// 문서 텍스트 안 조각의 위치 (렌더링이 같은 오프셋을 쓰도록)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextLoc {
    Block(usize),
    Cell {
        block: usize,
        row: usize,
        col: usize,
    },
    SignLine {
        block: usize,
        line: usize,
    },
}

/// 문서 텍스트를 만드는 순서: 블록마다 조각 → 줄바꿈. 표는 행마다 셀을 탭으로 잇고 행 끝에 줄바꿈.
/// 콜백은 (조각, 문서 텍스트 안 시작 문자 오프셋, 위치)를 받는다.
pub fn walk_text(doc: &ResolvedDoc, mut f: impl FnMut(&RText, usize, TextLoc)) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    let mut emit = |out: &mut String, chars: &mut usize, t: &RText, loc: TextLoc, sep: char| {
        f(t, *chars, loc);
        out.push_str(&t.text);
        out.push(sep);
        *chars += t.text.chars().count() + 1;
    };
    for (b, block) in doc.blocks.iter().enumerate() {
        match block {
            RBlock::Title(t) | RBlock::Paragraph { text: t, .. } => {
                emit(&mut out, &mut chars, t, TextLoc::Block(b), '\n')
            }
            RBlock::Table { rows, .. } => {
                for (r, row) in rows.iter().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        let sep = if c + 1 == row.len() { '\n' } else { '\t' };
                        emit(
                            &mut out,
                            &mut chars,
                            &cell.text,
                            TextLoc::Cell {
                                block: b,
                                row: r,
                                col: c,
                            },
                            sep,
                        );
                    }
                }
            }
            RBlock::Signature { lines, .. } => {
                for (i, line) in lines.iter().enumerate() {
                    emit(
                        &mut out,
                        &mut chars,
                        line,
                        TextLoc::SignLine { block: b, line: i },
                        '\n',
                    );
                }
            }
            RBlock::PageBreak => {}
        }
    }
    out
}

/// 페이지 수 (쪽 나눔 + 1)
pub fn page_count(doc: &ResolvedDoc) -> u32 {
    1 + doc
        .blocks
        .iter()
        .filter(|b| matches!(b, RBlock::PageBreak))
        .count() as u32
}

/// 텍스트 전용 모드 라벨 (박스 없음)
pub fn text_label(doc: &ResolvedDoc) -> DocLabel {
    let mut spans = Vec::new();
    let mut attributes = Vec::new();
    let mut negatives = Vec::new();
    let mut texts: Vec<(String, usize, Field)> = Vec::new();
    let text = walk_text(doc, |t, base, _| {
        for f in &t.fields {
            let value: String = t.text.chars().skip(f.start).take(f.end - f.start).collect();
            texts.push((value, base, f.clone()));
        }
    });
    for (value, base, f) in texts {
        let (s, e) = (CharOffset(base + f.start), CharOffset(base + f.end));
        match f.kind {
            FieldKind::Label { label, owner } => spans.push(Span {
                id: spans.len() as u32 + 1,
                label,
                owner,
                start: s,
                end: e,
                value,
                hand: f.hand,
                source: f.source,
            }),
            FieldKind::Attribute(kind) => attributes.push(AttributeSpan {
                id: attributes.len() as u32 + 1,
                kind,
                start: s,
                end: e,
                value,
            }),
            FieldKind::Negative => negatives.push(NegativeSpan {
                id: negatives.len() as u32 + 1,
                source: f.source,
                start: s,
                end: e,
                value,
            }),
            FieldKind::Plain | FieldKind::Check { .. } => {}
        }
    }
    DocLabel {
        doc_id: doc.doc_id.clone(),
        form: doc.form.clone(),
        layout: format!("{:?}", doc.layout),
        variant: doc.variant.clone(),
        employer_kind: doc.employer_kind.clone(),
        industry: doc.industry.clone(),
        difficulty: "clean".to_string(),
        handwriting_doc: doc.handwriting_doc,
        seed: doc.seed,
        pages: page_count(doc),
        text,
        spans,
        attributes,
        negatives,
        boxes: Vec::new(),
    }
}
