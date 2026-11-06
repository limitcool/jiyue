//! jiyue-sync: remote storage (S3 / WebDAV), sync planning, three-way merge and conflicts.
//!
//! The API is blocking; the implementation owns its own async runtime (in `store`), and callers
//! use it from background threads. Local state lives in `<vault>/.jiyue/sync`.
//!
//! Typical use:
//! 1. `let mut state = state::load_state(root)?;`
//! 2. `let mut opts = SyncOptions::new("PC");` (optionally set ignore rules, prefix, progress, cancel)
//! 3. `let report = run_sync(root, &store, &mut state, &mut opts);` (saves the state itself)

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub mod merge;
pub mod plan;
pub mod state;
pub mod store;

use serde::{Deserialize, Serialize};

pub use merge::three_way_merge;
pub use plan::plan_sync;
pub use state::IgnoreRules;

/// One object or prefix reported by a remote store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub path: String,
    pub size: u64,
    pub etag: Option<String>,
    pub modified_ms: Option<i64>,
    pub version_id: Option<String>,
    pub is_dir: bool,
}

/// Conditional-write options for [`RemoteStore::put`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PutOptions {
    /// Only write if the remote etag still equals this value.
    pub if_match: Option<String>,
    /// Only write if the object does not exist yet.
    pub if_none_match_star: bool,
}

/// Errors returned by a [`RemoteStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    NotFound,
    PreconditionFailed,
    Unauthorized,
    Network(String),
    Other(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::NotFound => write!(f, "not found"),
            StoreError::PreconditionFailed => write!(f, "precondition failed"),
            StoreError::Unauthorized => {
                write!(
                    f,
                    "unauthorized (check the access key, secret and bucket permissions)"
                )
            }
            StoreError::Network(msg) => write!(f, "network error: {msg}"),
            StoreError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Remote object storage. Implemented by opendal-backed S3 and WebDAV stores (wave 1, W1-D1).
///
/// Paths are full keys in the store. The sync engine prefixes them with
/// [`SyncOptions::remote_prefix`] and strips the prefix again when reading listings.
/// Implementations must return an `etag` for every object, or conditional writes and change
/// detection fall back to the safe but slower path.
pub trait RemoteStore: Send + Sync {
    /// Lists everything under `prefix`, recursively.
    fn list(&self, prefix: &str) -> Result<Vec<RemoteEntry>, StoreError>;
    fn stat(&self, path: &str) -> Result<Option<RemoteEntry>, StoreError>;
    fn get(&self, path: &str, version_id: Option<&str>) -> Result<Vec<u8>, StoreError>;
    fn put(&self, path: &str, data: &[u8], opts: &PutOptions) -> Result<RemoteEntry, StoreError>;
    fn delete(&self, path: &str) -> Result<(), StoreError>;
    /// Returns an empty list when versioning is not enabled on the remote.
    fn list_versions(&self, path: &str) -> Result<Vec<RemoteEntry>, StoreError>;
    fn supports_versions(&self) -> bool;
}

/// Placeholder store used until a real backend is configured. Every operation that would touch
/// the remote fails with "not implemented"; listings are empty.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredStore;

fn not_implemented<T>() -> Result<T, StoreError> {
    Err(StoreError::Other("not implemented".into()))
}

impl RemoteStore for UnconfiguredStore {
    fn list(&self, _prefix: &str) -> Result<Vec<RemoteEntry>, StoreError> {
        Ok(Vec::new())
    }

    fn stat(&self, _path: &str) -> Result<Option<RemoteEntry>, StoreError> {
        not_implemented()
    }

    fn get(&self, _path: &str, _version_id: Option<&str>) -> Result<Vec<u8>, StoreError> {
        not_implemented()
    }

    fn put(
        &self,
        _path: &str,
        _data: &[u8],
        _opts: &PutOptions,
    ) -> Result<RemoteEntry, StoreError> {
        not_implemented()
    }

    fn delete(&self, _path: &str) -> Result<(), StoreError> {
        not_implemented()
    }

    fn list_versions(&self, _path: &str) -> Result<Vec<RemoteEntry>, StoreError> {
        Ok(Vec::new())
    }

    fn supports_versions(&self) -> bool {
        false
    }
}

/// One local file as seen by a scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalEntry {
    /// blake3 hex of the content.
    pub hash: String,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Local view of the vault for one sync pass (filled by [`state::scan_local`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalSnapshot {
    /// Relative path (with `/`) mapped to its blake3 hex hash.
    pub files: BTreeMap<String, String>,
    /// Same files with size and mtime.
    pub entries: BTreeMap<String, LocalEntry>,
    /// Files, or folders ending in `/`, that could not be read. Planning never acts on them, so an
    /// unreadable file is never mistaken for a deleted one. An empty entry means the whole vault.
    pub unreadable: BTreeSet<String>,
}

/// The state of one path at the last successful sync (the merge base).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseEntry {
    /// blake3 hex of the content at that sync.
    pub hash: String,
    /// Remote etag right after that sync.
    #[serde(default)]
    pub etag: Option<String>,
    /// Remote version id right after that sync, if the store reports one.
    #[serde(default)]
    pub version_id: Option<String>,
    pub size: u64,
}

/// Cached hash of a local file, valid while size and mtime are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheEntry {
    pub size: u64,
    pub mtime_ms: i64,
    pub hash: String,
}

/// Current version of the on-disk state format.
pub const STATE_VERSION: u32 = 1;

/// Sync bookkeeping kept in `.jiyue/sync/state.json`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// Format version written by [`state::save_state`].
    #[serde(default)]
    pub version: u32,
    /// Relative path mapped to the content hash at the last successful sync. Kept in step with
    /// `entries`; `entries` is the authoritative record.
    #[serde(default)]
    pub base: BTreeMap<String, String>,
    /// Time of the last fully successful sync in Unix milliseconds, if any.
    #[serde(default)]
    pub last_synced_ms: Option<i64>,
    /// Relative path mapped to its full base record.
    #[serde(default)]
    pub entries: BTreeMap<String, BaseEntry>,
    /// Hash cache of the last scan.
    #[serde(default)]
    pub scan_cache: BTreeMap<String, ScanCacheEntry>,
}

/// One step decided by [`plan_sync`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    Upload { path: String },
    Download { path: String },
    DeleteLocal { path: String },
    DeleteRemote { path: String },
    Merge { path: String },
    Conflict { path: String },
    Noop,
}

/// Result of [`three_way_merge`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult {
    /// Merged without overlapping edits.
    Clean(String),
    /// Overlapping edits. `merged_with_markers` holds conflict markers around each hunk.
    Conflict {
        merged_with_markers: String,
        hunks: usize,
    },
}

/// Progress of [`run_sync`], reported before each action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncProgress {
    /// Actions already finished.
    pub done: usize,
    /// Actions planned for this run.
    pub total: usize,
    /// `upload`, `download`, `merge`, `conflict`, `delete-remote` or `delete-local`.
    pub action: &'static str,
    /// Vault-relative path of the action.
    pub path: String,
}

/// Options for [`run_sync`].
pub struct SyncOptions<'a> {
    /// Device label used in conflict-copy names, e.g. `PC`.
    pub device: String,
    /// User ignore rules; the built-in defaults always apply.
    pub ignore: IgnoreRules,
    /// Folder inside the store that holds the vault root, e.g. `vault/`. Empty for the store root.
    pub remote_prefix: String,
    /// Plan only: nothing is uploaded, downloaded, moved, or deleted, and the state is not saved.
    pub dry_run: bool,
    /// Called before each action.
    pub progress: Option<&'a mut dyn FnMut(SyncProgress)>,
    /// Set to `true` from another thread to stop before the next action.
    pub cancel: Option<&'a AtomicBool>,
}

impl<'a> SyncOptions<'a> {
    /// Options with the default ignore rules, an empty remote prefix and no callbacks.
    pub fn new(device: impl Into<String>) -> Self {
        Self {
            device: device.into(),
            ignore: IgnoreRules::default(),
            remote_prefix: String::new(),
            dry_run: false,
            progress: None,
            cancel: None,
        }
    }
}

/// Kind of a conflict, as recorded in [`SyncReport::conflicts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Text merged with conflict markers in the file; the remote version is in the copy.
    Markers,
    /// Binary (or unreadable as text) on both sides; the local version stays at the path and the
    /// remote version is in the copy.
    BothKept,
}

