//! Line diff with word-level highlight ranges.
//!
//! Lines are diffed with `similar` (Myers). Inside a replaced block, paired
//! lines are compared token by token. A token is an ASCII/Unicode word run,
//! a run of whitespace, a single CJK character (so Chinese and Japanese text
//! highlights per character instead of per long run), or a single punctuation
//! character. Highlights are byte ranges into the line text.

use std::ops::Range;

use similar::{Algorithm, DiffTag as SimilarTag, TextDiff, capture_diff_slices};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTag {
    Equal,
    Insert,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub tag: DiffTag,
    /// 1-based line number in the old text; `None` for inserted lines.
    pub old_line: Option<usize>,
    /// 1-based line number in the new text; `None` for deleted lines.
    pub new_line: Option<usize>,
    /// The line without its terminator.
    pub text: String,
    /// Byte ranges into `text` that changed. Empty for equal lines.
    pub highlights: Vec<Range<usize>>,
}

/// Diff two texts line by line.
pub fn diff_text(old: &str, new: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let old_lines: Vec<&str> = diff.old_slices().iter().map(|s| strip_eol(s)).collect();
    let new_lines: Vec<&str> = diff.new_slices().iter().map(|s| strip_eol(s)).collect();
    let mut out = Vec::new();
    for op in diff.ops() {
        let (tag, old_r, new_r) = op.as_tag_tuple();
        match tag {
            SimilarTag::Equal => {
                for (i, j) in old_r.zip(new_r) {
                    out.push(make_line(
                        DiffTag::Equal,
                        Some(i),
                        Some(j),
                        old_lines[i],
                        Vec::new(),
                    ));
                }
            }
            SimilarTag::Delete => {
                for i in old_r {
                    let text = old_lines[i];
                    out.push(make_line(
                        DiffTag::Delete,
                        Some(i),
                        None,
                        text,
                        full_range(text),
                    ));
                }
            }
            SimilarTag::Insert => {
                for j in new_r {
                    let text = new_lines[j];
                    out.push(make_line(
                        DiffTag::Insert,
                        None,
                        Some(j),
                        text,
                        full_range(text),
                    ));
                }
            }
            SimilarTag::Replace => {
                let mut old_hl: Vec<Vec<Range<usize>>> =
                    old_r.clone().map(|i| full_range(old_lines[i])).collect();
                let mut new_hl: Vec<Vec<Range<usize>>> =
                    new_r.clone().map(|j| full_range(new_lines[j])).collect();
                let pairs = old_r.len().min(new_r.len());
                for k in 0..pairs {
                    let (o, n) =
                        word_highlights(old_lines[old_r.start + k], new_lines[new_r.start + k]);
                    old_hl[k] = o;
                    new_hl[k] = n;
                }
                for (k, i) in old_r.enumerate() {
                    let hl = std::mem::take(&mut old_hl[k]);
                    out.push(make_line(DiffTag::Delete, Some(i), None, old_lines[i], hl));
                }
                for (k, j) in new_r.enumerate() {
                    let hl = std::mem::take(&mut new_hl[k]);
                    out.push(make_line(DiffTag::Insert, None, Some(j), new_lines[j], hl));
                }
            }
        }
    }
    out
}

fn make_line(
    tag: DiffTag,
    old_idx: Option<usize>,
    new_idx: Option<usize>,
    text: &str,
    highlights: Vec<Range<usize>>,
) -> DiffLine {
    DiffLine {
        tag,
        old_line: old_idx.map(|i| i + 1),
        new_line: new_idx.map(|j| j + 1),
        text: text.to_string(),
        highlights,
    }
}

fn strip_eol(s: &str) -> &str {
    let s = s.strip_suffix('\n').unwrap_or(s);
    s.strip_suffix('\r').unwrap_or(s)
}

fn full_range(s: &str) -> Vec<Range<usize>> {
    if s.trim().is_empty() {
        Vec::new()
    } else {
        std::iter::once(0..s.len()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Cjk,
    Punct,
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3000}'..='\u{303F}'
        | '\u{3040}'..='\u{30FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FFEF}'
        | '\u{20000}'..='\u{2FA1F}')
}

