#![allow(dead_code)]

use jiyue_query::date::{DAY_MS, days_from_civil};
use jiyue_query::{Cell, PropertyIndex, QueryContext, QueryResult, run_text};

/// Notes used by most tests. Unsorted queries return rows in path byte order:
/// `archive/...`, `inbox.md`, `日记/...`, `项目/...`.
pub const FIXTURE: &[(&str, &str)] = &[
    (
        "项目/rust-async.md",
        "---\ntitle: Rust async\nstatus: 进行中\ndue: 2026-11-08\npriority: 2\ntags: [rust, async]\n---\n# Rust async\nSee [[Tokio]] and #rust/tips here.\n",
    ),
    (
        "项目/tokio.md",
        "---\nstatus: 完成\ndue: 2026-10-01\npriority: 1\ntags:\n  - rust\n---\nRuntime notes. #rust\n",
    ),
    (
        "项目/gpui.md",
        "---\nstatus: 进行中\ndue: 2026-12-01\npriority: 3\n---\nUI toolkit, see [[rust-async]].\n",
    ),
    (
        "日记/2026-11-04.md",
        "今天写了 #daily 笔记。\n\n```\n#notatag\n```\n",
    ),
    (
        "inbox.md",
        "---\nstatus: 进行中\ndue: 2026-11-05\n---\n[[gpui]] [[gpui]] [[Nothing]]\n",
    ),
    (
        "archive/old.md",
        "---\nstatus: 完成\ndue: 2025-01-01\ntitle: \"Old: note\"\n---\nold text\n",
    ),
];

/// 2026-11-04 12:00 UTC, the fixed "now" used by the tests.
pub fn now_ms() -> i64 {
    days_from_civil(2026, 11, 4) * DAY_MS + 12 * 3_600_000
}

pub fn ctx() -> QueryContext {
    QueryContext::new(now_ms(), 0)
}

pub fn index_from(notes: &[(&str, &str)]) -> PropertyIndex {
    let mut idx = PropertyIndex::new();
    for (i, (path, text)) in notes.iter().enumerate() {
        let base = 1_700_000_000_000i64 + (i as i64) * 1000;
        idx.upsert(path, text, base, base - 5000);
    }
    idx
}

pub fn vault() -> PropertyIndex {
    index_from(FIXTURE)
}

pub fn run(idx: &PropertyIndex, src: &str) -> QueryResult {
    match run_text(idx, src, &ctx()) {
        Ok(r) => r,
        Err(e) => panic!("query failed to parse: {src}\n{e}"),
    }
}

pub fn run_with(idx: &PropertyIndex, src: &str, ctx: &QueryContext) -> QueryResult {
    match run_text(idx, src, ctx) {
        Ok(r) => r,
        Err(e) => panic!("query failed to parse: {src}\n{e}"),
    }
}

/// Paths of all rows, in output order.
pub fn paths(r: &QueryResult) -> Vec<String> {
    r.rows().map(|row| row.path.clone()).collect()
}

/// Note names (file stems) of all rows, in output order.
pub fn names(idx: &PropertyIndex, r: &QueryResult) -> Vec<String> {
    r.rows()
        .map(|row| {
            idx.get(&row.path)
                .map(|n| n.name.clone())
                .unwrap_or_default()
        })
        .collect()
}

/// Display text of one column for every row, in output order.
pub fn column(r: &QueryResult, col: usize) -> Vec<String> {
    r.rows()
        .map(|row| row.cells.get(col).map(Cell::display).unwrap_or_default())
        .collect()
}

/// Evaluates one expression against the note `old` (named by `archive/old.md`) and returns
/// its value.
pub fn value_on(idx: &PropertyIndex, note_name: &str, expr: &str) -> Cell {
    let src = format!("where name = \"{note_name}\" select {expr} as v");
    let r = run(idx, &src);
    match r.rows().next().and_then(|row| row.cells.first()) {
        Some(c) => c.clone(),
        None => panic!("no row for {note_name} in: {src}"),
    }
}

pub fn value(expr: &str) -> Cell {
    value_on(&vault(), "old", expr)
}
