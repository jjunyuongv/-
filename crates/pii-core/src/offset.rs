//! 오프셋 newtype (PLAN.md 3장 오프셋 규칙).
//!
//! 라벨의 `start/end`는 **문자(char) 오프셋**이다. `regex` 같은 바이트 오프셋과 섞이지 않도록 타입을 나누고,
//! 바꿀 때는 이 모듈의 변환 함수만 쓴다.

use serde::{Deserialize, Serialize};

/// 문자(char) 단위 오프셋
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct CharOffset(pub usize);

/// 바이트(UTF-8) 단위 오프셋
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ByteOffset(pub usize);

/// 바이트 오프셋 → 문자 오프셋. 글자 경계가 아니거나 범위를 벗어나면 `None`
pub fn byte_to_char(text: &str, b: ByteOffset) -> Option<CharOffset> {
    if b.0 > text.len() || !text.is_char_boundary(b.0) {
        return None;
    }
    Some(CharOffset(text[..b.0].chars().count()))
}

/// 문자 오프셋 → 바이트 오프셋. 범위를 벗어나면 `None`
pub fn char_to_byte(text: &str, c: CharOffset) -> Option<ByteOffset> {
    if c.0 == 0 {
        return Some(ByteOffset(0));
    }
    let mut count = 0;
    for (i, _) in text.char_indices() {
        if count == c.0 {
            return Some(ByteOffset(i));
        }
        count += 1;
    }
    (count == c.0).then_some(ByteOffset(text.len()))
}

/// 문자 오프셋 범위의 부분 문자열
pub fn slice_chars(text: &str, start: CharOffset, end: CharOffset) -> Option<&str> {
    let s = char_to_byte(text, start)?;
    let e = char_to_byte(text, end)?;
    text.get(s.0..e.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_mixed_scripts() {
        let t = "주민 ９００１０１-1🙂abc";
        for (ci, (bi, _)) in t.char_indices().enumerate() {
            assert_eq!(byte_to_char(t, ByteOffset(bi)), Some(CharOffset(ci)));
            assert_eq!(char_to_byte(t, CharOffset(ci)), Some(ByteOffset(bi)));
        }
        let n = t.chars().count();
        assert_eq!(char_to_byte(t, CharOffset(n)), Some(ByteOffset(t.len())));
        assert_eq!(char_to_byte(t, CharOffset(n + 1)), None);
        assert_eq!(byte_to_char(t, ByteOffset(1)), None); // '주'의 중간 바이트
        assert_eq!(
            slice_chars(t, CharOffset(3), CharOffset(9)),
            Some("９００１０１")
        );
    }
}
