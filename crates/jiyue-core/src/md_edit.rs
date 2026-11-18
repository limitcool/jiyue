//! Markdown-aware editing helpers (pure functions on text + selection, no UI).
//! Owned by the markdown-edit agent.
//!
//! Every public editing function takes the text plus a selection given as UTF-8
//! byte offsets and returns `Option<EditPlan>`. `None` means "no special
//! behaviour": the editor performs its default action.
//!
//! Offset convention: `EditPlan::start` and `EditPlan::end` are byte offsets into
//! the ORIGINAL text. `sel_anchor` and `sel_head` are byte offsets into the text
//! AFTER the plan has been applied, i.e. the selection to show afterwards.

/// One text replacement plus the selection to show afterwards.
///
/// `start..end` is replaced by `insert` (offsets in the original text).
/// `sel_anchor` / `sel_head` are offsets in the resulting text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPlan {
    pub start: usize,
    pub end: usize,
    pub insert: String,
    pub sel_anchor: usize,
    pub sel_head: usize,
}

/// What a line starts with, as far as list and quote handling is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// `- `, `* `, `+ ` (the char is the bullet).
    Bullet(char),
    /// `1. ` or `1) `.
    Ordered { number: u64, delim: char },
    /// `- [ ] `, `- [x] `, `- [X] `.
    Task { checked: bool, bullet: char },
    /// `> `, `> > ` (depth = number of `>`).
    Quote { depth: usize },
}

/// A list or quote marker found at the start of a line.
///
/// `indent` is the number of leading space/tab bytes before the marker.
/// `marker_len_bytes` is the byte length from the line start through the space
/// after the marker. For a quoted list item (`> - x`) the item is reported and
/// the length includes the quote prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListMarker {
    pub indent: usize,
    pub kind: ListKind,
    pub marker_len_bytes: usize,
}

const INDENT: &str = "    ";

/// Opener to closer. Symmetric pairs (`"`, `` ` ``, `*`, `_`, `~`) use the same char on both sides.
const PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('"', '"'),
    ('`', '`'),
    ('*', '*'),
    ('_', '_'),
    ('~', '~'),
    ('（', '）'),
    ('【', '】'),
    ('「', '」'),
    ('《', '》'),
    ('“', '”'),
    ('‘', '’'),
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Enter inside a list, quote, table, or fenced code block.
pub fn on_enter(text: &str, sel_start: usize, sel_end: usize) -> Option<EditPlan> {
    let (a, b) = normalize_selection(text, sel_start, sel_end);
    let (ls, le) = line_bounds(text, a);
    let line = &text[ls..le];
    let nl = newline_at(text, le);

    if is_in_fenced_code(text, a) {
        return Some(replace_with(a, b, format!("{nl}{}", leading_ws(line))));
    }

    if a == b
        && let Some(plan) = table_enter(text, a, ls, le, nl)
    {
        return Some(plan);
    }

    let p = parse_line(line);
    if p.item.is_none() && p.quote_depth == 0 {
        let ws = leading_ws(line);
        if ws.is_empty() {
            return None;
        }
        return Some(replace_with(a, b, format!("{nl}{ws}")));
    }

    // Caret inside the marker itself: let the editor do its default thing.
    if a - ls < p.item_end {
        return None;
    }

    let rest = &line[p.item_end..];
    if rest.trim().is_empty() {
        if p.item.is_none() {
            // Empty quote line: leave the quote.
            return Some(replace_with(ls, le, String::new()));
        }
        if p.quote_depth == 0 && p.indent > 0 {
            // Empty indented item: outdent one level, keep the marker.
            let r = outdent_width(&line[..p.indent]);
            return Some(replace_with(ls, le, line[r..].to_string()));
        }
        // Empty item: drop the marker (the quote prefix, if any, stays).
        let from = if p.quote_depth == 0 {
            ls
        } else {
            ls + p.quote_end
        };
        return Some(replace_with(from, le, String::new()));
    }

    let mut cont = String::new();
    cont.push_str(nl);
    cont.push_str(&line[..p.indent]);
    if p.quote_depth > 0 {
        cont.push_str(&line[p.indent..p.quote_end]);
    }
    match p.item {
        Some(ListKind::Bullet(c)) => {
            cont.push(c);
            cont.push(' ');
        }
        Some(ListKind::Task { bullet, .. }) => {
            cont.push(bullet);
            cont.push_str(" [ ] ");
        }
        Some(ListKind::Ordered { number, delim }) => {
            cont.push_str(&number.saturating_add(1).to_string());
            cont.push(delim);
            cont.push(' ');
        }
        _ => {}
    }

    // Splitting mid-item: the tail moves to the new line, without its leading blanks.
    let bytes = text.as_bytes();
    let mut end = b;
    if b <= le {
        while end < le && is_blank(bytes[end]) {
            end += 1;
        }
    }
    Some(replace_with(a, end, cont))
}

/// Tab / Shift+Tab: navigate GFM tables, indent or outdent lines, or insert four spaces at a plain caret.
pub fn on_tab(text: &str, sel_start: usize, sel_end: usize, shift: bool) -> Option<EditPlan> {
    let (a, b) = normalize_selection(text, sel_start, sel_end);
    let caret = a == b;
    let (caret_ls, caret_le) = line_bounds(text, a);

    if (caret || line_start(text, b) == caret_ls)
        && let Some(plan) = table_tab(text, a, shift)
    {
        return Some(plan);
    }

    if caret && !shift && parse_line(&text[caret_ls..caret_le]).item.is_none() {
        let pos = a + INDENT.len();
        return Some(plan(a, a, INDENT.to_string(), pos, pos));
    }

    let first = caret_ls;
    let last = if !caret && line_start(text, b) == b {
        // Selection ends exactly at a line start: that line is not included.
        line_start(text, b.saturating_sub(1))
    } else {
        line_start(text, b)
    };

    let mut out = String::new();
    let mut edits: Vec<(usize, usize, usize)> = Vec::new(); // (line start, removed, inserted)
    let mut changed = false;
    let mut pos = first;
    let mut ls = first;
    loop {
        let (le, next) = line_info(text, ls);
        let line = &text[ls..le];
        out.push_str(&text[pos..ls]);
        if shift {
            let removed = outdent_width(leading_ws(line));
            changed |= removed > 0;
            out.push_str(&line[removed..]);
            edits.push((ls, removed, 0));
        } else {
            changed = true;
            out.push_str(INDENT);
            out.push_str(line);
            edits.push((ls, 0, INDENT.len()));
        }
        pos = le;
        if ls >= last {
            break;
        }
        match next {
            Some(n) => ls = n,
            None => break,
        }
    }
    if !changed {
        return None;
    }
    Some(plan(
        first,
        pos,
        out,
        map_offset(a, &edits),
        map_offset(b, &edits),
    ))
}

/// Typed character: auto-pair, wrap a selection, or skip over a closer.
pub fn on_typed_char(text: &str, sel_start: usize, sel_end: usize, ch: char) -> Option<EditPlan> {
    let (a, b) = normalize_selection(text, sel_start, sel_end);
    let in_fence = is_in_fenced_code(text, a);
    if in_fence && !fence_allowed(ch) {
        return None;
    }

    // Skip over an identical closer that is already next to the caret.
    if a == b && is_closer(ch) && text[a..].starts_with(ch) {
        let caret = a + ch.len_utf8();
        return Some(plan(a, a, String::new(), caret, caret));
    }

    let close = pair_close(ch)?;

    if a < b {
        let sel = &text[a..b];
        let mut insert = String::with_capacity(sel.len() + ch.len_utf8() + close.len_utf8());
        insert.push(ch);
        insert.push_str(sel);
        insert.push(close);
        let anchor = a + ch.len_utf8();
        return Some(plan(a, b, insert, anchor, anchor + (b - a)));
    }

    let prev = text[..a].chars().next_back();
    let next = text[a..].chars().next();

    // `[` typed inside `[|]` gives `[[|]]`.
    if ch == '[' && prev == Some('[') && next == Some(']') {
        let caret = a + 1;
        return Some(plan(a, a, "[]".to_string(), caret, caret));
    }

    if ch == close {
        // Symmetric pair: do not pair inside words (don't, a"b) or next to a word.
        if prev.is_some_and(char::is_alphanumeric) || next.is_some_and(char::is_alphanumeric) {
            return None;
        }
        if matches!(ch, '*' | '_' | '~') {
            let ok = match prev {
                None => true,
                Some(c) => c.is_whitespace() || is_opener(c),
            };
            if !ok {
                return None;
            }
        }
    }

    let mut insert = String::new();
    insert.push(ch);
    insert.push(close);
    let caret = a + ch.len_utf8();
    Some(plan(a, a, insert, caret, caret))
}

