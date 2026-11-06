//! Blocking history work. Every function here runs on the background executor (see
//! `jobs::spawn_blocking`); none of them touches the UI state.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use jiyue_history::{
    DiffLine, EntryId, History, HistoryError, Kind, Retention, SnapshotMeta, SnapshotOutcome,
    diff_text,
};

use super::pure::{self, LIST_LIMIT};

/// The open history store, shared by the background jobs.
pub type Store = Arc<Mutex<History>>;

/// Why the store could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// Another process (a second window, the MCP server) holds the history directory.
    Locked,
    Other(String),
}

/// One version as listed (newest first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: EntryId,
    pub time_ms: i64,
    pub size: u64,
    pub kind: Kind,
    pub label: Option<String>,
    /// Lines added and removed compared with the next older version. `None` for binary content
    /// or when the older version cannot be read.
    pub counts: Option<(usize, usize)>,
}

/// A version opened for viewing: its text and the diff from the current text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub text: String,
    pub diff: Vec<DiffLine>,
}

/// Result of a snapshot job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapOutcome {
    Created,
    /// Identical to the newest version, or too large to keep.
    Unchanged,
    /// Too soon after the previous automatic version; the milliseconds left.
    Throttled(i64),
    Failed(String),
}

/// `<vault>/.jiyue/history`.
pub fn history_dir(root: &Path) -> PathBuf {
    root.join(".jiyue").join("history")
}

fn lock(store: &Store) -> MutexGuard<'_, History> {
    // A panic while holding the lock leaves the in-memory state usable; keep serving it.
    store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Opens the store of the vault at `root`, applies the default retention policy and makes the
/// result durable.
pub fn open(root: &Path) -> Result<Store, OpenError> {
    match History::open(&history_dir(root)) {
        Ok(mut history) => {
            let _ = history.apply_retention(&Retention::default(), jiyue_history::system_now_ms());
            let _ = history.flush();
            Ok(Arc::new(Mutex::new(history)))
        }
        Err(HistoryError::Locked) => Err(OpenError::Locked),
        Err(e) => Err(OpenError::Other(e.to_string())),
    }
}

fn write(
    history: &mut History,
    path: &str,
    text: &str,
    meta: SnapshotMeta,
    now_ms: i64,
) -> SnapOutcome {
    match history.snapshot_outcome(path, text.as_bytes(), meta, now_ms) {
        Ok(SnapshotOutcome::Created(_)) => {
            // Make the new version durable now; a crash must not lose what was just saved.
            let _ = history.flush();
            SnapOutcome::Created
        }
        Ok(SnapshotOutcome::Unchanged | SnapshotOutcome::TooLarge { .. }) => SnapOutcome::Unchanged,
        Err(e) => SnapOutcome::Failed(e.to_string()),
    }
}

/// An automatic version after a save. Waits for the interval of the note's previous version.
pub fn snapshot_auto(store: &Store, path: &str, text: &str, now_ms: i64) -> SnapOutcome {
    let mut history = lock(store);
    if let Some(left) = pure::throttle_remaining(history.latest_time(path), now_ms) {
        return SnapOutcome::Throttled(left);
    }
    write(&mut history, path, text, SnapshotMeta::auto(), now_ms)
}

/// A version without the interval check (a change made outside the app).
pub fn snapshot_now(store: &Store, path: &str, text: &str, now_ms: i64) -> SnapOutcome {
    write(&mut lock(store), path, text, SnapshotMeta::auto(), now_ms)
}

/// Records the disk text of the changed notes that differs from the text the app last saved
/// (`prev` is that fingerprint; `None` when unknown). Returns, per recorded change, the path, the
/// fingerprint of the disk text and the outcome. Our own writes match `prev` and are skipped.
pub fn external_changes(
    store: &Store,
    root: &Path,
    changed: &[(String, Option<u64>)],
    now_ms: i64,
) -> Vec<(String, u64, SnapOutcome)> {
    let mut out = Vec::new();
    for (path, prev) in changed {
        let Ok(text) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        let hash = pure::text_hash(&text);
        if *prev == Some(hash) {
            continue;
        }
        let outcome = snapshot_now(store, path, &text, now_ms);
        out.push((path.clone(), hash, outcome));
    }
    out
}

fn text_of(bytes: Vec<u8>) -> Option<String> {
    String::from_utf8(bytes).ok()
}

/// The newest versions of `path` (at most `LIST_LIMIT`), with their change counts.
pub fn list_rows(store: &Store, path: &str) -> Result<Vec<Row>, String> {
    let history = lock(store);
    let entries = history.list(path);
    // One extra text (the version just older than the last listed one) gives its count.
    let texts: Vec<Option<String>> = entries
        .iter()
        .take(LIST_LIMIT + 1)
        .map(|e| history.read(&e.id).ok().and_then(text_of))
        .collect();
    let rows = entries
        .iter()
        .take(LIST_LIMIT)
        .enumerate()
        .map(|(i, entry)| {
            let counts = match texts.get(i) {
                Some(Some(new)) => {
                    // The oldest version is compared with nothing.
                    let old: Option<&str> = match texts.get(i + 1) {
                        Some(Some(old)) => Some(old.as_str()),
                        Some(None) => None,
                        None if entries.len() <= LIST_LIMIT => Some(""),
                        None => None,
                    };
                    old.map(|old| pure::line_counts(&diff_text(old, new)))
                }
                _ => None,
            };
            Row {
                id: entry.id,
                time_ms: entry.time_ms,
                size: entry.size,
                kind: entry.kind,
                label: entry.label.clone(),
                counts,
            }
        })
        .collect();
    Ok(rows)
}

