//! 표 괘선을 이용한 읽기 순서 재구성.
//!
//! 같은 높이의 글자를 쪽 전체 폭으로 한 줄에 묶으면, 표의 왼쪽 항목명 칸이나 오른쪽 주석 칸이
//! 본문 줄 사이에 엇갈려 들어간다 (외국인근로자 양식 8~11쪽). 그래서
//! 1) path 객체에서 가로·세로 괘선 조각을 모아 선으로 잇고 (점선은 2pt 이하 점 수천 개로 그려짐),
//! 2) 가로선과 세로선이 서로 만나는 선 묶음만 "표"로 인정하고 (빈칸 밑줄은 제외),
//! 3) 표 안 글자는 칸(가장 가까운 괘선 4개, 없으면 표 외곽) 단위로 묶어 칸째로 출력한다.
//!
//! 다른 표를 품은 칸(7쪽 점선 상자처럼)은 칸으로 묶으면 순서가 깨지므로 줄 단위로 푼다.

use pdfium_render::prelude::*;

use super::{PlacedChar, group_lines};

/// 이보다 얇은 path는 괘선 조각으로 본다
const RULE_MAX_THICKNESS_PT: f32 = 3.0;
/// 같은 선으로 볼 위치(가로선이면 y) 차이
const RULE_ALIGN_PT: f32 = 1.0;
/// 점선·파선 조각을 이을 때 허용하는 빈틈
const RULE_JOIN_GAP_PT: f32 = 4.0;
/// 이어 붙인 뒤 이보다 짧은 선은 버린다
const RULE_MIN_LENGTH_PT: f32 = 8.0;
/// 가로선·세로선이 만난다고 볼 거리
const RULE_TOUCH_PT: f32 = 3.0;
/// 글자가 괘선 범위 안에 있는지 볼 때의 여유
const COVER_TOL_PT: f32 = 1.0;

/// 괘선 하나 (픽셀 좌표, 원점 왼쪽 위). 가로선이면 `pos`=y, `start..end`=x 범위. 세로선은 반대.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Rule {
    pub horizontal: bool,
    pub pos: f32,
    pub start: f32,
    pub end: f32,
}

impl Rule {
    fn len(&self) -> f32 {
        self.end - self.start
    }
    /// 가로선·세로선이 만나는지 (끝점 근처 포함)
    fn touches(&self, other: &Rule, tol: f32) -> bool {
        if self.horizontal == other.horizontal {
            return false;
        }
        let (h, v) = if self.horizontal {
            (self, other)
        } else {
            (other, self)
        };
        v.pos >= h.start - tol
            && v.pos <= h.end + tol
            && h.pos >= v.start - tol
            && h.pos <= v.end + tol
    }
}

/// 서로 만나는 괘선 묶음 하나 = 표 하나
#[derive(Debug, Clone)]
pub(super) struct Table {
    rules: Vec<Rule>,
    /// `[x0, y0, x1, y1]`
    bbox: [f32; 4],
}

impl Table {
    fn area(&self) -> f32 {
        (self.bbox[2] - self.bbox[0]) * (self.bbox[3] - self.bbox[1])
    }
    fn contains_point(&self, x: f32, y: f32, tol: f32) -> bool {
        x >= self.bbox[0] - tol
            && x <= self.bbox[2] + tol
            && y >= self.bbox[1] - tol
            && y <= self.bbox[3] + tol
    }
}

/// 쪽의 path 객체에서 표를 찾는다
pub(super) fn find_tables(page: &PdfPage, scale: f32) -> Vec<Table> {
    let height_pt = page.height().value;
    let mut pieces = Vec::new();
    for object in page.objects().iter() {
        collect_pieces(&object, height_pt, scale, &mut pieces);
    }
    build_tables(&pieces, scale)
}

/// path 객체 하나를 괘선 조각으로 바꾼다. 폼 XObject 안쪽도 따라 들어간다.
fn collect_pieces(object: &PdfPageObject, height_pt: f32, scale: f32, out: &mut Vec<Rule>) {
    match object.object_type() {
        PdfPageObjectType::Path => {
            let Ok(q) = object.bounds() else { return };
            let x0 = q.left().value * scale;
            let x1 = q.right().value * scale;
            let y0 = (height_pt - q.top().value) * scale;
            let y1 = (height_pt - q.bottom().value) * scale;
            push_piece(x0, y0, x1, y1, RULE_MAX_THICKNESS_PT * scale, out);
        }
        PdfPageObjectType::XObjectForm => {
            if let Some(form) = object.as_x_object_form_object() {
                for child in form.iter() {
                    collect_pieces(&child, height_pt, scale, out);
                }
            }
        }
        _ => {}
    }
}