/// Backspace: delete an empty pair, or remove a list marker at the start of its text.
pub fn on_backspace(text: &str, sel_start: usize, sel_end: usize) -> Option<EditPlan> {
    let (a, b) = normalize_selection(text, sel_start, sel_end);
    if a != b {
        return None;
    }
    let before = &text[..a];
    let after = &text[a..];

    if before.ends_with("[[") && after.starts_with("]]") {
        return Some(replace_with(a - 2, a + 2, String::new()));
    }
    if let (Some(open_ch), Some(next_ch)) = (before.chars().next_back(), after.chars().next())
        && pair_close(open_ch) == Some(next_ch)
    {
        return Some(replace_with(
            a - open_ch.len_utf8(),
            a + next_ch.len_utf8(),
            String::new(),
        ));
    }

    let (ls, le) = line_bounds(text, a);
    let line = &text[ls..le];
    let p = parse_line(line);
    if p.item.is_none() || a - ls != p.item_end {
        return None;
    }
    if p.quote_depth > 0 {
        let from = ls + p.quote_end;
        return Some(replace_with(from, a, String::new()));
    }
    if p.indent > 0 {
        let r = outdent_width(&line[..p.indent]);
        let caret = a - r;
        return Some(plan(ls, ls + r, String::new(), caret, caret));
    }
    Some(replace_with(ls, a, String::new()))
}

/// Paste: wrap a selection as a link when pasting a URL, normalise CRLF otherwise.
pub fn on_paste(text: &str, sel_start: usize, sel_end: usize, pasted: &str) -> Option<EditPlan> {
    let (a, b) = normalize_selection(text, sel_start, sel_end);
    let normalized = pasted.replace("\r\n", "\n");
    let url = normalized.trim();
    if a < b && is_single_line_url(url) {
        let selected = &text[a..b];
        if !selected.contains("](") {
            let insert = format!("[{selected}]({url})");
            return Some(replace_with(a, b, insert));
        }
    }
    if normalized != pasted {
        return Some(replace_with(a, b, normalized));
    }
    None
}

/// Flip the checkbox of the task item on the line at `offset`.
pub fn toggle_task(text: &str, offset: usize) -> Option<EditPlan> {
    let o = clamp_boundary(text, offset);
    let (ls, le) = line_bounds(text, o);
    let p = parse_line(&text[ls..le]);
    let checked = match p.item {
        Some(ListKind::Task { checked, .. }) => checked,
        _ => return None,
    };
    // Item starts at quote_end; `-`, space, `[`, then the state char.
    let state = ls + p.quote_end + 3;
    let insert = if checked { " " } else { "x" };
    Some(plan(state, state + 1, insert.to_string(), o, o))
}

/// Byte range `(start, end)` of the line containing `offset`, excluding the line
/// terminator (`\n` or `\r\n`).
pub fn line_bounds(text: &str, offset: usize) -> (usize, usize) {
    let start = line_start(text, offset);
    let (end, _) = line_info(text, start);
    (start, end)
}

/// True if `offset` is inside a fenced code block. Fence lines themselves count as inside.
pub fn is_in_fenced_code(text: &str, offset: usize) -> bool {
    let ls = line_start(text, offset);
    let (le, _) = line_info(text, ls);
    let mut inside = false;
    for line in text[..ls].split('\n') {
        if is_fence(line) {
            inside = !inside;
        }
    }
    if is_fence(&text[ls..le]) {
        return true;
    }
    inside
}

/// Detects a list or quote marker at the start of `line` (no line terminator).
pub fn list_marker(line: &str) -> Option<ListMarker> {
    let p = parse_line(line);
    if let Some(kind) = p.item {
        Some(ListMarker {
            indent: p.indent,
            kind,
            marker_len_bytes: p.item_end,
        })
    } else if p.quote_depth > 0 {
        Some(ListMarker {
            indent: p.indent,
            kind: ListKind::Quote {
                depth: p.quote_depth,
            },
            marker_len_bytes: p.quote_end,
        })
    } else {
        None
    }
}

/// True when the non-empty single-line selection is wrapped with `(open, close)`, either around
/// the selection (`**sel**`) or at its own ends (`⟦**sel**⟧`).
pub fn has_wrap(text: &str, sel_start: usize, sel_end: usize, open: &str, close: &str) -> bool {
    if open.is_empty() || close.is_empty() {
        return false;
    }
    let (a, b) = normalize_selection(text, sel_start, sel_end);
    if a >= b || is_in_fenced_code(text, a) || text[a..b].contains('\n') {
        return false;
    }
    has_outer_wrap(&text[..a], &text[b..], open, close) || has_inner_wrap(&text[a..b], open, close)
}

/// Toggles an inline marker pair `(open, close)` around the caret or single-line selection.
pub fn toggle_wrap(
    text: &str,
    sel_start: usize,
    sel_end: usize,
    open: &str,
    close: &str,
) -> Option<EditPlan> {
    if open.is_empty() || close.is_empty() {
        return None;
    }
    let (a, b) = normalize_selection(text, sel_start, sel_end);
    if is_in_fenced_code(text, a) || text[a..b].contains('\n') {
        return None;
    }
    if a == b {
        if has_outer_wrap(&text[..a], &text[a..], open, close) {
            let new_pos = a - open.len();
            return Some(plan(
                new_pos,
                a + close.len(),
                String::new(),
                new_pos,
                new_pos,
            ));
        }
        let caret = a + open.len();
        return Some(plan(a, a, format!("{open}{close}"), caret, caret));
    }

    let sel = &text[a..b];
    if has_outer_wrap(&text[..a], &text[b..], open, close) {
        let new_start = a - open.len();
        let new_end = new_start + sel.len();
        return Some(plan(
            new_start,
            b + close.len(),
            sel.to_string(),
            new_start,
            new_end,
        ));
    }
    if has_inner_wrap(sel, open, close) {
        let inner = &sel[open.len()..sel.len() - close.len()];
        return Some(plan(a, b, inner.to_string(), a, a + inner.len()));
    }
    let anchor = a + open.len();
    Some(plan(
        a,
        b,
        format!("{open}{sel}{close}"),
        anchor,
        anchor + sel.len(),
    ))
}

// ---------------------------------------------------------------------------
// Inline wrap & GFM table helpers
// ---------------------------------------------------------------------------

fn has_outer_wrap(before: &str, after: &str, open: &str, close: &str) -> bool {
    if open == "*" && close == "*" {
        let tri = before.ends_with("***") && after.starts_with("***");
        let single = before.ends_with('*')
            && !before.ends_with("**")
            && after.starts_with('*')
            && !after.starts_with("**");
        return tri || single;
    }
    before.ends_with(open) && after.starts_with(close)
}

fn has_inner_wrap(sel: &str, open: &str, close: &str) -> bool {
    if open == "*" && close == "*" {
        let tri = sel.len() >= 6 && sel.starts_with("***") && sel.ends_with("***");
        let single = sel.len() >= 2
            && sel.starts_with('*')
            && !sel.starts_with("**")
            && sel.ends_with('*')
            && !sel.ends_with("**");
        return tri || single;
    }
    sel.len() >= open.len() + close.len() && sel.starts_with(open) && sel.ends_with(close)
}

struct TableCtx {
    rows: Vec<(usize, usize)>,
    row_idx: usize,
    col_count: usize,
    indent: String,
}

fn top_level_pipes(line: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut pipes = Vec::new();
    let mut in_code = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if in_code == 0 && bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == b'`' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            let ticks = i - start;
            if in_code == 0 {
                in_code = ticks;
            } else if in_code == ticks {
                in_code = 0;
            }
            continue;
        }
        if in_code == 0 && bytes[i] == b'|' {
            pipes.push(i);
        }
        i += 1;
    }
    pipes
}

fn split_pipe_cells(line: &str) -> Option<Vec<(usize, usize)>> {
    let pipes = top_level_pipes(line);
    if pipes.is_empty() {
        return None;
    }
    let first = pipes[0];
    let last = *pipes.last()?;
    let has_leading = line[..first].trim().is_empty();
    let has_trailing = line[last + 1..].trim().is_empty();
    let mut cells = Vec::new();
    if !has_leading {
        cells.push((0, first));
    }
    for pair in pipes.windows(2) {
        cells.push((pair[0] + 1, pair[1]));
    }
    if !has_trailing {
        cells.push((last + 1, line.len()));
    }
    (!cells.is_empty()).then_some(cells)
}

