//! Footnotes of the live view (pure logic, no gpui).
//!
//! `[^label]` is a reference and a line that starts with `[^label]:` is a definition. Labels are
//! matched without case. The number shown for a label is its ordinal: 1 for the first label used
//! in the document, 2 for the next new label, and so on (the same label always shows the same
//! number). The source keeps the label. Footnote syntax is not recognised inside code (spans and
//! fenced or indented blocks), inside frontmatter, after a backslash or a `!`, or as a definition
//! label line that pulldown-cmark does not take as a definition.
//!
//! [`scan`] runs once per document change (from `map::compute_hides`); the records it gives become
//! `map::Hide`s with [`map::Deco::FootnoteRef`] and [`map::Deco::FootnoteDef`]. Clicks use
//! [`jump_target`] over those hides, so no second index is kept.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag};

use super::blocks;
use super::map::{Deco, Hide};

/// One reference `[^label]`, in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// Source range of `[^label]`.
    pub range: Range<usize>,
    /// The label without case: references and the definition with the same key belong together.
    pub key: String,
    /// The number shown for the label (see the module docs).
    pub ordinal: u32,
    /// Whether a definition with this label exists. A reference without one is a warning.
    pub defined: bool,
}

/// One definition: its label prefix `[^label]:` with the spaces after it. The body is the rest of
/// the line. Only the first definition of a label counts; a later one with the same label stays
/// plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// Source range of `[^label]:` and the spaces after the colon.
    pub range: Range<usize>,
    pub key: String,
    /// The ordinal of the label when a reference uses it, `None` when no reference does.
    pub ordinal: Option<u32>,
    /// Blank cells the prefix is shown with (see [`map::footnote_gap`]).
    pub cells: u32,
}

/// The footnotes of one document, in source order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub refs: Vec<Reference>,
    pub defs: Vec<Definition>,
}

/// Where a click on a footnote leads (see [`jump_target`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jump {
    /// To the definition with this ordinal (from a reference).
    Definition(u32),
    /// To the first reference with this ordinal (from the back arrow of a definition).
    Reference(u32),
}

/// The text to insert for a new footnote and the caret position in the result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insert {
    pub text: String,
    pub caret: usize,
}

/// Number of decimal digits of `n` (at least one).
pub fn digits(n: u32) -> u32 {
    n.to_string().len() as u32
}

/// Whether `p` lies inside one of `ranges`. The ranges must be sorted by start and must not
/// overlap each other.
fn inside(ranges: &[Range<usize>], p: usize) -> bool {
    ranges.iter().any(|r| r.start <= p && p < r.end)
}

/// Whether only spaces or tabs come between the start of the line and `p`.
fn starts_line(text: &str, p: usize) -> bool {
    let line_start = text[..p].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..p].bytes().all(|b| b == b' ' || b == b'\t')
}

