//! `pii-mask forms extract` — 표준근로계약서 원본 PDF에서 쪽별 텍스트(글자 위치 포함)와
//! 쪽별 PNG를 뽑고, 양식 제목을 기준으로 7종을 나눈다 (PLAN.md 1-1).
//!
//! 입력은 한컴 뷰어에서 뽑은 PDF(`assets/forms/reference/`)다. 양식 원본이라 개인정보가 없으므로
//! 추출 텍스트를 그대로 파일에 저장한다.

mod layout;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use image::{Rgb, RgbImage};
use pdfium_render::prelude::*;
use serde::Serialize;

/// 기본 입력 PDF (작업 폴더 기준 상대 경로)
pub const DEFAULT_INPUT: &str = "assets/forms/reference/표준_근로계약서_원본.pdf";
/// 기본 출력 폴더
pub const DEFAULT_OUT: &str = "assets/forms/extracted";
/// pdfium DLL 기본 위치 (작업 폴더 기준). `PDFIUM_DYNAMIC_LIB_PATH`가 있으면 그쪽을 먼저 본다.
const DEFAULT_PDFIUM_DIR: &str = "vendor/pdfium/bin";
/// PDF 단위(pt) → 인치
const POINTS_PER_INCH: f32 = 72.0;

/// 양식 제목 표. `pattern`은 문자(한글·영문)만 남긴 형태로 비교한다 (`letters_only`).
/// 한컴 PDF는 괄호·숫자의 텍스트 순서가 흔들리므로 문자만 비교하는 편이 안정적이다.
/// 같은 위치에서 여러 제목이 맞으면 표에서 앞에 있는 것을 고르므로 더 구체적인 제목을 앞에 둔다.
///
/// 외국인근로자 양식 2종은 굵은 한글 제목("표준근로계약서")이 PDF에서 곡선(벡터)으로 그려져
/// 텍스트가 없다 → 영문 제목으로 구분한다.
///
/// 문서는 8개이고, `layouts`는 PLAN.md 1-2의 레이아웃(A~F) 매핑이다.
const FORM_TITLES: &[FormTitle] = &[
    FormTitle {
        slug: "standard_indefinite",
        title: "표준근로계약서(기간의 정함이 없는 경우)",
        pattern: "표준근로계약서기간의정함이없는경우",
        layouts: &["A", "B"],
    },
    FormTitle {
        slug: "standard_fixed_term",
        title: "표준근로계약서(기간의 정함이 있는 경우)",
        pattern: "표준근로계약서기간의정함이있는경우",
        layouts: &["A", "B"],
    },
    FormTitle {
        slug: "minor",
        title: "연소근로자(18세 미만인 자) 표준근로계약서",
        pattern: "연소근로자세미만인자표준근로계약서",
        layouts: &["D"],
    },
    // 항목·서명란 소속(친권자)이 달라 별도 문서·템플릿으로 둔다
    FormTitle {
        slug: "guardian_consent",
        title: "친권자(후견인) 동의서",
        pattern: "친권자후견인동의서",
        layouts: &["D"],
    },
    FormTitle {
        slug: "construction_daily",
        title: "건설일용근로자 표준근로계약서",
        pattern: "건설일용근로자표준근로계약서",
        layouts: &["F"],
    },
    FormTitle {
        slug: "part_time",
        title: "단시간근로자 표준근로계약서",
        pattern: "단시간근로자표준근로계약서",
        layouts: &["E"],
    },
    FormTitle {
        slug: "foreign_worker_agri",
        title: "외국인근로자 표준근로계약서(농업ㆍ축산업ㆍ어업 분야) [별지 제6호의2서식]",
        pattern: "StandardLaborContractForAgricultureLivestockandFisherySectors",
        layouts: &["C"],
    },
    FormTitle {
        slug: "foreign_worker",
        title: "외국인근로자 표준근로계약서 [별지 제6호서식]",
        pattern: "StandardLaborContract",
        layouts: &["C"],
    },
];

