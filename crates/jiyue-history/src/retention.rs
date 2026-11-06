//! Retention policy and snapshot throttling. Pure functions; time is passed in.
//!
//! Rules for each note (entries sorted newest first):
//! - the newest entry is always kept (a note never loses its last snapshot);
//! - entries that are protected (`Manual` or labelled, when `keep_manual_forever`) are kept;
//! - entries younger than `keep_all_ms` are kept;
//! - entries up to `hourly_until_ms` old keep the newest entry of each clock hour;
//! - entries up to `daily_until_ms` old keep the newest entry of each day;
//! - older entries keep the newest entry of each ISO-less week when `weekly_after`,
//!   otherwise they are dropped.
//!
//! Afterwards, if the unique content exceeds `max_total_bytes`, the oldest
//! non-pinned kept entries are dropped until it fits (newest-per-note and
//! protected entries are never dropped, so the cap can be exceeded if only
//! those remain).

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::model::{Entry, EntryId, Kind};

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;
pub const WEEK_MS: i64 = 7 * DAY_MS;

/// Default minimum gap between two automatic snapshots of one note.
pub const DEFAULT_MIN_INTERVAL_MS: i64 = 120_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retention {
    /// Keep every entry younger than this (default 1 day).
    pub keep_all_ms: i64,
    /// Keep the newest entry per hour up to this age (default 7 days).
    pub hourly_until_ms: i64,
    /// Keep the newest entry per day up to this age (default 90 days).
    pub daily_until_ms: i64,
    /// Beyond `daily_until_ms`: keep the newest entry per week (true) or drop (false).
    pub weekly_after: bool,
    /// Cap on the unique content kept (default 500 MB, uncompressed sizes).
    pub max_total_bytes: u64,
    /// Never drop `Manual` entries or labelled entries.
    pub keep_manual_forever: bool,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            keep_all_ms: DAY_MS,
            hourly_until_ms: WEEK_MS,
            daily_until_ms: 90 * DAY_MS,
            weekly_after: true,
            max_total_bytes: 500 * 1024 * 1024,
            keep_manual_forever: true,
        }
    }
}

/// True when an automatic snapshot is due: never snapshotted, or at least
/// `min_interval_ms` since the last one. A clock that went backwards also
/// counts as due.
pub fn should_snapshot(last_time_ms: Option<i64>, now_ms: i64, min_interval_ms: i64) -> bool {
    match last_time_ms {
        None => true,
        Some(last) if now_ms < last => true,
        Some(last) => now_ms.saturating_sub(last) >= min_interval_ms,
    }
}

/// Like `should_snapshot`, but explicit kinds (`Manual`, `BeforeSync`,
/// `BeforeRestore`, `Deleted`) are always due.
pub fn should_snapshot_kind(
    kind: Kind,
    last_time_ms: Option<i64>,
    now_ms: i64,
    min_interval_ms: i64,
) -> bool {
    match kind {
        Kind::Manual | Kind::BeforeSync | Kind::BeforeRestore | Kind::Deleted => true,
        Kind::Auto => should_snapshot(last_time_ms, now_ms, min_interval_ms),
    }
}

pub(crate) fn is_protected(entry: &Entry, policy: &Retention) -> bool {
    policy.keep_manual_forever && (entry.kind == Kind::Manual || entry.label.is_some())
}

/// Bucket an entry falls into once it is older than `keep_all_ms`. `None` means
/// the entry is inside the keep-all window, or it is too old to keep under the
/// policy (then it is dropped unless pinned).
fn bucket_for(age: i64, time_ms: i64, policy: &Retention) -> Option<(u8, i64)> {
    if age <= policy.keep_all_ms {
        None
    } else if age <= policy.hourly_until_ms {
        Some((0, time_ms.div_euclid(HOUR_MS)))
    } else if age <= policy.daily_until_ms {
        Some((1, time_ms.div_euclid(DAY_MS)))
    } else if policy.weekly_after {
        Some((2, time_ms.div_euclid(WEEK_MS)))
    } else {
        None
    }
}