/// Finds the references and definitions of `text` (see the module docs).
pub fn scan(text: &str) -> Scan {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    // Places where footnote syntax does not count: code and frontmatter.
    let mut skip: Vec<Range<usize>> = blocks::frontmatter(text).into_iter().collect();
    // Definitions as pulldown-cmark takes them: the start of each block and its label.
    let mut raw_defs: Vec<(usize, String)> = Vec::new();
    for (event, range) in Parser::new_ext(text, opts).into_offset_iter() {
        match event {
            Event::Code(_) | Event::Start(Tag::CodeBlock(_)) => skip.push(range),
            Event::Start(Tag::FootnoteDefinition(label)) => {
                raw_defs.push((range.start, label.to_string()));
            }
            _ => {}
        }
    }
    skip.sort_by_key(|r| r.start);

    // Definitions: `[^label]:` at the block start, label up to the first `]:` on that line.
    let mut prefixes: Vec<Range<usize>> = Vec::new();
    let mut defs: Vec<(Range<usize>, String, u32)> = Vec::new();
    let mut def_keys: HashSet<String> = HashSet::new();
    let bytes = text.as_bytes();
    for (start, _label) in raw_defs {
        let Some(rest) = text.get(start..) else {
            continue;
        };
        if !rest.starts_with("[^") {
            continue;
        }
        let Some(close) = rest.find("]:") else {
            continue;
        };
        let label = &rest[2..close];
        if label.is_empty() || label.contains('\n') {
            continue;
        }
        let mut end = start + close + 2;
        while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t') {
            end += 1;
        }
        let key = label.to_lowercase();
        if !def_keys.insert(key.clone()) {
            continue;
        }
        prefixes.push(start..end);
        defs.push((start..end, key, label.chars().count() as u32));
    }
    prefixes.sort_by_key(|r| r.start);

    // References: every `[^` that is not in code, not escaped and closes on its line.
    let mut raw_refs: Vec<(Range<usize>, String)> = Vec::new();
    let mut from = 0usize;
    while let Some(off) = text.get(from..).and_then(|t| t.find("[^")) {
        let p = from + off;
        from = p + 2;
        if inside(&skip, p) || inside(&prefixes, p) {
            continue;
        }
        if p > 0 && (bytes[p - 1] == b'\\' || bytes[p - 1] == b'!') {
            continue;
        }
        // The label ends at the first `]`; another `[` or a line break means no reference.
        let mut q = p + 2;
        let mut closed = false;
        while q < bytes.len() {
            match bytes[q] {
                b']' => {
                    closed = true;
                    break;
                }
                b'[' | b'\n' => break,
                _ => q += 1,
            }
        }
        if !closed || q == p + 2 {
            continue;
        }
        // `[^label]:` at the start of a line is a definition the parser did not take: not a reference.
        if bytes.get(q + 1) == Some(&b':') && starts_line(text, p) {
            continue;
        }
        raw_refs.push((p..q + 1, text[p + 2..q].to_lowercase()));
        from = q + 1;
    }

    // Ordinals in first-reference order.
    let mut ordinals: HashMap<String, u32> = HashMap::new();
    let mut refs = Vec::with_capacity(raw_refs.len());
    for (range, key) in raw_refs {
        let next = ordinals.len() as u32 + 1;
        let ordinal = *ordinals.entry(key.clone()).or_insert(next);
        let defined = def_keys.contains(&key);
        refs.push(Reference {
            range,
            key,
            ordinal,
            defined,
        });
    }
    let defs = defs
        .into_iter()
        .map(|(range, key, chars)| {
            let ordinal = ordinals.get(&key).copied();
            let cells = match ordinal {
                Some(n) => digits(n) + 2,
                None => chars.max(1),
            };
            Definition {
                range,
                key,
                ordinal,
                cells,
            }
        })
        .collect();
    Scan { refs, defs }
}

/// The label of a definition prefix that starts at `start` in `text` (`[^note]: ` gives `note`).
/// `None` when the text there is not a definition prefix.
pub fn def_label_at(text: &str, start: usize) -> Option<&str> {
    let rest = text.get(start..)?;
    let body = rest.strip_prefix("[^")?;
    let close = body.find("]:")?;
    let label = &body[..close];
    (!label.is_empty() && !label.contains('\n')).then_some(label)
}

/// The number of the reference under source offset `at` (inclusive at both ends), if the offset
/// is in a reference record.
pub fn ref_at(hides: &[Hide], at: usize) -> Option<u32> {
    hides.iter().find_map(|h| match h.deco {
        Deco::FootnoteRef { ordinal, .. } if h.src.start <= at && at <= h.src.end => Some(ordinal),
        _ => None,
    })
}

/// Where `jump` leads: the start of the definition prefix, or of the first reference, in the
/// source. `None` when there is no such record (a reference without a definition has none).
pub fn jump_target(hides: &[Hide], jump: Jump) -> Option<usize> {
    hides.iter().find_map(|h| match (h.deco, jump) {
        (
            Deco::FootnoteDef {
                ordinal: Some(n), ..
            },
            Jump::Definition(want),
        ) if n == want => Some(h.src.start),
        (Deco::FootnoteRef { ordinal, .. }, Jump::Reference(want)) if ordinal == want => {
            Some(h.src.start)
        }
        _ => None,
    })
}

/// A hash of the numbers and widths that the footnote records show, in document order. The live
/// view reuses built lines across an edit only while this is unchanged: a new reference can
/// renumber the references after it, and their lines would otherwise keep the old numbers.
pub fn numbering_salt(hides: &[Hide]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for hide in hides {
        match hide.deco {
            Deco::FootnoteRef { ordinal, missing } => (1u8, ordinal, missing).hash(&mut h),
            Deco::FootnoteDef { ordinal, cells } => (2u8, ordinal, cells).hash(&mut h),
            _ => {}
        }
    }
    h.finish()
}

/// The smallest number from 1 that no footnote label of `text` uses (labels that are numbers).
pub fn next_free_number(text: &str) -> u32 {
    let s = scan(text);
    let used: HashSet<u32> = s
        .refs
        .iter()
        .map(|r| r.key.as_str())
        .chain(s.defs.iter().map(|d| d.key.as_str()))
        .filter_map(|k| k.parse::<u32>().ok())
        .collect();
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
}