/// 쪽 첫머리에서 제목을 찾을 범위 (`letters_only` 적용 후 글자 수)
const TITLE_SEARCH_WINDOW: usize = 150;
/// 이 글자 수보다 적으면 "텍스트 없음" 쪽으로 본다 (공백 제외)
const MIN_TEXT_CHARS_PER_PAGE: usize = 20;

#[derive(Debug, Clone, Copy)]
pub struct FormTitle {
    pub slug: &'static str,
    pub title: &'static str,
    pub pattern: &'static str,
    /// 이 문서를 쓰는 레이아웃 (PLAN.md 1-2)
    pub layouts: &'static [&'static str],
}

/// `forms extract` 옵션
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub input: PathBuf,
    pub out: PathBuf,
    pub dpi: f32,
    /// 글자 박스를 PNG 위에 그린 확인용 이미지도 저장
    pub overlay: bool,
}

/// 쪽별 글자 위치 파일 (`pages/page_XX.chars.json`)
#[derive(Debug, Serialize)]
struct PageChars {
    /// 1부터 시작하는 쪽 번호
    page: usize,
    width_pt: f32,
    height_pt: f32,
    dpi: f32,
    width_px: u32,
    height_px: u32,
    /// `bbox_px` 좌표계: PNG와 같은 픽셀 좌표, 원점 왼쪽 위, `[x0, y0, x1, y1]`
    chars: Vec<CharBox>,
}

#[derive(Debug, Serialize)]
struct CharBox {
    /// `page_XX.txt` 안의 문자(char) 오프셋
    offset: usize,
    ch: char,
    /// pdfium이 끼워 넣은 공백·줄바꿈이면 true (박스 없음)
    generated: bool,
    font_size_pt: f32,
    bbox_px: Option<[f32; 4]>,
}

/// 쪽 하나의 추출 결과 (메모리 내)
#[derive(Debug)]
struct PageExtract {
    text: String,
    /// 공백이 아닌 글자 수
    text_chars: usize,
    /// 사용자 영역(PUA)·대체 문자 수 — 글꼴 매핑이 깨졌는지 판단용
    suspicious_chars: usize,
    /// 유니코드로 바꾸지 못해 건너뛴 글자 수
    unmapped_chars: usize,
}

/// 쪽별 텍스트 상태
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextStatus {
    /// 정상 텍스트
    Ok,
    /// 글자가 거의 없음 (이미지로 인쇄됐을 가능성)
    Empty,
    /// 글자는 있지만 PUA·대체 문자가 많아 내용이 깨짐
    Garbled,
}

#[derive(Debug, Serialize)]
struct PageSummary {
    page: usize,
    text_status: TextStatus,
    text_chars: usize,
    suspicious_chars: usize,
    unmapped_chars: usize,
    png: String,
    txt: String,
    chars_json: String,
}

