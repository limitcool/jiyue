mod common;

use std::fs;

use common::{T0, TempVault};
use jiyue_history::{HistoryError, Retention, SnapshotMeta};

#[test]
fn second_open_of_the_same_directory_fails_with_locked() {
    let v = TempVault::new("lock_second");
    let _first = v.open();
    let second = jiyue_history::History::open(&v.history_dir());
    assert!(matches!(second, Err(HistoryError::Locked)));
}

#[test]
fn lock_is_released_when_the_owner_is_dropped() {
    let v = TempVault::new("lock_release");
    {
        let _first = v.open();
    }
    let again = jiyue_history::History::open(&v.history_dir());
    assert!(again.is_ok());
}

#[test]
fn lock_blocks_writers_while_held_and_readers_see_consistent_data_after() {
    let v = TempVault::new("lock_writer");
    let mut owner = v.open();
    owner
        .snapshot("a.md", b"from owner", SnapshotMeta::auto(), T0)
        .expect("s");
    assert!(matches!(
        jiyue_history::History::open(&v.history_dir()),
        Err(HistoryError::Locked)
    ));
    owner
        .snapshot("a.md", b"second", SnapshotMeta::auto(), T0 + 1)
        .expect("s");
    drop(owner);
    let reader = v.open();
    assert_eq!(reader.list("a.md").len(), 2);
}

#[test]
fn two_vaults_have_independent_locks() {
    let a = TempVault::new("lock_a");
    let b = TempVault::new("lock_b");
    let _ha = a.open();
    let _hb = b.open();
}

#[test]
fn lock_file_is_created_next_to_the_index() {
    let v = TempVault::new("lock_file");
    let _h = v.open();
    assert!(v.history_dir().join("LOCK").is_file());
}

#[test]
fn many_sequential_open_close_cycles_are_stable() {
    let v = TempVault::new("cycles");
    for i in 0..25 {
        let mut h = v.open();
        h.snapshot(
            "cycle.md",
            format!("cycle {i}").as_bytes(),
            SnapshotMeta::auto(),
            T0 + i,
        )
        .expect("s");
        assert_eq!(h.list("cycle.md").len(), i as usize + 1);
    }
}

#[test]
fn compaction_preserves_entries_labels_and_paths() {
    let v = TempVault::new("compact_state");
    let mut h = v.open();
    let mut ids = Vec::new();
    for i in 0..30 {
        let id = h
            .snapshot(
                &format!("p{}.md", i % 3),
                format!("body {i}").as_bytes(),
                SnapshotMeta::auto(),
                T0 + i,
            )
            .expect("s")
            .expect("id");
        ids.push(id);
    }
    h.set_label(&ids[4], Some("four".into())).expect("label");
    h.note_renamed("p0.md", "moved.md").expect("rename");
    let before: Vec<_> = (0..3)
        .flat_map(|p| {
            let path = if p == 0 {
                "moved.md".to_string()
            } else {
                format!("p{p}.md")
            };
            h.list(&path)
        })
        .collect();
    h.compact().expect("compact");
    let after: Vec<_> = (0..3)
        .flat_map(|p| {
            let path = if p == 0 {
                "moved.md".to_string()
            } else {
                format!("p{p}.md")
            };
            h.list(&path)
        })
        .collect();
    assert_eq!(before, after);
    drop(h);
    let h = v.open();
    let reopened: Vec<_> = (0..3)
        .flat_map(|p| {
            let path = if p == 0 {
                "moved.md".to_string()
            } else {
                format!("p{p}.md")
            };
            h.list(&path)
        })
        .collect();
    assert_eq!(before, reopened);
    assert_eq!(h.entry(&ids[4]).expect("e").label.as_deref(), Some("four"));
}

#[test]
fn compaction_keeps_the_next_id_so_removed_ids_are_never_reused() {
    let v = TempVault::new("compact_ids");
    let mut h = v.open();
    let mut max_id = 0;
    for i in 0..10 {
        let id = h
            .snapshot(
                "x.md",
                format!("v{i}").as_bytes(),
                SnapshotMeta::auto(),
                T0 + i,
            )
            .expect("s")
            .expect("id");
        max_id = max_id.max(id.0);
    }
    let strict = Retention {
        keep_all_ms: 0,
        hourly_until_ms: 0,
        daily_until_ms: 0,
        weekly_after: false,
        ..Retention::default()
    };
    h.apply_retention(&strict, T0 + 1000).expect("gc");
    h.compact().expect("compact");
    drop(h);
    let mut h = v.open();
    let fresh = h
        .snapshot("x.md", b"brand new", SnapshotMeta::auto(), T0 + 2000)
        .expect("s")
        .expect("id");
    assert!(
        fresh.0 > max_id,
        "ids must not be reused after GC and compaction"
    );
}

