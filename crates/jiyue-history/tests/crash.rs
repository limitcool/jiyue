mod common;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use common::{T0, TempVault};
use jiyue_history::{HistoryError, Retention, SnapshotMeta};

fn index_path(v: &TempVault) -> PathBuf {
    v.history_dir().join("index.jsonl")
}

/// Path of some blob file under `<history>/blobs/<aa>/`.
fn any_blob(v: &TempVault) -> PathBuf {
    let root = v.history_dir().join("blobs");
    for dir in fs::read_dir(&root).expect("blobs dir").flatten() {
        if let Some(file) = fs::read_dir(dir.path())
            .expect("fan-out dir")
            .flatten()
            .next()
        {
            return file.path();
        }
    }
    panic!("no blob files found");
}

fn blob_count(v: &TempVault) -> usize {
    let root = v.history_dir().join("blobs");
    fs::read_dir(&root)
        .expect("blobs dir")
        .flatten()
        .map(|d| fs::read_dir(d.path()).map(|f| f.count()).unwrap_or(0))
        .sum()
}

fn seed(v: &TempVault, notes: usize, versions: usize) -> Vec<jiyue_history::EntryId> {
    let mut h = v.open();
    let mut ids = Vec::new();
    for n in 0..notes {
        for k in 0..versions {
            let id = h
                .snapshot(
                    &format!("n{n}.md"),
                    format!("note {n} version {k}").as_bytes(),
                    SnapshotMeta::auto(),
                    T0 + k as i64,
                )
                .expect("seed")
                .expect("id");
            ids.push(id);
        }
    }
    ids
}

#[test]
fn truncated_index_tail_is_skipped_and_reported() {
    let v = TempVault::new("trunc_tail");
    seed(&v, 1, 5);
    let len = fs::metadata(index_path(&v)).expect("meta").len();
    OpenOptions::new()
        .write(true)
        .open(index_path(&v))
        .expect("open")
        .set_len(len - 10)
        .expect("truncate");
    let h = v.open();
    assert_eq!(
        h.list("n0.md").len(),
        4,
        "the last, truncated record is lost"
    );
    let report = h.verify();
    assert_eq!(report.bad_index_lines, 1);
    assert!(!report.is_ok());
}

#[test]
fn appends_after_a_truncated_tail_are_not_glued_onto_garbage() {
    let v = TempVault::new("trunc_glue");
    seed(&v, 1, 3);
    let len = fs::metadata(index_path(&v)).expect("meta").len();
    OpenOptions::new()
        .write(true)
        .open(index_path(&v))
        .expect("open")
        .set_len(len - 5)
        .expect("truncate");
    {
        let mut h = v.open();
        h.snapshot("n0.md", b"after crash", SnapshotMeta::auto(), T0 + 100)
            .expect("new snapshot");
    }
    let h = v.open();
    let texts: Vec<Vec<u8>> = h
        .list("n0.md")
        .iter()
        .map(|e| h.read(&e.id).expect("read"))
        .collect();
    assert_eq!(texts[0], b"after crash", "the new record must not be lost");
    assert_eq!(
        texts.len(),
        3,
        "two surviving old versions plus the new one"
    );
}

#[test]
fn garbage_bytes_in_the_middle_of_the_index_only_lose_that_line() {
    let v = TempVault::new("garbage_mid");
    seed(&v, 1, 4);
    let content = fs::read(index_path(&v)).expect("read");
    let mut lines: Vec<&[u8]> = content
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    let garbage: &[u8] = b"\x00\xff\xfe not json at all";
    lines.insert(2, garbage);
    let mut rebuilt = Vec::new();
    for l in lines {
        rebuilt.extend_from_slice(l);
        rebuilt.push(b'\n');
    }
    fs::write(index_path(&v), rebuilt).expect("write");
    let h = v.open();
    assert_eq!(h.list("n0.md").len(), 4);
    assert_eq!(h.verify().bad_index_lines, 1);
}

