//! 개선 기록 문서 갱신 (`pii-mask eval issues`)
//!
//! - `docs/issues.md`: 이슈마다 `<!-- metrics: 키, 키 -->`에 적힌 지표의 최신 실행 값으로
//!   `<!-- auto:start -->` ~ `<!-- auto:end -->` 사이만 다시 쓴다 (나머지 해석 영역은 건드리지 않음)
//! - 목표 미달(판정 "불가") 지표인데 그 키를 가진 이슈가 없으면 이슈를 자동 등록 (해석 영역은 "작성 필요")
//! - `docs/run_log.md`: 아직 없는 실행마다 한 줄 (실행 ID, 날짜, 데이터 시드, 변경 사항, 주요 지표)
//! - 실패 유형은 건수만 (평가용 세트의 문서 ID·값은 적지 않는다)

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::runlog::{Goal, Metric, RunRecord, load_all};

const AUTO_START: &str = "<!-- auto:start -->";
const AUTO_END: &str = "<!-- auto:end -->";

const ISSUES_HEADER: &str = "# 이슈 (개선 기록)

목표 미달 지표와 해결 과정을 기록한다. 규칙은 CLAUDE.md \"개선 기록\".
- 각 이슈의 `<!-- auto:start -->` ~ `<!-- auto:end -->` 사이는 `pii-mask eval issues`가 최신 실행 기록(`results/runs/`)으로 다시 쓴다 — 손으로 고치지 않는다
- 나머지(상태·왜 문제인가·원인·개선안·시도 기록)는 사람이 쓴다. 해결된 이슈도 지우지 않는다
- 실패 예시 문서는 **개발용 세트**에서만 적고, 평가용 세트는 건수만 적는다
";

const RUN_LOG_HEADER: &str = "# 실행 기록

`pii-mask eval issues`가 실행마다 한 줄씩 더한다 (`results/runs/<실행 ID>/`의 metrics.json·failures.json).

| 실행 ID | 날짜 | 데이터 (시드) | 변경 사항 | 주요 지표 |
|---|---|---|---|---|
";

/// 갱신 결과
#[derive(Debug, Default)]
pub struct Summary {
    pub issues_updated: usize,
    pub issues_registered: Vec<String>,
    pub runs_logged: usize,
}

/// 키마다 최신 지표 + 그 실행 (이전 실행 값도)
fn latest_by_key(
    runs: &[(RunRecord, BTreeMap<String, usize>)],
) -> BTreeMap<String, Vec<(usize, Metric)>> {
    let mut m: BTreeMap<String, Vec<(usize, Metric)>> = BTreeMap::new();
    for (i, (r, _)) in runs.iter().enumerate() {
        for x in &r.metrics {
            m.entry(x.key.clone()).or_default().push((i, x.clone()));
        }
    }
    m
}

fn auto_block(
    keys: &[String],
    runs: &[(RunRecord, BTreeMap<String, usize>)],
    by_key: &BTreeMap<String, Vec<(usize, Metric)>>,
    date: &str,
) -> String {
    let mut w = String::new();
    let _ = writeln!(w, "{AUTO_START}");
    let _ = writeln!(
        w,
        "| 지표 | 최신 값 (95% 신뢰구간) | 목표 | 판정 | 같은 세트 직전 값 | 실행 ID | 데이터 (시드) |"
    );
    let _ = writeln!(w, "|---|---|---|---|---|---|---|");
    let mut fail_runs: Vec<usize> = Vec::new();
    for k in keys {
        // 평가용 최신 1줄 + 개발용 최신 1줄 (개발용 실행이 평가용 상태를 덮어 보이지 않게)
        let mut latest: Vec<&(usize, Metric)> = Vec::new();
        for kind in ["평가용", "개발용"] {
            if let Some(x) = by_key
                .get(k)
                .into_iter()
                .flatten()
                .rev()
                .find(|(i, _)| runs[*i].0.set_kind == kind)
            {
                latest.push(x);
            }
        }
        if latest.is_empty() {
            let _ = writeln!(w, "| `{k}` | 기록 없음 | - | - | - | - | - |");
        }
        for (ri, m) in latest {
            {
                // 직전 값 = 같은 세트의 직전 실행 (세트가 다르면 비교하지 않음)
                let set = &runs[*ri].0.set;
                let prev = by_key
                    .get(k)
                    .into_iter()
                    .flatten()
                    .rev()
                    .find(|(i, _)| *i < *ri && &runs[*i].0.set == set)
                    .map_or("-".to_string(), |(i, p)| {
                        format!("{} (`{}`)", p.fmt_value(), runs[*i].0.run_id)
                    });
                let r = &runs[*ri].0;
                let _ = writeln!(
                    w,
                    "| {} (`{k}`) | {} | {} | {} | {prev} | `{}` | {} `{}` (시드 {}) |",
                    m.name,
                    m.fmt_value(),
                    m.fmt_target(),
                    m.verdict,
                    r.run_id,
                    r.set_kind,
                    r.set,
                    r.seed.map_or("-".into(), |s| s.to_string())
                );
                if !fail_runs.contains(ri) {
                    fail_runs.push(*ri);
                }
            }
        }
    }
    for ri in fail_runs {
        let (r, f) = &runs[ri];
        // 난이도별 항목("D1:…")은 빼고 전체 유형만, 건수 많은 순
        let mut items: Vec<(&String, &usize)> = f
            .iter()
            .filter(|(k, _)| !k.contains(':') || k.starts_with("누락:") || k.starts_with("오탐:"))
            .collect();
        items.sort_by(|a, b| b.1.cmp(a.1));
        let list: Vec<String> = items
            .iter()
            .take(10)
            .map(|(k, n)| format!("{k} {n}"))
            .collect();
        let _ = writeln!(
            w,
            "\n실패 유형 건수 (`{}`, {} 세트): {}",
            r.run_id,
            r.set_kind,
            if list.is_empty() {
                "없음".into()
            } else {
                list.join(", ")
            }
        );
    }
    let _ = writeln!(w, "\n_자동 갱신 {date}_");
    let _ = write!(w, "{AUTO_END}");
    w
}

