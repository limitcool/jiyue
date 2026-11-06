mod common;

use common::{MIN_MS, T0, TempVault};
use jiyue_history::{EntryId, HistoryError, Kind, MAX_FILE_BYTES, SnapshotMeta};

#[test]
fn rename_moves_history_to_the_new_path() {
    let v = TempVault::new("rename_move");
    let mut h = v.open();
    h.snapshot("old.md", b"one", SnapshotMeta::auto(), T0)
        .expect("s");
    h.snapshot("old.md", b"two", SnapshotMeta::auto(), T0 + 1)
        .expect("s");
    h.note_renamed("old.md", "new.md").expect("rename");
    assert!(h.list("old.md").is_empty());
    assert_eq!(h.list("new.md").len(), 2);
    assert_eq!(h.list("new.md")[0].path, "new.md");
}

#[test]
fn rename_to_same_path_is_noop() {
    let v = TempVault::new("rename_same");
    let mut h = v.open();
    h.snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_renamed("a.md", "a.md").expect("rename");
    assert_eq!(h.list("a.md").len(), 1);
}

#[test]
fn rename_of_unknown_note_is_noop() {
    let v = TempVault::new("rename_unknown");
    let mut h = v.open();
    h.note_renamed("ghost.md", "other.md").expect("rename");
    assert!(h.list("other.md").is_empty());
    assert_eq!(h.stats().entries, 0);
}

#[test]
fn rename_survives_reopen() {
    let v = TempVault::new("rename_reopen");
    let id = {
        let mut h = v.open();
        let id = h
            .snapshot("old.md", b"x", SnapshotMeta::auto(), T0)
            .expect("s")
            .expect("id");
        h.note_renamed("old.md", "moved/new.md").expect("rename");
        id
    };
    let h = v.open();
    assert!(h.list("old.md").is_empty());
    assert_eq!(h.list("moved/new.md").len(), 1);
    assert_eq!(h.entry(&id).expect("e").path, "moved/new.md");
}

#[test]
fn rename_chain_follows_the_note() {
    let v = TempVault::new("rename_chain");
    let mut h = v.open();
    h.snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_renamed("a.md", "b.md").expect("r1");
    h.note_renamed("b.md", "c.md").expect("r2");
    assert_eq!(h.list("c.md").len(), 1);
    assert!(h.list("a.md").is_empty() && h.list("b.md").is_empty());
}

#[test]
fn rename_onto_existing_history_merges_in_time_order() {
    let v = TempVault::new("rename_merge");
    let mut h = v.open();
    h.snapshot("src.md", b"s1", SnapshotMeta::auto(), T0)
        .expect("s");
    h.snapshot("dst.md", b"d1", SnapshotMeta::auto(), T0 + 10)
        .expect("s");
    h.snapshot("src.md", b"s2", SnapshotMeta::auto(), T0 + 20)
        .expect("s");
    h.note_renamed("src.md", "dst.md").expect("rename");
    let times: Vec<i64> = h.list("dst.md").iter().map(|e| e.time_ms).collect();
    assert_eq!(times, vec![T0 + 20, T0 + 10, T0]);
}

