//! Sync planning: local snapshot, remote listing and sync state to actions. Owned by W1-D2.
//!
//! Per path, three views are compared: the local file (content hash), the base (what both sides
//! had at the last successful sync) and the remote object (etag / version). Each path gets one
//! action; paths that need nothing produce no action.
//!
//! Decision table (local = present now, base = recorded at last sync, remote = listed now):
//!
//! | local | base | remote | action |
//! |---|---|---|---|
//! | new | none | none | Upload (create only if absent) |
//! | none | none | new | Download |
//! | new | none | present | Merge (text) / Conflict (binary): both sides are new |
//! | unchanged | present | deleted | DeleteLocal (moved to trash) |
//! | changed | present | deleted | Upload (resurrect; create only if absent) |
//! | deleted | present | unchanged | DeleteRemote |
//! | deleted | present | changed | Download (resurrect) |
//! | deleted | present | deleted | Noop (the executor forgets the base) |
//! | changed | present | unchanged | Upload, conditional on the base etag |
//! | unchanged | present | changed | Download |
//! | changed | present | changed | Merge (text) / Conflict (binary) |
//! | unchanged | present | unchanged | Noop |
//!
//! Renames are not detected: a rename is a delete plus a create. The delete side goes to trash on
//! the other device, so no content is lost.

use std::collections::{BTreeMap, BTreeSet};

use crate::state::is_text_path;
use crate::{BaseEntry, LocalSnapshot, RemoteEntry, SyncAction, SyncState};

impl SyncAction {
    /// The vault-relative path this action is about (`None` for `Noop`).
    pub fn path(&self) -> Option<&str> {
        match self {
            SyncAction::Upload { path }
            | SyncAction::Download { path }
            | SyncAction::DeleteLocal { path }
            | SyncAction::DeleteRemote { path }
            | SyncAction::Merge { path }
            | SyncAction::Conflict { path } => Some(path),
            SyncAction::Noop => None,
        }
    }
}

/// Decides what to do for each path. Paths under unreadable local folders are left alone.
/// `Noop` entries are omitted from the result. Order: uploads (smallest first), downloads, merges,
/// conflicts, remote deletes, local deletes; ties by path. The order is deterministic.
pub fn plan_sync(
    local: &LocalSnapshot,
    state: &SyncState,
    remote: &[RemoteEntry],
) -> Vec<SyncAction> {
    plan_sync_filtered(local, state, remote, &|_| false)
}

/// [`plan_sync`] that also leaves alone every path for which `skip` returns true (used for ignored
/// paths, so that a newly ignored file is not treated as deleted).
pub(crate) fn plan_sync_filtered(
    local: &LocalSnapshot,
    state: &SyncState,
    remote: &[RemoteEntry],
    skip: &dyn Fn(&str) -> bool,
) -> Vec<SyncAction> {
    let remote_map: BTreeMap<&str, &RemoteEntry> = remote
        .iter()
        .filter(|e| !e.is_dir)
        .map(|e| (e.path.as_str(), e))
        .collect();

    let mut paths: BTreeSet<&str> = BTreeSet::new();
    paths.extend(local.files.keys().map(String::as_str));
    paths.extend(state.entries.keys().map(String::as_str));
    paths.extend(state.base.keys().map(String::as_str));
    paths.extend(remote_map.keys().copied());

    let mut actions = Vec::new();
    for path in paths {
        if local.is_unreadable(path) || skip(path) {
            continue;
        }
        let base = state.base_entry(path);
        let action = decide(
            path,
            local.files.get(path).map(String::as_str),
            base.as_ref(),
            remote_map.get(path).copied(),
            is_text_path(path),
        );
        if action != SyncAction::Noop {
            actions.push(action);
        }
    }
    actions.sort_by_cached_key(|a| sort_key(a, local));
    actions
}

fn sort_key(action: &SyncAction, local: &LocalSnapshot) -> (u8, u64, String) {
    let path = action.path().unwrap_or_default().to_owned();
    match action {
        SyncAction::Upload { .. } => {
            let size = local.entries.get(&path).map(|e| e.size).unwrap_or(0);
            (0, size, path)
        }
        SyncAction::Download { .. } => (1, 0, path),
        SyncAction::Merge { .. } => (2, 0, path),
        SyncAction::Conflict { .. } => (3, 0, path),
        SyncAction::DeleteRemote { .. } => (4, 0, path),
        SyncAction::DeleteLocal { .. } => (5, 0, path),
        SyncAction::Noop => (6, 0, path),
    }
}

