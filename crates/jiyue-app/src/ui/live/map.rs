//! Live preview display mapping (pure logic, no gpui).
//!
//! The editor shows one note with its Markdown markers hidden away from the caret. This module
//! decides what is hidden and maps positions both ways:
//!
//! * [`compute_hides`] turns the md_style spans of a document into [`Hide`] records: inline
//!   markers (`**`, `*`, `~~`, `==`, backticks, `[`, `](url)`, `[[`, `]]`, the `target|` part of
//!   an alias) and block markers (`#` prefixes, `>` quote markers, bullets, task boxes, rules and
//!   code fences). Each record says what to show instead (nothing, a bullet, a checkbox box, a
//!   quote indent) and which source range makes it raw again (`reveal`).
//! * [`is_revealed`] decides, for a caret or selection, whether a record is shown raw. A record
//!   is revealed when the selection touches its reveal range, boundaries included, so the element
//!   the caret is in or next to always shows its markers and can be edited.
//! * [`line_pieces`] cuts one logical line into [`Piece`]s: source ranges with one style each,
//!   either shown as source or replaced by a display string. The pieces tile the line.
//! * [`to_display`] and [`to_source`] map a source offset to a display offset and back. A display
//!   offset inside a replacement maps to its start or its end; hidden pieces take no room.
//!
//! Offsets are UTF-8 byte offsets of the document. Display offsets are byte offsets of the text
//! that is shaped for the line, relative to the line start.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use super::blocks;
use super::callout;
use super::diagram;
use super::embed;
use super::footnote;
use crate::ui::md_style::{Segment, SpanStyle, StyleSpan};

/// Shown for a `- ` / `* ` / `+ ` bullet. The space after the marker stays as it is.
pub const BULLET_TEXT: &str = "\u{2022}\u{2002}";
/// Blank space that a task box is painted over (see [`Deco::Checkbox`]).
pub const CHECKBOX_SPACE: &str = "     ";
/// Blank space that a block quote marker leaves at the start of the line (see [`Deco::QuoteBar`]).
pub const QUOTE_SPACE: &str = "    ";

/// Something the editor paints over a replaced piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deco {
    None,
    /// A task box at the piece, filled when `checked`.
    Checkbox {
        checked: bool,
    },
    /// The vertical bar of a block quote level, at the piece.
    QuoteBar,
    /// A horizontal rule across the line (a thematic break).
    Rule,
    /// A block widget over whole source lines (see [`BlockKind`]). Only its record carries it.
    Block(BlockKind),
    /// An inline formula `$...$` (see `text_input`): drawn as a rendered formula when the caret is
    /// away from it, else as source. Only the first piece of its record carries it.
    InlineMath,
    /// A footnote reference `[^label]` (see `footnote.rs`): drawn as a small superscript of its
    /// ordinal, in the warning colour when no definition has the label. Only its record carries it.
    FootnoteRef {
        ordinal: u32,
        missing: bool,
    },
    /// The label prefix `[^label]:` of a footnote definition (see `footnote.rs`): drawn as the
    /// ordinal with a back arrow when a reference uses the label (`ordinal` is `Some`), else as the
    /// dimmed label. `cells` is the number of blank cells the prefix reserves.
    FootnoteDef {
        ordinal: Option<u32>,
        cells: u32,
    },
}

/// Blank cells that a footnote piece reserves (see [`GAP_CELL`]): the digits of a reference, the
/// cells of a definition prefix. Zero for every other record.
pub fn footnote_gap(deco: Deco) -> usize {
    match deco {
        Deco::FootnoteRef { ordinal, .. } => footnote::digits(ordinal) as usize,
        Deco::FootnoteDef { cells, .. } => cells as usize,
        _ => 0,
    }
}

/// The block widgets of the live view. The record's `src` is the block's whole source lines and
/// its `reveal` is the same range: the block is drawn as a widget unless the caret touches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// A fenced code block: a panel with the language and a copy button.
    Code,
    /// The YAML frontmatter at the top: hidden (it is shown in the properties panel).
    Frontmatter,
    /// A GFM table: a grid of cells (see `table.rs`).
    Table,
    /// A callout `> [!type]`: a tinted panel with a title and the body (see `callout.rs`).
    Callout,
    /// An image alone on its line (`![alt](pic.png)`, `![[pic.png|300]]`, remote placeholders): the
    /// picture at its display size, or a placeholder card (see `embed.rs`).
    Image,
    /// A note embed alone on its line (`![[note#Heading]]`): an excerpt panel (see `embed.rs`).
    Embed,
    /// A block formula `$$...$$` that fills its source lines: the rendered formula (see `embed.rs`).
    Math,
    /// A fenced block in the `mermaid` language: the rendered diagram (see `diagram.rs`).
    Mermaid,
}

/// The blank cell a formula piece reserves per [`Piece::gap`] unit (an en space, half an em).
pub const GAP_CELL: &str = "\u{2002}";

/// One hidden or replaced source range. `reveal` is the source range whose touching caret or
/// selection shows the range raw again: an inline element, or the whole line for block markers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hide {
    pub src: Range<usize>,
    /// What is shown instead. Empty: nothing (zero width).
    pub text: &'static str,
    pub reveal: Range<usize>,
    pub deco: Deco,
}

/// A piece of one logical line: a source range with one style. `replace` is `None` when the
/// source text is shown, otherwise the given text is shown instead (empty: hidden).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    /// Absolute source byte range.
    pub src: Range<usize>,
    pub style: SpanStyle,
    pub replace: Option<&'static str>,
    pub deco: Deco,
    /// Part of the IME marked (composing) text.
    pub ime: bool,
    /// Blank cells shown after `replace` (see [`GAP_CELL`]). Set for an inline formula that is
    /// drawn as an image: the cells reserve the width of the image. Zero otherwise.
    pub gap: usize,
}

