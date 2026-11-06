//! Callouts (admonitions) of the live view: `> [!type] Title` quotes (pure logic, no gpui).
//!
//! A callout is a block quote whose first line starts with `[!type]`. While the caret is outside
//! it, it is drawn as a tinted panel with a bar, an icon, the title and the body. The `+` and `-`
//! after the type make it foldable: `+` starts open, `-` starts closed (only the title row is
//! shown). A fold toggled by a click lives in memory only (see [`fold_key`]); the source is not
//! changed. Aliases share one style (see [`classify`]).

use std::hash::{Hash, Hasher};
use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag};

use super::map::{self, Deco, Piece};
use crate::ui::md_style::{self, StyleSpan};

/// The colour family of a callout. Blue is the accent, green the success colour, amber and red
/// are constants per theme (the theme has no warning or danger token), neutral is the secondary text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Blue,
    Green,
    Amber,
    Red,
    Neutral,
}

/// Whether a callout folds, and the state it starts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    /// No `+` or `-`: always open.
    Fixed,
    /// `[!type]+`: starts open.
    Open,
    /// `[!type]-`: starts closed.
    Closed,
}

/// A parsed callout. Ranges are absolute document bytes. `body` holds the content of every line
/// after the header (quote markers removed), empty lines included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Callout {
    /// The type as written, lower case.
    pub kind: String,
    pub tone: Tone,
    /// The icon name in `assets/icons` (without the folder and extension), when there is one.
    pub icon: Option<&'static str>,
    /// The title as written; `None` gives the default title of the type (see [`default_title`]).
    pub title: Option<Range<usize>>,
    pub fold: Fold,
    /// The header line, without its newline.
    pub header: Range<usize>,
    pub body: Vec<Range<usize>>,
}

/// The tone, icon and default title of a callout type. Aliases map to the same entry; an unknown
/// type shows as a note, like Obsidian does, with its own name as title.
pub fn classify(kind: &str) -> (Tone, Option<&'static str>, Option<&'static str>) {
    let k = kind.to_ascii_lowercase();
    match k.as_str() {
        "note" => (Tone::Blue, Some("pencil"), Some("Note")),
        "abstract" | "summary" | "tldr" => (Tone::Blue, Some("book-2"), Some("Abstract")),
        "info" => (Tone::Blue, Some("info-circle"), Some("Info")),
        "todo" => (Tone::Blue, Some("check"), Some("Todo")),
        "tip" | "hint" | "important" => (Tone::Green, Some("bulb"), Some("Tip")),
        "success" | "check" | "done" => (Tone::Green, Some("check"), Some("Success")),
        "question" | "help" | "faq" => (Tone::Amber, Some("help-circle"), Some("Question")),
        "warning" | "caution" | "attention" => {
            (Tone::Amber, Some("alert-triangle"), Some("Warning"))
        }
        "failure" | "fail" | "missing" => (Tone::Red, Some("x"), Some("Failure")),
        "danger" | "error" => (Tone::Red, Some("x"), Some("Danger")),
        "bug" => (Tone::Red, Some("bug"), Some("Bug")),
        "example" => (Tone::Neutral, Some("hash"), Some("Example")),
        "quote" | "cite" => (Tone::Neutral, Some("quote"), Some("Quote")),
        _ => (Tone::Blue, Some("pencil"), None),
    }
}

/// The title shown when the header has none: the type's default title, or the type itself with
/// its first letter capitalised.
pub fn default_title(kind: &str) -> String {
    match classify(kind).2 {
        Some(t) => t.to_string(),
        None => {
            let mut cs = kind.chars();
            match cs.next() {
                Some(c) => c.to_uppercase().collect::<String>() + cs.as_str(),
                None => String::new(),
            }
        }
    }
}

/// Byte offset (in the line) where the content after a block quote marker starts: up to three
/// spaces, a `>` and one optional space. A line without a marker (a lazy continuation) starts at 0.
fn content_start(line: &str) -> usize {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && i < 3 && b[i] == b' ' {
        i += 1;
    }
    if i < b.len() && b[i] == b'>' {
        i += 1;
        if i < b.len() && b[i] == b' ' {
            i += 1;
        }
        i
    } else {
        0
    }
}

/// Trims ASCII whitespace from both ends of a range of `text`.
fn trim(text: &str, r: Range<usize>) -> Range<usize> {
    let s = text.get(r.clone()).unwrap_or("");
    let lead = s.len() - s.trim_start().len();
    let trail = s.len() - s.trim_end().len();
    if lead + trail >= s.len() {
        let at = r.start + s.len();
        return at..at;
    }
    r.start + lead..r.end - trail
}

