//! Block ranges of the live preview widgets (pure logic, no gpui).
//!
//! A block widget replaces the source lines of one block while the caret is outside it:
//! fenced code blocks (a panel with the language and a copy button) and the YAML frontmatter
//! (hidden). This module finds the blocks and splits a fenced block into its language and body.
//! Ranges are UTF-8 byte offsets of the document; block ranges cover whole lines and exclude
//! the newline that ends the last line.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

use crate::ui::md_style;

/// The parser options of the style scanner, so both agree on what is a code block.
fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
}

/// Byte range of the line that contains `pos` (without its newline).
fn line_bounds(text: &str, pos: usize) -> Range<usize> {
    let b = text.as_bytes();
    let pos = pos.min(b.len());
    let start = b[..pos]
        .iter()
        .rposition(|&c| c == b'\n')
        .map_or(0, |i| i + 1);
    let end = b[pos..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |i| pos + i);
    start..end
}

/// The YAML frontmatter at the start of the document, as whole lines (`0..end`), if any.
pub fn frontmatter(text: &str) -> Option<Range<usize>> {
    md_style::frontmatter_end(text).map(|end| 0..end)
}

/// The caret a freshly opened note starts with. A caret at 0 would touch the frontmatter record and
/// show the hidden block raw, so a note that opens with frontmatter starts on the line after it.
/// Returns `None` when the note has no frontmatter (the caret stays at 0).
pub fn opening_caret(text: &str) -> Option<usize> {
    let fm = frontmatter(text)?;
    let at = fm.end + 1;
    (at <= text.len()).then_some(at)
}

