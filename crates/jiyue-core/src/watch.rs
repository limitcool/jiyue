//! Vault file watching (notify), debounced and self-write aware. Owned by W1-A.
//!
//! [`VaultWatcher::start`] watches a vault folder recursively. Raw notify events are normalised to
//! vault-relative paths, ignored paths are dropped, bursts are coalesced per path, and each change
//! is emitted once the path has been quiet for [`DEBOUNCE`]. Files this process writes through
//! `fsops::write_atomic` are recorded in a process-wide registry and not reported back.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecursiveMode, Watcher};

use crate::fsops;

/// Quiet period before a coalesced change is emitted.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// How long a self-write stays expected.
pub const SELF_WRITE_TTL: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(50);
/// A file that is still empty after this many debounce rounds is reported as it is.
const MAX_EMPTY_RETRIES: u8 = 3;

/// A change in the vault. Paths are vault-relative with forward slashes. Folders are reported
/// too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultEvent {
    Created(String),
    Modified(String),
    Removed(String),
    Renamed { from: String, to: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    Io(String),
    Notify(String),
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatchError::Io(msg) => write!(f, "watch I/O error: {msg}"),
            WatchError::Notify(msg) => write!(f, "file watcher error: {msg}"),
        }
    }
}

impl std::error::Error for WatchError {}

/// Owns the OS watcher. Dropping it stops the watch; the event channel then disconnects.
pub struct VaultWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl VaultWatcher {
    /// Starts watching `root` (recursively). Events are sent on `tx` until the watcher is dropped.
    pub fn start(root: &Path, tx: Sender<VaultEvent>) -> Result<VaultWatcher, WatchError> {
        let canonical = root
            .canonicalize()
            .map_err(|err| WatchError::Io(err.to_string()))?;
        let watch_dir = strip_verbatim(&canonical);
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let _ = raw_tx.send(res);
        })
        .map_err(|err| WatchError::Notify(err.to_string()))?;
        watcher
            .watch(&watch_dir, RecursiveMode::Recursive)
            .map_err(|err| WatchError::Notify(err.to_string()))?;
        let pipeline = Pipeline::new(&watch_dir, tx);
        thread::Builder::new()
            .name("jiyue-vault-watch".to_string())
            .spawn(move || run(pipeline, raw_rx))
            .map_err(|err| WatchError::Io(err.to_string()))?;
        Ok(VaultWatcher { _watcher: watcher })
    }
}

/// Removes the `\\?\` prefix that `canonicalize` adds on Windows, when it is a plain drive path.
fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\UNC\") {
        return path.to_path_buf();
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

// ---------------------------------------------------------------------------
// Self-write registry
// ---------------------------------------------------------------------------

/// Paths this process is about to write, with the time they were registered. An entry suppresses
/// events for its path until `ttl` has passed.
#[derive(Debug)]
pub struct ExpectedWrites {
    ttl: Duration,
    entries: HashMap<String, Instant>,
}

impl ExpectedWrites {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: HashMap::new(),
        }
    }

    pub fn expect_at(&mut self, path: &Path, now: Instant) {
        let ttl = self.ttl;
        self.entries
            .retain(|_, at| now.saturating_duration_since(*at) < ttl);
        self.entries.insert(path_key(path), now);
    }

    pub fn is_expected_at(&mut self, path: &Path, now: Instant) -> bool {
        let key = path_key(path);
        match self.entries.get(&key) {
            Some(at) if now.saturating_duration_since(*at) < self.ttl => true,
            Some(_) => {
                self.entries.remove(&key);
                false
            }
            None => false,
        }
    }
}

static EXPECTED: LazyLock<Mutex<ExpectedWrites>> =
    LazyLock::new(|| Mutex::new(ExpectedWrites::new(SELF_WRITE_TTL)));

/// Records that this process is about to write `path`. Events for it are then not reported for
/// [`SELF_WRITE_TTL`].
pub fn expect_write(path: &Path) {
    let key = registry_path(path);
    let mut guard = EXPECTED.lock().unwrap_or_else(|e| e.into_inner());
    guard.expect_at(&key, Instant::now());
}

/// True while a self-write of `path` is still expected.
pub fn is_expected_write(path: &Path) -> bool {
    let key = registry_path(path);
    let mut guard = EXPECTED.lock().unwrap_or_else(|e| e.into_inner());
    guard.is_expected_at(&key, Instant::now())
}

