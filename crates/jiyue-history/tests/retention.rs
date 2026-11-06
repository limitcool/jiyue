mod common;

use std::collections::HashSet;

use common::{T0, TempVault, days, hours};
use jiyue_history::{DAY_MS, Kind, Retention, SnapshotMeta};

fn body(i: usize, size: usize) -> Vec<u8> {
    let mut s = format!("tick {i}\n");
    while s.len() < size {
        s.push_str("filler text that does not change between revisions\n");
    }
    s.truncate(size);
    s.into_bytes()
}

fn no_cap() -> Retention {
    Retention {
        max_total_bytes: u64::MAX,
        ..Retention::default()
    }
}

#[test]
fn nothing_is_removed_inside_the_keep_all_window() {
    let v = TempVault::new("ret_keepall");
    let mut h = v.open();
    for i in 0..12 {
        h.snapshot(
            "a.md",
            &body(i, 300),
            SnapshotMeta::auto(),
            T0 + hours(i as i64),
        )
        .expect("s");
    }
    let report = h
        .apply_retention(&Retention::default(), T0 + hours(12))
        .expect("gc");
    assert_eq!(report.removed_entries, 0);
    assert_eq!(report.kept_entries, 12);
}

#[test]
fn four_simulated_months_thin_out_as_designed() {
    let v = TempVault::new("ret_months");
    let mut h = v.open();
    let total_hours = 120 * 24;
    // One note, one snapshot every 4 hours for 120 days.
    let mut i = 0usize;
    let mut t = 0i64;
    while t <= total_hours {
        h.snapshot(
            "journal.md",
            &body(i, 400),
            SnapshotMeta::auto(),
            T0 + hours(t),
        )
        .expect("s");
        i += 1;
        t += 4;
    }
    let now = T0 + hours(total_hours);
    let before = h.list("journal.md").len();
    assert_eq!(before, i);

    let report = h
        .apply_retention(
            &Retention {
                weekly_after: true,
                ..no_cap()
            },
            now,
        )
        .expect("gc");
    assert!(report.removed_entries > 0);
    let kept = h.list("journal.md");
    assert_eq!(kept.len(), before - report.removed_entries);

    // Newest entry always survives.
    assert_eq!(kept[0].time_ms, T0 + hours(total_hours));

    // Last day: everything kept (ages 0, 4, ..., 24 hours: seven snapshots).
    let last_day = kept.iter().filter(|e| now - e.time_ms <= DAY_MS).count();
    assert_eq!(last_day, 7);

    // Between 1 and 7 days: at most one entry per clock hour, and with 4-hour cadence that is all of them.
    let week: Vec<i64> = kept
        .iter()
        .map(|e| e.time_ms)
        .filter(|t| {
            let age = now - t;
            age > DAY_MS && age <= days(7)
        })
        .collect();
    let week_hours: HashSet<i64> = week.iter().map(|t| t.div_euclid(3_600_000)).collect();
    assert_eq!(week.len(), week_hours.len(), "one per hour bucket");

    // Between 7 and 90 days: at most one entry per day.
    let mid: Vec<i64> = kept
        .iter()
        .map(|e| e.time_ms)
        .filter(|t| {
            let age = now - t;
            age > days(7) && age <= days(90)
        })
        .collect();
    let mid_days: HashSet<i64> = mid.iter().map(|t| t.div_euclid(DAY_MS)).collect();
    assert_eq!(mid.len(), mid_days.len(), "one per day bucket");
    assert!(
        mid.len() >= 70 && mid.len() <= 84,
        "about one per day, got {}",
        mid.len()
    );

    // Older than 90 days: about one per week, and never more than the number of weeks in that range.
    let old: Vec<i64> = kept
        .iter()
        .map(|e| e.time_ms)
        .filter(|t| now - t > days(90))
        .collect();
    assert!(old.len() <= 6, "weekly buckets only, got {}", old.len());
    assert!(!old.is_empty());
}

#[test]
fn without_weekly_retention_nothing_older_than_ninety_days_survives_except_newest_per_note() {
    let v = TempVault::new("ret_noweekly");
    let mut h = v.open();
    for k in 0..200i64 {
        h.snapshot(
            "old.md",
            &body(k as usize, 200),
            SnapshotMeta::auto(),
            T0 + days(k),
        )
        .expect("s");
    }
    let policy = Retention {
        weekly_after: false,
        ..no_cap()
    };
    let now = T0 + days(200) + days(365);
    h.apply_retention(&policy, now).expect("gc");
    let kept = h.list("old.md");
    assert_eq!(
        kept.len(),
        1,
        "only the newest of a note survives when everything is old"
    );
}

