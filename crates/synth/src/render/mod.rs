//! A4 200dpi 페이지 렌더링 + 정답 박스 (PLAN.md 4단계).
//!
//! - 조립된 문서([`ResolvedDoc`])를 [`walk_text`]와 같은 순서로 배치 → 글자마다 문서 문자 오프셋을 안다
//! - 필드(span·참고 항목·네거티브) 박스 = 그 글자 박스를 **줄마다** 합친 사각형 (줄이 바뀌면 박스 여러 개)
//! - 손글씨 필드는 나눔손글씨 펜, 잉크색·세로 흔들림. 서명·도장·지장은 [`marks`]
//! - 좌표: 픽셀, 원점 왼쪽 위. `poly`는 4점 (5단계 기하 열화 후에도 같은 변환을 점에 적용)

pub mod marks;
pub mod pdf;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use pii_core::{BoxLabel, DocLabel};
use rand::RngExt;
use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::doc::{FieldKind, RBlock, RCell, RText, ResolvedDoc, TextLoc, text_label, walk_text};
use crate::fake::{SynthRng, pick, seeded};
use crate::template::Align;

/// 200dpi A4
pub const PAGE_W: u32 = 1654;
pub const PAGE_H: u32 = 2339;
const MARGIN_X: f32 = 150.0;
const MARGIN_TOP: f32 = 160.0;
const MARGIN_BOTTOM: f32 = 160.0;
/// 본문 글자 크기 (10pt @ 200dpi ≈ 28px)
const BODY_PX: f32 = 28.0;
const TITLE_PX: f32 = 44.0;
const TABLE_PX: f32 = 22.0;
const LINE_GAP: f32 = 1.45;
/// 표 안 줄 간격 (원본 서식처럼 촘촘하게)
const TABLE_LINE_GAP: f32 = 1.3;
const INDENT_PX: f32 = 40.0;
/// 손글씨 크기 (인쇄 글자 대비 눈에 보이는 크기, 실제 손글씨는 인쇄보다 크게 쓴다)
const HAND_SIZE_MIN: f32 = 1.2;
const HAND_SIZE_MAX: f32 = 1.5;
/// 문단에서 손글씨가 더 올라가는 높이 중 줄 간격으로 흡수하는 몫 (나머지는 위로 넘침)
const HAND_LINE_ROOM: f32 = 0.5;
const CELL_PAD: f32 = 7.0;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("폰트 파일을 읽을 수 없습니다: {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("폰트 형식 오류: {0}")]
    Font(String),
    #[error("페이지 이미지를 만들 수 없습니다")]
    Pixmap,
    #[error("PNG 저장 실패: {0}")]
    Png(String),
    #[error("{0}")]
    Pdf(String),
}

/// 폰트 묶음 (`assets/fonts`, 모두 OFL)
pub struct Fonts {
    pub gothic: FontArc,
    pub gothic_bold: FontArc,
    pub myeongjo: FontArc,
    pub pen: FontArc,
    pub brush: FontArc,
    /// 나눔 글꼴에 없는 글자(㎡ 등)를 원래 글자 그대로 그리는 대체 글꼴 (Noto Sans CJK KR 한국어 부분 집합)
    pub noto: FontArc,
    /// Noto Sans CJK에도 없는 문장 부호(U+2024 ONE DOT LEADER 등)용 Noto Sans
    pub noto_latin: FontArc,
    /// 펜 글꼴 한글을 고딕과 같은 눈에 보이는 크기로 맞추는 배율 (펜 글꼴은 같은 px에서 글자가 작다)
    pub pen_scale: f32,
    dir: PathBuf,
}

/// 한글 글자의 실제 잉크 높이 / 글꼴 높이(ascent - descent) — 같은 px에서 눈에 보이는 크기 비교용
fn hangul_ink_ratio(font: &FontArc) -> Option<f32> {
    // PxScale은 ascent - descent = px 기준이므로 100px에서 잰 잉크 높이 / 100 = 비율
    const PX: f32 = 100.0;
    let hs: Vec<f32> = "가나다라한글근로계약"
        .chars()
        .filter_map(|c| font.outline_glyph(font.glyph_id(c).with_scale(PX)))
        .map(|o| {
            let b = o.px_bounds();
            b.max.y - b.min.y
        })
        .collect();
    if hs.is_empty() {
        return None;
    }
    Some(hs.iter().sum::<f32>() / hs.len() as f32 / PX)
}