/// The form of `path` the registry compares. The watcher reports events under the canonical root
/// (see [`VaultWatcher::start`]), so a caller path that spells the same folder differently (a
/// junction, a `subst` drive, or a Windows 8.3 short name such as `RUNNER~1`) must map to the
/// same key. A file that does not exist yet (a rename target) is resolved through its folder.
fn registry_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return strip_verbatim(&canonical);
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(canonical) = parent.canonicalize()
    {
        return strip_verbatim(&canonical).join(name);
    }
    path.to_path_buf()
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Forward slashes, no `\\?\` prefix, no trailing slash.
fn norm_slashes(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s
        .strip_prefix("//?/")
        .map_or_else(|| s.clone(), str::to_string);
    s.trim_end_matches('/').to_string()
}

/// Registry key: normalised, and case-folded on case-insensitive platforms.
fn path_key(path: &Path) -> String {
    let s = norm_slashes(path);
    if cfg!(any(windows, target_os = "macos")) {
        s.to_lowercase()
    } else {
        s
    }
}

/// Key of a vault-relative path in the known-paths set. Case-folded where the file system is
/// case-insensitive, so that notify spelling a name differently does not look like a new file.
fn known_key(rel: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        rel.to_lowercase()
    } else {
        rel.to_string()
    }
}

/// `full` without the `root` prefix and the separating slash. Case-insensitive on Windows and
/// macOS.
fn strip_root<'a>(full: &'a str, root: &str) -> Option<&'a str> {
    let head = full.get(..root.len())?;
    let same = if cfg!(any(windows, target_os = "macos")) {
        head.eq_ignore_ascii_case(root)
    } else {
        head == root
    };
    if !same {
        return None;
    }
    full[root.len()..].strip_prefix('/')
}

/// Editor and OS temporary or backup files that are never reported.
pub fn is_editor_temp(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    const SUFFIXES: [&str; 7] = ["~", ".swp", ".swx", ".swo", ".tmp", ".bak", ".crdownload"];
    name.starts_with("~$")
        || name == "4913"
        || SUFFIXES
            .iter()
            .any(|suffix| fsops::ends_with_ignore_case(name, suffix))
}

fn wanted(rel: &str) -> bool {
    !fsops::is_ignored(rel) && !is_editor_temp(rel)
}

// ---------------------------------------------------------------------------
// Event pipeline
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Created,
    Modified,
    Removed,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    kind: Kind,
    last: Instant,
    empty_retries: u8,
}

/// Combines a pending kind with a new one. `None` means the two cancel out.
fn merge_kinds(prev: Option<Kind>, next: Kind) -> Option<Kind> {
    match (prev, next) {
        (None, k) => Some(k),
        (Some(Kind::Created), Kind::Removed) => None,
        (Some(Kind::Created), _) => Some(Kind::Created),
        (Some(Kind::Modified), Kind::Removed) => Some(Kind::Removed),
        (Some(Kind::Modified), _) => Some(Kind::Modified),
        (Some(Kind::Removed), Kind::Created | Kind::Modified) => Some(Kind::Modified),
        (Some(Kind::Removed), Kind::Removed) => Some(Kind::Removed),
    }
}

/// Turns raw notify events into debounced [`VaultEvent`]s. Kept free of threads so it can be
/// driven with explicit timestamps in tests.
struct Pipeline {
    root_norm: String,
    root_dir: PathBuf,
    out: Sender<VaultEvent>,
    pending: BTreeMap<String, Pending>,
    rename_from: Option<(String, Instant)>,
    /// Known-key form of every file and folder already reported or present at start. A create of
    /// a known path is a modification: the OS reports a write to an existing file as a create as
    /// well as a modify (FSEvents does this for files written close to the watch start).
    known: BTreeSet<String>,
    closed: bool,
}

impl Pipeline {
    fn new(root_dir: &Path, out: Sender<VaultEvent>) -> Self {
        Self {
            root_norm: norm_slashes(root_dir),
            root_dir: root_dir.to_path_buf(),
            out,
            pending: BTreeMap::new(),
            rename_from: None,
            known: BTreeSet::new(),
            closed: false,
        }
    }

    /// Records what already exists under the root. Runs once, before the first OS event is
    /// processed, so every event for a pre-existing path is classified against the start state.
    fn scan_existing(&mut self) {
        let mut stack = vec![self.root_dir.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(rel) = self.rel_of(&path).filter(|r| wanted(r)) else {
                    continue;
                };
                let is_real_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                self.known.insert(known_key(&rel));
                if is_real_dir {
                    stack.push(path);
                }
            }
        }
    }

    fn is_known(&self, rel: &str) -> bool {
        self.known.contains(&known_key(rel))
    }

    /// Removes `key` and everything below it from the known set and returns the removed keys.
    fn take_subtree(&mut self, key: &str) -> Vec<String> {
        let prefix = format!("{key}/");
        let found: Vec<String> = self
            .known
            .range(key.to_string()..)
            .take_while(|k| k.as_str() == key || k.starts_with(&prefix))
            .cloned()
            .collect();
        for k in &found {
            self.known.remove(k);
        }
        found
    }

    /// Keeps the known set in step with what is reported.
    fn track(&mut self, event: &VaultEvent) {
        match event {
            VaultEvent::Created(rel) | VaultEvent::Modified(rel) => {
                self.known.insert(known_key(rel));
            }
            VaultEvent::Removed(rel) => {
                self.take_subtree(&known_key(rel));
            }
            VaultEvent::Renamed { from, to } => {
                let from_key = known_key(from);
                let to_key = known_key(to);
                for key in self.take_subtree(&from_key) {
                    self.known
                        .insert(format!("{to_key}{}", &key[from_key.len()..]));
                }
            }
        }
    }

    /// Vault-relative path of `path`, or `None` when it is outside the root or is the root.
    fn rel_of(&self, path: &Path) -> Option<String> {
        let full = norm_slashes(path);
        strip_root(&full, &self.root_norm)
            .filter(|rel| !rel.is_empty())
            .map(str::to_string)
    }

    fn note(&mut self, path: &Path, kind: Kind, now: Instant) {
        if let Some(rel) = self.rel_of(path).filter(|rel| wanted(rel)) {
            let kind = if kind == Kind::Created && self.is_known(&rel) {
                Kind::Modified
            } else {
                kind
            };
            self.merge(rel, kind, now);
        }
    }

    fn merge(&mut self, rel: String, kind: Kind, now: Instant) {
        let prev = self.pending.get(&rel).map(|p| p.kind);
        match merge_kinds(prev, kind) {
            Some(kind) => {
                self.pending.insert(
                    rel,
                    Pending {
                        kind,
                        last: now,
                        empty_retries: 0,
                    },
                );
            }
            None => {
                self.pending.remove(&rel);
            }
        }
    }

    fn ingest(&mut self, event: notify::Event, now: Instant) {
        match event.kind {
            EventKind::Create(_) => {
                for path in &event.paths {
                    self.note(path, Kind::Created, now);
                }
            }
            EventKind::Remove(_) => {
                for path in &event.paths {
                    self.note(path, Kind::Removed, now);
                }
            }
            EventKind::Modify(ModifyKind::Name(mode)) => {
                self.ingest_rename(mode, &event.paths, now);
            }
            EventKind::Modify(ModifyKind::Metadata(_)) | EventKind::Access(_) => {}
            EventKind::Modify(_) | EventKind::Any => {
                for path in &event.paths {
                    self.note(path, Kind::Modified, now);
                }
            }
            EventKind::Other => {}
        }
    }

    fn ingest_rename(&mut self, mode: RenameMode, paths: &[PathBuf], now: Instant) {
        match mode {
            RenameMode::Both if paths.len() >= 2 => {
                let from = self.rel_of(&paths[0]).filter(|r| wanted(r));
                let to = self.rel_of(&paths[1]).filter(|r| wanted(r));
                self.emit_rename(from, to, now);
            }
            RenameMode::From => {
                if let Some(first) = paths.first() {
                    self.expire_rename_from(now);
                    self.rename_from = self.rel_of(first).filter(|r| wanted(r)).map(|r| (r, now));
                }
            }
            RenameMode::To => {
                let Some(first) = paths.first() else {
                    return;
                };
                let to = self.rel_of(first).filter(|r| wanted(r));
                let from = match self.rename_from.take() {
                    Some((rel, at)) if now.saturating_duration_since(at) <= DEBOUNCE => Some(rel),
                    Some((rel, at)) => {
                        self.merge(rel, Kind::Removed, at);
                        None
                    }
                    None => None,
                };
                self.emit_rename(from, to, now);
            }
            _ => {
                // FSEvents reports both sides of a rename as unpaired `Any` names: the old path
                // is gone, the new one exists (a create unless it is already known).
                for path in paths {
                    let kind = if path.exists() {
                        Kind::Created
                    } else {
                        Kind::Removed
                    };
                    self.note(path, kind, now);
                }
            }
        }
    }

    /// A `From` rename that never got its `To` becomes a removal.
    fn expire_rename_from(&mut self, now: Instant) {
        if let Some((rel, at)) = self.rename_from.take() {
            if now.saturating_duration_since(at) >= DEBOUNCE {
                self.merge(rel, Kind::Removed, at);
            } else {
                self.rename_from = Some((rel, at));
            }
        }
    }

    /// Both ends known: a rename. One end known: it is a create or a removal.
    fn emit_rename(&mut self, from: Option<String>, to: Option<String>, now: Instant) {
        match (from, to) {
            (Some(from), Some(to)) => {
                self.pending.remove(&from);
                self.pending.remove(&to);
                self.send(VaultEvent::Renamed { from, to });
            }
            (None, Some(to)) => self.merge(to, Kind::Created, now),
            (Some(from), None) => self.merge(from, Kind::Removed, now),
            (None, None) => {}
        }
    }

    /// Emits every pending change whose path has been quiet for [`DEBOUNCE`].
    fn flush(&mut self, now: Instant) {
        self.expire_rename_from(now);
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| now.saturating_duration_since(p.last) >= DEBOUNCE)
            .map(|(rel, _)| rel.clone())
            .collect();
        for rel in due {
            let Some(pending) = self.pending.remove(&rel) else {
                continue;
            };
            self.settle(rel, pending, now);
            if self.closed {
                return;
            }
        }
    }

    /// Checks the file system for what really happened to `rel`, then reports it.
    fn settle(&mut self, rel: String, pending: Pending, now: Instant) {
        let abs = self.root_dir.join(&rel);
        let meta = fs::metadata(&abs).ok();
        if is_expected_write(&abs) {
            // Our own write is not reported, but the known set must still match the disk.
            if meta.is_some() {
                self.known.insert(known_key(&rel));
            } else {
                self.take_subtree(&known_key(&rel));
            }
            return;
        }
        let known = self.is_known(&rel);
        let kind = match (pending.kind, meta.is_some()) {
            (Kind::Removed, true) => Kind::Modified,
            (Kind::Removed, false) => Kind::Removed,
            (Kind::Created, false) => return,
            (Kind::Modified, false) => Kind::Removed,
            (Kind::Created, true) if known => Kind::Modified,
            (kind, true) => kind,
        };
        if kind != Kind::Removed
            && let Some(meta) = &meta
            && meta.is_file()
            && meta.len() == 0
            && pending.empty_retries < MAX_EMPTY_RETRIES
        {
            self.pending.insert(
                rel,
                Pending {
                    kind,
                    last: now,
                    empty_retries: pending.empty_retries + 1,
                },
            );
            return;
        }
        let event = match kind {
            Kind::Created => VaultEvent::Created(rel),
            Kind::Modified => VaultEvent::Modified(rel),
            Kind::Removed => VaultEvent::Removed(rel),
        };
        self.send(event);
    }

    fn send(&mut self, event: VaultEvent) {
        self.track(&event);
        if self.out.send(event).is_err() {
            self.closed = true;
        }
    }
}

fn run(mut pipeline: Pipeline, raw: Receiver<notify::Result<notify::Event>>) {
    pipeline.scan_existing();
    loop {
        match raw.recv_timeout(TICK) {
            Ok(Ok(event)) => pipeline.ingest(event, Instant::now()),
            // An OS-level watch error (for example an overflowed buffer) carries no path to report.
            Ok(Err(_)) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                pipeline.flush(Instant::now() + DEBOUNCE);
                break;
            }
        }
        pipeline.flush(Instant::now());
        if pipeline.closed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, RemoveKind};

    /// A scratch vault under the system temp dir, removed on drop.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("jiyue-test-{}-watch-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create sandbox");
            Sandbox(dir)
        }

        fn root(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Upper bound for any wait on the OS watcher. notify 8.2.1 starts FSEvents with latency 0
    /// (`NoDefer`), so events arrive quickly, but the stream starts asynchronously and the kernel
    /// may still deliver flags for a write made just before the watch began. Waits end as soon as
    /// their condition holds and only fail after this long, which leaves room for slow runners.
    const WAIT: Duration = Duration::from_secs(10);
    /// Folder created by [`start`] to prove the OS watcher is live. Its event is consumed there.
    const PROBE: &str = "__watch_probe";

    fn event_rel(ev: &VaultEvent) -> Vec<&str> {
        match ev {
            VaultEvent::Created(r) | VaultEvent::Modified(r) | VaultEvent::Removed(r) => {
                vec![r.as_str()]
            }
            VaultEvent::Renamed { from, to } => vec![from.as_str(), to.as_str()],
        }
    }

    fn collect_for(rx: &Receiver<VaultEvent>, window: Duration) -> Vec<VaultEvent> {
        let deadline = Instant::now() + window;
        let mut out = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return out;
            }
            match rx.recv_timeout(left) {
                Ok(ev) => out.push(ev),
                Err(_) => return out,
            }
        }
    }

    fn wait_for(
        rx: &Receiver<VaultEvent>,
        timeout: Duration,
        pred: impl Fn(&VaultEvent) -> bool,
    ) -> Option<VaultEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            match rx.recv_timeout(left) {
                Ok(ev) if pred(&ev) => return Some(ev),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    fn mentions(ev: &VaultEvent, rel: &str) -> bool {
        event_rel(ev).contains(&rel)
    }

    /// Collects events until `done` holds for what has arrived so far, or `timeout` passes.
    fn collect_until(
        rx: &Receiver<VaultEvent>,
        timeout: Duration,
        done: impl Fn(&[VaultEvent]) -> bool,
    ) -> Vec<VaultEvent> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        while !done(&out) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(ev) => out.push(ev),
                Err(_) => break,
            }
        }
        out
    }

    /// Starts the OS watcher and returns once it demonstrably reports events. FSEvents starts its
    /// stream asynchronously, so a probe folder is created and its event awaited before a test
    /// changes anything it wants to observe.
    fn start(root: &Path) -> (VaultWatcher, Receiver<VaultEvent>) {
        let (tx, rx) = mpsc::channel();
        let watcher = VaultWatcher::start(root, tx).expect("start watcher");
        fs::create_dir_all(root.join(PROBE)).expect("probe folder");
        let ready = wait_for(&rx, WAIT, |e| mentions(e, PROBE));
        assert!(ready.is_some(), "watcher never reported the probe folder");
        (watcher, rx)
    }

    // ---- pure helpers ------------------------------------------------------

    #[test]
    fn merge_table_coalesces_bursts() {
        use Kind::*;
        assert_eq!(merge_kinds(None, Created), Some(Created));
        assert_eq!(merge_kinds(None, Modified), Some(Modified));
        assert_eq!(merge_kinds(None, Removed), Some(Removed));
        assert_eq!(merge_kinds(Some(Created), Modified), Some(Created));
        assert_eq!(merge_kinds(Some(Created), Created), Some(Created));
        assert_eq!(merge_kinds(Some(Created), Removed), None);
        assert_eq!(merge_kinds(Some(Modified), Modified), Some(Modified));
        assert_eq!(merge_kinds(Some(Modified), Removed), Some(Removed));
        assert_eq!(merge_kinds(Some(Removed), Created), Some(Modified));
        assert_eq!(merge_kinds(Some(Removed), Modified), Some(Modified));
        assert_eq!(merge_kinds(Some(Removed), Removed), Some(Removed));
    }

    #[test]
    fn expected_writes_expire_after_ttl() {
        let mut reg = ExpectedWrites::new(Duration::from_secs(2));
        let t0 = Instant::now();
        let p = Path::new("/vault/a.md");
        reg.expect_at(p, t0);
        assert!(reg.is_expected_at(p, t0 + Duration::from_millis(1500)));
        assert!(!reg.is_expected_at(p, t0 + Duration::from_millis(2500)));
        assert!(!reg.is_expected_at(p, t0), "expired entries are dropped");
    }

    #[test]
    fn expected_writes_only_match_their_own_path() {
        let mut reg = ExpectedWrites::new(Duration::from_secs(2));
        let t0 = Instant::now();
        reg.expect_at(Path::new("/vault/a.md"), t0);
        assert!(!reg.is_expected_at(Path::new("/vault/b.md"), t0));
    }

    #[test]
    fn expected_write_key_ignores_verbatim_prefix_and_separators() {
        let mut reg = ExpectedWrites::new(Duration::from_secs(2));
        let t0 = Instant::now();
        reg.expect_at(Path::new(r"\\?\C:\vault\dir\A.md"), t0);
        assert!(reg.is_expected_at(Path::new("C:/vault/dir/A.md"), t0));
        assert!(reg.is_expected_at(Path::new(r"C:\vault\dir\A.md"), t0));
    }

    #[test]
    fn global_expect_write_is_visible_to_watcher_check() {
        let path = std::env::temp_dir().join(format!(
            "jiyue-test-{}-registry-global.md",
            std::process::id()
        ));
        expect_write(&path);
        assert!(is_expected_write(&path));
    }

    #[test]
    fn strip_root_handles_prefixes_and_siblings() {
        assert_eq!(strip_root("a/b/c.md", "a/b"), Some("c.md"));
        assert_eq!(strip_root("a/bc/d", "a/b"), None);
        assert_eq!(strip_root("a/b", "a/b"), None);
        assert_eq!(strip_root("x/y", "a/b"), None);
        assert_eq!(strip_root("a", "a/b"), None);
    }

    #[test]
    fn norm_slashes_strips_verbatim_and_trailing_slash() {
        assert_eq!(norm_slashes(Path::new(r"\\?\D:\vault\")), "D:/vault");
        assert_eq!(norm_slashes(Path::new("/unix/vault/")), "/unix/vault");
    }

    #[test]
    fn strip_verbatim_only_for_drive_paths() {
        assert_eq!(strip_verbatim(Path::new("/x/y")), PathBuf::from("/x/y"));
        #[cfg(windows)]
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\D:\vault")),
            PathBuf::from(r"D:\vault")
        );
    }

    #[test]
    fn editor_temp_files_are_recognised() {
        for temp in [
            "note.md~",
            "~$doc.docx",
            ".note.md.swp",
            "note.md.swx",
            "sub/x.tmp",
            "4913",
            "a/old.bak",
            "dl.crdownload",
        ] {
            assert!(is_editor_temp(temp), "{temp} should be temp");
        }
        for real in ["note.md", "4914.md", "tilde~inside.md", "日记.md"] {
            assert!(!is_editor_temp(real), "{real} should be real");
        }
    }

    #[test]
    fn wanted_drops_internal_and_temp_paths() {
        assert!(!wanted(".trash/x.md"));
        assert!(!wanted("a/.hidden.md"));
        assert!(!wanted("a/b.tmp"));
        assert!(wanted("想法/沉积.md"));
    }

    // ---- deterministic pipeline tests (no OS watcher) -----------------------

    fn pipeline(sb: &Sandbox) -> (Pipeline, Receiver<VaultEvent>) {
        let (tx, rx) = mpsc::channel();
        (Pipeline::new(sb.root(), tx), rx)
    }

    fn ev(kind: EventKind, paths: &[PathBuf]) -> notify::Event {
        let mut event = notify::Event::new(kind);
        for p in paths {
            event = event.add_path(p.clone());
        }
        event
    }

    fn drain(rx: &Receiver<VaultEvent>) -> Vec<VaultEvent> {
        rx.try_iter().collect()
    }

    #[test]
    fn pipeline_coalesces_create_then_modify() {
        let sb = Sandbox::new("pipe-coalesce");
        fs::write(sb.root().join("a.md"), "x").expect("file");
        let (mut p, rx) = pipeline(&sb);
        let path = vec![sb.root().join("a.md")];
        let t0 = Instant::now();
        p.ingest(ev(EventKind::Create(CreateKind::File), &path), t0);
        p.ingest(
            ev(EventKind::Modify(ModifyKind::Any), &path),
            t0 + Duration::from_millis(10),
        );
        p.flush(t0 + Duration::from_millis(100));
        assert!(drain(&rx).is_empty(), "still inside the debounce window");
        p.flush(t0 + Duration::from_millis(400));
        assert_eq!(drain(&rx), vec![VaultEvent::Created("a.md".to_string())]);
    }

    #[test]
    fn pipeline_create_then_remove_is_silent() {
        let sb = Sandbox::new("pipe-cancel");
        let (mut p, rx) = pipeline(&sb);
        let path = vec![sb.root().join("tmp-like.md")];
        let t0 = Instant::now();
        p.ingest(ev(EventKind::Create(CreateKind::File), &path), t0);
        p.ingest(ev(EventKind::Remove(RemoveKind::File), &path), t0);
        p.flush(t0 + Duration::from_secs(1));
        assert!(drain(&rx).is_empty());
    }

    #[test]
    fn pipeline_remove_then_create_is_modified() {
        let sb = Sandbox::new("pipe-replace");
        fs::write(sb.root().join("n.md"), "new").expect("file");
        let (mut p, rx) = pipeline(&sb);
        let path = vec![sb.root().join("n.md")];
        let t0 = Instant::now();
        p.ingest(ev(EventKind::Remove(RemoveKind::File), &path), t0);
        p.ingest(ev(EventKind::Create(CreateKind::File), &path), t0);
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Modified("n.md".to_string())]);
    }

    #[test]
    fn pipeline_removed_file_that_still_exists_is_modified() {
        let sb = Sandbox::new("pipe-exists");
        fs::write(sb.root().join("n.md"), "still here").expect("file");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Remove(RemoveKind::File),
                &[sb.root().join("n.md")],
            ),
            t0,
        );
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Modified("n.md".to_string())]);
    }

    #[test]
    fn pipeline_modified_then_missing_is_removed() {
        let sb = Sandbox::new("pipe-gone");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Any),
                &[sb.root().join("gone.md")],
            ),
            t0,
        );
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Removed("gone.md".to_string())]);
    }

    #[test]
    fn pipeline_rename_both_is_one_event() {
        let sb = Sandbox::new("pipe-rename-both");
        let (mut p, rx) = pipeline(&sb);
        let paths = vec![sb.root().join("a.md"), sb.root().join("b.md")];
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                &paths,
            ),
            Instant::now(),
        );
        assert_eq!(
            drain(&rx),
            vec![VaultEvent::Renamed {
                from: "a.md".to_string(),
                to: "b.md".to_string()
            }]
        );
    }

    #[test]
    fn pipeline_rename_from_then_to_pairs_up() {
        let sb = Sandbox::new("pipe-rename-pair");
        fs::write(sb.root().join("b.md"), "x").expect("file");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                &[sb.root().join("a.md")],
            ),
            t0,
        );
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                &[sb.root().join("b.md")],
            ),
            t0 + Duration::from_millis(5),
        );
        assert_eq!(
            drain(&rx),
            vec![VaultEvent::Renamed {
                from: "a.md".to_string(),
                to: "b.md".to_string()
            }]
        );
    }

    #[test]
    fn pipeline_unknown_from_is_removal_and_to_is_creation() {
        let sb = Sandbox::new("pipe-rename-unknown");
        fs::write(sb.root().join("new.md"), "x").expect("file");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                &[sb.root().join("new.md")],
            ),
            t0,
        );
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Created("new.md".to_string())]);
    }

    #[test]
    fn pipeline_from_without_to_becomes_removal() {
        let sb = Sandbox::new("pipe-rename-from-only");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                &[sb.root().join("lost.md")],
            ),
            t0,
        );
        assert!(drain(&rx).is_empty());
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Removed("lost.md".to_string())]);
    }

    #[test]
    fn pipeline_rename_into_ignored_folder_is_removal() {
        let sb = Sandbox::new("pipe-rename-trash");
        let (mut p, rx) = pipeline(&sb);
        let paths = vec![sb.root().join("a.md"), sb.root().join(".trash/a.md")];
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                &paths,
            ),
            Instant::now(),
        );
        p.flush(Instant::now() + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Removed("a.md".to_string())]);
    }

    #[test]
    fn pipeline_rename_from_ignored_is_creation() {
        let sb = Sandbox::new("pipe-rename-from-tmp");
        fs::write(sb.root().join("final.md"), "x").expect("file");
        let (mut p, rx) = pipeline(&sb);
        let paths = vec![sb.root().join("draft.tmp"), sb.root().join("final.md")];
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                &paths,
            ),
            Instant::now(),
        );
        assert_eq!(drain(&rx), vec![]);
        p.flush(Instant::now() + Duration::from_secs(1));
        assert_eq!(
            drain(&rx),
            vec![VaultEvent::Created("final.md".to_string())]
        );
    }

    #[test]
    fn pipeline_zero_length_file_is_retried_then_reported() {
        let sb = Sandbox::new("pipe-empty");
        fs::write(sb.root().join("e.md"), "").expect("empty file");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Any),
                &[sb.root().join("e.md")],
            ),
            t0,
        );
        let mut seen = Vec::new();
        for step in 1..=6u64 {
            p.flush(t0 + Duration::from_millis(200 * step));
            seen.extend(drain(&rx));
            if !seen.is_empty() {
                break;
            }
        }
        assert_eq!(seen, vec![VaultEvent::Modified("e.md".to_string())]);
    }

    #[test]
    fn pipeline_zero_length_then_content_reports_once_with_content() {
        let sb = Sandbox::new("pipe-empty-then-data");
        fs::write(sb.root().join("e.md"), "").expect("empty");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Any),
                &[sb.root().join("e.md")],
            ),
            t0,
        );
        p.flush(t0 + Duration::from_millis(250));
        assert!(drain(&rx).is_empty(), "empty file is held back");
        fs::write(sb.root().join("e.md"), "filled").expect("fill");
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Any),
                &[sb.root().join("e.md")],
            ),
            t0 + Duration::from_millis(300),
        );
        p.flush(t0 + Duration::from_millis(700));
        assert_eq!(drain(&rx), vec![VaultEvent::Modified("e.md".to_string())]);
    }

    #[test]
    fn pipeline_suppresses_registered_self_writes() {
        let sb = Sandbox::new("pipe-self");
        let path = sb.root().join("own.md");
        fs::write(&path, "mine").expect("file");
        expect_write(&path);
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Any),
                std::slice::from_ref(&path),
            ),
            t0,
        );
        p.flush(t0 + Duration::from_secs(1));
        assert!(drain(&rx).is_empty());
    }

    #[test]
    fn pipeline_ignores_internal_and_outside_paths() {
        let sb = Sandbox::new("pipe-ignore");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        let outside = std::env::temp_dir().join("jiyue-outside-root.md");
        for path in [
            sb.root().join(".trash/1-x.md"),
            sb.root().join(".obsidian/app.json"),
            sb.root().join("a.tmp"),
            sb.root().join("~$doc.md"),
            sb.root().join(".hidden.md"),
            outside,
        ] {
            p.ingest(ev(EventKind::Create(CreateKind::File), &[path]), t0);
        }
        p.flush(t0 + Duration::from_secs(1));
        assert!(drain(&rx).is_empty());
    }

    #[test]
    fn pipeline_reports_folders_and_skips_metadata_and_access() {
        let sb = Sandbox::new("pipe-folder");
        fs::create_dir_all(sb.root().join("newdir")).expect("dir");
        let (mut p, rx) = pipeline(&sb);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Create(CreateKind::Folder),
                &[sb.root().join("newdir")],
            ),
            t0,
        );
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Metadata(notify::event::MetadataKind::Any)),
                &[sb.root().join("newdir")],
            ),
            t0,
        );
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Created("newdir".to_string())]);
    }

    #[test]
    fn pipeline_stops_when_receiver_is_gone() {
        let sb = Sandbox::new("pipe-closed");
        fs::write(sb.root().join("a.md"), "x").expect("file");
        let (mut p, rx) = pipeline(&sb);
        drop(rx);
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Create(CreateKind::File),
                &[sb.root().join("a.md")],
            ),
            t0,
        );
        p.flush(t0 + Duration::from_secs(1));
        assert!(p.closed);
    }

    #[test]
    fn pipeline_create_then_modify_of_known_file_is_modified() {
        // FSEvents reports a write to an existing file as Create(File) plus Modify(Data) when the
        // file was written just before the watch started. It must still be a modification.
        let sb = Sandbox::new("pipe-known-create");
        fs::write(sb.root().join("m.md"), "one").expect("file");
        let (mut p, rx) = pipeline(&sb);
        p.scan_existing();
        let t0 = Instant::now();
        let path = vec![sb.root().join("m.md")];
        p.ingest(ev(EventKind::Create(CreateKind::File), &path), t0);
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &path,
            ),
            t0 + Duration::from_millis(5),
        );
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Modified("m.md".to_string())]);
    }

    #[test]
    fn pipeline_known_file_created_again_after_removal_is_created() {
        let sb = Sandbox::new("pipe-known-removed");
        fs::write(sb.root().join("k.md"), "one").expect("file");
        let (mut p, rx) = pipeline(&sb);
        p.scan_existing();
        let t0 = Instant::now();
        fs::remove_file(sb.root().join("k.md")).expect("gone");
        p.ingest(
            ev(
                EventKind::Remove(RemoveKind::File),
                &[sb.root().join("k.md")],
            ),
            t0,
        );
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Removed("k.md".to_string())]);
        fs::write(sb.root().join("k.md"), "two").expect("new file");
        let t1 = t0 + Duration::from_secs(2);
        p.ingest(
            ev(
                EventKind::Create(CreateKind::File),
                &[sb.root().join("k.md")],
            ),
            t1,
        );
        p.flush(t1 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Created("k.md".to_string())]);
    }

    #[test]
    fn pipeline_rename_moves_known_folder_contents() {
        let sb = Sandbox::new("pipe-known-rename");
        fs::create_dir_all(sb.root().join("d")).expect("dir");
        fs::write(sb.root().join("d/x.md"), "one").expect("file");
        let (mut p, rx) = pipeline(&sb);
        p.scan_existing();
        fs::rename(sb.root().join("d"), sb.root().join("e")).expect("rename on disk");
        let t0 = Instant::now();
        p.ingest(
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                &[sb.root().join("d"), sb.root().join("e")],
            ),
            t0,
        );
        assert_eq!(
            drain(&rx),
            vec![VaultEvent::Renamed {
                from: "d".to_string(),
                to: "e".to_string()
            }]
        );
        p.ingest(
            ev(
                EventKind::Create(CreateKind::File),
                &[sb.root().join("e/x.md")],
            ),
            t0 + Duration::from_millis(5),
        );
        p.flush(t0 + Duration::from_secs(1));
        assert_eq!(drain(&rx), vec![VaultEvent::Modified("e/x.md".to_string())]);
    }

    #[test]
    fn known_keys_are_case_folded_only_where_the_file_system_is() {
        #[cfg(any(windows, target_os = "macos"))]
        assert_eq!(known_key("Dir/A.md"), "dir/a.md");
        #[cfg(not(any(windows, target_os = "macos")))]
        assert_eq!(known_key("Dir/A.md"), "Dir/A.md");
    }

    // ---- integration tests with the OS watcher -----------------------------

    #[test]
    fn watch_reports_created_file() {
        let sb = Sandbox::new("int-create");
        let (_w, rx) = start(sb.root());
        fs::write(sb.root().join("a.md"), "hello").expect("write");
        let ev = wait_for(&rx, Duration::from_secs(10), |e| mentions(e, "a.md"));
        assert!(
            matches!(
                ev,
                Some(VaultEvent::Created(_)) | Some(VaultEvent::Modified(_))
            ),
            "got {ev:?}"
        );
    }

    #[test]
    fn watch_reports_created_file_in_subfolder() {
        let sb = Sandbox::new("int-create-sub");
        fs::create_dir_all(sb.root().join("想法")).expect("dir");
        let (_w, rx) = start(sb.root());
        fs::write(sb.root().join("想法/沉积.md"), "x").expect("write");
        let ev = wait_for(&rx, WAIT, |e| mentions(e, "想法/沉积.md"));
        assert!(ev.is_some());
    }

    #[test]
    fn watch_reports_modified_file() {
        // The file exists before the watcher starts, so it is in the known set and any event for
        // it is normalised to Modified, whatever mix of create and modify the OS delivers.
        let sb = Sandbox::new("int-modify");
        fs::write(sb.root().join("m.md"), "one").expect("seed");
        let (_w, rx) = start(sb.root());
        fs::write(sb.root().join("m.md"), "two, longer").expect("modify");
        let ev = wait_for(&rx, WAIT, |e| mentions(e, "m.md"));
        assert_eq!(
            ev,
            Some(VaultEvent::Modified("m.md".to_string())),
            "external write to an existing note"
        );
    }

    #[test]
    fn watch_reports_removed_file() {
        let sb = Sandbox::new("int-remove");
        fs::write(sb.root().join("r.md"), "bye").expect("seed");
        let (_w, rx) = start(sb.root());
        fs::remove_file(sb.root().join("r.md")).expect("remove");
        let ev = wait_for(
            &rx,
            Duration::from_secs(10),
            |e| matches!(e, VaultEvent::Removed(r) if r == "r.md"),
        );
        assert!(ev.is_some(), "expected Removed(r.md)");
    }

    #[test]
    fn watch_reports_rename_as_pair_or_remove_and_create() {
        let sb = Sandbox::new("int-rename");
        fs::write(sb.root().join("old.md"), "x").expect("seed");
        let (_w, rx) = start(sb.root());
        fs::rename(sb.root().join("old.md"), sb.root().join("new.md")).expect("rename");
        // A paired rename arrives at once; an unpaired one (FSEvents) is a removal of the old path
        // plus a create of the new one, which settles after the debounce.
        let is_rename = |evs: &[VaultEvent]| {
            let renamed = evs.iter().any(
                |e| {
                    matches!(e, VaultEvent::Renamed { from, to } if from == "old.md" && to == "new.md")
                },
            );
            let split = evs
                .iter()
                .any(|e| matches!(e, VaultEvent::Removed(r) if r == "old.md"))
                && evs.iter().any(
                    |e| {
                        matches!(e, VaultEvent::Created(r) | VaultEvent::Modified(r) if r == "new.md")
                    },
                );
            renamed || split
        };
        let events = collect_until(&rx, WAIT, is_rename);
        assert!(is_rename(&events), "unexpected events: {events:?}");
    }

    #[test]
    fn watch_reports_created_folder() {
        let sb = Sandbox::new("int-folder");
        let (_w, rx) = start(sb.root());
        fs::create_dir_all(sb.root().join("projects")).expect("dir");
        let ev = wait_for(&rx, Duration::from_secs(10), |e| mentions(e, "projects"));
        assert!(ev.is_some());
    }

    #[test]
    fn watch_reports_atomic_save_pattern() {
        let sb = Sandbox::new("int-atomic");
        fs::write(sb.root().join("doc.md"), "v1").expect("seed");
        let (_w, rx) = start(sb.root());
        fs::write(sb.root().join("doc.md.part"), "v2").expect("temp");
        fs::rename(sb.root().join("doc.md.part"), sb.root().join("doc.md")).expect("swap");
        // The note still exists afterwards, so the outcome is a modification of doc.md or a
        // rename onto it, never a removal.
        let ev = wait_for(&rx, WAIT, |e| {
            matches!(e, VaultEvent::Modified(r) if r == "doc.md")
                || matches!(e, VaultEvent::Renamed { to, .. } if to == "doc.md")
        });
        assert!(ev.is_some(), "atomic save of doc.md not reported");
    }

    #[test]
    fn watch_does_not_report_own_atomic_writes() {
        let sb = Sandbox::new("int-self");
        let (_w, rx) = start(sb.root());
        let own = sb.root().join("own.md");
        fsops::write_atomic(&own, b"from the app").expect("atomic write");
        // A sentinel written after the own write marks the point up to which the OS has
        // delivered events: own.md settles no later than the sentinel, so once the sentinel is
        // reported, any event for own.md has been handled.
        fs::write(sb.root().join("sentinel.md"), "x").expect("sentinel");
        let mut events = collect_until(&rx, WAIT, |evs| {
            evs.iter().any(|e| mentions(e, "sentinel.md"))
        });
        assert!(
            events.iter().any(|e| mentions(e, "sentinel.md")),
            "sentinel never reported: {events:?}"
        );
        events.extend(collect_for(&rx, Duration::from_millis(500)));
        assert!(
            !events.iter().any(|e| mentions(e, "own.md")),
            "own write leaked: {events:?}"
        );
    }

    #[test]
    fn watch_still_reports_external_writes_after_own_write() {
        let sb = Sandbox::new("int-mixed");
        let (_w, rx) = start(sb.root());
        fsops::write_atomic(&sb.root().join("own.md"), b"mine").expect("own");
        fs::write(sb.root().join("other.md"), "theirs").expect("external");
        let ev = wait_for(&rx, Duration::from_secs(10), |e| mentions(e, "other.md"));
        assert!(ev.is_some());
    }

    #[test]
    fn watch_skips_trash_temp_and_hidden_files() {
        let sb = Sandbox::new("int-ignored");
        let (_w, rx) = start(sb.root());
        fs::create_dir_all(sb.root().join(".trash")).expect("trash dir");
        fs::write(sb.root().join(".trash/1-x.md"), "x").expect("trash item");
        fs::write(sb.root().join("draft.tmp"), "x").expect("tmp");
        fs::write(sb.root().join("~$lock.md"), "x").expect("lock");
        fs::write(sb.root().join(".hidden.md"), "x").expect("hidden");
        fs::write(sb.root().join("visible.md"), "v").expect("visible");
        let mut events = collect_until(&rx, WAIT, |evs| {
            evs.iter().any(|e| mentions(e, "visible.md"))
        });
        assert!(
            events.iter().any(|e| mentions(e, "visible.md")),
            "{events:?}"
        );
        events.extend(collect_for(&rx, Duration::from_millis(800)));
        for bad in [
            ".trash",
            ".trash/1-x.md",
            "draft.tmp",
            "~$lock.md",
            ".hidden.md",
        ] {
            assert!(
                !events.iter().any(|e| mentions(e, bad)),
                "{bad} leaked: {events:?}"
            );
        }
    }

    #[test]
    fn watch_coalesces_rapid_saves_into_one_event() {
        let sb = Sandbox::new("int-burst");
        fs::write(sb.root().join("b.md"), "0").expect("seed");
        let (_w, rx) = start(sb.root());
        for i in 1..=10 {
            fs::write(sb.root().join("b.md"), format!("{i}")).expect("burst");
        }
        // Ten writes must not produce ten events. The OS may split the burst over more than one
        // delivery on a slow runner, so the bound is loose: far fewer than the writes.
        let mut events = collect_until(&rx, WAIT, |evs| evs.iter().any(|e| mentions(e, "b.md")));
        events.extend(collect_for(&rx, Duration::from_millis(1500)));
        let count = events.iter().filter(|e| mentions(e, "b.md")).count();
        assert!(
            (1..=5).contains(&count),
            "expected coalescing, got {count}: {events:?}"
        );
    }

    #[test]
    fn watch_stops_when_dropped() {
        let sb = Sandbox::new("int-drop");
        let (tx, rx) = mpsc::channel();
        let watcher = VaultWatcher::start(sb.root(), tx).expect("start");
        drop(watcher);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "channel never disconnected after drop");
            match rx.recv_timeout(left) {
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => panic!("channel never disconnected after drop"),
                Ok(_) => continue,
            }
        }
    }

    #[test]
    fn watch_missing_root_is_an_error() {
        let missing =
            std::env::temp_dir().join(format!("jiyue-test-{}-watch-no-root", std::process::id()));
        let _ = fs::remove_dir_all(&missing);
        let (tx, _rx) = mpsc::channel();
        assert!(matches!(
            VaultWatcher::start(&missing, tx),
            Err(WatchError::Io(_))
        ));
    }

    #[test]
    fn watch_error_display_mentions_cause() {
        assert!(WatchError::Io("disk".into()).to_string().contains("disk"));
        assert!(
            WatchError::Notify("inotify".into())
                .to_string()
                .contains("inotify")
        );
    }
}
