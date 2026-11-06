mod common;

use std::time::Instant;

use common::{T0, TempVault};
use jiyue_history::{Retention, SnapshotMeta};

/// About 20 KB of prose-like text with a 16-byte revision marker at the start.
fn note_body(rev: u64) -> Vec<u8> {
    let mut body = format!("{rev:016}\n").into_bytes();
    let mut i = 0usize;
    while body.len() < 20 * 1024 {
        body.extend_from_slice(
            format!("line {i}: the quick brown fox jumps over the lazy dog\n").as_bytes(),
        );
        i += 1;
    }
    body.truncate(20 * 1024);
    body
}

#[test]
fn five_thousand_snapshots_of_a_twenty_kilobyte_note() {
    let v = TempVault::new("perf_5000");
    let mut h = v.open();

    let start = Instant::now();
    for rev in 0..5000u64 {
        let body = note_body(rev);
        let id = h
            .snapshot(
                "perf/big.md",
                &body,
                SnapshotMeta::auto(),
                T0 + rev as i64 * 1000,
            )
            .expect("snapshot");
        assert!(id.is_some());
    }
    let write_elapsed = start.elapsed();

    let start = Instant::now();
    h.flush().expect("flush");
    let flush_elapsed = start.elapsed();

    let start = Instant::now();
    let list = h.list("perf/big.md");
    let list_elapsed = start.elapsed();
    assert_eq!(list.len(), 5000);

    let start = Instant::now();
    let mut checked = 0usize;
    for e in list.iter().step_by(50) {
        let bytes = h.read(&e.id).expect("read");
        assert_eq!(bytes.len(), 20 * 1024);
        checked += 1;
    }
    let read_elapsed = start.elapsed() / checked.max(1) as u32;

    let start = Instant::now();
    let stats = h.stats();
    let stats_elapsed = start.elapsed();

    let start = Instant::now();
    let report = h.verify();
    let verify_elapsed = start.elapsed();
    assert!(report.is_ok());

    let start = Instant::now();
    let gc = h
        .apply_retention(&Retention::default(), T0 + 5000 * 1000 + 10)
        .expect("gc");
    let gc_elapsed = start.elapsed();

    let per_snapshot = write_elapsed / 5000;
    println!(
        "perf: 5000 snapshots of 20 KB in {:.2?} ({:.3?} per snapshot, {:.0} snapshots/s)",
        write_elapsed,
        per_snapshot,
        5000.0 / write_elapsed.as_secs_f64()
    );
    println!(
        "perf: flush (fsync of 5000 new blobs + index) in {:.2?}",
        flush_elapsed
    );
    println!("perf: list of 5000 entries in {:.2?}", list_elapsed);
    println!("perf: read (avg of {checked}) {:.3?}", read_elapsed);
    println!(
        "perf: stats {:.2?}, verify {:.2?}",
        stats_elapsed, verify_elapsed
    );
    println!(
        "perf: retention over 5000 entries in {:.2?}, removed {}",
        gc_elapsed, gc.removed_entries
    );
    println!(
        "perf: entries {}, unique blobs {}, logical {} KB, on disk {} KB, index {} KB",
        stats.entries,
        stats.unique_blobs,
        stats.logical_bytes / 1024,
        stats.bytes_on_disk / 1024,
        stats.index_bytes / 1024
    );

    // Deterministic checks only: the timings above are printed for humans and never asserted,
    // so a loaded CI machine cannot fail this test. The work itself is what is verified.
    assert_eq!(stats.entries, 5000, "one entry per snapshot");
    assert_eq!(
        stats.unique_blobs, 5000,
        "every revision has a distinct body"
    );
}

#[test]
fn identical_resaves_of_a_big_note_are_cheap() {
    let v = TempVault::new("perf_dedupe");
    let mut h = v.open();
    let body = note_body(7);
    h.snapshot("same.md", &body, SnapshotMeta::auto(), T0)
        .expect("first");
    let start = Instant::now();
    for i in 0..2000 {
        assert!(
            h.snapshot("same.md", &body, SnapshotMeta::auto(), T0 + i)
                .expect("again")
                .is_none()
        );
    }
    let elapsed = start.elapsed();
    println!("perf: 2000 unchanged snapshots in {elapsed:.2?}");
    assert_eq!(h.list("same.md").len(), 1);
}
