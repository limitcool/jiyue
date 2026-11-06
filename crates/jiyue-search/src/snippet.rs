//! Snippet extraction and highlight ranges. The snippet is taken from the stored body, and its
//! edges are snapped to whitespace where possible. Highlights come from re-analysing the body,
//! so they always line up with real token boundaries.

use std::collections::HashSet;
use std::ops::Range;

use crate::analysis::index_tokens;

/// Characters of context kept before the first match.
const BEFORE_CHARS: usize = 40;
/// Total characters in the snippet window, before ellipses are added.
const WINDOW_CHARS: usize = 120;
/// How far an edge may move to land on whitespace.
const SNAP_CHARS: usize = 12;
const ELLIPSIS: &str = "…";

/// A snippet with highlight ranges (byte offsets into `text`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snippet {
    pub text: String,
    pub highlights: Vec<Range<usize>>,
}

/// Builds a snippet of `body` around the first token whose text is in `terms`.
pub(crate) fn build(body: &str, terms: &HashSet<String>) -> Snippet {
    let anchor = if terms.is_empty() {
        None
    } else {
        index_tokens(body).find(|tok| terms.contains(&tok.text))
    };

    let (start, end) = match &anchor {
        Some(tok) => {
            let start = snap_start(body, back_chars(body, tok.start, BEFORE_CHARS), tok.start);
            let end = forward_chars(body, start, WINDOW_CHARS).max(tok.end);
            (start, snap_end(body, end, tok.end))
        }
        None => (0, snap_end(body, forward_chars(body, 0, WINDOW_CHARS), 0)),
    };

    let prefix = if start > 0 { ELLIPSIS } else { "" };
    let suffix = if end < body.len() { ELLIPSIS } else { "" };
    let mut text = String::with_capacity(prefix.len() + (end - start) + suffix.len());
    text.push_str(prefix);
    // Line breaks and tabs become spaces. Both are one byte, so offsets stay valid.
    text.extend(body[start..end].chars().map(|c| {
        if matches!(c, '\n' | '\r' | '\t') {
            ' '
        } else {
            c
        }
    }));
    text.push_str(suffix);

    let mut highlights = Vec::new();
    if !terms.is_empty() {
        let offset = prefix.len();
        for tok in index_tokens(body) {
            if tok.start >= end {
                break;
            }
            if tok.start >= start && tok.end <= end && terms.contains(&tok.text) {
                highlights.push(tok.start - start + offset..tok.end - start + offset);
            }
        }
    }

    Snippet {
        text,
        highlights: merge(highlights),
    }
}

/// Sorts ranges and merges the ones that overlap or touch.
fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// Byte index `n` chars before `byte` (or 0).
fn back_chars(s: &str, byte: usize, n: usize) -> usize {
    s[..byte]
        .char_indices()
        .rev()
        .nth(n.saturating_sub(1))
        .map_or(0, |(i, _)| i)
}

/// Byte index `n` chars after `byte` (or the end of `s`).
fn forward_chars(s: &str, byte: usize, n: usize) -> usize {
    s[byte..]
        .char_indices()
        .nth(n)
        .map_or(s.len(), |(i, _)| byte + i)
}

/// Moves a start edge forward past the first whitespace nearby, never beyond `ceiling`.
fn snap_start(s: &str, start: usize, ceiling: usize) -> usize {
    if start == 0 {
        return 0;
    }
    let window_end = forward_chars(s, start, SNAP_CHARS);
    if let Some(p) = s[start..window_end].find(char::is_whitespace) {
        let ws = start + p;
        let after = ws + s[ws..].chars().next().map_or(0, char::len_utf8);
        if after <= ceiling {
            return after;
        }
    }
    start
}

