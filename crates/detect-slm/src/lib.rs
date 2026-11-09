//! SLM 민감정보 탐지 (PLAN.md 11단계, 실험 E4) — `llama-server`(OpenAI 호환 API)
//!
//! - **트리거 방식**: 규칙 팩의 SLM 트리거 단어가 있는 줄(청크)만 보낸다
//! - JSON 스키마로 출력 강제 → `{"items": [{"label": "HEALTH", "text": "…"}]}`
//! - **원문 재검색**: SLM이 준 문자열은 그 청크 안에서만 찾는다. 0개면 버림(환각), 여러 개면 모두 가림.
//!   정확히 안 맞으면 공백을 무시하고 한 번 더 찾는다
//! - 시스템 프롬프트(라벨 정의 + 예시)는 매 요청 같아서 llama-server 프롬프트 캐시에 남는다 (실서비스 조건)
//! - few-shot 예시는 합성 문장 풀에 없는 값으로만 만든다 (평가 부풀림 방지)

use std::time::{Duration, Instant};

use pii_core::{CharOffset, Detection, Detector, Label};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum SlmError {
    #[error("SLM 서버 요청 실패: {0}")]
    Http(String),
    #[error("SLM 응답 형식 오류")]
    Format,
}

const SYSTEM: &str = "너는 한국어 근로계약서 문장에서 민감정보를 찾는 도구다. 아래 다섯 가지에 해당하는 표현을 문장에 나온 그대로(글자 하나도 바꾸지 말고) 뽑는다. 민감정보를 드러내는 명사구 전체를 뽑는다: 병명 뒤에 붙은 경과 표현(예: 수술 후 회복), 조합 이름 뒤의 지부·지회·분과까지 포함한다.\n\
- HEALTH: 질병·장애·임신 등 건강 상태 (예: 병명, 장애 종류, 임신 주수)\n\
- UNION: 노동조합 이름 또는 노동조합 가입·활동 사실을 드러내는 조합 이름\n\
- CRIMINAL: 범죄·전과의 죄명\n\
- RELIGION: 종교 이름\n\
- POLITICAL: 정당·정치단체 이름\n\
해당하는 것이 없으면 빈 배열을 준다. 회사명·날짜·금액·법 조항은 뽑지 않는다.\n\
\n\
예시 1) 문장: 근로자는 편두통 치료를 위해 월 1회 반차를 사용할 수 있다.\n\
답: {\"items\":[{\"label\":\"HEALTH\",\"text\":\"편두통\"}]}\n\
예시 2) 문장: 근로자가 무릎 인대 수술 후 재활로 통원하는 날은 유급으로 한다.\n\
답: {\"items\":[{\"label\":\"HEALTH\",\"text\":\"무릎 인대 수술 후 재활\"}]}\n\
예시 3) 문장: 근로자가 푸른바다노동조합 부산지부 조합원으로서 활동하는 것을 이유로 불이익을 주지 않는다.\n\
답: {\"items\":[{\"label\":\"UNION\",\"text\":\"푸른바다노동조합 부산지부\"}]}\n\
예시 4) 문장: 근로자는 공무집행방해 전과가 있음을 회사에 알렸다.\n\
답: {\"items\":[{\"label\":\"CRIMINAL\",\"text\":\"공무집행방해\"}]}\n\
예시 5) 문장: 회사는 근로자가 성공회 신자로서 주일 예배에 참석하는 것을 보장한다.\n\
답: {\"items\":[{\"label\":\"RELIGION\",\"text\":\"성공회\"}]}\n\
예시 6) 문장: 근로자의 들꽃당 당원 활동은 근무시간 외에만 한다.\n\
답: {\"items\":[{\"label\":\"POLITICAL\",\"text\":\"들꽃당\"}]}\n\
예시 7) 문장: 연장근로에 대하여는 근로기준법 제56조에 따라 가산하여 지급한다.\n\
답: {\"items\":[]}";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "label": {"type": "string", "enum": ["HEALTH", "UNION", "CRIMINAL", "RELIGION", "POLITICAL"]},
                        "text": {"type": "string"}
                    },
                    "required": ["label", "text"]
                }
            }
        },
        "required": ["items"]
    })
}

/// SLM 호출 통계 (값은 담지 않음)
#[derive(Debug, Clone, Default)]
pub struct SlmStats {
    pub chunks: usize,
    pub triggered: usize,
    pub calls: usize,
    pub items: usize,
    /// 원문에서 못 찾아 버린 항목 (환각)
    pub dropped: usize,
    /// 청크 안에서 여러 번 나와 모두 가린 항목
    pub multi: usize,
    pub seconds: f64,
}

pub struct SlmClient {
    url: String,
    http: reqwest::blocking::Client,
    triggers: Vec<String>,
    generic: Vec<String>,
    /// Some(제목 단어)면 그 제목 줄 다음부터 날짜 줄 전까지는 트리거 없이도 모두 보냄 (E4b: 특약 전체)
    section: Option<String>,
}

fn label_of(s: &str) -> Option<Label> {
    Some(match s {
        "HEALTH" => Label::Health,
        "UNION" => Label::Union,
        "CRIMINAL" => Label::Criminal,
        "RELIGION" => Label::Religion,
        "POLITICAL" => Label::Political,
        _ => return None,
    })
}