/// 얇은 사각형을 가로·세로 조각으로 분류한다. 점(가로·세로 모두 얇음)은 양쪽 후보로 넣는다.
fn push_piece(x0: f32, y0: f32, x1: f32, y1: f32, max_thick: f32, out: &mut Vec<Rule>) {
    let (w, h) = (x1 - x0, y1 - y0);
    let horizontal = Rule {
        horizontal: true,
        pos: (y0 + y1) / 2.0,
        start: x0,
        end: x1,
    };
    let vertical = Rule {
        horizontal: false,
        pos: (x0 + x1) / 2.0,
        start: y0,
        end: y1,
    };
    match (h <= max_thick, w <= max_thick) {
        (true, true) => {
            out.push(horizontal);
            out.push(vertical);
        }
        (true, false) => out.push(horizontal),
        (false, true) => out.push(vertical),
        (false, false) => {}
    }
}

/// 조각을 선으로 잇고, 서로 만나는 선끼리 묶어 표를 만든다
fn build_tables(pieces: &[Rule], scale: f32) -> Vec<Table> {
    let mut rules = merge_pieces(pieces, true, scale);
    rules.extend(merge_pieces(pieces, false, scale));

    // 만나는 선끼리 union-find
    let n = rules.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while parent[r] != r {
            r = parent[r];
        }
        let mut c = i;
        while parent[c] != r {
            let next = parent[c];
            parent[c] = r;
            c = next;
        }
        r
    }
    let tol = RULE_TOUCH_PT * scale;
    let mut touched = vec![false; n];
    for i in 0..n {
        for j in (i + 1)..n {
            if rules[i].touches(&rules[j], tol) {
                touched[i] = true;
                touched[j] = true;
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }

    // 다른 선과 만나지 않는 선(빈칸 밑줄 등)은 버린다
    let mut groups: Vec<(usize, Vec<Rule>)> = Vec::new();
    for i in 0..n {
        if !touched[i] {
            continue;
        }
        let root = find(&mut parent, i);
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, g)) => g.push(rules[i]),
            None => groups.push((root, vec![rules[i]])),
        }
    }

    groups
        .into_iter()
        .map(|(_, rules)| {
            let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for r in &rules {
                let (x0, y0, x1, y1) = if r.horizontal {
                    (r.start, r.pos, r.end, r.pos)
                } else {
                    (r.pos, r.start, r.pos, r.end)
                };
                bbox = [
                    bbox[0].min(x0),
                    bbox[1].min(y0),
                    bbox[2].max(x1),
                    bbox[3].max(y1),
                ];
            }
            Table { rules, bbox }
        })
        .collect()
}

/// 한 방향 조각을 위치별로 모아 빈틈이 작으면 이어 붙인다
fn merge_pieces(pieces: &[Rule], horizontal: bool, scale: f32) -> Vec<Rule> {
    let align = RULE_ALIGN_PT * scale;
    let gap = RULE_JOIN_GAP_PT * scale;
    let min_len = RULE_MIN_LENGTH_PT * scale;

    let mut sorted: Vec<Rule> = pieces
        .iter()
        .filter(|p| p.horizontal == horizontal)
        .copied()
        .collect();
    sorted.sort_by(|a, b| a.pos.total_cmp(&b.pos));

    let mut out = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        // 위치가 비슷한 조각 묶음
        let first_pos = sorted[i].pos;
        let mut j = i;
        while j < sorted.len() && sorted[j].pos - first_pos <= align {
            j += 1;
        }
        let mut cluster: Vec<Rule> = sorted[i..j].to_vec();
        let pos = cluster.iter().map(|r| r.pos).sum::<f32>() / cluster.len() as f32;
        cluster.sort_by(|a, b| a.start.total_cmp(&b.start));

        let mut cur: Option<Rule> = None;
        for piece in cluster {
            cur = match cur {
                Some(mut c) if piece.start <= c.end + gap => {
                    c.end = c.end.max(piece.end);
                    Some(c)
                }
                Some(c) => {
                    if c.len() >= min_len {
                        out.push(c);
                    }
                    Some(Rule { pos, ..piece })
                }
                None => Some(Rule { pos, ..piece }),
            };
        }
        if let Some(c) = cur
            && c.len() >= min_len
        {
            out.push(c);
        }
        i = j;
    }
    out
}

/// 칸 하나: 표 번호 + `[x0, y0, x1, y1]`
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cell {
    table: usize,
    rect: [f32; 4],
}