/// The text after inserting a new footnote at `caret`: the reference `[^n]` at the caret and the
/// definition `[^n]: ` at the end of the document, where `n` is [`next_free_number`]. The caret
/// goes after the colon and its space. Blank lines are added only where the document needs them.
/// A caret that is not on a character boundary moves back to one.
pub fn plan_insert(text: &str, caret: usize) -> Insert {
    let n = next_free_number(text);
    let mut at = caret.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str(&text[..at]);
    out.push_str(&format!("[^{n}]"));
    out.push_str(&text[at..]);
    let sep = if out.ends_with("\n\n") {
        ""
    } else if out.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    out.push_str(sep);
    out.push_str(&format!("[^{n}]: "));
    let caret = out.len();
    Insert { text: out, caret }
}

#[cfg(test)]
mod tests {
    use super::super::map::{compute_hides, display_text, line_pieces};
    use super::*;
    use crate::ui::md_style::{compute_spans, line_segments, split_lines};

    /// A selection far outside every test document.
    const OFF: Range<usize> = 10_000..10_000;

    fn refs(text: &str) -> Vec<(String, u32, bool)> {
        scan(text)
            .refs
            .into_iter()
            .map(|r| (text[r.range.clone()].to_string(), r.ordinal, r.defined))
            .collect()
    }

    #[test]
    fn reference_and_definition_are_found_and_numbered() {
        let text = "Text[^1] and[^2].\n\n[^1]: one\n[^2]: two\n";
        let s = scan(text);
        assert_eq!(s.refs.len(), 2);
        assert_eq!((s.refs[0].ordinal, s.refs[1].ordinal), (1, 2));
        assert!(s.refs.iter().all(|r| r.defined));
        assert_eq!(s.defs.len(), 2);
        assert_eq!(s.defs[0].ordinal, Some(1));
        assert_eq!(&text[s.defs[0].range.clone()], "[^1]: ");
        assert_eq!(s.defs[0].cells, 3);
    }

    #[test]
    fn numbers_follow_first_reference_not_the_label() {
        let text = "a[^b] c[^a] d[^b]\n\n[^a]: x\n[^b]: y\n";
        let got = refs(text);
        assert_eq!(
            got,
            vec![
                ("[^b]".to_string(), 1, true),
                ("[^a]".to_string(), 2, true),
                ("[^b]".to_string(), 1, true),
            ]
        );
    }

    #[test]
    fn labels_match_without_case() {
        let text = "x[^Note] y[^note]\n\n[^NOTE]: body\n";
        let s = scan(text);
        assert_eq!(
            s.refs.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
            vec![1, 1]
        );
        assert!(s.refs.iter().all(|r| r.defined));
        assert_eq!(s.defs[0].ordinal, Some(1));
    }

    #[test]
    fn cjk_labels_keep_byte_ranges_valid() {
        let text = "见[^注释]。\n\n[^注释]: 说明文字\n";
        let s = scan(text);
        assert_eq!(s.refs.len(), 1);
        assert_eq!(&text[s.refs[0].range.clone()], "[^注释]");
        assert!(s.refs[0].defined);
        assert_eq!(&text[s.defs[0].range.clone()], "[^注释]: ");
        assert_eq!(def_label_at(text, s.defs[0].range.start), Some("注释"));
        assert_eq!(s.defs[0].cells, digits(1) + 2);
    }

    #[test]
    fn duplicate_definitions_keep_the_first() {
        let text = "x[^1]\n\n[^1]: first\n\n[^1]: second\n";
        let s = scan(text);
        assert_eq!(s.defs.len(), 1);
        assert!(text[s.defs[0].range.end..].starts_with("first"));
        assert_eq!(s.refs.len(), 1);
    }

    #[test]
    fn definitions_inside_fences_are_not_footnotes() {
        let text = "```\n[^1]: no\n```\n\ntext[^1]\n";
        let s = scan(text);
        assert!(s.defs.is_empty());
        assert_eq!(s.refs.len(), 1);
        assert!(!s.refs[0].defined);
        assert_eq!(s.refs[0].ordinal, 1);
    }

    #[test]
    fn references_inside_code_spans_are_not_footnotes() {
        let text = "use `[^1]` here[^2]\n\n[^2]: x\n";
        let got = refs(text);
        assert_eq!(got, vec![("[^2]".to_string(), 1, true)]);
    }

    #[test]
    fn nested_brackets_and_escapes_are_not_references() {
        assert!(scan("x [^a[b]] y").refs.is_empty());
        assert!(scan("x \\[^1] y").refs.is_empty());
        assert!(scan("x ![^1] y").refs.is_empty());
        assert!(scan("x [^] y").refs.is_empty());
        assert!(scan("x [^1 y\nz").refs.is_empty());
    }

    #[test]
    fn missing_definition_is_a_reference_without_a_definition() {
        let text = "x[^9]\n";
        let s = scan(text);
        assert_eq!(s.refs.len(), 1);
        assert!(!s.refs[0].defined);
        assert!(s.defs.is_empty());
    }

