mod common;

use common::{MIN_MS, T0, TempVault, text};
use jiyue_history::{EntryId, HistoryError, Kind, MAX_FILE_BYTES, SnapshotMeta, SnapshotOutcome};

#[test]
fn first_snapshot_creates_an_entry() {
    let v = TempVault::new("first");
    let mut h = v.open();
    let id = h
        .snapshot("a.md", b"hello", SnapshotMeta::auto(), T0)
        .expect("snap");
    assert!(id.is_some());
    let list = h.list("a.md");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].size, 5);
    assert_eq!(list[0].kind, Kind::Auto);
    assert_eq!(list[0].time_ms, T0);
}

#[test]
fn identical_content_is_deduplicated_against_latest() {
    let v = TempVault::new("dedupe");
    let mut h = v.open();
    assert!(
        h.snapshot("a.md", b"same", SnapshotMeta::auto(), T0)
            .expect("1")
            .is_some()
    );
    assert!(
        h.snapshot("a.md", b"same", SnapshotMeta::auto(), T0 + MIN_MS)
            .expect("2")
            .is_none()
    );
    assert_eq!(h.list("a.md").len(), 1);
}

#[test]
fn changed_content_after_identical_creates_new_entry() {
    let v = TempVault::new("changed");
    let mut h = v.open();
    h.snapshot("a.md", b"one", SnapshotMeta::auto(), T0)
        .expect("1");
    h.snapshot("a.md", b"two", SnapshotMeta::auto(), T0 + 1)
        .expect("2");
    h.snapshot("a.md", b"one", SnapshotMeta::auto(), T0 + 2)
        .expect("3");
    assert_eq!(h.list("a.md").len(), 3);
}

#[test]
fn list_is_newest_first_by_time() {
    let v = TempVault::new("order");
    let mut h = v.open();
    h.snapshot("a.md", b"v1", SnapshotMeta::auto(), T0)
        .expect("1");
    h.snapshot("a.md", b"v2", SnapshotMeta::auto(), T0 + 300)
        .expect("2");
    h.snapshot("a.md", b"v3", SnapshotMeta::auto(), T0 + 200)
        .expect("3");
    let times: Vec<i64> = h.list("a.md").iter().map(|e| e.time_ms).collect();
    assert_eq!(times, vec![T0 + 300, T0 + 200, T0]);
}

#[test]
fn equal_timestamps_order_by_id_descending() {
    let v = TempVault::new("tie");
    let mut h = v.open();
    let a = h
        .snapshot("a.md", b"x", SnapshotMeta::auto(), T0)
        .expect("1")
        .expect("id");
    let b = h
        .snapshot("a.md", b"y", SnapshotMeta::auto(), T0)
        .expect("2")
        .expect("id");
    let list = h.list("a.md");
    assert_eq!(list[0].id, b);
    assert_eq!(list[1].id, a);
}

#[test]
fn ids_are_unique_and_increasing() {
    let v = TempVault::new("ids");
    let mut h = v.open();
    let mut last = 0u64;
    for i in 0..20 {
        let id = h
            .snapshot(
                "a.md",
                format!("c{i}").as_bytes(),
                SnapshotMeta::auto(),
                T0 + i,
            )
            .expect("snap")
            .expect("created");
        assert!(id.0 > last);
        last = id.0;
    }
}

#[test]
fn same_content_in_two_notes_shares_one_blob() {
    let v = TempVault::new("shared");
    let mut h = v.open();
    h.snapshot("a.md", b"shared body", SnapshotMeta::auto(), T0)
        .expect("a");
    h.snapshot("b.md", b"shared body", SnapshotMeta::auto(), T0 + 1)
        .expect("b");
    let st = h.stats();
    assert_eq!(st.entries, 2);
    assert_eq!(st.unique_blobs, 1);
    assert_eq!(h.list("a.md")[0].hash, h.list("b.md")[0].hash);
}

#[test]
fn notes_are_independent() {
    let v = TempVault::new("indep");
    let mut h = v.open();
    h.snapshot("a.md", b"a1", SnapshotMeta::auto(), T0)
        .expect("a");
    h.snapshot("b.md", b"b1", SnapshotMeta::auto(), T0 + 1)
        .expect("b");
    assert_eq!(h.list("a.md").len(), 1);
    assert_eq!(h.list("b.md").len(), 1);
    assert!(h.list("c.md").is_empty());
}

