//! GFM tables of the live view (pure logic, no gpui).
//!
//! A table block is drawn as a grid while the caret is outside it (see `text_input.rs`). This
//! module reads the block's source lines into cells, styles the text of each cell with the same
//! inline rules as the rest of the note (see [`cell_pieces`]), and shares the width of the grid
//! between the columns (see [`allocate_widths`]). Every cell keeps its source range, so a click
//! can put the caret at the matching source position.
//!
//! Splitting follows GFM: a `|` splits cells even inside a code span, unless it is escaped as
//! `\|`. An escaped pipe is shown as `|`. Rows shorter than the header are padded with empty cells,
//! longer rows lose their extra cells. A header and delimiter row with different cell counts are
//! not a table.

use std::ops::Range;

use super::map::{self, Hide, Piece};
use crate::ui::md_style::{self, Segment, SpanStyle, StyleSpan};

/// Column alignment from the delimiter row (`:---`, `:---:`, `---:`; no colon is left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A parsed table. `rows[0]` is the header. Every row has one range per column; a range is the
/// trimmed source of the cell in absolute document bytes (empty when the cell is empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub aligns: Vec<Align>,
    pub rows: Vec<Vec<Range<usize>>>,
}

/// Parses the table whose whole source lines are `block` (header, delimiter, body rows). `None`
/// when the lines are not a table.
pub fn parse(text: &str, block: Range<usize>) -> Option<Table> {
    let src = text.get(block.clone())?;
    let mut lines: Vec<Range<usize>> = Vec::new();
    let mut off = block.start;
    for part in src.split('\n') {
        lines.push(off..off + part.len());
        off += part.len() + 1;
    }
    if lines.len() < 2 {
        return None;
    }
    let header = split_cells(text, lines[0].clone());
    let delim = split_cells(text, lines[1].clone());
    if header.is_empty() || header.len() != delim.len() {
        return None;
    }
    let aligns = delim
        .iter()
        .map(|r| alignment(text.get(r.clone()).unwrap_or("")))
        .collect::<Option<Vec<Align>>>()?;
    let cols = aligns.len();
    let mut rows = vec![fit(header, cols, block.end)];
    for line in &lines[2..] {
        let cells = split_cells(text, line.clone());
        rows.push(fit(cells, cols, line.end));
    }
    Some(Table { aligns, rows })
}

/// Pads or cuts a row to `cols` cells. Padding cells are empty ranges at `at`.
fn fit(mut cells: Vec<Range<usize>>, cols: usize, at: usize) -> Vec<Range<usize>> {
    cells.truncate(cols);
    while cells.len() < cols {
        cells.push(at..at);
    }
    cells
}

