//! The `History` store: owns the lock, the index and the blob directory.
//!
//! Every mutation first appends its record to the index (after the blob it
//! refers to is durable), then applies the same record to the in-memory state.
//! Opening replays the index with the same code path, so live state and
//! replayed state cannot diverge.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::blobs::{self, BlobError};
use crate::diff::{DiffLine, diff_text};
use crate::error::{HistoryError, Result};
use crate::index::{self, IndexLog, Record};
use crate::model::{
    DeletedNote, Entry, EntryId, GcReport, Kind, MAX_FILE_BYTES, SnapshotMeta, SnapshotOutcome,
    Stats, VerifyReport,
};
use crate::retention::{self, Retention};

const INDEX_FILE: &str = "index.jsonl";
const BLOB_DIR: &str = "blobs";
const LOCK_FILE: &str = "LOCK";
/// Extra records tolerated before the index is compacted.
const COMPACT_SLACK: usize = 512;
const MAX_LABEL_CHARS: usize = 256;

/// Wall-clock time in milliseconds since the Unix epoch. The store itself
/// never reads the clock; callers pass times in.
pub fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Vault-relative note path: forward slashes, no leading slash, no `.`/`..`.
fn normalize_path(path: &str) -> Result<String> {
    let replaced = path.replace('\\', "/");
    let parts: Vec<&str> = replaced
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    if parts.is_empty() || parts.contains(&"..") || path.contains('\0') {
        return Err(HistoryError::InvalidPath(path.to_string()));
    }
    Ok(parts.join("/"))
}

