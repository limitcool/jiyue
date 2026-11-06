//! Three-way text merge (line based, diff3 style). Owned by W1-D2.
//!
//! Both sides are diffed against the base with Myers (via `similar`). Hunks that do not touch
//! each other are applied together. Hunks whose base ranges overlap are grouped into clusters;
//! a cluster whose local and remote versions differ becomes a conflict block with markers.
//! Append-only edits on both sides (typical for daily notes) are kept in full, local lines first,
//! skipping remote lines that the local additions already contain.
//!
//! Line endings: a file whose dominant line ending is CRLF is merged with LF and written back
//! with CRLF. The final-newline state follows the side that changed it.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};

use crate::MergeResult;

/// Opening marker of a conflict block (the local side follows).
pub const MARKER_LOCAL: &str = "<<<<<<< 本机";
/// Separator between the local and remote versions inside a conflict block.
pub const MARKER_SPLIT: &str = "=======";
/// Closing marker of a conflict block.
pub const MARKER_REMOTE: &str = ">>>>>>> 远端";

/// Time budget for each diff. On timeout `similar` returns a coarser but still valid diff.
const DIFF_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Local,
    Remote,
}

/// A replacement of base lines `bs..be` by `new` lines, produced by one side.
#[derive(Debug)]
struct Hunk<'a> {
    bs: usize,
    be: usize,
    new: Vec<&'a str>,
    side: Side,
}

/// A group of hunks (from one or both sides) whose base ranges overlap.
#[derive(Debug)]
struct Cluster<'a> {
    cs: usize,
    ce: usize,
    hunks: Vec<Hunk<'a>>,
}

/// Three-way merge of text.
///
/// Returns [`MergeResult::Clean`] when every change applies without overlap (or when both sides
/// made the same change, or only one side changed). Returns [`MergeResult::Conflict`] with
/// markers around each overlapping, differing region; `hunks` counts those regions.
pub fn three_way_merge(base: &str, local: &str, remote: &str) -> MergeResult {
    if local == remote {
        return MergeResult::Clean(local.to_owned());
    }
    if base == local {
        return MergeResult::Clean(remote.to_owned());
    }
    if base == remote {
        return MergeResult::Clean(local.to_owned());
    }

    let crlf = prefers_crlf(&[base, local, remote]);
    let base_n = normalize(base);
    let local_n = normalize(local);
    let remote_n = normalize(remote);
    let (b, b_nl) = split_lines(&base_n);
    let (l, l_nl) = split_lines(&local_n);
    let (r, r_nl) = split_lines(&remote_n);
    // An emptied side has no final-newline state of its own: use the other side's.
    let (l_nl, r_nl) = match (l.is_empty(), r.is_empty()) {
        (true, false) => (r_nl, r_nl),
        (false, true) => (l_nl, l_nl),
        _ => (l_nl, r_nl),
    };
    let final_nl = pick_final_newline(b_nl, l_nl, r_nl);

    // Both sides only appended at the end: keep both additions.
    if let (Some(la), Some(ra)) = (appended(&b, &l), appended(&b, &r)) {
        let mut out = b.clone();
        out.extend_from_slice(&la);
        out.extend(unmatched_lines(&la, &ra));
        return MergeResult::Clean(assemble(&out, final_nl, crlf));
    }

    let deadline = Instant::now() + DIFF_BUDGET;
    let mut hunks = diff_hunks(&b, &l, Side::Local, deadline);
    hunks.extend(diff_hunks(&b, &r, Side::Remote, deadline));
    hunks.sort_by_key(|x| (x.bs, x.be, x.side));

    let mut clusters: Vec<Cluster> = Vec::new();
    for h in hunks {
        match clusters.last_mut() {
            Some(c) if interacts(c.cs, c.ce, h.bs, h.be) => {
                c.ce = c.ce.max(h.be);
                c.hunks.push(h);
            }
            _ => clusters.push(Cluster {
                cs: h.bs,
                ce: h.be,
                hunks: vec![h],
            }),
        }
    }

    let mut out: Vec<&str> = Vec::with_capacity(b.len() + 16);
    let mut conflicts = 0usize;
    let mut pos = 0usize;
    for c in &clusters {
        out.extend_from_slice(&b[pos..c.cs]);
        let has_local = c.hunks.iter().any(|h| h.side == Side::Local);
        let has_remote = c.hunks.iter().any(|h| h.side == Side::Remote);
        match (has_local, has_remote) {
            (true, false) => out.extend(render(&b, c, Side::Local)),
            (false, true) => out.extend(render(&b, c, Side::Remote)),
            _ => {
                let lv = render(&b, c, Side::Local);
                let rv = render(&b, c, Side::Remote);
                if lv == rv {
                    out.extend(lv);
                } else {
                    conflicts += 1;
                    out.push(MARKER_LOCAL);
                    out.extend(lv);
                    out.push(MARKER_SPLIT);
                    out.extend(rv);
                    out.push(MARKER_REMOTE);
                }
            }
        }
        pos = c.ce;
    }
    out.extend_from_slice(&b[pos..]);

    let text = assemble(&out, final_nl, crlf);
    if conflicts == 0 {
        MergeResult::Clean(text)
    } else {
        MergeResult::Conflict {
            merged_with_markers: text,
            hunks: conflicts,
        }
    }
}