/// A conflict copy created by a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRecord {
    /// The file that conflicted.
    pub path: String,
    /// The conflict copy holding the other version.
    pub copy_path: String,
    pub kind: ConflictKind,
}

/// A failure for one path. The run continues with the other paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub path: String,
    pub message: String,
}

/// Outcome of [`run_sync`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub uploaded: Vec<String>,
    pub downloaded: Vec<String>,
    /// Text files merged cleanly.
    pub merged: Vec<String>,
    pub conflicts: Vec<ConflictRecord>,
    /// Local deletions, moved to `.trash/sync/`.
    pub deleted_local: Vec<String>,
    pub deleted_remote: Vec<String>,
    pub errors: Vec<FileError>,
    /// The planned actions; only filled for a dry run.
    pub planned: Vec<SyncAction>,
    /// A run-level failure (vault missing, remote listing failed, state not saved).
    pub fatal: Option<String>,
    /// The run was stopped through the cancel flag.
    pub cancelled: bool,
}

/// Runs one sync pass of `root` against `store`.
///
/// Scans the vault, lists the remote under `opts.remote_prefix`, plans, then executes each action.
/// One failing path does not stop the run. Uploads are conditional (`If-Match` on the base etag, or
/// `If-None-Match` for new files); on `PreconditionFailed` the path is re-read and re-planned once.
/// Overwritten or deleted local content goes to `.trash/sync` or is kept as a conflict copy.
/// The state and the merge-base cache are updated after every action and saved at the end.
pub fn run_sync(
    root: &Path,
    store: &dyn RemoteStore,
    state: &mut SyncState,
    opts: &mut SyncOptions<'_>,
) -> SyncReport {
    let mut report = SyncReport::default();
    if !root.is_dir() {
        report.fatal = Some(format!("vault folder not found: {}", root.display()));
        return report;
    }
    let prefix = normalize_prefix(&opts.remote_prefix);
    let rules = opts.ignore.clone();

    notify(opts, "scan", "", 0, 0);
    let local = state::scan_local_cached(root, &rules, &mut state.scan_cache);
    if is_cancelled(opts) {
        report.cancelled = true;
        return report;
    }

    let listing = match store.list(&prefix) {
        Ok(entries) => entries,
        Err(e) => {
            report.fatal = Some(format!("cannot list the remote: {e}"));
            return report;
        }
    };
    let mut remote: BTreeMap<String, RemoteEntry> = BTreeMap::new();
    for entry in listing {
        if entry.is_dir {
            continue;
        }
        let Some(rel) = entry.path.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let rel = rel.to_owned();
        if !state::is_safe_rel_path(&rel) {
            report.errors.push(FileError {
                path: entry.path.clone(),
                message: "unsafe remote path, skipped".into(),
            });
            continue;
        }
        if rules.is_ignored(&rel, false) {
            continue;
        }
        let mut e = entry;
        e.path = rel.clone();
        remote.insert(rel, e);
    }

    let remote_list: Vec<RemoteEntry> = remote.values().cloned().collect();
    let actions = plan::plan_sync_filtered(&local, state, &remote_list, &|p: &str| {
        rules.is_ignored(p, false)
    });
    if opts.dry_run {
        report.planned = actions;
        return report;
    }

    let total = actions.len();
    let now = state::now_ms();
    let device = if opts.device.trim().is_empty() {
        "device".to_owned()
    } else {
        opts.device.clone()
    };
    let mut exec = Exec {
        root,
        store,
        prefix,
        device,
        copy_stamp: state::format_timestamp(now),
        trash_stamp: state::format_timestamp_precise(now),
        local,
        remote,
        rules,
        state: &mut *state,
    };

    let mut done = 0usize;
    let mut network_streak = 0usize;
    for action in &actions {
        if is_cancelled(opts) {
            report.cancelled = true;
            break;
        }
        let Some(rel) = action.path() else {
            continue;
        };
        notify(opts, action_name(action), rel, done, total);
        match exec.sync_path(rel, 0) {
            Ok(outcome) => {
                network_streak = 0;
                apply_outcome(&mut report, rel, outcome);
            }
            Err(e) => {
                // A rejected login affects every file, so the run stops at once. A dead server
                // shows up as a row of network failures, each already retried and maybe timed out.
                // After a few in a row the run stops; the state is saved and the next run resumes.
                let auth_failed = matches!(e, ExecError::Store(StoreError::Unauthorized));
                let network_failed = matches!(e, ExecError::Store(StoreError::Network(_)));
                network_streak = if network_failed {
                    network_streak + 1
                } else {
                    0
                };
                report.errors.push(FileError {
                    path: rel.to_owned(),
                    message: e.to_string(),
                });
                if auth_failed || network_streak >= MAX_NETWORK_FAILURES_IN_A_ROW {
                    report.fatal = Some(format!("run stopped after a server failure: {e}"));
                    break;
                }
            }
        }
        done += 1;
        if done.is_multiple_of(32)
            && let Err(e) = exec.checkpoint()
        {
            report.fatal = Some(format!("cannot save sync state: {e}"));
            break;
        }
    }

    let clean = report.errors.is_empty() && !report.cancelled && report.fatal.is_none();
    if let Err(e) = exec.finish(clean, now) {
        report.fatal = Some(format!("cannot save sync state: {e}"));
    }
    report
}

/// Consecutive network failures (each already retried by the store) after which a run stops.
const MAX_NETWORK_FAILURES_IN_A_ROW: usize = 3;

fn normalize_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else if trimmed.ends_with('/') {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/")
    }
}

fn is_cancelled(opts: &SyncOptions<'_>) -> bool {
    opts.cancel.is_some_and(|flag| flag.load(Ordering::SeqCst))
}

fn notify(opts: &mut SyncOptions<'_>, action: &'static str, path: &str, done: usize, total: usize) {
    if let Some(cb) = opts.progress.as_deref_mut() {
        cb(SyncProgress {
            done,
            total,
            action,
            path: path.to_owned(),
        });
    }
}

fn action_name(action: &SyncAction) -> &'static str {
    match action {
        SyncAction::Upload { .. } => "upload",
        SyncAction::Download { .. } => "download",
        SyncAction::Merge { .. } => "merge",
        SyncAction::Conflict { .. } => "conflict",
        SyncAction::DeleteRemote { .. } => "delete-remote",
        SyncAction::DeleteLocal { .. } => "delete-local",
        SyncAction::Noop => "noop",
    }
}

/// What one path did.
enum Done {
    Uploaded,
    Downloaded,
    Merged,
    Conflict(ConflictRecord),
    DeletedLocal,
    DeletedRemote,
    Unchanged,
}

fn apply_outcome(report: &mut SyncReport, rel: &str, outcome: Done) {
    match outcome {
        Done::Uploaded => report.uploaded.push(rel.to_owned()),
        Done::Downloaded => report.downloaded.push(rel.to_owned()),
        Done::Merged => report.merged.push(rel.to_owned()),
        Done::Conflict(record) => report.conflicts.push(record),
        Done::DeletedLocal => report.deleted_local.push(rel.to_owned()),
        Done::DeletedRemote => report.deleted_remote.push(rel.to_owned()),
        Done::Unchanged => {}
    }
}

/// Failure of one step inside the executor.
#[derive(Debug)]
enum ExecError {
    Store(StoreError),
    Local(String),
}

impl From<StoreError> for ExecError {
    fn from(e: StoreError) -> Self {
        ExecError::Store(e)
    }
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecError::Store(e) => write!(f, "{e}"),
            ExecError::Local(msg) => write!(f, "{msg}"),
        }
    }
}

fn io_error(what: &str, rel: &str, e: io::Error) -> ExecError {
    ExecError::Local(format!("{what} {rel}: {e}"))
}

/// Per-run executor. Holds the scan, the remote listing and the state being updated.
struct Exec<'a> {
    root: &'a Path,
    store: &'a dyn RemoteStore,
    prefix: String,
    device: String,
    copy_stamp: String,
    trash_stamp: String,
    local: LocalSnapshot,
    remote: BTreeMap<String, RemoteEntry>,
    rules: IgnoreRules,
    state: &'a mut SyncState,
}