impl Fonts {
    pub fn load(dir: &Path) -> Result<Self, RenderError> {
        let load = |name: &str| -> Result<FontArc, RenderError> {
            let path = dir.join(name);
            let bytes = std::fs::read(&path).map_err(|source| RenderError::Io {
                path: path.display().to_string(),
                source,
            })?;
            FontArc::try_from_vec(bytes).map_err(|e| RenderError::Font(format!("{name}: {e}")))
        };
        let gothic = load(FontKind::Gothic.file_name())?;
        let pen = load(FontKind::Pen.file_name())?;
        let pen_scale = match (hangul_ink_ratio(&gothic), hangul_ink_ratio(&pen)) {
            (Some(g), Some(p)) if p > 0.0 => g / p,
            _ => return Err(RenderError::Font("한글 글자 크기를 잴 수 없음".into())),
        };
        Ok(Fonts {
            gothic,
            gothic_bold: load(FontKind::GothicBold.file_name())?,
            myeongjo: load(FontKind::Myeongjo.file_name())?,
            pen,
            brush: load(FontKind::Brush.file_name())?,
            noto: load(FontKind::Noto.file_name())?,
            noto_latin: load(FontKind::NotoLatin.file_name())?,
            pen_scale,
            dir: dir.to_path_buf(),
        })
    }

    /// 글꼴 파일 경로 (D0 PDF에 원본 글꼴을 부분 집합으로 넣을 때)
    pub fn path(&self, f: FontKind) -> PathBuf {
        self.dir.join(f.file_name())
    }

    pub fn get(&self, f: FontKind) -> &FontArc {
        match f {
            FontKind::Gothic => &self.gothic,
            FontKind::GothicBold => &self.gothic_bold,
            FontKind::Myeongjo => &self.myeongjo,
            FontKind::Pen => &self.pen,
            FontKind::Brush => &self.brush,
            FontKind::Noto => &self.noto,
            FontKind::NotoLatin => &self.noto_latin,
        }
    }

    /// 글자가 없는 폰트면 고딕 → 명조 → Noto Sans CJK KR → Noto Sans 순으로 대신 쓴다 (원래 글자 그대로 → 이미지 = 정답 텍스트)
    pub fn resolve(&self, f: FontKind, c: char) -> FontKind {
        for k in [
            f,
            FontKind::Gothic,
            FontKind::Myeongjo,
            FontKind::Noto,
            FontKind::NotoLatin,
        ] {
            if self.get(k).glyph_id(c).0 != 0 {
                return k;
            }
        }
        f
    }

    /// 어느 글꼴로도 그릴 수 없는 글자인가
    pub fn missing(&self, c: char) -> bool {
        self.get(self.resolve(FontKind::Gothic, c)).glyph_id(c).0 == 0
    }

    pub fn advance(&self, f: FontKind, px: f32, c: char) -> f32 {
        let k = self.resolve(f, c);
        let font = self.get(k).as_scaled(PxScale::from(px));
        font.h_advance(font.glyph_id(c))
    }