/// 양식 하나가 차지하는 쪽 범위
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormSpan {
    pub slug: String,
    pub title: String,
    /// 1부터 시작하는 쪽 번호
    pub pages: Vec<usize>,
    /// 이 문서를 쓰는 레이아웃 (PLAN.md 1-2)
    pub layouts: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Index {
    source: String,
    page_count: usize,
    dpi: f32,
    text_status: TextStatus,
    pages: Vec<PageSummary>,
    forms: Vec<FormSpan>,
    /// 어느 양식 제목에도 속하지 않은 앞쪽 쪽들
    unassigned_pages: Vec<usize>,
}

/// `pii-mask forms extract` 본체
pub fn extract(opts: &ExtractOptions) -> Result<()> {
    if !opts.input.is_file() {
        bail!("입력 PDF가 없습니다: {}", opts.input.display());
    }
    if !(opts.dpi > 0.0 && opts.dpi <= 600.0) {
        bail!("dpi는 0 초과 600 이하여야 합니다: {}", opts.dpi);
    }

    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(&opts.input, None)
        .with_context(|| format!("PDF를 열 수 없습니다: {}", opts.input.display()))?;

    let pages_dir = opts.out.join("pages");
    let forms_dir = opts.out.join("forms");
    fs::create_dir_all(&pages_dir)
        .with_context(|| format!("출력 폴더를 만들 수 없습니다: {}", pages_dir.display()))?;
    fs::create_dir_all(&forms_dir)
        .with_context(|| format!("출력 폴더를 만들 수 없습니다: {}", forms_dir.display()))?;

    let scale = opts.dpi / POINTS_PER_INCH;
    let render_config = PdfRenderConfig::new().scale_page_by_factor(scale);

    let page_count = usize::try_from(document.pages().len()).context("쪽 수가 음수입니다")?;
    let mut extracts = Vec::with_capacity(page_count);
    let mut summaries = Vec::with_capacity(page_count);

    for (idx, page) in document.pages().iter().enumerate() {
        let page_no = idx + 1;
        let stem = format!("page_{page_no:02}");

        // 1) PNG 렌더링 — 텍스트가 없어도 PNG는 남긴다 (옮겨 적기 대비)
        let image = page
            .render_with_config(&render_config)
            .with_context(|| format!("{page_no}쪽 렌더링 실패"))?
            .as_image()
            .with_context(|| format!("{page_no}쪽 이미지 변환 실패"))?
            .to_rgb8();
        let png_path = pages_dir.join(format!("{stem}.png"));
        image
            .save(&png_path)
            .with_context(|| format!("PNG 저장 실패: {}", png_path.display()))?;

        // 2) 글자 + 위치
        let (extract, raw_text, chars) = extract_page_text(&page, page_no, scale)?;
        let width_pt = page.width().value;
        let height_pt = page.height().value;
        let page_chars = PageChars {
            page: page_no,
            width_pt,
            height_pt,
            dpi: opts.dpi,
            width_px: image.width(),
            height_px: image.height(),
            chars,
        };

        let txt_path = pages_dir.join(format!("{stem}.txt"));
        write_text(&txt_path, &extract.text)?;
        write_text(&pages_dir.join(format!("{stem}.raw.txt")), &raw_text)?;
        let json_path = pages_dir.join(format!("{stem}.chars.json"));
        write_json(&json_path, &page_chars)?;

        if opts.overlay {
            let overlay_path = pages_dir.join(format!("{stem}.overlay.png"));
            let overlay = draw_char_boxes(image, &page_chars.chars);
            overlay
                .save(&overlay_path)
                .with_context(|| format!("PNG 저장 실패: {}", overlay_path.display()))?;
        }

        let status = classify_page(&extract);
        summaries.push(PageSummary {
            page: page_no,
            text_status: status,
            text_chars: extract.text_chars,
            suspicious_chars: extract.suspicious_chars,
            unmapped_chars: extract.unmapped_chars,
            png: format!("pages/{stem}.png"),
            txt: format!("pages/{stem}.txt"),
            chars_json: format!("pages/{stem}.chars.json"),
        });
        extracts.push(extract);
    }

    let page_texts: Vec<&str> = extracts.iter().map(|e| e.text.as_str()).collect();
    let doc_status = overall_status(summaries.iter().map(|s| s.text_status));

    // 전체 텍스트 (쪽 구분선 포함)
    let mut all = String::new();
    for (i, text) in page_texts.iter().enumerate() {
        all.push_str(&format!("===== page {:02} =====\n", i + 1));
        all.push_str(text);
        if !text.ends_with('\n') {
            all.push('\n');
        }
    }
    write_text(&opts.out.join("all.txt"), &all)?;

    let (forms, unassigned_pages) = split_forms(&page_texts, FORM_TITLES);
    for form in &forms {
        let mut body = String::new();
        for &p in &form.pages {
            body.push_str(&format!("===== page {p:02} =====\n"));
            let text = page_texts.get(p - 1).copied().unwrap_or_default();
            body.push_str(text);
            if !text.ends_with('\n') {
                body.push('\n');
            }
        }
        write_text(&forms_dir.join(format!("{}.txt", form.slug)), &body)?;
    }

    let index = Index {
        source: opts.input.display().to_string().replace('\\', "/"),
        page_count,
        dpi: opts.dpi,
        text_status: doc_status,
        pages: summaries,
        forms,
        unassigned_pages,
    };
    write_json(&opts.out.join("index.json"), &index)?;

    print_summary(&index, &opts.out);

    if doc_status != TextStatus::Ok {
        let bad: Vec<String> = index
            .pages
            .iter()
            .filter(|p| p.text_status != TextStatus::Ok)
            .map(|p| format!("{}쪽({:?})", p.page, p.text_status))
            .collect();
        bail!(
            "PDF 텍스트 추출이 정상이 아닙니다: {}. PNG는 저장했으니 쪽 이미지를 보고 옮겨 적는 방식으로 바꿔야 합니다.",
            bad.join(", ")
        );
    }
    Ok(())
}

/// pdfium DLL 연결. 순서: `PDFIUM_DYNAMIC_LIB_PATH` → `vendor/pdfium/bin` → 시스템 경로
fn bind_pdfium() -> Result<Pdfium> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("PDFIUM_DYNAMIC_LIB_PATH") {
        dirs.push(PathBuf::from(dir));
    }
    dirs.push(PathBuf::from(DEFAULT_PDFIUM_DIR));

    let mut tried = Vec::new();
    for dir in dirs {
        let lib = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if !lib.is_file() {
            tried.push(format!("{} (없음)", lib.display()));
            continue;
        }
        match Pdfium::bind_to_library(&lib) {
            Ok(bindings) => return Ok(Pdfium::new(bindings)),
            Err(e) => tried.push(format!("{} ({e})", lib.display())),
        }
    }
    match Pdfium::bind_to_system_library() {
        Ok(bindings) => Ok(Pdfium::new(bindings)),
        Err(e) => bail!(
            "pdfium 라이브러리를 찾지 못했습니다. 시도: {}; 시스템 경로 ({e}). \
             PLAN.md 1-1의 pdfium 설치 안내를 따라 vendor\\pdfium\\bin\\pdfium.dll을 두세요.",
            tried.join(", ")
        ),
    }
}

