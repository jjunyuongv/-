//! 5단계 OCR 난이도 열화 (D1~D4). D0는 열화 없이 텍스트 레이어 PDF(`render::pdf`)
//!
//! | 난이도 | 처리 |
//! |---|---|
//! | D1 깨끗한 스캔 | 회전 ±1°, 약한 노이즈, 살짝 누런 종이, JPEG 85~92 |
//! | D2 지저분한 스캔 | 회전 ±3°, 블러, 노이즈, 누런 종이, 얼룩, 밝기 불균일, 검은 덮개 가장자리, JPEG 60~75 |
//! | D3 휴대폰 촬영 | 원근 왜곡, 책상 배경, 조명 그라데이션·그림자·색 틀어짐, 해상도 0.55~0.75배, JPEG 70~85 |
//! | D4 팩스·복사본 | 팩스 해상도(일반 200×100dpi 70% / 고화질 200×200dpi 30%) + 회전 ±1.5°, 이진화, 가로 줄무늬 2~6줄, 점 노이즈, PNG |
//!
//! - 덧칠(형광펜·"사본" 워터마크·접힌 자국)은 원본 좌표에서 먼저 그리고, 기하 변환은 **3×3 행렬 하나**로
//!   한 번에 적용한다. 같은 행렬을 박스 꼭짓점에 곱해 라벨을 옮긴다 (PLAN.md 5단계)
//! - 덧칠 비율은 `assets/synth/ratios.toml` (`overlay_*`), 변형 범위(각도·블러 등)는 PLAN 표의 값
//! - 같은 시드 → 같은 결과

use ab_glyph::{Font, PxScale, ScaleFont, point};
use image::{GrayImage, RgbImage};
use pii_core::DocLabel;
use rand::{Rng, RngExt};
use tiny_skia::Pixmap;

use crate::doc::{SynthConfig, SynthError};
use crate::render::{FontKind, Fonts};

/// 열화 난이도
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difficulty {
    D0,
    D1,
    D2,
    D3,
    D4,
}

impl Difficulty {
    pub const ALL: [Difficulty; 5] = [
        Difficulty::D0,
        Difficulty::D1,
        Difficulty::D2,
        Difficulty::D3,
        Difficulty::D4,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Difficulty::D0 => "D0",
            Difficulty::D1 => "D1",
            Difficulty::D2 => "D2",
            Difficulty::D3 => "D3",
            Difficulty::D4 => "D4",
        }
    }
}

/// 3×3 행렬 (원본 픽셀 → 결과 픽셀, 동차 좌표)
pub type Mat3 = [[f64; 3]; 3];

pub fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

pub fn mat_inv(m: &Mat3) -> Option<Mat3> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let c =
        |r1: usize, c1: usize, r2: usize, c2: usize| m[r1][c1] * m[r2][c2] - m[r1][c2] * m[r2][c1];
    Some([
        [
            c(1, 1, 2, 2) / det,
            -c(0, 1, 2, 2) / det,
            c(0, 1, 1, 2) / det,
        ],
        [
            -c(1, 0, 2, 2) / det,
            c(0, 0, 2, 2) / det,
            -c(0, 0, 1, 2) / det,
        ],
        [
            c(1, 0, 2, 1) / det,
            -c(0, 0, 2, 1) / det,
            c(0, 0, 1, 1) / det,
        ],
    ])
}

pub fn mat_apply(m: &Mat3, x: f64, y: f64) -> (f64, f64) {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    let w = if w.abs() < 1e-12 { 1e-12 } else { w };
    (
        (m[0][0] * x + m[0][1] * y + m[0][2]) / w,
        (m[1][0] * x + m[1][1] * y + m[1][2]) / w,
    )
}

fn translate(tx: f64, ty: f64) -> Mat3 {
    [[1.0, 0.0, tx], [0.0, 1.0, ty], [0.0, 0.0, 1.0]]
}

