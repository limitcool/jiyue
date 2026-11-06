//! Which lines of the live view can be reused after an edit (pure logic, no gpui).
//!
//! The editor compares the text of a frame with the text of the frame before. The two texts
//! share a prefix and a suffix; the bytes between them are the change. A logical line in the
//! prefix is unchanged and keeps its offsets. A line in the suffix has the same text but moved by
//! the length change. Every other line overlaps the change and is rebuilt.

use std::ops::Range;

/// The changed part of an edit: the bytes shared at the start and at the end of the two texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditSpan {
    /// Bytes at the start that are equal in both texts (on a char boundary).
    pub prefix: usize,
    /// Offset in the old text where the shared suffix starts (the change ends here).
    pub old_end: usize,
    /// Offset in the new text where the shared suffix starts.
    pub new_end: usize,
}

impl EditSpan {
    /// Growth of the text by the edit: new length minus old length.
    pub fn delta(&self) -> isize {
        self.new_end as isize - self.old_end as isize
    }
}

/// Finds the prefix and suffix that two texts share. Identical texts give a span with no change.
pub fn edit_span(old: &str, new: &str) -> EditSpan {
    let ob = old.as_bytes();
    let nb = new.as_bytes();
    let min_len = ob.len().min(nb.len());
    let mut prefix = ob.iter().zip(nb).take_while(|(a, b)| a == b).count();
    while prefix > 0 && !(old.is_char_boundary(prefix) && new.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let max_suffix = min_len - prefix;
    let mut suffix = ob
        .iter()
        .rev()
        .zip(nb.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);
    while suffix > 0
        && !(old.is_char_boundary(ob.len() - suffix) && new.is_char_boundary(nb.len() - suffix))
    {
        suffix -= 1;
    }
    EditSpan {
        prefix,
        old_end: ob.len() - suffix,
        new_end: nb.len() - suffix,
    }
}

/// Moves a byte offset by `delta`, never below zero.
pub fn shift(offset: usize, delta: isize) -> usize {
    offset.checked_add_signed(delta).unwrap_or(0)
}

/// Where an old offset is in the new text: an offset in the prefix stays, an offset in the suffix
/// moves by the change, and an offset inside the change keeps its value.
pub fn map_offset(offset: usize, span: Option<&EditSpan>) -> usize {
    match span {
        Some(s) if offset >= s.old_end => shift(offset, s.delta()),
        _ => offset,
    }
}

/// Moves a byte range by `delta`.
pub fn shift_range(range: &Range<usize>, delta: isize) -> Range<usize> {
    shift(range.start, delta)..shift(range.end, delta)
}

/// Where a logical line of the new text comes from in the old text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    /// Start offset of the line in the old text.
    pub old_start: usize,
    /// Index of the line in the old text.
    pub old_line: usize,
    /// Offset change of the line (0 for the prefix).
    pub delta: isize,
}

/// The old unit that a new unit of `len` bytes at `start` (line index `line`) can be reused from,
/// or `None` when the unit overlaps the change. `line_delta` is the change in the number of
/// newlines, which moves the line index of the suffix. With no span nothing changed.
pub fn source(
    start: usize,
    len: usize,
    line: usize,
    span: Option<&EditSpan>,
    line_delta: isize,
) -> Option<Source> {
    let Some(s) = span else {
        return Some(Source {
            old_start: start,
            old_line: line,
            delta: 0,
        });
    };
    let end = start + len;
    // The newline after the unit is at `end`: when it lies in the prefix the whole unit does.
    if end < s.prefix {
        return Some(Source {
            old_start: start,
            old_line: line,
            delta: 0,
        });
    }
    // A unit that starts in the shared suffix keeps its text. Its newline before (`start - 1`) must
    // be the same byte in both texts: in the prefix or in the suffix.
    let newline_same = start == 0 || start - 1 < s.prefix || start > s.new_end;
    if start >= s.new_end && newline_same {
        let delta = s.delta();
        return Some(Source {
            old_start: shift(start, -delta),
            old_line: shift(line, -line_delta),
            delta,
        });
    }
    None
}