    /// (ascent, descent) 픽셀. descent는 양수로
    pub fn metrics(&self, f: FontKind, px: f32) -> (f32, f32) {
        let font = self.get(f).as_scaled(PxScale::from(px));
        (font.ascent(), -font.descent())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontKind {
    Gothic,
    GothicBold,
    Myeongjo,
    Pen,
    Brush,
    Noto,
    NotoLatin,
}

impl FontKind {
    pub const ALL: [FontKind; 7] = [
        FontKind::Gothic,
        FontKind::GothicBold,
        FontKind::Myeongjo,
        FontKind::Pen,
        FontKind::Brush,
        FontKind::Noto,
        FontKind::NotoLatin,
    ];

    /// `assets/fonts/` 안 파일 이름
    pub fn file_name(self) -> &'static str {
        match self {
            FontKind::Gothic => "NanumGothic-Regular.ttf",
            FontKind::GothicBold => "NanumGothic-Bold.ttf",
            FontKind::Myeongjo => "NanumMyeongjo-Regular.ttf",
            FontKind::Pen => "NanumPenScript-Regular.ttf",
            FontKind::Brush => "NanumBrushScript-Regular.ttf",
            FontKind::Noto => "NotoSansKR-Regular.otf",
            FontKind::NotoLatin => "NotoSans-Regular.ttf",
        }
    }
}

/// 그릴 것
#[derive(Debug, Clone)]
enum Item {
    Glyph {
        font: FontKind,
        ch: char,
        x: f32,
        y: f32,
        px: f32,
        rgb: [u8; 3],
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke: f32,
    },
    Mark(marks::Mark),
}

/// 글자 박스 (문서 문자 오프셋별)
#[derive(Debug, Clone, Copy)]
struct CharBox {
    page: u32,
    line: u32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

/// 렌더링 결과
pub struct RenderedDoc {
    pub pages: Vec<Pixmap>,
    pub label: DocLabel,
}

/// 문서 렌더링 상태
struct Layouter<'f> {
    fonts: &'f Fonts,
    rng: SynthRng,
    body: FontKind,
    ink: [u8; 3],
    pages: Vec<Vec<Item>>,
    y: f32,
    line_id: u32,
    boxes: BTreeMap<usize, CharBox>,
    marks: Vec<BoxLabel>,
}

/// 배치할 글자 하나
#[derive(Debug, Clone, Copy)]
struct Run {
    ch: char,
    doc: usize,
    font: FontKind,
    px: f32,
    rgb: [u8; 3],
    dy: f32,
    adv: f32,
}

impl<'f> Layouter<'f> {
    fn page(&self) -> u32 {
        self.pages.len().saturating_sub(1) as u32
    }

    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = MARGIN_TOP;
    }

    fn push(&mut self, item: Item) {
        if let Some(p) = self.pages.last_mut() {
            p.push(item);
        }
    }

    fn bottom() -> f32 {
        PAGE_H as f32 - MARGIN_BOTTOM
    }