fn scale(s: f64) -> Mat3 {
    [[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, 1.0]]
}

fn scale_xy(sx: f64, sy: f64) -> Mat3 {
    [[sx, 0.0, 0.0], [0.0, sy, 0.0], [0.0, 0.0, 1.0]]
}

/// (cx, cy) 중심으로 deg도 회전
fn rotate_about(cx: f64, cy: f64, deg: f64) -> Mat3 {
    let (s, c) = deg.to_radians().sin_cos();
    let r = [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]];
    mat_mul(&translate(cx, cy), &mat_mul(&r, &translate(-cx, -cy)))
}

/// 네 점 대응으로 원근 변환 행렬 (src[i] → dst[i])
pub fn homography(src: [(f64, f64); 4], dst: [(f64, f64); 4]) -> Option<Mat3> {
    // 8×9 선형식 → 가우스 소거
    let mut a = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = src[i];
        let (u, v) = dst[i];
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for col in 0..8 {
        let piv = (col..8).max_by(|&p, &q| a[p][col].abs().total_cmp(&a[q][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        let p = a[col][col];
        for v in a[col].iter_mut() {
            *v /= p;
        }
        for r in 0..8 {
            if r != col {
                let f = a[r][col];
                if f != 0.0 {
                    let pivot_row = a[col];
                    for (v, pv) in a[r].iter_mut().zip(pivot_row.iter()) {
                        *v -= f * pv;
                    }
                }
            }
        }
    }
    let h: Vec<f64> = (0..8).map(|i| a[i][8]).collect();
    Some([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]])
}

/// 작업용 RGB 이미지 (0~255 실수)
#[derive(Debug, Clone)]
pub struct Img {
    pub w: u32,
    pub h: u32,
    pub px: Vec<[f32; 3]>,
}

impl Img {
    fn new(w: u32, h: u32, c: [f32; 3]) -> Self {
        Img {
            w,
            h,
            px: vec![c; (w * h) as usize],
        }
    }

    /// 렌더링한 쪽(흰 배경) → 작업용
    pub fn from_pixmap(p: &Pixmap) -> Self {
        let px = p
            .data()
            .chunks_exact(4)
            .map(|c| {
                // premultiplied → 흰 바탕 위 합성
                let a = 1.0 - f32::from(c[3]) / 255.0;
                [
                    f32::from(c[0]) + 255.0 * a,
                    f32::from(c[1]) + 255.0 * a,
                    f32::from(c[2]) + 255.0 * a,
                ]
            })
            .collect();
        Img {
            w: p.width(),
            h: p.height(),
            px,
        }
    }

    fn idx(&self, x: u32, y: u32) -> usize {
        (y * self.w + x) as usize
    }

    /// 양선형 보간 (밖이면 None)
    fn sample(&self, x: f64, y: f64) -> Option<[f32; 3]> {
        if x < 0.0 || y < 0.0 || x > f64::from(self.w - 1) || y > f64::from(self.h - 1) {
            return None;
        }
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = ((x - f64::from(x0)) as f32, (y - f64::from(y0)) as f32);
        let p = |xx, yy| self.px[self.idx(xx, yy)];
        let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
        let mut out = [0.0; 3];
        for k in 0..3 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bot = c[k] + (d[k] - c[k]) * fx;
            out[k] = top + (bot - top) * fy;
        }
        Some(out)
    }

    pub fn to_rgb(&self) -> RgbImage {
        let mut data = Vec::with_capacity(self.px.len() * 3);
        for p in &self.px {
            for v in p {
                data.push(v.round().clamp(0.0, 255.0) as u8);
            }
        }
        RgbImage::from_raw(self.w, self.h, data).unwrap_or_else(|| RgbImage::new(self.w, self.h))
    }

    fn luma(p: [f32; 3]) -> f32 {
        0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]
    }
}