/// Inline flags whose content has markers at both ends.
const INLINE_FLAGS: u32 = SpanStyle::BOLD
    | SpanStyle::ITALIC
    | SpanStyle::STRIKE
    | SpanStyle::HIGHLIGHT
    | SpanStyle::CODE
    | SpanStyle::LINK_TEXT
    | SpanStyle::LINK_URL
    | SpanStyle::WIKILINK;

/// Whether a record with `reveal` is shown raw for the selection `sel` (a caret is an empty
/// range). Touching counts: a caret just before or after an element belongs to it.
pub fn is_revealed(reveal: &Range<usize>, sel: &Range<usize>) -> bool {
    sel.start <= reveal.end && sel.end >= reveal.start
}

/// Whether a record is shown as source. Only the live view reveals (read mode never does, whatever
/// the caret or the find match), and a record is revealed by a touching selection or a find hit.
pub fn shows_source(
    live: bool,
    read_only: bool,
    reveal: &Range<usize>,
    sel: &Range<usize>,
    found_here: bool,
) -> bool {
    live && !read_only && (is_revealed(reveal, sel) || found_here)
}

/// Whether a record is shown raw because the current find match lies inside it. Every block
/// widget (code, table, callout, image, embed, formula, diagram) and every inline formula reveals
/// its source then, so a hit inside a picture of the text is always visible in the source.
pub fn found_reveals(deco: Deco, src: &Range<usize>, found: Option<&Range<usize>>) -> bool {
    matches!(deco, Deco::Block(_) | Deco::InlineMath)
        && found.is_some_and(|m| m.start < src.end && m.end > src.start)
}