fn normalize_label(label: Option<String>) -> Option<String> {
    label.and_then(|l| {
        let t: String = l.trim().chars().take(MAX_LABEL_CHARS).collect();
        if t.is_empty() { None } else { Some(t) }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlobStatus {
    Ok,
    Missing,
    Corrupt,
}

fn check_blob(root: &Path, hash: &str) -> BlobStatus {
    match blobs::read_blob(root, hash) {
        Ok(_) => BlobStatus::Ok,
        Err(BlobError::Missing) => BlobStatus::Missing,
        Err(_) => BlobStatus::Corrupt,
    }
}

/// All regular files under `root/<aa>/`, as (file name, path).
fn walk_blobs(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(dirs) = fs::read_dir(root) else {
        return out;
    };
    for dir in dirs.flatten() {
        let dir_path = dir.path();
        if !dir_path.is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(&dir_path) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.is_file() {
                out.push((file.file_name().to_string_lossy().into_owned(), path));
            }
        }
    }
    out
}

/// Single-process owner of one history directory. Holds an exclusive
/// advisory lock on `LOCK` for its whole lifetime.
pub struct History {
    dir: PathBuf,
    blob_root: PathBuf,
    _lock: File,
    index: IndexLog,
    entries: BTreeMap<u64, Entry>,
    /// Path -> entry ids (any order; sorted when listed).
    notes: BTreeMap<String, Vec<u64>>,
    next_id: u64,
    /// Hashes whose blob file is missing. Their entries are hidden from listings.
    missing: HashSet<String>,
    bad_index_lines: usize,
    tmp_seq: u64,
    /// Blob files written since the last `flush`.
    dirty_blobs: Vec<PathBuf>,
}

impl Drop for History {
    fn drop(&mut self) {
        // Best effort: nothing useful can be done with an error during drop.
        let _ = self.flush();
    }
}

impl History {
    /// Open (or create) the history in `dir`, normally `<vault>/.jiyue/history`.
    ///
    /// Fails with `HistoryError::Locked` if another `History` holds the same
    /// directory. Damaged index lines and missing blobs do not fail the open;
    /// see `verify`.
    pub fn open(dir: &Path) -> Result<History> {
        fs::create_dir_all(dir)?;
        let blob_root = dir.join(BLOB_DIR);
        fs::create_dir_all(&blob_root)?;

        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(dir.join(LOCK_FILE))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(HistoryError::Locked),
            Err(TryLockError::Error(e)) => return Err(HistoryError::Io(e)),
        }

        let index_path = dir.join(INDEX_FILE);
        let loaded = index::load(&index_path)?;
        let mut h = History {
            dir: dir.to_path_buf(),
            blob_root,
            _lock: lock,
            index: IndexLog::open(&index_path)?,
            entries: BTreeMap::new(),
            notes: BTreeMap::new(),
            next_id: 1,
            missing: HashSet::new(),
            bad_index_lines: loaded.bad_lines,
            tmp_seq: 0,
            dirty_blobs: Vec::new(),
        };
        let valid_records = loaded.records.len();
        for rec in loaded.records {
            if !h.apply_mem(rec) {
                h.bad_index_lines += 1;
            }
        }
        h.index.records = valid_records;
        if loaded.needs_newline {
            h.index.append_raw(b"\n")?;
        }

        let hashes: HashSet<String> = h.entries.values().map(|e| e.hash.clone()).collect();
        for hash in hashes {
            if !blobs::blob_path(&h.blob_root, &hash).is_file() {
                h.missing.insert(hash);
            }
        }
        if h.needs_compaction() {
            h.compact()?;
        }
        Ok(h)
    }

    /// The directory this history lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    // ----- snapshots -------------------------------------------------------

    /// Record the content of `path` at `now_ms`. Returns `None` when nothing
    /// was written (identical to the latest snapshot, or too large).
    pub fn snapshot(
        &mut self,
        path: &str,
        bytes: &[u8],
        meta: SnapshotMeta,
        now_ms: i64,
    ) -> Result<Option<EntryId>> {
        Ok(match self.snapshot_outcome(path, bytes, meta, now_ms)? {
            SnapshotOutcome::Created(id) => Some(id),
            SnapshotOutcome::Unchanged | SnapshotOutcome::TooLarge { .. } => None,
        })
    }

    /// Like `snapshot`, but says why nothing was written.
    ///
    /// `Deleted` snapshots are written even when the content equals the latest
    /// entry, unless the note is already marked deleted. A label given with
    /// identical content is attached to the latest entry instead of creating one.
    pub fn snapshot_outcome(
        &mut self,
        path: &str,
        bytes: &[u8],
        meta: SnapshotMeta,
        now_ms: i64,
    ) -> Result<SnapshotOutcome> {
        let path = normalize_path(path)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Ok(SnapshotOutcome::TooLarge {
                size: bytes.len() as u64,
            });
        }
        let hash = blobs::hash_hex(bytes);
        if let Some(latest) = self.latest_norm(&path) {
            let same = if meta.kind == Kind::Deleted {
                latest.kind == Kind::Deleted
            } else {
                latest.kind != Kind::Deleted && latest.hash == hash
            };
            if same {
                if meta.kind != Kind::Deleted
                    && let Some(label) = normalize_label(meta.label)
                    && latest.label.as_deref() != Some(label.as_str())
                {
                    self.set_label(&latest.id, Some(label))?;
                }
                return Ok(SnapshotOutcome::Unchanged);
            }
        }
        self.ensure_blob(&hash, bytes)?;
        let id = self.next_id;
        let rec = Record::Put {
            id,
            path,
            time_ms: now_ms,
            size: bytes.len() as u64,
            hash,
            kind: meta.kind,
            label: normalize_label(meta.label),
        };
        self.commit(rec)?;
        Ok(SnapshotOutcome::Created(EntryId(id)))
    }

    /// Time of the newest visible entry of `path`, for throttling with
    /// `should_snapshot`.
    pub fn latest_time(&self, path: &str) -> Option<i64> {
        let path = normalize_path(path).ok()?;
        self.latest_norm(&path).map(|e| e.time_ms)
    }

    // ----- listing and reading -------------------------------------------

    /// Visible entries of `path`, newest first. Entries with a missing blob are hidden.
    pub fn list(&self, path: &str) -> Vec<Entry> {
        match normalize_path(path) {
            Ok(p) => self.list_norm(&p),
            Err(_) => Vec::new(),
        }
    }

    /// One entry by id (even if its blob is missing).
    pub fn entry(&self, id: &EntryId) -> Option<Entry> {
        self.entries.get(&id.0).cloned()
    }

    /// Full content of an entry, verified against its hash.
    pub fn read(&self, id: &EntryId) -> Result<Vec<u8>> {
        let entry = self.entries.get(&id.0).ok_or(HistoryError::NotFound(*id))?;
        self.read_entry(entry)
    }

    /// Line diff between two entries. Both must be UTF-8 text.
    pub fn diff(&self, a: &EntryId, b: &EntryId) -> Result<Vec<DiffLine>> {
        let old = self.read(a)?;
        let new = self.read(b)?;
        let old = std::str::from_utf8(&old).map_err(|_| HistoryError::NotText)?;
        let new = std::str::from_utf8(&new).map_err(|_| HistoryError::NotText)?;
        Ok(diff_text(old, new))
    }

    /// Content to put back, plus snapshots that keep the current state safe.
    ///
    /// With `current = Some(bytes)`, the current content is snapshotted as
    /// `BeforeRestore` first (unless identical to the latest snapshot). With
    /// `current = None` (file gone), the restored content is snapshotted as
    /// `Auto`, which also brings a deleted note back into the normal list.
    /// The caller writes the returned bytes to the file.
    pub fn restore_bytes(
        &mut self,
        id: &EntryId,
        current: Option<&[u8]>,
        now_ms: i64,
    ) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .get(&id.0)
            .cloned()
            .ok_or(HistoryError::NotFound(*id))?;
        if let Some(cur) = current
            && cur.len() > MAX_FILE_BYTES
        {
            return Err(HistoryError::TooLarge {
                size: cur.len() as u64,
            });
        }
        let old = self.read_entry(&entry)?;
        match current {
            Some(cur) => {
                self.snapshot_outcome(&entry.path, cur, SnapshotMeta::before_restore(), now_ms)?;
            }
            None => {
                self.snapshot_outcome(&entry.path, &old, SnapshotMeta::auto(), now_ms)?;
            }
        }
        Ok(old)
    }

    /// Notes whose latest entry is a deletion, newest first.
    pub fn list_deleted(&self) -> Vec<DeletedNote> {
        let mut out: Vec<DeletedNote> = self
            .notes
            .keys()
            .filter_map(|path| self.latest_norm(path))
            .filter(|e| e.kind == Kind::Deleted)
            .map(|e| DeletedNote {
                path: e.path,
                id: e.id,
                time_ms: e.time_ms,
                size: e.size,
            })
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse((e.time_ms, e.id.0)));
        out
    }

    // ----- structural changes --------------------------------------------

    /// Record that `path` was deleted. `last_bytes` is the last known content.
    /// Silently ignored when that content is larger than `MAX_FILE_BYTES`.
    pub fn note_deleted(&mut self, path: &str, last_bytes: &[u8], now_ms: i64) -> Result<()> {
        self.snapshot_outcome(path, last_bytes, SnapshotMeta::deleted(), now_ms)?;
        Ok(())
    }

    /// The note moved from `old` to `new`. Its history follows. Renaming to a
    /// path that already has history merges the two histories.
    pub fn note_renamed(&mut self, old: &str, new: &str) -> Result<()> {
        let old = normalize_path(old)?;
        let new = normalize_path(new)?;
        if old == new || !self.notes.contains_key(&old) {
            return Ok(());
        }
        self.commit(Record::Rename { old, new })
    }

    /// Set (or with `None`/blank, clear) the label of an entry.
    pub fn set_label(&mut self, id: &EntryId, label: Option<String>) -> Result<()> {
        let current = self.entries.get(&id.0).ok_or(HistoryError::NotFound(*id))?;
        let label = normalize_label(label);
        if current.label == label {
            return Ok(());
        }
        self.commit(Record::Label { id: id.0, label })
    }

    // ----- retention, maintenance ----------------------------------------

    /// Remove entries the policy does not keep, then delete unreferenced blob
    /// files, and compact the index if it has grown too large.
    pub fn apply_retention(&mut self, policy: &Retention, now_ms: i64) -> Result<GcReport> {
        let drop_ids: Vec<u64> = {
            let refs: Vec<&Entry> = self.entries.values().collect();
            retention::plan_removals(&refs, policy, now_ms)
                .into_iter()
                .map(|id| id.0)
                .collect()
        };
        let mut report = GcReport {
            removed_entries: drop_ids.len(),
            ..GcReport::default()
        };
        if !drop_ids.is_empty() {
            self.commit(Record::Remove { ids: drop_ids })?;
        }
        report.kept_entries = self.entries.len();
        let (blobs_removed, freed) = self.collect_blobs()?;
        report.removed_blobs = blobs_removed;
        report.freed_bytes = freed;
        if self.needs_compaction() {
            self.compact()?;
        }
        Ok(report)
    }

    /// Rewrite the index from the in-memory state. Ids and the next id are preserved.
    pub fn compact(&mut self) -> Result<()> {
        let mut records = Vec::with_capacity(self.entries.len() + 1);
        records.push(Record::Seq { next: self.next_id });
        for e in self.entries.values() {
            records.push(Record::Put {
                id: e.id.0,
                path: e.path.clone(),
                time_ms: e.time_ms,
                size: e.size,
                hash: e.hash.clone(),
                kind: e.kind,
                label: e.label.clone(),
            });
        }
        self.tmp_seq += 1;
        let tag = self.tmp_seq;
        self.index.rewrite(&records, tag)?;
        self.bad_index_lines = 0;
        Ok(())
    }

    /// Check every entry's blob (existence, decoding, hash) and the index.
    /// Read-only; never changes anything.
    pub fn verify(&self) -> VerifyReport {
        let mut report = VerifyReport {
            bad_index_lines: self.bad_index_lines,
            ..VerifyReport::default()
        };
        let mut cache: HashMap<&str, BlobStatus> = HashMap::new();
        for e in self.entries.values() {
            report.entries_checked += 1;
            let status = *cache
                .entry(e.hash.as_str())
                .or_insert_with(|| check_blob(&self.blob_root, &e.hash));
            match status {
                BlobStatus::Ok => {}
                BlobStatus::Missing => report.missing_blobs.push(e.id),
                BlobStatus::Corrupt => report.corrupt_blobs.push(e.id),
            }
        }
        let referenced: HashSet<&str> = self.entries.values().map(|e| e.hash.as_str()).collect();
        for (name, _) in walk_blobs(&self.blob_root) {
            if !blobs::is_valid_hash(&name) {
                report.stray_files += 1;
            } else if !referenced.contains(name.as_str()) {
                report.orphan_blobs += 1;
            }
        }
        report
    }

    /// Aggregate numbers. Stats the blob files, so it costs one syscall per unique blob.
    pub fn stats(&self) -> Stats {
        let visible: Vec<&Entry> = self
            .entries
            .values()
            .filter(|e| !self.missing.contains(&e.hash))
            .collect();
        let notes: HashSet<&str> = visible.iter().map(|e| e.path.as_str()).collect();
        let mut unique: HashMap<&str, u64> = HashMap::new();
        for e in &visible {
            unique.entry(e.hash.as_str()).or_insert(e.size);
        }
        let logical_bytes: u64 = unique.values().sum();
        let bytes_on_disk: u64 = unique
            .keys()
            .map(|h| {
                fs::metadata(blobs::blob_path(&self.blob_root, h))
                    .map(|m| m.len())
                    .unwrap_or(0)
            })
            .sum();
        Stats {
            entries: visible.len(),
            notes: notes.len(),
            unique_blobs: unique.len(),
            logical_bytes,
            bytes_on_disk,
            index_bytes: self.index.bytes,
        }
    }

    // ----- internals -----------------------------------------------------

    fn needs_compaction(&self) -> bool {
        self.index.records > 2 * (self.entries.len() + 1) + COMPACT_SLACK
    }

    /// Append `rec` to the index, then apply it in memory.
    fn commit(&mut self, rec: Record) -> Result<()> {
        self.index.append(&rec)?;
        let _ = self.apply_mem(rec);
        Ok(())
    }

    /// Apply one record to the in-memory state. Returns false for a record
    /// that cannot be applied (only possible when replaying a damaged index).
    fn apply_mem(&mut self, rec: Record) -> bool {
        match rec {
            Record::Seq { next } => {
                self.next_id = self.next_id.max(next);
                true
            }
            Record::Put {
                id,
                path,
                time_ms,
                size,
                hash,
                kind,
                label,
            } => {
                if !blobs::is_valid_hash(&hash) || path.is_empty() {
                    return false;
                }
                self.insert_mem(Entry {
                    id: EntryId(id),
                    path,
                    time_ms,
                    size,
                    hash,
                    kind,
                    label,
                });
                true
            }
            Record::Rename { old, new } => {
                self.rename_mem(&old, &new);
                true
            }
            Record::Label { id, label } => {
                if let Some(e) = self.entries.get_mut(&id) {
                    e.label = label;
                }
                true
            }
            Record::Remove { ids } => {
                for id in ids {
                    self.remove_mem(id);
                }
                true
            }
        }
    }

    fn insert_mem(&mut self, entry: Entry) {
        let id = entry.id.0;
        self.remove_mem(id);
        self.next_id = self.next_id.max(id.saturating_add(1));
        self.notes.entry(entry.path.clone()).or_default().push(id);
        self.entries.insert(id, entry);
    }

    fn remove_mem(&mut self, id: u64) {
        if let Some(e) = self.entries.remove(&id)
            && let Some(ids) = self.notes.get_mut(&e.path)
        {
            ids.retain(|x| *x != id);
            if ids.is_empty() {
                self.notes.remove(&e.path);
            }
        }
    }

    fn rename_mem(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        if let Some(ids) = self.notes.remove(old) {
            for id in &ids {
                if let Some(e) = self.entries.get_mut(id) {
                    e.path = new.to_string();
                }
            }
            self.notes.entry(new.to_string()).or_default().extend(ids);
        }
    }

    fn list_norm(&self, path: &str) -> Vec<Entry> {
        let mut out: Vec<Entry> = self
            .notes
            .get(path)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.entries.get(id))
                    .filter(|e| !self.missing.contains(&e.hash))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|e| std::cmp::Reverse((e.time_ms, e.id.0)));
        out
    }

    /// Newest visible entry of `path`. Scans without cloning the whole history.
    fn latest_norm(&self, path: &str) -> Option<Entry> {
        self.notes
            .get(path)?
            .iter()
            .filter_map(|id| self.entries.get(id))
            .filter(|e| !self.missing.contains(&e.hash))
            .max_by_key(|e| (e.time_ms, e.id.0))
            .cloned()
    }

    fn read_entry(&self, entry: &Entry) -> Result<Vec<u8>> {
        if self.missing.contains(&entry.hash) {
            return Err(HistoryError::BlobMissing {
                hash: entry.hash.clone(),
            });
        }
        blobs::read_blob(&self.blob_root, &entry.hash).map_err(|e| match e {
            BlobError::Missing => HistoryError::BlobMissing {
                hash: entry.hash.clone(),
            },
            BlobError::Corrupt(reason) => HistoryError::Corrupt {
                hash: entry.hash.clone(),
                reason,
            },
            BlobError::Io(e) => HistoryError::Io(e),
        })
    }

    /// Make sure a valid blob for `hash` exists. Repairs a missing or damaged one.
    fn ensure_blob(&mut self, hash: &str, raw: &[u8]) -> Result<()> {
        if blobs::read_blob(&self.blob_root, hash).is_ok() {
            self.missing.remove(hash);
            return Ok(());
        }
        let path = blobs::blob_path(&self.blob_root, hash);
        let stored = blobs::encode(raw);
        if path.exists() {
            // Damaged blob: replace it atomically so readers never see a partial file.
            self.tmp_seq += 1;
            blobs::write_atomic(&path, &stored, self.tmp_seq, false)?;
        } else {
            blobs::write_new(&path, &stored)?;
        }
        self.dirty_blobs.push(path);
        self.missing.remove(hash);
        Ok(())
    }

    /// Make everything written so far durable: fsync the blob files written since
    /// the last flush, then the index. Runs on drop too. Call it periodically
    /// (for example every few seconds, or when the app goes idle) to bound what a
    /// power loss can lose. A lost blob or index tail is detected by `verify`
    /// and never prevents opening.
    pub fn flush(&mut self) -> Result<()> {
        let mut first_err: Option<std::io::Error> = None;
        for path in std::mem::take(&mut self.dirty_blobs) {
            match blobs::sync_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        if let Err(e) = self.index.sync() {
            first_err.get_or_insert(e);
        }
        match first_err {
            Some(e) => Err(HistoryError::Io(e)),
            None => Ok(()),
        }
    }

    /// Delete blob files no entry references (including temp leftovers).
    /// Returns (files removed, bytes freed).
    fn collect_blobs(&self) -> Result<(usize, u64)> {
        let referenced: HashSet<&str> = self.entries.values().map(|e| e.hash.as_str()).collect();
        let mut removed = 0;
        let mut freed = 0;
        for (name, path) in walk_blobs(&self.blob_root) {
            if blobs::is_valid_hash(&name) && referenced.contains(name.as_str()) {
                continue;
            }
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if fs::remove_file(&path).is_ok() {
                removed += 1;
                freed += size;
            }
        }
        Ok((removed, freed))
    }
}