fn is_table_delimiter_line(line: &str) -> Option<usize> {
    let cells = split_pipe_cells(line)?;
    for (s, e) in &cells {
        let seg = line[*s..*e].trim();
        let inner = seg.strip_prefix(':').unwrap_or(seg);
        let inner = inner.strip_suffix(':').unwrap_or(inner);
        if inner.is_empty() || !inner.bytes().all(|b| b == b'-') {
            return None;
        }
    }
    Some(cells.len())
}

fn is_pipe_table_row(line: &str) -> bool {
    !line.trim().is_empty() && !is_fence(line) && split_pipe_cells(line).is_some()
}

fn table_at(text: &str, pos: usize) -> Option<TableCtx> {
    if is_in_fenced_code(text, pos) {
        return None;
    }
    let (cur_ls, cur_le) = line_bounds(text, pos);
    if !is_pipe_table_row(&text[cur_ls..cur_le]) {
        return None;
    }
    let mut above = Vec::new();
    let mut scan = cur_ls;
    while scan > 0 {
        let prev = line_start(text, scan - 1);
        let (pe, _) = line_info(text, prev);
        if !is_pipe_table_row(&text[prev..pe]) {
            break;
        }
        above.push((prev, pe));
        scan = prev;
    }
    above.reverse();
    let row_idx = above.len();
    let mut rows = above;
    rows.push((cur_ls, cur_le));
    let mut next_opt = line_info(text, cur_ls).1;
    while let Some(ns) = next_opt {
        let (ne, next2) = line_info(text, ns);
        if !is_pipe_table_row(&text[ns..ne]) {
            break;
        }
        rows.push((ns, ne));
        next_opt = next2;
    }
    if rows.len() < 2 {
        return None;
    }
    let header = &text[rows[0].0..rows[0].1];
    let delim = &text[rows[1].0..rows[1].1];
    if is_table_delimiter_line(header).is_some() {
        return None;
    }
    let col_count = is_table_delimiter_line(delim)?;
    if split_pipe_cells(header)?.len() != col_count {
        return None;
    }
    Some(TableCtx {
        indent: leading_ws(header).to_string(),
        rows,
        row_idx,
        col_count,
    })
}

fn empty_table_row(indent: &str, col_count: usize) -> String {
    let mut out = String::with_capacity(indent.len() + 1 + col_count * 3);
    out.push_str(indent);
    out.push('|');
    for _ in 0..col_count {
        out.push_str("  |");
    }
    out
}

fn cell_selection(line: &str, ls: usize, raw: (usize, usize)) -> (usize, usize) {
    let seg = &line[raw.0..raw.1];
    let trimmed = seg.trim();
    if !trimmed.is_empty() {
        let lead = seg.len() - seg.trim_start().len();
        let a = ls + raw.0 + lead;
        return (a, a + trimmed.len());
    }
    let off = if raw.1 - raw.0 >= 2 { raw.0 + 1 } else { raw.1 };
    (ls + off, ls + off)
}

fn col_index_at(cells: &[(usize, usize)], col_byte: usize) -> usize {
    cells
        .iter()
        .position(|&(_, end)| col_byte <= end)
        .unwrap_or_else(|| cells.len().saturating_sub(1))
}

fn table_tab(text: &str, caret: usize, shift: bool) -> Option<EditPlan> {
    let ctx = table_at(text, caret)?;
    let (ls, le) = ctx.rows[ctx.row_idx];
    let line = &text[ls..le];
    let cells = split_pipe_cells(line)?;
    let col_idx = col_index_at(&cells, caret.saturating_sub(ls));

    if !shift {
        if col_idx + 1 < cells.len() {
            let (a, b) = cell_selection(line, ls, cells[col_idx + 1]);
            return Some(plan(0, 0, String::new(), a, b));
        }
        let next_row = if ctx.row_idx == 0 { 2 } else { ctx.row_idx + 1 };
        if let Some(&(nls, nle)) = ctx.rows.get(next_row) {
            let nline = &text[nls..nle];
            let ncells = split_pipe_cells(nline)?;
            let (a, b) = cell_selection(nline, nls, ncells[0]);
            return Some(plan(0, 0, String::new(), a, b));
        }
        let last_le = ctx.rows.last()?.1;
        let nl = newline_at(text, last_le);
        let row = empty_table_row(&ctx.indent, ctx.col_count);
        let target = last_le + nl.len() + ctx.indent.len() + 2;
        return Some(plan(last_le, last_le, format!("{nl}{row}"), target, target));
    }

    if col_idx > 0 {
        let (a, b) = cell_selection(line, ls, cells[col_idx - 1]);
        return Some(plan(0, 0, String::new(), a, b));
    }
    if ctx.row_idx == 0 {
        let (a, b) = cell_selection(line, ls, cells[0]);
        return Some(plan(0, 0, String::new(), a, b));
    }
    let prev_row = if ctx.row_idx <= 2 { 0 } else { ctx.row_idx - 1 };
    let (pls, ple) = ctx.rows[prev_row];
    let pline = &text[pls..ple];
    let pcells = split_pipe_cells(pline)?;
    let (a, b) = cell_selection(pline, pls, *pcells.last()?);
    Some(plan(0, 0, String::new(), a, b))
}

fn table_enter(
    text: &str,
    caret: usize,
    ls: usize,
    le: usize,
    nl: &'static str,
) -> Option<EditPlan> {
    let ctx = table_at(text, caret)?;
    let line = &text[ls..le];
    if ctx.row_idx >= 2
        && let Some(cells) = split_pipe_cells(line)
        && cells.iter().all(|&(s, e)| line[s..e].trim().is_empty())
    {
        return Some(replace_with(ls, le, String::new()));
    }
    let anchor_le = if ctx.row_idx == 0 { ctx.rows[1].1 } else { le };
    let anchor_nl = newline_at(text, anchor_le);
    let _ = nl;
    let row = empty_table_row(&ctx.indent, ctx.col_count);
    let target = anchor_le + anchor_nl.len() + ctx.indent.len() + 2;
    Some(plan(
        anchor_le,
        anchor_le,
        format!("{anchor_nl}{row}"),
        target,
        target,
    ))
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn plan(start: usize, end: usize, insert: String, sel_anchor: usize, sel_head: usize) -> EditPlan {
    EditPlan {
        start,
        end,
        insert,
        sel_anchor,
        sel_head,
    }
}

/// Replacement whose resulting caret sits right after the inserted text.
fn replace_with(start: usize, end: usize, insert: String) -> EditPlan {
    let caret = start + insert.len();
    plan(start, end, insert, caret, caret)
}

/// Clamps to the text length and moves down to a char boundary. Never panics.
fn clamp_boundary(text: &str, offset: usize) -> usize {
    let mut o = offset.min(text.len());
    while o > 0 && !text.is_char_boundary(o) {
        o -= 1;
    }
    o
}

fn normalize_selection(text: &str, sel_start: usize, sel_end: usize) -> (usize, usize) {
    let s = clamp_boundary(text, sel_start);
    let e = clamp_boundary(text, sel_end);
    if s <= e { (s, e) } else { (e, s) }
}

fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Start of the line containing `offset`.
fn line_start(text: &str, offset: usize) -> usize {
    let o = clamp_boundary(text, offset);
    text.as_bytes()[..o]
        .iter()
        .rposition(|&c| c == b'\n')
        .map_or(0, |i| i + 1)
}

/// For a line starting at `start`: (end without terminator, start of the next line).
fn line_info(text: &str, start: usize) -> (usize, Option<usize>) {
    let bytes = text.as_bytes();
    let start = start.min(bytes.len());
    match bytes[start..].iter().position(|&c| c == b'\n') {
        None => (bytes.len(), None),
        Some(i) => {
            let nl = start + i;
            let end = if nl > start && bytes[nl - 1] == b'\r' {
                nl - 1
            } else {
                nl
            };
            (end, Some(nl + 1))
        }
    }
}

/// Line terminator to use when splitting the line that ends at `line_end`.
fn newline_at(text: &str, line_end: usize) -> &'static str {
    if text.as_bytes().get(line_end) == Some(&b'\r') {
        "\r\n"
    } else {
        "\n"
    }
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

fn leading_ws(line: &str) -> &str {
    let n = line.bytes().take_while(|&c| is_blank(c)).count();
    &line[..n]
}

/// How many bytes an outdent removes from leading whitespace `ws`: one tab, or up to four spaces.
fn outdent_width(ws: &str) -> usize {
    let bytes = ws.as_bytes();
    match bytes.first() {
        Some(b'\t') => 1,
        Some(b' ') => bytes
            .iter()
            .take(INDENT.len())
            .take_while(|&&c| c == b' ')
            .count(),
        _ => 0,
    }
}

/// Maps an original offset into the text after a set of line edits.
/// Each edit is `(line start, bytes removed at line start, bytes inserted at line start)`.
fn map_offset(x: usize, edits: &[(usize, usize, usize)]) -> usize {
    let mut out = x as isize;
    for &(p, removed, inserted) in edits {
        if x >= p {
            let consumed = removed.min(x - p);
            out += inserted as isize - consumed as isize;
        }
    }
    out.max(0) as usize
}

fn pair_close(open: char) -> Option<char> {
    PAIRS.iter().find(|&&(o, _)| o == open).map(|&(_, c)| c)
}

fn is_opener(ch: char) -> bool {
    PAIRS.iter().any(|&(o, _)| o == ch)
}

fn is_closer(ch: char) -> bool {
    PAIRS.iter().any(|&(_, c)| c == ch)
}

/// Characters that may be auto-paired or skipped inside fenced code blocks.
fn fence_allowed(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '"'
            | '（'
            | '）'
            | '【'
            | '】'
            | '「'
            | '」'
            | '《'
            | '》'
            | '“'
            | '”'
            | '‘'
            | '’'
    )
}