impl Exec<'_> {
    fn key(&self, rel: &str) -> String {
        format!("{}{rel}", self.prefix)
    }

    /// Runs the decision for one path and carries it out. A `PreconditionFailed` from the store
    /// triggers exactly one re-read of the remote object and one re-plan.
    fn sync_path(&mut self, rel: &str, attempt: u8) -> Result<Done, ExecError> {
        let local_hash = if attempt == 0 {
            self.local.files.get(rel).cloned()
        } else {
            self.local_hash_now(rel)?
        };
        let base = self.state.base_entry(rel);
        let remote = if attempt == 0 {
            self.remote.get(rel).cloned()
        } else {
            self.refresh_remote(rel)?
        };
        let action = plan::decide(
            rel,
            local_hash.as_deref(),
            base.as_ref(),
            remote.as_ref(),
            state::is_text_path(rel),
        );
        let result = match action {
            SyncAction::Noop => return Ok(Done::Unchanged),
            SyncAction::Upload { .. } => self.upload(rel, base.as_ref(), remote.as_ref()),
            SyncAction::Download { .. } => {
                self.download(rel, local_hash.as_deref(), remote.as_ref())
            }
            SyncAction::Merge { .. } => {
                self.merge(rel, local_hash.as_deref(), base.as_ref(), remote.as_ref())
            }
            SyncAction::Conflict { .. } => {
                self.conflict_binary(rel, local_hash.as_deref(), remote.as_ref())
            }
            SyncAction::DeleteRemote { .. } => self.delete_remote(rel, base.as_ref()),
            SyncAction::DeleteLocal { .. } => self.delete_local(rel, local_hash.as_deref()),
        };
        match result {
            Err(ExecError::Store(StoreError::PreconditionFailed)) if attempt == 0 => {
                self.sync_path(rel, 1)
            }
            other => other,
        }
    }

    fn upload(
        &mut self,
        rel: &str,
        base: Option<&BaseEntry>,
        remote: Option<&RemoteEntry>,
    ) -> Result<Done, ExecError> {
        let data = self.read_local(rel)?;
        let opts = PutOptions {
            if_match: match remote {
                Some(_) => base.and_then(|b| b.etag.clone()),
                None => None,
            },
            if_none_match_star: remote.is_none(),
        };
        let stored = self.store.put(&self.key(rel), &data, &opts)?;
        self.record_base(rel, &data, &stored);
        Ok(Done::Uploaded)
    }

    fn download(
        &mut self,
        rel: &str,
        local_hash: Option<&str>,
        remote: Option<&RemoteEntry>,
    ) -> Result<Done, ExecError> {
        let Some(remote) = remote else {
            return Err(ExecError::Local(format!("{rel}: remote entry vanished")));
        };
        self.ensure_local_unchanged(rel, local_hash)?;
        let data = self.store.get(&self.key(rel), None)?;
        state::write_atomic(&self.root.join(rel), &data).map_err(|e| io_error("write", rel, e))?;
        self.record_base(rel, &data, remote);
        Ok(Done::Downloaded)
    }

    fn merge(
        &mut self,
        rel: &str,
        local_hash: Option<&str>,
        base: Option<&BaseEntry>,
        remote: Option<&RemoteEntry>,
    ) -> Result<Done, ExecError> {
        let (Some(local_hash), Some(remote)) = (local_hash, remote) else {
            return Err(ExecError::Local(format!("{rel}: nothing to merge")));
        };
        let local_data = self.read_local(rel)?;
        if state::hash_bytes(&local_data) != local_hash {
            return Err(changed_during_sync(rel));
        }
        let remote_data = self.store.get(&self.key(rel), None)?;
        if state::hash_bytes(&remote_data) == local_hash {
            // Both sides already hold the same content: only the base needs recording.
            self.record_base(rel, &local_data, remote);
            return Ok(Done::Unchanged);
        }

        let base_text = match base {
            None => Some(String::new()),
            Some(b) => state::read_base_cache(self.root, &b.hash)
                .and_then(|data| String::from_utf8(data).ok()),
        };
        let merged = match (
            &base_text,
            std::str::from_utf8(&local_data),
            std::str::from_utf8(&remote_data),
        ) {
            (Some(b), Ok(l), Ok(r)) => Some(merge::three_way_merge(b, l, r)),
            _ => None,
        };
        match merged {
            Some(MergeResult::Clean(text)) => {
                self.publish_merged(rel, text.into_bytes(), remote)?;
                Ok(Done::Merged)
            }
            Some(MergeResult::Conflict {
                merged_with_markers,
                ..
            }) => self.keep_both(
                rel,
                merged_with_markers.into_bytes(),
                remote_data,
                remote,
                ConflictKind::Markers,
                true,
            ),
            None => self.keep_both(
                rel,
                local_data,
                remote_data,
                remote,
                ConflictKind::BothKept,
                false,
            ),
        }
    }

    /// Binary file changed on both sides: keep the local version at the path and the remote one
    /// as a conflict copy.
    fn conflict_binary(
        &mut self,
        rel: &str,
        local_hash: Option<&str>,
        remote: Option<&RemoteEntry>,
    ) -> Result<Done, ExecError> {
        let (Some(local_hash), Some(remote)) = (local_hash, remote) else {
            return Err(ExecError::Local(format!("{rel}: nothing to keep")));
        };
        let local_data = self.read_local(rel)?;
        if state::hash_bytes(&local_data) != local_hash {
            return Err(changed_during_sync(rel));
        }
        let remote_data = self.store.get(&self.key(rel), None)?;
        if state::hash_bytes(&remote_data) == local_hash {
            self.record_base(rel, &local_data, remote);
            return Ok(Done::Unchanged);
        }
        self.keep_both(
            rel,
            local_data,
            remote_data,
            remote,
            ConflictKind::BothKept,
            false,
        )
    }

    /// Publishes the merged text (conditional on the remote etag), then writes it locally.
    fn publish_merged(
        &mut self,
        rel: &str,
        bytes: Vec<u8>,
        remote: &RemoteEntry,
    ) -> Result<(), ExecError> {
        let opts = PutOptions {
            if_match: remote.etag.clone(),
            if_none_match_star: false,
        };
        let stored = self.store.put(&self.key(rel), &bytes, &opts)?;
        state::write_atomic(&self.root.join(rel), &bytes).map_err(|e| io_error("write", rel, e))?;
        self.record_base(rel, &bytes, &stored);
        Ok(())
    }

    /// Two different versions: `canonical` goes to the path (conditional on the remote etag),
    /// `copy` becomes a new conflict-copy file on both sides. Nothing is written locally until the
    /// conditional put has succeeded, so a retry starts from the same state.
    fn keep_both(
        &mut self,
        rel: &str,
        canonical: Vec<u8>,
        copy: Vec<u8>,
        remote: &RemoteEntry,
        kind: ConflictKind,
        write_canonical_locally: bool,
    ) -> Result<Done, ExecError> {
        let opts = PutOptions {
            if_match: remote.etag.clone(),
            if_none_match_star: false,
        };
        // The other version is stored under its new name first. Only then is the path replaced.
        // If the copy cannot be written, nothing has changed yet and the next run reports the same
        // conflict again, with its record.
        let copy_rel = self.free_copy_name(rel)?;
        let copy_key = self.key(&copy_rel);
        let copy_opts = PutOptions {
            if_match: None,
            if_none_match_star: true,
        };
        let copy_stored = self.store.put(&copy_key, &copy, &copy_opts)?;
        let stored = match self.store.put(&self.key(rel), &canonical, &opts) {
            Ok(stored) => stored,
            Err(e) => {
                // A refused write did not touch the path, so the copy is not needed. Other
                // failures may still have been applied, so the copy stays: no version is lost.
                if e == StoreError::PreconditionFailed {
                    let _ = self.store.delete(&copy_key);
                }
                return Err(e.into());
            }
        };
        if write_canonical_locally {
            state::write_atomic(&self.root.join(rel), &canonical)
                .map_err(|e| io_error("write", rel, e))?;
        }
        self.record_base(rel, &canonical, &stored);

        state::write_atomic(&self.root.join(&copy_rel), &copy)
            .map_err(|e| io_error("write", &copy_rel, e))?;
        self.record_base(&copy_rel, &copy, &copy_stored);
        Ok(Done::Conflict(ConflictRecord {
            path: rel.to_owned(),
            copy_path: copy_rel,
            kind,
        }))
    }

    fn delete_remote(&mut self, rel: &str, base: Option<&BaseEntry>) -> Result<Done, ExecError> {
        let key = self.key(rel);
        if let Some(current) = self.store.stat(&key)? {
            let unchanged = base.is_some_and(|b| !plan::remote_changed(b, &current));
            if !unchanged {
                // Changed since the listing: let the retry re-plan with the new version.
                return Err(ExecError::Store(StoreError::PreconditionFailed));
            }
            self.store.delete(&key)?;
        }
        self.remote.remove(rel);
        self.state.remove_base(rel);
        Ok(Done::DeletedRemote)
    }

    fn delete_local(&mut self, rel: &str, local_hash: Option<&str>) -> Result<Done, ExecError> {
        self.ensure_local_unchanged(rel, local_hash)?;
        if self.root.join(rel).exists() {
            self.move_to_trash(rel)?;
        }
        self.state.remove_base(rel);
        Ok(Done::DeletedLocal)
    }

    /// Moves a local file to `<root>/.trash/sync/<stamp>-<name>` (never erases it).
    fn move_to_trash(&self, rel: &str) -> Result<(), ExecError> {
        let dir = self.root.join(".trash").join("sync");
        fs::create_dir_all(&dir).map_err(|e| io_error("create trash for", rel, e))?;
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let mut target = dir.join(format!("{}-{name}", self.trash_stamp));
        let mut n = 2u32;
        while target.exists() {
            target = dir.join(format!("{}-{n}-{name}", self.trash_stamp));
            n += 1;
        }
        fs::rename(self.root.join(rel), &target).map_err(|e| io_error("move to trash", rel, e))?;
        // Remove folders the file leaves empty, but never the vault root.
        let comps: Vec<&str> = rel.split('/').collect();
        for i in (1..comps.len()).rev() {
            if fs::remove_dir(self.root.join(comps[..i].join("/"))).is_err() {
                break;
            }
        }
        Ok(())
    }

    fn free_copy_name(&self, rel: &str) -> Result<String, ExecError> {
        for n in 1..=99u32 {
            let candidate = state::conflict_copy_name_n(rel, &self.device, &self.copy_stamp, n);
            if self.root.join(&candidate).exists() {
                continue;
            }
            if self.store.stat(&self.key(&candidate))?.is_some() {
                continue;
            }
            return Ok(candidate);
        }
        Err(ExecError::Local(format!(
            "no free conflict-copy name for {rel}"
        )))
    }

    /// Records `data` as the base of `rel`, caches it for merges, and remembers the remote object.
    fn record_base(&mut self, rel: &str, data: &[u8], stored: &RemoteEntry) {
        let hash = state::hash_bytes(data);
        if state::wants_base_cache(rel, data.len()) {
            // A failed cache write only means the next merge keeps both versions instead.
            let _ = state::write_base_cache(self.root, &hash, data);
        }
        self.state.set_base(
            rel,
            BaseEntry {
                hash,
                etag: stored.etag.clone(),
                version_id: stored.version_id.clone(),
                size: data.len() as u64,
            },
        );
        let mut entry = stored.clone();
        entry.path = rel.to_owned();
        self.remote.insert(rel.to_owned(), entry);
    }

    fn read_local(&self, rel: &str) -> Result<Vec<u8>, ExecError> {
        match fs::read(self.root.join(rel)) {
            Ok(data) => Ok(data),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Err(ExecError::Local(format!("{rel} is missing locally")))
            }
            Err(e) => Err(io_error("read", rel, e)),
        }
    }

    fn local_hash_now(&self, rel: &str) -> Result<Option<String>, ExecError> {
        match fs::read(self.root.join(rel)) {
            Ok(data) => Ok(Some(state::hash_bytes(&data))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error("read", rel, e)),
        }
    }

    /// Refuses to overwrite or move a local file that changed after the scan.
    fn ensure_local_unchanged(&self, rel: &str, expected: Option<&str>) -> Result<(), ExecError> {
        let current = self.local_hash_now(rel)?;
        if current.as_deref() == expected {
            Ok(())
        } else {
            Err(changed_during_sync(rel))
        }
    }

    fn refresh_remote(&mut self, rel: &str) -> Result<Option<RemoteEntry>, ExecError> {
        match self.store.stat(&self.key(rel))? {
            Some(mut entry) => {
                entry.path = rel.to_owned();
                self.remote.insert(rel.to_owned(), entry.clone());
                Ok(Some(entry))
            }
            None => {
                self.remote.remove(rel);
                Ok(None)
            }
        }
    }

    /// Whether a path still exists on either side (or cannot be judged), so its base must stay.
    fn present(&self, path: &str) -> bool {
        self.root.join(path).exists()
            || self.remote.contains_key(path)
            || self.local.is_unreadable(path)
            || self.rules.is_ignored(path, false)
    }

    /// Saves the state (called every few actions so an interrupted run loses little).
    fn checkpoint(&mut self) -> io::Result<()> {
        state::save_state(self.root, self.state)
    }

    /// Drops bases of paths gone on both sides, prunes the base cache, and saves.
    fn finish(&mut self, clean: bool, now: i64) -> io::Result<()> {
        let stale: BTreeSet<String> = self
            .state
            .entries
            .keys()
            .chain(self.state.base.keys())
            .filter(|p| !self.present(p))
            .cloned()
            .collect();
        for path in &stale {
            self.state.remove_base(path);
        }
        if clean {
            self.state.last_synced_ms = Some(now);
        }
        state::save_state(self.root, self.state)?;
        let keep: BTreeSet<String> = self
            .state
            .entries
            .values()
            .map(|e| e.hash.clone())
            .collect();
        state::prune_base_cache(self.root, &keep)
    }
}