#[test]
fn index_shrinks_after_compaction() {
    let v = TempVault::new("compact_shrink");
    let mut h = v.open();
    let id = h
        .snapshot("s.md", b"first", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    for i in 0..300 {
        h.set_label(&id, Some(format!("label {i}"))).expect("label");
    }
    let bloated = h.stats().index_bytes;
    h.compact().expect("compact");
    let slim = h.stats().index_bytes;
    assert!(slim < bloated / 10, "bloated {bloated} slim {slim}");
    assert_eq!(h.entry(&id).expect("e").label.as_deref(), Some("label 299"));
}

#[test]
fn automatic_compaction_on_open_when_log_is_bloated() {
    let v = TempVault::new("compact_open");
    {
        let mut h = v.open();
        let id = h
            .snapshot("s.md", b"one", SnapshotMeta::auto(), T0)
            .expect("s")
            .expect("id");
        for i in 0..1200 {
            h.set_label(&id, Some(format!("l{i}"))).expect("label");
        }
    }
    let size_before = fs::metadata(v.history_dir().join("index.jsonl"))
        .expect("meta")
        .len();
    let h = v.open();
    let size_after = fs::metadata(v.history_dir().join("index.jsonl"))
        .expect("meta")
        .len();
    assert!(
        size_after < size_before / 5,
        "before {size_before} after {size_after}"
    );
    assert_eq!(h.list("s.md").len(), 1);
}

#[test]
fn compaction_is_idempotent() {
    let v = TempVault::new("compact_twice");
    let mut h = v.open();
    h.snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
        .expect("s");
    h.compact().expect("1");
    let once = fs::read(v.history_dir().join("index.jsonl")).expect("read");
    h.compact().expect("2");
    let twice = fs::read(v.history_dir().join("index.jsonl")).expect("read");
    assert_eq!(once, twice);
}

#[test]
fn writes_after_compaction_are_persisted() {
    let v = TempVault::new("compact_then_write");
    {
        let mut h = v.open();
        h.snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
            .expect("s");
        h.compact().expect("compact");
        h.snapshot("a.md", b"y", SnapshotMeta::auto(), T0 + 1)
            .expect("s");
    }
    let h = v.open();
    assert_eq!(h.list("a.md").len(), 2);
}

#[test]
fn flush_on_an_empty_store_is_ok_and_idempotent() {
    let v = TempVault::new("flush_empty");
    let mut h = v.open();
    h.flush().expect("flush");
    h.flush().expect("flush again");
    assert_eq!(h.stats().entries, 0);
}

#[test]
fn flushed_writes_are_visible_to_a_later_open() {
    let v = TempVault::new("flush_visible");
    let mut h = v.open();
    let id = h
        .snapshot("f.md", b"durable soon", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    h.flush().expect("flush");
    drop(h);
    let h = v.open();
    assert_eq!(h.read(&id).expect("read"), b"durable soon");
}

#[test]
fn dropping_the_owner_flushes_pending_writes() {
    let v = TempVault::new("flush_on_drop");
    {
        let mut h = v.open();
        h.snapshot("d.md", b"pending at drop", SnapshotMeta::auto(), T0)
            .expect("s");
    }
    let h = v.open();
    assert_eq!(h.list("d.md").len(), 1);
    assert!(h.verify().is_ok());
}

#[test]
fn flush_after_a_blob_vanished_still_succeeds() {
    let v = TempVault::new("flush_vanished");
    let mut h = v.open();
    h.snapshot("v.md", b"short lived", SnapshotMeta::auto(), T0)
        .expect("s");
    let blob_dir = v.history_dir().join("blobs");
    for dir in std::fs::read_dir(&blob_dir).expect("blobs").flatten() {
        for file in std::fs::read_dir(dir.path()).expect("fanout").flatten() {
            std::fs::remove_file(file.path()).expect("remove");
        }
    }
    h.flush().expect("missing files are not an error for flush");
}

#[test]
fn stats_and_verify_on_an_empty_store() {
    let v = TempVault::new("empty_store");
    let h = v.open();
    let st = h.stats();
    assert_eq!(st.entries, 0);
    assert_eq!(st.notes, 0);
    assert_eq!(st.bytes_on_disk, 0);
    assert!(h.verify().is_ok());
    assert_eq!(h.verify().entries_checked, 0);
}

#[test]
fn verify_is_clean_after_normal_use() {
    let v = TempVault::new("verify_clean");
    let mut h = v.open();
    for i in 0..20 {
        h.snapshot(
            &format!("n{}.md", i % 4),
            format!("content {i}").as_bytes(),
            SnapshotMeta::auto(),
            T0 + i,
        )
        .expect("s");
    }
    let report = h.verify();
    assert!(report.is_ok());
    assert_eq!(report.entries_checked, 20);
    assert_eq!(report.orphan_blobs, 0);
    assert_eq!(report.stray_files, 0);
}
