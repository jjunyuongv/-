//! 번호 표기: 자릿수 묶음 + 표기 방식 (하이픈·공백·점·붙여 쓰기, 전각 숫자, 뒷부분 가림)

/// 묶음 사이 구분자
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    Hyphen,
    Space,
    Dot,
    /// 붙여 쓰기
    None,
}

impl Separator {
    pub const ALL: [Separator; 4] = [
        Separator::Hyphen,
        Separator::Space,
        Separator::Dot,
        Separator::None,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Separator::Hyphen => "-",
            Separator::Space => " ",
            Separator::Dot => ".",
            Separator::None => "",
        }
    }
}

/// 표기 방식. 기본값은 하이픈, 반각, 가림 없음
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberStyle {
    pub sep: Separator,
    /// 숫자를 전각(０-９)으로
    pub fullwidth: bool,
    /// 마지막 묶음 중 끝에서 n자리를 `*`로 가림 (예: 주민번호 뒷자리 `******`)
    pub mask_tail: usize,
}

impl Default for NumberStyle {
    fn default() -> Self {
        NumberStyle {
            sep: Separator::Hyphen,
            fullwidth: false,
            mask_tail: 0,
        }
    }
}

/// 자릿수 묶음으로 된 번호 (예: `["010", "1234", "5678"]`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberText {
    pub groups: Vec<String>,
}

impl NumberText {
    pub fn new(groups: Vec<String>) -> Self {
        NumberText { groups }
    }

    /// 묶음 없이 이어 붙인 숫자 (검증·비교용)
    pub fn digits(&self) -> String {
        self.groups.concat()
    }

    /// 기본 표기 (하이픈)
    pub fn canonical(&self) -> String {
        self.render(NumberStyle::default())
    }

    pub fn render(&self, style: NumberStyle) -> String {
        let mut groups = self.groups.clone();
        if style.mask_tail > 0
            && let Some(last) = groups.last_mut()
        {
            let keep = last.chars().count().saturating_sub(style.mask_tail);
            *last = last
                .chars()
                .enumerate()
                .map(|(i, c)| if i < keep { c } else { '*' })
                .collect();
        }
        let joined = groups.join(style.sep.as_str());
        if style.fullwidth {
            joined.chars().map(to_fullwidth).collect()
        } else {
            joined
        }
    }
}

/// 반각 숫자 → 전각 숫자 (다른 글자는 그대로)
pub fn to_fullwidth(c: char) -> char {
    match c {
        '0'..='9' => char::from_u32(u32::from(c) - u32::from('0') + 0xFF10).unwrap_or(c),
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_styles() {
        let n = NumberText::new(vec!["900101".into(), "1234567".into()]);
        assert_eq!(n.canonical(), "900101-1234567");
        assert_eq!(n.digits(), "9001011234567");
        let masked = NumberStyle {
            mask_tail: 6,
            ..NumberStyle::default()
        };
        assert_eq!(n.render(masked), "900101-1******");
        let full = NumberStyle {
            sep: Separator::Space,
            fullwidth: true,
            mask_tail: 0,
        };
        assert_eq!(n.render(full), "９００１０１ １２３４５６７");
        let none = NumberStyle {
            sep: Separator::None,
            ..NumberStyle::default()
        };
        assert_eq!(n.render(none), "9001011234567");
    }
}