/// The decision for one path. `local` is the current content hash (`None` when absent), `base`
/// the recorded base, `remote` the current remote object. `text` selects merge over conflict.
pub(crate) fn decide(
    path: &str,
    local: Option<&str>,
    base: Option<&BaseEntry>,
    remote: Option<&RemoteEntry>,
    text: bool,
) -> SyncAction {
    let p = path.to_owned();
    let both_changed = if text {
        SyncAction::Merge { path: p.clone() }
    } else {
        SyncAction::Conflict { path: p.clone() }
    };
    match (local, base, remote) {
        (None, None, None) => SyncAction::Noop,
        (Some(_), None, None) => SyncAction::Upload { path: p },
        (None, None, Some(_)) => SyncAction::Download { path: p },
        (Some(_), None, Some(_)) => both_changed,
        (Some(l), Some(b), None) => {
            if l == b.hash {
                SyncAction::DeleteLocal { path: p }
            } else {
                SyncAction::Upload { path: p }
            }
        }
        // Deleted on both sides: nothing to do; the executor forgets the base.
        (None, Some(_), None) => SyncAction::Noop,
        (None, Some(b), Some(r)) => {
            if remote_changed(b, r) {
                SyncAction::Download { path: p }
            } else {
                SyncAction::DeleteRemote { path: p }
            }
        }
        (Some(l), Some(b), Some(r)) => match (l != b.hash, remote_changed(b, r)) {
            (false, false) => SyncAction::Noop,
            (true, false) => SyncAction::Upload { path: p },
            (false, true) => SyncAction::Download { path: p },
            (true, true) => both_changed,
        },
    }
}