#[test]
fn entirely_garbage_index_opens_empty_and_accepts_new_work() {
    let v = TempVault::new("garbage_index");
    fs::create_dir_all(v.history_dir()).expect("dir");
    fs::write(
        index_path(&v),
        [0xde, 0xad, 0xbe, 0xef, b'\n', b'{', b'}', b'\n'],
    )
    .expect("write");
    {
        let mut h = v.open();
        assert_eq!(h.stats().entries, 0);
        assert!(h.verify().bad_index_lines >= 1);
        h.snapshot("fresh.md", b"works", SnapshotMeta::auto(), T0)
            .expect("snap");
    }
    let h = v.open();
    assert_eq!(h.list("fresh.md").len(), 1);
}

#[test]
fn index_with_invalid_hash_is_a_bad_line() {
    let v = TempVault::new("bad_hash_line");
    fs::create_dir_all(v.history_dir()).expect("dir");
    let line = r#"{"op":"put","id":1,"path":"a.md","time_ms":1,"size":1,"hash":"../../etc","kind":"auto","label":null}"#;
    fs::write(index_path(&v), format!("{line}\n")).expect("write");
    let h = v.open();
    assert_eq!(h.stats().entries, 0);
    assert_eq!(h.verify().bad_index_lines, 1);
}

#[test]
fn index_with_empty_path_is_a_bad_line() {
    let v = TempVault::new("bad_path_line");
    fs::create_dir_all(v.history_dir()).expect("dir");
    let hash = "a".repeat(64);
    let line = format!(
        r#"{{"op":"put","id":1,"path":"","time_ms":1,"size":1,"hash":"{hash}","kind":"auto","label":null}}"#
    );
    fs::write(index_path(&v), format!("{line}\n")).expect("write");
    let h = v.open();
    assert_eq!(h.stats().entries, 0);
    assert_eq!(h.verify().bad_index_lines, 1);
}

#[test]
fn deleted_blob_hides_its_entries_but_does_not_break_open() {
    let v = TempVault::new("missing_blob");
    seed(&v, 2, 2);
    let victim = any_blob(&v);
    fs::remove_file(&victim).expect("remove blob");
    let h = v.open();
    let total_visible: usize = (0..2).map(|n| h.list(&format!("n{n}.md")).len()).sum();
    assert!(
        total_visible < 4,
        "entries with the missing blob are hidden"
    );
    let report = h.verify();
    assert!(!report.missing_blobs.is_empty());
    assert!(!report.is_ok());
}

#[test]
fn reading_an_entry_whose_blob_is_missing_is_a_clear_error() {
    let v = TempVault::new("missing_read");
    let mut h_setup = v.open();
    let id = h_setup
        .snapshot("lost.md", b"gone soon", SnapshotMeta::auto(), T0)
        .expect("s")
        .expect("id");
    drop(h_setup);
    fs::remove_file(any_blob(&v)).expect("remove");
    let h = v.open();
    assert!(matches!(h.read(&id), Err(HistoryError::BlobMissing { .. })));
    assert!(h.list("lost.md").is_empty());
}

#[test]
fn snapshot_repairs_a_missing_blob_with_identical_content() {
    let v = TempVault::new("repair_blob");
    {
        let mut h = v.open();
        h.snapshot("r.md", b"repair me", SnapshotMeta::auto(), T0)
            .expect("s");
    }
    fs::remove_file(any_blob(&v)).expect("remove");
    let mut h = v.open();
    assert!(h.list("r.md").is_empty());
    let id = h
        .snapshot("r.md", b"repair me", SnapshotMeta::auto(), T0 + 1)
        .expect("s");
    assert!(
        id.is_some(),
        "a new entry is written because the old one is unusable"
    );
    // The old entry shares the hash, so it becomes visible again once the blob is repaired.
    let list = h.list("r.md");
    assert_eq!(list.len(), 2);
    for e in &list {
        assert_eq!(h.read(&e.id).expect("read"), b"repair me");
    }
    drop(h);
    let h = v.open();
    assert!(h.verify().missing_blobs.is_empty());
}