/// 원본 → 결과 크기(ow, oh)로 원근 변환. 원본 밖은 `bg(x, y)`
fn warp(src: &Img, m: &Mat3, ow: u32, oh: u32, bg: impl Fn(u32, u32) -> [f32; 3]) -> Img {
    let inv = mat_inv(m).unwrap_or(*m);
    let mut out = Img::new(ow, oh, [255.0; 3]);
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = mat_apply(&inv, f64::from(x) + 0.5, f64::from(y) + 0.5);
            let i = out.idx(x, y);
            out.px[i] = src.sample(sx - 0.5, sy - 0.5).unwrap_or_else(|| bg(x, y));
        }
    }
    out
}

/// 분리형 가우시안 블러
fn blur(img: &mut Img, sigma: f32) {
    if sigma <= 0.05 {
        return;
    }
    let r = (sigma * 3.0).ceil() as i32;
    let k: Vec<f32> = (-r..=r)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = k.iter().sum();
    let k: Vec<f32> = k.iter().map(|v| v / sum).collect();
    let (w, h) = (img.w as i32, img.h as i32);
    let mut tmp = img.px.clone();
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for (j, kv) in k.iter().enumerate() {
                let xx = (x + j as i32 - r).clamp(0, w - 1);
                let p = img.px[(y * w + xx) as usize];
                for c in 0..3 {
                    acc[c] += p[c] * kv;
                }
            }
            tmp[(y * w + x) as usize] = acc;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for (j, kv) in k.iter().enumerate() {
                let yy = (y + j as i32 - r).clamp(0, h - 1);
                let p = tmp[(yy * w + x) as usize];
                for c in 0..3 {
                    acc[c] += p[c] * kv;
                }
            }
            img.px[(y * w + x) as usize] = acc;
        }
    }
}

/// 표준정규 난수 (Box-Muller)
fn normal<R: Rng + ?Sized>(rng: &mut R) -> f32 {
    let u1: f32 = rng.random_range(f32::EPSILON..1.0);
    let u2: f32 = rng.random_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
}

/// 픽셀 노이즈 (밝기 노이즈 + 약한 색 노이즈)
fn noise<R: Rng + ?Sized>(img: &mut Img, rng: &mut R, sigma: f32) {
    for p in img.px.iter_mut() {
        let l = normal(rng) * sigma;
        let c = normal(rng) * sigma * 0.25;
        p[0] += l + c;
        p[1] += l;
        p[2] += l - c;
    }
}

/// 픽셀마다 곱하기 (종이 색·조명)
fn multiply(img: &mut Img, f: impl Fn(u32, u32) -> [f32; 3]) {
    for y in 0..img.h {
        for x in 0..img.w {
            let m = f(x, y);
            let i = img.idx(x, y);
            for (v, k) in img.px[i].iter_mut().zip(m) {
                *v *= k;
            }
        }
    }
}

