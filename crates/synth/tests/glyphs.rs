//! 템플릿·문장 풀·자료 표에 나오는 모든 글자를 그릴 수 있는지 (나눔고딕 → 나눔명조 → Noto Sans CJK KR 대체 순).
//! 대체 글꼴로 원래 글자를 그대로 그리므로 이미지 글자 = 정답 텍스트 글자.
use std::collections::BTreeSet;
use std::path::PathBuf;

use ab_glyph::Font;
use synth::render::{FontKind, Fonts};

#[test]
fn all_used_characters_have_glyphs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fonts = Fonts::load(&root.join("assets/fonts")).unwrap_or_else(|e| panic!("{e}"));
    let mut chars = BTreeSet::new();
    for dir in ["assets/templates", "assets/clauses", "assets/synth"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if let Ok(s) = std::fs::read_to_string(&p) {
                // 주석 줄은 빼고
                for line in s.lines().filter(|l| !l.trim_start().starts_with('#')) {
                    chars.extend(
                        line.chars()
                            .filter(|c| !c.is_ascii_control() && !c.is_whitespace()),
                    );
                }
            }
        }
    }
    // 렌더링이 만드는 체크 표시·도장 글자, 전각 숫자·하이픈 (표기 변형)
    chars.extend(['■', '□', '인', '지', '－']);
    chars.extend('０'..='９');
    let missing: Vec<String> = chars
        .iter()
        .filter(|c| fonts.missing(**c))
        .map(|c| format!("{c} U+{:04X}", u32::from(*c)))
        .collect();
    assert!(missing.is_empty(), "그릴 수 없는 글자: {missing:?}");

    // 나눔 글꼴에 없어서 Noto로 그리는 글자 (예전에 대체 그리기를 하던 ㎡, U+2024 포함)
    let noto: Vec<(char, FontKind)> = chars
        .iter()
        .map(|c| (*c, fonts.resolve(FontKind::Gothic, *c)))
        .filter(|(_, k)| matches!(k, FontKind::Noto | FontKind::NotoLatin))
        .collect();
    assert_eq!(fonts.resolve(FontKind::Gothic, '㎡'), FontKind::Noto);
    assert_eq!(
        fonts.resolve(FontKind::Gothic, '\u{2024}'),
        FontKind::NotoLatin
    );
    assert!(fonts.noto.glyph_id('㎡').0 != 0);
    println!("Noto로 그리는 글자: {noto:?}");
}

#[test]
fn handwriting_is_visually_bigger_than_print() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fonts = Fonts::load(&root.join("assets/fonts")).unwrap_or_else(|e| panic!("{e}"));
    // 펜 글꼴은 같은 px에서 한글이 작다 → 배율로 보정 (보정 없으면 손글씨가 인쇄보다 작아 보임)
    assert!(
        fonts.pen_scale > 1.0 && fonts.pen_scale < 3.0,
        "{}",
        fonts.pen_scale
    );
}
