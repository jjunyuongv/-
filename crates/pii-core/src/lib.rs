//! 공통 타입: Label, Span, BBox, DocLabel, CharOffset/ByteOffset, 정규화 대응표.
//!
//! 지금 구현된 것: 라벨(`Label`), 소속(`Owner`), 참고 항목 종류(`AttributeKind`), 오프셋 newtype(`offset`),
//! 문서 라벨 JSON(`doc_label`). 정규화 대응표는 PLAN.md 9단계에서 채운다.

pub mod detection;
pub mod doc_label;
pub mod normalize;
pub mod offset;

pub use detection::{Detection, Detector};
pub use doc_label::{AttributeSpan, BoxLabel, DocLabel, MarkKind, NegativeSpan, Span};
pub use normalize::Normalized;
pub use offset::{ByteOffset, CharOffset, byte_to_char, char_to_byte, slice_chars};

use serde::{Deserialize, Serialize};

/// 개인정보 라벨 (PLAN.md 3장). 직렬화 이름은 `RRN`, `DRIVER_LICENSE` 같은 대문자 스네이크.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Label {
    // 고유식별정보
    Rrn,
    Frn,
    Passport,
    DriverLicense,
    // 일반 개인정보
    Name,
    Address,
    Phone,
    Email,
    Account,
    BirthDate,
    EmpId,
    // 민감정보 (제23조)
    Health,
    Union,
    Criminal,
    Religion,
    Political,
    /// 성생활 (제23조). 탐지기·규칙 팩에는 없는 라벨 — 처음 보는 서식 시험의 정답에만 쓰고 재현율 묶음에 넣지 않는다
    SexLife,
    // 이미지 전용
    Signature,
    Seal,
    Fingerprint,
    // 준식별 (마스킹 정책으로 켜고 끔)
    Salary,
    BizNo,
}

impl Label {
    /// 정형 라벨 — 텍스트 1,000건에서 span 최소 500개를 목표로 하는 라벨 (PLAN.md 5단계)
    pub const STRUCTURED: [Label; 7] = [
        Label::Rrn,
        Label::Frn,
        Label::Passport,
        Label::DriverLicense,
        Label::Phone,
        Label::Email,
        Label::Account,
    ];

    /// 고유식별정보 — **소속(`owner`)과 관계없이 항상 가린다** (1-2 결정, 2026-11-08).
    /// 규칙 팩 마스킹 정책으로 끌 수 없다: 이 라벨을 끄는 팩은 검증 단계에서 거부한다 (PLAN.md 9단계).
    pub const ALWAYS_MASKED: [Label; 4] = [
        Label::Rrn,
        Label::Frn,
        Label::Passport,
        Label::DriverLicense,
    ];

    /// 마스킹 정책으로 끌 수 없는 라벨인지
    pub fn is_always_masked(self) -> bool {
        Self::ALWAYS_MASKED.contains(&self)
    }

    /// 이미지에서 영역으로만 표시하는 라벨 (텍스트 span 없음)
    pub fn is_image_only(self) -> bool {
        matches!(self, Label::Signature | Label::Seal | Label::Fingerprint)
    }
}

/// 개인정보의 소속. 서명란 박스와 span 모두에 붙인다 (1-2 결정: 사업주 쪽을 정책으로 따로 켜고 끔).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Worker,
    Employer,
    Guardian,
}

/// 참고 항목 종류 (1-2 결정). 라벨은 아니지만 위치를 기록하고, 평가에서 여기 걸린 탐지는 중립.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeKind {
    /// 친권자와 연소근로자의 관계
    Relation,
    /// 만 나이
    Age,
    /// 국적
    Nationality,
    /// 비상연락처와 근로자의 관계
    EmergencyRelation,
    /// 사업장 주소·근무 장소 (2단계 검토 결정: 개인정보 아님, 주소 탐지가 걸려도 중립)
    WorkplaceAddress,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_names_match_plan() {
        let names: Vec<String> = [
            Label::Rrn,
            Label::DriverLicense,
            Label::BirthDate,
            Label::EmpId,
            Label::BizNo,
        ]
        .iter()
        .map(|l| serde_json::to_string(l).unwrap())
        .collect();
        assert_eq!(
            names,
            [
                "\"RRN\"",
                "\"DRIVER_LICENSE\"",
                "\"BIRTH_DATE\"",
                "\"EMP_ID\"",
                "\"BIZ_NO\""
            ]
        );
    }

    #[test]
    fn owner_and_attribute_names() {
        assert_eq!(
            serde_json::to_string(&Owner::Employer).unwrap(),
            "\"employer\""
        );
        assert_eq!(
            serde_json::to_string(&AttributeKind::EmergencyRelation).unwrap(),
            "\"emergency_relation\""
        );
    }

    #[test]
    fn structured_labels_are_text_labels() {
        assert!(Label::STRUCTURED.iter().all(|l| !l.is_image_only()));
    }

    #[test]
    fn unique_identifiers_are_always_masked() {
        for l in [
            Label::Rrn,
            Label::Frn,
            Label::Passport,
            Label::DriverLicense,
        ] {
            assert!(l.is_always_masked(), "{l:?}");
        }
        // 준식별·일반 개인정보는 정책으로 켜고 끔
        for l in [Label::Name, Label::Phone, Label::Salary, Label::BizNo] {
            assert!(!l.is_always_masked(), "{l:?}");
        }
        // 고유식별정보는 모두 정형 라벨 (텍스트 500개 목표에 포함)
        assert!(
            Label::ALWAYS_MASKED
                .iter()
                .all(|l| Label::STRUCTURED.contains(l))
        );
    }
}