fn issue_template(no: usize, m: &Metric, date: &str) -> String {
    format!(
        "\n## ISSUE-{no:03}: {} 목표 미달\n<!-- metrics: {} -->\n- 상태: 열림 (자동 등록 {date})\n\n### 자동 영역\n{AUTO_START}\n{AUTO_END}\n\n### 왜 문제인가\n(작성 필요)\n\n### 원인\n(작성 필요)\n\n### 개선안\n(작성 필요)\n\n### 시도 기록\n| 날짜 | 시도 | 실행 ID (전 → 후) | 전 → 후 | 결과 |\n|---|---|---|---|---|\n",
        m.name, m.key
    )
}

/// 이슈 블록들의 (시작 위치, 번호, 지표 키)
fn parse_issues(text: &str) -> Vec<(usize, usize, Vec<String>)> {
    let mut out = Vec::new();
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("## ISSUE-") {
            let no: usize = rest
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            out.push((pos, no, Vec::new()));
        }
        if let Some(rest) = line.trim().strip_prefix("<!-- metrics:")
            && let Some(last) = out.last_mut()
        {
            last.2 = rest
                .trim_end_matches("-->")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        pos += line.len();
    }
    out
}

/// 문서 갱신
pub fn update(root: &Path) -> std::io::Result<Summary> {
    let runs = load_all(root);
    let by_key = latest_by_key(&runs);
    let (date, _) = crate::runlog::now_kst();
    let mut sum = Summary::default();
    let docs = root.join("docs");
    fs::create_dir_all(&docs)?;

    // ── issues.md ──
    let path = docs.join("issues.md");
    let mut text = fs::read_to_string(&path).unwrap_or_else(|_| ISSUES_HEADER.to_string());
    // 자동 등록: 최신 판정이 "불가"인데 어느 이슈도 그 키를 갖지 않음
    let known: Vec<String> = parse_issues(&text)
        .into_iter()
        .flat_map(|(_, _, k)| k)
        .collect();
    let mut next = parse_issues(&text)
        .iter()
        .map(|(_, n, _)| *n)
        .max()
        .unwrap_or(0)
        + 1;
    for (k, v) in &by_key {
        // 판정은 평가용 세트의 최신 실행으로만 (개발용 결과로 이슈를 만들지 않음)
        if let Some((_, m)) = v
            .iter()
            .rev()
            .find(|(i, _)| runs[*i].0.set_kind == "평가용")
            && m.failing()
            && m.goal != Goal::Report
            && !known.contains(k)
        {
            text.push_str(&issue_template(next, m, &date));
            sum.issues_registered.push(format!("ISSUE-{next:03} ({k})"));
            next += 1;
        }
    }
    // 자동 영역 다시 쓰기 (뒤에서부터 → 앞 위치가 안 바뀜)
    let issues = parse_issues(&text);
    let ends: Vec<usize> = issues
        .iter()
        .skip(1)
        .map(|x| x.0)
        .chain([text.len()])
        .collect();
    for ((start, _, keys), end) in issues.iter().zip(ends).rev() {
        let block = &text[*start..end];
        let (Some(a), Some(b)) = (block.find(AUTO_START), block.find(AUTO_END)) else {
            continue;
        };
        if b < a {
            continue;
        }
        let new = auto_block(keys, &runs, &by_key, &date);
        text.replace_range(start + a..start + b + AUTO_END.len(), &new);
        sum.issues_updated += 1;
    }
    fs::write(&path, text)?;

    // ── run_log.md ──
    let path = docs.join("run_log.md");
    let mut log = fs::read_to_string(&path).unwrap_or_else(|_| RUN_LOG_HEADER.to_string());
    for (r, _) in &runs {
        if log.contains(&format!("`{}`", r.run_id)) {
            continue;
        }
        let main: Vec<String> = r
            .metrics
            .iter()
            .filter(|m| m.goal != Goal::Report)
            .map(|m| {
                let v = if m.n == 0 {
                    "-".into()
                } else {
                    format!("{:.1}%", m.value * 100.0)
                };
                format!(
                    "{} {v}{}",
                    m.key.split_once('.').map_or(m.key.as_str(), |x| x.1),
                    if m.failing() { "✗" } else { "" }
                )
            })
            .collect();
        let _ = writeln!(
            log,
            "| `{}` | {} | {} `{}` (시드 {}, {}건) | {} | {} |",
            r.run_id,
            r.date,
            r.set_kind,
            r.set,
            r.seed.map_or("-".into(), |s| s.to_string()),
            r.docs,
            r.note.replace('|', "/"),
            main.join(", ")
        );
        sum.runs_logged += 1;
    }
    fs::write(&path, log)?;
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_issue_blocks_and_keys() {
        let t = "# x\n\n## ISSUE-001: a\n<!-- metrics: e4.sensitive_recall, e4b.sensitive_recall -->\n<!-- auto:start -->\n<!-- auto:end -->\n## ISSUE-002: b\n";
        let p = parse_issues(t);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].1, 1);
        assert_eq!(p[0].2, vec!["e4.sensitive_recall", "e4b.sensitive_recall"]);
    }
}
