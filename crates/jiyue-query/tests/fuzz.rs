//! Deterministic fuzz-style tests: random queries must never panic, and every accepted query
//! must execute and return consistent counts. Uses a fixed-seed LCG so failures reproduce.

mod common;

use common::vault;
use jiyue_query::{PropertyIndex, QueryContext, execute, parse, suggest};

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % (n.max(1) as u64)) as usize
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs.get(self.below(xs.len())).copied().unwrap_or("")
    }
}

const TOKENS: &[&str] = &[
    "from",
    "where",
    "select",
    "sort",
    "group",
    "by",
    "limit",
    "view",
    "and",
    "or",
    "not",
    "(",
    ")",
    "[",
    "]",
    ",",
    "=",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "+",
    "-",
    "*",
    "/",
    "contains",
    "has",
    "in",
    "as",
    "asc",
    "desc",
    "today()",
    "now()",
    "date(\"2026-01-31\")",
    "7d",
    "2mo",
    "1y",
    "count()",
    "sum(size)",
    "avg(words)",
    "min(due)",
    "max(mtime)",
    "folder(\"项目\")",
    "tag(\"rust\")",
    "link_to(\"gpui\")",
    "all",
    "status",
    "due",
    "状态",
    "`due date`",
    "\"text\"",
    "'x'",
    "12",
    "3.5",
    "true",
    "false",
    "null",
    "backlinks",
    "tags",
    "name",
    "len(tags)",
    "exists(due)",
    "default(x, 1)",
    "prop(\"status\")",
    "table",
    "cards",
    "\\",
    "\"",
    "`",
    "#",
    "?",
    "!",
    "\n",
    "  ",
    "状",
    "é",
    "🎉",
    "lower(name)",
    "count(links)",
    "-5",
    "0",
    "1e9",
    "99999999999999999999",
];

const CHARS: &[char] = &[
    'a', 'z', 'Z', '0', '9', ' ', '\n', '\t', '"', '\'', '`', '(', ')', '[', ']', ',', '=', '<',
    '>', '!', '+', '-', '*', '/', '#', '\\', '状', '态', '项', 'é', '🎉', '{', '}', ':', '.',
];

fn token_soup(rng: &mut Lcg) -> String {
    let n = 1 + rng.below(30);
    let mut out = String::new();
    for _ in 0..n {
        out.push_str(rng.pick(TOKENS));
        out.push(' ');
    }
    out
}

fn char_soup(rng: &mut Lcg) -> String {
    let n = 1 + rng.below(80);
    (0..n)
        .map(|_| CHARS.get(rng.below(CHARS.len())).copied().unwrap_or('x'))
        .collect()
}

/// Grammar-shaped expressions, so most generated queries are accepted by the parser.
fn expr(rng: &mut Lcg, depth: usize) -> String {
    if depth == 0 || rng.below(3) == 0 {
        return atom(rng);
    }
    match rng.below(6) {
        0 => format!(
            "({} {} {})",
            expr(rng, depth - 1),
            op(rng),
            expr(rng, depth - 1)
        ),
        1 => format!("not {}", expr(rng, depth - 1)),
        2 => format!("{} and {}", expr(rng, depth - 1), expr(rng, depth - 1)),
        3 => format!("{} or {}", expr(rng, depth - 1), expr(rng, depth - 1)),
        4 => format!("{} + {}", atom(rng), atom(rng)),
        _ => format!("{} {} {}", atom(rng), op(rng), atom(rng)),
    }
}

fn op(rng: &mut Lcg) -> &'static str {
    [
        "=", "!=", "<", "<=", ">", ">=", "contains", "has", "in", "+", "-", "*", "/",
    ][rng.below(13)]
}

fn atom(rng: &mut Lcg) -> String {
    [
        "status",
        "due",
        "priority",
        "name",
        "mtime",
        "tags",
        "backlinks",
        "\"进行中\"",
        "3",
        "-2.5",
        "7d",
        "today()",
        "now()",
        "date(\"2026-02-28\")",
        "[\"a\", 1]",
        "null",
        "true",
        "len(tags)",
        "count(backlinks)",
        "lower(status)",
        "default(due, today())",
        "exists(priority)",
        "x",
    ][rng.below(23)]
    .to_string()
}

fn grammar_query(rng: &mut Lcg) -> String {
    let mut parts = Vec::new();
    if rng.below(2) == 0 {
        parts.push(format!(
            "from {}",
            [
                "all",
                "folder(\"项目\")",
                "tag(\"rust\")",
                "link_to(\"gpui\")",
                "not tag(\"x\")"
            ][rng.below(5)]
        ));
    }
    if rng.below(4) != 0 {
        parts.push(format!("where {}", expr(rng, 3)));
    }
    if rng.below(2) == 0 {
        parts.push("select name, status as s, due, count(backlinks) as b".to_string());
    }
    if rng.below(2) == 0 {
        parts.push(format!(
            "sort {} {}",
            atom(rng),
            if rng.below(2) == 0 { "asc" } else { "desc" }
        ));
    }
    if rng.below(3) == 0 {
        parts.push("group by status".to_string());
    }
    if rng.below(3) == 0 {
        parts.push(format!("limit {}", rng.below(20)));
    }
    parts.join(" ")
}