#[test]
fn read_returns_exact_content() {
    let v = TempVault::new("read");
    let mut h = v.open();
    let body = text("doc", 40);
    let id = h
        .snapshot("a.md", &body, SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    assert_eq!(h.read(&id).expect("read"), body);
}

#[test]
fn empty_content_round_trips() {
    let v = TempVault::new("empty");
    let mut h = v.open();
    let id = h
        .snapshot("empty.md", b"", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    assert_eq!(h.read(&id).expect("read"), Vec::<u8>::new());
    assert_eq!(h.list("empty.md")[0].size, 0);
}

#[test]
fn non_utf8_binary_content_round_trips() {
    let v = TempVault::new("binary");
    let mut h = v.open();
    let bytes: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(31) % 256) as u8)
        .collect();
    let id = h
        .snapshot("img.bin", &bytes, SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    assert_eq!(h.read(&id).expect("read"), bytes);
}

#[test]
fn oversized_content_is_ignored_with_reason() {
    let v = TempVault::new("big");
    let mut h = v.open();
    let big = vec![b'x'; MAX_FILE_BYTES + 1];
    let outcome = h
        .snapshot_outcome("big.md", &big, SnapshotMeta::auto(), T0)
        .expect("outcome");
    assert_eq!(
        outcome,
        SnapshotOutcome::TooLarge {
            size: (MAX_FILE_BYTES + 1) as u64
        }
    );
    assert!(
        h.snapshot("big.md", &big, SnapshotMeta::auto(), T0)
            .expect("s")
            .is_none()
    );
    assert!(h.list("big.md").is_empty());
}

#[test]
fn content_exactly_at_the_limit_is_accepted() {
    let v = TempVault::new("limit");
    let mut h = v.open();
    let edge = vec![b'y'; MAX_FILE_BYTES];
    let outcome = h
        .snapshot_outcome("edge.md", &edge, SnapshotMeta::auto(), T0)
        .expect("outcome");
    assert!(matches!(outcome, SnapshotOutcome::Created(_)));
}

#[test]
fn snapshot_outcome_reports_unchanged() {
    let v = TempVault::new("unchanged");
    let mut h = v.open();
    h.snapshot("a.md", b"q", SnapshotMeta::auto(), T0)
        .expect("1");
    let outcome = h
        .snapshot_outcome("a.md", b"q", SnapshotMeta::auto(), T0 + 1)
        .expect("2");
    assert_eq!(outcome, SnapshotOutcome::Unchanged);
}

#[test]
fn manual_snapshot_with_label_is_stored() {
    let v = TempVault::new("manual");
    let mut h = v.open();
    let id = h
        .snapshot(
            "a.md",
            b"m",
            SnapshotMeta::manual(Some("before refactor".into())),
            T0,
        )
        .expect("s")
        .expect("id");
    let e = h.entry(&id).expect("entry");
    assert_eq!(e.kind, Kind::Manual);
    assert_eq!(e.label.as_deref(), Some("before refactor"));
}

#[test]
fn label_is_trimmed_and_blank_label_is_none() {
    let v = TempVault::new("labeltrim");
    let mut h = v.open();
    let id = h
        .snapshot(
            "a.md",
            b"m",
            SnapshotMeta::manual(Some("  spaced  ".into())),
            T0,
        )
        .expect("s")
        .expect("id");
    assert_eq!(h.entry(&id).expect("e").label.as_deref(), Some("spaced"));
    let id2 = h
        .snapshot(
            "a.md",
            b"n",
            SnapshotMeta::manual(Some("   ".into())),
            T0 + 1,
        )
        .expect("s")
        .expect("id");
    assert_eq!(h.entry(&id2).expect("e").label, None);
}

#[test]
fn very_long_label_is_capped() {
    let v = TempVault::new("longlabel");
    let mut h = v.open();
    let long = "é".repeat(1000);
    let id = h
        .snapshot("a.md", b"m", SnapshotMeta::manual(Some(long)), T0)
        .expect("s")
        .expect("id");
    let label = h.entry(&id).expect("e").label.expect("label");
    assert_eq!(label.chars().count(), 256);
}

#[test]
fn identical_manual_snapshot_attaches_label_to_latest() {
    let v = TempVault::new("relabel");
    let mut h = v.open();
    let first = h
        .snapshot("a.md", b"same", SnapshotMeta::auto(), T0)
        .expect("1")
        .expect("id");
    let outcome = h
        .snapshot_outcome(
            "a.md",
            b"same",
            SnapshotMeta::manual(Some("keep".into())),
            T0 + 1,
        )
        .expect("2");
    assert_eq!(outcome, SnapshotOutcome::Unchanged);
    assert_eq!(h.entry(&first).expect("e").label.as_deref(), Some("keep"));
}

#[test]
fn before_sync_kind_is_recorded() {
    let v = TempVault::new("beforesync");
    let mut h = v.open();
    let id = h
        .snapshot("a.md", b"s", SnapshotMeta::before_sync(), T0)
        .expect("s")
        .expect("id");
    assert_eq!(h.entry(&id).expect("e").kind, Kind::BeforeSync);
}

#[test]
fn path_is_normalized_to_forward_slashes() {
    let v = TempVault::new("norm");
    let mut h = v.open();
    h.snapshot("dir\\sub\\n.md", b"x", SnapshotMeta::auto(), T0)
        .expect("s");
    assert_eq!(h.list("dir/sub/n.md").len(), 1);
    assert_eq!(h.list("/dir//sub/./n.md").len(), 1);
}

#[test]
fn invalid_paths_are_rejected_on_write() {
    let v = TempVault::new("badpath");
    let mut h = v.open();
    for bad in ["", "/", "../escape.md", "a/../../b.md", "x\0y.md"] {
        let res = h.snapshot(bad, b"x", SnapshotMeta::auto(), T0);
        assert!(
            matches!(res, Err(HistoryError::InvalidPath(_))),
            "path {bad:?}"
        );
    }
    assert_eq!(h.stats().entries, 0);
}

#[test]
fn list_of_invalid_path_is_empty_not_error() {
    let v = TempVault::new("badlist");
    let h = v.open();
    assert!(h.list("../x").is_empty());
}

#[test]
fn latest_time_tracks_newest_entry() {
    let v = TempVault::new("latesttime");
    let mut h = v.open();
    assert_eq!(h.latest_time("a.md"), None);
    h.snapshot("a.md", b"1", SnapshotMeta::auto(), T0)
        .expect("1");
    h.snapshot("a.md", b"2", SnapshotMeta::auto(), T0 + 5 * MIN_MS)
        .expect("2");
    assert_eq!(h.latest_time("a.md"), Some(T0 + 5 * MIN_MS));
}

#[test]
fn entry_lookup_by_id_and_unknown_id() {
    let v = TempVault::new("entrylookup");
    let mut h = v.open();
    let id = h
        .snapshot("a.md", b"e", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    assert_eq!(h.entry(&id).expect("e").path, "a.md");
    assert!(h.entry(&EntryId(999_999)).is_none());
}

#[test]
fn read_of_unknown_id_is_not_found() {
    let v = TempVault::new("unknownread");
    let h = v.open();
    assert!(matches!(
        h.read(&EntryId(42)),
        Err(HistoryError::NotFound(_))
    ));
}

#[test]
fn entry_id_parses_and_prints() {
    let id: EntryId = "17".parse().expect("parse");
    assert_eq!(id, EntryId(17));
    assert_eq!(id.to_string(), "17");
    assert!("x".parse::<EntryId>().is_err());
}

#[test]
fn stats_count_entries_notes_and_blobs() {
    let v = TempVault::new("stats");
    let mut h = v.open();
    h.snapshot("a.md", b"aaa", SnapshotMeta::auto(), T0)
        .expect("1");
    h.snapshot("a.md", b"bbb", SnapshotMeta::auto(), T0 + 1)
        .expect("2");
    h.snapshot("b.md", b"bbb", SnapshotMeta::auto(), T0 + 2)
        .expect("3");
    let st = h.stats();
    assert_eq!(st.entries, 3);
    assert_eq!(st.notes, 2);
    assert_eq!(st.unique_blobs, 2);
    assert_eq!(st.logical_bytes, 6);
    assert!(st.bytes_on_disk > 0);
    assert!(st.index_bytes > 0);
}

#[test]
fn history_survives_reopen() {
    let v = TempVault::new("reopen");
    let id = {
        let mut h = v.open();
        h.snapshot("a.md", b"persist me", SnapshotMeta::auto(), T0)
            .expect("s")
            .expect("id")
    };
    let h = v.open();
    assert_eq!(h.read(&id).expect("read"), b"persist me");
    assert_eq!(h.list("a.md").len(), 1);
}

#[test]
fn ids_are_not_reused_after_reopen() {
    let v = TempVault::new("noreuse");
    let first = {
        let mut h = v.open();
        h.snapshot("a.md", b"1", SnapshotMeta::auto(), T0)
            .expect("s")
            .expect("id")
    };
    let mut h = v.open();
    let second = h
        .snapshot("a.md", b"2", SnapshotMeta::auto(), T0 + 1)
        .expect("s")
        .expect("id");
    assert!(second.0 > first.0);
}

#[test]
fn snapshots_of_many_notes_list_separately() {
    let v = TempVault::new("many");
    let mut h = v.open();
    for n in 0..30 {
        let path = format!("notes/n{n}.md");
        for k in 0..3 {
            h.snapshot(
                &path,
                format!("{n}-{k}").as_bytes(),
                SnapshotMeta::auto(),
                T0 + k,
            )
            .expect("s");
        }
    }
    for n in 0..30 {
        assert_eq!(h.list(&format!("notes/n{n}.md")).len(), 3);
    }
    assert_eq!(h.stats().notes, 30);
}