/// Replaces CRLF with LF. Borrows when there is nothing to replace.
fn normalize(s: &str) -> Cow<'_, str> {
    if s.contains("\r\n") {
        Cow::Owned(s.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(s)
    }
}

/// True when CRLF line endings are the majority across the given texts.
fn prefers_crlf(texts: &[&str]) -> bool {
    let mut crlf = 0usize;
    let mut bare = 0usize;
    for t in texts {
        let all = t.matches('\n').count();
        let pairs = t.matches("\r\n").count();
        crlf += pairs;
        bare += all - pairs;
    }
    crlf > bare
}

/// Splits LF-normalized text into lines without terminators. Returns the lines and whether the
/// text ends with a newline. The empty text has no lines.
fn split_lines(s: &str) -> (Vec<&str>, bool) {
    if s.is_empty() {
        return (Vec::new(), false);
    }
    let nl = s.ends_with('\n');
    let body = if nl { &s[..s.len() - 1] } else { s };
    (body.split('\n').collect(), nl)
}

/// Joins lines with the chosen line ending, adding a final ending when requested.
fn assemble(lines: &[&str], final_nl: bool, crlf: bool) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let eol = if crlf { "\r\n" } else { "\n" };
    let mut s = String::with_capacity(lines.iter().map(|l| l.len() + 2).sum());
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            s.push_str(eol);
        }
        s.push_str(line);
    }
    if final_nl {
        s.push_str(eol);
    }
    s
}

/// The final-newline state: the side that changed it wins; when both agree, that state is used.
fn pick_final_newline(base_nl: bool, local_nl: bool, remote_nl: bool) -> bool {
    if local_nl == remote_nl {
        local_nl
    } else if local_nl == base_nl {
        remote_nl
    } else {
        local_nl
    }
}

/// When `side` starts with all of `base`, returns the lines it added at the end.
fn appended<'a>(base: &[&'a str], side: &[&'a str]) -> Option<Vec<&'a str>> {
    if side.len() >= base.len() && side[..base.len()] == *base {
        Some(side[base.len()..].to_vec())
    } else {
        None
    }
}

/// Remote lines that are not part of an LCS match with the local additions, in order.
fn unmatched_lines<'a>(local: &[&'a str], remote: &[&'a str]) -> Vec<&'a str> {
    let deadline = Instant::now() + DIFF_BUDGET;
    let mut matched = vec![false; remote.len()];
    for op in capture_diff_slices_deadline(Algorithm::Myers, local, remote, Some(deadline)) {
        if let DiffOp::Equal { new_index, len, .. } = op {
            for m in &mut matched[new_index..new_index + len] {
                *m = true;
            }
        }
    }
    remote
        .iter()
        .zip(matched)
        .filter(|(_, m)| !m)
        .map(|(line, _)| *line)
        .collect()
}

/// Hunks of one side against the base, adjacent or overlapping ones coalesced.
fn diff_hunks<'a>(
    base: &[&'a str],
    side_lines: &[&'a str],
    side: Side,
    deadline: Instant,
) -> Vec<Hunk<'a>> {
    let ops = capture_diff_slices_deadline(Algorithm::Myers, base, side_lines, Some(deadline));
    let mut out: Vec<Hunk<'a>> = Vec::new();
    for op in ops {
        let (bs, be, ns, ne) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index, old_index + old_len, new_index, new_index),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index, old_index, new_index, new_index + new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (
                old_index,
                old_index + old_len,
                new_index,
                new_index + new_len,
            ),
        };
        let new: Vec<&'a str> = side_lines[ns..ne].to_vec();
        if let Some(last) = out.last_mut()
            && bs <= last.be
        {
            last.be = last.be.max(be);
            last.new.extend(new);
            continue;
        }
        out.push(Hunk { bs, be, new, side });
    }
    out
}

