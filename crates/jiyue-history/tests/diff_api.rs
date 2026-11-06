mod common;

use common::{T0, TempVault};
use jiyue_history::{DiffTag, HistoryError, SnapshotMeta, diff_text};

#[test]
fn history_diff_between_two_entries() {
    let v = TempVault::new("diff_api");
    let mut h = v.open();
    let a = h
        .snapshot("n.md", "alpha\nbeta\n".as_bytes(), SnapshotMeta::auto(), T0)
        .expect("a")
        .expect("id");
    let b = h
        .snapshot(
            "n.md",
            "alpha\ngamma\n".as_bytes(),
            SnapshotMeta::auto(),
            T0 + 1,
        )
        .expect("b")
        .expect("id");
    let d = h.diff(&a, &b).expect("diff");
    assert!(
        d.iter()
            .any(|l| l.tag == DiffTag::Delete && l.text == "beta")
    );
    assert!(
        d.iter()
            .any(|l| l.tag == DiffTag::Insert && l.text == "gamma")
    );
    assert_eq!(d.iter().filter(|l| l.tag == DiffTag::Equal).count(), 1);
}

#[test]
fn history_diff_of_an_entry_with_itself_has_no_changes() {
    let v = TempVault::new("diff_self");
    let mut h = v.open();
    let a = h
        .snapshot("n.md", b"x\ny\n", SnapshotMeta::auto(), T0)
        .expect("a")
        .expect("id");
    let d = h.diff(&a, &a).expect("diff");
    assert!(d.iter().all(|l| l.tag == DiffTag::Equal));
}

#[test]
fn history_diff_rejects_binary_content() {
    let v = TempVault::new("diff_binary");
    let mut h = v.open();
    let bin = h
        .snapshot("b.bin", &[0xff, 0xfe, 0x00], SnapshotMeta::auto(), T0)
        .expect("a")
        .expect("id");
    let txt = h
        .snapshot("t.md", b"text", SnapshotMeta::auto(), T0)
        .expect("b")
        .expect("id");
    assert!(matches!(h.diff(&bin, &txt), Err(HistoryError::NotText)));
}

#[test]
fn history_diff_of_cjk_notes() {
    let v = TempVault::new("diff_cjk");
    let mut h = v.open();
    let a = h
        .snapshot(
            "c.md",
            "# 标题\n今天天气很好\n".as_bytes(),
            SnapshotMeta::auto(),
            T0,
        )
        .expect("a")
        .expect("id");
    let b = h
        .snapshot(
            "c.md",
            "# 标题\n今天天气不错\n".as_bytes(),
            SnapshotMeta::auto(),
            T0 + 1,
        )
        .expect("b")
        .expect("id");
    let d = h.diff(&a, &b).expect("diff");
    let del = d.iter().find(|l| l.tag == DiffTag::Delete).expect("delete");
    let hl: Vec<&str> = del
        .highlights
        .iter()
        .map(|r| &del.text[r.clone()])
        .collect();
    assert_eq!(hl, vec!["很好"]);
}

#[test]
fn diff_text_free_function_matches_history_diff() {
    let d = diff_text("one two\n", "one three\n");
    let del = d.iter().find(|l| l.tag == DiffTag::Delete).expect("delete");
    assert_eq!(del.highlights.len(), 1);
    assert_eq!(&del.text[del.highlights[0].clone()], "two");
}

#[test]
fn diff_line_numbers_are_one_based_and_consistent() {
    let d = diff_text("a\nb\n", "a\nb\nc\n");
    assert_eq!(d[0].old_line, Some(1));
    assert_eq!(d[0].new_line, Some(1));
    let ins = d.last().expect("last");
    assert_eq!(ins.tag, DiffTag::Insert);
    assert_eq!(ins.new_line, Some(3));
    assert_eq!(ins.old_line, None);
}

#[test]
fn diff_handles_large_documents() {
    let old: String = (0..2000).map(|i| format!("paragraph {i} text\n")).collect();
    let new: String = (0..2000)
        .map(|i| {
            if i == 1000 {
                "changed\n".to_string()
            } else {
                format!("paragraph {i} text\n")
            }
        })
        .collect();
    let d = diff_text(&old, &new);
    assert_eq!(d.iter().filter(|l| l.tag == DiffTag::Delete).count(), 1);
    assert_eq!(d.iter().filter(|l| l.tag == DiffTag::Insert).count(), 1);
    assert_eq!(d.len(), 2001);
}