/// The alignment of one delimiter cell, or `None` when the cell is not a delimiter (`:?-+:?`).
fn alignment(cell: &str) -> Option<Align> {
    let t = cell.trim();
    let body = t.trim_start_matches(':').trim_end_matches(':');
    if body.is_empty() || !body.bytes().all(|b| b == b'-') {
        return None;
    }
    let left = t.starts_with(':');
    let right = t.ends_with(':') && t.len() > 1;
    Some(match (left, right) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

/// Trims ASCII whitespace from both ends of `r` (of `text`).
fn trim_range(text: &str, r: Range<usize>) -> Range<usize> {
    let s = text.get(r.clone()).unwrap_or("");
    let lead = s.len() - s.trim_start().len();
    let trail = s.len() - s.trim_end().len();
    if lead + trail >= s.len() {
        let at = r.start + s.len();
        return at..at;
    }
    r.start + lead..r.end - trail
}

/// Whether the byte at `at` is escaped by an odd number of backslashes before it.
fn is_escaped(bytes: &[u8], at: usize) -> bool {
    let n = bytes[..at]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count();
    n % 2 == 1
}

/// Splits one table row (the source line `line` of `text`) into its cells: trimmed source ranges.
/// The optional outer pipes are dropped. An unescaped `|` always splits, also inside code spans.
// A blank line yields one empty cell range by design; `vec!` of one range is the intended shape.
#[allow(clippy::single_range_in_vec_init)]
pub fn split_cells(text: &str, line: Range<usize>) -> Vec<Range<usize>> {
    let Some(s) = text.get(line.clone()) else {
        return Vec::new();
    };
    let bytes = s.as_bytes();
    let mut start = s.len() - s.trim_start().len();
    let mut end = s.trim_end().len();
    if start < end && bytes[start] == b'|' {
        start += 1;
    }
    if end > start && bytes[end - 1] == b'|' && !is_escaped(bytes, end - 1) {
        end -= 1;
    }
    if start >= end {
        return if s.trim().is_empty() {
            Vec::new()
        } else {
            vec![line.start..line.start]
        };
    }
    let base = line.start;
    let mut out = Vec::new();
    let mut seg = start;
    let mut j = start;
    while j < end {
        match bytes[j] {
            b'\\' => j += 2,
            b'|' => {
                out.push(trim_range(text, base + seg..base + j));
                seg = j + 1;
                j += 1;
            }
            _ => j += 1,
        }
    }
    out.push(trim_range(text, base + seg..base + end));
    out
}

/// Whether a style span is inline (it may sit in a cell). Block constructs such as headings,
/// list and task markers, rules and quote markers are not drawn in a cell.
fn is_inline(s: &StyleSpan, sub: &str) -> bool {
    if s.style.heading != 0 {
        return false;
    }
    if s.style.has(SpanStyle::LIST_MARKER)
        || s.style.has(SpanStyle::TASK_MARKER)
        || s.style.has(SpanStyle::HR)
        || s.style.has(SpanStyle::QUOTE)
    {
        return false;
    }
    if s.style.has(SpanStyle::MARKER) {
        let first = sub.get(s.range.clone()).and_then(|t| t.bytes().next());
        if matches!(first, Some(b'#' | b'>')) {
            return false;
        }
    }
    true
}

/// The pieces of one cell (see `map::line_pieces`): the cell's text with inline markers hidden
/// away from `sel` and each `\|` shown as `|`. `bold` styles the whole cell (header row). The
/// pieces tile the cell range and carry absolute document offsets.
pub fn cell_pieces(text: &str, cell: Range<usize>, sel: &Range<usize>, bold: bool) -> Vec<Piece> {
    let base = cell.start;
    let Some(sub) = text.get(cell.clone()) else {
        return Vec::new();
    };
    if sub.is_empty() {
        return Vec::new();
    }
    // Spans and hides are computed on the cell text alone, then moved to document offsets.
    let spans: Vec<StyleSpan> = md_style::compute_spans(sub)
        .into_iter()
        .filter(|s| is_inline(s, sub))
        .collect();
    let mut hides: Vec<Hide> = map::compute_hides(sub, &spans)
        .into_iter()
        .map(|mut h| {
            h.src = h.src.start + base..h.src.end + base;
            h.reveal = h.reveal.start + base..h.reveal.end + base;
            h
        })
        .collect();
    let shifted: Vec<StyleSpan> = spans
        .into_iter()
        .map(|s| StyleSpan {
            range: s.range.start + base..s.range.end + base,
            style: s.style,
        })
        .collect();
    let bytes = sub.as_bytes();
    let mut j = 0;
    while j + 1 < bytes.len() {
        if bytes[j] == b'\\' && bytes[j + 1] == b'|' {
            hides.push(Hide {
                src: base + j..base + j + 2,
                text: "|",
                reveal: cell.clone(),
                deco: map::Deco::None,
            });
            j += 2;
        } else {
            j += 1;
        }
    }
    hides.sort_by_key(|h| h.src.start);
    let segs: Vec<Segment> = md_style::line_segments(&shifted, std::slice::from_ref(&cell))
        .into_iter()
        .next()
        .unwrap_or_default();
    let mut pieces = map::line_pieces(cell, &segs, &hides, sel, None, true);
    if bold {
        for p in &mut pieces {
            p.style = p.style.union(SpanStyle::of(SpanStyle::BOLD));
        }
    }
    pieces
}

/// Width in text units of a string: a CJK or full-width character counts 2, anything else 1.
/// Used when the text system cannot measure a cell.
pub fn text_units(s: &str) -> f32 {
    s.chars()
        .map(|c| {
            let u = c as u32;
            let wide = matches!(u,
                0x1100..=0x115F
                | 0x2E80..=0x303E
                | 0x3041..=0x33FF
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xAC00..=0xD7A3
                | 0xF900..=0xFAFF
                | 0xFE30..=0xFE4F
                | 0xFF00..=0xFF60
                | 0xFFE0..=0xFFE6
                | 0x1F300..=0x1FAFF
                | 0x20000..=0x3FFFD);
            if wide { 2.0 } else { 1.0 }
        })
        .sum()
}

/// Shares `avail` between the columns whose natural widths are `natural`. A table narrower than
/// `avail` grows its columns in proportion to fill it. A wider table shrinks: each column keeps at
/// least `min_w` and the rest is shared by the amount each column wants above `min_w`. When even
/// `min_w` does not fit, the columns share `avail` equally (their text then wraps).
pub fn allocate_widths(natural: &[f32], avail: f32, min_w: f32) -> Vec<f32> {
    let n = natural.len();
    if n == 0 {
        return Vec::new();
    }
    let avail = avail.max(1.0);
    let nat: Vec<f32> = natural.iter().map(|w| w.max(1.0)).collect();
    let total: f32 = nat.iter().sum();
    if total <= avail {
        return nat.iter().map(|w| w * avail / total).collect();
    }
    if avail <= n as f32 * min_w {
        return vec![avail / n as f32; n];
    }
    let spare = avail - n as f32 * min_w;
    let wants: Vec<f32> = nat.iter().map(|w| (w - min_w).max(0.0)).collect();
    let want_sum: f32 = wants.iter().sum();
    if want_sum <= 0.0 {
        return vec![avail / n as f32; n];
    }
    wants.iter().map(|w| min_w + w * spare / want_sum).collect()
}

/// Horizontal alignment of a column.
pub fn align_of(table: &Table, col: usize) -> Align {
    table.aligns.get(col).copied().unwrap_or(Align::Left)
}

#[cfg(test)]
mod tests {
    use super::super::blocks;
    use super::*;

    /// The cell texts of a table (source slices), row by row.
    fn cells_of(text: &str) -> Vec<Vec<&str>> {
        let blocks = blocks::table_blocks(text);
        let t = parse(text, blocks[0].clone()).expect("table");
        t.rows
            .iter()
            .map(|r| r.iter().map(|c| &text[c.clone()]).collect())
            .collect()
    }

    #[test]
    fn parses_header_delimiter_and_rows() {
        let text = "| 名字 | 数量 |\n| :--- | ---: |\n| 苹果 | 3 |\n| 梨 | 12 |\n";
        assert_eq!(
            cells_of(text),
            vec![vec!["名字", "数量"], vec!["苹果", "3"], vec!["梨", "12"]]
        );
        let t = parse(text, blocks::table_blocks(text)[0].clone()).unwrap();
        assert_eq!(t.aligns, vec![Align::Left, Align::Right]);
    }

    #[test]
    fn alignment_comes_from_the_delimiter_row() {
        let text = "a | b | c | d\n:-- | :-: | --: | ---\n1 | 2 | 3 | 4\n";
        let t = parse(text, 0..text.len() - 1).unwrap();
        assert_eq!(
            t.aligns,
            vec![Align::Left, Align::Center, Align::Right, Align::Left]
        );
    }

    #[test]
    fn escaped_pipe_stays_in_its_cell() {
        let text = "| a | b |\n| - | - |\n| x\\|y | z |\n";
        let cells = cells_of(text);
        assert_eq!(cells[1], vec!["x\\|y", "z"]);
        // It is shown as a plain pipe.
        let pieces = cell_pieces(
            text,
            parse(text, 0..text.len() - 1).unwrap().rows[1][0].clone(),
            &(0..0),
            false,
        );
        let shown = map::display_text(&pieces, text);
        assert_eq!(shown, "x|y");
    }

    #[test]
    fn pipe_inside_code_splits_unless_escaped() {
        // GFM: an unescaped pipe in a code span still ends the cell.
        let split = "| a | b |\n| - | - |\n| `x|y` |\n";
        assert_eq!(cells_of(split)[1], vec!["`x", "y`"]);
        let escaped = "| a |\n| - |\n| `x\\|y` |\n";
        let cells = cells_of(escaped);
        assert_eq!(cells[1], vec!["`x\\|y`"]);
        let t = parse(escaped, 0..escaped.len() - 1).unwrap();
        let pieces = cell_pieces(escaped, t.rows[1][0].clone(), &(0..0), false);
        assert_eq!(map::display_text(&pieces, escaped), "x|y");
    }

    #[test]
    fn ragged_rows_are_padded_and_cut() {
        let text = "| a | b |\n| - | - |\n| 1 |\n| 1 | 2 | 3 |\n";
        let cells = cells_of(text);
        assert_eq!(cells[1], vec!["1", ""]);
        assert_eq!(cells[2], vec!["1", "2"]);
    }

    #[test]
    fn header_and_delimiter_must_agree() {
        let text = "| a | b |\n| - |\n";
        assert!(parse(text, 0..text.len() - 1).is_none());
    }

    #[test]
    fn outer_pipes_are_optional() {
        let text = "a | b\n--|--\n1 | 2\n";
        assert_eq!(cells_of(text), vec![vec!["a", "b"], vec!["1", "2"]]);
    }

    #[test]
    fn empty_cells_keep_a_range() {
        let text = "| a | b |\n| - | - |\n|  | y |\n";
        let t = parse(text, 0..text.len() - 1).unwrap();
        assert!(t.rows[1][0].is_empty());
        assert_eq!(&text[t.rows[1][1].clone()], "y");
    }

    #[test]
    fn cjk_units_count_double() {
        assert_eq!(text_units("abc"), 3.0);
        assert_eq!(text_units("中文a"), 5.0);
        assert_eq!(text_units("ＡＢ"), 4.0);
    }

    #[test]
    fn narrow_table_grows_to_fill_the_width() {
        let w = allocate_widths(&[100.0, 300.0], 800.0, 40.0);
        assert!((w.iter().sum::<f32>() - 800.0).abs() < 0.01);
        assert!((w[1] / w[0] - 3.0).abs() < 0.01);
    }

    #[test]
    fn wide_table_shrinks_and_keeps_the_minimum() {
        let w = allocate_widths(&[50.0, 1000.0, 400.0], 600.0, 60.0);
        assert!((w.iter().sum::<f32>() - 600.0).abs() < 0.01);
        assert!(w.iter().all(|&x| x >= 60.0 - 0.01));
        // The widest column keeps the most room.
        assert!(w[1] > w[2] && w[2] > w[0]);
        // Below the minimum everything is equal.
        let eq = allocate_widths(&[500.0, 500.0, 500.0], 90.0, 60.0);
        assert!(eq.iter().all(|&x| (x - 30.0).abs() < 0.01));
    }

    #[test]
    fn inline_styles_in_a_cell_hide_their_markers() {
        let text = "| a |\n| - |\n| **粗** and `c` |\n";
        let t = parse(text, 0..text.len() - 1).unwrap();
        let cell = t.rows[1][0].clone();
        let pieces = cell_pieces(text, cell.clone(), &(0..0), false);
        assert_eq!(map::display_text(&pieces, text), "粗 and c");
        // The caret inside the bold reveals its markers.
        let inside = cell_pieces(text, cell.clone(), &(cell.start + 3..cell.start + 3), false);
        assert_eq!(map::display_text(&inside, text), "**粗** and c");
        // Header cells are bold.
        let head = cell_pieces(text, t.rows[0][0].clone(), &(0..0), true);
        assert!(head.iter().all(|p| p.style.has(SpanStyle::BOLD)));
    }

    #[test]
    fn table_inside_a_quote_is_not_a_grid() {
        let text = "> | a |\n> | - |\n";
        assert!(blocks::table_blocks(text).is_empty());
    }
}