/// 잉크를 옅게(검정이 완전 검정이 아니게) — 스캔·촬영의 대비 저하
fn lift_blacks(img: &mut Img, black: f32) {
    for p in img.px.iter_mut() {
        for v in p.iter_mut() {
            *v = black + *v * (255.0 - black) / 255.0;
        }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 덧칠: 형광펜 (박스 하나 위에 노란색 곱하기, 가장자리 거칢)
fn highlighter<R: Rng + ?Sized>(img: &mut Img, rng: &mut R, rect: [f32; 4]) {
    let color = *crate::fake::pick(
        rng,
        &[[1.0, 0.96, 0.45], [0.75, 1.0, 0.6], [1.0, 0.75, 0.85]],
    )
    .unwrap_or(&[1.0, 0.96, 0.45]);
    let pad_y = rng.random_range(2.0..8.0);
    let (x0, x1) = (
        rect[0] - rng.random_range(0.0..12.0),
        rect[2] + rng.random_range(0.0..12.0),
    );
    let (y0, y1) = (rect[1] - pad_y, rect[3] + pad_y);
    let tilt = rng.random_range(-0.01..0.01);
    for y in (y0.max(0.0) as u32)..(y1.min(img.h as f32 - 1.0) as u32) {
        for x in (x0.max(0.0) as u32)..(x1.min(img.w as f32 - 1.0) as u32) {
            let dy = (x as f32 - x0) * tilt;
            let yy = y as f32 - dy;
            if yy < y0 || yy > y1 {
                continue;
            }
            let edge = smoothstep(0.0, 4.0, (yy - y0).min(y1 - yy)) * 0.85;
            let i = img.idx(x, y);
            for (v, k) in img.px[i].iter_mut().zip(color) {
                *v *= 1.0 - edge * (1.0 - k);
            }
        }
    }
}

/// 덧칠: "사본" 워터마크 (크게, 기울여, 옅게)
fn copy_watermark<R: Rng + ?Sized>(img: &mut Img, rng: &mut R, fonts: &Fonts) {
    let font = fonts.get(FontKind::GothicBold);
    let px = rng.random_range(260.0..380.0);
    let scaled = font.as_scaled(PxScale::from(px));
    let text = "사 본";
    // 기울이지 않은 글자 마스크
    let width: f32 = text
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let (mw, mh) = (
        width.ceil() as u32 + 4,
        (scaled.ascent() - scaled.descent()).ceil() as u32 + 4,
    );
    let mut mask = vec![0.0f32; (mw * mh) as usize];
    let mut pen = 2.0;
    for c in text.chars() {
        let g = font
            .glyph_id(c)
            .with_scale_and_position(PxScale::from(px), point(pen, 2.0 + scaled.ascent()));
        pen += scaled.h_advance(font.glyph_id(c));
        if let Some(o) = font.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, cov| {
                let (x, y) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if x >= 0 && y >= 0 && (x as u32) < mw && (y as u32) < mh {
                    mask[(y as u32 * mw + x as u32) as usize] = cov;
                }
            });
        }
    }
    let angle = rng.random_range(-40.0f64..-20.0);
    let (cx, cy) = (
        f64::from(img.w) * rng.random_range(0.4..0.6),
        f64::from(img.h) * rng.random_range(0.35..0.65),
    );
    // 결과 좌표 → 마스크 좌표
    let to_mask = mat_mul(
        &translate(f64::from(mw) / 2.0, f64::from(mh) / 2.0),
        &mat_mul(&rotate_about(0.0, 0.0, -angle), &translate(-cx, -cy)),
    );
    let red = rng.random_bool(0.3);
    let color = if red {
        [200.0, 40.0, 40.0]
    } else {
        [90.0, 90.0, 90.0]
    };
    let alpha = rng.random_range(0.10..0.22);
    let reach = f64::from(mw.max(mh));
    let (x0, x1) = (
        (cx - reach).max(0.0) as u32,
        ((cx + reach) as u32).min(img.w),
    );
    let (y0, y1) = (
        (cy - reach).max(0.0) as u32,
        ((cy + reach) as u32).min(img.h),
    );
    for y in y0..y1 {
        for x in x0..x1 {
            let (mx, my) = mat_apply(&to_mask, f64::from(x), f64::from(y));
            if mx < 0.0 || my < 0.0 || mx >= f64::from(mw) || my >= f64::from(mh) {
                continue;
            }
            let a = mask[(my as u32 * mw + mx as u32) as usize] * alpha;
            if a > 0.0 {
                let i = img.idx(x, y);
                for (v, k) in img.px[i].iter_mut().zip(color) {
                    *v = *v * (1.0 - a) + k * a;
                }
            }
        }
    }
}

