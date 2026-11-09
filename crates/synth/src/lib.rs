//! 합성 데이터: 가짜 PII, 템플릿 채우기, 렌더링, 열화.
//!
//! 지금 구현된 것: 양식 템플릿 스키마·검증 (`template`, PLAN.md 1-2), 합성 비율 설정 (`ratios`),
//! 문장 풀 스키마·검증·조사 처리 (`clauses`, PLAN.md 2단계).
//! 가짜 PII·번호 생성기 (`fake`, PLAN.md 3단계 — 번호 계열). 이름·주소 등과 렌더링·열화는 3~5단계에서 채운다.

pub mod clauses;
pub mod dataset;
pub mod degrade;
pub mod doc;
pub mod fake;
pub mod ratios;
pub mod render;
pub mod template;