/// Decide which entries to drop. Returns ids in input order.
pub(crate) fn plan_removals(entries: &[&Entry], policy: &Retention, now_ms: i64) -> Vec<EntryId> {
    let mut by_path: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for e in entries {
        by_path.entry(e.path.as_str()).or_default().push(e);
    }

    let mut keep: HashSet<u64> = HashSet::new();
    let mut pinned: HashSet<u64> = HashSet::new();
    for list in by_path.values_mut() {
        list.sort_by_key(|e| std::cmp::Reverse((e.time_ms, e.id.0)));
        let mut buckets: HashSet<(u8, i64)> = HashSet::new();
        for (idx, e) in list.iter().enumerate() {
            let age = now_ms.saturating_sub(e.time_ms);
            let bucket = bucket_for(age, e.time_ms, policy);
            // The newest entry and protected entries are always kept. They also claim
            // their bucket, so older entries in the same bucket are not kept for it.
            if idx == 0 || is_protected(e, policy) {
                keep.insert(e.id.0);
                pinned.insert(e.id.0);
                if let Some(b) = bucket {
                    buckets.insert(b);
                }
                continue;
            }
            if age <= policy.keep_all_ms {
                keep.insert(e.id.0);
                continue;
            }
            if let Some(b) = bucket
                && buckets.insert(b)
            {
                keep.insert(e.id.0);
            }
        }
    }

    // Size cap on unique content.
    let mut refs: HashMap<&str, (u64, usize)> = HashMap::new();
    for e in entries.iter().filter(|e| keep.contains(&e.id.0)) {
        refs.entry(e.hash.as_str()).or_insert((e.size, 0)).1 += 1;
    }
    let mut total: u64 = refs.values().map(|(size, _)| *size).sum();
    if total > policy.max_total_bytes {
        let mut candidates: Vec<&Entry> = entries
            .iter()
            .copied()
            .filter(|e| keep.contains(&e.id.0) && !pinned.contains(&e.id.0))
            .collect();
        candidates.sort_by_key(|a| (a.time_ms, a.id.0));
        for c in candidates {
            if total <= policy.max_total_bytes {
                break;
            }
            if let Some(slot) = refs.get_mut(c.hash.as_str()) {
                slot.1 = slot.1.saturating_sub(1);
                if slot.1 == 0 {
                    total = total.saturating_sub(slot.0);
                }
            }
            keep.remove(&c.id.0);
        }
    }

    entries
        .iter()
        .filter(|e| !keep.contains(&e.id.0))
        .map(|e| e.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_800_000_000_000;

    fn entry(id: u64, path: &str, time_ms: i64, hash: &str, size: u64, kind: Kind) -> Entry {
        Entry {
            id: EntryId(id),
            path: path.to_string(),
            time_ms,
            size,
            hash: hash.to_string(),
            kind,
            label: None,
        }
    }

    fn plan(entries: &[Entry], policy: &Retention, now: i64) -> Vec<u64> {
        let refs: Vec<&Entry> = entries.iter().collect();
        plan_removals(&refs, policy, now)
            .into_iter()
            .map(|id| id.0)
            .collect()
    }

    fn no_cap() -> Retention {
        Retention {
            max_total_bytes: u64::MAX,
            ..Retention::default()
        }
    }

    #[test]
    fn should_snapshot_first_time_and_after_interval() {
        assert!(should_snapshot(None, T0, DEFAULT_MIN_INTERVAL_MS));
        assert!(!should_snapshot(
            Some(T0),
            T0 + 119_999,
            DEFAULT_MIN_INTERVAL_MS
        ));
        assert!(should_snapshot(
            Some(T0),
            T0 + 120_000,
            DEFAULT_MIN_INTERVAL_MS
        ));
    }

    #[test]
    fn should_snapshot_when_clock_went_backwards() {
        assert!(should_snapshot(Some(T0), T0 - 5, DEFAULT_MIN_INTERVAL_MS));
    }

    #[test]
    fn should_snapshot_kind_always_for_explicit_kinds() {
        for kind in [
            Kind::Manual,
            Kind::BeforeSync,
            Kind::BeforeRestore,
            Kind::Deleted,
        ] {
            assert!(should_snapshot_kind(
                kind,
                Some(T0),
                T0,
                DEFAULT_MIN_INTERVAL_MS
            ));
        }
        assert!(!should_snapshot_kind(
            Kind::Auto,
            Some(T0),
            T0,
            DEFAULT_MIN_INTERVAL_MS
        ));
    }

    #[test]
    fn everything_younger_than_a_day_is_kept() {
        let entries: Vec<Entry> = (0..50)
            .map(|i| entry(i + 1, "a.md", T0 + i as i64 * 60_000, "h", 1, Kind::Auto))
            .collect();
        let now = T0 + 50 * 60_000;
        assert!(plan(&entries, &Retention::default(), now).is_empty());
    }

    #[test]
    fn hourly_bucket_keeps_newest_per_hour_within_a_week() {
        // 60 entries, one per minute, starting exactly on an hour boundary.
        let base = (T0 / HOUR_MS) * HOUR_MS;
        let entries: Vec<Entry> = (0..60)
            .map(|i| entry(i + 1, "a.md", base + i as i64 * 60_000, "h", 1, Kind::Auto))
            .collect();
        let now = base + 3 * DAY_MS;
        let dropped = plan(&entries, &no_cap(), now);
        // Entries span one hour, so exactly one bucket is kept; the newest (id 60) must survive.
        assert!(!dropped.contains(&60));
        assert_eq!(dropped.len(), 59);
    }

    #[test]
    fn daily_bucket_applies_between_a_week_and_ninety_days() {
        let base = (T0 / DAY_MS) * DAY_MS;
        // Twelve entries spread over one day, all 30 days old.
        let entries: Vec<Entry> = (0..12)
            .map(|i| {
                entry(
                    i + 1,
                    "a.md",
                    base + i as i64 * 2 * HOUR_MS,
                    "h",
                    1,
                    Kind::Auto,
                )
            })
            .collect();
        let now = base + 30 * DAY_MS;
        let dropped = plan(&entries, &no_cap(), now);
        // All in one day bucket: only the newest survives; the newest per note is the one kept.
        assert_eq!(dropped.len(), 11);
        assert!(!dropped.contains(&12));
    }

    #[test]
    fn weekly_bucket_after_ninety_days_when_enabled() {
        let base = (T0 / WEEK_MS) * WEEK_MS;
        // One entry per day for 28 days, all older than 90 days.
        let entries: Vec<Entry> = (0..28)
            .map(|i| entry(i + 1, "a.md", base + i as i64 * DAY_MS, "h", 1, Kind::Auto))
            .collect();
        let now = base + 28 * DAY_MS + 200 * DAY_MS;
        let dropped = plan(&entries, &no_cap(), now);
        // 28 days span exactly four week buckets (plus at most one partial): 4 or 5 survive.
        let kept = 28 - dropped.len();
        assert!((4..=5).contains(&kept), "kept {kept}");
        assert!(!dropped.contains(&28), "newest must survive");
    }

    #[test]
    fn old_entries_are_dropped_when_weekly_disabled() {
        let base = (T0 / WEEK_MS) * WEEK_MS;
        let entries: Vec<Entry> = (0..10)
            .map(|i| entry(i + 1, "a.md", base + i as i64 * DAY_MS, "h", 1, Kind::Auto))
            .collect();
        let policy = Retention {
            weekly_after: false,
            ..no_cap()
        };
        let now = base + 10 * DAY_MS + 365 * DAY_MS;
        let dropped = plan(&entries, &policy, now);
        // Only the newest per note (id 10) survives.
        assert_eq!(dropped.len(), 9);
        assert!(!dropped.contains(&10));
    }

    #[test]
    fn newest_entry_of_each_note_is_always_kept() {
        let entries = vec![
            entry(1, "a.md", T0, "h1", 1, Kind::Auto),
            entry(2, "b.md", T0, "h2", 1, Kind::Auto),
        ];
        let now = T0 + 5000 * DAY_MS;
        let policy = Retention {
            weekly_after: false,
            ..no_cap()
        };
        assert!(plan(&entries, &policy, now).is_empty());
    }

    #[test]
    fn sole_snapshot_of_an_old_note_survives_even_with_tiny_cap() {
        let entries = vec![entry(1, "old.md", T0, "h", 1000, Kind::Auto)];
        let policy = Retention {
            max_total_bytes: 1,
            ..Retention::default()
        };
        assert!(plan(&entries, &policy, T0 + 4000 * DAY_MS).is_empty());
    }

    #[test]
    fn manual_entries_are_kept_forever_when_configured() {
        let mut manual = entry(1, "a.md", T0, "h1", 1, Kind::Manual);
        manual.label = None;
        let newer = entry(2, "a.md", T0 + 1000 * DAY_MS, "h2", 1, Kind::Auto);
        let entries = vec![manual, newer];
        let policy = Retention {
            weekly_after: false,
            ..no_cap()
        };
        let now = T0 + 2000 * DAY_MS;
        assert!(plan(&entries, &policy, now).is_empty());
        let forget = Retention {
            keep_manual_forever: false,
            ..policy
        };
        assert_eq!(plan(&entries, &forget, now), vec![1]);
    }

    #[test]
    fn labelled_entries_are_protected_like_manual() {
        let mut labelled = entry(1, "a.md", T0, "h1", 1, Kind::Auto);
        labelled.label = Some("before refactor".into());
        let newer = entry(2, "a.md", T0 + 1000 * DAY_MS, "h2", 1, Kind::Auto);
        let policy = Retention {
            weekly_after: false,
            ..no_cap()
        };
        assert!(plan(&[labelled, newer], &policy, T0 + 2000 * DAY_MS).is_empty());
    }

    #[test]
    fn size_cap_drops_oldest_unpinned_first() {
        // Three notes-worth of distinct content, 400 bytes each, all recent (kept by age).
        let entries = vec![
            entry(1, "a.md", T0, "h1", 400, Kind::Auto),
            entry(2, "a.md", T0 + 1, "h2", 400, Kind::Auto),
            entry(3, "a.md", T0 + 2, "h3", 400, Kind::Auto),
        ];
        let policy = Retention {
            max_total_bytes: 900,
            ..Retention::default()
        };
        let dropped = plan(&entries, &policy, T0 + 3);
        assert_eq!(dropped, vec![1]);
    }

    #[test]
    fn size_cap_counts_shared_content_once() {
        let entries = vec![
            entry(1, "a.md", T0, "same", 400, Kind::Auto),
            entry(2, "b.md", T0 + 1, "same", 400, Kind::Auto),
            entry(3, "a.md", T0 + 2, "other", 400, Kind::Auto),
        ];
        // Unique content is 800 bytes, under the cap, so nothing is dropped.
        let policy = Retention {
            max_total_bytes: 800,
            ..Retention::default()
        };
        assert!(plan(&entries, &policy, T0 + 3).is_empty());
    }

    #[test]
    fn size_cap_releases_content_only_when_last_reference_goes() {
        let entries = vec![
            entry(1, "a.md", T0, "shared", 500, Kind::Auto),
            entry(2, "b.md", T0 + 1, "shared", 500, Kind::Auto),
            entry(3, "a.md", T0 + 2, "tail", 300, Kind::Auto),
            entry(4, "b.md", T0 + 3, "tail", 300, Kind::Auto),
        ];
        // Pinned: newest per note = ids 3 and 4 (both "tail"). Candidates: 1 and 2 ("shared").
        // Total unique = 800; cap 500 -> drop 1 (still shared, total 800), then 2 (total 300).
        let policy = Retention {
            max_total_bytes: 500,
            ..Retention::default()
        };
        assert_eq!(plan(&entries, &policy, T0 + 4), vec![1, 2]);
    }

    #[test]
    fn size_cap_cannot_drop_pinned_entries() {
        let entries = vec![
            entry(1, "a.md", T0, "h1", 1000, Kind::Manual),
            entry(2, "b.md", T0 + 1, "h2", 1000, Kind::Auto),
        ];
        let policy = Retention {
            max_total_bytes: 10,
            ..Retention::default()
        };
        // Id 1 is protected (manual), id 2 is the newest of its note: nothing may go.
        assert!(plan(&entries, &policy, T0 + 2).is_empty());
    }

    #[test]
    fn future_timestamps_are_kept() {
        let entries = vec![
            entry(1, "a.md", T0 + 10 * DAY_MS, "h1", 1, Kind::Auto),
            entry(2, "a.md", T0 + 9 * DAY_MS, "h2", 1, Kind::Auto),
        ];
        let policy = Retention {
            weekly_after: false,
            ..no_cap()
        };
        assert!(plan(&entries, &policy, T0).is_empty());
    }

    #[test]
    fn retention_is_per_note() {
        let base = (T0 / DAY_MS) * DAY_MS;
        let entries = vec![
            entry(1, "a.md", base, "x1", 1, Kind::Auto),
            entry(2, "a.md", base + HOUR_MS, "x2", 1, Kind::Auto),
            entry(3, "b.md", base, "y1", 1, Kind::Auto),
            entry(4, "b.md", base + HOUR_MS, "y2", 1, Kind::Auto),
        ];
        let policy = Retention {
            weekly_after: false,
            ..no_cap()
        };
        let now = base + 30 * DAY_MS;
        let dropped = plan(&entries, &policy, now);
        // Each note keeps only its newest (2 and 4): one entry per note per day bucket.
        assert_eq!(dropped, vec![1, 3]);
    }

    #[test]
    fn default_policy_values_match_spec() {
        let p = Retention::default();
        assert_eq!(p.keep_all_ms, DAY_MS);
        assert_eq!(p.hourly_until_ms, 7 * DAY_MS);
        assert_eq!(p.daily_until_ms, 90 * DAY_MS);
        assert_eq!(p.max_total_bytes, 500 * 1024 * 1024);
        assert!(p.weekly_after);
        assert!(p.keep_manual_forever);
    }
}