/// Moves an end edge back to the last whitespace nearby, never before `floor`.
fn snap_end(s: &str, end: usize, floor: usize) -> usize {
    if end >= s.len() {
        return s.len();
    }
    let window_start = back_chars(s, end, SNAP_CHARS);
    if let Some(p) = s[window_start..end].rfind(char::is_whitespace) {
        let candidate = window_start + p;
        if candidate >= floor {
            return candidate;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(words: &[&str]) -> HashSet<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    fn highlighted(snip: &Snippet) -> Vec<&str> {
        snip.highlights
            .iter()
            .map(|r| &snip.text[r.clone()])
            .collect()
    }

    fn assert_boundaries(snip: &Snippet) {
        for r in &snip.highlights {
            assert!(snip.text.is_char_boundary(r.start), "start {r:?}");
            assert!(snip.text.is_char_boundary(r.end), "end {r:?}");
            assert!(r.start < r.end);
        }
    }

    #[test]
    fn english_match_is_highlighted() {
        let snip = build("Learn Rust today", &terms(&["rust"]));
        assert_eq!(snip.text, "Learn Rust today");
        assert_eq!(highlighted(&snip), vec!["Rust"]);
        assert_boundaries(&snip);
    }

    #[test]
    fn every_occurrence_is_highlighted() {
        let snip = build("rust and RUST and rust", &terms(&["rust"]));
        assert_eq!(highlighted(&snip), vec!["rust", "RUST", "rust"]);
    }

    #[test]
    fn cjk_phrase_bigrams_merge_into_one_range() {
        let snip = build("我们学习异步编程的基础", &terms(&["异步", "步编", "编程"]));
        assert_eq!(highlighted(&snip), vec!["异步编程"]);
        assert_boundaries(&snip);
    }

    #[test]
    fn single_cjk_char_is_highlighted_alone() {
        let snip = build("异步编程", &terms(&["步"]));
        assert_eq!(highlighted(&snip), vec!["步"]);
    }

    #[test]
    fn unmatched_terms_produce_no_highlights() {
        let snip = build("nothing here", &terms(&["rust"]));
        assert!(snip.highlights.is_empty());
    }

    #[test]
    fn empty_body_gives_empty_snippet() {
        let snip = build("", &terms(&["rust"]));
        assert_eq!(snip.text, "");
        assert!(snip.highlights.is_empty());
    }

    #[test]
    fn snippet_is_centred_on_first_match_with_ellipses() {
        let filler = "lorem ipsum dolor sit amet ".repeat(30);
        let body = format!("{filler}needle here {filler}");
        let snip = build(&body, &terms(&["needle"]));
        assert!(snip.text.starts_with(ELLIPSIS));
        assert!(snip.text.ends_with(ELLIPSIS));
        assert!(snip.text.contains("needle"));
        let chars = snip.text.chars().count();
        assert!(
            chars <= WINDOW_CHARS + 2 * ELLIPSIS.chars().count() + SNAP_CHARS,
            "{chars}"
        );
        assert_eq!(highlighted(&snip), vec!["needle"]);
        assert_boundaries(&snip);
    }

    #[test]
    fn snippet_falls_back_to_start_of_body() {
        let snip = build("Intro paragraph about things", &terms(&["missing"]));
        assert!(snip.text.starts_with("Intro"));
        assert!(!snip.text.starts_with(ELLIPSIS));
    }

    #[test]
    fn long_body_without_match_is_truncated_with_suffix() {
        let body = "word ".repeat(200);
        let snip = build(&body, &terms(&["zzz"]));
        assert!(snip.text.ends_with(ELLIPSIS));
        assert!(!snip.text.starts_with(ELLIPSIS));
        assert!(snip.text.len() < 200);
    }

    #[test]
    fn newlines_become_spaces_and_offsets_stay_valid() {
        let body = "line one\nline two rust\nline three";
        let snip = build(body, &terms(&["rust"]));
        assert!(!snip.text.contains('\n'));
        assert_eq!(highlighted(&snip), vec!["rust"]);
        assert_boundaries(&snip);
    }

    #[test]
    fn cjk_snippet_ranges_are_on_char_boundaries() {
        let body = format!("{}异步编程{}", "中文内容很长".repeat(20), "结束".repeat(40));
        let snip = build(&body, &terms(&["异步", "步编", "编程"]));
        assert_boundaries(&snip);
        assert_eq!(highlighted(&snip), vec!["异步编程"]);
        assert!(snip.text.chars().count() <= WINDOW_CHARS + 2 + SNAP_CHARS);
    }

    #[test]
    fn highlight_does_not_match_inside_a_longer_word() {
        // "rust" must not highlight the middle of "trusty".
        let snip = build("trusty rust", &terms(&["rust"]));
        assert_eq!(highlighted(&snip), vec!["rust"]);
    }

    #[test]
    fn highlight_covers_latin_and_cjk_across_boundary() {
        // Touching ranges merge, so the whole "rust异步" is one highlight.
        let snip = build("rust异步", &terms(&["rust", "异步"]));
        assert_eq!(highlighted(&snip), vec!["rust异步"]);
        assert_boundaries(&snip);
    }

    #[test]
    fn merge_combines_overlaps_and_touching_ranges() {
        assert_eq!(merge(vec![0..3, 2..5, 5..6, 9..10]), vec![0..6, 9..10]);
        assert_eq!(merge(vec![4..5, 0..1]), vec![0..1, 4..5]);
        assert!(merge(vec![]).is_empty());
    }

    #[test]
    fn back_and_forward_char_helpers() {
        let s = "a异b步c";
        assert_eq!(forward_chars(s, 0, 2), "a异".len());
        assert_eq!(forward_chars(s, 0, 100), s.len());
        assert_eq!(back_chars(s, s.len(), 2), "a异b".len());
        assert_eq!(back_chars(s, 0, 5), 0);
    }
}