fn classify(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if is_cjk(c) {
        Class::Cjk
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// Split into tokens (byte ranges covering the whole string).
fn tokenize(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let class = classify(c);
        let mut end = start + c.len_utf8();
        if matches!(class, Class::Word | Class::Space) {
            while let Some(&(i, d)) = chars.peek() {
                if classify(d) != class {
                    break;
                }
                end = i + d.len_utf8();
                chars.next();
            }
        }
        out.push(start..end);
    }
    out
}

fn push_span(out: &mut Vec<Range<usize>>, text: &str, tokens: &[Range<usize>], span: Range<usize>) {
    if span.is_empty() {
        return;
    }
    let start = tokens[span.start].start;
    let end = tokens[span.end - 1].end;
    if text[start..end].trim().is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.end == start
    {
        last.end = end;
        return;
    }
    out.push(start..end);
}

/// Token-level highlights for one pair of lines: (old side, new side).
fn word_highlights(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    if old == new {
        return (Vec::new(), Vec::new());
    }
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    let old_strs: Vec<&str> = old_tokens.iter().map(|r| &old[r.clone()]).collect();
    let new_strs: Vec<&str> = new_tokens.iter().map(|r| &new[r.clone()]).collect();
    let mut old_hl = Vec::new();
    let mut new_hl = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &old_strs, &new_strs) {
        let (tag, o, n) = op.as_tag_tuple();
        match tag {
            SimilarTag::Equal => {}
            SimilarTag::Delete => push_span(&mut old_hl, old, &old_tokens, o),
            SimilarTag::Insert => push_span(&mut new_hl, new, &new_tokens, n),
            SimilarTag::Replace => {
                push_span(&mut old_hl, old, &old_tokens, o);
                push_span(&mut new_hl, new, &new_tokens, n);
            }
        }
    }
    (old_hl, new_hl)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(lines: &[DiffLine]) -> Vec<DiffTag> {
        lines.iter().map(|l| l.tag).collect()
    }

    fn highlighted(line: &DiffLine) -> Vec<&str> {
        line.highlights
            .iter()
            .map(|r| &line.text[r.clone()])
            .collect()
    }

    #[test]
    fn identical_texts_are_all_equal() {
        let d = diff_text("a\nb\n", "a\nb\n");
        assert_eq!(tags(&d), vec![DiffTag::Equal, DiffTag::Equal]);
        assert!(d.iter().all(|l| l.highlights.is_empty()));
    }

    #[test]
    fn both_empty_gives_no_lines() {
        assert!(diff_text("", "").is_empty());
    }

    #[test]
    fn insertion_into_empty_old() {
        let d = diff_text("", "x\ny\n");
        assert_eq!(tags(&d), vec![DiffTag::Insert, DiffTag::Insert]);
        assert_eq!(d[0].new_line, Some(1));
        assert_eq!(d[1].new_line, Some(2));
        assert_eq!(d[0].old_line, None);
    }

    #[test]
    fn deletion_to_empty_new() {
        let d = diff_text("x\n", "");
        assert_eq!(tags(&d), vec![DiffTag::Delete]);
        assert_eq!(d[0].old_line, Some(1));
        assert_eq!(d[0].new_line, None);
        assert_eq!(d[0].highlights, vec![0..1]);
    }

    #[test]
    fn line_numbers_track_both_sides() {
        let d = diff_text("a\nb\nc\n", "a\nX\nc\n");
        let eq_first = &d[0];
        assert_eq!((eq_first.old_line, eq_first.new_line), (Some(1), Some(1)));
        let del = d.iter().find(|l| l.tag == DiffTag::Delete).expect("delete");
        assert_eq!(del.old_line, Some(2));
        let ins = d.iter().find(|l| l.tag == DiffTag::Insert).expect("insert");
        assert_eq!(ins.new_line, Some(2));
        let eq_last = d.last().expect("last");
        assert_eq!((eq_last.old_line, eq_last.new_line), (Some(3), Some(3)));
    }

    #[test]
    fn word_change_highlights_only_the_word() {
        let d = diff_text("the quick fox\n", "the slow fox\n");
        let del = d.iter().find(|l| l.tag == DiffTag::Delete).expect("delete");
        let ins = d.iter().find(|l| l.tag == DiffTag::Insert).expect("insert");
        assert_eq!(highlighted(del), vec!["quick"]);
        assert_eq!(highlighted(ins), vec!["slow"]);
    }

    #[test]
    fn whole_word_is_highlighted_not_a_fragment() {
        let d = diff_text("food\n", "foods\n");
        let del = d.iter().find(|l| l.tag == DiffTag::Delete).expect("delete");
        let ins = d.iter().find(|l| l.tag == DiffTag::Insert).expect("insert");
        assert_eq!(highlighted(del), vec!["food"]);
        assert_eq!(highlighted(ins), vec!["foods"]);
    }

    #[test]
    fn cjk_change_highlights_only_changed_characters() {
        let d = diff_text("今天天气很好\n", "今天天气不错\n");
        let del = d.iter().find(|l| l.tag == DiffTag::Delete).expect("delete");
        let ins = d.iter().find(|l| l.tag == DiffTag::Insert).expect("insert");
        assert_eq!(highlighted(del), vec!["很好"]);
        assert_eq!(highlighted(ins), vec!["不错"]);
    }

    #[test]
    fn cjk_highlight_ranges_are_char_boundaries() {
        let d = diff_text("设计文档一\n", "设计说明一\n");
        for line in &d {
            for r in &line.highlights {
                assert!(line.text.is_char_boundary(r.start));
                assert!(line.text.is_char_boundary(r.end));
            }
        }
    }

    #[test]
    fn mixed_latin_and_cjk_line() {
        let d = diff_text("note: 会议 at 3pm\n", "note: 会议 at 4pm\n");
        let del = d.iter().find(|l| l.tag == DiffTag::Delete).expect("delete");
        assert_eq!(highlighted(del), vec!["3pm"]);
    }

    #[test]
    fn replaced_block_pairs_lines_for_highlights() {
        let d = diff_text("alpha one\nbeta two\n", "alpha uno\nbeta dos\n");
        let dels: Vec<&DiffLine> = d.iter().filter(|l| l.tag == DiffTag::Delete).collect();
        let inss: Vec<&DiffLine> = d.iter().filter(|l| l.tag == DiffTag::Insert).collect();
        assert_eq!(dels.len(), 2);
        assert_eq!(inss.len(), 2);
        assert_eq!(highlighted(dels[0]), vec!["one"]);
        assert_eq!(highlighted(inss[1]), vec!["dos"]);
    }

    #[test]
    fn unpaired_lines_in_a_block_are_fully_highlighted() {
        let d = diff_text("keep\nold1\nold2\nold3\n", "keep\nnew1\n");
        let inserted = d.iter().find(|l| l.tag == DiffTag::Insert).expect("insert");
        assert_eq!(highlighted(inserted), vec!["new1"]);
        let dels: Vec<&DiffLine> = d.iter().filter(|l| l.tag == DiffTag::Delete).collect();
        assert_eq!(dels.len(), 3);
        assert_eq!(dels[2].highlights, vec![0..4]);
    }

    #[test]
    fn crlf_line_endings_are_stripped() {
        let d = diff_text("a\r\nb\r\n", "a\r\nc\r\n");
        assert_eq!(
            d.iter().find(|l| l.tag == DiffTag::Equal).expect("eq").text,
            "a"
        );
        assert_eq!(
            d.iter()
                .find(|l| l.tag == DiffTag::Insert)
                .expect("ins")
                .text,
            "c"
        );
    }

    #[test]
    fn missing_final_newline_is_handled() {
        // "a" without a newline differs from "a\n", so it is a delete + insert pair.
        let d = diff_text("a", "a\nb");
        assert_eq!(d.iter().filter(|l| l.tag == DiffTag::Delete).count(), 1);
        assert!(d.iter().any(|l| l.tag == DiffTag::Insert && l.text == "b"));
        assert_eq!(d.iter().filter(|l| l.tag == DiffTag::Insert).count(), 2);
    }

    #[test]
    fn whitespace_only_change_is_not_highlighted_as_text() {
        let d = diff_text("a  b\n", "a b\n");
        for line in &d {
            for r in &line.highlights {
                assert!(!line.text[r.clone()].trim().is_empty());
            }
        }
    }

    #[test]
    fn tokenizer_covers_the_whole_string() {
        let s = "héllo, 世界 foo_bar  x";
        let toks = tokenize(s);
        let joined: String = toks.iter().map(|r| &s[r.clone()]).collect();
        assert_eq!(joined, s);
        assert!(toks.iter().any(|r| &s[r.clone()] == "世"));
        assert!(toks.iter().any(|r| &s[r.clone()] == "foo_bar"));
    }

    #[test]
    fn many_line_document_diff_is_consistent() {
        let old: String = (0..300).map(|i| format!("line {i}\n")).collect();
        let new: String = (0..300)
            .map(|i| {
                if i % 50 == 0 {
                    format!("changed {i}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        let d = diff_text(&old, &new);
        let deletes = d.iter().filter(|l| l.tag == DiffTag::Delete).count();
        let inserts = d.iter().filter(|l| l.tag == DiffTag::Insert).count();
        assert_eq!(deletes, 6);
        assert_eq!(inserts, 6);
    }
}