    #[test]
    fn unused_definition_has_no_ordinal() {
        let text = "[^5]: lone\n";
        let s = scan(text);
        assert_eq!(s.defs.len(), 1);
        assert_eq!(s.defs[0].ordinal, None);
        assert_eq!(s.defs[0].cells, 1);
        assert_eq!(def_label_at(text, 0), Some("5"));
    }

    #[test]
    fn a_definition_label_line_is_not_a_reference_but_its_body_can_hold_one() {
        let text = "[^1]: body [^2]\n\n[^2]: b\n";
        let s = scan(text);
        assert_eq!(s.refs.len(), 1);
        assert_eq!(&text[s.refs[0].range.clone()], "[^2]");
        assert_eq!(s.refs[0].ordinal, 1);
        assert_eq!(s.defs.len(), 2);
        assert_eq!(s.defs[0].ordinal, None);
        assert_eq!(s.defs[1].ordinal, Some(1));
    }

    #[test]
    fn frontmatter_is_not_scanned() {
        let text = "---\nnote: \"[^1]\"\n---\n\nbody[^1]\n\n[^1]: d\n";
        let s = scan(text);
        assert_eq!(s.refs.len(), 1);
        assert!(s.refs[0].range.start > 20);
    }

    #[test]
    fn jump_targets_go_both_ways_and_missing_has_none() {
        let text = "a[^1] b[^1] c[^2]\n\n[^1]: one\n";
        let spans = compute_spans(text);
        let hides = compute_hides(text, &spans);
        let def_at = scan(text).defs[0].range.start;
        assert_eq!(jump_target(&hides, Jump::Definition(1)), Some(def_at));
        assert_eq!(jump_target(&hides, Jump::Reference(1)), Some(1));
        // [^2] has no definition: no target in either direction.
        assert_eq!(jump_target(&hides, Jump::Definition(2)), None);
        assert_eq!(ref_at(&hides, 2), Some(1));
        assert_eq!(ref_at(&hides, 0), None);
    }

    #[test]
    fn next_free_number_skips_used_numbers() {
        assert_eq!(next_free_number(""), 1);
        assert_eq!(next_free_number("a[^1] b[^2]"), 3);
        assert_eq!(next_free_number("a[^2]\n\n[^1]: x"), 3);
        assert_eq!(next_free_number("a[^note]"), 1);
        assert_eq!(next_free_number("`[^1]`"), 1);
    }

    #[test]
    fn insert_into_an_empty_note() {
        let ins = plan_insert("", 0);
        assert_eq!(ins.text, "[^1]\n\n[^1]: ");
        assert_eq!(ins.caret, ins.text.len());
    }

    #[test]
    fn insert_at_the_caret_and_append_the_definition() {
        let ins = plan_insert("a\n[^1]: x", 1);
        assert_eq!(ins.text, "a[^2]\n[^1]: x\n\n[^2]: ");
        assert_eq!(ins.caret, ins.text.len());
    }

    #[test]
    fn insert_after_a_trailing_newline_adds_one_blank_line() {
        assert_eq!(plan_insert("a\n", 2).text, "a\n[^1]\n\n[^1]: ");
        assert_eq!(plan_insert("a\n\n", 3).text, "a\n\n[^1]\n\n[^1]: ");
    }

    #[test]
    fn insert_with_a_caret_inside_a_character_moves_back() {
        // "注" is three bytes; the caret at byte 1 is inside it.
        let ins = plan_insert("注", 1);
        assert_eq!(ins.text, "[^1]注\n\n[^1]: ");
    }

    #[test]
    fn reference_shows_its_ordinal_only_when_the_caret_is_away() {
        let text = "a[^12] b";
        let spans = compute_spans(text);
        let lines = split_lines(text);
        let segs = line_segments(&spans, &lines);
        let hides = compute_hides(text, &spans);
        let away = line_pieces(lines[0].clone(), &segs[0], &hides, &OFF, None, true);
        // The label `12` is the first one used, so its ordinal is 1: one blank cell.
        assert_eq!(display_text(&away, text), "a\u{2002} b");
        // With the caret on the reference it is shown as source.
        let inside = line_pieces(lines[0].clone(), &segs[0], &hides, &(3..3), None, true);
        assert_eq!(display_text(&inside, text), "a[^12] b");
    }

    #[test]
    fn missing_reference_keeps_its_number_as_a_warning_record() {
        let text = "x[^3]";
        let spans = compute_spans(text);
        let hides = compute_hides(text, &spans);
        assert!(hides.iter().any(|h| h.deco
            == Deco::FootnoteRef {
                ordinal: 1,
                missing: true
            }));
    }
}
