//! 서명·도장·지장 합성 (PLAN.md 4단계 축 1·2). 실제 사람 서명 이미지는 쓰지 않는다 — 모두 절차적 생성.
//!
//! - 위치: 서명란 앵커("(서명)" 등) 박스 주변, 도장은 앵커 글자를 덮도록
//! - 정답 박스: `SIGNATURE`/`SEAL`/`FINGERPRINT` + `owner` + `kind`. 회전한 도장·지장은 회전된 4점 다각형

use pii_core::{BoxLabel, Label, MarkKind, Owner};
use rand::RngExt;
use tiny_skia::{Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use super::{FontKind, Fonts, draw_glyph};
use crate::doc::{SignMethod, SignPlan};
use crate::fake::{SynthRng, pick};

const SEAL_RED: [u8; 3] = [205, 35, 45];

/// 그릴 표시 하나
#[derive(Debug, Clone)]
pub enum Mark {
    /// 글자 서명 (붓·펜 글꼴): (글자, x, 기준선 y)
    Text {
        font: FontKind,
        glyphs: Vec<(char, f32, f32)>,
        px: f32,
        rgb: [u8; 3],
    },
    /// 곡선 획 (각 획 = 시작점 + 3차 베지어 제어점 3개씩)
    Strokes {
        paths: Vec<Vec<[f32; 2]>>,
        width: f32,
        rgb: [u8; 3],
    },
    Fingerprint {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        rot: f32,
        /// 링마다 끊김 위치(각도) — 생성 시 정해 둠
        rings: Vec<Vec<f32>>,
    },
    Seal {
        cx: f32,
        cy: f32,
        size: f32,
        rot: f32,
        corporate: bool,
        round: bool,
        chars: Vec<char>,
        company: Vec<char>,
        /// 찍힘 불균일: 지울 점 (seal 이미지 안 좌표)
        speckles: Vec<[f32; 2]>,
    },
}

fn rect_poly(x0: f32, y0: f32, x1: f32, y1: f32) -> [[f32; 2]; 4] {
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

/// 중심·크기·회전으로 4점
fn rotated_poly(cx: f32, cy: f32, w: f32, h: f32, deg: f32) -> [[f32; 2]; 4] {
    let (s, c) = deg.to_radians().sin_cos();
    let pts = [
        [-w / 2.0, -h / 2.0],
        [w / 2.0, -h / 2.0],
        [w / 2.0, h / 2.0],
        [-w / 2.0, h / 2.0],
    ];
    pts.map(|[x, y]| [cx + x * c - y * s, cy + x * s + y * c])
}

fn bbox(points: impl Iterator<Item = [f32; 2]>, pad: f32) -> [[f32; 2]; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for [x, y] in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    rect_poly(x0 - pad, y0 - pad, x1 + pad, y1 + pad)
}

fn label_box(
    label: Label,
    owner: Owner,
    kind: MarkKind,
    page: u32,
    poly: [[f32; 2]; 4],
) -> BoxLabel {
    BoxLabel {
        span_id: None,
        attribute_id: None,
        negative_id: None,
        label: Some(label),
        owner: Some(owner),
        kind: Some(kind),
        page,
        poly,
    }
}

/// 사인(낙서형) 획: 상자 안에 3~6획, 획마다 베지어 1~3구간
fn scribble(rng: &mut SynthRng, cx: f32, cy: f32, w: f32, h: f32) -> Vec<Vec<[f32; 2]>> {
    let rnd = |rng: &mut SynthRng| {
        [
            cx + rng.random_range(-w / 2.0..=w / 2.0),
            cy + rng.random_range(-h / 2.0..=h / 2.0),
        ]
    };
    (0..rng.random_range(3..=6))
        .map(|_| {
            let mut pts = vec![rnd(rng)];
            for _ in 0..rng.random_range(1..=3) {
                pts.push(rnd(rng));
                pts.push(rnd(rng));
                pts.push(rnd(rng));
            }
            pts
        })
        .collect()
}

/// 글자 배치 (글자, x, 기준선 y)
type Glyphs = Vec<(char, f32, f32)>;
/// 4점 다각형
type Poly = [[f32; 2]; 4];

/// 흘림 이름: 붓 글꼴, 글자 높낮이·간격 흔들림. `center` = 이름 가운데 [x, y]
fn name_glyphs(
    rng: &mut SynthRng,
    fonts: &Fonts,
    font: FontKind,
    name: &str,
    px: f32,
    center: [f32; 2],
    tight: f32,
) -> (Glyphs, Poly) {
    let [cx, cy] = center;
    let chars: Vec<char> = if name.is_ascii() {
        name.split(' ')
            .next()
            .unwrap_or(name)
            .chars()
            .take(10)
            .collect()
    } else {
        name.chars().take(4).collect()
    };
    let total: f32 = chars
        .iter()
        .map(|c| fonts.advance(font, px, *c) * tight)
        .sum();
    let (asc, desc) = fonts.metrics(font, px);
    let mut x = cx - total / 2.0;
    let base = cy + (asc - desc) / 2.0;
    let mut glyphs = Vec::new();
    let mut ys = Vec::new();
    for c in chars {
        let y = base + rng.random_range(-6.0..=6.0);
        glyphs.push((c, x, y));
        ys.push(y);
        x += fonts.advance(font, px, c) * tight;
    }
    let y_min = ys.iter().copied().fold(f32::MAX, f32::min) - asc;
    let y_max = ys.iter().copied().fold(f32::MIN, f32::max) + desc;
    (
        glyphs,
        rect_poly(cx - total / 2.0, y_min, cx + total / 2.0, y_max),
    )
}

/// 서명란 하나의 표시와 정답 박스를 정한다
pub fn plan_marks(
    rng: &mut SynthRng,
    fonts: &Fonts,
    sign: &SignPlan,
    owner: Owner,
    anchor: [f32; 4],
    page: u32,
    body_px: f32,
) -> (Vec<Mark>, Vec<BoxLabel>) {
    let [ax0, ay0, ax1, ay1] = anchor;
    let h = (ay1 - ay0).max(body_px);
    let acx = (ax0 + ax1) / 2.0;
    let acy = (ay0 + ay1) / 2.0;
    let mut marks = Vec::new();
    let mut labels = Vec::new();
    let ink = *pick(rng, &[[25u8, 25, 25], [30, 60, 170], [180, 30, 30]]).unwrap_or(&[25, 25, 25]);
    // 서명은 앵커 왼쪽에서 앵커를 일부 덮도록
    let scx = ax0 - h * rng.random_range(0.2..=1.0);
    let scy = acy + rng.random_range(-0.2..=0.2) * h;
    match sign.method {
        SignMethod::NameSign => {
            let px = body_px * 1.7;
            let (glyphs, poly) = name_glyphs(
                rng,
                fonts,
                FontKind::Brush,
                &sign.signer,
                px,
                [scx, scy],
                0.85,
            );
            // 꼬리 획
            let tail_start = [poly[1][0] - 4.0, scy + h * 0.2];
            let tail = vec![
                tail_start,
                [tail_start[0] + h * 0.6, tail_start[1] + h * 0.5],
                [tail_start[0] + h * 1.2, tail_start[1] - h * 0.4],
                [tail_start[0] + h * 1.6, tail_start[1] - h * 0.1],
            ];
            let all = poly.into_iter().chain(tail.iter().copied());
            labels.push(label_box(
                Label::Signature,
                owner,
                MarkKind::NameSign,
                page,
                bbox(all, 4.0),
            ));
            marks.push(Mark::Text {
                font: FontKind::Brush,
                glyphs,
                px,
                rgb: ink,
            });
            marks.push(Mark::Strokes {
                paths: vec![tail],
                width: 3.0,
                rgb: ink,
            });
        }
        SignMethod::Scribble => {
            let paths = scribble(rng, scx, scy, h * 3.0, h * 1.3);
            let poly = bbox(paths.iter().flatten().copied(), 4.0);
            labels.push(label_box(
                Label::Signature,
                owner,
                MarkKind::Scribble,
                page,
                poly,
            ));
            marks.push(Mark::Strokes {
                paths,
                width: rng.random_range(2.5..=4.0),
                rgb: ink,
            });
        }
        SignMethod::PrintSign => {
            let px = body_px * 1.4;
            let (glyphs, name_poly) = name_glyphs(
                rng,
                fonts,
                FontKind::Pen,
                &sign.signer,
                px,
                [scx - h, scy],
                1.0,
            );
            labels.push(BoxLabel {
                span_id: None,
                attribute_id: None,
                negative_id: None,
                label: Some(Label::Name),
                owner: Some(owner),
                kind: Some(MarkKind::PrintSign),
                page,
                poly: name_poly,
            });
            let paths = scribble(rng, scx + h * 0.8, scy, h * 2.2, h * 1.1);
            labels.push(label_box(
                Label::Signature,
                owner,
                MarkKind::PrintSign,
                page,
                bbox(paths.iter().flatten().copied(), 4.0),
            ));
            marks.push(Mark::Text {
                font: FontKind::Pen,
                glyphs,
                px,
                rgb: ink,
            });
            marks.push(Mark::Strokes {
                paths,
                width: 3.0,
                rgb: ink,
            });
        }
        SignMethod::Fingerprint => {
            let s = body_px / 30.0;
            let (rx, ry) = (
                rng.random_range(26.0..=32.0) * s,
                rng.random_range(34.0..=42.0) * s,
            );
            let rot = rng.random_range(-25.0..=25.0);
            let (cx, cy) = (
                acx + rng.random_range(-10.0..=10.0),
                acy + rng.random_range(-6.0..=6.0),
            );
            let rings: Vec<Vec<f32>> = (0..rng.random_range(9..=12))
                .map(|_| {
                    (0..rng.random_range(1..=3))
                        .map(|_| rng.random_range(0.0..std::f32::consts::TAU))
                        .collect()
                })
                .collect();
            labels.push(label_box(
                Label::Fingerprint,
                owner,
                MarkKind::Fingerprint,
                page,
                rotated_poly(cx, cy, 2.0 * rx + 6.0, 2.0 * ry + 6.0, rot),
            ));
            marks.push(Mark::Fingerprint {
                cx,
                cy,
                rx,
                ry,
                rot,
                rings,
            });
        }
        SignMethod::None => {}
    }
    if let Some(kind) = sign.seal {
        let corporate = kind == MarkKind::CorporateSeal;
        let size = if corporate { h * 4.2 } else { h * 2.6 };
        let cx = acx + rng.random_range(-0.3..=0.3) * h + if corporate { h * 0.8 } else { 0.0 };
        let cy = acy + rng.random_range(-0.3..=0.3) * h;
        let rot = rng.random_range(-15.0..=15.0);
        let round = corporate || rng.random_bool(0.5);
        let mut chars: Vec<char> = if sign.signer.is_ascii() {
            sign.signer
                .chars()
                .filter(char::is_ascii_alphabetic)
                .take(3)
                .collect()
        } else {
            sign.signer.chars().take(4).collect()
        };
        match chars.len() {
            2 => chars.extend(['지', '인']),
            3 => chars.push('인'),
            _ => {}
        }
        let company: Vec<char> = sign
            .company
            .chars()
            .filter(|c| !matches!(c, '(' | ')' | ' '))
            .collect();
        let speckles = (0..rng.random_range(15..=40))
            .map(|_| [rng.random_range(0.0..size), rng.random_range(0.0..size)])
            .collect();
        labels.push(label_box(
            Label::Seal,
            owner,
            kind,
            page,
            rotated_poly(cx, cy, size, size, rot),
        ));
        marks.push(Mark::Seal {
            cx,
            cy,
            size,
            rot,
            corporate,
            round,
            chars,
            company,
            speckles,
        });
    }
    (marks, labels)
}

fn paint(rgb: [u8; 3], alpha: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(rgb[0], rgb[1], rgb[2], alpha);
    p.anti_alias = true;
    p
}

/// 표시 하나를 그린다
pub fn draw_mark(pix: &mut Pixmap, fonts: &Fonts, mark: &Mark) {
    match mark {
        Mark::Text {
            font,
            glyphs,
            px,
            rgb,
        } => {
            for (c, x, y) in glyphs {
                draw_glyph(
                    pix,
                    fonts.get(fonts.resolve(*font, *c)),
                    *c,
                    *px,
                    *x,
                    *y,
                    *rgb,
                    0.95,
                );
            }
        }
        Mark::Strokes { paths, width, rgb } => {
            for pts in paths {
                let mut pb = PathBuilder::new();
                if let Some(first) = pts.first() {
                    pb.move_to(first[0], first[1]);
                }
                for c in pts[1..].chunks_exact(3) {
                    pb.cubic_to(c[0][0], c[0][1], c[1][0], c[1][1], c[2][0], c[2][1]);
                }
                if let Some(path) = pb.finish() {
                    let stroke = Stroke {
                        width: *width,
                        line_cap: tiny_skia::LineCap::Round,
                        ..Stroke::default()
                    };
                    pix.stroke_path(
                        &path,
                        &paint(*rgb, 230),
                        &stroke,
                        Transform::identity(),
                        None,
                    );
                }
            }
        }
        Mark::Fingerprint {
            cx,
            cy,
            rx,
            ry,
            rot,
            rings,
        } => {
            let n = rings.len().max(1) as f32;
            for (k, gaps) in rings.iter().enumerate() {
                let f = (k as f32 + 1.0) / n;
                let mut pb = PathBuilder::new();
                let steps = 60;
                let mut pen_down = false;
                for i in 0..=steps {
                    let t = i as f32 / steps as f32 * std::f32::consts::TAU;
                    let gap = gaps.iter().any(|g| (t - g).abs() < 0.25);
                    let wobble = 1.0 + 0.04 * (t * 7.0 + k as f32).sin();
                    let (x, y) = (
                        cx + rx * f * wobble * t.cos(),
                        cy + ry * f * wobble * t.sin(),
                    );
                    if gap {
                        pen_down = false;
                        continue;
                    }
                    if pen_down {
                        pb.line_to(x, y);
                    } else {
                        pb.move_to(x, y);
                        pen_down = true;
                    }
                }
                if let Some(path) = pb.finish() {
                    let stroke = Stroke {
                        width: 2.0,
                        ..Stroke::default()
                    };
                    pix.stroke_path(
                        &path,
                        &paint([185, 45, 50], 170),
                        &stroke,
                        Transform::from_rotate_at(*rot, *cx, *cy),
                        None,
                    );
                }
            }
        }
        Mark::Seal {
            cx,
            cy,
            size,
            rot,
            corporate,
            round,
            chars,
            company,
            speckles,
        } => {
            let s = size.ceil() as u32 + 8;
            let Some(mut seal) = Pixmap::new(s, s) else {
                return;
            };
            let c = s as f32 / 2.0;
            let r = size / 2.0 - 2.0;
            let border = Stroke {
                width: if *corporate { 4.0 } else { 5.0 },
                ..Stroke::default()
            };
            let mut pb = PathBuilder::new();
            if *round {
                pb.push_circle(c, c, r);
                if *corporate {
                    pb.push_circle(c, c, r - 9.0);
                }
            } else if let Some(rect) = tiny_skia::Rect::from_xywh(c - r, c - r, 2.0 * r, 2.0 * r) {
                pb.push_rect(rect);
            }
            if let Some(path) = pb.finish() {
                seal.stroke_path(
                    &path,
                    &paint(SEAL_RED, 255),
                    &border,
                    Transform::identity(),
                    None,
                );
            }
            let font = fonts.get(FontKind::GothicBold);
            if *corporate {
                // 둘레: 회사 이름 (위쪽 호)
                let n = company.len().max(1);
                let px = (r * 0.28).min(2.6 * r / n as f32);
                let rt = r - 9.0 - px * 0.8;
                for (i, ch) in company.iter().enumerate() {
                    let a = (200.0 + 140.0 * (i as f32 + 0.5) / n as f32).to_radians();
                    let (x, y) = (c + rt * a.cos() - px / 2.0, c + rt * a.sin() + px / 3.0);
                    draw_glyph(&mut seal, font, *ch, px, x, y, SEAL_RED, 1.0);
                }
                // 가운데: 대표 / 이사 / 인
                let px = r * 0.3;
                for (row, text) in ["대표", "이사", "인"].iter().enumerate() {
                    let w = text.chars().count() as f32 * px;
                    let y = c - px * 0.2 + (row as f32 - 1.0) * px * 1.05 + px * 0.5;
                    for (i, ch) in text.chars().enumerate() {
                        draw_glyph(
                            &mut seal,
                            font,
                            ch,
                            px,
                            c - w / 2.0 + i as f32 * px,
                            y,
                            SEAL_RED,
                            1.0,
                        );
                    }
                }
            } else {
                // 이름 2×2 (오른쪽 열 위→아래, 왼쪽 열 위→아래)
                let px = r * 0.78;
                let order = [(1.0f32, 0.0f32), (1.0, 1.0), (0.0, 0.0), (0.0, 1.0)];
                for (ch, (col, row)) in chars.iter().zip(order) {
                    let x = c - px + col * px;
                    let y = c - px * 0.15 + row * px * 0.95;
                    draw_glyph(&mut seal, font, *ch, px, x, y, SEAL_RED, 1.0);
                }
            }
            // 찍힘 불균일: 작은 원을 지움
            for [x, y] in speckles {
                if let Some(path) = PathBuilder::from_circle(*x, *y, 2.5) {
                    let mut p = Paint::default();
                    p.set_color_rgba8(0, 0, 0, 255);
                    p.blend_mode = tiny_skia::BlendMode::Clear;
                    seal.fill_path(
                        &path,
                        &p,
                        tiny_skia::FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
            let t = Transform::from_rotate_at(*rot, *cx, *cy)
                .pre_concat(Transform::from_translate(cx - c, cy - c));
            let pp = PixmapPaint {
                opacity: 0.85,
                ..PixmapPaint::default()
            };
            pix.draw_pixmap(0, 0, seal.as_ref(), &pp, t, None);
        }
    }
}