/// The tables and the fenced code blocks of `text`, from one parse (both use the same options).
/// Each list is sorted and without duplicates; see `table_blocks` and `code_blocks`.
pub fn table_and_code_blocks(text: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let fm_end = md_style::frontmatter_end(text);
    let mut tables: Vec<Range<usize>> = Vec::new();
    let mut codes: Vec<Range<usize>> = Vec::new();
    for (event, range) in Parser::new_ext(text, parser_options()).into_offset_iter() {
        if range.start >= range.end || range.end > text.len() {
            continue;
        }
        if fm_end.is_some_and(|end| range.start < end) {
            continue;
        }
        match event {
            Event::Start(Tag::Table(_)) => {
                // Only a table at the very start of its line: an indented one may sit in a list item.
                let first = line_bounds(text, range.start);
                if first.start != range.start {
                    continue;
                }
                let last = line_bounds(text, range.end - 1);
                let block = first.start..last.end;
                if super::table::parse(text, block.clone()).is_some() {
                    tables.push(block);
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                let first = line_bounds(text, range.start);
                if !opens_fence(text.get(first.clone()).unwrap_or("")) {
                    continue;
                }
                let last = line_bounds(text, range.end - 1);
                if last.end <= first.start {
                    continue;
                }
                codes.push(first.start..last.end);
            }
            _ => {}
        }
    }
    tables.sort_by_key(|r| r.start);
    tables.dedup();
    codes.sort_by_key(|r| r.start);
    codes.dedup();
    (tables, codes)
}

/// The whole source lines of every GFM table that starts its line (not one inside a block quote or
/// a list item) and parses as a table (see `table::parse`). Tables inside frontmatter are skipped.
pub fn table_blocks(text: &str) -> Vec<Range<usize>> {
    table_and_code_blocks(text).0
}

/// The source lines of every fenced code block (a fence on its own line, see `opens_fence`).
pub fn code_blocks(text: &str) -> Vec<Range<usize>> {
    table_and_code_blocks(text).1
}

/// Whether a line opens a fence (after indentation): three or more backticks or tildes.
fn opens_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Whether a line closes a fence: only backticks (or only tildes), at least three.
fn is_closing_fence(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && (t.bytes().all(|c| c == b'`') || t.bytes().all(|c| c == b'~'))
}

/// Splits the source of one fenced block (its lines, no trailing newline) into the language
/// (the first word of the info string, empty when there is none) and the range of the body: the
/// lines between the fences, relative to `block`. An unclosed block has all its lines after the
/// opening fence as body. An empty body is an empty range at the end of `block`.
pub fn split_fenced(block: &str) -> (String, Range<usize>) {
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut off = 0;
    for part in block.split('\n') {
        lines.push((off, part));
        off += part.len() + 1;
    }
    let Some(&(_, first)) = lines.first() else {
        return (String::new(), 0..0);
    };
    let info = first.trim_start().trim_start_matches(['`', '~']).trim();
    let lang = info.split_whitespace().next().unwrap_or("").to_string();
    if lines.len() < 2 {
        return (lang, block.len()..block.len());
    }
    let closed = is_closing_fence(lines[lines.len() - 1].1);
    let body = if closed {
        &lines[1..lines.len() - 1]
    } else {
        &lines[1..]
    };
    match (body.first(), body.last()) {
        (Some(&(a, _)), Some(&(o, l))) => (lang, a..(o + l.len()).min(block.len())),
        _ => (lang, block.len()..block.len()),
    }
}

/// Height of a code panel: a header band, the padding above and below the body, and `body_rows`
/// visual rows of `row_h` each (a wrapped code line takes several rows).
pub fn code_panel_height(body_rows: &[usize], row_h: f32, header_h: f32, pad: f32) -> f32 {
    let body: usize = body_rows.iter().sum();
    header_h + pad * 2.0 + body as f32 * row_h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_block_lines_cover_fences_without_the_newline() {
        // "```rust" (0..7) "\n" "let x;" (8..14) "\n" "```" (15..18) "\n" "z"
        let text = "```rust\nlet x;\n```\nz";
        assert_eq!(code_blocks(text), vec![0..18]);
    }

    #[test]
    fn unclosed_fence_runs_to_the_end_of_the_document() {
        let text = "```\ncode\nmore";
        assert_eq!(code_blocks(text), vec![0..text.len()]);
    }

    #[test]
    fn tilde_fence_is_a_block() {
        let text = "~~~\na\n~~~\n";
        assert_eq!(code_blocks(text), vec![0..9]);
    }

    #[test]
    fn indented_code_is_not_a_widget() {
        assert!(code_blocks("    indented\n    code\n").is_empty());
    }

    #[test]
    fn fence_inside_a_block_quote_is_left_alone() {
        let text = "> ```rust\n> a\n> ```\n";
        assert!(code_blocks(text).is_empty());
    }

    #[test]
    fn two_blocks_in_order() {
        let text = "```a\n1\n```\n\ntext\n\n```b\n2\n```";
        let blocks = code_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].end < blocks[1].start);
        assert_eq!(&text[blocks[1].clone()], "```b\n2\n```");
    }

    #[test]
    fn frontmatter_is_its_own_block_and_fences_inside_it_are_not_code() {
        let text = "---\ntitle: x\n---\n```rust\na\n```\n";
        assert_eq!(frontmatter(text), Some(0..16));
        let blocks = code_blocks(text);
        assert_eq!(blocks, vec![17..30]);
        assert_eq!(frontmatter("# no frontmatter\n"), None);
    }

    #[test]
    fn opening_caret_skips_the_frontmatter() {
        // Frontmatter 0..16, the caret goes to the line after it (17).
        let text = "---\ntitle: x\n---\n# Head\n";
        assert_eq!(opening_caret(text), Some(17));
        assert_eq!(opening_caret("# no frontmatter\n"), None);
        // A frontmatter with no body after it: the caret stays at its end.
        assert_eq!(opening_caret("---\na: 1\n---"), None);
    }

    #[test]
    fn split_reads_language_and_body() {
        let block = "```rust\nlet a = 1;\nlet b = 2;\n```";
        let (lang, body) = split_fenced(block);
        assert_eq!(lang, "rust");
        assert_eq!(&block[body], "let a = 1;\nlet b = 2;");
    }

    #[test]
    fn split_takes_the_first_word_of_the_info_string() {
        let block = "```python title=\"x.py\"\nprint(1)\n```";
        let (lang, body) = split_fenced(block);
        assert_eq!(lang, "python");
        assert_eq!(&block[body], "print(1)");
    }

    #[test]
    fn split_handles_empty_and_unclosed_bodies() {
        let empty = "```\n```";
        let (lang, body) = split_fenced(empty);
        assert_eq!(lang, "");
        assert!(body.is_empty());
        let open = "```text";
        let (lang, body) = split_fenced(open);
        assert_eq!(lang, "text");
        assert!(body.is_empty());
        let unclosed = "~~~\na\nb";
        let (_, body) = split_fenced(unclosed);
        assert_eq!(&unclosed[body], "a\nb");
    }

    #[test]
    fn split_keeps_blank_lines_inside_the_body() {
        let block = "```\na\n\nb\n```";
        let (_, body) = split_fenced(block);
        assert_eq!(&block[body], "a\n\nb");
    }

    #[test]
    fn cjk_code_keeps_byte_ranges_valid() {
        let block = "```\n中文 = 1\n```";
        let (_, body) = split_fenced(block);
        assert_eq!(&block[body], "中文 = 1");
    }

    #[test]
    fn panel_height_adds_header_and_padding_per_row() {
        // Three lines, the second wraps to two rows.
        assert_eq!(
            code_panel_height(&[1, 2, 1], 20.0, 30.0, 8.0),
            30.0 + 16.0 + 80.0
        );
        assert_eq!(code_panel_height(&[], 20.0, 30.0, 8.0), 46.0);
    }
}