/// Whether the reveal records of the last frame still line up with the new ones. A record in the
/// prefix must be unchanged, a record in the suffix must be moved by the change, and a record that
/// overlaps the change is rebuilt with its lines. A different number of records never lines up.
pub fn records_match(prev: &[Range<usize>], now: &[Range<usize>], span: Option<&EditSpan>) -> bool {
    if prev.len() != now.len() {
        return false;
    }
    prev.iter().zip(now).all(|(old, new)| match span {
        None => old == new,
        Some(s) if old.end <= s.prefix => old == new,
        Some(s) if old.start >= s.old_end => *new == shift_range(old, s.delta()),
        Some(_) => true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_starts(text: &str) -> Vec<(usize, usize)> {
        let mut start = 0;
        text.split('\n')
            .map(|l| {
                let item = (start, l.len());
                start += l.len() + 1;
                item
            })
            .collect()
    }

    /// Lines of `new` that can be reused, as (new start, old start).
    fn reused(old: &str, new: &str) -> Vec<(usize, usize)> {
        let span = edit_span(old, new);
        let nl = |t: &str| t.matches('\n').count() as isize;
        let line_delta = nl(new) - nl(old);
        line_starts(new)
            .into_iter()
            .enumerate()
            .filter_map(|(li, (s, l))| {
                source(s, l, li, Some(&span), line_delta).map(|src| (s, src.old_start))
            })
            .collect()
    }

    #[test]
    fn identical_texts_change_nothing() {
        let span = edit_span("abc\nde", "abc\nde");
        assert_eq!(span.delta(), 0);
        assert_eq!(span.prefix, 6);
        assert_eq!(span.old_end, 6);
    }

    #[test]
    fn insert_inside_a_line_keeps_other_lines() {
        let old = "one\ntwo\nthree";
        let new = "one\ntXwo\nthree";
        let got = reused(old, new);
        // Line 0 is in the prefix, line 2 is in the suffix and moved by one byte.
        assert_eq!(got, vec![(0, 0), (9, 8)]);
    }

    #[test]
    fn delete_shifts_the_suffix_back() {
        let old = "aa\nbb\ncc\ndd";
        let new = "aa\ncc\ndd";
        let got = reused(old, new);
        assert!(got.contains(&(0, 0)));
        assert!(got.contains(&(3, 6)));
        assert!(got.contains(&(6, 9)));
    }

    #[test]
    fn newline_split_rebuilds_the_split_line_only() {
        let old = "abcd\nxyz";
        let new = "ab\ncd\nxyz";
        let got = reused(old, new);
        // "ab" and "cd" overlap the change; "xyz" moved by one byte and keeps its line.
        assert_eq!(got, vec![(6, 5)]);
    }

    #[test]
    fn newline_merge_rebuilds_the_merged_line() {
        let old = "ab\ncd\nxyz";
        let new = "abcd\nxyz";
        let got = reused(old, new);
        assert_eq!(got, vec![(5, 6)]);
    }

    #[test]
    fn cjk_edit_keeps_char_boundaries() {
        let old = "中文一\n二三四";
        let new = "中文字\n二三四";
        let span = edit_span(old, new);
        assert!(old.is_char_boundary(span.prefix));
        assert!(old.is_char_boundary(span.old_end));
        assert!(new.is_char_boundary(span.new_end));
        let got = reused(old, new);
        assert_eq!(got, vec![(10, 10)]);
    }

    #[test]
    fn edit_inside_a_block_marks_the_block_dirty() {
        // A fenced block of three lines: the edit inside it makes the block unit dirty, while the
        // unit after it moves.
        let old = "```\nab\ncd\n```\ntail";
        let new = "```\nabX\ncd\n```\ntail";
        let span = edit_span(old, new);
        let block_len = old.find("\ntail").unwrap();
        let nl = |t: &str| t.matches('\n').count() as isize;
        let line_delta = nl(new) - nl(old);
        assert_eq!(source(0, block_len, 0, Some(&span), line_delta), None);
        let tail_start = new.find("tail").unwrap();
        let src = source(tail_start, 4, 4, Some(&span), line_delta).unwrap();
        assert_eq!(src.delta, 1);
        assert_eq!(src.old_start, tail_start - 1);
    }

    #[test]
    fn undo_gives_the_same_reuse_as_the_edit() {
        let old = "one\ntwo\nthree";
        let edited = "one\ntXwo\nthree";
        let forward = edit_span(old, edited);
        let back = edit_span(edited, old);
        assert_eq!(forward.delta(), -back.delta());
        assert_eq!(reused(edited, old), vec![(0, 0), (8, 9)]);
    }

    #[test]
    fn records_line_up_across_an_edit() {
        let old = vec![0..3, 10..14];
        let new = vec![0..3, 11..15];
        let span = edit_span("abc\n\nxyz\nqqqq", "abc\nZ\nxyz\nqqqq");
        assert!(records_match(&old, &new, Some(&span)));
        assert!(!records_match(
            &old,
            std::slice::from_ref(&(0..3)),
            Some(&span)
        ));
    }

    #[test]
    fn map_offset_follows_the_suffix() {
        let span = edit_span("ab\ncd\nef", "ab\nXXcd\nef");
        assert_eq!(map_offset(0, Some(&span)), 0);
        assert_eq!(map_offset(6, Some(&span)), 8);
        assert_eq!(map_offset(6, None), 6);
    }

    #[test]
    fn shift_never_goes_below_zero() {
        assert_eq!(shift(2, -5), 0);
        assert_eq!(shift(2, 3), 5);
        assert_eq!(shift_range(&(4..6), -1), 3..5);
    }
}
