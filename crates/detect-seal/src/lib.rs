//! 서명란 탐지 (PLAN.md 9단계): OCR 줄에서 앵커 문구 → 소속 판별 → 앵커 주변 영역 + 근처 붉은색 덩어리
//!
//! - 소속: 같은 줄 앵커 왼쪽 → 앵커 위 `lines_above`줄 안에서 가장 가까운 키워드. 못 찾으면 unknown (정책상 가림)
//! - 영역: 글자 높이 h(OCR 박스 높이 × 0.6) 기준 왼쪽 4h·오른쪽 2.5h·앵커 중심에서 위아래 1.8h (서명·도장이 앵커 글자와 이름을 덮는 위치)
//! - 붉은색: 영역을 3h 넓힌 범위 안의 붉은 픽셀 덩어리(도장·지장)를 영역에 합친다 — 먼 곳의 붉은색(워터마크 등)은 무시

use image::RgbImage;
use ocr::OcrPage;
use rulepack::SignatureDef;

/// 서명란 영역
#[derive(Debug, Clone)]
pub struct SignRegion {
    /// worker / employer / guardian / unknown
    pub owner: String,
    pub rect: [f32; 4],
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn line_rect(poly: &[[f32; 2]; 4]) -> [f32; 4] {
    let xs = poly.iter().map(|p| p[0]);
    let ys = poly.iter().map(|p| p[1]);
    [
        xs.clone().fold(f32::MAX, f32::min),
        ys.clone().fold(f32::MAX, f32::min),
        xs.fold(f32::MIN, f32::max),
        ys.fold(f32::MIN, f32::max),
    ]
}

fn owner_in(text: &str, def: &SignatureDef) -> Option<&'static str> {
    // 가장 오른쪽(앵커에 가까운) 키워드, 공백 무시 ("대 표 자")
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut best: Option<(usize, &'static str)> = None;
    for (kws, name) in [
        (&def.worker, "worker"),
        (&def.employer, "employer"),
        (&def.guardian, "guardian"),
    ] {
        for k in kws {
            if let Some(i) = text.rfind(k.as_str())
                && best.is_none_or(|(j, _)| i > j)
            {
                best = Some((i, name));
            }
        }
    }
    best.map(|(_, n)| n)
}

/// 붉은 픽셀 (도장·지장 인주색)
fn is_red(p: &image::Rgb<u8>) -> bool {
    let [r, g, b] = p.0;
    let (r, g, b) = (f32::from(r), f32::from(g), f32::from(b));
    r > 110.0 && r > g * 1.45 && r > b * 1.35
}

/// 범위 안 붉은 픽셀 덩어리(3px 격자 연결 성분) 중 `touch`와 겹치는 것들의 경계
fn red_bounds(img: &RgbImage, area: [f32; 4], touch: [f32; 4]) -> Option<[f32; 4]> {
    let (w, h) = img.dimensions();
    let step = 3u32;
    let x0 = area[0].max(0.0) as u32;
    let y0 = area[1].max(0.0) as u32;
    let x1 = (area[2].max(0.0) as u32).min(w);
    let y1 = (area[3].max(0.0) as u32).min(h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (gw, gh) = (((x1 - x0) / step) as usize, ((y1 - y0) / step) as usize);
    let mut grid = vec![false; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            grid[gy * gw + gx] =
                is_red(img.get_pixel(x0 + gx as u32 * step, y0 + gy as u32 * step));
        }
    }
    let mut seen = vec![false; gw * gh];
    let mut out: Option<[f32; 4]> = None;
    for start in 0..gw * gh {
        if !grid[start] || seen[start] {
            continue;
        }
        // 연결 성분 (8방향, 한 칸 건너뛰기 허용 — 도장 테두리 끊김)
        let mut stack = vec![start];
        seen[start] = true;
        let mut n = 0;
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        while let Some(c) = stack.pop() {
            n += 1;
            let (cx, cy) = ((c % gw) as i64, (c / gw) as i64);
            let px = (x0 + cx as u32 * step) as f32;
            let py = (y0 + cy as u32 * step) as f32;
            b = union(b, [px, py, px + step as f32, py + step as f32]);
            for dy in -2i64..=2 {
                for dx in -2i64..=2 {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= gw as i64 || ny >= gh as i64 {
                        continue;
                    }
                    let k = ny as usize * gw + nx as usize;
                    if grid[k] && !seen[k] {
                        seen[k] = true;
                        stack.push(k);
                    }
                }
            }
        }
        let overlaps = b[0] < touch[2] && touch[0] < b[2] && b[1] < touch[3] && touch[1] < b[3];
        if n >= 6 && overlaps {
            out = Some(out.map_or(b, |a| union(a, b)));
        }
    }
    out
}

/// 쪽 하나의 서명란 영역
pub fn detect(page: &OcrPage, img: Option<&RgbImage>, def: &SignatureDef) -> Vec<SignRegion> {
    let rects: Vec<[f32; 4]> = page.lines.iter().map(|l| line_rect(&l.poly)).collect();
    // 쪽의 보통 줄 높이 (중앙값) — 손글씨·도장과 붙어 큰 박스가 된 줄 때문에 영역이 커지지 않게 상한
    let mut heights: Vec<f32> = rects
        .iter()
        .map(|r| r[3] - r[1])
        .filter(|h| *h > 0.0)
        .collect();
    heights.sort_by(f32::total_cmp);
    let median_h = heights.get(heights.len() / 2).copied().unwrap_or(30.0);
    // 가장 아래 날짜 줄 (숫자 + 년·월·일, 짧은 줄)의 y 중심
    let date_y = page
        .lines
        .iter()
        .zip(&rects)
        .filter(|(l, _)| {
            let t = l.text.trim();
            t.chars().count() <= 20
                && t.contains('년')
                && t.contains('월')
                && t.contains('일')
                && t.chars().any(|c| c.is_ascii_digit())
        })
        .map(|(_, r)| (r[1] + r[3]) / 2.0)
        .fold(None, |a: Option<f32>, y| Some(a.map_or(y, |a| a.max(y))));
    let mut out = Vec::new();
    for (li, line) in page.lines.iter().enumerate() {
        let chars: Vec<char> = line.text.chars().collect();
        // 앵커 위치 (공백 무시)
        let flat: Vec<(usize, char)> = chars
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.is_whitespace())
            .map(|(i, c)| (i, *c))
            .collect();
        let mut anchor: Option<(usize, usize)> = None;
        for a in &def.anchors {
            let av: Vec<char> = a.chars().filter(|c| !c.is_whitespace()).collect();
            if av.is_empty() || flat.len() < av.len() {
                continue;
            }
            if let Some(k) = (0..=flat.len() - av.len()).rev().find(|&k| {
                flat[k..k + av.len()]
                    .iter()
                    .map(|x| x.1)
                    .eq(av.iter().copied())
            }) {
                anchor = Some((flat[k].0, flat[k + av.len() - 1].0 + 1));
                break;
            }
        }
        // 앵커를 못 찾았지만 서명 줄 모양(이름 키로 시작: "성명:"·"근로자:"·"친권자(후견인)" 등)이면
        // 줄 끝을 앵커로 삼는다 — 도장·서명이 "(서명)"을 덮어 OCR이 못 읽은 경우 (ISSUE-004, 개발용 세트 확인)
        // 서명란은 계약일 줄("2025년 11월 27일") 아래에 온다 → 날짜 줄 아래 줄에만 대체 앵커 (제목·인적사항 줄 제외)
        let below_date = date_y.is_none_or(|dy| (rects[li][1] + rects[li][3]) / 2.0 > dy);
        let anchor = anchor.or_else(|| {
            if !below_date {
                return None;
            }
            let core: String = flat.iter().map(|x| x.1).collect();
            let sign_line = def.sign_line_keys.iter().any(|k| {
                let k: String = k.chars().filter(|c| !c.is_whitespace()).collect();
                core.starts_with(&k)
            });
            (sign_line && !chars.is_empty()).then(|| (chars.len() - 1, chars.len()))
        });
        let Some((s, e)) = anchor else { continue };
        let ab = line.char_boxes
            [s.min(line.char_boxes.len().saturating_sub(1))..e.min(line.char_boxes.len())]
            .iter()
            .copied()
            .reduce(union)
            .unwrap_or(rects[li]);
        // OCR 검출 박스는 글자보다 위아래로 넓다(언클립) → 글자 높이 ≈ 박스 높이 × 0.6
        let h = ((ab[3] - ab[1]) * 0.6).clamp(8.0, median_h * 0.9);
        // 소속: 같은 줄 왼쪽 → 위 줄들 (가까운 순)
        let left: String = chars[..s].iter().collect();
        let mut owner = owner_in(&left, def);
        if owner.is_none() {
            let cy = (ab[1] + ab[3]) / 2.0;
            let mut above: Vec<(f32, usize)> = rects
                .iter()
                .enumerate()
                .filter(|(j, r)| *j != li && (r[1] + r[3]) / 2.0 < cy - h * 0.3)
                .map(|(j, r)| (cy - (r[1] + r[3]) / 2.0, j))
                .filter(|(d, _)| *d < median_h * 1.6 * def.lines_above as f32)
                .collect();
            above.sort_by(|a, b| a.0.total_cmp(&b.0));
            owner = above
                .iter()
                .find_map(|(_, j)| owner_in(&page.lines[*j].text, def));
        }
        let cy = (ab[1] + ab[3]) / 2.0;
        // 서명란 블록: 같은 줄 ':' 뒤(이름 칸 시작)부터 앵커 오른쪽까지 — 긴 이름 위에 그린 서명도 덮음
        let value_start = chars[..s]
            .iter()
            .rposition(|c| matches!(c, ':' | '：'))
            .and_then(|i| line.char_boxes.get(i + 1))
            .map_or(ab[0] - 4.0 * h, |b| b[0].min(ab[0] - 4.0 * h));
        let mut rect = [
            value_start - 0.5 * h,
            cy - 1.8 * h,
            ab[2] + 2.5 * h,
            cy + 1.8 * h,
        ];
        if let Some(img) = img {
            let near = [
                rect[0] - 3.0 * h,
                rect[1] - 2.0 * h,
                rect[2] + 3.0 * h,
                rect[3] + 2.0 * h,
            ];
            if let Some(rb) = red_bounds(img, near, rect) {
                rect = union(rect, [rb[0] - 4.0, rb[1] - 4.0, rb[2] + 4.0, rb[3] + 4.0]);
            }
        }
        out.push(SignRegion {
            owner: owner.unwrap_or("unknown").to_string(),
            rect,
        });
    }
    out
}