/// Byte range of the line that contains `pos` (without its newline).
fn line_of(text: &str, pos: usize) -> Range<usize> {
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

/// One inline element: its content with the opening and closing marker ranges.
struct Elem {
    flag: u32,
    content: Range<usize>,
    open: Range<usize>,
    close: Range<usize>,
    reveal: Range<usize>,
}

/// The single inline flag of a span style (the lowest one when several are set).
fn inline_flag(style: SpanStyle) -> Option<u32> {
    let bits = style.bits() & INLINE_FLAGS;
    if bits == 0 {
        None
    } else {
        Some(bits.isolate_lowest_one())
    }
}

/// The hidden and replaced source ranges of a document, sorted by start and non-overlapping.
/// `spans` are the md_style spans of `text` (as `md_style::compute_spans` returns them).
pub fn compute_hides(text: &str, spans: &[StyleSpan]) -> Vec<Hide> {
    let bytes = text.as_bytes();
    // Marker spans by the position where they end (openers) and where they begin (closers).
    let mut open_at: HashMap<usize, Range<usize>> = HashMap::new();
    let mut close_at: HashMap<usize, Range<usize>> = HashMap::new();
    for s in spans {
        if s.style.has(SpanStyle::MARKER) {
            open_at
                .entry(s.range.end)
                .or_insert_with(|| s.range.clone());
            close_at
                .entry(s.range.start)
                .or_insert_with(|| s.range.clone());
        }
    }

    // Pass 1: inline elements whose markers are present and look right.
    let mut elems: Vec<Elem> = Vec::new();
    // End offsets of the `]` of image texts. A `(url)` after one of these is an image source.
    let mut image_ends: HashSet<usize> = HashSet::new();
    for s in spans {
        if s.style.heading != 0 {
            continue;
        }
        let Some(flag) = inline_flag(s.style) else {
            continue;
        };
        let content = s.range.clone();
        let (Some(open), Some(close)) = (
            open_at.get(&content.start).cloned(),
            close_at.get(&content.end).cloned(),
        ) else {
            continue;
        };
        let ot = text.get(open.clone()).unwrap_or("");
        let ct = text.get(close.clone()).unwrap_or("");
        let bang = open.start > 0 && bytes.get(open.start - 1) == Some(&b'!');
        let ok = match flag {
            SpanStyle::BOLD | SpanStyle::ITALIC | SpanStyle::STRIKE | SpanStyle::HIGHLIGHT => {
                !ot.is_empty()
                    && ot == ct
                    && ot.bytes().all(|b| matches!(b, b'*' | b'_' | b'~' | b'='))
            }
            SpanStyle::CODE => !ot.is_empty() && ot == ct && ot.bytes().all(|b| b == b'`'),
            SpanStyle::WIKILINK => ot == "[[" && ct == "]]" && !bang,
            SpanStyle::LINK_TEXT => {
                if ot == "[" && ct == "]" && bang {
                    image_ends.insert(close.end);
                }
                ot == "[" && ct == "]" && !bang
            }
            SpanStyle::LINK_URL => {
                ((ot == "(" && ct == ")") || (ot == "[" && ct == "]"))
                    && !image_ends.contains(&open.start)
            }
            _ => false,
        };
        if !ok {
            continue;
        }
        let reveal = open.start..close.end;
        elems.push(Elem {
            flag,
            content,
            open,
            close,
            reveal,
        });
    }

    // Pass 2: a link text and the url right after its `]` reveal together.
    let mut text_by_close_end: HashMap<usize, usize> = HashMap::new();
    for (i, e) in elems.iter().enumerate() {
        if e.flag == SpanStyle::LINK_TEXT {
            text_by_close_end.entry(e.close.end).or_insert(i);
        }
    }
    let merges: Vec<(usize, usize)> = elems
        .iter()
        .enumerate()
        .filter(|(_, e)| e.flag == SpanStyle::LINK_URL)
        .filter_map(|(ui, e)| text_by_close_end.get(&e.open.start).map(|&ti| (ti, ui)))
        .collect();
    for (ti, ui) in merges {
        let r = elems[ti].open.start..elems[ui].close.end;
        elems[ti].reveal = r.clone();
        elems[ui].reveal = r;
    }

    let mut out: Vec<Hide> = Vec::new();
    for e in &elems {
        let hide = |src: Range<usize>| Hide {
            src,
            text: "",
            reveal: e.reveal.clone(),
            deco: Deco::None,
        };
        out.push(hide(e.open.clone()));
        out.push(hide(e.close.clone()));
        if e.flag == SpanStyle::LINK_URL {
            out.push(hide(e.content.clone()));
        }
        if e.flag == SpanStyle::WIKILINK {
            // `[[target|alias]]` shows the alias only.
            if let Some(p) = text.get(e.content.clone()).and_then(|t| t.find('|')) {
                out.push(hide(e.content.start..e.content.start + p + 1));
            }
        }
    }

    // Pass 3: block markers. Their reveal is the whole line.
    for s in spans {
        let r = s.range.clone();
        let Some(first) = text.get(r.clone()).and_then(|t| t.bytes().next()) else {
            continue;
        };
        if s.style.has(SpanStyle::MARKER) {
            match first {
                b'#' => out.push(Hide {
                    src: r.clone(),
                    text: "",
                    reveal: line_of(text, r.start),
                    deco: Deco::None,
                }),
                b'>' => out.push(Hide {
                    src: r.clone(),
                    text: QUOTE_SPACE,
                    reveal: line_of(text, r.start),
                    deco: Deco::QuoteBar,
                }),
                _ => {}
            }
        } else if s.style.has(SpanStyle::LIST_MARKER) && matches!(first, b'-' | b'*' | b'+') {
            let line = line_of(text, r.start);
            let rest = text.get(r.end..line.end).unwrap_or("");
            let task = rest.trim_start_matches([' ', '\t']).starts_with('[');
            out.push(Hide {
                src: r.clone(),
                text: if task { "" } else { BULLET_TEXT },
                reveal: line,
                deco: Deco::None,
            });
        } else if s.style.has(SpanStyle::TASK_MARKER) {
            let checked = matches!(text.as_bytes().get(r.start + 1), Some(&b'x' | &b'X'));
            out.push(Hide {
                src: r.clone(),
                text: CHECKBOX_SPACE,
                reveal: line_of(text, r.start),
                deco: Deco::Checkbox { checked },
            });
        } else if s.style.has(SpanStyle::HR) {
            // The rule span may take the newline too: the whole line is the rule.
            let line = line_of(text, r.start);
            out.push(Hide {
                src: line.clone(),
                text: "",
                reveal: line,
                deco: Deco::Rule,
            });
        }
    }

    // Footnotes: references and definition prefixes. They join `out` before the block records are
    // merged in, so a footnote inside a table, callout or code block stays source text.
    let notes = footnote::scan(text);
    for r in &notes.refs {
        out.push(Hide {
            src: r.range.clone(),
            text: "",
            reveal: r.range.clone(),
            deco: Deco::FootnoteRef {
                ordinal: r.ordinal,
                missing: !r.defined,
            },
        });
    }
    for d in &notes.defs {
        out.push(Hide {
            src: d.range.clone(),
            text: "",
            reveal: line_of(text, d.range.start),
            deco: Deco::FootnoteDef {
                ordinal: d.ordinal,
                cells: d.cells,
            },
        });
    }

    // Pass 4: block widgets (fenced code, frontmatter). A block replaces every record inside it:
    // when the block is drawn raw, its markers and inline styling show as source.
    let mut blocks: Vec<Hide> = Vec::new();
    if let Some(fm) = blocks::frontmatter(text) {
        blocks.push(Hide {
            src: fm.clone(),
            text: "",
            reveal: fm,
            deco: Deco::Block(BlockKind::Frontmatter),
        });
    }
    let (tables, codes) = blocks::table_and_code_blocks(text);
    for r in codes {
        let kind = if diagram::is_mermaid_block(text.get(r.clone()).unwrap_or("")) {
            BlockKind::Mermaid
        } else {
            BlockKind::Code
        };
        blocks.push(Hide {
            src: r.clone(),
            text: "",
            reveal: r,
            deco: Deco::Block(kind),
        });
    }
    for r in tables {
        blocks.push(Hide {
            src: r.clone(),
            text: "",
            reveal: r,
            deco: Deco::Block(BlockKind::Table),
        });
    }
    for r in callout::callout_blocks(text) {
        blocks.push(Hide {
            src: r.clone(),
            text: "",
            reveal: r,
            deco: Deco::Block(BlockKind::Callout),
        });
    }
    // Pass 5: an image or a note embed alone on its line. Lines inside another block (code,
    // tables, callouts, frontmatter) are left to that block.
    let mut line_start = 0usize;
    for line in text.split('\n') {
        let start = line_start;
        let end = start + line.len();
        line_start = end + 1;
        if !line.contains("![") {
            continue;
        }
        let Some(e) = embed::line_embed(line) else {
            continue;
        };
        if blocks
            .iter()
            .any(|b| b.src.start < end && b.src.end > start)
        {
            continue;
        }
        let kind = match e.class {
            embed::EmbedClass::Note => BlockKind::Embed,
            embed::EmbedClass::Image | embed::EmbedClass::Remote => BlockKind::Image,
        };
        blocks.push(Hide {
            src: start..end,
            text: "",
            reveal: start..end,
            deco: Deco::Block(kind),
        });
    }

    // Pass 6: block formulas. A display span that starts its line and ends its line is a block;
    // an inline span (`$2^n$`) stays raw text.
    for span in jiyue_math::find_math_spans(text) {
        if !span.display {
            // An inline formula is a record of its own: the editor draws it as an image while the
            // caret is away. It stays out of blocks (tables, code) and wins over inline markers.
            if blocks
                .iter()
                .any(|b| b.src.start < span.range.end && b.src.end > span.range.start)
            {
                continue;
            }
            blocks.push(Hide {
                src: span.range.clone(),
                text: "",
                reveal: span.range,
                deco: Deco::InlineMath,
            });
            continue;
        }
        let first = line_of(text, span.range.start);
        let last = line_of(text, span.range.end.saturating_sub(1));
        let whole_lines = text
            .get(first.start..span.range.start)
            .is_some_and(|s| s.trim().is_empty())
            && text
                .get(span.range.end..last.end)
                .is_some_and(|s| s.trim().is_empty());
        if !whole_lines {
            continue;
        }
        let src = first.start..last.end;
        if blocks
            .iter()
            .any(|b| b.src.start < src.end && b.src.end > src.start)
        {
            continue;
        }
        blocks.push(Hide {
            src: src.clone(),
            text: "",
            reveal: src,
            deco: Deco::Block(BlockKind::Math),
        });
    }

    blocks.sort_by_key(|h| h.src.start);
    out.retain(|h| {
        let i = blocks.partition_point(|b| b.src.end <= h.src.start);
        !(i < blocks.len() && blocks[i].src.start < h.src.end)
    });
    out.extend(blocks);

    out.retain(|h| h.src.start < h.src.end);
    out.sort_by_key(|h| (h.src.start, h.src.end));
    // Overlaps (not produced by md_style for one construct) keep the first record.
    let mut kept: Vec<Hide> = Vec::with_capacity(out.len());
    for h in out {
        if kept.last().is_none_or(|k| h.src.start >= k.src.end) {
            kept.push(h);
        }
    }
    kept
}

/// The pieces of one logical line. `line` is its absolute byte range (without the newline),
/// `segs` its style segments, `hides` the records of the document that lie in the line (see
/// [`compute_hides`]), `sel` the selection (a caret is an empty range) and `marked` the IME
/// composing range. Records are applied only when `live` is set. An empty line has no pieces.
pub fn line_pieces(
    line: Range<usize>,
    segs: &[Segment],
    hides: &[Hide],
    sel: &Range<usize>,
    marked: Option<&Range<usize>>,
    live: bool,
) -> Vec<Piece> {
    if line.start >= line.end {
        return Vec::new();
    }
    let hides: &[Hide] = if live { hides } else { &[] };
    let mut cuts = vec![line.start, line.end];
    for s in segs {
        cuts.push(s.range.start.clamp(line.start, line.end));
        cuts.push(s.range.end.clamp(line.start, line.end));
    }
    for h in hides {
        cuts.push(h.src.start.clamp(line.start, line.end));
        cuts.push(h.src.end.clamp(line.start, line.end));
    }
    if let Some(m) = marked {
        for b in [m.start, m.end] {
            if b > line.start && b < line.end {
                cuts.push(b);
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut out = Vec::with_capacity(cuts.len().saturating_sub(1));
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        let style = segs
            .iter()
            .find(|s| s.range.start <= a && a < s.range.end)
            .map_or(SpanStyle::default(), |s| s.style);
        let hide = hides.iter().find(|h| h.src.start <= a && a < h.src.end);
        let (replace, deco) = match hide {
            // An inline formula keeps its source here; the editor replaces it with the rendered
            // image once the formula is ready (see `text_input`). A formula that is not drawn
            // (loading, refused, or in excerpts) therefore shows its source.
            Some(h) if h.deco == Deco::InlineMath && !is_revealed(&h.reveal, sel) => {
                if a == h.src.start {
                    (None, Deco::InlineMath)
                } else {
                    (None, Deco::None)
                }
            }
            Some(h) if !is_revealed(&h.reveal, sel) => {
                // The record's text and deco go on its first piece; later pieces stay hidden.
                if a == h.src.start {
                    (Some(h.text), h.deco)
                } else {
                    (Some(""), Deco::None)
                }
            }
            _ => (None, Deco::None),
        };
        let ime = marked.is_some_and(|m| a >= m.start && a < m.end);
        // A footnote reserves its cells on the first piece of its record (only when replaced).
        let gap = match (hide, replace) {
            (Some(h), Some(_)) if a == h.src.start => footnote_gap(h.deco),
            _ => 0,
        };
        out.push(Piece {
            src: a..b,
            style,
            replace,
            deco,
            ime,
            gap,
        });
    }
    out
}

/// Length in bytes of the text a piece shows.
pub fn display_len(p: &Piece) -> usize {
    match p.replace {
        Some(r) => r.len() + p.gap * GAP_CELL.len(),
        None => p.src.len(),
    }
}

/// The text a line shows, from its pieces and the document text.
pub fn display_text(pieces: &[Piece], text: &str) -> String {
    let mut out = String::new();
    for p in pieces {
        match p.replace {
            Some(r) => {
                out.push_str(r);
                for _ in 0..p.gap {
                    out.push_str(GAP_CELL);
                }
            }
            None => out.push_str(text.get(p.src.clone()).unwrap_or("")),
        }
    }
    out
}

/// Display offset (relative to the line's display start) of the source offset `src`. An offset
/// inside a hidden range maps to where the range was (the visible text after it). An offset
/// inside a replacement maps to its start, or to its end when it is past the start.
pub fn to_display(pieces: &[Piece], src: usize) -> usize {
    let mut o = 0;
    for p in pieces {
        if src < p.src.start {
            return o;
        }
        if src <= p.src.end {
            return match p.replace {
                None => o + (src - p.src.start),
                Some(_) => {
                    if src == p.src.start {
                        o
                    } else {
                        o + display_len(p)
                    }
                }
            };
        }
        o += display_len(p);
    }
    o
}

/// Source offset of the display offset `disp` (relative to the line's display start). Zero-width
/// pieces are skipped, so the result is the first visible source offset at or after `disp`.
/// Past the last visible piece the result is the end of the last piece, or `line_end` when the
/// line has no pieces.
pub fn to_source(pieces: &[Piece], disp: usize, line_end: usize) -> usize {
    let mut o = 0;
    for p in pieces {
        let len = display_len(p);
        if len == 0 {
            continue;
        }
        if disp < o + len {
            return match p.replace {
                None => p.src.start + (disp - o),
                Some(_) => {
                    if disp == o {
                        p.src.start
                    } else {
                        p.src.end
                    }
                }
            };
        }
        o += len;
    }
    pieces.last().map_or(line_end, |p| p.src.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::md_style::{compute_spans, line_segments, split_lines};

    /// A selection far outside the test documents: no element and no line is touched.
    const OFF: Range<usize> = 10_000..10_000;

    /// Pieces of logical line `line_no` of `text`, with the hides of the whole document.
    fn pieces_at(text: &str, line_no: usize, sel: Range<usize>) -> Vec<Piece> {
        let spans = compute_spans(text);
        let lines = split_lines(text);
        let segs = line_segments(&spans, &lines);
        let hides = compute_hides(text, &spans);
        let line = lines[line_no].clone();
        let in_line: Vec<Hide> = hides
            .iter()
            .filter(|h| h.src.start >= line.start && h.src.end <= line.end)
            .cloned()
            .collect();
        line_pieces(line, &segs[line_no], &in_line, &sel, None, true)
    }

    /// What line `line_no` shows with the given selection.
    fn shown(text: &str, line_no: usize, sel: Range<usize>) -> String {
        display_text(&pieces_at(text, line_no, sel), text)
    }

    #[test]
    fn bold_markers_are_hidden_off_the_caret() {
        // "a **bold** b": element 2..10, markers 2..4 and 8..10.
        let text = "a **bold** b\nc";
        let spans = compute_spans(text);
        let hides = compute_hides(text, &spans);
        let src: Vec<_> = hides.iter().map(|h| h.src.clone()).collect();
        assert_eq!(src, vec![2..4, 8..10]);
        assert!(hides.iter().all(|h| h.reveal == (2..10)));
        assert_eq!(shown(text, 0, OFF), "a bold b");
        assert_eq!(shown(text, 0, 5..5), "a **bold** b");
        // Touching the element boundaries reveals it too.
        assert_eq!(shown(text, 0, 2..2), "a **bold** b");
        assert_eq!(shown(text, 0, 10..10), "a **bold** b");
        assert_eq!(shown(text, 0, 11..11), "a bold b");
    }

    #[test]
    fn heading_marker_hides_and_reveals_the_line() {
        let text = "## Title\n## Next";
        let hides = compute_hides(text, &compute_spans(text));
        assert_eq!(hides.len(), 2);
        assert_eq!(hides[0].src, 0..3);
        assert_eq!(hides[0].reveal, 0..8);
        // Caret on the heading line: raw markers.
        assert_eq!(shown(text, 0, 5..5), "## Title");
        // Caret on the other line: the first heading shows its text only.
        assert_eq!(shown(text, 0, 12..12), "Title");
        assert_eq!(shown(text, 1, 12..12), "## Next");
        assert_eq!(shown(text, 1, 0..0), "Next");
    }

    #[test]
    fn italic_strike_highlight_and_code_markers() {
        let text = "*i* ~~s~~ ==h== `c`";
        let hides = compute_hides(text, &compute_spans(text));
        assert_eq!(hides.len(), 8);
        assert_eq!(shown(text, 0, OFF), "i s h c");
        // The caret inside the code span reveals only its backticks.
        assert_eq!(shown(text, 0, 17..17), "i s h `c`");
    }

    #[test]
    fn markdown_link_shows_only_the_text() {
        // "see [docs](https://example.com) now": the element runs from 4 to 31.
        let text = "see [docs](https://example.com) now\nx";
        assert_eq!(shown(text, 0, OFF), "see docs now");
        // Caret inside the link text reveals the whole link, the url included.
        assert_eq!(shown(text, 0, 6..6), "see [docs](https://example.com) now");
        let hides = compute_hides(text, &compute_spans(text));
        assert!(hides.iter().all(|h| h.reveal == (4..31)));
    }

    #[test]
    fn image_alone_on_its_line_is_a_block_widget() {
        let text = "![alt](pic.png)\nx";
        let hides = compute_hides(text, &compute_spans(text));
        assert_eq!(hides.len(), 1);
        assert_eq!(hides[0].src, 0..15);
        assert_eq!(hides[0].deco, Deco::Block(BlockKind::Image));
        assert_eq!(shown(text, 0, OFF), "");
        // The caret on the line shows the source.
        assert_eq!(shown(text, 0, 3..3), "![alt](pic.png)");
    }

    #[test]
    fn inline_image_in_a_paragraph_stays_raw() {
        let text = "see ![alt](pic.png) here\nx";
        let hides = compute_hides(text, &compute_spans(text));
        assert!(hides.is_empty());
        assert_eq!(shown(text, 0, OFF), "see ![alt](pic.png) here");
    }

    #[test]
    fn block_formula_fills_its_lines_and_inline_formula_stays_raw() {
        let text = "$$\n\\frac{a}{b}\n$$\n\nsee $2^n$ here\n";
        let hides = compute_hides(text, &compute_spans(text));
        let math: Vec<_> = hides
            .iter()
            .filter(|h| h.deco == Deco::Block(BlockKind::Math))
            .map(|h| h.src.clone())
            .collect();
        assert_eq!(math, vec![0..17]);
        // The inline formula is not a record: its text stays.
        assert_eq!(shown(text, 4, OFF), "see $2^n$ here");
        // A formula with text beside it on the same line is not a block.
        let mixed = "x $$a$$\n";
        assert!(
            compute_hides(mixed, &compute_spans(mixed))
                .iter()
                .all(|h| h.deco != Deco::Block(BlockKind::Math))
        );
    }

    #[test]
    fn note_embed_alone_is_an_embed_block_and_code_wins() {
        let text = "![[Plan#Goals]]\n```\n![[pic.png]]\n```\n";
        let hides = compute_hides(text, &compute_spans(text));
        let kinds: Vec<_> = hides
            .iter()
            .filter_map(|h| match h.deco {
                Deco::Block(k) => Some(k),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, vec![BlockKind::Embed, BlockKind::Code]);
    }

    #[test]
    fn wikilink_hides_brackets_and_alias_part() {
        let text = "go [[Note|shown name]] end\nz";
        assert_eq!(shown(text, 0, OFF), "go shown name end");
        assert_eq!(shown(text, 0, 0..0), "go shown name end");
        assert_eq!(shown(text, 0, 4..4), "go [[Note|shown name]] end");
        let plain = "go [[Note#Head]]\nz";
        assert_eq!(shown(plain, 0, OFF), "go Note#Head");
        // An embed alone on its line is a block widget: nothing is drawn as text, the caret shows it.
        let embed = "![[pic]]\nz";
        assert_eq!(shown(embed, 0, OFF), "");
        assert_eq!(shown(embed, 0, 0..0), "![[pic]]");
    }

    #[test]
    fn bullet_is_a_glyph_and_numbers_stay() {
        // "- item" / "1. one" / "* star"
        let text = "- item\n1. one\n* star";
        assert_eq!(shown(text, 0, OFF), format!("{BULLET_TEXT} item"));
        assert_eq!(shown(text, 1, OFF), "1. one");
        assert_eq!(shown(text, 2, OFF), format!("{BULLET_TEXT} star"));
        // The caret on the bullet line shows the marker.
        assert_eq!(shown(text, 0, 0..0), "- item");
    }

    #[test]
    fn task_box_is_a_checkbox_record() {
        let text = "- [ ] open\n- [x] done\nz";
        let hides = compute_hides(text, &compute_spans(text));
        let cb: Vec<_> = hides
            .iter()
            .filter(|h| matches!(h.deco, Deco::Checkbox { .. }))
            .map(|h| (h.src.clone(), h.deco))
            .collect();
        assert_eq!(
            cb,
            vec![
                (2..5, Deco::Checkbox { checked: false }),
                (13..16, Deco::Checkbox { checked: true }),
            ]
        );
        // The dash is hidden, the space before the box stays.
        assert_eq!(shown(text, 0, OFF), format!(" {CHECKBOX_SPACE} open"));
        assert_eq!(shown(text, 0, 3..3), "- [ ] open");
    }

    #[test]
    fn block_quote_marker_becomes_an_indent_with_bar() {
        let text = "> quoted\nx";
        let hides = compute_hides(text, &compute_spans(text));
        assert_eq!(hides.len(), 1);
        assert_eq!(hides[0].deco, Deco::QuoteBar);
        assert_eq!(hides[0].text, QUOTE_SPACE);
        assert_eq!(shown(text, 0, OFF), "    quoted");
        assert_eq!(shown(text, 0, 0..0), "> quoted");
    }

    #[test]
    fn rule_line_is_empty_with_a_rule_decoration() {
        // A rule needs blank lines around it; "a\n---" would be a setext heading.
        let text = "a\n\n---\n\nb";
        let pieces = pieces_at(text, 2, OFF);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].deco, Deco::Rule);
        assert_eq!(display_text(&pieces, text), "");
        // Caret on the rule shows it.
        assert_eq!(shown(text, 2, 4..4), "---");
    }

    #[test]
    fn fenced_code_is_one_block_record() {
        // "```rust" / "let x = 1;" / "```" / "z": the block is the three fence lines.
        let text = "```rust\nlet x = 1;\n```\nz";
        let hides = compute_hides(text, &compute_spans(text));
        assert_eq!(hides.len(), 1);
        assert_eq!(hides[0].src, 0..22);
        assert_eq!(hides[0].reveal, 0..22);
        assert_eq!(hides[0].deco, Deco::Block(BlockKind::Code));
        // The block's inline records are gone: markers inside code are source.
        let inner = "```\n**not bold**\n```\n";
        let inner_hides = compute_hides(inner, &compute_spans(inner));
        assert_eq!(inner_hides.len(), 1);
        assert!(matches!(inner_hides[0].deco, Deco::Block(BlockKind::Code)));
    }

    #[test]
    fn frontmatter_is_one_block_record() {
        let text = "---\ntitle: x\n---\n# Head\n";
        let hides = compute_hides(text, &compute_spans(text));
        let fm: Vec<_> = hides
            .iter()
            .filter(|h| h.deco == Deco::Block(BlockKind::Frontmatter))
            .map(|h| h.src.clone())
            .collect();
        assert_eq!(fm, vec![0..16]);
        // The heading marker after it still hides.
        assert!(hides.iter().any(|h| h.src.start == 17));
    }

    #[test]
    fn nothing_is_hidden_when_not_live() {
        let text = "**b** # h";
        let spans = compute_spans(text);
        let lines = split_lines(text);
        let segs = line_segments(&spans, &lines);
        let hides = compute_hides(text, &spans);
        let p = line_pieces(lines[0].clone(), &segs[0], &hides, &OFF, None, false);
        assert_eq!(display_text(&p, text), text);
        assert!(p.iter().all(|x| x.replace.is_none()));
    }

    #[test]
    fn selection_spanning_lines_reveals_every_touched_line() {
        // "# A" (0..3), "plain **b**" (4..15), "# C" (16..19)
        let text = "# A\nplain **b**\n# C";
        let sel = 1..12;
        assert_eq!(shown(text, 0, sel.clone()), "# A");
        assert_eq!(shown(text, 1, sel.clone()), "plain **b**");
        assert_eq!(shown(text, 2, sel), "C");
    }

    #[test]
    fn overlapping_records_keep_the_first() {
        // Two heading markers over the same bytes: only the first survives.
        let text = "# a\n";
        let spans = vec![
            StyleSpan {
                range: 0..2,
                style: SpanStyle::of(SpanStyle::MARKER),
            },
            StyleSpan {
                range: 1..3,
                style: SpanStyle::of(SpanStyle::MARKER),
            },
        ];
        let hides = compute_hides(text, &spans);
        assert_eq!(hides.len(), 1);
        assert_eq!(hides[0].src, 0..2);
    }

    #[test]
    fn offsets_round_trip_on_plain_text() {
        let text = "hello world\nx";
        let p = pieces_at(text, 0, OFF);
        for s in 0..=11 {
            assert_eq!(to_display(&p, s), s);
            assert_eq!(to_source(&p, s, 11), s);
        }
    }

    #[test]
    fn offsets_across_a_hidden_bold() {
        // "a **bold** b" (0..12), the caret is on the next line.
        let text = "a **bold** b\nq";
        let p = pieces_at(text, 0, 13..13);
        assert_eq!(display_text(&p, text), "a bold b");
        assert_eq!(to_display(&p, 0), 0);
        assert_eq!(to_display(&p, 1), 1);
        // The visible 'b' (src 4) is display 2.
        assert_eq!(to_display(&p, 4), 2);
        assert_eq!(to_source(&p, 2, 12), 4);
        // Display 6 is the space after "bold", source 10 (after the closing `**`).
        assert_eq!(to_source(&p, 6, 12), 10);
        // The end of the line maps to display 8, the end of "a bold b".
        assert_eq!(to_display(&p, 12), 8);
    }

    #[test]
    fn cjk_and_emoji_offsets_are_byte_based() {
        // "**中文**😀 x": 中 at 2..5, 文 at 5..8, emoji at 10..14.
        let text = "**中文**😀 x\nq";
        let p = pieces_at(text, 0, OFF);
        assert_eq!(display_text(&p, text), "中文😀 x");
        assert_eq!(to_display(&p, 2), 0);
        assert_eq!(to_display(&p, 5), 3);
        assert_eq!(to_source(&p, 3, 16), 5);
        let emoji_src = text.find('😀').unwrap();
        let disp = to_display(&p, emoji_src);
        assert_eq!(disp, 6);
        assert_eq!(to_source(&p, disp, 16), emoji_src);
    }

    #[test]
    fn empty_lines_have_no_pieces_and_map_to_the_line_end() {
        let text = "a\n\nb";
        let p = pieces_at(text, 1, OFF);
        assert!(p.is_empty());
        assert_eq!(to_display(&p, 2), 0);
        assert_eq!(to_source(&p, 0, 2), 2);
    }

    #[test]
    fn click_into_a_hidden_marker_goes_to_the_visible_text() {
        // "x **bold** y" (0..12)
        let text = "x **bold** y\nq";
        let p = pieces_at(text, 0, 13..13);
        // Display 2 is the start of "bold", which maps past the hidden `**`.
        assert_eq!(to_source(&p, 2, 12), 4);
        // Display 6 is the space after "bold", source 10.
        assert_eq!(to_source(&p, 6, 12), 10);
    }

    #[test]
    fn replacement_maps_to_its_ends() {
        // "- item": the bullet replaces the dash at 0..1.
        let text = "- item\nq";
        let p = pieces_at(text, 0, 20..20);
        assert_eq!(to_source(&p, 0, 6), 0);
        assert_eq!(to_source(&p, 1, 6), 1);
        assert_eq!(to_display(&p, 1), BULLET_TEXT.len());
    }

    #[test]
    fn ime_range_is_flagged_per_piece() {
        let text = "ab\nq";
        let spans = compute_spans(text);
        let lines = split_lines(text);
        let segs = line_segments(&spans, &lines);
        let p = line_pieces(
            lines[0].clone(),
            &segs[0],
            &[],
            &(0..0),
            Some(&(1..2)),
            true,
        );
        let flags: Vec<bool> = p.iter().map(|x| x.ime).collect();
        assert_eq!(flags, vec![false, true]);
    }

    #[test]
    fn touching_counts_as_inside() {
        assert!(is_revealed(&(2..8), &(8..8)));
        assert!(is_revealed(&(2..8), &(0..2)));
        assert!(!is_revealed(&(2..8), &(9..9)));
        assert!(!is_revealed(&(2..8), &(0..1)));
    }

    #[test]
    fn long_documents_keep_record_offsets_absolute() {
        // A bold word on the third line: offsets are document offsets, not line offsets.
        let text = "one\ntwo\nx **y** z\n";
        let hides = compute_hides(text, &compute_spans(text));
        let src: Vec<_> = hides.iter().map(|h| h.src.clone()).collect();
        assert_eq!(
            src,
            vec![
                offset_of(text, "**y")..offset_of(text, "**y") + 2,
                offset_of(text, "y**") + 1..offset_of(text, "y**") + 3
            ]
        );
        let p = pieces_at(text, 2, OFF);
        assert_eq!(display_text(&p, text), "x y z");
    }

    /// Replaces the pieces of the inline formula at `span` with `cells` blank cells, as the editor
    /// does when the formula is drawn (see `text_input::inline_math_pass`).
    fn draw_inline(pieces: &mut [Piece], span: Range<usize>, cells: usize) {
        for p in pieces
            .iter_mut()
            .filter(|p| p.src.start >= span.start && p.src.end <= span.end)
        {
            p.replace = Some("");
        }
        if let Some(first) = pieces.iter_mut().find(|p| p.src.start == span.start) {
            first.gap = cells;
        }
    }

    #[test]
    fn inline_formula_is_a_record_with_its_source_range() {
        // "a $2^n$ b": the formula is bytes 2..7.
        let text = "a $2^n$ b\nq";
        let hides = compute_hides(text, &compute_spans(text));
        let inline: Vec<_> = hides
            .iter()
            .filter(|h| h.deco == Deco::InlineMath)
            .map(|h| h.src.clone())
            .collect();
        assert_eq!(inline, vec![2..7]);
        assert_eq!(&text[2..7], "$2^n$");
        // A formula inside code is not one.
        let code = "x `$a$` y\nq";
        assert!(
            compute_hides(code, &compute_spans(code))
                .iter()
                .all(|h| h.deco != Deco::InlineMath)
        );
    }

    #[test]
    fn inline_formula_shows_its_source_until_it_is_drawn() {
        let text = "a $2^n$ b\nq";
        assert_eq!(shown(text, 0, OFF), "a $2^n$ b");
        let p = pieces_at(text, 0, OFF);
        assert_eq!(p.iter().filter(|x| x.deco == Deco::InlineMath).count(), 1);
    }

    #[test]
    fn drawn_formula_maps_offsets_both_ways() {
        // "a $2^n$ b": the source "a " is display 0..2, the formula 2..7 is two blank cells,
        // then " b".
        let text = "a $2^n$ b\nq";
        let mut p = pieces_at(text, 0, OFF);
        draw_inline(&mut p, 2..7, 2);
        let cell = GAP_CELL.len();
        assert_eq!(display_text(&p, text), format!("a {GAP_CELL}{GAP_CELL} b"));
        assert_eq!(display_len(&p[1]), 2 * cell);
        // Display 2 is the start of the formula, display 8 is the end of it.
        assert_eq!(to_display(&p, 2), 2);
        assert_eq!(to_display(&p, 7), 2 + 2 * cell);
        assert_eq!(to_display(&p, 9), 2 + 2 * cell + 2);
        // Going back: the start of the formula, its inside goes to its end, the text after to 'b'.
        assert_eq!(to_source(&p, 2, 9), 2);
        assert_eq!(to_source(&p, 2 + cell, 9), 7);
        assert_eq!(to_source(&p, 2 + 2 * cell + 1, 9), 8);
    }

    #[test]
    fn caret_or_selection_in_a_drawn_formula_shows_raw_source() {
        let text = "a $2^n$ b\nq";
        // Caret inside, and touching the formula boundaries.
        for sel in [3..3, 2..2, 7..7, 1..4] {
            let p = pieces_at(text, 0, sel.clone());
            assert_eq!(display_text(&p, text), "a $2^n$ b", "sel {sel:?}");
        }
        // Caret away from it: the formula is drawn (blank cells), the text is hidden.
        let mut away = pieces_at(text, 0, 9..9);
        draw_inline(&mut away, 2..7, 1);
        assert!(!display_text(&away, text).contains('$'));
    }

    #[test]
    fn copy_of_a_drawn_formula_is_its_source() {
        // The pieces tile the source line, so the source text of the formula is "$2^n$".
        let text = "a $2^n$ b\nq";
        let mut p = pieces_at(text, 0, OFF);
        draw_inline(&mut p, 2..7, 2);
        let source: String = p
            .iter()
            .filter(|x| x.src.start >= 2 && x.src.end <= 7)
            .map(|x| &text[x.src.clone()])
            .collect();
        assert_eq!(source, "$2^n$");
    }

    #[test]
    fn read_mode_never_shows_source() {
        let reveal = 2..8;
        let on_caret = 5..5;
        let away = 20..20;
        // The live view shows source for a touching caret or a find hit.
        assert!(shows_source(true, false, &reveal, &on_caret, false));
        assert!(shows_source(true, false, &reveal, &away, true));
        assert!(!shows_source(true, false, &reveal, &away, false));
        // Read mode ignores the caret and the find match.
        assert!(!shows_source(true, true, &reveal, &on_caret, false));
        assert!(!shows_source(true, true, &reveal, &away, true));
        // Source mode (not live) never reveals a record.
        assert!(!shows_source(false, false, &reveal, &on_caret, true));
    }

    #[test]
    fn find_hit_inside_any_widget_reveals_it() {
        let inside = 4..6;
        let block = Deco::Block(BlockKind::Table);
        let src = 0..10;
        assert!(found_reveals(block, &src, Some(&inside)));
        assert!(found_reveals(
            Deco::Block(BlockKind::Mermaid),
            &src,
            Some(&inside)
        ));
        assert!(found_reveals(
            Deco::Block(BlockKind::Code),
            &src,
            Some(&inside)
        ));
        assert!(found_reveals(Deco::InlineMath, &src, Some(&inside)));
        // A hit outside the record, or no hit, reveals nothing.
        assert!(!found_reveals(block, &src, Some(&(10..12))));
        assert!(!found_reveals(block, &src, None));
        // Plain inline records never reveal through the find fallback.
        assert!(!found_reveals(Deco::None, &src, Some(&inside)));
        assert!(!found_reveals(
            Deco::Checkbox { checked: true },
            &src,
            Some(&inside)
        ));
    }

    fn offset_of(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap_or(0)
    }

    /// Per-frame cost of the pure part for a 5000-line note (run with `--ignored --nocapture`,
    /// preferably in release): the hides and the pieces of every line, as the editor computes
    /// them each frame.
    #[test]
    #[ignore]
    fn bench_pieces_for_five_thousand_lines() {
        let mut text = String::new();
        for i in 0..5000 {
            match i % 6 {
                0 => text.push_str(&format!("- 第 {i} 行 **粗体** and [[沉积的想法]]\n")),
                1 => text.push_str(&format!(
                    "普通段落 {i} with *italic* and `code` [link](https://x/{i}).\n"
                )),
                2 => text.push_str(&format!("- [ ] 任务 {i} open\n")),
                3 => text.push_str(&format!("> 引用 {i} ==高亮== text\n")),
                4 => text.push_str(&format!(
                    "Plain line {i} with ~~strike~~ words and more text to wrap.\n"
                )),
                _ => text.push('\n'),
            }
        }
        let spans = compute_spans(&text);
        let lines = split_lines(&text);
        let segs = line_segments(&spans, &lines);
        let started = std::time::Instant::now();
        let hides = compute_hides(&text, &spans);
        let hide_time = started.elapsed();
        let sel = 0..0;
        let started = std::time::Instant::now();
        let mut total = 0usize;
        let mut start = 0usize;
        for (i, line) in lines.iter().enumerate() {
            let end = start + line.len();
            let lo = hides.partition_point(|h| h.src.end <= start);
            let hi = hides.partition_point(|h| h.src.start < end);
            let pieces = line_pieces(
                start..end,
                &segs[i],
                &hides[lo..hi.max(lo)],
                &sel,
                None,
                true,
            );
            total += display_text(&pieces, &text).len();
            start = end + 1;
        }
        let frame_time = started.elapsed();
        eprintln!(
            "5000 lines: {} hides, compute_hides {:?}, per-frame pieces+display {:?} ({} bytes shown)",
            hides.len(),
            hide_time,
            frame_time,
            total
        );
    }
}
