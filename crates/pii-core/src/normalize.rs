//! 탐지용 텍스트 정규화 + 원문 위치 대응표 (PLAN.md 9단계, CLAUDE.md 도메인 규칙)
//!
//! - NFKC (전각 → 반각, `㎡` → `m2`, `․` → `.`) — 글자 하나가 여러 글자로 늘어날 수 있다
//! - OCR 오인식 치환: 숫자 사이의 `O`/`o` → `0`, `l`/`I`/`|` → `1`
//! - 하이픈 변형(‐‑‒–—―－ㅡ 등) → `-`
//!
//! 정규화 텍스트의 글자마다 원문 글자 위치를 들고 있어, 탐지 결과를 **원문 위치로 되돌린다**.
//! 정규화 글자 범위가 원문 글자의 일부만 덮으면 그 원문 글자 전체로 넓힌다 (유출 방지 우선).

use unicode_normalization::UnicodeNormalization;

use crate::offset::CharOffset;

/// 정규화 결과
#[derive(Debug, Clone)]
pub struct Normalized {
    pub text: String,
    /// 정규화 글자 i → 원문 글자 위치
    map: Vec<usize>,
    /// 원문 글자 수
    orig_len: usize,
}

fn is_hyphen_like(c: char) -> bool {
    matches!(
        c,
        '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{FE58}' | '\u{FE63}' | '\u{FF0D}' | 'ㅡ'
    )
}

impl Normalized {
    pub fn new(original: &str) -> Self {
        let orig: Vec<char> = original.chars().collect();
        let mut chars: Vec<char> = Vec::with_capacity(orig.len());
        let mut map: Vec<usize> = Vec::with_capacity(orig.len());
        for (i, c) in orig.iter().enumerate() {
            if is_hyphen_like(*c) {
                chars.push('-');
                map.push(i);
                continue;
            }
            let mut any = false;
            for n in c.to_string().nfkc() {
                chars.push(n);
                map.push(i);
                any = true;
            }
            if !any {
                // NFKC가 글자를 지우는 경우는 없지만, 대응표가 비지 않게 원래 글자를 둔다
                chars.push(*c);
                map.push(i);
            }
        }
        // 숫자 사이 OCR 오인식 치환 (앞뒤가 숫자일 때만)
        let is_digit = |c: char| c.is_ascii_digit();
        for i in 0..chars.len() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1).copied();
            let between = prev.is_some_and(is_digit) && next.is_some_and(is_digit);
            // 숫자 한쪽 옆이고 반대쪽이 글자가 아님 (예: 맨 앞 O10-…, 끝자리 …123O)
            let not_letter = |c: Option<char>| !c.is_some_and(char::is_alphabetic);
            let edge = (prev.is_some_and(is_digit) && not_letter(next))
                || (next.is_some_and(is_digit) && not_letter(prev));
            let c = chars[i];
            if (c == 'O' && (between || edge)) || (c == 'o' && between) {
                chars[i] = '0';
            } else if matches!(c, 'l' | 'I' | '|') && between {
                chars[i] = '1';
            }
        }
        Normalized {
            text: chars.into_iter().collect(),
            map,
            orig_len: orig.len(),
        }
    }

    /// 정규화 글자 범위 [start, end) → 원문 글자 범위 (원문 글자를 일부만 덮으면 그 글자 전체)
    pub fn to_original(&self, start: CharOffset, end: CharOffset) -> (CharOffset, CharOffset) {
        if self.map.is_empty() || start.0 >= end.0 {
            return (CharOffset(0), CharOffset(0));
        }
        let s = self.map[start.0.min(self.map.len() - 1)];
        let e = self.map[(end.0 - 1).min(self.map.len() - 1)] + 1;
        (CharOffset(s), CharOffset(e.min(self.orig_len)))
    }

    pub fn char_len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn back(n: &Normalized, orig: &str, needle: &str) -> String {
        let chars: Vec<char> = n.text.chars().collect();
        let nn: Vec<char> = needle.chars().collect();
        let i = (0..chars.len())
            .find(|&i| chars[i..].starts_with(&nn))
            .unwrap();
        let (s, e) = n.to_original(CharOffset(i), CharOffset(i + nn.len()));
        orig.chars().skip(s.0).take(e.0 - s.0).collect()
    }

    #[test]
    fn plan_examples_map_back_to_original() {
        // PLAN.md 9단계 예시
        let orig = "연락처 ０１０－１２３４－５６７８, 백상지(80g/㎡), 시간외․야간";
        let n = Normalized::new(orig);
        assert!(n.text.contains("010-1234-5678"), "{}", n.text);
        assert_eq!(
            back(&n, orig, "010-1234-5678"),
            "０１０－１２３４－５６７８"
        );
        // ㎡ → m2: 2글자가 원문 1글자로
        assert!(n.text.contains("80g/m2"));
        assert_eq!(back(&n, orig, "m2"), "㎡");
        // 일부(m)만 덮어도 원문 글자 전체
        assert_eq!(back(&n, orig, "/m"), "/㎡");
        assert!(n.text.contains("시간외.야간"));
    }

    #[test]
    fn ocr_confusions_between_digits() {
        let orig = "O1O-12l4-5678 Office I am";
        let n = Normalized::new(orig);
        assert!(n.text.starts_with("010-1214-5678"), "{}", n.text);
        assert!(n.text.ends_with("Office I am"), "{}", n.text);
    }
}