/// Parses the callout whose whole source lines are `block`: the header line and the lines after it
/// (as a block quote). `None` when the first line is not a `[!type]` header.
pub fn parse(text: &str, block: Range<usize>) -> Option<Callout> {
    let src = text.get(block.clone())?;
    let mut lines: Vec<Range<usize>> = Vec::new();
    let mut off = block.start;
    for part in src.split('\n') {
        lines.push(off..off + part.len());
        off += part.len() + 1;
    }
    let header = lines.first()?.clone();
    let head_text = text.get(header.clone())?;
    let content_off = content_start(head_text);
    let content = header.start + content_off..header.end;
    let raw = text.get(content.clone())?;
    if !raw.starts_with("[!") {
        return None;
    }
    let close = raw.find(']')?;
    let kind = &raw[2..close];
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let after = close + 1;
    let (fold, rest_at) = match raw.as_bytes().get(after) {
        Some(b'+') => (Fold::Open, after + 1),
        Some(b'-') => (Fold::Closed, after + 1),
        _ => (Fold::Fixed, after),
    };
    let title_rel = trim(text, content.start + rest_at..content.end);
    let title = (!title_rel.is_empty()).then_some(title_rel);
    let (tone, icon, _) = classify(kind);
    let body = lines[1..]
        .iter()
        .map(|l| {
            let s = content_start(text.get(l.clone()).unwrap_or(""));
            trim_right(text, l.start + s..l.end)
        })
        .collect();
    Some(Callout {
        kind: kind.to_ascii_lowercase(),
        tone,
        icon,
        title,
        fold,
        header,
        body,
    })
}

/// Trims trailing whitespace (and a carriage return) from a range of `text`.
fn trim_right(text: &str, r: Range<usize>) -> Range<usize> {
    let s = text.get(r.clone()).unwrap_or("");
    let trail = s.len() - s.trim_end().len();
    r.start..r.end - trail
}

/// The whole source lines of every callout block: an outermost block quote that starts its line
/// (not inside a list item), whose first line is a `[!type]` header. Frontmatter is skipped.
pub fn callout_blocks(text: &str) -> Vec<Range<usize>> {
    let fm_end = md_style::frontmatter_end(text);
    let mut opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    opts.insert(Options::ENABLE_TASKLISTS);
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut covered = 0usize;
    for (event, range) in Parser::new_ext(text, opts).into_offset_iter() {
        let Event::Start(Tag::BlockQuote(_)) = event else {
            continue;
        };
        if range.start >= range.end || range.end > text.len() || range.start < covered {
            continue;
        }
        if fm_end.is_some_and(|end| range.start < end) {
            continue;
        }
        covered = range.end;
        // Only a quote at the very start of its line: an indented one may sit in a list item.
        let first = line_bounds(text, range.start);
        if first.start != range.start {
            continue;
        }
        let last = line_bounds(text, range.end - 1);
        let block = first.start..last.end;
        if parse(text, block.clone()).is_some() {
            out.push(block);
        }
    }
    out
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

/// The key that keeps a fold toggle with its callout while the text changes elsewhere: the hash of
/// the header line, and the number of earlier callouts with the same header.
pub fn fold_key(header: &str, ordinal: usize) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    header.hash(&mut h);
    ordinal.hash(&mut h);
    h.finish()
}

/// Hash of a header line (the part of `fold_key` that does not depend on the position).
pub fn header_hash(header: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    header.hash(&mut h);
    h.finish()
}

