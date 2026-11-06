mod common;

use common::{MIN_MS, T0, TempVault};
use jiyue_history::{EntryId, HistoryError, Kind, MAX_FILE_BYTES, SnapshotMeta};

#[test]
fn restore_returns_the_old_content() {
    let v = TempVault::new("restore_basic");
    let mut h = v.open();
    let old = h
        .snapshot("a.md", b"version one", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.snapshot("a.md", b"version two", SnapshotMeta::auto(), T0 + MIN_MS)
        .expect("s");
    let bytes = h
        .restore_bytes(&old, Some(b"version two"), T0 + 2 * MIN_MS)
        .expect("restore");
    assert_eq!(bytes, b"version one");
}

#[test]
fn restore_snapshots_current_as_before_restore_first() {
    let v = TempVault::new("restore_before");
    let mut h = v.open();
    let old = h
        .snapshot("a.md", b"old", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    // "current" was edited after the last autosave, so it has no snapshot yet.
    h.restore_bytes(&old, Some(b"current"), T0 + 2)
        .expect("restore");
    let list = h.list("a.md");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].kind, Kind::BeforeRestore);
    assert_eq!(h.read(&list[0].id).expect("read"), b"current");
}

#[test]
fn restore_when_current_equals_latest_adds_nothing() {
    let v = TempVault::new("restore_same");
    let mut h = v.open();
    let old = h
        .snapshot("a.md", b"old", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.snapshot("a.md", b"latest", SnapshotMeta::auto(), T0 + 1)
        .expect("s");
    let before = h.list("a.md").len();
    h.restore_bytes(&old, Some(b"latest"), T0 + 2)
        .expect("restore");
    // Current content equals the latest snapshot, so no BeforeRestore entry is added.
    assert_eq!(h.list("a.md").len(), before);
}

#[test]
fn restore_with_missing_current_records_restored_content() {
    let v = TempVault::new("restore_none");
    let mut h = v.open();
    let old = h
        .snapshot("a.md", b"back again", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    let bytes = h.restore_bytes(&old, None, T0 + 1).expect("restore");
    assert_eq!(bytes, b"back again");
    let list = h.list("a.md");
    assert_eq!(list.len(), 1, "identical to latest, so nothing new");
}

#[test]
fn restore_of_deleted_note_brings_it_back_into_normal_list() {
    let v = TempVault::new("restore_deleted");
    let mut h = v.open();
    h.snapshot("gone.md", b"last words", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_deleted("gone.md", b"last words", T0 + 1)
        .expect("del");
    let deleted = h.list_deleted();
    assert_eq!(deleted.len(), 1);
    let bytes = h
        .restore_bytes(&deleted[0].id, None, T0 + 2)
        .expect("restore");
    assert_eq!(bytes, b"last words");
    assert!(h.list_deleted().is_empty());
    assert_eq!(h.list("gone.md")[0].kind, Kind::Auto);
}

#[test]
fn restore_of_unknown_id_is_not_found() {
    let v = TempVault::new("restore_unknown");
    let mut h = v.open();
    let res = h.restore_bytes(&EntryId(12345), None, T0);
    assert!(matches!(res, Err(HistoryError::NotFound(_))));
}

#[test]
fn restore_refuses_when_current_is_too_large() {
    let v = TempVault::new("restore_toolarge");
    let mut h = v.open();
    let old = h
        .snapshot("a.md", b"small", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    let huge = vec![0u8; MAX_FILE_BYTES + 1];
    let res = h.restore_bytes(&old, Some(&huge), T0 + 1);
    assert!(matches!(res, Err(HistoryError::TooLarge { .. })));
    // Nothing was written, so the note is unchanged.
    assert_eq!(h.list("a.md").len(), 1);
}

#[test]
fn restore_does_not_change_the_restored_entry() {
    let v = TempVault::new("restore_untouched");
    let mut h = v.open();
    let old = h
        .snapshot("a.md", b"keep me", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.snapshot("a.md", b"other", SnapshotMeta::auto(), T0 + 1)
        .expect("s");
    h.restore_bytes(&old, Some(b"other"), T0 + 2)
        .expect("restore");
    assert_eq!(h.read(&old).expect("read"), b"keep me");
    assert_eq!(h.entry(&old).expect("entry").kind, Kind::Auto);
}

#[test]
fn restore_then_reopen_keeps_before_restore_entry() {
    let v = TempVault::new("restore_reopen");
    let before_id = {
        let mut h = v.open();
        let old = h
            .snapshot("a.md", b"old", SnapshotMeta::auto(), T0)
            .expect("s")
            .expect("id");
        h.restore_bytes(&old, Some(b"cur"), T0 + 2)
            .expect("restore");
        h.list("a.md")[0].id
    };
    let h = v.open();
    assert_eq!(h.entry(&before_id).expect("e").kind, Kind::BeforeRestore);
    assert_eq!(h.read(&before_id).expect("read"), b"cur");
}

#[test]
fn restore_of_a_renamed_note_uses_its_new_path() {
    let v = TempVault::new("restore_renamed");
    let mut h = v.open();
    let old = h
        .snapshot("old.md", b"body", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.note_renamed("old.md", "new.md").expect("rename");
    // "changed" exists only in the file (not snapshotted), so restoring preserves it first.
    h.restore_bytes(&old, Some(b"changed"), T0 + 2)
        .expect("restore");
    assert_eq!(h.list("new.md")[0].kind, Kind::BeforeRestore);
    assert!(h.list("old.md").is_empty());
}

#[test]
fn restore_chain_walks_back_through_history() {
    let v = TempVault::new("restore_chain");
    let mut h = v.open();
    let mut ids = Vec::new();
    for i in 0..5 {
        let body = format!("rev {i}");
        ids.push(
            h.snapshot("a.md", body.as_bytes(), SnapshotMeta::auto(), T0 + i)
                .expect("s")
                .expect("id"),
        );
    }
    let mut current = b"rev 4".to_vec();
    for (i, id) in ids.iter().enumerate().rev().skip(1) {
        let restored = h
            .restore_bytes(id, Some(&current), T0 + 100 + i as i64)
            .expect("restore");
        assert_eq!(restored, format!("rev {i}").into_bytes());
        current = restored;
    }
    assert_eq!(current, b"rev 0");
}