/// 위치가 있는 글자 하나 (읽기 순서 재구성 전)
#[derive(Debug, Clone, Copy)]
struct PlacedChar {
    c: char,
    /// 픽셀 좌표 `[x0, y0, x1, y1]`, 원점 왼쪽 위
    bbox: [f32; 4],
    font_size_pt: f32,
}

impl PlacedChar {
    fn center_y(&self) -> f32 {
        (self.bbox[1] + self.bbox[3]) / 2.0
    }
    fn height(&self) -> f32 {
        self.bbox[3] - self.bbox[1]
    }
}

/// 쪽 하나의 글자와 위치를 읽고, 위치 기준 읽기 순서로 텍스트를 재구성한다.
///
/// 한컴 뷰어가 만든 PDF는 문장부호·숫자가 텍스트 스트림에서 줄 끝으로 밀려 있다
/// (예: `표준근로계약서 기간의 정함이 없는 경우 ( )`). 글자 박스 위치는 정확하므로
/// 박스 위치로 다시 정렬한다: 표 밖은 줄(y)·x 순서, 표 안은 칸 단위 (`layout` 모듈).
/// 공백·줄바꿈은 글자 간격으로 새로 만든다.
/// 텍스트를 글자 목록에서 직접 만들므로 `CharBox::offset`이 곧 문자 오프셋이다.
/// 두 번째 반환값은 pdfium 스트림 순서 그대로의 텍스트(참고용)다.
fn extract_page_text(
    page: &PdfPage,
    page_no: usize,
    scale: f32,
) -> Result<(PageExtract, String, Vec<CharBox>)> {
    let text_page = page
        .text()
        .with_context(|| format!("{page_no}쪽 텍스트 레이어를 열 수 없습니다"))?;
    let height_pt = page.height().value;

    let mut raw_text = String::new();
    let mut placed = Vec::new();
    let mut unmapped_chars = 0;

    for ch in text_page.chars().iter() {
        let Some(c) = ch.unicode_char() else {
            unmapped_chars += 1;
            continue;
        };
        // pdfium은 줄바꿈을 "\r\n"으로 넣는다 → '\n'만 남긴다
        if c == '\r' {
            continue;
        }
        raw_text.push(c);
        if c.is_whitespace() || ch.is_generated().unwrap_or(false) {
            continue;
        }
        match ch.loose_bounds() {
            Ok(r) => placed.push(PlacedChar {
                c,
                bbox: [
                    round1(r.left().value * scale),
                    round1((height_pt - r.top().value) * scale),
                    round1(r.right().value * scale),
                    round1((height_pt - r.bottom().value) * scale),
                ],
                font_size_pt: round1(ch.scaled_font_size().value),
            }),
            // 위치를 모르는 글자는 읽기 순서에 넣을 수 없다 → 변환불가로 센다
            Err(_) => unmapped_chars += 1,
        }
    }

    let text_chars = placed.len();
    let suspicious_chars = placed.iter().filter(|p| is_suspicious(p.c)).count();
    let tables = layout::find_tables(page, scale);
    let lines = layout::reading_order(placed, &tables, scale);
    let (text, boxes) = lines_to_text(&lines, scale);

    Ok((
        PageExtract {
            text,
            text_chars,
            suspicious_chars,
            unmapped_chars,
        },
        raw_text,
        boxes,
    ))
}