/// The pieces of one body line of a callout (see `map::line_pieces`): the line's own Markdown
/// (inline markers, bullets, headings) with the markers hidden away from the caret. Checkbox boxes,
/// quote bars and rules are not painted in a callout, so their markers stay as text.
pub fn body_pieces(text: &str, line: Range<usize>) -> Vec<Piece> {
    let Some(sub) = text.get(line.clone()) else {
        return Vec::new();
    };
    if sub.is_empty() {
        return Vec::new();
    }
    let base = line.start;
    let spans: Vec<StyleSpan> = md_style::compute_spans(sub);
    let mut hides: Vec<map::Hide> = map::compute_hides(sub, &spans)
        .into_iter()
        .filter(|h| h.deco == Deco::None)
        .map(|mut h| {
            h.src = h.src.start + base..h.src.end + base;
            h.reveal = h.reveal.start + base..h.reveal.end + base;
            h
        })
        .collect();
    hides.sort_by_key(|h| h.src.start);
    let shifted: Vec<StyleSpan> = spans
        .into_iter()
        .map(|s| StyleSpan {
            range: s.range.start + base..s.range.end + base,
            style: s.style,
        })
        .collect();
    let segs = md_style::line_segments(&shifted, std::slice::from_ref(&line))
        .into_iter()
        .next()
        .unwrap_or_default();
    map::line_pieces(line, &segs, &hides, &(usize::MAX..usize::MAX), None, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Callout {
        let blocks = callout_blocks(text);
        assert_eq!(blocks.len(), 1, "one callout in {text:?}");
        parse(text, blocks[0].clone()).expect("callout")
    }

    #[test]
    fn every_type_and_alias_maps_to_a_tone() {
        assert_eq!(classify("note").0, Tone::Blue);
        assert_eq!(classify("SUMMARY").0, Tone::Blue);
        assert_eq!(classify("hint").0, Tone::Green);
        assert_eq!(classify("check").0, Tone::Green);
        assert_eq!(classify("caution").0, Tone::Amber);
        assert_eq!(classify("fail").0, Tone::Red);
        assert_eq!(classify("error").0, Tone::Red);
        assert_eq!(classify("cite").0, Tone::Neutral);
        assert_eq!(classify("custom").0, Tone::Blue);
        assert_eq!(default_title("tip"), "Tip");
        assert_eq!(default_title("custom"), "Custom");
    }

    #[test]
    fn header_with_title_and_body() {
        let text = "> [!warning] 小心\n> 第一行\n> - 列表\n\nafter";
        let c = parsed(text);
        assert_eq!(c.kind, "warning");
        assert_eq!(c.tone, Tone::Amber);
        assert_eq!(c.fold, Fold::Fixed);
        assert_eq!(&text[c.title.clone().unwrap()], "小心");
        let body: Vec<&str> = c.body.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(body, vec!["第一行", "- 列表"]);
    }

    #[test]
    fn fold_markers_set_the_start_state() {
        assert_eq!(parsed("> [!tip]+ Open\n> a\n").fold, Fold::Open);
        assert_eq!(parsed("> [!tip]- Closed\n> a\n").fold, Fold::Closed);
        // Without a title the default one is used.
        let c = parsed("> [!note]\n> body\n");
        assert!(c.title.is_none());
        assert_eq!(c.body.len(), 1);
    }

    #[test]
    fn plain_quote_and_bad_headers_are_not_callouts() {
        assert!(callout_blocks("> just a quote\n").is_empty());
        assert!(callout_blocks("> [!] empty type\n").is_empty());
        assert!(callout_blocks("> [!a b] spaces\n").is_empty());
        // Inside a list item: left alone.
        assert!(callout_blocks("- item\n  > [!note] x\n").is_empty());
    }

    #[test]
    fn nested_quote_is_part_of_its_callout() {
        let text = "> [!note] A\n> > inner\n\nz";
        let blocks = callout_blocks(text);
        assert_eq!(blocks, vec![0..text.find("\n\n").unwrap()]);
    }

    #[test]
    fn callout_inside_frontmatter_or_code_is_skipped() {
        let fm = "---\na: 1\n> [!note] x\n---\n";
        assert!(callout_blocks(fm).is_empty());
        let code = "```\n> [!note] x\n```\n";
        assert!(callout_blocks(code).is_empty());
    }

    #[test]
    fn fold_key_depends_on_header_and_ordinal() {
        assert_eq!(fold_key("> [!tip]- a", 0), fold_key("> [!tip]- a", 0));
        assert_ne!(fold_key("> [!tip]- a", 0), fold_key("> [!tip]- a", 1));
        assert_ne!(fold_key("> [!tip]- a", 0), fold_key("> [!tip]- b", 0));
    }

    #[test]
    fn body_line_hides_inline_markers_and_keeps_checkboxes() {
        let text = "> [!todo] t\n> - [ ] **go** now\n";
        let c = parsed(text);
        let line = c.body[0].clone();
        let pieces = body_pieces(text, line.clone());
        let shown = map::display_text(&pieces, text);
        // The bullet glyph replaces the dash, the bold markers are hidden, the box stays as text.
        assert!(shown.contains("[ ] go now"), "shown: {shown:?}");
        assert!(!shown.contains("**"));
        assert!(!shown.contains(map::CHECKBOX_SPACE));
        assert!(
            shown.starts_with(' ') || shown.starts_with(map::BULLET_TEXT.chars().next().unwrap())
        );
    }

    #[test]
    fn block_is_a_record_in_the_map() {
        let text = "> [!note] Title\n> body **b**\n\n# h\n";
        let end = text.find("\n\n").unwrap();
        let hides = map::compute_hides(text, &md_style::compute_spans(text));
        let blk: Vec<_> = hides
            .iter()
            .filter(|h| h.deco == Deco::Block(map::BlockKind::Callout))
            .map(|h| h.src.clone())
            .collect();
        assert_eq!(blk, vec![0..end]);
        // The inline records inside the callout are gone; the heading after it still hides.
        assert!(
            !hides
                .iter()
                .any(|h| h.deco == Deco::None && h.src.start < end)
        );
        assert!(hides.iter().any(|h| h.src.start == end + 2));
    }

    #[test]
    fn every_callout_icon_is_registered_in_assets() {
        use gpui::AssetSource;
        let kinds = [
            "note",
            "abstract",
            "info",
            "todo",
            "tip",
            "success",
            "question",
            "help",
            "faq",
            "warning",
            "caution",
            "attention",
            "failure",
            "danger",
            "bug",
            "example",
            "quote",
            "cite",
            "unknown-type",
        ];
        for kind in kinds {
            if let Some(name) = classify(kind).1 {
                let path = format!("icons/{name}.svg");
                let loaded = crate::ui::assets::Assets.load(&path);
                assert!(
                    matches!(loaded, Ok(Some(_))),
                    "callout {kind} uses icon {path} which is not registered"
                );
            }
        }
    }
}
