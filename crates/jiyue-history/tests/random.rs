mod common;

use common::{Lcg, MIN_MS, T0, TempVault, text};
use jiyue_history::{EntryId, HistoryError, Retention, SnapshotMeta};

const PATHS: [&str; 5] = ["a.md", "b.md", "dir/c.md", "dir/d.md", "e.md"];

fn hash_of(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Invariants that must hold after every operation.
fn check_invariants(h: &jiyue_history::History) {
    let mut seen = std::collections::HashSet::new();
    for path in PATHS {
        let list = h.list(path);
        for pair in list.windows(2) {
            assert!(
                (pair[0].time_ms, pair[0].id.0) >= (pair[1].time_ms, pair[1].id.0),
                "list must be newest first"
            );
        }
        for e in &list {
            assert_eq!(e.path, path);
            assert!(seen.insert(e.id), "entry {} listed twice", e.id);
            let bytes = h.read(&e.id).expect("every listed entry is readable");
            assert_eq!(bytes.len() as u64, e.size, "size matches");
            assert_eq!(hash_of(&bytes), e.hash, "content matches its hash");
        }
    }
    assert!(h.verify().is_ok(), "verify must be clean: {:?}", h.verify());
}

#[test]
fn two_hundred_deterministic_operations_preserve_invariants() {
    let v = TempVault::new("random200");
    let mut rng = Lcg(0x5EED_2026);
    let pool: Vec<Vec<u8>> = (0..6).map(|i| text(&format!("pool{i}"), 2 + i)).collect();
    let mut now = T0;
    let mut h = v.open();
    let mut known: Vec<EntryId> = Vec::new();
    let mut counts = [0usize; 8];

    for _step in 0..200 {
        now += (rng.below(120) as i64) * MIN_MS;
        let op = rng.below(100);
        match op {
            0..=39 => {
                counts[0] += 1;
                let path = PATHS[rng.below(PATHS.len() as u32) as usize];
                let body = &pool[rng.below(pool.len() as u32) as usize];
                let latest_before = h.list(path).first().map(|e| e.id);
                let res = h
                    .snapshot(path, body, SnapshotMeta::auto(), now)
                    .expect("snapshot");
                let latest_after = h.list(path).first().map(|e| e.id);
                match res {
                    Some(id) => {
                        assert_eq!(latest_after, Some(id));
                        known.push(id);
                    }
                    None => {
                        assert_eq!(latest_after, latest_before, "dedupe must not move the head")
                    }
                }
            }
            40..=49 => {
                counts[1] += 1;
                let path = PATHS[rng.below(PATHS.len() as u32) as usize];
                let body = &pool[rng.below(pool.len() as u32) as usize];
                let label = if rng.below(2) == 0 {
                    Some(format!("mark {now}"))
                } else {
                    None
                };
                if let Some(id) = h
                    .snapshot(path, body, SnapshotMeta::manual(label), now)
                    .expect("manual snapshot")
                {
                    known.push(id);
                }
            }
            50..=57 => {
                counts[2] += 1;
                let from = PATHS[rng.below(PATHS.len() as u32) as usize];
                let to = PATHS[rng.below(PATHS.len() as u32) as usize];
                let before: usize = h.list(from).len();
                h.note_renamed(from, to).expect("rename");
                if from != to {
                    assert!(h.list(from).is_empty());
                    assert!(h.list(to).len() >= before);
                }
            }
            58..=63 => {
                counts[3] += 1;
                let path = PATHS[rng.below(PATHS.len() as u32) as usize];
                let body = pool[rng.below(pool.len() as u32) as usize].clone();
                h.note_deleted(path, &body, now).expect("delete");
            }
            64..=71 => {
                counts[4] += 1;
                if let Some(&target) = known.get(rng.below(known.len().max(1) as u32) as usize) {
                    let current: Option<Vec<u8>> = if rng.below(3) == 0 {
                        None
                    } else {
                        Some(pool[rng.below(pool.len() as u32) as usize].clone())
                    };
                    match h.restore_bytes(&target, current.as_deref(), now) {
                        Ok(bytes) => {
                            let original = h.read(&target).expect("target still readable");
                            assert_eq!(bytes, original);
                        }
                        Err(HistoryError::NotFound(_)) => {
                            // Removed by an earlier retention pass; acceptable.
                        }
                        Err(other) => panic!("unexpected restore error: {other}"),
                    }
                }
            }
            72..=79 => {
                counts[5] += 1;
                if let Some(&target) = known.get(rng.below(known.len().max(1) as u32) as usize) {
                    let label = if rng.below(2) == 0 {
                        Some(format!("L{now}"))
                    } else {
                        None
                    };
                    match h.set_label(&target, label) {
                        Ok(()) | Err(HistoryError::NotFound(_)) => {}
                        Err(other) => panic!("unexpected label error: {other}"),
                    }
                }
            }
            80..=86 => {
                counts[6] += 1;
                let policy = if rng.below(2) == 0 {
                    Retention::default()
                } else {
                    Retention {
                        max_total_bytes: 200,
                        ..Retention::default()
                    }
                };
                h.apply_retention(&policy, now).expect("retention");
            }
            87..=92 => {
                counts[7] += 1;
                drop(h);
                h = v.open();
            }
            _ => {
                let _ = h.stats();
            }
        }
        check_invariants(&h);
    }

    assert!(
        counts.iter().all(|c| *c > 0),
        "every operation kind ran: {counts:?}"
    );
    let stats = h.stats();
    assert!(stats.entries > 0);
}

#[test]
fn second_random_sequence_with_a_different_seed_also_holds() {
    let v = TempVault::new("random_seed2");
    let mut rng = Lcg(42);
    let mut h = v.open();
    let mut now = T0;
    for _ in 0..120 {
        now += (rng.below(600) as i64) * MIN_MS;
        let path = PATHS[rng.below(PATHS.len() as u32) as usize];
        let body = text(&format!("s{}", rng.below(4)), 1 + rng.below(4) as usize);
        h.snapshot(path, &body, SnapshotMeta::auto(), now)
            .expect("snap");
        if rng.below(10) == 0 {
            h.apply_retention(&Retention::default(), now).expect("gc");
        }
    }
    check_invariants(&h);
}
