//! KoELECTRA 한국어 PII NER (PLAN.md 10단계, 실험 E2) — `ort` + `tokenizers`
//!
//! - 모델: `models/ner/koelectra-ko-pii-ner/` (`model_int8.onnx`, `tokenizer.json`, `config.json`의 id2label)
//! - 512 토큰 제한 → 줄 단위로 묶어 청크(약 300글자)로 나누고, 넘치면 줄 하나씩
//! - 토큰 위치는 `encode_char_offsets`로 **글자 오프셋** (CLAUDE.md 오프셋 규칙)
//! - 모델 라벨 29종 → 프로젝트 라벨. `none` 계층(DATE·PLACE·ORGANIZATION 등)은 가리지 않으므로 버림

use std::collections::BTreeMap;
use std::path::Path;

use ort::session::Session;
use ort::value::Tensor;
use pii_core::{CharOffset, Detection, Detector, Label};
use tokenizers::Tokenizer;

#[derive(Debug, thiserror::Error)]
pub enum NerError {
    #[error("NER 모델 준비 실패: {0}")]
    Load(String),
    #[error("NER 실행 실패: {0}")]
    Run(String),
}

/// 모델 라벨 → 프로젝트 라벨 (None = 가리지 않음)
pub fn map_label(model: &str) -> Option<Label> {
    Some(match model {
        "NAME" => Label::Name,
        "ADDRESS" => Label::Address,
        "PHONE" => Label::Phone,
        "EMAIL" => Label::Email,
        "RRN" => Label::Rrn,
        "ALIEN_ID" => Label::Frn,
        "PASSPORT" => Label::Passport,
        "DRIVER_LICENSE" => Label::DriverLicense,
        "ACCOUNT_NUMBER" | "CARD_NUMBER" | "CARD_EXPIRY" | "CVC" | "SECRET" | "IPIN" => {
            Label::Account
        }
        "BIRTHDATE" => Label::BirthDate,
        "GENERIC_ID" | "USER_ID" | "VEHICLE_NUMBER" => Label::EmpId,
        // 종교 등 속성 (민감정보 쪽)
        "ATTRIBUTE" => Label::Religion,
        _ => return None,
    })
}

pub struct NerDetector {
    session: Session,
    tokenizer: Tokenizer,
    id2label: Vec<String>,
    with_type_ids: bool,
}

/// 줄 단위 청크: (청크 시작 글자 위치, 청크 텍스트)
fn chunks(text: &str, max_chars: usize) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_start = 0;
    let mut cur_len = 0;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let n = line.chars().count();
        if cur_len + n > max_chars && cur_len > 0 {
            out.push((cur_start, std::mem::take(&mut cur)));
            cur_len = 0;
        }
        if cur_len == 0 {
            cur_start = pos;
        }
        cur.push_str(line);
        cur_len += n;
        pos += n;
    }
    if cur_len > 0 {
        out.push((cur_start, cur));
    }
    out
}

impl NerDetector {
    pub fn load(dir: &Path) -> Result<Self, NerError> {
        let builder = Session::builder().map_err(|e| NerError::Load(e.to_string()))?;
        let mut builder = builder
            .with_intra_threads(4)
            .map_err(|e| NerError::Load(e.to_string()))?;
        let session = builder
            .commit_from_file(dir.join("model_int8.onnx"))
            .map_err(|e| NerError::Load(e.to_string()))?;
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| NerError::Load(e.to_string()))?;
        let config: serde_json::Value = std::fs::read_to_string(dir.join("config.json"))
            .map_err(|e| NerError::Load(e.to_string()))
            .and_then(|s| serde_json::from_str(&s).map_err(|e| NerError::Load(e.to_string())))?;
        let map: BTreeMap<usize, String> = config["id2label"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if map.is_empty() {
            return Err(NerError::Load("id2label 없음".into()));
        }
        let id2label = (0..=*map.keys().max().unwrap_or(&0))
            .map(|i| map.get(&i).cloned().unwrap_or_else(|| "O".into()))
            .collect();
        let with_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        Ok(NerDetector {
            session,
            tokenizer,
            id2label,
            with_type_ids,
        })
    }