/// 청크 안에서 값의 모든 위치 (정확히 → 없으면 공백 무시)
pub fn find_all(chunk: &[char], value: &str) -> Vec<(usize, usize)> {
    let v: Vec<char> = value.chars().collect();
    let mut out = Vec::new();
    if v.is_empty() {
        return out;
    }
    let mut i = 0;
    while i + v.len() <= chunk.len() {
        if chunk[i..i + v.len()] == v[..] {
            out.push((i, i + v.len()));
            i += v.len();
        } else {
            i += 1;
        }
    }
    if !out.is_empty() {
        return out;
    }
    // 공백 무시 비교
    let vs: Vec<char> = v.iter().copied().filter(|c| !c.is_whitespace()).collect();
    if vs.is_empty() {
        return out;
    }
    let idx: Vec<usize> = (0..chunk.len())
        .filter(|&k| !chunk[k].is_whitespace())
        .collect();
    let flat: Vec<char> = idx.iter().map(|&k| chunk[k]).collect();
    let mut i = 0;
    while i + vs.len() <= flat.len() {
        if flat[i..i + vs.len()] == vs[..] {
            out.push((idx[i], idx[i + vs.len() - 1] + 1));
            i += vs.len();
        } else {
            i += 1;
        }
    }
    out
}

impl SlmClient {
    pub fn new(
        base_url: &str,
        triggers: Vec<String>,
        generic: Vec<String>,
    ) -> Result<Self, SlmError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| SlmError::Http(e.to_string()))?;
        Ok(SlmClient {
            url: format!("{}/v1/chat/completions", base_url.trim_end_matches('/')),
            http,
            triggers,
            generic,
            section: None,
        })
    }

    fn ask(&self, chunk: &str) -> Result<Vec<(String, String)>, SlmError> {
        let body = json!({
            "messages": [
                {"role": "system", "content": SYSTEM},
                {"role": "user", "content": format!("문장: {chunk}")}
            ],
            "temperature": 0,
            "max_tokens": 256,
            "cache_prompt": true,
            "chat_template_kwargs": {"enable_thinking": false},
            "response_format": {"type": "json_schema", "json_schema": {"name": "pii", "schema": schema()}}
        });
        let r: Value = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|e| SlmError::Http(e.to_string()))?;
        let content = r["choices"][0]["message"]["content"]
            .as_str()
            .ok_or(SlmError::Format)?;
        let v: Value = serde_json::from_str(content).map_err(|_| SlmError::Format)?;
        Ok(v["items"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        Some((
                            x["label"].as_str()?.to_string(),
                            x["text"].as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// E4b: `heading`(예: "특약") 제목 줄 다음 줄들은 트리거 없이도 SLM에 보낸다
    pub fn with_section(mut self, heading: &str) -> Self {
        self.section = Some(heading.to_string());
        self
    }

    /// 문서 텍스트 → 민감정보 탐지 (트리거 단어가 있는 줄만 SLM에 보냄)
    pub fn detect(&self, text: &str) -> Result<(Vec<Detection>, SlmStats), SlmError> {
        let t0 = Instant::now();
        let mut stats = SlmStats::default();
        let mut out = Vec::new();
        let mut pos = 0usize;
        let mut in_section = false;
        for line in text.split_inclusive('\n') {
            let n = line.chars().count();
            stats.chunks += 1;
            let t = line.trim();
            // 구역 끝: 날짜 줄 (예: 2025년 5월 28일) 또는 서명란
            let is_date =
                t.chars().count() < 24 && t.contains('년') && t.contains('월') && t.contains('일');
            if in_section && (is_date || t.contains("(서명") || t.contains("(인)")) {
                in_section = false;
            }
            let section_line = in_section && t.chars().count() >= 8;
            if let Some(h) = &self.section
                && t.contains(h.as_str())
                && t.chars().count() < 30
            {
                in_section = true;
            }
            if section_line || self.triggers.iter().any(|t| line.contains(t.as_str())) {
                stats.triggered += 1;
                stats.calls += 1;
                let items = self.ask(line.trim_end())?;
                let chars: Vec<char> = line.chars().collect();
                for (label, value) in items {
                    stats.items += 1;
                    let Some(label) = label_of(&label) else {
                        continue;
                    };
                    if self.generic.iter().any(|g| g == value.trim()) {
                        stats.dropped += 1;
                        continue;
                    }
                    let hits = find_all(&chars, &value);
                    match hits.len() {
                        0 => stats.dropped += 1,
                        1 => {}
                        _ => stats.multi += 1,
                    }
                    for (s, e) in hits {
                        out.push(Detection {
                            label,
                            start: CharOffset(pos + s),
                            end: CharOffset(pos + e),
                            detector: Detector::Slm,
                        });
                    }
                }
            }
            pos += n;
        }
        stats.seconds = t0.elapsed().as_secs_f64();
        Ok((out, stats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn research_rules() {
        let c: Vec<char> = "고혈압 치료와 고혈압 관리".chars().collect();
        assert_eq!(find_all(&c, "고혈압").len(), 2);
        // 공백 차이
        let c: Vec<char> = "허리 디스크 치료".chars().collect();
        assert_eq!(find_all(&c, "허리디스크"), vec![(0, 6)]);
        // 환각
        assert!(find_all(&c, "당뇨병").is_empty());
    }
}
