//! 문서당 라벨 JSON (`DocLabel`, PLAN.md 3장)

use serde::{Deserialize, Serialize};

use crate::offset::{CharOffset, slice_chars};
use crate::{AttributeKind, Label, Owner};

/// 정답 PII 위치
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub id: u32,
    pub label: Label,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    pub start: CharOffset,
    pub end: CharOffset,
    pub value: String,
    /// 손글씨로 기입된 필드인지
    pub hand: bool,
    /// 이 값을 만든 템플릿 슬롯·문장 풀 (추적용)
    pub source: String,
}

/// 참고 항목 위치 (평가에서 중립)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributeSpan {
    pub id: u32,
    pub kind: AttributeKind,
    pub start: CharOffset,
    pub end: CharOffset,
    pub value: String,
}

/// 양식 안 하드네거티브 위치 (가리면 오탐)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NegativeSpan {
    pub id: u32,
    pub source: String,
    pub start: CharOffset,
    pub end: CharOffset,
    pub value: String,
}

/// 서명란 표시 종류 (PLAN.md 4단계 축 1·2)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    NameSign,
    Scribble,
    PrintSign,
    PersonalSeal,
    CorporateSeal,
    Fingerprint,
}

/// 이미지 박스 (4점 다각형, 픽셀 좌표, 원점 왼쪽 위)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxLabel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<Label>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<MarkKind>,
    /// 0부터
    pub page: u32,
    pub poly: [[f32; 2]; 4],
}

/// 문서 하나의 정답
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocLabel {
    pub doc_id: String,
    /// 템플릿 id (레이아웃 D는 `minor+guardian_consent`)
    pub form: String,
    pub layout: String,
    /// `standard` / `company_added`
    pub variant: String,
    pub employer_kind: String,
    pub industry: String,
    /// 열화 단계 (4단계 렌더링 직후는 `clean`, 5단계에서 D0~D4)
    pub difficulty: String,
    pub handwriting_doc: bool,
    pub seed: u64,
    pub pages: u32,
    pub text: String,
    pub spans: Vec<Span>,
    pub attributes: Vec<AttributeSpan>,
    pub negatives: Vec<NegativeSpan>,
    pub boxes: Vec<BoxLabel>,
}

impl DocLabel {
    /// 모든 span·참고 항목·네거티브의 `value`가 `text`의 그 위치 글자와 같은지 (오프셋 검증)
    pub fn offset_problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        let mut check =
            |kind: &str, id: u32, s: CharOffset, e: CharOffset, v: &str| match slice_chars(
                &self.text, s, e,
            ) {
                Some(t) if t == v => {}
                Some(t) => p.push(format!("{kind} {id}: 위치의 글자 {t:?} ≠ 값 {v:?}")),
                None => p.push(format!("{kind} {id}: 범위 밖 {}..{}", s.0, e.0)),
            };
        for x in &self.spans {
            check("span", x.id, x.start, x.end, &x.value);
        }
        for x in &self.attributes {
            check("attribute", x.id, x.start, x.end, &x.value);
        }
        for x in &self.negatives {
            check("negative", x.id, x.start, x.end, &x.value);
        }
        p
    }
}