/// 양식 키 뒤 값이 비어 읽힌 줄(예: "대표자:" — 손글씨·도장에 가려 OCR이 못 읽음) → 키 오른쪽 칸 영역
pub fn empty_value_regions(page: &OcrPage, def: &SignatureDef) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for line in &page.lines {
        let core: String = line.text.chars().filter(|c| !c.is_whitespace()).collect();
        let core = core.trim_end_matches([':', '：']);
        if def.empty_value_keys.iter().any(|k| {
            let k: String = k.chars().filter(|c| !c.is_whitespace()).collect();
            core.ends_with(&k) && core.chars().count() <= k.chars().count() + 6
        }) {
            let r = line_rect(&line.poly);
            let h = (r[3] - r[1]) * 0.6;
            out.push([r[2], r[1], r[2] + 9.0 * h, r[3]]);
        }
    }
    out
}

/// 필드 영역 마스킹: 개인정보 양식 키로 시작하는 줄의 값 칸 전체 (OCR이 값을 틀리게 읽어도 칸은 가린다)
///
/// - 키가 줄 앞부분(공백 무시 8글자 안)에 있고 뒤에 값이 이어지면: 키 뒤 글자들의 박스
/// - 키만 있는 줄(표의 키 칸)이면: 같은 행 오른쪽에서 가장 가까운 줄(값 칸) 전체
pub fn field_regions(page: &OcrPage, keys: &[String]) -> Vec<[f32; 4]> {
    let keys: Vec<Vec<char>> = keys
        .iter()
        .map(|k| k.chars().filter(|c| !c.is_whitespace()).collect())
        .filter(|k: &Vec<char>| !k.is_empty())
        .collect();
    let rects: Vec<[f32; 4]> = page.lines.iter().map(|l| line_rect(&l.poly)).collect();
    let mut out = Vec::new();
    for (li, line) in page.lines.iter().enumerate() {
        let chars: Vec<char> = line.text.chars().collect();
        let core: Vec<(usize, char)> = chars
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.is_whitespace())
            .map(|(i, c)| (i, *c))
            .collect();
        // 줄 앞부분의 키 (가장 긴 키 우선)
        let mut hit: Option<usize> = None; // 키 끝 다음 글자의 줄 위치
        for k in &keys {
            for start in 0..core.len().min(8) {
                if start + k.len() <= core.len()
                    && core[start..start + k.len()]
                        .iter()
                        .map(|x| x.1)
                        .eq(k.iter().copied())
                {
                    let end = core.get(start + k.len()).map_or(chars.len(), |x| x.0);
                    if hit.is_none_or(|h| end > h) {
                        hit = Some(end);
                    }
                    break;
                }
            }
        }
        let Some(mut v) = hit else { continue };
        while v < chars.len() && (chars[v].is_whitespace() || matches!(chars[v], ':' | '：')) {
            v += 1;
        }
        let r = rects[li];
        let h = (r[3] - r[1]).max(4.0);
        if v < chars.len() && v < line.char_boxes.len() {
            // 같은 줄의 값 — 글자 박스 묶음으로 (기울어진 줄에서 위아래 줄을 덮지 않게, ISSUE-005)
            for b in ocr::segment_rects(line.char_boxes[v..].iter().copied()) {
                let bh = (b[3] - b[1]).max(4.0);
                out.push([b[0] - 0.3 * bh, b[1], b[2] + 0.3 * bh, b[3]]);
            }
        } else {
            // 키만 있는 줄 → 같은 행 오른쪽 가장 가까운 줄 (세로 겹침 50% 이상)
            let next = rects
                .iter()
                .enumerate()
                .filter(|(j, o)| {
                    let ov = (r[3].min(o[3]) - r[1].max(o[1])).max(0.0);
                    *j != li && o[0] >= r[2] - 0.5 * h && ov >= 0.5 * h.min(o[3] - o[1])
                })
                .min_by(|a, b| a.1[0].total_cmp(&b.1[0]));
            if let Some((j, o)) = next {
                let segs = ocr::segment_rects(page.lines[j].char_boxes.iter().copied());
                if segs.is_empty() {
                    out.push([o[0] - 0.3 * h, o[1], o[2] + 0.3 * h, o[3]]);
                }
                for b in segs {
                    let bh = (b[3] - b[1]).max(4.0);
                    out.push([b[0] - 0.3 * bh, b[1], b[2] + 0.3 * bh, b[3]]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocr::OcrLine;

    fn line(text: &str, y: f32) -> OcrLine {
        let n = text.chars().count();
        OcrLine {
            text: text.into(),
            confidence: 1.0,
            poly: [
                [100.0, y],
                [100.0 + 20.0 * n as f32, y],
                [100.0 + 20.0 * n as f32, y + 20.0],
                [100.0, y + 20.0],
            ],
            char_boxes: (0..n)
                .map(|i| {
                    [
                        100.0 + 20.0 * i as f32,
                        y,
                        120.0 + 20.0 * i as f32,
                        y + 20.0,
                    ]
                })
                .collect(),
        }
    }

    #[test]
    fn owner_from_lines_above() {
        let def = SignatureDef {
            anchors: vec!["(서명)".into()],
            worker: vec!["근로자".into()],
            employer: vec!["사업주".into(), "대표자".into()],
            guardian: vec!["친권자".into()],
            lines_above: 3,
            empty_value_keys: vec![],
            sign_line_keys: vec![],
        };
        let page = OcrPage {
            width: 800,
            height: 600,
            lines: vec![
                line("(근로자) 주 소 : 어딘가", 100.0),
                line("연 락 처 : 010", 140.0),
                line("성 명 : 홍길동 (서명)", 180.0),
                line("대 표 자 : 김철수 (서명)", 300.0),
            ],
        };
        let r = detect(&page, None, &def);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].owner, "worker");
        assert_eq!(r[1].owner, "employer");
    }
}