/// 글자가 속한 칸. 가장 작은 표를 고르고, 그 표 안에서 가장 가까운 괘선 4개로 칸을 정한다.
/// 해당 방향에 괘선이 없으면 표 외곽을 경계로 쓴다 (서식은 좌우 바깥 세로선을 자주 생략한다).
fn cell_of(ch: &PlacedChar, tables: &[Table], tol: f32) -> Option<Cell> {
    let cx = (ch.bbox[0] + ch.bbox[2]) / 2.0;
    let cy = ch.center_y();
    let (ti, table) = tables
        .iter()
        .enumerate()
        .filter(|(_, t)| t.contains_point(cx, cy, tol))
        .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))?;

    let mut rect = table.bbox;
    for r in &table.rules {
        if r.horizontal {
            if cx < r.start - tol || cx > r.end + tol {
                continue;
            }
            if r.pos <= cy {
                rect[1] = rect[1].max(r.pos);
            } else {
                rect[3] = rect[3].min(r.pos);
            }
        } else {
            if cy < r.start - tol || cy > r.end + tol {
                continue;
            }
            if r.pos <= cx {
                rect[0] = rect[0].max(r.pos);
            } else {
                rect[2] = rect[2].min(r.pos);
            }
        }
    }
    Some(Cell { table: ti, rect })
}

/// 출력 단위: 칸 하나(여러 줄) 또는 표 밖 글자 한 줄
#[derive(Debug)]
struct Block {
    top: f32,
    bottom: f32,
    left: f32,
    lines: Vec<Vec<PlacedChar>>,
    is_cell: bool,
}

/// 표를 고려한 읽기 순서로 글자를 줄 목록으로 만든다.
/// 블록(칸·표 밖 줄)을 (위, 왼쪽) 순으로 놓고, 같은 행(위·아래 경계가 같은)의 한 줄짜리 칸들은
/// 한 줄로 합친다 (예: `2. 근로장소 ※ 근로자를 …`, 격자 표의 한 행).
pub(super) fn reading_order(
    chars: Vec<PlacedChar>,
    tables: &[Table],
    scale: f32,
) -> Vec<Vec<PlacedChar>> {
    let tol = COVER_TOL_PT * scale;

    // 글자를 칸별로 나눈다
    let mut cells: Vec<(Cell, Vec<PlacedChar>)> = Vec::new();
    let mut free = Vec::new();
    for ch in chars {
        match cell_of(&ch, tables, tol) {
            Some(cell) => match cells
                .iter_mut()
                .find(|(c, _)| c.table == cell.table && same_rect(&c.rect, &cell.rect, tol))
            {
                Some((_, v)) => v.push(ch),
                None => cells.push((cell, vec![ch])),
            },
            None => free.push(ch),
        }
    }

    // 다른 표를 품은 칸은 줄 단위로 푼다
    let mut blocks = Vec::new();
    for (cell, cell_chars) in cells {
        let is_container = tables.iter().enumerate().any(|(ti, t)| {
            ti != cell.table
                && t.bbox[0] >= cell.rect[0] - tol
                && t.bbox[1] >= cell.rect[1] - tol
                && t.bbox[2] <= cell.rect[2] + tol
                && t.bbox[3] <= cell.rect[3] + tol
        });
        if is_container {
            free.extend(cell_chars);
        } else {
            blocks.push(Block {
                top: cell.rect[1],
                bottom: cell.rect[3],
                left: cell.rect[0],
                lines: group_lines(cell_chars),
                is_cell: true,
            });
        }
    }
    for line in group_lines(free) {
        let top = line.iter().map(|c| c.bbox[1]).fold(f32::MAX, f32::min);
        let bottom = line.iter().map(|c| c.bbox[3]).fold(f32::MIN, f32::max);
        let left = line.iter().map(|c| c.bbox[0]).fold(f32::MAX, f32::min);
        blocks.push(Block {
            top,
            bottom,
            left,
            lines: vec![line],
            is_cell: false,
        });
    }
    blocks.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.left.total_cmp(&b.left)));

    // 같은 행의 한 줄짜리 칸들은 한 줄로
    let mut out: Vec<Vec<PlacedChar>> = Vec::new();
    let mut prev_row: Option<(f32, f32)> = None;
    for block in blocks {
        let single = block.is_cell && block.lines.len() == 1;
        let same_row = prev_row
            .is_some_and(|(t, b)| (t - block.top).abs() <= tol && (b - block.bottom).abs() <= tol);
        if single
            && same_row
            && let Some(last) = out.last_mut()
        {
            last.extend(block.lines.into_iter().flatten());
            continue;
        }
        prev_row = single.then_some((block.top, block.bottom));
        out.extend(block.lines);
    }
    out
}