/// 글자를 줄로 묶는다. 세로 중심 순으로 훑으면서, 지금 줄에서 가장 키 큰 글자의 세로 범위 안에
/// 중심이 들어오면 같은 줄로 본다. 줄 안은 x 순으로 정렬한다.
fn group_lines(mut chars: Vec<PlacedChar>) -> Vec<Vec<PlacedChar>> {
    chars.sort_by(|a, b| {
        a.center_y()
            .total_cmp(&b.center_y())
            .then(a.bbox[0].total_cmp(&b.bbox[0]))
    });
    let mut lines: Vec<Vec<PlacedChar>> = Vec::new();
    let mut band: Option<(f32, f32, f32)> = None; // (top, bottom, height)
    for ch in chars {
        let cy = ch.center_y();
        match band {
            Some((top, bottom, h)) if cy >= top && cy <= bottom => {
                if let Some(line) = lines.last_mut() {
                    line.push(ch);
                }
                if ch.height() > h {
                    band = Some((ch.bbox[1], ch.bbox[3], ch.height()));
                }
            }
            _ => {
                band = Some((ch.bbox[1], ch.bbox[3], ch.height()));
                lines.push(vec![ch]);
            }
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
    }
    lines
}

/// 줄 목록을 텍스트와 글자 박스로 바꾼다. 글자 사이 간격이 글자 크기의 1/4을 넘으면 공백 하나,
/// 줄 끝에는 줄바꿈을 넣는다 (둘 다 `generated: true`, 박스 없음).
fn lines_to_text(lines: &[Vec<PlacedChar>], scale: f32) -> (String, Vec<CharBox>) {
    let mut text = String::new();
    let mut boxes = Vec::new();
    let mut push = |text: &mut String, c: char, font_size_pt: f32, bbox_px: Option<[f32; 4]>| {
        boxes.push(CharBox {
            offset: boxes.len(),
            ch: c,
            generated: bbox_px.is_none(),
            font_size_pt,
            bbox_px,
        });
        text.push(c);
    };
    for line in lines {
        let mut prev: Option<&PlacedChar> = None;
        for ch in line {
            if let Some(p) = prev {
                let gap = ch.bbox[0] - p.bbox[2];
                let size_px = p.font_size_pt.max(ch.font_size_pt) * scale;
                if gap > size_px * 0.25 {
                    push(&mut text, ' ', 0.0, None);
                }
            }
            push(&mut text, ch.c, ch.font_size_pt, Some(ch.bbox));
            prev = Some(ch);
        }
        push(&mut text, '\n', 0.0, None);
    }
    (text, boxes)
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// 글꼴 매핑이 깨졌을 때 흔히 나오는 문자: 사용자 영역(PUA), 대체 문자, 제어 문자
fn is_suspicious(c: char) -> bool {
    matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{FFFD}' | '\u{F0000}'..='\u{10FFFF}')
        || (c.is_control() && c != '\n' && c != '\t')
}

fn classify_page(extract: &PageExtract) -> TextStatus {
    if extract.text_chars < MIN_TEXT_CHARS_PER_PAGE {
        TextStatus::Empty
    } else if extract.suspicious_chars * 10 > extract.text_chars {
        // 공백 제외 글자의 10% 넘게 깨졌으면 내용을 믿을 수 없다
        TextStatus::Garbled
    } else {
        TextStatus::Ok
    }
}

/// 문서 전체 상태: 하나라도 Empty면 Empty, 아니고 하나라도 Garbled면 Garbled
fn overall_status(statuses: impl Iterator<Item = TextStatus>) -> TextStatus {
    let mut result = TextStatus::Ok;
    for s in statuses {
        match s {
            TextStatus::Empty => return TextStatus::Empty,
            TextStatus::Garbled => result = TextStatus::Garbled,
            TextStatus::Ok => {}
        }
    }
    result
}

/// 문자(한글·영문 등 alphabetic)만 남긴다 (제목 비교용). 공백·문장부호·숫자는 지운다.
fn letters_only(s: &str) -> String {
    s.chars().filter(|c| c.is_alphabetic()).collect()
}

/// 쪽 첫머리에서 양식 제목을 찾는다. 가장 앞에 나온 제목을 고르고,
/// 같은 위치면 표에서 앞에 있는 제목을 고른다 (`min_by_key`는 동률이면 첫 번째를 돌려준다).
fn detect_title<'a>(page_text: &str, titles: &'a [FormTitle]) -> Option<&'a FormTitle> {
    let head: String = letters_only(page_text)
        .chars()
        .take(TITLE_SEARCH_WINDOW)
        .collect();
    titles
        .iter()
        .filter_map(|t| head.find(t.pattern).map(|pos| (pos, t)))
        .min_by_key(|(pos, _)| *pos)
        .map(|(_, t)| t)
}