/// 덧칠: 접힌 자국 (가로 한 줄 + 한쪽 그림자)
fn fold_mark<R: Rng + ?Sized>(img: &mut Img, rng: &mut R) {
    let base = f32::from(*crate::fake::pick(rng, &[1u8, 2]).unwrap_or(&1)) / 3.0;
    let y_fold = img.h as f32 * (base + rng.random_range(-0.03..0.03));
    let slope = rng.random_range(-0.004..0.004);
    let width = rng.random_range(12.0..40.0);
    let dark = rng.random_range(0.06..0.14);
    let below = rng.random_bool(0.5);
    for y in 0..img.h {
        for x in 0..img.w {
            let d = y as f32 - (y_fold + x as f32 * slope);
            let line = (1.0 - (d.abs() / 1.5).min(1.0)) * 0.12;
            let side = if below { d } else { -d };
            let shade = if side > 0.0 {
                dark * (1.0 - smoothstep(0.0, width, side))
            } else {
                -dark * 0.3 * (1.0 - smoothstep(0.0, width * 0.5, -side))
            };
            let m = 1.0 - line - shade;
            if (m - 1.0).abs() > 1e-4 {
                let i = img.idx(x, y);
                for c in 0..3 {
                    img.px[i][c] *= m;
                }
            }
        }
    }
}

/// 열화 결과 쪽 이미지
pub enum PageImage {
    /// JPEG로 저장 (품질)
    Jpeg(RgbImage, u8),
    /// PNG 흑백 (팩스)
    Png(GrayImage),
}

impl PageImage {
    pub fn ext(&self) -> &'static str {
        match self {
            PageImage::Jpeg(..) => "jpg",
            PageImage::Png(_) => "png",
        }
    }

    pub fn size(&self) -> (u32, u32) {
        match self {
            PageImage::Jpeg(i, _) => i.dimensions(),
            PageImage::Png(i) => i.dimensions(),
        }
    }

    /// 파일 바이트 (JPEG 압축 손실도 열화의 일부)
    pub fn encode(&self) -> Result<Vec<u8>, SynthError> {
        let mut out = std::io::Cursor::new(Vec::new());
        let r = match self {
            PageImage::Jpeg(img, q) => {
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, *q).encode_image(img)
            }
            PageImage::Png(img) => img.write_to(&mut out, image::ImageFormat::Png),
        };
        r.map_err(|e| SynthError::Assemble(format!("쪽 이미지 저장 실패: {e}")))?;
        Ok(out.into_inner())
    }
}