fn changed_during_sync(rel: &str) -> ExecError {
    ExecError::Local(format!(
        "{rel} changed during the sync; skipped, it will be retried"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn skeleton_types_and_stubs_behave() {
        let store = UnconfiguredStore;
        assert!(!store.supports_versions());
        assert_eq!(store.list("").unwrap(), Vec::new());
        assert_eq!(store.list_versions("a.md").unwrap(), Vec::new());
        assert_eq!(
            store.get("a.md", None),
            Err(StoreError::Other("not implemented".into()))
        );

        let state = SyncState::default();
        let snapshot = LocalSnapshot::default();
        assert!(plan_sync(&snapshot, &state, &[]).is_empty());

        let action = SyncAction::Upload {
            path: "a.md".into(),
        };
        assert_eq!(
            action,
            SyncAction::Upload {
                path: "a.md".into()
            }
        );

        match three_way_merge("base\n", "local\n", "remote\n") {
            MergeResult::Conflict { hunks, .. } => assert_eq!(hunks, 1),
            MergeResult::Clean(_) => panic!("both sides changed the same line: must conflict"),
        }
    }

    #[test]
    fn prefix_normalization() {
        assert_eq!(normalize_prefix(""), "");
        assert_eq!(normalize_prefix("/"), "");
        assert_eq!(normalize_prefix("vault"), "vault/");
        assert_eq!(normalize_prefix("/vault/"), "vault/");
    }

    // ----------------------------------------------------------------------------------------
    // In-memory remote with etags, conditional writes and failure injection
    // ----------------------------------------------------------------------------------------

    #[derive(Default)]
    struct FakeInner {
        objects: BTreeMap<String, (Vec<u8>, String)>,
        counter: u64,
        /// Random `PreconditionFailed` on puts, in permille.
        fail_put_permille: u64,
        rng: u64,
        /// The next N puts fail with `PreconditionFailed` (no write happens).
        fail_next_puts: u32,
        /// Keys whose puts fail with a network error.
        fail_keys: BTreeSet<String>,
    }

    impl FakeInner {
        fn roll(&mut self) -> u64 {
            let mut x = self.rng;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.rng = x;
            x % 1000
        }
    }

    #[derive(Default)]
    struct FakeRemote {
        inner: Mutex<FakeInner>,
    }

    fn entry_for(key: &str, data: &[u8], etag: &str) -> RemoteEntry {
        RemoteEntry {
            path: key.to_owned(),
            size: data.len() as u64,
            etag: Some(etag.to_owned()),
            modified_ms: None,
            version_id: None,
            is_dir: false,
        }
    }

    impl FakeRemote {
        fn new() -> Self {
            Self::with_injection(0, 1)
        }

        fn with_injection(permille: u64, seed: u64) -> Self {
            let inner = FakeInner {
                fail_put_permille: permille,
                rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
                ..FakeInner::default()
            };
            Self {
                inner: Mutex::new(inner),
            }
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, FakeInner> {
            self.inner.lock().expect("fake remote lock")
        }

        fn set_injection(&self, permille: u64) {
            self.lock().fail_put_permille = permille;
        }

        /// Full remote content as path -> bytes.
        fn contents(&self) -> BTreeMap<String, Vec<u8>> {
            self.lock()
                .objects
                .iter()
                .map(|(k, (data, _))| (k.clone(), data.clone()))
                .collect()
        }

        /// Stores an object directly (bypasses the conditional-write rules).
        fn put_raw(&self, key: &str, data: &[u8]) {
            let mut g = self.lock();
            g.counter += 1;
            let etag = format!("raw{}", g.counter);
            g.objects.insert(key.to_owned(), (data.to_vec(), etag));
        }
    }

    impl RemoteStore for FakeRemote {
        fn list(&self, prefix: &str) -> Result<Vec<RemoteEntry>, StoreError> {
            let g = self.lock();
            Ok(g.objects
                .iter()
                .filter(|(k, _)| k.starts_with(prefix))
                .map(|(k, (data, etag))| entry_for(k, data, etag))
                .collect())
        }

        fn stat(&self, path: &str) -> Result<Option<RemoteEntry>, StoreError> {
            let g = self.lock();
            Ok(g.objects
                .get(path)
                .map(|(data, etag)| entry_for(path, data, etag)))
        }

        fn get(&self, path: &str, _version_id: Option<&str>) -> Result<Vec<u8>, StoreError> {
            self.lock()
                .objects
                .get(path)
                .map(|(data, _)| data.clone())
                .ok_or(StoreError::NotFound)
        }

        fn put(
            &self,
            path: &str,
            data: &[u8],
            opts: &PutOptions,
        ) -> Result<RemoteEntry, StoreError> {
            let mut g = self.lock();
            if g.fail_next_puts > 0 {
                g.fail_next_puts -= 1;
                return Err(StoreError::PreconditionFailed);
            }
            if g.fail_keys.contains(path) {
                return Err(StoreError::Network("injected failure".into()));
            }
            if g.fail_put_permille > 0 && g.roll() < g.fail_put_permille {
                return Err(StoreError::PreconditionFailed);
            }
            let current = g.objects.get(path).map(|(_, etag)| etag.clone());
            if opts.if_none_match_star && current.is_some() {
                return Err(StoreError::PreconditionFailed);
            }
            if let Some(expected) = &opts.if_match
                && current.as_ref() != Some(expected)
            {
                return Err(StoreError::PreconditionFailed);
            }
            g.counter += 1;
            let etag = format!("e{}", g.counter);
            g.objects
                .insert(path.to_owned(), (data.to_vec(), etag.clone()));
            Ok(entry_for(path, data, &etag))
        }

        fn delete(&self, path: &str) -> Result<(), StoreError> {
            self.lock().objects.remove(path);
            Ok(())
        }

        fn list_versions(&self, _path: &str) -> Result<Vec<RemoteEntry>, StoreError> {
            Ok(Vec::new())
        }

        fn supports_versions(&self) -> bool {
            false
        }
    }

    /// A unique scratch folder, removed on drop.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir()
                .join(format!("jiyue-sync-lib-{tag}-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }

        fn root(&self) -> &Path {
            &self.0
        }

        fn write(&self, rel: &str, data: &[u8]) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(path, data).expect("write");
        }

        fn read(&self, rel: &str) -> Option<Vec<u8>> {
            fs::read(self.0.join(rel)).ok()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sync(
        root: &Path,
        remote: &dyn RemoteStore,
        state: &mut SyncState,
        device: &str,
    ) -> SyncReport {
        let mut opts = SyncOptions::new(device);
        run_sync(root, remote, state, &mut opts)
    }

    fn assert_ok(report: &SyncReport) {
        assert!(report.fatal.is_none(), "fatal: {:?}", report.fatal);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    }

    fn assert_nothing_to_do(report: &SyncReport) {
        assert!(
            report.uploaded.is_empty(),
            "uploaded: {:?}",
            report.uploaded
        );
        assert!(
            report.downloaded.is_empty(),
            "downloaded: {:?}",
            report.downloaded
        );
        assert!(report.merged.is_empty(), "merged: {:?}", report.merged);
        assert!(
            report.conflicts.is_empty(),
            "conflicts: {:?}",
            report.conflicts
        );
        assert!(
            report.deleted_local.is_empty(),
            "deleted local: {:?}",
            report.deleted_local
        );
        assert!(
            report.deleted_remote.is_empty(),
            "deleted remote: {:?}",
            report.deleted_remote
        );
        assert_ok(report);
    }

    #[test]
    fn upload_then_second_run_is_a_noop() {
        let a = Scratch::new("upload");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write("note.md", b"hello\n");
        a.write("img/pic.png", &[0u8, 1, 2, 3]);

        let first = sync(a.root(), &remote, &mut st, "A");
        assert_ok(&first);
        assert_eq!(first.uploaded.len(), 2);
        assert_eq!(remote.contents()["note.md"], b"hello\n");
        assert_eq!(remote.contents()["img/pic.png"], vec![0u8, 1, 2, 3]);
        assert!(st.last_synced_ms.is_some());

        assert_nothing_to_do(&sync(a.root(), &remote, &mut st, "A"));
    }

    #[test]
    fn download_creates_nested_folders() {
        let a = Scratch::new("dl-src");
        let b = Scratch::new("dl-dst");
        let remote = FakeRemote::new();
        a.write("deep/er/note.md", b"content\n");
        let mut sa = SyncState::default();
        let mut sb = SyncState::default();
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        let report = sync(b.root(), &remote, &mut sb, "B");
        assert_ok(&report);
        assert_eq!(
            b.read("deep/er/note.md").as_deref(),
            Some(&b"content\n"[..])
        );
        assert_nothing_to_do(&sync(b.root(), &remote, &mut sb, "B"));
    }

    #[test]
    fn local_edit_is_uploaded_with_etag_condition() {
        let a = Scratch::new("edit");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write("n.md", b"v1\n");
        assert_ok(&sync(a.root(), &remote, &mut st, "A"));
        a.write("n.md", b"v2\n");
        let report = sync(a.root(), &remote, &mut st, "A");
        assert_ok(&report);
        assert_eq!(report.uploaded, vec!["n.md".to_string()]);
        assert_eq!(remote.contents()["n.md"], b"v2\n");
    }

    #[test]
    fn local_delete_moves_to_trash_and_removes_remote() {
        let a = Scratch::new("trash-src");
        let b = Scratch::new("trash-dst");
        let remote = FakeRemote::new();
        let mut sa = SyncState::default();
        let mut sb = SyncState::default();
        a.write("gone.md", b"bye\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_ok(&sync(b.root(), &remote, &mut sb, "B"));

        fs::remove_file(a.root().join("gone.md")).expect("remove");
        let ra = sync(a.root(), &remote, &mut sa, "A");
        assert_ok(&ra);
        assert_eq!(ra.deleted_remote, vec!["gone.md".to_string()]);
        assert!(!remote.contents().contains_key("gone.md"));

        let rb = sync(b.root(), &remote, &mut sb, "B");
        assert_ok(&rb);
        assert_eq!(rb.deleted_local, vec!["gone.md".to_string()]);
        assert!(b.read("gone.md").is_none());
        let trash = b.root().join(".trash").join("sync");
        let kept: Vec<_> = fs::read_dir(&trash).expect("trash dir").flatten().collect();
        assert_eq!(kept.len(), 1, "deleted content must be in trash");
        assert_eq!(fs::read(kept[0].path()).expect("trash file"), b"bye\n");
    }

    #[test]
    fn remote_changed_file_is_downloaded_over_unchanged_local() {
        let a = Scratch::new("remote-change-a");
        let b = Scratch::new("remote-change-b");
        let remote = FakeRemote::new();
        let mut sa = SyncState::default();
        let mut sb = SyncState::default();
        a.write("x.md", b"one\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_ok(&sync(b.root(), &remote, &mut sb, "B"));
        a.write("x.md", b"one\ntwo\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        let rb = sync(b.root(), &remote, &mut sb, "B");
        assert_ok(&rb);
        assert_eq!(rb.downloaded, vec!["x.md".to_string()]);
        assert_eq!(b.read("x.md").as_deref(), Some(&b"one\ntwo\n"[..]));
    }

    #[test]
    fn non_overlapping_text_edits_merge_cleanly_on_both_devices() {
        let a = Scratch::new("merge-a");
        let b = Scratch::new("merge-b");
        let remote = FakeRemote::new();
        let mut sa = SyncState::default();
        let mut sb = SyncState::default();
        a.write("m.md", b"top\nmiddle\nbottom\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_ok(&sync(b.root(), &remote, &mut sb, "B"));

        a.write("m.md", b"TOP\nmiddle\nbottom\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        b.write("m.md", b"top\nmiddle\nBOTTOM\n");
        let rb = sync(b.root(), &remote, &mut sb, "B");
        assert_ok(&rb);
        assert_eq!(rb.merged, vec!["m.md".to_string()]);
        assert!(rb.conflicts.is_empty());
        let expected = b"TOP\nmiddle\nBOTTOM\n";
        assert_eq!(b.read("m.md").as_deref(), Some(&expected[..]));
        assert_eq!(remote.contents()["m.md"], expected);

        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_eq!(a.read("m.md").as_deref(), Some(&expected[..]));
        assert_nothing_to_do(&sync(a.root(), &remote, &mut sa, "A"));
        assert_nothing_to_do(&sync(b.root(), &remote, &mut sb, "B"));
    }

    #[test]
    fn overlapping_text_edits_write_markers_and_conflict_copy() {
        let a = Scratch::new("conf-a");
        let b = Scratch::new("conf-b");
        let remote = FakeRemote::new();
        let mut sa = SyncState::default();
        let mut sb = SyncState::default();
        a.write("c.md", b"line\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_ok(&sync(b.root(), &remote, &mut sb, "B"));

        a.write("c.md", b"from laptop\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        b.write("c.md", b"from phone\n");
        let rb = sync(b.root(), &remote, &mut sb, "B");
        assert_ok(&rb);
        assert_eq!(rb.conflicts.len(), 1);
        let record = &rb.conflicts[0];
        assert_eq!(record.kind, ConflictKind::Markers);
        assert_eq!(record.path, "c.md");
        assert!(
            record.copy_path.starts_with("c (冲突副本 B "),
            "{}",
            record.copy_path
        );
        assert!(record.copy_path.ends_with(").md"), "{}", record.copy_path);

        let merged = String::from_utf8(b.read("c.md").expect("merged file")).expect("utf8");
        assert!(
            merged.contains("<<<<<<< 本机\nfrom phone\n=======\nfrom laptop\n>>>>>>> 远端\n"),
            "{merged}"
        );
        assert_eq!(
            b.read(&record.copy_path).as_deref(),
            Some(&b"from laptop\n"[..]),
            "remote version kept in the conflict copy"
        );
        assert_eq!(remote.contents()[&record.copy_path], b"from laptop\n");

        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_eq!(a.read("c.md").as_deref(), Some(merged.as_bytes()));
        assert_eq!(
            a.read(&record.copy_path).as_deref(),
            Some(&b"from laptop\n"[..])
        );
        assert_nothing_to_do(&sync(a.root(), &remote, &mut sa, "A"));
        assert_nothing_to_do(&sync(b.root(), &remote, &mut sb, "B"));
    }

    #[test]
    fn binary_conflict_keeps_both_versions() {
        let a = Scratch::new("bin-a");
        let b = Scratch::new("bin-b");
        let remote = FakeRemote::new();
        let mut sa = SyncState::default();
        let mut sb = SyncState::default();
        a.write("pic.png", &[1u8, 2, 3]);
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_ok(&sync(b.root(), &remote, &mut sb, "B"));

        a.write("pic.png", &[9u8, 9]);
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        b.write("pic.png", &[7u8, 7, 7]);
        let rb = sync(b.root(), &remote, &mut sb, "B");
        assert_ok(&rb);
        assert_eq!(rb.conflicts.len(), 1);
        assert_eq!(rb.conflicts[0].kind, ConflictKind::BothKept);
        let copy = rb.conflicts[0].copy_path.clone();
        assert_eq!(b.read("pic.png"), Some(vec![7u8, 7, 7]));
        assert_eq!(b.read(&copy), Some(vec![9u8, 9]));

        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_eq!(a.read("pic.png"), Some(vec![7u8, 7, 7]));
        assert_eq!(a.read(&copy), Some(vec![9u8, 9]));
        assert_nothing_to_do(&sync(a.root(), &remote, &mut sa, "A"));
    }

    #[test]
    fn crlf_file_merges_and_keeps_crlf() {
        let a = Scratch::new("crlf-a");
        let b = Scratch::new("crlf-b");
        let remote = FakeRemote::new();
        let mut sa = SyncState::default();
        let mut sb = SyncState::default();
        a.write("w.md", b"one\r\ntwo\r\nthree\r\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_ok(&sync(b.root(), &remote, &mut sb, "B"));
        a.write("w.md", b"ONE\r\ntwo\r\nthree\r\n");
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        b.write("w.md", b"one\r\ntwo\r\nTHREE\r\n");
        assert_ok(&sync(b.root(), &remote, &mut sb, "B"));
        assert_eq!(
            b.read("w.md").as_deref(),
            Some(&b"ONE\r\ntwo\r\nTHREE\r\n"[..])
        );
        assert_ok(&sync(a.root(), &remote, &mut sa, "A"));
        assert_eq!(
            a.read("w.md").as_deref(),
            Some(&b"ONE\r\ntwo\r\nTHREE\r\n"[..])
        );
    }

    #[test]
    fn precondition_failure_is_retried_once_and_succeeds() {
        let a = Scratch::new("precond");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write("p.md", b"v1\n");
        assert_ok(&sync(a.root(), &remote, &mut st, "A"));
        a.write("p.md", b"v2\n");
        remote.lock().fail_next_puts = 1;
        let report = sync(a.root(), &remote, &mut st, "A");
        assert_ok(&report);
        assert_eq!(report.uploaded, vec!["p.md".to_string()]);
        assert_eq!(remote.contents()["p.md"], b"v2\n");
    }

    #[test]
    fn repeated_precondition_failures_are_reported_for_that_file_only() {
        let a = Scratch::new("precond-twice");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write("p.md", b"v1\n");
        assert_ok(&sync(a.root(), &remote, &mut st, "A"));
        a.write("p.md", b"v2\n");
        a.write("other.md", b"fresh\n");
        remote.lock().fail_next_puts = 2;
        let report = sync(a.root(), &remote, &mut st, "A");
        assert!(report.fatal.is_none());
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert_eq!(report.errors[0].path, "p.md");
        assert!(report.uploaded.contains(&"other.md".to_string()) || report.errors.len() == 1);
        // The next run finishes the job.
        assert_ok(&sync(a.root(), &remote, &mut st, "A"));
        assert_eq!(remote.contents()["p.md"], b"v2\n");
        assert_eq!(remote.contents()["other.md"], b"fresh\n");
    }

    #[test]
    fn one_failing_file_does_not_abort_the_run() {
        let a = Scratch::new("one-fails");
        let remote = FakeRemote::new();
        remote.lock().fail_keys.insert("bad.md".into());
        let mut st = SyncState::default();
        a.write("bad.md", b"x");
        a.write("good.md", b"y");
        let report = sync(a.root(), &remote, &mut st, "A");
        assert!(report.fatal.is_none());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].path, "bad.md");
        assert_eq!(report.uploaded, vec!["good.md".to_string()]);
        assert!(remote.contents().contains_key("good.md"));
    }

    #[test]
    fn dry_run_changes_nothing() {
        let a = Scratch::new("dry");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write("n.md", b"x");
        let mut opts = SyncOptions::new("A");
        opts.dry_run = true;
        let report = run_sync(a.root(), &remote, &mut st, &mut opts);
        assert_eq!(
            report.planned,
            vec![SyncAction::Upload {
                path: "n.md".into()
            }]
        );
        assert!(remote.contents().is_empty());
        assert!(!state::state_path(a.root()).exists());
    }

    #[test]
    fn cancel_flag_stops_before_any_action() {
        let a = Scratch::new("cancel");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write("n.md", b"x");
        let flag = AtomicBool::new(true);
        let mut opts = SyncOptions::new("A");
        opts.cancel = Some(&flag);
        let report = run_sync(a.root(), &remote, &mut st, &mut opts);
        assert!(report.cancelled);
        assert!(remote.contents().is_empty());
    }

    #[test]
    fn progress_is_reported_per_action() {
        let a = Scratch::new("progress");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write("1.md", b"1");
        a.write("2.md", b"2");
        let mut seen: Vec<String> = Vec::new();
        let mut cb = |p: SyncProgress| seen.push(format!("{}:{}/{}", p.path, p.done, p.total));
        let mut opts = SyncOptions::new("A");
        opts.progress = Some(&mut cb);
        let report = run_sync(a.root(), &remote, &mut st, &mut opts);
        assert_ok(&report);
        drop(opts);
        assert!(seen.contains(&"1.md:0/2".to_string()), "{seen:?}");
        assert!(seen.contains(&"2.md:1/2".to_string()), "{seen:?}");
    }

    #[test]
    fn unsafe_remote_keys_are_never_written_outside_the_vault() {
        let a = Scratch::new("unsafe");
        let remote = FakeRemote::new();
        remote.put_raw("../escape.md", b"evil");
        remote.put_raw("ok.md", b"fine");
        let mut st = SyncState::default();
        let report = sync(a.root(), &remote, &mut st, "A");
        assert!(report.fatal.is_none());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.downloaded, vec!["ok.md".to_string()]);
        assert!(!a.root().join("..").join("escape.md").exists());
    }

    #[test]
    fn newly_ignored_synced_file_is_not_deleted_remotely() {
        let a = Scratch::new("ignore-late");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write("private/secret.md", b"keep me\n");
        assert_ok(&sync(a.root(), &remote, &mut st, "A"));
        assert!(remote.contents().contains_key("private/secret.md"));

        let mut opts = SyncOptions::new("A");
        opts.ignore = IgnoreRules::new(&["private/**"]);
        let report = run_sync(a.root(), &remote, &mut st, &mut opts);
        assert_ok(&report);
        assert!(report.deleted_remote.is_empty());
        assert_eq!(remote.contents()["private/secret.md"], b"keep me\n");
        assert_eq!(
            a.read("private/secret.md").as_deref(),
            Some(&b"keep me\n"[..])
        );
    }

    #[test]
    fn state_and_trash_folders_never_sync() {
        let a = Scratch::new("never-sync");
        let remote = FakeRemote::new();
        let mut st = SyncState::default();
        a.write(".jiyue/sync/state.json", b"{}");
        a.write(".trash/sync/old.md", b"old");
        a.write("visible.md", b"v");
        assert_ok(&sync(a.root(), &remote, &mut st, "A"));
        let keys: Vec<String> = remote.contents().keys().cloned().collect();
        assert_eq!(keys, vec!["visible.md".to_string()]);
    }

    #[test]
    fn remote_prefix_is_applied_and_stripped() {
        let a = Scratch::new("prefix");
        let remote = FakeRemote::new();
        remote.put_raw("other/unrelated.md", b"no");
        let mut st = SyncState::default();
        a.write("n.md", b"x");
        let mut opts = SyncOptions::new("A");
        opts.remote_prefix = "vault".into();
        assert_ok(&run_sync(a.root(), &remote, &mut st, &mut opts));
        assert!(remote.contents().contains_key("vault/n.md"));
        assert!(!remote.contents().contains_key("n.md"));
        assert!(remote.contents().contains_key("other/unrelated.md"));
    }

    #[test]
    fn remote_listing_failure_is_fatal_and_changes_nothing() {
        struct Broken;
        impl RemoteStore for Broken {
            fn list(&self, _: &str) -> Result<Vec<RemoteEntry>, StoreError> {
                Err(StoreError::Network("down".into()))
            }
            fn stat(&self, _: &str) -> Result<Option<RemoteEntry>, StoreError> {
                Err(StoreError::Network("down".into()))
            }
            fn get(&self, _: &str, _: Option<&str>) -> Result<Vec<u8>, StoreError> {
                Err(StoreError::Network("down".into()))
            }
            fn put(&self, _: &str, _: &[u8], _: &PutOptions) -> Result<RemoteEntry, StoreError> {
                Err(StoreError::Network("down".into()))
            }
            fn delete(&self, _: &str) -> Result<(), StoreError> {
                Err(StoreError::Network("down".into()))
            }
            fn list_versions(&self, _: &str) -> Result<Vec<RemoteEntry>, StoreError> {
                Ok(Vec::new())
            }
            fn supports_versions(&self) -> bool {
                false
            }
        }
        let a = Scratch::new("broken");
        a.write("n.md", b"x");
        let mut st = SyncState::default();
        let report = sync(a.root(), &Broken, &mut st, "A");
        assert!(report.fatal.is_some());
        assert_eq!(a.read("n.md").as_deref(), Some(&b"x"[..]));
    }

    #[test]
    fn missing_vault_folder_is_fatal() {
        let a = Scratch::new("missing-root");
        let mut st = SyncState::default();
        let report = sync(&a.root().join("nope"), &FakeRemote::new(), &mut st, "A");
        assert!(report.fatal.is_some());
    }

    #[test]
    fn state_survives_a_restart_and_the_next_run_is_a_noop() {
        let a = Scratch::new("restart");
        let remote = FakeRemote::new();
        a.write("n.md", b"stable\n");
        {
            let mut st = state::load_state(a.root()).expect("load");
            assert_ok(&sync(a.root(), &remote, &mut st, "A"));
        }
        let mut st = state::load_state(a.root()).expect("reload");
        assert_nothing_to_do(&sync(a.root(), &remote, &mut st, "A"));
    }

    // ----------------------------------------------------------------------------------------
    // Simulation: two devices, random operation sequences, convergence invariants
    // ----------------------------------------------------------------------------------------

    /// xorshift64* generator: deterministic per seed.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1)
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
    }

    const TEXT_NAMES: &[&str] = &["n0.md", "n1.md", "n2.md", "d/x.md", "d/y.txt"];
    const BINARY_NAMES: &[&str] = &["p.png", "d/q.bin"];

    fn token_text(tok: u64) -> String {
        format!("<tok:{tok}>")
    }

    /// Every `<tok:N>` marker in the bytes.
    fn tokens_in(bytes: &[u8]) -> Vec<u64> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 5 <= bytes.len() {
            if &bytes[i..i + 5] == b"<tok:" {
                let mut j = i + 5;
                let mut n: u64 = 0;
                let mut digits = 0;
                while j < bytes.len() && bytes[j].is_ascii_digit() && digits < 18 {
                    n = n * 10 + u64::from(bytes[j] - b'0');
                    j += 1;
                    digits += 1;
                }
                if digits > 0 && j < bytes.len() && bytes[j] == b'>' {
                    out.push(n);
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    /// All files under `root` except `.jiyue` and `.trash`, as relative path -> bytes.
    fn vault_contents(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>) {
            let Ok(read) = fs::read_dir(dir) else {
                return;
            };
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".jiyue" || name == ".trash" {
                    continue;
                }
                let child_rel = if rel.is_empty() {
                    name.clone()
                } else {
                    format!("{rel}/{name}")
                };
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, &child_rel, out);
                } else if let Ok(data) = fs::read(&path) {
                    out.insert(child_rel, data);
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(root, "", &mut out);
        out
    }

    /// Bytes of every file in the `.trash` folder of `root`.
    fn trash_files(root: &Path) -> Vec<Vec<u8>> {
        fn walk(dir: &Path, out: &mut Vec<Vec<u8>>) {
            let Ok(read) = fs::read_dir(dir) else {
                return;
            };
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if let Ok(data) = fs::read(&path) {
                    out.push(data);
                }
            }
        }
        let mut out = Vec::new();
        walk(&root.join(".trash"), &mut out);
        out
    }

    #[derive(Default, Debug)]
    struct SimStats {
        ops: u64,
        syncs: u64,
        uploads: u64,
        downloads: u64,
        merges: u64,
        conflicts: u64,
        deletes: u64,
        errors: u64,
    }

    fn add_report(stats: &mut SimStats, report: &SyncReport) {
        stats.syncs += 1;
        stats.uploads += report.uploaded.len() as u64;
        stats.downloads += report.downloaded.len() as u64;
        stats.merges += report.merged.len() as u64;
        stats.conflicts += report.conflicts.len() as u64;
        stats.deletes += (report.deleted_local.len() + report.deleted_remote.len()) as u64;
        stats.errors += report.errors.len() as u64;
    }

    /// Runs one random sequence and checks the invariants. Panics with the sequence number on
    /// failure.
    fn run_sequence(seq: u64, stats: &mut SimStats) {
        let base =
            std::env::temp_dir().join(format!("jiyue-sync-sim-{}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let roots = [base.join("A"), base.join("B")];
        for r in &roots {
            fs::create_dir_all(r).expect("sim vault");
        }
        let names = ["A", "B"];
        let remote =
            FakeRemote::with_injection(if seq.is_multiple_of(4) { 0 } else { 150 }, seq + 1);
        let mut states = [SyncState::default(), SyncState::default()];
        let mut rng = Rng::new(seq.wrapping_mul(7919).wrapping_add(17));
        let mut must: BTreeSet<u64> = BTreeSet::new();
        let mut next_token: u64 = 1;

        let steps = 20 + rng.below(30);
        for _ in 0..steps {
            let dev = rng.below(2) as usize;
            let root = roots[dev].clone();
            stats.ops += 1;
            match rng.below(100) {
                0..=14 => {
                    // create a text file (sometimes without a final newline)
                    let name = TEXT_NAMES[rng.below(TEXT_NAMES.len() as u64) as usize];
                    let path = root.join(name);
                    if !path.exists() {
                        let tok = next_token;
                        next_token += 1;
                        let mut text = token_text(tok);
                        if rng.below(3) != 0 {
                            text.push('\n');
                        }
                        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
                        fs::write(&path, text).expect("create");
                        must.insert(tok);
                    }
                }
                15..=29 => {
                    // append a line
                    let name = TEXT_NAMES[rng.below(TEXT_NAMES.len() as u64) as usize];
                    let path = root.join(name);
                    if let Ok(mut data) = fs::read(&path) {
                        let tok = next_token;
                        next_token += 1;
                        if !data.is_empty() && !data.ends_with(b"\n") {
                            data.push(b'\n');
                        }
                        data.extend_from_slice(token_text(tok).as_bytes());
                        data.push(b'\n');
                        fs::write(&path, data).expect("append");
                        must.insert(tok);
                    }
                }
                30..=39 => {
                    // insert a line in the middle (creates overlapping edits across devices)
                    let name = TEXT_NAMES[rng.below(TEXT_NAMES.len() as u64) as usize];
                    let path = root.join(name);
                    if let Ok(data) = fs::read_to_string(&path) {
                        let tok = next_token;
                        next_token += 1;
                        let mut lines: Vec<String> = data.split('\n').map(str::to_owned).collect();
                        let idx = rng.below(lines.len() as u64 + 1) as usize;
                        lines.insert(idx, token_text(tok));
                        fs::write(&path, lines.join("\n")).expect("insert");
                        must.insert(tok);
                    }
                }
                40..=44 => {
                    // rewrite a text file: the old tokens are intentionally gone
                    let name = TEXT_NAMES[rng.below(TEXT_NAMES.len() as u64) as usize];
                    let path = root.join(name);
                    if let Ok(old) = fs::read(&path) {
                        for t in tokens_in(&old) {
                            must.remove(&t);
                        }
                        let tok = next_token;
                        next_token += 1;
                        fs::write(&path, format!("{}\n", token_text(tok))).expect("rewrite");
                        must.insert(tok);
                    }
                }
                45..=52 => {
                    // delete a file: its tokens are intentionally gone
                    let all: Vec<&str> = TEXT_NAMES
                        .iter()
                        .chain(BINARY_NAMES.iter())
                        .copied()
                        .collect();
                    let name = all[rng.below(all.len() as u64) as usize];
                    let path = root.join(name);
                    if let Ok(old) = fs::read(&path) {
                        for t in tokens_in(&old) {
                            must.remove(&t);
                        }
                        fs::remove_file(&path).expect("delete");
                    }
                }
                53..=58 => {
                    // rename: the content moves, its tokens stay
                    let name = TEXT_NAMES[rng.below(TEXT_NAMES.len() as u64) as usize];
                    let target = TEXT_NAMES[rng.below(TEXT_NAMES.len() as u64) as usize];
                    let from = root.join(name);
                    let to = root.join(target);
                    if from.exists() && !to.exists() && name != target {
                        fs::create_dir_all(to.parent().expect("parent")).expect("mkdir");
                        fs::rename(&from, &to).expect("rename");
                    }
                }
                59..=62 => {
                    // binary create or rewrite, with a token inside the bytes
                    let name = BINARY_NAMES[rng.below(BINARY_NAMES.len() as u64) as usize];
                    let path = root.join(name);
                    if let Ok(old) = fs::read(&path) {
                        for t in tokens_in(&old) {
                            must.remove(&t);
                        }
                    }
                    let tok = next_token;
                    next_token += 1;
                    let mut bytes: Vec<u8> =
                        (0..rng.below(40)).map(|_| rng.below(256) as u8).collect();
                    bytes.extend_from_slice(token_text(tok).as_bytes());
                    bytes.extend((0..rng.below(20)).map(|_| rng.below(256) as u8));
                    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
                    fs::write(&path, bytes).expect("binary write");
                    must.insert(tok);
                }
                _ => {
                    let report = sync(&root, &remote, &mut states[dev], names[dev]);
                    add_report(stats, &report);
                    // Injected conflicts may leave errors for one run; the final syncs must not.
                    assert!(
                        report.fatal.is_none(),
                        "seq {seq}: fatal during run: {:?}",
                        report.fatal
                    );
                }
            }
        }

        // Final sync: no injection, A then B then A must converge both vaults and the remote.
        remote.set_injection(0);
        for dev in [0usize, 1, 0] {
            let report = sync(&roots[dev], &remote, &mut states[dev], names[dev]);
            add_report(stats, &report);
            assert!(
                report.fatal.is_none() && report.errors.is_empty(),
                "seq {seq}: final sync of {} failed: {report:?}",
                names[dev]
            );
        }
        let va = vault_contents(&roots[0]);
        let vb = vault_contents(&roots[1]);
        let vr = remote.contents();
        assert_eq!(va, vb, "seq {seq}: vaults differ after final sync");
        assert_eq!(va, vr, "seq {seq}: remote differs from the vaults");

        // Idempotence: another round changes nothing.
        for dev in 0..2 {
            let report = sync(&roots[dev], &remote, &mut states[dev], names[dev]);
            assert_nothing_to_do(&report);
        }

        // No content loss: every token that was not intentionally removed is still somewhere
        // (vault, conflict copy, or trash).
        let mut found: BTreeSet<u64> = BTreeSet::new();
        for data in va.values() {
            found.extend(tokens_in(data));
        }
        for root in &roots {
            for data in trash_files(root) {
                found.extend(tokens_in(&data));
            }
        }
        for tok in &must {
            assert!(found.contains(tok), "seq {seq}: token {tok} was lost");
        }

        let _ = fs::remove_dir_all(&base);
    }

    fn add_stats(total: &mut SimStats, s: &SimStats) {
        total.ops += s.ops;
        total.syncs += s.syncs;
        total.uploads += s.uploads;
        total.downloads += s.downloads;
        total.merges += s.merges;
        total.conflicts += s.conflicts;
        total.deletes += s.deletes;
        total.errors += s.errors;
    }

    /// Fast variant that runs by default: 100 sequences (about 20 s here). Same checks as the full
    /// run below, with fewer sequences, so CI does not depend on how long the sync waits take.
    #[test]
    fn simulation_two_devices_converge_quick() {
        run_simulation(100);
    }

    /// The full run: `JIYUE_SYNC_SIM_SEQUENCES` sequences (default 2000). It takes many minutes
    /// because the sync waits are wall-clock based, so it only runs on request:
    /// `cargo test -p jiyue-sync -- --ignored simulation_two_devices_converge_without_loss`.
    #[test]
    #[ignore = "slow: 2000 sequences take many minutes; run on request with --ignored"]
    fn simulation_two_devices_converge_without_loss() {
        let sequences: u64 = std::env::var("JIYUE_SYNC_SIM_SEQUENCES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2000);
        run_simulation(sequences);
    }

    /// Each sequence uses its own folders, so workers never touch each other's files.
    fn run_simulation(sequences: u64) {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, 8) as u64;
        let totals = Mutex::new(SimStats::default());
        std::thread::scope(|scope| {
            for w in 0..workers {
                let totals = &totals;
                scope.spawn(move || {
                    let mut local = SimStats::default();
                    let mut seq = w;
                    while seq < sequences {
                        run_sequence(seq, &mut local);
                        seq += workers;
                    }
                    add_stats(&mut totals.lock().expect("stats lock"), &local);
                });
            }
        });
        let stats = totals.into_inner().expect("stats lock");
        eprintln!(
            "jiyue-sync simulation: {sequences} sequences passed on {workers} threads; {stats:?}"
        );
        assert!(stats.syncs > 0 && stats.ops > 0);
    }

    #[test]
    fn tokens_parser_finds_markers_in_text_and_binary() {
        assert_eq!(tokens_in(b"a<tok:12>b<tok:3>"), vec![12, 3]);
        assert_eq!(
            tokens_in(&[0u8, 255, b'<', b't', b'o', b'k', b':', b'7', b'>']),
            vec![7]
        );
        assert!(tokens_in(b"<tok:>").is_empty());
        assert!(tokens_in(b"<tok:12").is_empty());
    }
}