fn check_consistency(idx: &PropertyIndex, src: &str, ctx: &QueryContext) -> bool {
    let q = match parse(src) {
        Ok(q) => q,
        Err(_) => return false,
    };
    let r = execute(idx, &q, ctx);
    let grouped: usize = r.groups.iter().map(|g| g.rows.len()).sum();
    assert_eq!(
        grouped, r.shown,
        "shown must equal the rows in all groups: {src}"
    );
    assert!(r.shown <= r.total, "shown beyond total: {src}");
    for g in &r.groups {
        for row in &g.rows {
            assert_eq!(row.cells.len(), r.columns.len(), "cell count: {src}");
        }
        assert_eq!(
            g.aggregates.len(),
            r.columns.len(),
            "aggregate count: {src}"
        );
    }
    true
}

#[test]
fn fuzz_token_soup_never_panics() {
    let idx = vault();
    let ctx = common::ctx();
    let mut rng = Lcg(0x5EED_0001);
    let mut accepted = 0usize;
    for _ in 0..3000 {
        let src = token_soup(&mut rng);
        if check_consistency(&idx, &src, &ctx) {
            accepted += 1;
        }
    }
    // Token soup is mostly invalid; the point is that nothing panics.
    assert!(accepted < 3000);
}

#[test]
fn fuzz_character_soup_never_panics() {
    let idx = vault();
    let ctx = common::ctx();
    let mut rng = Lcg(0x5EED_0002);
    for _ in 0..3000 {
        let src = char_soup(&mut rng);
        let _ = check_consistency(&idx, &src, &ctx);
        let cursor = rng.below(src.len() + 3);
        let _ = suggest(&idx, &src, cursor);
    }
}

#[test]
fn fuzz_grammar_queries_execute_consistently() {
    let idx = vault();
    let ctx = common::ctx();
    let mut rng = Lcg(0x5EED_0003);
    let mut accepted = 0usize;
    for _ in 0..1500 {
        let src = grammar_query(&mut rng);
        if check_consistency(&idx, &src, &ctx) {
            accepted += 1;
        }
    }
    assert!(
        accepted > 500,
        "most grammar queries should parse, got {accepted}"
    );
}

#[test]
fn fuzz_suggest_at_every_cursor_position() {
    let idx = vault();
    let mut rng = Lcg(0x5EED_0004);
    for _ in 0..200 {
        let src = grammar_query(&mut rng);
        for cursor in 0..=src.len() + 2 {
            let _ = suggest(&idx, &src, cursor);
        }
    }
}

#[test]
fn fuzz_edits_with_random_text_round_trip() {
    let mut rng = Lcg(0x5EED_0005);
    for _ in 0..300 {
        let value = char_soup(&mut rng);
        let text = jiyue_query::plan_cell_edit(
            "---\nk: 1\n---\nbody\n",
            "k",
            &jiyue_query::Cell::Text(value.clone()),
        );
        let mut idx = PropertyIndex::new();
        idx.upsert("n.md", &text, 1, 1);
        let got = idx.get("n.md").and_then(|r| r.props.get("k").cloned());
        assert_eq!(
            got,
            Some(jiyue_query::Cell::Text(value.clone())),
            "value {value:?}"
        );
    }
}

#[test]
fn fuzz_index_random_operations_keep_counts_consistent() {
    let mut rng = Lcg(0x5EED_0006);
    let mut idx = PropertyIndex::new();
    let mut live: Vec<String> = Vec::new();
    for step in 0..2000i64 {
        match rng.below(4) {
            0 | 1 => {
                let path = format!("f{}/n{}.md", rng.below(4), rng.below(30));
                let text = format!(
                    "---\nk{}: {}\n---\n#t{}\n",
                    rng.below(3),
                    step,
                    rng.below(3)
                );
                idx.upsert(&path, &text, step, step);
                if !live.contains(&path) {
                    live.push(path);
                }
            }
            2 => {
                if let Some(p) = live.get(rng.below(live.len().max(1))).cloned() {
                    idx.remove(&p);
                    live.retain(|x| x != &p);
                }
            }
            _ => {
                if let Some(p) = live.get(rng.below(live.len().max(1))).cloned() {
                    let to = format!("moved/{}.md", rng.below(50));
                    if idx.rename(&p, &to) {
                        live.retain(|x| x != &p && x != &to);
                        live.push(to);
                    }
                }
            }
        }
    }
    assert_eq!(idx.len(), live.len());
    let total_tags: usize = idx.tag_counts().values().sum();
    let expected_tags: usize = idx.records().map(|r| r.tags.len()).sum();
    assert_eq!(total_tags, expected_tags);
    let total_props: usize = idx.property_counts().values().sum();
    let expected_props: usize = idx.records().map(|r| r.props.len()).sum();
    assert_eq!(total_props, expected_props);
}