fn is_single_line_url(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let scheme_len = if lower.starts_with("https://") {
        8
    } else if lower.starts_with("http://") {
        7
    } else {
        return false;
    };
    s.len() > scheme_len && !s.chars().any(char::is_whitespace)
}

fn skip_blanks(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && is_blank(bytes[i]) {
        i += 1;
    }
    i
}

/// Structure of a line prefix: indentation, quote markers, optional list item.
#[derive(Debug, Clone, Copy)]
struct Parsed {
    /// Leading blank bytes before the first marker.
    indent: usize,
    quote_depth: usize,
    /// Byte index where the quote prefix ends (equals `indent` without quotes).
    quote_end: usize,
    item: Option<ListKind>,
    /// Byte index where the item text starts. Equals `quote_end` when there is no item.
    item_end: usize,
}

fn parse_line(line: &str) -> Parsed {
    let bytes = line.as_bytes();
    let indent = skip_blanks(bytes, 0);
    let mut i = indent;
    let mut quote_depth = 0;
    while i < bytes.len() && bytes[i] == b'>' {
        quote_depth += 1;
        i = skip_blanks(bytes, i + 1);
    }
    let quote_end = i;
    let (item, item_end) = match parse_item(bytes, quote_end) {
        Some((kind, end)) => (Some(kind), end),
        None => (None, quote_end),
    };
    Parsed {
        indent,
        quote_depth,
        quote_end,
        item,
        item_end,
    }
}

/// Parses a list item starting at `i`. Returns the kind and the byte index after its marker
/// (including one following blank, when present).
fn parse_item(bytes: &[u8], i: usize) -> Option<(ListKind, usize)> {
    let n = bytes.len();
    let c = *bytes.get(i)?;
    if matches!(c, b'-' | b'*' | b'+') {
        let bullet = c as char;
        if i + 1 == n {
            return Some((ListKind::Bullet(bullet), n));
        }
        if !is_blank(bytes[i + 1]) {
            return None;
        }
        if let Some((checked, box_end)) = task_box(bytes, i + 2) {
            if box_end == n {
                return Some((ListKind::Task { checked, bullet }, n));
            }
            if is_blank(bytes[box_end]) {
                return Some((ListKind::Task { checked, bullet }, box_end + 1));
            }
        }
        return Some((ListKind::Bullet(bullet), i + 2));
    }
    if c.is_ascii_digit() {
        let mut j = i;
        while j < n && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j < n && (bytes[j] == b'.' || bytes[j] == b')') && (j + 1 == n || is_blank(bytes[j + 1]))
        {
            let number: u64 = std::str::from_utf8(&bytes[i..j]).ok()?.parse().ok()?;
            let delim = bytes[j] as char;
            let end = if j + 1 < n { j + 2 } else { j + 1 };
            return Some((ListKind::Ordered { number, delim }, end));
        }
    }
    None
}