#[test]
fn corrupt_blob_is_detected_on_read_and_verify() {
    let v = TempVault::new("corrupt_blob");
    let id = {
        let mut h = v.open();
        h.snapshot("c.md", b"will be damaged", SnapshotMeta::auto(), T0)
            .expect("s")
            .expect("id")
    };
    fs::write(any_blob(&v), b"\x10\x00\x00\x00 definitely not lz4 data").expect("damage");
    let h = v.open();
    assert!(matches!(h.read(&id), Err(HistoryError::Corrupt { .. })));
    let report = h.verify();
    assert_eq!(report.corrupt_blobs, vec![id]);
    assert!(report.missing_blobs.is_empty());
}

#[test]
fn blob_with_oversized_declared_length_is_corrupt_not_a_crash() {
    let v = TempVault::new("oversize_prefix");
    let id = {
        let mut h = v.open();
        h.snapshot("c.md", b"huge prefix", SnapshotMeta::auto(), T0)
            .expect("s")
            .expect("id")
    };
    let mut bytes = u32::MAX.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
    fs::write(any_blob(&v), bytes).expect("damage");
    let h = v.open();
    assert!(matches!(h.read(&id), Err(HistoryError::Corrupt { .. })));
}

#[test]
fn stray_files_in_the_blob_tree_are_reported_and_collected() {
    let v = TempVault::new("stray");
    seed(&v, 1, 2);
    let stray = v.history_dir().join("blobs").join("zz");
    fs::create_dir_all(&stray).expect("dir");
    fs::write(stray.join("not-a-hash.tmp"), b"junk").expect("write");
    let mut h = v.open();
    assert_eq!(h.verify().stray_files, 1);
    let report = h
        .apply_retention(&Retention::default(), T0 + 1)
        .expect("gc");
    assert!(report.removed_blobs >= 1);
    assert_eq!(h.verify().stray_files, 0);
}

#[test]
fn orphan_blob_without_index_entry_is_collected_by_gc() {
    let v = TempVault::new("orphan");
    seed(&v, 1, 1);
    let orphan_hash = "b".repeat(64);
    let dir = v.history_dir().join("blobs").join("bb");
    fs::create_dir_all(&dir).expect("dir");
    fs::write(dir.join(&orphan_hash), b"orphan").expect("write");
    let mut h = v.open();
    assert_eq!(h.verify().orphan_blobs, 1);
    h.apply_retention(&Retention::default(), T0 + 1)
        .expect("gc");
    assert_eq!(h.verify().orphan_blobs, 0);
    assert!(!dir.join(&orphan_hash).exists());
}

#[test]
fn missing_index_with_blobs_present_opens_empty() {
    let v = TempVault::new("no_index");
    seed(&v, 1, 3);
    let blobs_before = blob_count(&v);
    fs::remove_file(index_path(&v)).expect("remove index");
    let h = v.open();
    assert_eq!(h.stats().entries, 0);
    assert_eq!(h.verify().orphan_blobs, blobs_before);
}

#[test]
fn leftover_compaction_temp_file_is_ignored() {
    let v = TempVault::new("tmp_index");
    seed(&v, 1, 2);
    fs::write(
        v.history_dir().join("index.jsonl.9.tmp"),
        b"half written garbage",
    )
    .expect("write");
    let h = v.open();
    assert_eq!(h.list("n0.md").len(), 2);
}

#[test]
fn history_directory_is_created_on_demand() {
    let v = TempVault::new("mkdir");
    let dir = v.history_dir();
    assert!(!Path::new(&dir).exists());
    let h = v.open();
    assert!(dir.join("blobs").is_dir());
    drop(h);
}

#[test]
fn many_interrupted_sessions_keep_every_good_record() {
    let v = TempVault::new("many_crashes");
    let mut expected = 0usize;
    for round in 0..6 {
        {
            let mut h = v.open();
            assert_eq!(h.list("r.md").len(), expected);
            h.snapshot(
                "r.md",
                format!("round {round}").as_bytes(),
                SnapshotMeta::auto(),
                T0 + round,
            )
            .expect("s");
            expected += 1;
        }
        // Simulate a torn write at the end of the log after every session.
        let mut f = OpenOptions::new()
            .append(true)
            .open(index_path(&v))
            .expect("open");
        f.write_all(b"{\"op\":\"put\",\"id\":").expect("tear");
    }
    let h = v.open();
    assert_eq!(h.list("r.md").len(), expected);
}