#[test]
fn rename_then_snapshot_at_new_path_dedupes_against_moved_history() {
    let v = TempVault::new("rename_dedupe");
    let mut h = v.open();
    h.snapshot("a.md", b"same", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_renamed("a.md", "b.md").expect("rename");
    assert!(
        h.snapshot("b.md", b"same", SnapshotMeta::auto(), T0 + MIN_MS)
            .expect("s")
            .is_none()
    );
}

#[test]
fn delete_creates_a_deleted_entry_with_last_bytes() {
    let v = TempVault::new("delete_entry");
    let mut h = v.open();
    h.snapshot("a.md", b"final", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_deleted("a.md", b"final", T0 + 1).expect("del");
    let list = h.list("a.md");
    assert_eq!(list[0].kind, Kind::Deleted);
    assert_eq!(h.read(&list[0].id).expect("read"), b"final");
}

#[test]
fn delete_is_recorded_even_when_content_matches_latest() {
    let v = TempVault::new("delete_same");
    let mut h = v.open();
    h.snapshot("a.md", b"abc", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_deleted("a.md", b"abc", T0 + 1).expect("del");
    assert_eq!(h.list("a.md").len(), 2);
    assert_eq!(h.list("a.md")[0].kind, Kind::Deleted);
}

#[test]
fn deleting_twice_records_one_deletion() {
    let v = TempVault::new("delete_twice");
    let mut h = v.open();
    h.snapshot("a.md", b"abc", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_deleted("a.md", b"abc", T0 + 1).expect("del");
    h.note_deleted("a.md", b"abc", T0 + 2).expect("del again");
    assert_eq!(h.list("a.md").len(), 2);
}

#[test]
fn list_deleted_shows_only_notes_whose_last_entry_is_deletion() {
    let v = TempVault::new("list_deleted");
    let mut h = v.open();
    h.snapshot("alive.md", b"a", SnapshotMeta::auto(), T0)
        .expect("s");
    h.snapshot("dead.md", b"d", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_deleted("dead.md", b"d", T0 + 5).expect("del");
    let deleted = h.list_deleted();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].path, "dead.md");
    assert_eq!(deleted[0].time_ms, T0 + 5);
}

#[test]
fn list_deleted_is_newest_first() {
    let v = TempVault::new("list_deleted_order");
    let mut h = v.open();
    for (i, name) in ["a.md", "b.md", "c.md"].iter().enumerate() {
        h.snapshot(name, b"x", SnapshotMeta::auto(), T0).expect("s");
        h.note_deleted(name, b"x", T0 + 10 * i as i64).expect("del");
    }
    let paths: Vec<String> = h.list_deleted().into_iter().map(|d| d.path).collect();
    assert_eq!(paths, vec!["c.md", "b.md", "a.md"]);
}

#[test]
fn list_deleted_after_rename_of_deleted_note_follows_new_name() {
    let v = TempVault::new("list_deleted_rename");
    let mut h = v.open();
    h.snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_deleted("a.md", b"x", T0 + 1).expect("del");
    h.note_renamed("a.md", "trash/a.md").expect("rename");
    let deleted = h.list_deleted();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].path, "trash/a.md");
}

#[test]
fn deletion_of_a_note_with_no_history_still_records() {
    let v = TempVault::new("delete_fresh");
    let mut h = v.open();
    h.note_deleted("never-seen.md", b"content", T0)
        .expect("del");
    assert_eq!(h.list_deleted().len(), 1);
}

#[test]
fn oversized_last_bytes_on_delete_are_ignored_quietly() {
    let v = TempVault::new("delete_big");
    let mut h = v.open();
    let big = vec![b'z'; MAX_FILE_BYTES + 1];
    h.note_deleted("big.md", &big, T0).expect("del returns Ok");
    assert!(h.list_deleted().is_empty());
}

#[test]
fn deleted_notes_survive_reopen() {
    let v = TempVault::new("delete_reopen");
    {
        let mut h = v.open();
        h.snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
            .expect("s");
        h.note_deleted("a.md", b"x", T0 + 1).expect("del");
    }
    let h = v.open();
    assert_eq!(h.list_deleted().len(), 1);
}

#[test]
fn set_label_on_existing_entry_persists() {
    let v = TempVault::new("label_set");
    let id = {
        let mut h = v.open();
        let id = h
            .snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
            .expect("s")
            .expect("id");
        h.set_label(&id, Some("milestone".into())).expect("label");
        assert_eq!(h.entry(&id).expect("e").label.as_deref(), Some("milestone"));
        id
    };
    let h = v.open();
    assert_eq!(h.entry(&id).expect("e").label.as_deref(), Some("milestone"));
}

#[test]
fn clearing_a_label_removes_it() {
    let v = TempVault::new("label_clear");
    let mut h = v.open();
    let id = h
        .snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.set_label(&id, Some("tmp".into())).expect("set");
    h.set_label(&id, None).expect("clear");
    assert_eq!(h.entry(&id).expect("e").label, None);
    h.set_label(&id, Some("   ".into())).expect("blank");
    assert_eq!(h.entry(&id).expect("e").label, None);
}

#[test]
fn setting_the_same_label_twice_is_fine() {
    let v = TempVault::new("label_twice");
    let mut h = v.open();
    let id = h
        .snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.set_label(&id, Some("same".into())).expect("1");
    h.set_label(&id, Some("same".into())).expect("2");
    assert_eq!(h.entry(&id).expect("e").label.as_deref(), Some("same"));
}

#[test]
fn set_label_on_unknown_id_is_not_found() {
    let v = TempVault::new("label_unknown");
    let mut h = v.open();
    let res = h.set_label(&EntryId(77), Some("x".into()));
    assert!(matches!(res, Err(HistoryError::NotFound(_))));
}

#[test]
fn labelled_entry_is_still_listed_and_readable() {
    let v = TempVault::new("label_list");
    let mut h = v.open();
    let id = h
        .snapshot("a.md", b"content", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.set_label(&id, Some("v1".into())).expect("label");
    assert_eq!(h.list("a.md")[0].label.as_deref(), Some("v1"));
    assert_eq!(h.read(&id).expect("read"), b"content");
}

#[test]
fn label_on_identical_content_does_not_create_entry() {
    let v = TempVault::new("label_nodup");
    let mut h = v.open();
    let id = h
        .snapshot("a.md", b"same", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.snapshot(
        "a.md",
        b"same",
        SnapshotMeta::manual(Some("named".into())),
        T0 + 1,
    )
    .expect("s");
    assert_eq!(h.list("a.md").len(), 1);
    assert_eq!(h.entry(&id).expect("e").label.as_deref(), Some("named"));
}