fn same_rect(a: &[f32; 4], b: &[f32; 4], tol: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: char, x0: f32, y0: f32) -> PlacedChar {
        PlacedChar {
            c,
            bbox: [x0, y0, x0 + 10.0, y0 + 14.0],
            font_size_pt: 10.0,
        }
    }

    fn text_of(lines: &[Vec<PlacedChar>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.iter().map(|c| c.c).collect())
            .collect()
    }

    fn h(pos: f32, start: f32, end: f32) -> Rule {
        Rule {
            horizontal: true,
            pos,
            start,
            end,
        }
    }
    fn v(pos: f32, start: f32, end: f32) -> Rule {
        Rule {
            horizontal: false,
            pos,
            start,
            end,
        }
    }

    #[test]
    fn dotted_pieces_merge_into_lines_and_underlines_are_dropped() {
        // 점선 가로선: 1px 점이 3px 간격으로 0~300
        let mut pieces = Vec::new();
        let mut x = 0.0;
        while x < 300.0 {
            push_piece(x, 99.5, x + 1.0, 100.5, 3.0, &mut pieces);
            x += 3.0;
        }
        // 세로 실선 2개가 점선과 만남 → 표
        pieces.push(v(0.0, 0.0, 200.0));
        pieces.push(v(300.0, 0.0, 200.0));
        // 떨어진 밑줄 → 버려져야 함
        pieces.push(h(500.0, 0.0, 120.0));

        let tables = build_tables(&pieces, 1.0);
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        let hs: Vec<&Rule> = t.rules.iter().filter(|r| r.horizontal).collect();
        assert_eq!(hs.len(), 1, "점선이 한 줄로 이어져야 함: {hs:?}");
        assert!(hs[0].start <= 0.5 && hs[0].end >= 298.0);
        assert!(!t.rules.iter().any(|r| r.horizontal && r.pos == 500.0));
    }

    /// 왼쪽 항목명 칸 + 가운데 본문(2줄) + 오른쪽 주석 칸(3줄, 본문과 높이가 엇갈림)
    #[test]
    fn side_columns_do_not_interleave_with_body() {
        // 표 외곽 위·아래 가로선, 칸 구분 세로선 2개 (좌우 바깥 세로선은 없음)
        let tables = vec![Table {
            rules: vec![
                h(0.0, 0.0, 600.0),
                h(100.0, 0.0, 600.0),
                v(100.0, 0.0, 100.0),
                v(400.0, 0.0, 100.0),
            ],
            bbox: [0.0, 0.0, 600.0, 100.0],
        }];
        let chars = vec![
            // 항목명 (세로 가운데)
            ch('항', 10.0, 40.0),
            ch('목', 20.0, 40.0),
            // 본문 2줄
            ch('본', 110.0, 20.0),
            ch('일', 120.0, 20.0),
            ch('본', 110.0, 60.0),
            ch('이', 120.0, 60.0),
            // 주석 3줄 (본문 줄과 y가 겹치는 줄 포함)
            ch('주', 410.0, 10.0),
            ch('하', 410.0, 40.0),
            ch('나', 410.0, 70.0),
            // 표 밖 (위)
            ch('머', 0.0, -40.0),
            ch('리', 10.0, -40.0),
        ];
        let lines = reading_order(chars, &tables, 1.0);
        assert_eq!(
            text_of(&lines),
            vec!["머리", "항목", "본일", "본이", "주", "하", "나"]
        );
    }

    #[test]
    fn single_line_cells_in_same_row_join() {
        let tables = vec![Table {
            rules: vec![h(0.0, 0.0, 300.0), h(30.0, 0.0, 300.0), v(100.0, 0.0, 30.0)],
            bbox: [0.0, 0.0, 300.0, 30.0],
        }];
        let chars = vec![ch('나', 120.0, 8.0), ch('가', 10.0, 8.0)];
        let lines = reading_order(chars, &tables, 1.0);
        assert_eq!(text_of(&lines), vec!["가나"]);
    }

    #[test]
    fn container_cell_is_unrolled_around_inner_table() {
        // 바깥 상자 안에 위 문장 / 안쪽 표 / 아래 문장
        let outer = Table {
            rules: vec![
                h(0.0, 0.0, 500.0),
                h(300.0, 0.0, 500.0),
                v(0.0, 0.0, 300.0),
                v(500.0, 0.0, 300.0),
            ],
            bbox: [0.0, 0.0, 500.0, 300.0],
        };
        let inner = Table {
            rules: vec![
                h(100.0, 50.0, 450.0),
                h(200.0, 50.0, 450.0),
                v(50.0, 100.0, 200.0),
                v(450.0, 100.0, 200.0),
            ],
            bbox: [50.0, 100.0, 450.0, 200.0],
        };
        let chars = vec![
            ch('아', 60.0, 250.0), // 아래 문장 (먼저 넣어도 순서는 위치로)
            ch('위', 60.0, 30.0),
            ch('표', 60.0, 140.0),
        ];
        let lines = reading_order(chars, &[outer, inner], 1.0);
        assert_eq!(text_of(&lines), vec!["위", "표", "아"]);
    }
}