/// 쪽마다 제목을 찾아 양식 경계로 삼는다. 제목이 없는 쪽은 앞 양식에 이어 붙인다.
/// 같은 제목이 다시 나오면 `_2`, `_3`을 붙여 별도 양식으로 둔다.
pub fn split_forms(page_texts: &[&str], titles: &[FormTitle]) -> (Vec<FormSpan>, Vec<usize>) {
    let mut forms: Vec<FormSpan> = Vec::new();
    // forms와 같은 순서의 양식 종류(FormTitle::slug) — `_2` 접미사 없이 비교하기 위함
    let mut kinds: Vec<&str> = Vec::new();
    let mut unassigned = Vec::new();
    for (i, text) in page_texts.iter().enumerate() {
        let page_no = i + 1;
        if let Some(t) = detect_title(text, titles) {
            let seen = kinds.iter().filter(|k| **k == t.slug).count();
            kinds.push(t.slug);
            let slug = if seen == 0 {
                t.slug.to_string()
            } else {
                format!("{}_{}", t.slug, seen + 1)
            };
            forms.push(FormSpan {
                slug,
                title: t.title.to_string(),
                pages: vec![page_no],
                layouts: t.layouts.iter().map(|l| l.to_string()).collect(),
            });
        } else if let Some(last) = forms.last_mut() {
            last.pages.push(page_no);
        } else {
            unassigned.push(page_no);
        }
    }
    (forms, unassigned)
}