/// Whether the remote object differs from the base. Compares etags, then version ids. When the
/// store gives neither, the object counts as changed (safe: a merge never loses content).
pub(crate) fn remote_changed(base: &BaseEntry, remote: &RemoteEntry) -> bool {
    if let (Some(b), Some(r)) = (&base.etag, &remote.etag) {
        return b != r;
    }
    if let (Some(b), Some(r)) = (&base.version_id, &remote.version_id) {
        return b != r;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LocalEntry;

    fn base(hash: &str, etag: Option<&str>) -> BaseEntry {
        BaseEntry {
            hash: hash.into(),
            etag: etag.map(str::to_owned),
            version_id: None,
            size: 1,
        }
    }

    fn remote(path: &str, etag: Option<&str>) -> RemoteEntry {
        RemoteEntry {
            path: path.into(),
            size: 1,
            etag: etag.map(str::to_owned),
            modified_ms: None,
            version_id: None,
            is_dir: false,
        }
    }

    fn snapshot(files: &[(&str, &str)]) -> LocalSnapshot {
        let mut snap = LocalSnapshot::default();
        for (p, h) in files {
            snap.files.insert((*p).to_owned(), (*h).to_owned());
            snap.entries.insert(
                (*p).to_owned(),
                LocalEntry {
                    hash: (*h).to_owned(),
                    size: 1,
                    mtime_ms: 0,
                },
            );
        }
        snap
    }

    fn state_with(bases: &[(&str, &str, &str)]) -> SyncState {
        let mut st = SyncState::default();
        for (p, h, e) in bases {
            st.set_base(p, base(h, Some(e)));
        }
        st
    }

    fn label(a: &SyncAction) -> String {
        match a {
            SyncAction::Upload { path } => format!("up:{path}"),
            SyncAction::Download { path } => format!("down:{path}"),
            SyncAction::Merge { path } => format!("merge:{path}"),
            SyncAction::Conflict { path } => format!("conflict:{path}"),
            SyncAction::DeleteRemote { path } => format!("delremote:{path}"),
            SyncAction::DeleteLocal { path } => format!("dellocal:{path}"),
            SyncAction::Noop => "noop".to_owned(),
        }
    }

    /// Decision for a text file named `a.md`.
    fn text(local: Option<&str>, b: Option<&BaseEntry>, r: Option<&RemoteEntry>) -> SyncAction {
        decide("a.md", local, b, r, true)
    }

    /// Decision for a binary file named `a.png`.
    fn binary(local: Option<&str>, b: Option<&BaseEntry>, r: Option<&RemoteEntry>) -> SyncAction {
        decide("a.png", local, b, r, false)
    }

    // ----- decision table, row by row -----

    #[test]
    fn row_nothing_anywhere_is_noop() {
        assert_eq!(text(None, None, None), SyncAction::Noop);
    }

    #[test]
    fn row_new_local_uploads() {
        assert_eq!(
            text(Some("h"), None, None),
            SyncAction::Upload {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_new_remote_downloads() {
        let r = remote("a.md", Some("e1"));
        assert_eq!(
            text(None, None, Some(&r)),
            SyncAction::Download {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_both_new_text_merges() {
        let r = remote("a.md", Some("e1"));
        assert_eq!(
            text(Some("h"), None, Some(&r)),
            SyncAction::Merge {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_both_new_binary_conflicts() {
        let r = remote("a.png", Some("e1"));
        assert_eq!(
            binary(Some("h"), None, Some(&r)),
            SyncAction::Conflict {
                path: "a.png".into()
            }
        );
    }

    #[test]
    fn row_unchanged_both_is_noop() {
        let b = base("h", Some("e1"));
        let r = remote("a.md", Some("e1"));
        assert_eq!(text(Some("h"), Some(&b), Some(&r)), SyncAction::Noop);
    }

    #[test]
    fn row_only_local_changed_uploads() {
        let b = base("h", Some("e1"));
        let r = remote("a.md", Some("e1"));
        assert_eq!(
            text(Some("h2"), Some(&b), Some(&r)),
            SyncAction::Upload {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_only_remote_changed_downloads() {
        let b = base("h", Some("e1"));
        let r = remote("a.md", Some("e2"));
        assert_eq!(
            text(Some("h"), Some(&b), Some(&r)),
            SyncAction::Download {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_both_changed_text_merges() {
        let b = base("h", Some("e1"));
        let r = remote("a.md", Some("e2"));
        assert_eq!(
            text(Some("h2"), Some(&b), Some(&r)),
            SyncAction::Merge {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_both_changed_binary_conflicts() {
        let b = base("h", Some("e1"));
        let r = remote("a.png", Some("e2"));
        assert_eq!(
            binary(Some("h2"), Some(&b), Some(&r)),
            SyncAction::Conflict {
                path: "a.png".into()
            }
        );
    }

    #[test]
    fn row_local_deleted_remote_unchanged_deletes_remote() {
        let b = base("h", Some("e1"));
        let r = remote("a.md", Some("e1"));
        assert_eq!(
            text(None, Some(&b), Some(&r)),
            SyncAction::DeleteRemote {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_local_deleted_remote_changed_resurrects_by_download() {
        let b = base("h", Some("e1"));
        let r = remote("a.md", Some("e2"));
        assert_eq!(
            text(None, Some(&b), Some(&r)),
            SyncAction::Download {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_remote_deleted_local_unchanged_deletes_local() {
        let b = base("h", Some("e1"));
        assert_eq!(
            text(Some("h"), Some(&b), None),
            SyncAction::DeleteLocal {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_remote_deleted_local_changed_reuploads() {
        let b = base("h", Some("e1"));
        assert_eq!(
            text(Some("h2"), Some(&b), None),
            SyncAction::Upload {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn row_deleted_on_both_sides_is_noop() {
        let b = base("h", Some("e1"));
        assert_eq!(text(None, Some(&b), None), SyncAction::Noop);
    }

    #[test]
    fn remote_changed_uses_etag_then_version_then_assumes_changed() {
        let b = base("h", Some("e1"));
        assert!(!remote_changed(&b, &remote("a", Some("e1"))));
        assert!(remote_changed(&b, &remote("a", Some("e2"))));

        let bv = BaseEntry {
            hash: "h".into(),
            etag: None,
            version_id: Some("v1".into()),
            size: 1,
        };
        let same = RemoteEntry {
            version_id: Some("v1".into()),
            ..remote("a", None)
        };
        let newer = RemoteEntry {
            version_id: Some("v2".into()),
            ..remote("a", None)
        };
        assert!(!remote_changed(&bv, &same));
        assert!(remote_changed(&bv, &newer));

        assert!(remote_changed(&base("h", None), &remote("a", None)));
    }

    #[test]
    fn etag_takes_precedence_over_matching_version() {
        let b = BaseEntry {
            hash: "h".into(),
            etag: Some("e1".into()),
            version_id: Some("v1".into()),
            size: 1,
        };
        let r = RemoteEntry {
            etag: Some("e2".into()),
            version_id: Some("v1".into()),
            ..remote("a", None)
        };
        assert!(remote_changed(&b, &r));
    }

    #[test]
    fn action_path_accessor() {
        assert_eq!(SyncAction::Upload { path: "x".into() }.path(), Some("x"));
        assert_eq!(SyncAction::Conflict { path: "y".into() }.path(), Some("y"));
        assert_eq!(SyncAction::Noop.path(), None);
    }

    // ----- whole-plan behaviour -----

    #[test]
    fn plan_produces_one_action_per_changed_path() {
        let local = snapshot(&[
            ("new-local.md", "n1"),
            ("edited-local.md", "e2"),
            ("both-edit.md", "b2"),
            ("keep.md", "k"),
            ("unchanged-both.md", "x"),
            ("remote-gone.md", "rg"),
        ]);
        let st = state_with(&[
            ("edited-local.md", "e1", "E1"),
            ("both-edit.md", "b1", "B1"),
            ("keep.md", "k", "K1"),
            ("unchanged-both.md", "x", "X1"),
            ("remote-gone.md", "rg", "RG1"),
            ("local-deleted-later.md", "ld", "LD1"),
            ("local-gone.md", "g", "G1"),
            ("remote-deleted-twice.md", "r", "R1"),
        ]);
        let remote_list = vec![
            remote("edited-local.md", Some("E1")),
            remote("both-edit.md", Some("B2")),
            remote("keep.md", Some("K1")),
            remote("unchanged-both.md", Some("X1")),
            remote("local-gone.md", Some("G2")),
            remote("new-remote.md", Some("N")),
            remote("local-deleted-later.md", Some("LD1")),
        ];
        let labels: Vec<String> = plan_sync(&local, &st, &remote_list)
            .iter()
            .map(label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "up:edited-local.md",
                "up:new-local.md",
                "down:local-gone.md",
                "down:new-remote.md",
                "merge:both-edit.md",
                "delremote:local-deleted-later.md",
                "dellocal:remote-gone.md",
            ]
        );
    }

    #[test]
    fn plan_order_uploads_small_first_then_downloads_merges_deletes() {
        let mut local = snapshot(&[("big.md", "1"), ("small.md", "2"), ("m.md", "mh2")]);
        if let Some(e) = local.entries.get_mut("big.md") {
            e.size = 900;
        }
        if let Some(e) = local.entries.get_mut("small.md") {
            e.size = 10;
        }
        let st = state_with(&[("m.md", "mh", "M1"), ("gone.md", "g", "G1")]);
        let remote_list = vec![
            remote("dl.md", Some("D")),
            remote("m.md", Some("M2")),
            remote("gone.md", Some("G1")),
            remote("bin.png", Some("P")),
        ];
        let labels: Vec<String> = plan_sync(&local, &st, &remote_list)
            .iter()
            .map(label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "up:small.md",
                "up:big.md",
                "down:bin.png",
                "down:dl.md",
                "merge:m.md",
                "delremote:gone.md",
            ]
        );
    }

    #[test]
    fn plan_leaves_unreadable_paths_alone() {
        let mut local = LocalSnapshot::default();
        local.unreadable.insert("locked.md".into());
        local.unreadable.insert("broken/".into());
        let st = state_with(&[("locked.md", "h", "E1"), ("broken/x.md", "h", "E1")]);
        assert!(plan_sync(&local, &st, &[]).is_empty());
    }

    #[test]
    fn plan_skips_paths_rejected_by_filter() {
        let local = snapshot(&[("ignored.md", "h")]);
        let st = SyncState::default();
        assert_eq!(
            plan_sync_filtered(&local, &st, &[], &|_| false)
                .iter()
                .map(label)
                .collect::<Vec<String>>(),
            vec!["up:ignored.md".to_owned()]
        );
        assert!(plan_sync_filtered(&local, &st, &[], &|p| p == "ignored.md").is_empty());
    }

    #[test]
    fn plan_ignores_remote_directories() {
        let dir = RemoteEntry {
            is_dir: true,
            ..remote("folder/", None)
        };
        assert!(plan_sync(&snapshot(&[]), &SyncState::default(), &[dir]).is_empty());
    }

    #[test]
    fn plan_is_deterministic() {
        let local = snapshot(&[("a.md", "1"), ("b.md", "2")]);
        let st = SyncState::default();
        let remote_list = vec![remote("c.md", Some("x")), remote("d.md", Some("y"))];
        assert_eq!(
            plan_sync(&local, &st, &remote_list),
            plan_sync(&local, &st, &remote_list)
        );
    }

    #[test]
    fn plan_after_successful_sync_is_empty() {
        // Both sides hold "1" and the base records exactly that; nothing left to do.
        let local = snapshot(&[("a.md", "1")]);
        let st = state_with(&[("a.md", "1", "E1")]);
        let remote_list = vec![remote("a.md", Some("E1"))];
        assert!(plan_sync(&local, &st, &remote_list).is_empty());
    }
}