/// 쪽 하나를 열화하고 그 쪽 박스를 같은 변환으로 옮긴다
pub fn degrade_page<R: Rng + ?Sized>(
    cfg: &SynthConfig,
    fonts: &Fonts,
    page: &Pixmap,
    page_no: u32,
    label: &mut DocLabel,
    level: Difficulty,
    rng: &mut R,
) -> Result<PageImage, SynthError> {
    let mut img = Img::from_pixmap(page);
    let (w, h) = (f64::from(img.w), f64::from(img.h));

    // 덧칠 (원본 좌표, D1~D4)
    if level != Difficulty::D0 {
        if cfg.choose(rng, "overlay_highlighter", ["yes", "no"])? == 0 {
            let spans: Vec<[f32; 4]> = label
                .boxes
                .iter()
                .filter(|b| b.page == page_no && b.kind.is_none())
                .map(|b| {
                    let xs = b.poly.iter().map(|p| p[0]);
                    let ys = b.poly.iter().map(|p| p[1]);
                    [
                        xs.clone().fold(f32::MAX, f32::min),
                        ys.clone().fold(f32::MAX, f32::min),
                        xs.fold(f32::MIN, f32::max),
                        ys.fold(f32::MIN, f32::max),
                    ]
                })
                .collect();
            for _ in 0..rng.random_range(1..=2u8) {
                if let Some(r) = crate::fake::pick(rng, &spans) {
                    highlighter(&mut img, rng, *r);
                }
            }
        }
        if cfg.choose(rng, "overlay_copy_watermark", ["yes", "no"])? == 0 {
            copy_watermark(&mut img, rng, fonts);
        }
        if cfg.choose(rng, "overlay_fold", ["yes", "no"])? == 0 {
            fold_mark(&mut img, rng);
        }
    }

    let (m, out) = match level {
        Difficulty::D0 => (scale(1.0), PageOut::Same),
        Difficulty::D1 => {
            let m = rotate_about(w / 2.0, h / 2.0, rng.random_range(-1.0..1.0));
            let paper = [
                rng.random_range(0.97..1.0),
                rng.random_range(0.96..0.99),
                rng.random_range(0.92..0.97),
            ];
            let mut o = warp(&img, &m, img.w, img.h, |_, _| [250.0, 250.0, 248.0]);
            lift_blacks(&mut o, rng.random_range(8.0..20.0));
            multiply(&mut o, |_, _| paper);
            blur(&mut o, rng.random_range(0.3..0.6));
            let sigma = rng.random_range(2.0..4.0);
            noise(&mut o, rng, sigma);
            (m, PageOut::Jpeg(o, rng.random_range(85..=92)))
        }
        Difficulty::D2 => {
            let m = rotate_about(w / 2.0, h / 2.0, rng.random_range(-3.0..3.0));
            let lid = rng.random_range(40.0..110.0);
            let mut o = warp(&img, &m, img.w, img.h, |_, _| [lid, lid, lid]);
            lift_blacks(&mut o, rng.random_range(20.0..45.0));
            // 누런 종이 + 밝기 불균일 (한쪽이 어두움)
            let paper = [
                rng.random_range(0.97..1.0),
                rng.random_range(0.94..0.98),
                rng.random_range(0.84..0.93),
            ];
            let (gx, gy) = (
                rng.random_range(-1.0..1.0f32),
                rng.random_range(-1.0..1.0f32),
            );
            let depth = rng.random_range(0.08..0.18);
            let (ow, oh) = (o.w as f32, o.h as f32);
            multiply(&mut o, |x, y| {
                let t = ((x as f32 / ow - 0.5) * gx + (y as f32 / oh - 0.5) * gy + 1.0) / 2.0;
                let k = 1.0 - depth * t.clamp(0.0, 1.0);
                [paper[0] * k, paper[1] * k, paper[2] * k]
            });
            // 얼룩 (커피·물 자국)
            for _ in 0..rng.random_range(2..=5u8) {
                let (sx, sy) = (rng.random_range(0.0..ow), rng.random_range(0.0..oh));
                let (rx, ry) = (
                    rng.random_range(30.0..160.0f32),
                    rng.random_range(30.0..160.0f32),
                );
                let a = rng.random_range(0.10..0.25);
                let tint = [0.92, 0.82, 0.62];
                multiply(&mut o, |x, y| {
                    let d = ((x as f32 - sx) / rx).powi(2) + ((y as f32 - sy) / ry).powi(2);
                    if d > 1.3 {
                        return [1.0; 3];
                    }
                    // 가장자리가 더 진한 물 자국
                    let ring =
                        a * (1.0 - smoothstep(0.7, 1.3, d)) * (0.5 + 0.5 * smoothstep(0.3, 1.0, d));
                    [
                        1.0 - ring * (1.0 - tint[0]) * 3.0,
                        1.0 - ring * (1.0 - tint[1]) * 3.0,
                        1.0 - ring * (1.0 - tint[2]) * 3.0,
                    ]
                });
            }
            blur(&mut o, rng.random_range(0.8..1.4));
            let sigma = rng.random_range(6.0..10.0);
            noise(&mut o, rng, sigma);
            (m, PageOut::Jpeg(o, rng.random_range(60..=75)))
        }
        Difficulty::D3 => {
            // 해상도 0.55~0.75배, 종이가 화면의 대부분 + 원근 (모서리 흔들림 최대 5%)
            let s = rng.random_range(0.55..0.75);
            let margin = rng.random_range(0.04..0.10);
            let (pw, ph) = (w * s, h * s);
            let (ow, oh) = (
                (pw * (1.0 + 2.0 * margin)) as u32,
                (ph * (1.0 + 2.0 * margin)) as u32,
            );
            let (mx, my) = (pw * margin, ph * margin);
            let j = |rng: &mut R, d: f64| rng.random_range(-0.05..0.05) * d;
            let dst = [
                (mx + j(rng, pw), my + j(rng, ph)),
                (mx + pw + j(rng, pw), my + j(rng, ph)),
                (mx + pw + j(rng, pw), my + ph + j(rng, ph)),
                (mx + j(rng, pw), my + ph + j(rng, ph)),
            ];
            let m = homography([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)], dst)
                .ok_or_else(|| SynthError::Assemble("원근 변환 실패".into()))?;
            // 축소 전 앨리어싱 방지
            blur(&mut img, (0.45 / s) as f32);
            let desk = *crate::fake::pick(
                rng,
                &[
                    [120.0, 85.0, 55.0],
                    [70.0, 70.0, 75.0],
                    [200.0, 200.0, 195.0],
                    [40.0, 40.0, 45.0],
                ],
            )
            .unwrap_or(&[120.0, 85.0, 55.0]);
            let mut o = warp(&img, &m, ow, oh, |x, y| {
                let t = ((x + y) % 7) as f32 - 3.0;
                [desk[0] + t, desk[1] + t, desk[2] + t]
            });
            lift_blacks(&mut o, rng.random_range(25.0..50.0));
            // 조명: 한 방향 그라데이션 + 비네팅 + 색 틀어짐 + 그림자(한쪽 반평면)
            let (fw, fh) = (o.w as f32, o.h as f32);
            let ang = rng.random_range(0.0..std::f32::consts::TAU);
            let (dx, dy) = (ang.cos(), ang.sin());
            let grad = rng.random_range(0.15..0.30);
            let cast = *crate::fake::pick(
                rng,
                &[[1.0, 0.97, 0.88], [0.92, 0.96, 1.0], [1.0, 1.0, 0.97]],
            )
            .unwrap_or(&[1.0, 1.0, 1.0]);
            let sh_ang = rng.random_range(0.0..std::f32::consts::TAU);
            let (sdx, sdy) = (sh_ang.cos(), sh_ang.sin());
            let sh_off = rng.random_range(0.15..0.40);
            let sh_a = rng.random_range(0.20..0.40);
            let bright = rng.random_range(0.95..1.05);
            multiply(&mut o, |x, y| {
                let (u, v) = (x as f32 / fw - 0.5, y as f32 / fh - 0.5);
                let g = 1.0 - grad * (0.5 + u * dx + v * dy).clamp(0.0, 1.0);
                let vig = 1.0 - 0.25 * (u * u + v * v);
                let sd = u * sdx + v * sdy - sh_off;
                let sh = 1.0 - sh_a * smoothstep(-0.03, 0.05, sd);
                let k = g * vig * sh * bright;
                [cast[0] * k, cast[1] * k, cast[2] * k]
            });
            blur(&mut o, rng.random_range(0.6..1.2));
            let sigma = rng.random_range(3.0..6.0);
            noise(&mut o, rng, sigma);
            (m, PageOut::Jpeg(o, rng.random_range(70..=85)))
        }
        Difficulty::D4 => {
            // 실제 팩스 해상도: 일반 모드 가로 200 × 세로 100dpi / 고화질 200 × 200dpi (fax_resolution).
            // 일반 모드는 세로를 반으로 줄여 이진화한 뒤, 팩스 뷰어처럼 줄을 두 번씩 반복해 정사각 픽셀로 되돌린다
            let normal = cfg.choose(rng, "fax_resolution", ["normal", "fine"])? == 0;
            let sy = if normal { 0.5 } else { 1.0 };
            let m = mat_mul(
                &scale_xy(1.0, sy),
                &rotate_about(w / 2.0, h / 2.0, rng.random_range(-1.5..1.5)),
            );
            let (ow, oh) = (w as u32, (h * sy) as u32);
            blur(&mut img, 0.5);
            let mut o = warp(&img, &m, ow, oh, |_, _| [255.0; 3]);
            let sigma = rng.random_range(8.0..14.0);
            noise(&mut o, rng, sigma);
            blur(&mut o, 0.5);
            let t = rng.random_range(150.0..185.0);
            let mut g: Vec<u8> =
                o.px.iter()
                    .map(|p| if Img::luma(*p) < t { 0 } else { 255 })
                    .collect();
            // 가로 줄무늬: 검은 선(끊김 있음) + 흰 빠짐 줄
            for _ in 0..rng.random_range(2..=6u8) {
                let y = rng.random_range(0..oh);
                let thick = rng.random_range(1..=2u32);
                let black = rng.random_bool(0.6);
                let (mut x, mut on) = (0u32, true);
                while x < ow {
                    let run = rng.random_range(20..300u32);
                    if on {
                        for xx in x..(x + run).min(ow) {
                            for yy in y..(y + thick).min(oh) {
                                g[(yy * ow + xx) as usize] = if black { 0 } else { 255 };
                            }
                        }
                    }
                    x += run;
                    on = !on || rng.random_bool(0.3);
                }
            }
            // 점 노이즈
            let density = rng.random_range(0.0005..0.002);
            for v in g.iter_mut() {
                if rng.random_bool(density) {
                    *v = if *v == 0 { 255 } else { 0 };
                }
            }
            // 일반 모드: 줄 반복으로 세로 2배 (박스도 같은 배율)
            let (g, oh, m) = if normal {
                let row = ow as usize;
                let mut doubled = Vec::with_capacity(g.len() * 2);
                for r in g.chunks_exact(row) {
                    doubled.extend_from_slice(r);
                    doubled.extend_from_slice(r);
                }
                (doubled, oh * 2, mat_mul(&scale_xy(1.0, 2.0), &m))
            } else {
                (g, oh, m)
            };
            let gray = GrayImage::from_raw(ow, oh, g)
                .ok_or_else(|| SynthError::Assemble("팩스 이미지 크기".into()))?;
            (m, PageOut::Gray(gray))
        }
    };

    // 같은 변환을 그 쪽 박스에
    for b in label.boxes.iter_mut().filter(|b| b.page == page_no) {
        for p in b.poly.iter_mut() {
            let (x, y) = mat_apply(&m, f64::from(p[0]), f64::from(p[1]));
            *p = [x as f32, y as f32];
        }
    }
    Ok(match out {
        PageOut::Same => PageImage::Jpeg(img.to_rgb(), 95),
        PageOut::Jpeg(o, q) => PageImage::Jpeg(o.to_rgb(), q),
        PageOut::Gray(g) => PageImage::Png(g),
    })
}

enum PageOut {
    Same,
    Jpeg(Img, u8),
    Gray(GrayImage),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homography_maps_corners_and_inverts() {
        let src = [(0.0, 0.0), (100.0, 0.0), (100.0, 200.0), (0.0, 200.0)];
        let dst = [(10.0, 5.0), (95.0, 12.0), (105.0, 190.0), (3.0, 210.0)];
        let m = homography(src, dst).unwrap();
        for (s, d) in src.iter().zip(dst) {
            let (x, y) = mat_apply(&m, s.0, s.1);
            assert!((x - d.0).abs() < 1e-6 && (y - d.1).abs() < 1e-6);
        }
        let inv = mat_inv(&m).unwrap();
        let (x, y) = mat_apply(&inv, 10.0, 5.0);
        assert!(x.abs() < 1e-6 && y.abs() < 1e-6);
    }

    #[test]
    fn rotation_keeps_center() {
        let m = rotate_about(50.0, 80.0, 2.5);
        let (x, y) = mat_apply(&m, 50.0, 80.0);
        assert!((x - 50.0).abs() < 1e-9 && (y - 80.0).abs() < 1e-9);
    }
}