/// Reads one version and diffs it against `current` (the text now in the editor). In the diff,
/// `Insert` lines come from the version and `Delete` lines from the current text.
pub fn load_version(store: &Store, id: EntryId, current: &str) -> Result<Loaded, String> {
    let history = lock(store);
    let bytes = history.read(&id).map_err(|e| e.to_string())?;
    let text = text_of(bytes).ok_or_else(|| "这个版本不是文本".to_owned())?;
    let diff = diff_text(current, &text);
    Ok(Loaded { text, diff })
}

/// Restores one version. The current text is kept first as a `BeforeRestore` version. Returns
/// the restored text; the caller puts it in the editor.
pub fn restore(store: &Store, id: EntryId, current: &str, now_ms: i64) -> Result<String, String> {
    let mut history = lock(store);
    let bytes = history
        .restore_bytes(&id, Some(current.as_bytes()), now_ms)
        .map_err(|e| e.to_string())?;
    let _ = history.flush();
    text_of(bytes).ok_or_else(|| "这个版本不是文本".to_owned())
}

/// The history of a note follows it to its new path (a rename or a move).
pub fn follow_rename(store: &Store, old: &str, new: &str) -> Result<(), String> {
    let mut history = lock(store);
    history.note_renamed(old, new).map_err(|e| e.to_string())?;
    let _ = history.flush();
    Ok(())
}

/// The note was deleted: its last text stays available as a `Deleted` version.
pub fn record_deleted(store: &Store, path: &str, text: &str, now_ms: i64) -> Result<(), String> {
    let mut history = lock(store);
    history
        .note_deleted(path, text.as_bytes(), now_ms)
        .map_err(|e| e.to_string())?;
    let _ = history.flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("jiyue-history-ui-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn auto_snapshots_are_throttled_and_listed_with_counts() {
        let root = temp_root("list");
        let store = open(&root).expect("open");
        let t0 = 1_800_000_000_000;
        assert_eq!(
            snapshot_auto(&store, "a.md", "one\n", t0),
            SnapOutcome::Created
        );
        match snapshot_auto(&store, "a.md", "one\ntwo\n", t0 + 1_000) {
            SnapOutcome::Throttled(left) => assert!(left > 0),
            other => panic!("expected throttle, got {other:?}"),
        }
        let later = t0 + jiyue_history::DEFAULT_MIN_INTERVAL_MS + 1;
        assert_eq!(
            snapshot_auto(&store, "a.md", "one\ntwo\n", later),
            SnapOutcome::Created
        );

        let rows = list_rows(&store, "a.md").expect("rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].counts, Some((1, 0)), "newest adds one line");
        assert_eq!(
            rows[1].counts,
            Some((1, 0)),
            "oldest counts against nothing"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_returns_the_old_text_and_keeps_the_current_one() {
        let root = temp_root("restore");
        let store = open(&root).expect("open");
        let t0 = 1_800_000_000_000;
        snapshot_now(&store, "b.md", "old text\n", t0);
        let rows = list_rows(&store, "b.md").expect("rows");
        let id = rows[0].id;
        let restored = restore(&store, id, "new text\n", t0 + 10).expect("restore");
        assert_eq!(restored, "old text\n");
        let rows = list_rows(&store, "b.md").expect("rows");
        assert!(
            rows.iter().any(|r| r.kind == Kind::BeforeRestore),
            "current text is kept before the restore"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_second_open_reports_the_lock() {
        let root = temp_root("lock");
        let first = open(&root).expect("first open");
        assert_eq!(open(&root).err(), Some(OpenError::Locked));
        drop(first);
        assert!(open(&root).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_rename_moves_the_history_and_a_delete_keeps_the_last_text() {
        let root = temp_root("follow");
        let store = open(&root).expect("open");
        let t0 = 1_800_000_000_000;
        snapshot_now(&store, "old.md", "first\n", t0);
        follow_rename(&store, "old.md", "new.md").expect("rename");
        assert!(list_rows(&store, "old.md").expect("old").is_empty());
        assert_eq!(list_rows(&store, "new.md").expect("new").len(), 1);

        record_deleted(&store, "new.md", "last words\n", t0 + 10).expect("delete");
        let rows = list_rows(&store, "new.md").expect("rows");
        assert_eq!(rows[0].kind, Kind::Deleted);
        let restored = restore(&store, rows[0].id, "", t0 + 20).expect("read deleted text");
        assert_eq!(restored, "last words\n");
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }
}