    /// 조각의 글자들을 글꼴·색과 함께 준비 (손글씨 필드는 펜 글꼴·잉크색·흔들림).
    /// 손글씨는 눈에 보이는 크기가 인쇄 글자의 1.2~1.5배 (필드마다 무작위). 기준선은 인쇄 줄과 같아서
    /// 칸이 좁아도 글자를 줄이지 않고 위로 조금 넘친다
    fn runs(&mut self, t: &RText, base: usize, font: FontKind, px: f32) -> Vec<Run> {
        let mut hand_dy: BTreeMap<usize, (f32, f32)> = BTreeMap::new();
        for (i, f) in t.fields.iter().enumerate() {
            if f.hand {
                let size = self.rng.random_range(HAND_SIZE_MIN..=HAND_SIZE_MAX);
                let dy = self.rng.random_range(-3.0..=3.0);
                hand_dy.insert(i, (px * size * self.fonts.pen_scale, dy));
            }
        }
        t.text
            .chars()
            .enumerate()
            .map(|(i, ch)| {
                let field = t
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| i >= f.start && i < f.end);
                let (fk, fpx, rgb, dy) = match field {
                    Some((fi, f)) if f.hand => {
                        let (hpx, dy) = hand_dy.get(&fi).copied().unwrap_or((px, 0.0));
                        (FontKind::Pen, hpx, self.ink, dy)
                    }
                    _ => (font, px, [20, 20, 20], 0.0),
                };
                let fk = self.fonts.resolve(fk, ch);
                Run {
                    ch,
                    doc: base + i,
                    font: fk,
                    px: fpx,
                    rgb,
                    dy,
                    adv: self.fonts.advance(fk, fpx, ch),
                }
            })
            .collect()
    }

    /// 너비 안에서 줄 나누기 (띄어쓰기 단위, 긴 단어는 글자 단위). `\n`은 강제 줄바꿈
    fn wrap(runs: &[Run], width: f32) -> Vec<Vec<Run>> {
        let mut lines: Vec<Vec<Run>> = Vec::new();
        let mut cur: Vec<Run> = Vec::new();
        let mut cur_w = 0.0;
        let mut word: Vec<Run> = Vec::new();
        let mut word_w = 0.0;
        let flush_word = |lines: &mut Vec<Vec<Run>>,
                          cur: &mut Vec<Run>,
                          cur_w: &mut f32,
                          word: &mut Vec<Run>,
                          word_w: &mut f32| {
            if *cur_w + *word_w > width && !cur.is_empty() {
                lines.push(std::mem::take(cur));
                *cur_w = 0.0;
            }
            for r in word.drain(..) {
                if *cur_w + r.adv > width && !cur.is_empty() {
                    lines.push(std::mem::take(cur));
                    *cur_w = 0.0;
                }
                *cur_w += r.adv;
                cur.push(r);
            }
            *word_w = 0.0;
        };
        for r in runs {
            if r.ch == '\n' {
                flush_word(&mut lines, &mut cur, &mut cur_w, &mut word, &mut word_w);
                lines.push(std::mem::take(&mut cur));
                cur_w = 0.0;
                continue;
            }
            word_w += r.adv;
            word.push(*r);
            if r.ch == ' ' {
                flush_word(&mut lines, &mut cur, &mut cur_w, &mut word, &mut word_w);
            }
        }
        flush_word(&mut lines, &mut cur, &mut cur_w, &mut word, &mut word_w);
        if !cur.is_empty() || lines.is_empty() {
            lines.push(cur);
        }
        lines
    }

    fn line_height(px: f32) -> f32 {
        px * LINE_GAP
    }

    /// 줄 하나를 (x, 기준선 y)에 놓고 글자 박스를 기록
    fn place_line(&mut self, line: &[Run], x: f32, baseline: f32) {
        self.line_id += 1;
        let page = self.page();
        let mut pen = x;
        for r in line {
            let (asc, desc) = self.fonts.metrics(r.font, r.px);
            let y = baseline + r.dy;
            self.boxes.insert(
                r.doc,
                CharBox {
                    page,
                    line: self.line_id,
                    x0: pen,
                    y0: y - asc,
                    x1: pen + r.adv,
                    y1: y + desc,
                },
            );
            // 띄어쓰기도 글자로 둔다 (이미지에는 아무것도 안 그려지고, D0 PDF에는 공백 글리프가 들어가 텍스트 추출에 띄어쓰기가 나옴)
            if r.ch == ' ' || !r.ch.is_whitespace() {
                self.push(Item::Glyph {
                    font: r.font,
                    ch: r.ch,
                    x: pen,
                    y,
                    px: r.px,
                    rgb: r.rgb,
                });
            }
            pen += r.adv;
        }
    }

    /// 문단 (쪽 넘김 포함)
    fn paragraph(
        &mut self,
        t: &RText,
        base: usize,
        indent: u8,
        align: Align,
        font: FontKind,
        px: f32,
    ) {
        let x0 = MARGIN_X + f32::from(indent) * INDENT_PX;
        let width = PAGE_W as f32 - MARGIN_X - x0;
        let runs = self.runs(t, base, font, px);
        let lh = Self::line_height(px);
        for line in Self::wrap(&runs, width) {
            if self.y + lh > Self::bottom() {
                self.new_page();
            }
            let w: f32 = line.iter().map(|r| r.adv).sum();
            let x = match align {
                Align::Left => x0,
                Align::Center => x0 + (width - w) / 2.0,
                Align::Right => x0 + width - w,
            };
            // 손글씨가 인쇄 글자보다 높이 올라가는 만큼의 절반은 줄 간격을 늘리고, 나머지 절반은 위로 넘치게 둔다
            // (윗줄 글자를 덮지 않을 만큼만 넘침. 표 칸 안은 칸 높이를 바꾸지 않고 위로 넘침)
            let print_asc = self.fonts.metrics(font, px).0;
            let rise = line
                .iter()
                .map(|r| self.fonts.metrics(r.font, r.px).0 - r.dy - print_asc)
                .fold(0.0f32, f32::max);
            self.y += rise * HAND_LINE_ROOM;
            self.place_line(&line, x, self.y + px);
            self.y += lh;
        }
    }

    /// 표: 열 폭 비율, rowspan·colspan. rowspan으로 묶인 행 묶음은 쪽을 넘기지 않음
    fn table(
        &mut self,
        widths: &[f32],
        rows: &[Vec<RCell>],
        bases: &BTreeMap<(usize, usize), usize>,
    ) {
        let content_w = PAGE_W as f32 - 2.0 * MARGIN_X;
        let ncols = if widths.is_empty() {
            rows.iter()
                .map(|r| r.iter().map(|c| usize::from(c.colspan)).sum::<usize>())
                .max()
                .unwrap_or(1)
        } else {
            widths.len()
        };
        let col_w: Vec<f32> = if widths.is_empty() {
            vec![content_w / ncols as f32; ncols]
        } else {
            widths.iter().map(|w| w * content_w).collect()
        };
        // 셀을 격자에 놓기 (HTML 표와 같은 방식)
        struct Placed {
            r: usize,
            c: usize,
            col: usize,
            rowspan: usize,
            colspan: usize,
        }
        let mut occupied: BTreeMap<(usize, usize), bool> = BTreeMap::new();
        let mut placed = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let mut col = 0;
            for (c, cell) in row.iter().enumerate() {
                while occupied.contains_key(&(r, col)) {
                    col += 1;
                }
                let rs = usize::from(cell.rowspan.max(1));
                let cs = usize::from(cell.colspan.max(1));
                for dr in 0..rs {
                    for dc in 0..cs {
                        occupied.insert((r + dr, col + dc), true);
                    }
                }
                placed.push(Placed {
                    r,
                    c,
                    col,
                    rowspan: rs,
                    colspan: cs,
                });
                col += cs;
            }
        }
        let nrows = rows.len();
        let lh = TABLE_PX * TABLE_LINE_GAP;
        // 셀마다 필요한 높이
        let mut cell_lines = BTreeMap::new();
        for p in &placed {
            let w: f32 = col_w[p.col.min(ncols - 1)..(p.col + p.colspan).min(ncols)]
                .iter()
                .sum::<f32>()
                - 2.0 * CELL_PAD;
            let base = bases.get(&(p.r, p.c)).copied().unwrap_or(0);
            let runs = self.runs(&rows[p.r][p.c].text, base, self.body, TABLE_PX);
            let lines = Self::wrap(&runs, w.max(10.0));
            cell_lines.insert((p.r, p.c), lines);
        }
        let mut row_h = vec![lh + 2.0 * CELL_PAD; nrows];
        for p in placed.iter().filter(|p| p.rowspan == 1) {
            let need = cell_lines[&(p.r, p.c)].len() as f32 * lh + 2.0 * CELL_PAD;
            row_h[p.r] = row_h[p.r].max(need);
        }
        for p in placed.iter().filter(|p| p.rowspan > 1) {
            let need = cell_lines[&(p.r, p.c)].len() as f32 * lh + 2.0 * CELL_PAD;
            let end = (p.r + p.rowspan).min(nrows);
            let have: f32 = row_h[p.r..end].iter().sum();
            if need > have {
                row_h[end - 1] += need - have;
            }
        }
        // rowspan으로 묶인 행 묶음
        let mut group_end = vec![0usize; nrows];
        for (r, g) in group_end.iter_mut().enumerate() {
            let mut end = r + 1;
            for p in placed.iter().filter(|p| p.r <= r && p.r + p.rowspan > r) {
                end = end.max(p.r + p.rowspan);
            }
            *g = end.min(nrows);
        }
        let x0 = MARGIN_X;
        let mut row_y = vec![0.0f32; nrows];
        let mut row_page = vec![0u32; nrows];
        let mut r = 0;
        while r < nrows {
            let mut end = group_end[r];
            let mut k = r;
            while k < end {
                end = end.max(group_end[k]);
                k += 1;
            }
            let h: f32 = row_h[r..end].iter().sum();
            if self.y + h > Self::bottom() && self.y > MARGIN_TOP {
                self.new_page();
            }
            for i in r..end {
                row_y[i] = self.y;
                row_page[i] = self.page();
                self.y += row_h[i];
            }
            r = end;
        }
        // 셀 그리기
        for p in &placed {
            let x = x0 + col_w[..p.col.min(ncols)].iter().sum::<f32>();
            let w: f32 = col_w[p.col.min(ncols - 1)..(p.col + p.colspan).min(ncols)]
                .iter()
                .sum();
            let end = (p.r + p.rowspan).min(nrows);
            let h: f32 = row_h[p.r..end].iter().sum();
            let y = row_y[p.r];
            let page = row_page[p.r] as usize;
            if let Some(items) = self.pages.get_mut(page) {
                items.push(Item::Rect {
                    x,
                    y,
                    w,
                    h,
                    stroke: 2.0,
                });
            }
            let lines = cell_lines.remove(&(p.r, p.c)).unwrap_or_default();
            let saved_pages = self.pages.len();
            // 셀 글자는 그 행의 쪽에 놓는다
            let mut ly = y + CELL_PAD;
            for line in &lines {
                self.line_id += 1;
                let line_id = self.line_id;
                let mut pen = x + CELL_PAD;
                for run in line {
                    let (asc, desc) = self.fonts.metrics(run.font, run.px);
                    let gy = ly + TABLE_PX + run.dy;
                    self.boxes.insert(
                        run.doc,
                        CharBox {
                            page: page as u32,
                            line: line_id,
                            x0: pen,
                            y0: gy - asc,
                            x1: pen + run.adv,
                            y1: gy + desc,
                        },
                    );
                    if !run.ch.is_whitespace()
                        && let Some(items) = self.pages.get_mut(page)
                    {
                        items.push(Item::Glyph {
                            font: run.font,
                            ch: run.ch,
                            x: pen,
                            y: gy,
                            px: run.px,
                            rgb: run.rgb,
                        });
                    }
                    pen += run.adv;
                }
                ly += lh;
            }
            debug_assert_eq!(saved_pages, self.pages.len());
        }
        self.y += lh * 0.4;
    }
}

