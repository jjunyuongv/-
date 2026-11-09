//! D0 디지털 문서: 텍스트 레이어가 있는 PDF (PLAN.md 4단계, 사용자 결정 2026-11-08 — `krilla`)
//!
//! - 이미지와 **같은 배치**(`layout`)를 쓴다 → 라벨 박스(200dpi 픽셀)에 `PT_PER_PX`를 곱하면 PDF 좌표(pt, 왼쪽 위 원점)
//! - 글자: 배치에 쓴 글꼴 파일을 부분 집합으로 넣고, 글리프 번호·전진 폭을 ab_glyph와 똑같이 지정 → 이미지와 같은 자리
//! - 표 괘선: 벡터 선
//! - 서명·도장·지장: 쪽마다 투명 이미지 한 장으로 붙임 (전자계약처럼) — 텍스트 레이어에는 없음
//! - 문서 정보(제목·작성자 등)는 넣지 않는다 (개인정보가 메타데이터로 새지 않게)

use std::collections::BTreeMap;

use ab_glyph::Font as _;
use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use tiny_skia::Pixmap;

use super::{FontKind, Fonts, Item, LaidOut, PAGE_H, PAGE_W, RenderError, marks};

/// 200dpi 픽셀 → pt (1pt = 1/72인치)
pub const PT_PER_PX: f32 = 72.0 / 200.0;

/// 같은 줄에 이어지는 같은 글꼴·크기·색 글자 묶음 (텍스트 객체 하나)
struct GlyphRun {
    font: FontKind,
    px: f32,
    rgb: [u8; 3],
    x: f32,
    y: f32,
    /// 다음 글자가 와야 할 x (이어지는지 판단)
    next_x: f32,
    text: String,
    glyphs: Vec<KrillaGlyph>,
}