#[test]
fn manual_snapshots_survive_years_of_retention() {
    let v = TempVault::new("ret_manual");
    let mut h = v.open();
    let manual = h
        .snapshot(
            "keep.md",
            b"manual version",
            SnapshotMeta::manual(Some("1.0".into())),
            T0,
        )
        .expect("s")
        .expect("id");
    for k in 0..30i64 {
        h.snapshot(
            "keep.md",
            &body(k as usize, 100),
            SnapshotMeta::auto(),
            T0 + days(k),
        )
        .expect("s");
    }
    h.apply_retention(&Retention::default(), T0 + days(365 * 3))
        .expect("gc");
    assert!(h.entry(&manual).is_some(), "manual entry must survive");
    assert_eq!(h.read(&manual).expect("read"), b"manual version");
}

#[test]
fn manual_entries_are_dropped_when_policy_allows() {
    let v = TempVault::new("ret_manual_drop");
    let mut h = v.open();
    let manual = h
        .snapshot("keep.md", b"manual version", SnapshotMeta::manual(None), T0)
        .expect("s")
        .expect("id");
    h.snapshot("keep.md", b"newer", SnapshotMeta::auto(), T0 + days(400))
        .expect("s");
    let policy = Retention {
        keep_manual_forever: false,
        weekly_after: false,
        ..no_cap()
    };
    h.apply_retention(&policy, T0 + days(401)).expect("gc");
    assert!(h.entry(&manual).is_none());
}

#[test]
fn size_cap_is_enforced_on_unique_content() {
    let v = TempVault::new("ret_cap");
    let mut h = v.open();
    for i in 0..50 {
        h.snapshot(
            "big.md",
            &body(i, 20_000),
            SnapshotMeta::auto(),
            T0 + i as i64,
        )
        .expect("s");
    }
    let policy = Retention {
        max_total_bytes: 300_000,
        ..Retention::default()
    };
    let report = h.apply_retention(&policy, T0 + 50).expect("gc");
    assert!(report.removed_entries > 0);
    let st = h.stats();
    assert!(st.logical_bytes <= 300_000, "logical {}", st.logical_bytes);
    // Oldest go first: the newest survivors are the most recent snapshots.
    let kept = h.list("big.md");
    assert_eq!(kept[0].time_ms, T0 + 49);
    let oldest_kept = kept.last().expect("some").time_ms;
    assert!(oldest_kept > T0, "oldest entries were dropped first");
}

#[test]
fn size_cap_keeps_newest_entry_per_note_even_when_it_alone_exceeds_cap() {
    let v = TempVault::new("ret_cap_tiny");
    let mut h = v.open();
    h.snapshot("a.md", &body(1, 5000), SnapshotMeta::auto(), T0)
        .expect("s");
    h.snapshot("b.md", &body(2, 5000), SnapshotMeta::auto(), T0 + 1)
        .expect("s");
    let policy = Retention {
        max_total_bytes: 100,
        ..Retention::default()
    };
    h.apply_retention(&policy, T0 + 2).expect("gc");
    assert_eq!(h.list("a.md").len(), 1);
    assert_eq!(h.list("b.md").len(), 1);
}

#[test]
fn gc_deletes_blob_files_no_longer_referenced() {
    let v = TempVault::new("ret_blobs");
    let mut h = v.open();
    for i in 0..10 {
        h.snapshot("a.md", &body(i, 4000), SnapshotMeta::auto(), T0 + i as i64)
            .expect("s");
    }
    let before = h.stats();
    let policy = Retention {
        max_total_bytes: 10_000,
        ..Retention::default()
    };
    let report = h.apply_retention(&policy, T0 + 10).expect("gc");
    assert!(report.removed_blobs > 0);
    assert!(report.freed_bytes > 0);
    let after = h.stats();
    assert!(after.bytes_on_disk < before.bytes_on_disk);
    assert_eq!(h.verify().orphan_blobs, 0);
    assert!(h.verify().is_ok());
}