/// 배치 결과: 쪽별로 그릴 것 + 정답 라벨(박스 포함). 이미지(`render`)와 D0 PDF(`pdf::write_pdf`)가 같은 배치를 쓴다
pub struct LaidOut {
    pages: Vec<Vec<Item>>,
    pub label: DocLabel,
}

/// 문서 하나를 배치한다 (같은 시드면 같은 배치)
pub fn layout(doc: &ResolvedDoc, fonts: &Fonts) -> LaidOut {
    let mut rng = seeded(doc.seed ^ 0x9E37_79B9_7F4A_7C15);
    let body =
        *pick(&mut rng, &[FontKind::Myeongjo, FontKind::Gothic]).unwrap_or(&FontKind::Myeongjo);
    let ink = *pick(&mut rng, &[[30u8, 60, 170], [25, 25, 25]]).unwrap_or(&[30, 60, 170]);
    // 외국인 양식(레이아웃 C)은 한·영 병기라 원본 글자가 작다 → 촘촘한 크기 (앞·뒤 2쪽에 맞춤)
    let (body_px, title_px) = if doc.layout == crate::template::Layout::C {
        (24.0, 38.0)
    } else {
        (BODY_PX, TITLE_PX)
    };
    let mut l = Layouter {
        fonts,
        rng,
        body,
        ink,
        pages: vec![Vec::new()],
        y: MARGIN_TOP,
        line_id: 0,
        boxes: BTreeMap::new(),
        marks: Vec::new(),
    };

    // 조각별 문서 오프셋 (텍스트 라벨과 같은 순서)
    let mut block_base: BTreeMap<usize, usize> = BTreeMap::new();
    let mut cell_base: BTreeMap<usize, BTreeMap<(usize, usize), usize>> = BTreeMap::new();
    let mut line_base: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    walk_text(doc, |_, base, loc| match loc {
        TextLoc::Block(b) => {
            block_base.insert(b, base);
        }
        TextLoc::Cell { block, row, col } => {
            cell_base.entry(block).or_default().insert((row, col), base);
        }
        TextLoc::SignLine { block, line } => {
            line_base.insert((block, line), base);
        }
    });

    for (b, block) in doc.blocks.iter().enumerate() {
        let base = block_base.get(&b).copied().unwrap_or(0);
        match block {
            RBlock::Title(t) => {
                l.y += 10.0;
                l.paragraph(t, base, 0, Align::Center, FontKind::GothicBold, title_px);
                l.y += 20.0;
            }
            RBlock::Paragraph {
                text,
                indent,
                align,
            } => {
                l.paragraph(text, base, *indent, *align, body, body_px);
            }
            RBlock::Table { widths, rows } => {
                let empty = BTreeMap::new();
                let bases = cell_base.get(&b).unwrap_or(&empty);
                l.y += 8.0;
                l.table(widths, rows, bases);
            }
            RBlock::Signature {
                owner,
                anchor,
                lines,
                sign,
            } => {
                l.y += 10.0;
                // 서명란은 쪽 경계에서 갈라지지 않게 한 쪽에 모은다 (연속된 서명란 블록 묶음의 첫 블록에서 판단)
                let first_of_group =
                    b == 0 || !matches!(doc.blocks[b - 1], RBlock::Signature { .. });
                let group_lines: usize = doc.blocks[b..]
                    .iter()
                    .take_while(|x| matches!(x, RBlock::Signature { .. }))
                    .map(|x| match x {
                        RBlock::Signature { lines, .. } => lines.len(),
                        _ => 0,
                    })
                    .sum();
                let need = group_lines as f32 * (body_px * LINE_GAP + body_px * 0.5) + 10.0;
                if first_of_group && l.y + need > Layouter::bottom() {
                    l.new_page();
                }
                let mut anchor_box: Option<(u32, [f32; 4])> = None;
                for (i, line) in lines.iter().enumerate() {
                    let base = line_base.get(&(b, i)).copied().unwrap_or(0);
                    // 서명란은 줄 간격을 넓혀 서명·도장 자리를 둔다
                    l.paragraph(line, base, 2, Align::Left, body, body_px);
                    l.y += body_px * 0.5;
                    if let Some(pos) = line.text.rfind(anchor.as_str()) {
                        let start = line.text[..pos].chars().count();
                        let n = anchor.chars().count();
                        let bs: Vec<CharBox> = (start..start + n)
                            .filter_map(|i| l.boxes.get(&(base + i)).copied())
                            .collect();
                        if let Some(first) = bs.first() {
                            let x0 = bs.iter().map(|c| c.x0).fold(f32::MAX, f32::min);
                            let y0 = bs.iter().map(|c| c.y0).fold(f32::MAX, f32::min);
                            let x1 = bs.iter().map(|c| c.x1).fold(f32::MIN, f32::max);
                            let y1 = bs.iter().map(|c| c.y1).fold(f32::MIN, f32::max);
                            anchor_box = Some((first.page, [x0, y0, x1, y1]));
                        }
                    }
                }
                if let Some((page, abox)) = anchor_box {
                    let (items, labels) =
                        marks::plan_marks(&mut l.rng, fonts, sign, *owner, abox, page, body_px);
                    if let Some(p) = l.pages.get_mut(page as usize) {
                        p.extend(items.into_iter().map(Item::Mark));
                    }
                    l.marks.extend(labels);
                }
            }
            RBlock::PageBreak => l.new_page(),
        }
    }

    // 정답 라벨 + 박스
    let mut label = text_label(doc);
    label.pages = l.pages.len() as u32;
    let mut boxes = Vec::new();
    let field_boxes = |start: usize, end: usize| -> Vec<(u32, [f32; 4])> {
        let mut by_line: BTreeMap<u32, (u32, [f32; 4])> = BTreeMap::new();
        for i in start..end {
            if let Some(c) = l.boxes.get(&i) {
                let e = by_line
                    .entry(c.line)
                    .or_insert((c.page, [c.x0, c.y0, c.x1, c.y1]));
                e.1 = [
                    e.1[0].min(c.x0),
                    e.1[1].min(c.y0),
                    e.1[2].max(c.x1),
                    e.1[3].max(c.y1),
                ];
            }
        }
        by_line.into_values().collect()
    };
    let poly = |r: [f32; 4]| [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    for s in &label.spans {
        for (page, r) in field_boxes(s.start.0, s.end.0) {
            boxes.push(BoxLabel {
                span_id: Some(s.id),
                attribute_id: None,
                negative_id: None,
                label: Some(s.label),
                owner: s.owner,
                kind: None,
                page,
                poly: poly(r),
            });
        }
    }
    for a in &label.attributes {
        for (page, r) in field_boxes(a.start.0, a.end.0) {
            boxes.push(BoxLabel {
                span_id: None,
                attribute_id: Some(a.id),
                negative_id: None,
                label: None,
                owner: None,
                kind: None,
                page,
                poly: poly(r),
            });
        }
    }
    for n in &label.negatives {
        for (page, r) in field_boxes(n.start.0, n.end.0) {
            boxes.push(BoxLabel {
                span_id: None,
                attribute_id: None,
                negative_id: Some(n.id),
                label: None,
                owner: None,
                kind: None,
                page,
                poly: poly(r),
            });
        }
    }
    boxes.extend(l.marks.clone());
    label.boxes = boxes;
    LaidOut {
        pages: l.pages,
        label,
    }
}

/// 문서 하나를 렌더링한다 (같은 시드면 같은 이미지)
pub fn render(doc: &ResolvedDoc, fonts: &Fonts) -> Result<RenderedDoc, RenderError> {
    let laid = layout(doc, fonts);
    let mut pages = Vec::with_capacity(laid.pages.len());
    for items in &laid.pages {
        let mut pix = Pixmap::new(PAGE_W, PAGE_H).ok_or(RenderError::Pixmap)?;
        pix.fill(Color::WHITE);
        for item in items {
            match item {
                Item::Glyph {
                    font,
                    ch,
                    x,
                    y,
                    px,
                    rgb,
                } => draw_glyph(&mut pix, fonts.get(*font), *ch, *px, *x, *y, *rgb, 1.0),
                Item::Rect { x, y, w, h, stroke } => {
                    stroke_rect(&mut pix, *x, *y, *w, *h, *stroke, [0, 0, 0])
                }
                Item::Mark(m) => marks::draw_mark(&mut pix, fonts, m),
            }
        }
        pages.push(pix);
    }
    Ok(RenderedDoc {
        pages,
        label: laid.label,
    })
}

/// 글자 하나를 그린다 (커버리지를 색으로 섞음)
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_glyph(
    pix: &mut Pixmap,
    font: &FontArc,
    ch: char,
    px: f32,
    x: f32,
    y: f32,
    rgb: [u8; 3],
    alpha: f32,
) {
    let glyph = font
        .glyph_id(ch)
        .with_scale_and_position(PxScale::from(px), point(x, y));
    if let Some(outline) = font.outline_glyph(glyph) {
        let b = outline.px_bounds();
        let (ox, oy) = (b.min.x as i32, b.min.y as i32);
        outline.draw(|gx, gy, c| blend(pix, ox + gx as i32, oy + gy as i32, rgb, c * alpha));
    }
}

/// 픽셀 하나에 색을 섞는다 (premultiplied RGBA)
pub(crate) fn blend(pix: &mut Pixmap, x: i32, y: i32, rgb: [u8; 3], a: f32) {
    let (w, h) = (pix.width() as i32, pix.height() as i32);
    if x < 0 || y < 0 || x >= w || y >= h || a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let i = ((y * w + x) * 4) as usize;
    let data = pix.data_mut();
    for (k, c) in rgb.iter().enumerate() {
        let dst = f32::from(data[i + k]);
        data[i + k] = (f32::from(*c) * a + dst * (1.0 - a)).round() as u8;
    }
    let da = f32::from(data[i + 3]) / 255.0;
    data[i + 3] = ((a + da * (1.0 - a)) * 255.0).round() as u8;
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stroke_rect(
    pix: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    width: f32,
    rgb: [u8; 3],
) {
    if let Some(rect) = Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)) {
        let path = PathBuilder::from_rect(rect);
        let mut paint = Paint::default();
        paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
        paint.anti_alias = true;
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        pix.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// 확인용 오버레이: span 빨강, 참고 항목 파랑, 네거티브 초록, 서명·도장·지장 보라
pub fn overlay(page: &Pixmap, label: &DocLabel, page_no: u32) -> Pixmap {
    let mut out = page.clone();
    for b in label.boxes.iter().filter(|b| b.page == page_no) {
        let rgb = if b.kind.is_some() {
            [170, 0, 200]
        } else if b.span_id.is_some() {
            [230, 20, 20]
        } else if b.attribute_id.is_some() {
            [20, 90, 230]
        } else {
            [20, 160, 60]
        };
        let mut pb = PathBuilder::new();
        pb.move_to(b.poly[0][0], b.poly[0][1]);
        for p in &b.poly[1..] {
            pb.line_to(p[0], p[1]);
        }
        pb.close();
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
            paint.anti_alias = true;
            let stroke = Stroke {
                width: 3.0,
                ..Stroke::default()
            };
            out.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
    out
}

/// PNG 바이트
pub fn png(pix: &Pixmap) -> Result<Vec<u8>, RenderError> {
    pix.encode_png()
        .map_err(|e| RenderError::Png(e.to_string()))
}

/// 박스가 텍스트 필드 범위를 모두 덮는지 확인할 때 쓰는 헬퍼 (테스트용 공개)
pub fn is_label_field(k: &FieldKind) -> bool {
    !matches!(k, FieldKind::Plain | FieldKind::Check { .. })
}