/// 배치 결과를 PDF 바이트로
pub fn write_pdf(laid: &LaidOut, fonts: &Fonts) -> Result<Vec<u8>, RenderError> {
    // 쓰인 글꼴만 krilla 글꼴로 읽는다
    let mut pdf_fonts: BTreeMap<FontKind, Font> = BTreeMap::new();
    for items in &laid.pages {
        for item in items {
            if let Item::Glyph { font, .. } = item
                && !pdf_fonts.contains_key(font)
            {
                let path = fonts.path(*font);
                let bytes = std::fs::read(&path).map_err(|source| RenderError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
                let f = Font::new(bytes.into(), 0).ok_or_else(|| {
                    RenderError::Font(format!("PDF 글꼴로 읽을 수 없음: {}", font.file_name()))
                })?;
                pdf_fonts.insert(*font, f);
            }
        }
    }

    let mut doc = Document::new();
    let settings = PageSettings::from_wh(PAGE_W as f32 * PT_PER_PX, PAGE_H as f32 * PT_PER_PX)
        .ok_or_else(|| RenderError::Pdf("쪽 크기".into()))?;
    for items in &laid.pages {
        let mut page = doc.start_page_with(settings.clone());
        let mut surface = page.surface();
        let mut run: Option<GlyphRun> = None;
        for item in items {
            match item {
                Item::Glyph {
                    font,
                    ch,
                    x,
                    y,
                    px,
                    rgb,
                } => {
                    let continues = run.as_ref().is_some_and(|r| {
                        r.font == *font
                            && r.px == *px
                            && r.rgb == *rgb
                            && r.y == *y
                            && (r.next_x - *x).abs() < 0.01
                    });
                    if !continues && let Some(r) = run.take() {
                        draw_run(&mut surface, &pdf_fonts, fonts, r);
                    }
                    let r = run.get_or_insert_with(|| GlyphRun {
                        font: *font,
                        px: *px,
                        rgb: *rgb,
                        x: *x,
                        y: *y,
                        next_x: *x,
                        text: String::new(),
                        glyphs: Vec::new(),
                    });
                    let af = fonts.get(*font);
                    let gid = af.glyph_id(*ch);
                    let upem = af.units_per_em().unwrap_or(1000.0);
                    let start = r.text.len();
                    r.text.push(*ch);
                    // krilla 전진 폭은 em 단위로 정규화 (ab_glyph 전진 폭 / units_per_em)
                    r.glyphs.push(KrillaGlyph::new(
                        GlyphId::new(u32::from(gid.0)),
                        af.h_advance_unscaled(gid) / upem,
                        0.0,
                        0.0,
                        0.0,
                        start..r.text.len(),
                        None,
                    ));
                    r.next_x = *x + fonts.advance(*font, *px, *ch);
                }
                Item::Rect { x, y, w, h, stroke } => {
                    if let Some(r) = run.take() {
                        draw_run(&mut surface, &pdf_fonts, fonts, r);
                    }
                    draw_rect(&mut surface, *x, *y, *w, *h, *stroke);
                }
                Item::Mark(_) => {}
            }
        }
        if let Some(r) = run.take() {
            draw_run(&mut surface, &pdf_fonts, fonts, r);
        }
        draw_marks(&mut surface, fonts, items)?;
        surface.finish();
        page.finish();
    }
    doc.finish()
        .map_err(|e| RenderError::Pdf(format!("PDF 생성 실패: {e}")))
}

fn draw_run(
    surface: &mut Surface,
    pdf_fonts: &BTreeMap<FontKind, Font>,
    fonts: &Fonts,
    r: GlyphRun,
) {
    let Some(font) = pdf_fonts.get(&r.font) else {
        return;
    };
    let af = fonts.get(r.font);
    // ab_glyph PxScale(px) = ascent - descent가 px → PDF 글꼴 크기(em) = px × upem / 높이
    let upem = af.units_per_em().unwrap_or(1000.0);
    let height = af.height_unscaled();
    if height <= 0.0 {
        return;
    }
    let size = r.px * upem / height * PT_PER_PX;
    surface.set_fill(Some(Fill {
        paint: rgb::Color::new(r.rgb[0], r.rgb[1], r.rgb[2]).into(),
        opacity: NormalizedF32::ONE,
        rule: Default::default(),
    }));
    surface.draw_glyphs(
        Point::from_xy(r.x * PT_PER_PX, r.y * PT_PER_PX),
        &r.glyphs,
        font.clone(),
        &r.text,
        size,
        false,
    );
}

fn draw_rect(surface: &mut Surface, x: f32, y: f32, w: f32, h: f32, stroke: f32) {
    let Some(rect) = Rect::from_xywh(x * PT_PER_PX, y * PT_PER_PX, w * PT_PER_PX, h * PT_PER_PX)
    else {
        return;
    };
    let mut pb = PathBuilder::new();
    pb.push_rect(rect);
    let Some(path) = pb.finish() else {
        return;
    };
    surface.set_fill(None);
    surface.set_stroke(Some(Stroke {
        width: stroke * PT_PER_PX,
        ..Default::default()
    }));
    surface.draw_path(&path);
    surface.set_stroke(None);
}

/// 쪽의 서명·도장·지장을 투명 이미지 한 장으로 붙인다 (그린 부분만 잘라서)
fn draw_marks(surface: &mut Surface, fonts: &Fonts, items: &[Item]) -> Result<(), RenderError> {
    let marks_on_page: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            Item::Mark(m) => Some(m),
            _ => None,
        })
        .collect();
    if marks_on_page.is_empty() {
        return Ok(());
    }
    let mut pix = Pixmap::new(PAGE_W, PAGE_H).ok_or(RenderError::Pixmap)?;
    for m in marks_on_page {
        marks::draw_mark(&mut pix, fonts, m);
    }
    let Some((x0, y0, x1, y1)) = alpha_bounds(&pix) else {
        return Ok(());
    };
    let (w, h) = (x1 - x0, y1 - y0);
    // tiny-skia는 premultiplied RGBA → PDF 이미지는 straight RGBA
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    let data = pix.data();
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * PAGE_W + x) * 4) as usize;
            let a = data[i + 3];
            for k in 0..3 {
                let c = if a == 0 {
                    0
                } else {
                    ((u32::from(data[i + k]) * 255 + u32::from(a) / 2) / u32::from(a)).min(255)
                        as u8
                };
                rgba.push(c);
            }
            rgba.push(a);
        }
    }
    let image = Image::from_rgba8(rgba, w, h);
    let size = Size::from_wh(w as f32 * PT_PER_PX, h as f32 * PT_PER_PX)
        .ok_or_else(|| RenderError::Pdf("도장 이미지 크기".into()))?;
    surface.push_transform(&Transform::from_translate(
        x0 as f32 * PT_PER_PX,
        y0 as f32 * PT_PER_PX,
    ));
    surface.draw_image(image, size);
    surface.pop();
    Ok(())
}

/// 알파가 있는 픽셀의 경계 (x0, y0, x1, y1), 끝은 제외
fn alpha_bounds(pix: &Pixmap) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (pix.width(), pix.height());
    let data = pix.data();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if data[((y * w + x) * 4 + 3) as usize] > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}