    /// 청크 하나 → (글자 시작, 글자 끝, 모델 라벨) 개체들 (청크 기준 위치)
    fn run_chunk(&mut self, chunk: &str) -> Result<Vec<(usize, usize, String)>, NerError> {
        let enc = self
            .tokenizer
            .encode_char_offsets(chunk, true)
            .map_err(|e| NerError::Run(e.to_string()))?;
        let n = enc.get_ids().len();
        if n > 512 {
            return Err(NerError::Run("토큰 512개 초과".into()));
        }
        let ids: Vec<i64> = enc.get_ids().iter().map(|x| i64::from(*x)).collect();
        let mask: Vec<i64> = enc
            .get_attention_mask()
            .iter()
            .map(|x| i64::from(*x))
            .collect();
        let types: Vec<i64> = enc.get_type_ids().iter().map(|x| i64::from(*x)).collect();
        let shape = [1usize, n];
        let t =
            |v: Vec<i64>| Tensor::from_array((shape, v)).map_err(|e| NerError::Run(e.to_string()));
        let outputs = if self.with_type_ids {
            self.session.run(ort::inputs![
                "input_ids" => t(ids)?,
                "attention_mask" => t(mask)?,
                "token_type_ids" => t(types)?
            ])
        } else {
            self.session.run(ort::inputs![
                "input_ids" => t(ids)?,
                "attention_mask" => t(mask)?
            ])
        }
        .map_err(|e| NerError::Run(e.to_string()))?;
        let (oshape, logits) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| NerError::Run(e.to_string()))?;
        let classes = oshape.last().copied().unwrap_or(0) as usize;
        if classes == 0 {
            return Ok(Vec::new());
        }
        let offsets = enc.get_offsets();
        let specials = enc.get_special_tokens_mask();
        let mut ents: Vec<(usize, usize, String)> = Vec::new();
        for (i, (s, e)) in offsets.iter().enumerate() {
            if specials.get(i) == Some(&1) || s == e {
                continue;
            }
            let row = &logits[i * classes..(i + 1) * classes];
            let best = row
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or(0, |(k, _)| k);
            let tag = self.id2label.get(best).map_or("O", String::as_str);
            if tag == "O" {
                continue;
            }
            let (bio, ty) = tag.split_once('-').unwrap_or(("B", tag));
            match ents.last_mut() {
                // 같은 종류의 I- (또는 바로 붙은 B-)는 이어 붙임
                Some(last) if last.2 == ty && (bio == "I" || *s <= last.1 + 1) => last.1 = *e,
                _ => ents.push((*s, *e, ty.to_string())),
            }
        }
        Ok(ents)
    }

    /// 문서 텍스트 → 탐지 (원문 글자 위치)
    pub fn detect(&mut self, text: &str) -> Result<Vec<Detection>, NerError> {
        let mut out = Vec::new();
        for (base, chunk) in chunks(text, 300) {
            let parts: Vec<(usize, String)> = match self.run_chunk(&chunk) {
                Ok(ents) => {
                    push(&mut out, base, ents);
                    continue;
                }
                // 512 토큰을 넘으면 줄 하나씩
                Err(_) => chunks(&chunk, 1),
            };
            for (b, line) in parts {
                if let Ok(ents) = self.run_chunk(&line) {
                    push(&mut out, base + b, ents);
                }
            }
        }
        Ok(out)
    }
}

fn push(out: &mut Vec<Detection>, base: usize, ents: Vec<(usize, usize, String)>) {
    for (s, e, ty) in ents {
        if let Some(label) = map_label(&ty) {
            out.push(Detection {
                label,
                start: CharOffset(base + s),
                end: CharOffset(base + e),
                detector: Detector::Ner,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_keep_char_positions() {
        let text = "가나다\n라마\n바사아자\n";
        let c = chunks(text, 5);
        for (base, s) in &c {
            let orig: String = text.chars().skip(*base).take(s.chars().count()).collect();
            assert_eq!(&orig, s);
        }
        assert!(c.len() >= 2);
    }

    #[test]
    #[ignore = "모델 파일 필요 (models/ner)"]
    fn model_finds_name_and_phone() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../models/ner/koelectra-ko-pii-ner");
        let mut ner = NerDetector::load(&dir).unwrap();
        let text = "김민준 씨한테 010-1234-5678로 연락해줘";
        let d = ner.detect(text).unwrap();
        assert!(d.iter().any(|x| x.label == Label::Name), "{d:?}");
        assert!(d.iter().any(|x| x.label == Label::Phone), "{d:?}");
    }
}