/// Whether a hunk covering base `s..e` overlaps a cluster covering `cs..ce`.
/// Non-empty ranges conflict only when they share base lines; insertions conflict when they fall
/// strictly inside a range, or at the same point as another insertion.
fn interacts(cs: usize, ce: usize, s: usize, e: usize) -> bool {
    if s == e {
        (cs < s && s < ce) || (cs == ce && cs == s)
    } else {
        s < ce && cs < e
    }
}

/// The lines one side ends up with inside a cluster (its hunks applied over the base).
fn render<'a>(base: &[&'a str], c: &Cluster<'a>, side: Side) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = Vec::new();
    let mut pos = c.cs;
    for h in c.hunks.iter().filter(|h| h.side == side) {
        out.extend_from_slice(&base[pos..h.bs]);
        out.extend_from_slice(&h.new);
        pos = h.be;
    }
    out.extend_from_slice(&base[pos..c.ce]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(base: &str, local: &str, remote: &str) -> String {
        match three_way_merge(base, local, remote) {
            MergeResult::Clean(text) => text,
            MergeResult::Conflict {
                merged_with_markers,
                hunks,
            } => panic!("expected clean merge, got {hunks} conflicts:\n{merged_with_markers}"),
        }
    }

    fn conflict(base: &str, local: &str, remote: &str) -> (String, usize) {
        match three_way_merge(base, local, remote) {
            MergeResult::Conflict {
                merged_with_markers,
                hunks,
            } => (merged_with_markers, hunks),
            MergeResult::Clean(text) => panic!("expected conflict, got clean:\n{text}"),
        }
    }

    #[test]
    fn identical_sides_are_clean() {
        assert_eq!(clean("a\nb\n", "a\nX\n", "a\nX\n"), "a\nX\n");
        assert_eq!(clean("a\n", "a\n", "a\n"), "a\n");
    }

    #[test]
    fn only_local_changed_takes_local() {
        assert_eq!(clean("a\nb\n", "a\nlocal\n", "a\nb\n"), "a\nlocal\n");
    }

    #[test]
    fn only_remote_changed_takes_remote() {
        assert_eq!(clean("a\nb\n", "a\nb\n", "new\nb\n"), "new\nb\n");
    }

    #[test]
    fn non_overlapping_edits_are_both_applied() {
        let base = "one\ntwo\nthree\nfour\nfive\n";
        let local = "ONE\ntwo\nthree\nfour\nfive\n";
        let remote = "one\ntwo\nthree\nfour\nFIVE\n";
        assert_eq!(clean(base, local, remote), "ONE\ntwo\nthree\nfour\nFIVE\n");
    }

    #[test]
    fn same_change_on_both_sides_is_taken_once() {
        let base = "a\nold\nc\n";
        let both = "a\nnew\nc\n";
        assert_eq!(clean(base, both, both), both);
    }

    #[test]
    fn overlapping_different_edits_conflict() {
        let base = "a\nb\nc\n";
        let (text, hunks) = conflict(base, "a\nLOCAL\nc\n", "a\nREMOTE\nc\n");
        assert_eq!(hunks, 1);
        assert_eq!(
            text,
            "a\n<<<<<<< 本机\nLOCAL\n=======\nREMOTE\n>>>>>>> 远端\nc\n"
        );
    }

    #[test]
    fn conflict_keeps_clean_parts_applied() {
        let base = "top\nmid\nend\nfoot\n";
        let local = "TOP\nmid\nLOCAL\nfoot\n";
        let remote = "top\nmid\nREMOTE\nfoot\n";
        let (text, hunks) = conflict(base, local, remote);
        assert_eq!(hunks, 1);
        assert_eq!(
            text,
            "TOP\nmid\n<<<<<<< 本机\nLOCAL\n=======\nREMOTE\n>>>>>>> 远端\nfoot\n"
        );
    }

    #[test]
    fn identical_hunk_on_both_sides_next_to_other_edits() {
        let base = "a\nold\nc\nd\n";
        let local = "a\nnew\nc\nD\n";
        let remote = "a\nnew\nc\nd\n";
        assert_eq!(clean(base, local, remote), "a\nnew\nc\nD\n");
    }

    #[test]
    fn two_separate_conflicts_are_counted() {
        let base = "a\nb\nc\nd\ne\n";
        let local = "A\nb\nc\nD\ne\n";
        let remote = "x\nb\nc\ny\ne\n";
        let (text, hunks) = conflict(base, local, remote);
        assert_eq!(hunks, 2);
        assert_eq!(text.matches(MARKER_LOCAL).count(), 2);
        assert_eq!(text.matches(MARKER_SPLIT).count(), 2);
        assert_eq!(text.matches(MARKER_REMOTE).count(), 2);
    }

    #[test]
    fn adjacent_line_edits_do_not_conflict() {
        let base = "a\nb\nc\nd\n";
        let local = "a\nB\nc\nd\n";
        let remote = "a\nb\nC\nd\n";
        assert_eq!(clean(base, local, remote), "a\nB\nC\nd\n");
    }

    #[test]
    fn local_deletes_line_remote_edits_other_line() {
        let base = "a\nb\nc\nd\n";
        let local = "a\nc\nd\n";
        let remote = "a\nb\nc\nD\n";
        assert_eq!(clean(base, local, remote), "a\nc\nD\n");
    }

    #[test]
    fn both_delete_same_line_is_clean() {
        let base = "a\nb\nc\n";
        assert_eq!(clean(base, "a\nc\n", "a\nc\n"), "a\nc\n");
    }

    #[test]
    fn both_delete_different_duplicate_lines_is_clean() {
        let base = "x\nkeep\nx\n";
        let local = "keep\nx\n";
        let remote = "x\nkeep\n";
        assert_eq!(clean(base, local, remote), "keep\n");
    }

    #[test]
    fn delete_versus_edit_of_same_line_conflicts() {
        let base = "a\nb\nc\n";
        let (text, hunks) = conflict(base, "a\nc\n", "a\nB2\nc\n");
        assert_eq!(hunks, 1);
        assert!(text.contains("<<<<<<< 本机\n=======\nB2\n>>>>>>> 远端\n"));
    }

    #[test]
    fn insertions_at_same_point_conflict() {
        let base = "a\nc\n";
        let (text, hunks) = conflict(base, "a\nlocal\nc\n", "a\nremote\nc\n");
        assert_eq!(hunks, 1);
        assert!(text.contains("local"));
        assert!(text.contains("remote"));
    }

    #[test]
    fn insertions_at_different_points_are_clean() {
        let base = "a\nb\nc\n";
        assert_eq!(
            clean(base, "x\na\nb\nc\n", "a\nb\nc\ny\n"),
            "x\na\nb\nc\ny\n"
        );
    }

    #[test]
    fn append_only_both_sides_keeps_both_local_first() {
        let base = "# Day\n- morning\n";
        let local = "# Day\n- morning\n- local note\n";
        let remote = "# Day\n- morning\n- remote note\n";
        assert_eq!(
            clean(base, local, remote),
            "# Day\n- morning\n- local note\n- remote note\n"
        );
    }

    #[test]
    fn append_only_identical_additions_appear_once() {
        let base = "x\n";
        let both = "x\nsame\n";
        assert_eq!(clean(base, both, both), both);
    }

    #[test]
    fn append_only_skips_remote_lines_already_added_locally() {
        let base = "x\n";
        let local = "x\nshared\nlocal only\n";
        let remote = "x\nshared\nremote only\n";
        assert_eq!(
            clean(base, local, remote),
            "x\nshared\nlocal only\nremote only\n"
        );
    }

    #[test]
    fn append_only_with_repeated_blank_lines() {
        let base = "t\n";
        let local = "t\n\n- a\n\n";
        let remote = "t\n\n- b\n\n";
        let merged = clean(base, local, remote);
        assert!(merged.contains("- a"));
        assert!(merged.contains("- b"));
        assert!(merged.starts_with("t\n\n- a\n"));
    }

    #[test]
    fn empty_base_both_sides_add_keeps_both() {
        assert_eq!(clean("", "local\n", "remote\n"), "local\nremote\n");
    }

    #[test]
    fn empty_base_identical_content_is_clean() {
        assert_eq!(clean("", "same\n", "same\n"), "same\n");
    }

    #[test]
    fn empty_base_and_empty_remote_takes_local() {
        assert_eq!(clean("", "local\n", ""), "local\n");
    }

    #[test]
    fn all_empty_is_clean_empty() {
        assert_eq!(clean("", "", ""), "");
    }

    #[test]
    fn local_emptied_file_and_remote_appended() {
        let base = "a\nb\n";
        let merged = clean(base, "", "a\nb\nc\n");
        assert_eq!(merged, "c\n");
    }

    #[test]
    fn crlf_files_stay_crlf_when_clean() {
        let base = "a\r\nb\r\nc\r\n";
        let local = "A\r\nb\r\nc\r\n";
        let remote = "a\r\nb\r\nC\r\n";
        assert_eq!(clean(base, local, remote), "A\r\nb\r\nC\r\n");
    }

    #[test]
    fn crlf_conflict_uses_crlf_everywhere() {
        let base = "a\r\nb\r\n";
        let (text, hunks) = conflict(base, "a\r\nL\r\n", "a\r\nR\r\n");
        assert_eq!(hunks, 1);
        assert_eq!(
            text,
            "a\r\n<<<<<<< 本机\r\nL\r\n=======\r\nR\r\n>>>>>>> 远端\r\n"
        );
    }

    #[test]
    fn lf_files_stay_lf() {
        let base = "a\nb\n";
        let merged = clean(base, "a\nB\n", "a\nb\nc\n");
        assert!(!merged.contains('\r'));
        assert_eq!(merged, "a\nB\nc\n");
    }

    #[test]
    fn majority_crlf_decides_line_ending_for_mixed_input() {
        let base = "a\r\nb\r\nc\r\n";
        let local = "A\r\nb\r\nc\r\n";
        let remote = "a\nb\nC\n";
        let merged = clean(base, local, remote);
        assert_eq!(merged, "A\r\nb\r\nC\r\n");
    }

    #[test]
    fn missing_final_newline_is_preserved() {
        let base = "a\nb";
        let merged = clean(base, "a\nB", "a\nb");
        assert_eq!(merged, "a\nB");
    }

    #[test]
    fn local_appends_to_unterminated_last_line() {
        let base = "a\nlast";
        let merged = clean(base, "a\nlast\nnew", "a\nlast");
        assert_eq!(merged, "a\nlast\nnew");
    }

    #[test]
    fn append_to_unterminated_file_on_both_sides() {
        let base = "a";
        let merged = clean(base, "a\nlocal", "a\nremote");
        assert_eq!(merged, "a\nlocal\nremote");
    }

    #[test]
    fn final_newline_added_by_remote_is_kept() {
        let base = "a\nb";
        let merged = clean(base, "a\nB", "a\nb\n");
        assert_eq!(merged, "a\nB\n");
    }

    #[test]
    fn final_newline_removed_on_one_side_follows_that_side() {
        let base = "a\nb\n";
        let merged = clean(base, "a\nb", "A\nb\n");
        assert_eq!(merged, "A\nb");
    }

    #[test]
    fn single_newline_file_round_trips() {
        assert_eq!(clean("\n", "x\n", "\n"), "x\n");
        assert_eq!(clean("\n", "\n", "y\n"), "y\n");
    }

    #[test]
    fn blank_lines_are_ordinary_lines() {
        let base = "a\n\nb\n";
        let merged = clean(base, "a\n\n\nb\n", "a\n\nb\nc\n");
        assert_eq!(merged, "a\n\n\nb\nc\n");
    }

    #[test]
    fn unicode_and_tabs_are_compared_exactly() {
        let base = "\t项目\n中文 line\n";
        let local = "\t项目 A\n中文 line\n";
        let remote = "\t项目\n中文 line 2\n";
        assert_eq!(clean(base, local, remote), "\t项目 A\n中文 line 2\n");
    }

    #[test]
    fn full_rewrite_versus_append_merges_cleanly() {
        // Local replaced everything; remote only appended. The append does not touch the
        // replaced lines, so the three-way result is the rewrite plus the append.
        let base = "a\nb\n";
        assert_eq!(clean(base, "x\ny\n", "a\nb\nc\n"), "x\ny\nc\n");
    }

    #[test]
    fn full_rewrite_versus_edit_of_same_line_conflicts() {
        let base = "a\nb\n";
        let (text, hunks) = conflict(base, "x\ny\n", "a\nB\n");
        assert_eq!(hunks, 1);
        for needle in ["x", "y", "a", "B"] {
            assert!(
                text.lines().any(|l| l == needle),
                "lost line {needle}: {text}"
            );
        }
    }

    #[test]
    fn conflict_markers_are_on_their_own_lines() {
        let (text, _) = conflict("q\n", "l\n", "r\n");
        for line in text.lines() {
            if line.contains("<<<") || line.contains(">>>") {
                assert!(line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> "));
            }
        }
        assert_eq!(text.lines().count(), 5);
    }

    #[test]
    fn conflict_with_remote_deleting_block_and_local_editing_inside() {
        let base = "1\n2\n3\n4\n";
        let local = "1\n2X\n3\n4\n";
        let remote = "1\n4\n";
        let (text, hunks) = conflict(base, local, remote);
        assert_eq!(hunks, 1);
        assert!(text.contains("2X"));
    }

    #[test]
    fn remote_deletes_and_local_untouched_elsewhere_is_clean() {
        let base = "1\n2\n3\n";
        assert_eq!(clean(base, "1\n2\n3\n4\n", "1\n3\n"), "1\n3\n4\n");
    }

    #[test]
    fn property_style_no_inserted_line_is_dropped() {
        // Deterministic pseudo-random small edits: every line either side inserted must survive.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..300 {
            let n = (next() % 6) as usize + 1;
            let base_lines: Vec<String> = (0..n).map(|i| format!("base{case}-{i}")).collect();
            let base = join_with_nl(&base_lines);
            let local_lines = edit(&base_lines, "L", case, &mut next);
            let remote_lines = edit(&base_lines, "R", case, &mut next);
            let local = join_with_nl(&local_lines);
            let remote = join_with_nl(&remote_lines);
            let out = match three_way_merge(&base, &local, &remote) {
                MergeResult::Clean(t) => t,
                MergeResult::Conflict {
                    merged_with_markers,
                    ..
                } => merged_with_markers,
            };
            for line in local_lines.iter().chain(remote_lines.iter()) {
                if line.starts_with("L") || line.starts_with("R") {
                    assert!(
                        out.lines().any(|l| l == line),
                        "case {case}: lost {line:?} in\n{out}\nbase={base:?}\nlocal={local:?}\nremote={remote:?}"
                    );
                }
            }
        }
    }

    fn join_with_nl(lines: &[String]) -> String {
        let mut s = String::new();
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    /// Replaces a random base line with a new line tagged by `tag`, or appends one.
    fn edit(
        base: &[String],
        tag: &str,
        case: usize,
        next: &mut impl FnMut() -> u64,
    ) -> Vec<String> {
        let mut out: Vec<String> = base.to_vec();
        let choice = next() % 3;
        if choice == 0 || out.is_empty() {
            out.push(format!("{tag}{case}-new{}", next() % 1000));
        } else {
            let idx = (next() as usize) % out.len();
            out[idx] = format!("{tag}{case}-rep{}", next() % 1000);
        }
        out
    }

    #[test]
    fn large_file_five_megabytes_stays_fast() {
        let mut base = String::with_capacity(5_300_000);
        for i in 0..250_000 {
            base.push_str(&format!("line {i:06} text\n"));
        }
        let local = base.replacen("line 000100 text", "LOCAL 000100", 1);
        let remote = base.replacen("line 200000 text", "REMOTE 200000", 1);
        let started = Instant::now();
        let merged = clean(&base, &local, &remote);
        let elapsed = started.elapsed();
        assert!(merged.contains("LOCAL 000100"));
        assert!(merged.contains("REMOTE 200000"));
        assert_eq!(merged.lines().count(), 250_000);
        assert!(elapsed < Duration::from_secs(20), "merge took {elapsed:?}");
    }

    #[test]
    fn large_file_conflict_is_detected() {
        let mut base = String::new();
        for i in 0..50_000 {
            base.push_str(&format!("row {i}\n"));
        }
        let local = base.replacen("row 25000\n", "mine\n", 1);
        let remote = base.replacen("row 25000\n", "theirs\n", 1);
        let (text, hunks) = conflict(&base, &local, &remote);
        assert_eq!(hunks, 1);
        assert!(text.contains("mine") && text.contains("theirs"));
    }

    #[test]
    fn three_way_merge_is_deterministic() {
        let base = "a\nb\nc\nd\n";
        let local = "a\nX\nc\nD\n";
        let remote = "a\nY\nc\nd\n";
        let first = three_way_merge(base, local, remote);
        let second = three_way_merge(base, local, remote);
        assert_eq!(first, second);
    }
}