/// `[ ]`, `[x]` or `[X]` at `p`. Returns (checked, index after the `]`).
fn task_box(bytes: &[u8], p: usize) -> Option<(bool, usize)> {
    if p + 3 <= bytes.len() && bytes[p] == b'[' && bytes[p + 2] == b']' {
        let checked = match bytes[p + 1] {
            b' ' => false,
            b'x' | b'X' => true,
            _ => return None,
        };
        Some((checked, p + 3))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// `|` marks a caret, `⟦` and `⟧` mark a selection start and end.
    fn parse(s: &str) -> (String, usize, usize) {
        let mut text = String::new();
        let mut start = None;
        let mut end = None;
        for ch in s.chars() {
            match ch {
                '|' => {
                    start = Some(text.len());
                    end = Some(text.len());
                }
                '⟦' => start = Some(text.len()),
                '⟧' => end = Some(text.len()),
                _ => text.push(ch),
            }
        }
        let s0 = start.unwrap_or(0);
        (text, s0, end.unwrap_or(s0))
    }

    /// Applies the plan and renders the result with the same markers as `parse`.
    fn render(text: &str, p: &EditPlan) -> String {
        let mut out = String::new();
        out.push_str(&text[..p.start]);
        out.push_str(&p.insert);
        out.push_str(&text[p.end..]);
        let lo = p.sel_anchor.min(p.sel_head);
        let hi = p.sel_anchor.max(p.sel_head);
        if lo == hi {
            out.insert(lo, '|');
        } else {
            out.insert(hi, '⟧');
            out.insert(lo, '⟦');
        }
        out
    }

    fn enter(s: &str) -> Option<String> {
        let (t, a, b) = parse(s);
        on_enter(&t, a, b).map(|p| render(&t, &p))
    }

    fn tab(s: &str) -> Option<String> {
        let (t, a, b) = parse(s);
        on_tab(&t, a, b, false).map(|p| render(&t, &p))
    }

    fn shift_tab(s: &str) -> Option<String> {
        let (t, a, b) = parse(s);
        on_tab(&t, a, b, true).map(|p| render(&t, &p))
    }

    fn typed(s: &str, ch: char) -> Option<String> {
        let (t, a, b) = parse(s);
        on_typed_char(&t, a, b, ch).map(|p| render(&t, &p))
    }

    fn backspace(s: &str) -> Option<String> {
        let (t, a, b) = parse(s);
        on_backspace(&t, a, b).map(|p| render(&t, &p))
    }

    fn paste(s: &str, pasted: &str) -> Option<String> {
        let (t, a, b) = parse(s);
        on_paste(&t, a, b, pasted).map(|p| render(&t, &p))
    }

    fn toggle(s: &str) -> Option<String> {
        let (t, a, _) = parse(s);
        toggle_task(&t, a).map(|p| render(&t, &p))
    }

    // ---- on_enter: lists and quotes -------------------------------------

    #[test]
    fn enter_continues_dash_bullet() {
        assert_eq!(enter("- foo|").as_deref(), Some("- foo\n- |"));
    }

    #[test]
    fn enter_continues_star_bullet() {
        assert_eq!(enter("* foo|").as_deref(), Some("* foo\n* |"));
    }

    #[test]
    fn enter_continues_plus_bullet() {
        assert_eq!(enter("+ foo|").as_deref(), Some("+ foo\n+ |"));
    }

    #[test]
    fn enter_empty_bullet_removes_marker() {
        assert_eq!(enter("- |").as_deref(), Some("|"));
    }

    #[test]
    fn enter_bare_dash_removes_marker() {
        assert_eq!(enter("-|").as_deref(), Some("|"));
    }

    #[test]
    fn enter_empty_indented_bullet_outdents_spaces() {
        assert_eq!(enter("    - |").as_deref(), Some("- |"));
    }

    #[test]
    fn enter_empty_indented_bullet_outdents_tab() {
        assert_eq!(enter("\t- |").as_deref(), Some("- |"));
    }

    #[test]
    fn enter_keeps_bullet_indentation() {
        assert_eq!(enter("  - foo|").as_deref(), Some("  - foo\n  - |"));
    }

    #[test]
    fn enter_task_continues_unchecked_after_checked() {
        assert_eq!(enter("- [x] done|").as_deref(), Some("- [x] done\n- [ ] |"));
    }

    #[test]
    fn enter_task_continues_unchecked() {
        assert_eq!(enter("- [ ] todo|").as_deref(), Some("- [ ] todo\n- [ ] |"));
    }

    #[test]
    fn enter_empty_task_removes_marker() {
        assert_eq!(enter("- [ ] |").as_deref(), Some("|"));
    }

    #[test]
    fn enter_ordered_increments_number() {
        assert_eq!(enter("1. one|").as_deref(), Some("1. one\n2. |"));
    }

    #[test]
    fn enter_ordered_nine_to_ten() {
        assert_eq!(enter("9. nine|").as_deref(), Some("9. nine\n10. |"));
    }

    #[test]
    fn enter_ordered_paren_delimiter_kept() {
        assert_eq!(enter("1) one|").as_deref(), Some("1) one\n2) |"));
    }

    #[test]
    fn enter_empty_ordered_removes_marker() {
        assert_eq!(enter("3. |").as_deref(), Some("|"));
    }

    #[test]
    fn enter_empty_nested_ordered_outdents() {
        assert_eq!(enter("  1. |").as_deref(), Some("1. |"));
    }

    #[test]
    fn enter_quote_continues() {
        assert_eq!(enter("> quote|").as_deref(), Some("> quote\n> |"));
    }

    #[test]
    fn enter_nested_quote_continues() {
        assert_eq!(enter("> > deep|").as_deref(), Some("> > deep\n> > |"));
    }

    #[test]
    fn enter_empty_quote_exits() {
        assert_eq!(enter("> |").as_deref(), Some("|"));
    }

    #[test]
    fn enter_quoted_bullet_continues() {
        assert_eq!(enter("> - item|").as_deref(), Some("> - item\n> - |"));
    }

    #[test]
    fn enter_empty_quoted_bullet_keeps_quote() {
        assert_eq!(enter("> - |").as_deref(), Some("> |"));
    }

    #[test]
    fn enter_quoted_task_continues() {
        assert_eq!(enter("> - [ ] x|").as_deref(), Some("> - [ ] x\n> - [ ] |"));
    }

    #[test]
    fn enter_quote_without_space_continues() {
        assert_eq!(enter(">x|").as_deref(), Some(">x\n>|"));
    }

    #[test]
    fn enter_mid_item_moves_tail_to_new_item() {
        assert_eq!(enter("- foo|bar").as_deref(), Some("- foo\n- |bar"));
    }

    #[test]
    fn enter_mid_item_drops_blank_before_tail() {
        assert_eq!(enter("- foo| bar").as_deref(), Some("- foo\n- |bar"));
    }

    #[test]
    fn enter_mid_task_moves_tail() {
        assert_eq!(enter("- [ ] ab|cd").as_deref(), Some("- [ ] ab\n- [ ] |cd"));
    }

    #[test]
    fn enter_selection_replaced_by_continuation() {
        assert_eq!(enter("- ⟦foo⟧").as_deref(), Some("- \n- |"));
    }

    #[test]
    fn enter_selection_inside_item_keeps_tail() {
        assert_eq!(enter("- ⟦fo⟧o").as_deref(), Some("- \n- |o"));
    }

    #[test]
    fn enter_second_line_item_continues() {
        assert_eq!(
            enter("# title\n- a|\n").as_deref(),
            Some("# title\n- a\n- |\n")
        );
    }

    #[test]
    fn enter_empty_bullet_on_second_line_removes_marker() {
        assert_eq!(enter("- a\n- |").as_deref(), Some("- a\n|"));
    }

    #[test]
    fn enter_unicode_item_text() {
        assert_eq!(enter("- 你好|").as_deref(), Some("- 你好\n- |"));
    }

    #[test]
    fn enter_caret_before_marker_is_default() {
        assert_eq!(enter("|- foo"), None);
    }

    // ---- on_enter: fences and plain paragraphs --------------------------

    #[test]
    fn enter_in_fence_plain_newline() {
        assert_eq!(
            enter("```\ncode|\n```").as_deref(),
            Some("```\ncode\n|\n```")
        );
    }

    #[test]
    fn enter_in_fence_ignores_list_markers() {
        assert_eq!(
            enter("```\n- item|\n```").as_deref(),
            Some("```\n- item\n|\n```")
        );
    }

    #[test]
    fn enter_in_fence_keeps_indent() {
        assert_eq!(
            enter("```\n    x = 1|\n```").as_deref(),
            Some("```\n    x = 1\n    |\n```")
        );
    }

    #[test]
    fn enter_in_tilde_fence_ignores_markers() {
        assert_eq!(enter("~~~\n- a|\n~~~").as_deref(), Some("~~~\n- a\n|\n~~~"));
    }

    #[test]
    fn enter_plain_paragraph_is_default() {
        assert_eq!(enter("hello|"), None);
    }

    #[test]
    fn enter_plain_indented_line_copies_indent() {
        assert_eq!(enter("    code|").as_deref(), Some("    code\n    |"));
    }

    #[test]
    fn enter_plain_tab_indented_line_copies_tab() {
        assert_eq!(enter("\tcode|").as_deref(), Some("\tcode\n\t|"));
    }

    #[test]
    fn enter_crlf_uses_crlf_terminator() {
        assert_eq!(
            enter("- foo|\r\nbar").as_deref(),
            Some("- foo\r\n- |\r\nbar")
        );
    }

    #[test]
    fn enter_on_empty_text_is_default() {
        assert_eq!(enter("|"), None);
    }

    // ---- on_tab / on_tab shift -------------------------------------------

    #[test]
    fn tab_plain_caret_inserts_spaces() {
        assert_eq!(tab("text|").as_deref(), Some("text    |"));
    }

    #[test]
    fn tab_plain_caret_mid_text_inserts_spaces() {
        assert_eq!(tab("ab|cd").as_deref(), Some("ab    |cd"));
    }

    #[test]
    fn tab_list_item_indents_line_and_caret_follows() {
        assert_eq!(tab("- foo|").as_deref(), Some("    - foo|"));
    }

    #[test]
    fn tab_list_item_caret_at_line_start_follows_text() {
        assert_eq!(tab("|- foo").as_deref(), Some("    |- foo"));
    }

    #[test]
    fn tab_multi_line_selection_keeps_selection() {
        assert_eq!(
            tab("- a\n⟦- b\n- c⟧").as_deref(),
            Some("- a\n    ⟦- b\n    - c⟧")
        );
    }

    #[test]
    fn tab_single_line_selection_indents_line() {
        assert_eq!(tab("⟦foo⟧").as_deref(), Some("    ⟦foo⟧"));
    }

    #[test]
    fn tab_selection_ending_at_line_start_skips_that_line() {
        assert_eq!(tab("⟦- a\n⟧- b").as_deref(), Some("    ⟦- a\n⟧- b"));
    }

    #[test]
    fn tab_crlf_selection_keeps_selection() {
        assert_eq!(tab("⟦a\r\nb⟧").as_deref(), Some("    ⟦a\r\n    b⟧"));
    }

    #[test]
    fn tab_on_empty_text_indents() {
        assert_eq!(tab("|").as_deref(), Some("    |"));
    }

    #[test]
    fn shift_tab_outdents_spaces() {
        assert_eq!(shift_tab("    - foo|").as_deref(), Some("- foo|"));
    }

    #[test]
    fn shift_tab_outdents_tab_character() {
        assert_eq!(shift_tab("\tfoo|").as_deref(), Some("foo|"));
    }

    #[test]
    fn shift_tab_removes_partial_indent() {
        assert_eq!(shift_tab("  foo|").as_deref(), Some("foo|"));
    }

    #[test]
    fn shift_tab_caret_inside_indent_moves_with_text() {
        assert_eq!(shift_tab("    |foo").as_deref(), Some("|foo"));
    }

    #[test]
    fn shift_tab_unindented_line_is_none() {
        assert_eq!(shift_tab("foo|"), None);
    }

    #[test]
    fn shift_tab_multi_line_selection_keeps_selection() {
        assert_eq!(shift_tab("⟦    a\n  b⟧").as_deref(), Some("⟦a\nb⟧"));
    }

    #[test]
    fn shift_tab_multi_line_all_unindented_is_none() {
        assert_eq!(shift_tab("⟦a\nb⟧"), None);
    }

    // ---- on_typed_char ---------------------------------------------------

    #[test]
    fn typed_wraps_selection_in_parens() {
        assert_eq!(typed("⟦foo⟧", '(').as_deref(), Some("(⟦foo⟧)"));
    }

    #[test]
    fn typed_pairs_paren_after_word() {
        assert_eq!(typed("x|", '(').as_deref(), Some("x(|)"));
    }

    #[test]
    fn typed_pairs_bracket() {
        assert_eq!(typed("|", '[').as_deref(), Some("[|]"));
    }

    #[test]
    fn typed_pairs_brace() {
        assert_eq!(typed("|", '{').as_deref(), Some("{|}"));
    }

    #[test]
    fn typed_skips_over_closing_paren() {
        assert_eq!(typed("(foo|)", ')').as_deref(), Some("(foo)|"));
    }

    #[test]
    fn typed_skips_over_closing_bracket() {
        assert_eq!(typed("[a|]", ']').as_deref(), Some("[a]|"));
    }

    #[test]
    fn typed_pairs_double_quote_after_space() {
        assert_eq!(typed("say |", '"').as_deref(), Some("say \"|\""));
    }

    #[test]
    fn typed_pairs_double_quote_after_opener() {
        assert_eq!(typed("(|", '"').as_deref(), Some("(\"|\""));
    }

    #[test]
    fn typed_double_quote_before_word_is_default() {
        assert_eq!(typed("|abc", '"'), None);
    }

    #[test]
    fn typed_backtick_after_space_pairs() {
        assert_eq!(typed("a |", '`').as_deref(), Some("a `|`"));
    }

    #[test]
    fn typed_backtick_after_word_is_default() {
        assert_eq!(typed("a|", '`'), None);
    }

    #[test]
    fn typed_apostrophe_is_never_paired() {
        assert_eq!(typed("don|", '\''), None);
    }

    #[test]
    fn typed_quote_inside_contraction_is_default() {
        assert_eq!(typed("don|", '"'), None);
    }

    #[test]
    fn typed_star_after_space_pairs() {
        assert_eq!(typed("x |", '*').as_deref(), Some("x *|*"));
    }

    #[test]
    fn typed_star_after_word_is_default() {
        assert_eq!(typed("a|", '*'), None);
    }

    #[test]
    fn typed_star_between_words_is_default() {
        assert_eq!(typed("x|y", '*'), None);
    }

    #[test]
    fn typed_underscore_at_start_pairs() {
        assert_eq!(typed("|", '_').as_deref(), Some("_|_"));
    }

    #[test]
    fn typed_tilde_after_opener_pairs() {
        assert_eq!(typed("(|", '~').as_deref(), Some("(~|~"));
    }

    #[test]
    fn typed_star_after_opener_pairs() {
        assert_eq!(typed("(|", '*').as_deref(), Some("(*|*"));
    }

    #[test]
    fn typed_fullwidth_paren_pairs() {
        assert_eq!(typed("|", '（').as_deref(), Some("（|）"));
    }

    #[test]
    fn typed_lenticular_pairs() {
        assert_eq!(typed("|", '【').as_deref(), Some("【|】"));
    }

    #[test]
    fn typed_corner_bracket_pairs() {
        assert_eq!(typed("|", '「').as_deref(), Some("「|」"));
    }

    #[test]
    fn typed_book_title_pairs() {
        assert_eq!(typed("|", '《').as_deref(), Some("《|》"));
    }

    #[test]
    fn typed_curly_double_quote_pairs() {
        assert_eq!(typed("|", '“').as_deref(), Some("“|”"));
    }

    #[test]
    fn typed_curly_single_quote_pairs() {
        assert_eq!(typed("|", '‘').as_deref(), Some("‘|’"));
    }

    #[test]
    fn typed_fullwidth_closer_skips_over() {
        assert_eq!(typed("（a|）", '）').as_deref(), Some("（a）|"));
    }

    #[test]
    fn typed_cjk_pair_wraps_cjk_selection() {
        assert_eq!(typed("⟦你好⟧", '【').as_deref(), Some("【⟦你好⟧】"));
    }

    #[test]
    fn typed_bracket_inside_empty_pair_makes_double() {
        assert_eq!(typed("[|]", '[').as_deref(), Some("[[|]]"));
    }

    #[test]
    fn typed_mismatched_closer_is_default() {
        assert_eq!(typed("(a|]", ')'), None);
    }

    #[test]
    fn typed_plain_char_is_default() {
        assert_eq!(typed("a|", 'x'), None);
    }

    #[test]
    fn typed_fence_blocks_star_pairing() {
        assert_eq!(typed("```\n|\n```", '*'), None);
    }

    #[test]
    fn typed_fence_allows_paren_pairing() {
        assert_eq!(typed("```\n|\n```", '(').as_deref(), Some("```\n(|)\n```"));
    }

    #[test]
    fn typed_fence_skips_over_closer() {
        assert_eq!(
            typed("```\n(a|)\n```", ')').as_deref(),
            Some("```\n(a)|\n```")
        );
    }

    #[test]
    fn typed_star_wraps_selection_outside_fence() {
        assert_eq!(typed("⟦word⟧", '*').as_deref(), Some("*⟦word⟧*"));
    }

    #[test]
    fn typed_wrap_blocked_inside_fence_for_star() {
        assert_eq!(typed("```\n⟦x⟧\n```", '*'), None);
    }

    #[test]
    fn typed_on_empty_text_pairs_bracket() {
        assert_eq!(typed("|", '[').as_deref(), Some("[|]"));
    }

    // ---- on_backspace ----------------------------------------------------

    #[test]
    fn backspace_removes_paren_pair() {
        assert_eq!(backspace("f(|)").as_deref(), Some("f|"));
    }

    #[test]
    fn backspace_removes_bracket_pair() {
        assert_eq!(backspace("[|]").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_quote_pair() {
        assert_eq!(backspace("\"|\"").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_backtick_pair() {
        assert_eq!(backspace("`|`").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_star_pair() {
        assert_eq!(backspace("*|*").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_underscore_pair() {
        assert_eq!(backspace("_|_").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_tilde_pair() {
        assert_eq!(backspace("~|~").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_fullwidth_paren_pair() {
        assert_eq!(backspace("（|）").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_lenticular_pair() {
        assert_eq!(backspace("【|】").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_corner_bracket_pair() {
        assert_eq!(backspace("「|」").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_book_title_pair() {
        assert_eq!(backspace("《|》").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_curly_double_pair() {
        assert_eq!(backspace("“|”").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_curly_single_pair() {
        assert_eq!(backspace("‘|’").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_double_bracket_pair() {
        assert_eq!(backspace("[[|]]").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_mismatched_pair_is_default() {
        assert_eq!(backspace("(|]"), None);
    }

    #[test]
    fn backspace_non_empty_pair_is_default() {
        assert_eq!(backspace("(a|)"), None);
    }

    #[test]
    fn backspace_with_selection_is_default() {
        assert_eq!(backspace("⟦ab⟧"), None);
    }

    #[test]
    fn backspace_removes_empty_bullet_marker() {
        assert_eq!(backspace("- |").as_deref(), Some("|"));
    }

    #[test]
    fn backspace_removes_marker_before_item_text() {
        assert_eq!(backspace("- |foo").as_deref(), Some("|foo"));
    }

    #[test]
    fn backspace_outdents_indented_marker() {
        assert_eq!(backspace("    - |foo").as_deref(), Some("- |foo"));
    }

    #[test]
    fn backspace_removes_task_marker() {
        assert_eq!(backspace("- [ ] |todo").as_deref(), Some("|todo"));
    }

    #[test]
    fn backspace_removes_quoted_bullet_marker_keeps_quote() {
        assert_eq!(backspace("> - |x").as_deref(), Some("> |x"));
    }

    #[test]
    fn backspace_mid_item_is_default() {
        assert_eq!(backspace("- ab|c"), None);
    }

    #[test]
    fn backspace_plain_text_is_default() {
        assert_eq!(backspace("abc|"), None);
    }

    // ---- on_paste --------------------------------------------------------

    #[test]
    fn paste_url_over_selection_makes_link() {
        assert_eq!(
            paste("⟦click⟧", "https://a.com").as_deref(),
            Some("[click](https://a.com)|")
        );
    }

    #[test]
    fn paste_http_url_over_selection_makes_link() {
        assert_eq!(
            paste("⟦x⟧", "http://x.io").as_deref(),
            Some("[x](http://x.io)|")
        );
    }

    #[test]
    fn paste_url_without_selection_is_default() {
        assert_eq!(paste("a|", "https://a.com"), None);
    }

    #[test]
    fn paste_url_into_existing_link_text_is_default() {
        assert_eq!(paste("⟦a](b⟧", "https://a.com"), None);
    }

    #[test]
    fn paste_plain_text_over_selection_is_default() {
        assert_eq!(paste("⟦foo⟧", "hello"), None);
    }

    #[test]
    fn paste_ftp_url_over_selection_is_default() {
        assert_eq!(paste("⟦x⟧", "ftp://a.com"), None);
    }

    #[test]
    fn paste_multi_line_url_is_not_linked() {
        assert_eq!(paste("⟦x⟧", "https://a.com\nmore"), None);
    }

    #[test]
    fn paste_url_with_trailing_newline_is_linked() {
        assert_eq!(
            paste("⟦x⟧", "https://a.com\r\n").as_deref(),
            Some("[x](https://a.com)|")
        );
    }

    #[test]
    fn paste_crlf_is_normalised() {
        assert_eq!(paste("a|", "l1\r\nl2").as_deref(), Some("al1\nl2|"));
    }

    #[test]
    fn paste_crlf_over_selection_is_normalised() {
        assert_eq!(paste("x⟦y⟧", "1\r\n").as_deref(), Some("x1\n|"));
    }

    #[test]
    fn paste_plain_text_is_default() {
        assert_eq!(paste("a|", "hello"), None);
    }

    // ---- toggle_task -----------------------------------------------------

    #[test]
    fn toggle_unchecked_to_checked() {
        assert_eq!(toggle("- [ ] task|").as_deref(), Some("- [x] task|"));
    }

    #[test]
    fn toggle_checked_to_unchecked() {
        assert_eq!(toggle("- [x] task|").as_deref(), Some("- [ ] task|"));
    }

    #[test]
    fn toggle_uppercase_checked_to_unchecked() {
        assert_eq!(toggle("- [X] task|").as_deref(), Some("- [ ] task|"));
    }

    #[test]
    fn toggle_star_bullet_task() {
        assert_eq!(toggle("* [ ] a|").as_deref(), Some("* [x] a|"));
    }

    #[test]
    fn toggle_quoted_task() {
        assert_eq!(toggle("> - [ ] a|").as_deref(), Some("> - [x] a|"));
    }

    #[test]
    fn toggle_indented_task() {
        assert_eq!(toggle("  - [ ] a|").as_deref(), Some("  - [x] a|"));
    }

    #[test]
    fn toggle_task_on_second_line() {
        assert_eq!(toggle("a\n|- [ ] b").as_deref(), Some("a\n|- [x] b"));
    }

    #[test]
    fn toggle_plain_bullet_is_none() {
        assert_eq!(toggle("- plain|"), None);
    }

    #[test]
    fn toggle_ordered_item_is_none() {
        assert_eq!(toggle("1. [ ] a|"), None);
    }

    #[test]
    fn toggle_plain_text_is_none() {
        assert_eq!(toggle("text|"), None);
    }

    #[test]
    fn toggle_clamps_offset() {
        let p = toggle_task("- [ ] a", 999).expect("task");
        assert_eq!(p.start, 3);
        assert_eq!(p.insert, "x");
    }

    // ---- list_marker ------------------------------------------------------

    #[test]
    fn list_marker_bullet_with_indent() {
        assert_eq!(
            list_marker("  - x"),
            Some(ListMarker {
                indent: 2,
                kind: ListKind::Bullet('-'),
                marker_len_bytes: 4,
            })
        );
    }

    #[test]
    fn list_marker_ordered_paren() {
        assert_eq!(
            list_marker("12) x"),
            Some(ListMarker {
                indent: 0,
                kind: ListKind::Ordered {
                    number: 12,
                    delim: ')',
                },
                marker_len_bytes: 4,
            })
        );
    }

    #[test]
    fn list_marker_checked_task() {
        assert_eq!(
            list_marker("- [X] x"),
            Some(ListMarker {
                indent: 0,
                kind: ListKind::Task {
                    checked: true,
                    bullet: '-',
                },
                marker_len_bytes: 6,
            })
        );
    }

    #[test]
    fn list_marker_nested_quote_depth() {
        assert_eq!(
            list_marker("> > x"),
            Some(ListMarker {
                indent: 0,
                kind: ListKind::Quote { depth: 2 },
                marker_len_bytes: 4,
            })
        );
    }

    #[test]
    fn list_marker_quoted_bullet_reports_item() {
        assert_eq!(
            list_marker("> * x"),
            Some(ListMarker {
                indent: 0,
                kind: ListKind::Bullet('*'),
                marker_len_bytes: 4,
            })
        );
    }

    #[test]
    fn list_marker_bare_dash() {
        assert_eq!(
            list_marker("-"),
            Some(ListMarker {
                indent: 0,
                kind: ListKind::Bullet('-'),
                marker_len_bytes: 1,
            })
        );
    }

    #[test]
    fn list_marker_plain_text_is_none() {
        assert_eq!(list_marker("plain"), None);
    }

    #[test]
    fn list_marker_horizontal_rule_is_none() {
        assert_eq!(list_marker("---"), None);
    }

    #[test]
    fn list_marker_bold_is_none() {
        assert_eq!(list_marker("**bold**"), None);
    }

    // ---- fences and line bounds -----------------------------------------

    #[test]
    fn fence_opening_line_is_inside() {
        assert!(is_in_fenced_code("```\ncode\n```", 0));
    }

    #[test]
    fn fence_body_is_inside() {
        assert!(is_in_fenced_code("```\ncode\n```", 5));
    }

    #[test]
    fn fence_text_after_close_is_outside() {
        let text = "```\ncode\n```\nafter";
        assert!(!is_in_fenced_code(text, 13));
    }

    #[test]
    fn tilde_fence_body_is_inside() {
        assert!(is_in_fenced_code("~~~\nx\n~~~", 4));
    }

    #[test]
    fn inline_backticks_are_not_a_fence() {
        assert!(!is_in_fenced_code("use ```x``` here", 0));
    }

    #[test]
    fn line_bounds_middle_line() {
        assert_eq!(line_bounds("ab\ncd\nef", 4), (3, 5));
    }

    #[test]
    fn line_bounds_crlf_excludes_terminator() {
        assert_eq!(line_bounds("ab\r\ncd", 0), (0, 2));
    }

    #[test]
    fn line_bounds_last_line_without_newline() {
        assert_eq!(line_bounds("ab\ncd", 5), (3, 5));
    }

    #[test]
    fn line_bounds_offset_on_newline_belongs_to_its_line() {
        assert_eq!(line_bounds("ab\ncd", 2), (0, 2));
    }

    #[test]
    fn line_bounds_clamps_past_end() {
        assert_eq!(line_bounds("ab", 99), (0, 2));
    }

    #[test]
    fn line_bounds_inside_multibyte_char() {
        assert_eq!(line_bounds("你\nx", 1), (0, 3));
    }

    // ---- clamping and empty input ---------------------------------------

    #[test]
    fn enter_clamps_out_of_range_offsets() {
        let p = on_enter("- a", 99, 99).expect("plan");
        assert_eq!(render("- a", &p), "- a\n- |");
    }

    #[test]
    fn tab_clamps_out_of_range_caret() {
        let p = on_tab("a", 10, 10, false).expect("plan");
        assert_eq!(render("a", &p), "a    |");
    }

    #[test]
    fn typed_clamps_mid_char_offset() {
        let p = on_typed_char("你", 1, 1, '(').expect("plan");
        assert_eq!(render("你", &p), "(|)你");
    }

    #[test]
    fn backspace_on_empty_text_is_default() {
        assert_eq!(on_backspace("", 5, 5), None);
    }

    #[test]
    fn enter_on_empty_text_is_default_no_panic() {
        assert_eq!(on_enter("", 0, 0), None);
    }

    #[test]
    fn tab_on_empty_text_inserts_indent_without_panic() {
        let p = on_tab("", 0, 0, false).expect("plan");
        assert_eq!(p.insert, "    ");
    }

    #[test]
    fn paste_with_bad_offsets_does_not_panic() {
        assert_eq!(on_paste("ab", 99, 99, "x"), None);
    }

    #[test]
    fn toggle_on_empty_text_is_none() {
        assert_eq!(toggle_task("", 7), None);
    }

    // ---- toggle_wrap & has_wrap -----------------------------------------

    fn wrap(s: &str, open: &str, close: &str) -> Option<String> {
        let (t, a, b) = parse(s);
        toggle_wrap(&t, a, b, open, close).map(|p| render(&t, &p))
    }

    #[test]
    fn toggle_wrap_bold_and_unwrap_outer_or_inner() {
        assert_eq!(
            wrap("say ⟦hello⟧!", "**", "**").as_deref(),
            Some("say **⟦hello⟧**!")
        );
        assert_eq!(
            wrap("say **⟦hello⟧**!", "**", "**").as_deref(),
            Some("say ⟦hello⟧!")
        );
        assert_eq!(
            wrap("say ⟦**hello**⟧!", "**", "**").as_deref(),
            Some("say ⟦hello⟧!")
        );
    }

    #[test]
    fn toggle_wrap_italic_does_not_confuse_bold_stars() {
        let (t, a, b) = parse("**⟦bold⟧**");
        assert!(has_wrap(&t, a, b, "**", "**"));
        assert!(!has_wrap(&t, a, b, "*", "*"));
        assert_eq!(
            wrap("**⟦bold⟧**", "*", "*").as_deref(),
            Some("***⟦bold⟧***")
        );
        assert_eq!(
            wrap("***⟦bold⟧***", "*", "*").as_deref(),
            Some("**⟦bold⟧**")
        );
        assert_eq!(
            wrap("***⟦bold⟧***", "**", "**").as_deref(),
            Some("*⟦bold⟧*")
        );
    }

    #[test]
    fn toggle_wrap_strike_highlight_code_wikilink_and_unicode() {
        assert_eq!(
            wrap("⟦删除⟧内容", "~~", "~~").as_deref(),
            Some("~~⟦删除⟧~~内容")
        );
        assert_eq!(
            wrap("~~⟦删除⟧~~内容", "~~", "~~").as_deref(),
            Some("⟦删除⟧内容")
        );
        assert_eq!(wrap("⟦重点⟧", "==", "==").as_deref(), Some("==⟦重点⟧=="));
        assert_eq!(wrap("==⟦重点⟧==", "==", "==").as_deref(), Some("⟦重点⟧"));
        assert_eq!(wrap("run ⟦cmd⟧", "`", "`").as_deref(), Some("run `⟦cmd⟧`"));
        assert_eq!(
            wrap("link ⟦笔记名⟧", "[[", "]]").as_deref(),
            Some("link [[⟦笔记名⟧]]")
        );
        assert_eq!(
            wrap("link [[⟦笔记名⟧]]", "[[", "]]").as_deref(),
            Some("link ⟦笔记名⟧")
        );
    }

    #[test]
    fn toggle_wrap_caret_inserts_pair_then_removes_empty_pair() {
        assert_eq!(wrap("a | b", "**", "**").as_deref(), Some("a **|** b"));
        assert_eq!(wrap("a **|** b", "**", "**").as_deref(), Some("a | b"));
    }

    #[test]
    fn toggle_wrap_ignores_multiline_and_fenced_code() {
        assert_eq!(wrap("⟦line1\nline2⟧", "**", "**"), None);
        assert_eq!(wrap("```\n⟦code⟧\n```", "**", "**"), None);
        let (t, a, b) = parse("```\n⟦code⟧\n```");
        assert!(!has_wrap(&t, a, b, "**", "**"));
    }

    // ---- GFM table Tab / Shift+Tab / Enter ------------------------------

    /// Uses `^` as caret marker so Markdown table `|` characters stay intact.
    fn parse_table(s: &str) -> (String, usize, usize) {
        let mut text = String::new();
        let mut start = None;
        let mut end = None;
        for ch in s.chars() {
            match ch {
                '^' => {
                    start = Some(text.len());
                    end = Some(text.len());
                }
                '⟦' => start = Some(text.len()),
                '⟧' => end = Some(text.len()),
                _ => text.push(ch),
            }
        }
        let s0 = start.unwrap_or(0);
        (text, s0, end.unwrap_or(s0))
    }

    fn render_table(text: &str, p: &EditPlan) -> String {
        let mut out = String::new();
        out.push_str(&text[..p.start]);
        out.push_str(&p.insert);
        out.push_str(&text[p.end..]);
        let lo = p.sel_anchor.min(p.sel_head);
        let hi = p.sel_anchor.max(p.sel_head);
        if lo == hi {
            out.insert(lo, '^');
        } else {
            out.insert(hi, '⟧');
            out.insert(lo, '⟦');
        }
        out
    }

    fn t_tab(s: &str) -> Option<String> {
        let (t, a, b) = parse_table(s);
        on_tab(&t, a, b, false).map(|p| render_table(&t, &p))
    }

    fn t_shift_tab(s: &str) -> Option<String> {
        let (t, a, b) = parse_table(s);
        on_tab(&t, a, b, true).map(|p| render_table(&t, &p))
    }

    fn t_enter(s: &str) -> Option<String> {
        let (t, a, b) = parse_table(s);
        on_enter(&t, a, b).map(|p| render_table(&t, &p))
    }

    #[test]
    fn table_tab_moves_through_cells_skips_delimiter_and_appends_row() {
        // 光标在表头第 1 格内 -> 跳到表头第 2 格选中 "数量"
        assert_eq!(
            t_tab("| 名^称 | 数量 |\n| --- | --- |\n| 苹果 | 3 |").as_deref(),
            Some("| 名称 | ⟦数量⟧ |\n| --- | --- |\n| 苹果 | 3 |")
        );
        // 表头最后 1 格按 Tab -> 跳过分隔行直接选中第 1 数据行第 1 格
        assert_eq!(
            t_tab("| 名称 | 数量^ |\n| --- | --- |\n| 苹果 | 3 |").as_deref(),
            Some("| 名称 | 数量 |\n| --- | --- |\n| ⟦苹果⟧ | 3 |")
        );
        // 数据行第 1 格按 Tab -> 选中第 2 格 "3"
        assert_eq!(
            t_tab("| 名称 | 数量 |\n| --- | --- |\n| 苹^果 | 3 |").as_deref(),
            Some("| 名称 | 数量 |\n| --- | --- |\n| 苹果 | ⟦3⟧ |")
        );
        // 最后一行最后一格按 Tab -> 在表尾插入新空行并将光标停在第 1 格中
        assert_eq!(
            t_tab("| 名称 | 数量 |\n| --- | --- |\n| 苹果 | 3^ |").as_deref(),
            Some("| 名称 | 数量 |\n| --- | --- |\n| 苹果 | 3 |\n| ^ |  |")
        );
    }

    #[test]
    fn table_shift_tab_moves_backward_and_skips_delimiter() {
        // 数据行第 1 格 Shift+Tab -> 跳回表头最后一格
        assert_eq!(
            t_shift_tab("| 名称 | 数量 |\n| --- | --- |\n| ⟦苹果⟧ | 3 |").as_deref(),
            Some("| 名称 | ⟦数量⟧ |\n| --- | --- |\n| 苹果 | 3 |")
        );
        // 数据行第 2 格 Shift+Tab -> 选中第 1 格
        assert_eq!(
            t_shift_tab("| 名称 | 数量 |\n| --- | --- |\n| 苹果 | ^3 |").as_deref(),
            Some("| 名称 | 数量 |\n| --- | --- |\n| ⟦苹果⟧ | 3 |")
        );
        // 表头第 1 格 Shift+Tab -> 保持在表头第 1 格不破坏表格
        assert_eq!(
            t_shift_tab("| ^名称 | 数量 |\n| --- | --- |\n| 苹果 | 3 |").as_deref(),
            Some("| ⟦名称⟧ | 数量 |\n| --- | --- |\n| 苹果 | 3 |")
        );
    }

    #[test]
    fn table_enter_inserts_row_and_empty_row_exits_table() {
        assert_eq!(
            t_enter("| 名称 | 数量 |\n| --- | --- |\n| 苹果 | 3^ |").as_deref(),
            Some("| 名称 | 数量 |\n| --- | --- |\n| 苹果 | 3 |\n| ^ |  |")
        );
        // 在空数据行再次按 Enter -> 删除空表行退出表格
        assert_eq!(
            t_enter("| 名称 | 数量 |\n| --- | --- |\n| ^ |  |").as_deref(),
            Some("| 名称 | 数量 |\n| --- | --- |\n^")
        );
        // 含转义管道和行内代码管道的单元格不误分列
        assert_eq!(
            t_tab("| 语法 |\n| --- |\n| `a | b` \\| c^ |").as_deref(),
            Some("| 语法 |\n| --- |\n| `a | b` \\| c |\n| ^ |")
        );
    }

    #[test]
    fn plain_pipe_text_not_a_table_keeps_default_behavior() {
        assert_eq!(t_tab("a | b^").as_deref(), Some("a | b    ^"));
        assert_eq!(t_enter("a | b^"), None);
    }
}