/// 확인용: 글자 박스 테두리를 빨간색으로 그린다
fn draw_char_boxes(mut image: RgbImage, chars: &[CharBox]) -> RgbImage {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return image;
    }
    let red = Rgb([230, 30, 30]);
    let clamp = |v: f32, max: u32| -> u32 { (v.max(0.0) as u32).min(max - 1) };
    for b in chars.iter().filter_map(|c| c.bbox_px) {
        let (x0, y0, x1, y1) = (
            clamp(b[0], w),
            clamp(b[1], h),
            clamp(b[2], w),
            clamp(b[3], h),
        );
        for x in x0..=x1 {
            image.put_pixel(x, y0, red);
            image.put_pixel(x, y1, red);
        }
        for y in y0..=y1 {
            image.put_pixel(x0, y, red);
            image.put_pixel(x1, y, red);
        }
    }
    image
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).with_context(|| format!("파일 저장 실패: {}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value).context("JSON 직렬화 실패")?;
    write_text(path, &json)
}

fn print_summary(index: &Index, out: &Path) {
    println!("입력: {}", index.source);
    println!("전체 쪽 수: {}", index.page_count);
    println!("텍스트 상태: {:?}", index.text_status);
    for p in &index.pages {
        println!(
            "  {:>2}쪽  {:?}  글자 {:>5}  깨짐 {:>3}  변환불가 {:>3}",
            p.page, p.text_status, p.text_chars, p.suspicious_chars, p.unmapped_chars
        );
    }
    println!("문서 {}개:", index.forms.len());
    for f in &index.forms {
        println!(
            "  {:<22} {:<10} 레이아웃 {:<5} {}",
            f.slug,
            format!("{:?}", f.pages),
            f.layouts.join(","),
            f.title
        );
    }
    if !index.unassigned_pages.is_empty() {
        println!("제목을 못 찾은 앞쪽 쪽: {:?}", index.unassigned_pages);
    }
    println!("출력: {}", out.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_are_letters_only() {
        for t in FORM_TITLES {
            assert_eq!(letters_only(t.pattern), t.pattern, "{}", t.slug);
        }
    }

    #[test]
    fn detects_title_with_shuffled_punctuation() {
        // 한컴 PDF 스트림 순서 그대로: 괄호·숫자가 줄 끝으로 밀림
        let page = "연소근로자 세 미만인 자 표준근로계약서 (18 )\n( “ ” ) 이하 사업주";
        assert_eq!(detect_title(page, FORM_TITLES).unwrap().slug, "minor");
    }

    #[test]
    fn detects_foreign_worker_forms_by_english_title() {
        let general = "■ 외국인근로자의 고용 등에 관한 법률 시행규칙 [별지 제6호서식] <개정 2019. 0. 00.>\n\
                       Standard Labor Contract\n(앞쪽)";
        let agri = "■ 외국인근로자의 고용 등에 관한 법률 시행규칙 [별지 제6호의2서식] <개정 2019. 0. 00.>\n\
                    ( ㆍ ㆍ )\n\
                    Standard Labor Contract(For Agriculture, Livestock and Fishery Sectors)\n(앞쪽)";
        assert_eq!(
            detect_title(general, FORM_TITLES).unwrap().slug,
            "foreign_worker"
        );
        assert_eq!(
            detect_title(agri, FORM_TITLES).unwrap().slug,
            "foreign_worker_agri"
        );
    }

    #[test]
    fn detects_title_with_spacing_and_line_breaks() {
        let page = "표준근로계약서\n(기간의 정함이 없는 경우)\n\n________(이하 \"사업주\"라 함)과";
        let t = detect_title(page, FORM_TITLES).unwrap();
        assert_eq!(t.slug, "standard_indefinite");

        let page = "표 준 근 로 계 약 서 ( 기간의 정함이 있는 경우 )";
        assert_eq!(
            detect_title(page, FORM_TITLES).unwrap().slug,
            "standard_fixed_term"
        );
    }

    #[test]
    fn title_far_from_page_head_is_ignored() {
        // 본문 중간에 다른 양식 이름이 언급돼도 경계로 보지 않는다
        let filler = "가".repeat(TITLE_SEARCH_WINDOW);
        let page = format!("{filler} 단시간근로자 표준근로계약서");
        assert!(detect_title(&page, FORM_TITLES).is_none());
    }

    #[test]
    fn split_assigns_continuation_pages_and_numbers_duplicates() {
        let pages = [
            "안내문",
            "표준근로계약서(기간의 정함이 없는 경우) 1",
            "계속되는 내용",
            "단시간근로자 표준근로계약서",
            "단시간근로자 표준근로계약서",
            "연소근로자(18세 미만인 자) 표준근로계약서",
            "친권자(후견인) 동의서",
            "Standard Labor Contract",
            "Standard Labor Contract(For Agriculture, Livestock and Fishery Sectors)",
        ];
        let (forms, unassigned) = split_forms(&pages, FORM_TITLES);
        assert_eq!(unassigned, vec![1]);
        let got: Vec<(&str, Vec<usize>)> = forms
            .iter()
            .map(|f| (f.slug.as_str(), f.pages.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("standard_indefinite", vec![2, 3]),
                ("part_time", vec![4]),
                ("part_time_2", vec![5]),
                ("minor", vec![6]),
                // 친권자 동의서는 연소근로자 뒤에 와도 별도 문서
                ("guardian_consent", vec![7]),
                ("foreign_worker", vec![8]),
                // 이름이 foreign_worker_로 시작해도 별개 양식이다 (중복 번호가 붙지 않아야 함)
                ("foreign_worker_agri", vec![9]),
            ]
        );
        assert_eq!(forms[4].layouts, vec!["D".to_string()]);
    }

    #[test]
    fn every_layout_a_to_f_has_a_document() {
        for layout in ["A", "B", "C", "D", "E", "F"] {
            assert!(
                FORM_TITLES.iter().any(|t| t.layouts.contains(&layout)),
                "{layout}"
            );
        }
        assert_eq!(FORM_TITLES.len(), 8, "문서 8개");
    }

    #[test]
    fn layout_reorders_shuffled_stream_and_offsets_match_text() {
        // 스트림 순서: "표준" 뒤에 "(", ")"가 밀려 있음. 위치상으로는 "(표준)"
        let mk = |c, x0: f32, y0: f32| PlacedChar {
            c,
            bbox: [x0, y0, x0 + 10.0, y0 + 14.0],
            font_size_pt: 10.0,
        };
        let stream = vec![
            mk('표', 10.0, 0.0),
            mk('준', 20.0, 0.0),
            mk('(', 0.0, 1.0),
            mk(')', 30.0, 1.0),
            mk('다', 0.0, 40.0),
            mk('음', 40.0, 40.0), // 큰 간격 → 공백
        ];
        let lines = group_lines(stream);
        let (text, boxes) = lines_to_text(&lines, 1.0);
        assert_eq!(text, "(표준)\n다 음\n");
        assert_eq!(boxes.len(), text.chars().count());
        for (i, (b, c)) in boxes.iter().zip(text.chars()).enumerate() {
            assert_eq!(b.offset, i);
            assert_eq!(b.ch, c);
            assert_eq!(b.generated, b.bbox_px.is_none());
        }
    }

    #[test]
    fn classifies_page_text() {
        let mk = |text_chars, suspicious_chars| PageExtract {
            text: String::new(),
            text_chars,
            suspicious_chars,
            unmapped_chars: 0,
        };
        assert_eq!(classify_page(&mk(0, 0)), TextStatus::Empty);
        assert_eq!(classify_page(&mk(500, 3)), TextStatus::Ok);
        assert_eq!(classify_page(&mk(500, 200)), TextStatus::Garbled);
        assert_eq!(
            overall_status([TextStatus::Ok, TextStatus::Garbled, TextStatus::Ok].into_iter()),
            TextStatus::Garbled
        );
        assert_eq!(
            overall_status([TextStatus::Garbled, TextStatus::Empty].into_iter()),
            TextStatus::Empty
        );
    }

    #[test]
    fn suspicious_chars() {
        assert!(is_suspicious('\u{E001}'));
        assert!(is_suspicious('\u{FFFD}'));
        assert!(!is_suspicious('가'));
        assert!(!is_suspicious('\n'));
        assert!(!is_suspicious('□'));
    }
}
