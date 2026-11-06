//! Word-level comparison of the original text with a proposed replacement, for the diff card.
//!
//! Tokens are whitespace runs, runs of Latin letters and digits or punctuation, and single CJK
//! characters (Chinese text has no spaces). Concatenating the tokens gives back the input, so the
//! segments can always be replayed into either side.

use similar::{Algorithm, DiffOp, capture_diff_slices};

/// Whether a segment is kept, removed from the original, or added by the replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegKind {
    Same,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub kind: SegKind,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Cjk,
    Word,
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{303f}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{ff00}'..='\u{ffef}')
}

fn class_of(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if is_cjk(c) {
        Class::Cjk
    } else {
        Class::Word
    }
}

/// Splits text into tokens whose concatenation is the input.
pub fn tokens(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut current: Option<(usize, Class)> = None;
    for (i, c) in s.char_indices() {
        let cl = class_of(c);
        if cl == Class::Cjk {
            if let Some((start, _)) = current.take() {
                out.push(&s[start..i]);
            }
            out.push(&s[i..i + c.len_utf8()]);
            continue;
        }
        match current {
            Some((_, prev)) if prev == cl => {}
            Some((start, _)) => {
                out.push(&s[start..i]);
                current = Some((i, cl));
            }
            None => current = Some((i, cl)),
        }
    }
    if let Some((start, _)) = current {
        out.push(&s[start..]);
    }
    out
}

/// Word-level diff from `old` to `new`. Adjacent segments of the same kind are merged.
pub fn word_diff(old: &str, new: &str) -> Vec<Seg> {
    let a = tokens(old);
    let b = tokens(new);
    let ops = capture_diff_slices(Algorithm::Myers, &a, &b);
    let mut out: Vec<Seg> = Vec::new();
    for op in ops {
        match op {
            DiffOp::Equal { old_index, len, .. } => {
                push(&mut out, SegKind::Same, &a[old_index..old_index + len])
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => push(
                &mut out,
                SegKind::Removed,
                &a[old_index..old_index + old_len],
            ),
            DiffOp::Insert {
                new_index, new_len, ..
            } => push(&mut out, SegKind::Added, &b[new_index..new_index + new_len]),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                push(
                    &mut out,
                    SegKind::Removed,
                    &a[old_index..old_index + old_len],
                );
                push(&mut out, SegKind::Added, &b[new_index..new_index + new_len]);
            }
        }
    }
    out
}

fn push(out: &mut Vec<Seg>, kind: SegKind, tokens: &[&str]) {
    if tokens.is_empty() {
        return;
    }
    let text: String = tokens.concat();
    match out.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(&text),
        _ => out.push(Seg { kind, text }),
    }
}

/// The original side of a diff (kept and removed segments).
pub fn original_side(segs: &[Seg]) -> String {
    segs.iter()
        .filter(|s| s.kind != SegKind::Added)
        .map(|s| s.text.as_str())
        .collect()
}

/// The replacement side of a diff (kept and added segments).
pub fn replacement_side(segs: &[Seg]) -> String {
    segs.iter()
        .filter(|s| s.kind != SegKind::Removed)
        .map(|s| s.text.as_str())
        .collect()
}

/// Count of changed segments (removed or added), for the card header.
pub fn change_count(segs: &[Seg]) -> usize {
    segs.iter().filter(|s| s.kind != SegKind::Same).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_cover_the_input() {
        for s in ["Hello, 世界!  new\nline", "", "中文混合English 123", "  "] {
            let joined: String = tokens(s).concat();
            assert_eq!(joined, s);
        }
        assert_eq!(tokens("ab 中"), vec!["ab", " ", "中"]);
    }

    #[test]
    fn chinese_is_split_per_character() {
        assert_eq!(tokens("你好"), vec!["你", "好"]);
    }

    #[test]
    fn identical_text_has_no_changes() {
        let segs = word_diff("同样的文字", "同样的文字");
        assert_eq!(change_count(&segs), 0);
        assert_eq!(original_side(&segs), "同样的文字");
    }

    #[test]
    fn both_sides_replay_exactly() {
        let old = "The quick brown fox jumps over the lazy dog.";
        let new = "The quick red fox leaps over a lazy dog!";
        let segs = word_diff(old, new);
        assert_eq!(original_side(&segs), old);
        assert_eq!(replacement_side(&segs), new);
        assert!(change_count(&segs) > 0);
    }

    #[test]
    fn chinese_rewrite_marks_only_changed_characters() {
        let old = "今天天气很好我们去公园";
        let new = "今天天气非常好我们去公园";
        let segs = word_diff(old, new);
        assert_eq!(original_side(&segs), old);
        assert_eq!(replacement_side(&segs), new);
        let added: String = segs
            .iter()
            .filter(|s| s.kind == SegKind::Added)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(added, "非常");
    }

    #[test]
    fn adjacent_segments_of_one_kind_are_merged() {
        let segs = word_diff("a b c", "x y z");
        let kinds: Vec<SegKind> = segs.iter().map(|s| s.kind).collect();
        for pair in kinds.windows(2) {
            assert_ne!(pair[0], pair[1], "{segs:?}");
        }
    }

    #[test]
    fn empty_sides() {
        assert!(word_diff("", "").is_empty());
        let segs = word_diff("", "新增");
        assert_eq!(
            segs,
            vec![Seg {
                kind: SegKind::Added,
                text: "新增".into()
            }]
        );
        let segs = word_diff("删除", "");
        assert_eq!(
            segs,
            vec![Seg {
                kind: SegKind::Removed,
                text: "删除".into()
            }]
        );
    }
}