#[test]
fn shared_blob_is_kept_while_any_entry_references_it() {
    let v = TempVault::new("ret_shared");
    let mut h = v.open();
    h.snapshot("a.md", b"shared", SnapshotMeta::auto(), T0)
        .expect("s");
    h.snapshot("a.md", b"newer a", SnapshotMeta::auto(), T0 + 2)
        .expect("s");
    h.snapshot("b.md", b"shared", SnapshotMeta::auto(), T0 + 1)
        .expect("s");
    // Strict policy: only the newest entry of each note survives. The dropped a.md entry
    // held "shared", but b.md's newest entry still references that blob, so it must stay.
    let policy = Retention {
        keep_all_ms: 0,
        hourly_until_ms: 0,
        daily_until_ms: 0,
        weekly_after: false,
        ..no_cap()
    };
    let report = h.apply_retention(&policy, T0 + 100).expect("gc");
    assert_eq!(report.removed_entries, 1);
    assert_eq!(report.removed_blobs, 0);
    assert!(h.verify().is_ok());
    let b = h.list("b.md");
    assert_eq!(h.read(&b[0].id).expect("read"), b"shared");
}

#[test]
fn retention_with_nothing_to_do_reports_zero() {
    let v = TempVault::new("ret_noop");
    let mut h = v.open();
    h.snapshot("a.md", b"only", SnapshotMeta::auto(), T0)
        .expect("s");
    let report = h
        .apply_retention(&Retention::default(), T0 + 1)
        .expect("gc");
    assert_eq!(report.removed_entries, 0);
    assert_eq!(report.removed_blobs, 0);
    assert_eq!(report.freed_bytes, 0);
}

#[test]
fn retention_result_survives_reopen() {
    let v = TempVault::new("ret_reopen");
    let (kept_before, kept_ids) = {
        let mut h = v.open();
        for k in 0..60i64 {
            h.snapshot(
                "a.md",
                &body(k as usize, 300),
                SnapshotMeta::auto(),
                T0 + hours(k * 3),
            )
            .expect("s");
        }
        h.apply_retention(
            &Retention {
                weekly_after: true,
                ..no_cap()
            },
            T0 + days(30),
        )
        .expect("gc");
        let ids: Vec<_> = h.list("a.md").iter().map(|e| e.id).collect();
        (ids.len(), ids)
    };
    let h = v.open();
    let ids: Vec<_> = h.list("a.md").iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), kept_before);
    assert_eq!(ids, kept_ids);
}

#[test]
fn retention_never_removes_the_only_snapshot_of_any_note() {
    let v = TempVault::new("ret_only");
    let mut h = v.open();
    for n in 0..10 {
        h.snapshot(&format!("n{n}.md"), b"x", SnapshotMeta::auto(), T0 + n)
            .expect("s");
    }
    let policy = Retention {
        keep_all_ms: 0,
        hourly_until_ms: 0,
        daily_until_ms: 0,
        weekly_after: false,
        max_total_bytes: 0,
        keep_manual_forever: false,
    };
    h.apply_retention(&policy, T0 + days(10_000)).expect("gc");
    for n in 0..10 {
        assert_eq!(h.list(&format!("n{n}.md")).len(), 1);
    }
}

#[test]
fn deleted_note_keeps_its_final_entry_under_retention() {
    let v = TempVault::new("ret_deleted");
    let mut h = v.open();
    h.snapshot("gone.md", b"last", SnapshotMeta::auto(), T0)
        .expect("s");
    h.note_deleted("gone.md", b"last", T0 + 1).expect("del");
    let policy = Retention {
        keep_all_ms: 0,
        hourly_until_ms: 0,
        daily_until_ms: 0,
        weekly_after: false,
        ..no_cap()
    };
    h.apply_retention(&policy, T0 + days(900)).expect("gc");
    assert_eq!(h.list_deleted().len(), 1);
    assert_eq!(h.list("gone.md")[0].kind, Kind::Deleted);
}

#[test]
fn compaction_after_heavy_retention_keeps_state() {
    let v = TempVault::new("ret_compact");
    let mut h = v.open();
    for k in 0..1200i64 {
        h.snapshot(
            "hot.md",
            &body(k as usize, 64),
            SnapshotMeta::auto(),
            T0 + k,
        )
        .expect("s");
    }
    let before_bytes = h.stats().index_bytes;
    let policy = Retention {
        keep_all_ms: 0,
        hourly_until_ms: 0,
        daily_until_ms: 0,
        weekly_after: false,
        ..no_cap()
    };
    let report = h.apply_retention(&policy, T0 + days(1000)).expect("gc");
    assert!(report.removed_entries >= 1190);
    assert!(h.stats().index_bytes < before_bytes, "index compacted");
    drop(h);
    let h = v.open();
    assert_eq!(h.list("hot.md").len(), 1);
}
