//! 탐지 결과 (규칙·NER·SLM 공통). 위치는 원문 글자 단위, 값은 담지 않는다 (로그에 원문이 남지 않게)

use serde::{Deserialize, Serialize};

use crate::{CharOffset, Label};

/// 어느 탐지기가 찾았나
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detector {
    Rule,
    Ner,
    Slm,
    /// 이미 찾은 값과 같은 문자열 (문서 안 다른 위치)
    Propagated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detection {
    pub label: Label,
    pub start: CharOffset,
    pub end: CharOffset,
    pub detector: Detector,
}

impl Detection {
    pub fn overlaps(&self, start: usize, end: usize) -> bool {
        self.start.0 < end && start < self.end.0
    }
}
