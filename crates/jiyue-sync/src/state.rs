//! Local sync state: ignore rules, vault scanning, persistence in `.jiyue/sync`, the merge-base
//! cache, device id, conflict-copy names and text/binary detection. Owned by W1-D2.
//!
//! On disk (all under `<vault>/.jiyue/sync/`):
//! - `state.json`: [`SyncState`] (version, per-path base entries, scan cache). Written atomically.
//! - `base/<blake3 hex>`: content of text files at their last synced version (<= 2 MB each), used
//!   as the merge base. Pruned after every run.
//! - `device_id`: stable random id of this device, created on first use.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{BaseEntry, LocalEntry, LocalSnapshot, STATE_VERSION, ScanCacheEntry, SyncState};

/// Patterns that are ignored unless the user removes them (they are not removable from code).
pub const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
    ".jiyue/",
    ".trash/",
    ".git/",
    ".obsidian/workspace*.json",
    ".obsidian/cache",
    "*.tmp",
    "~$*",
    ".DS_Store",
    "Thumbs.db",
];

/// Largest text file whose content is kept as a merge base.
pub const BASE_CACHE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// Files modified this recently are re-hashed even when size and mtime match the cache, because
/// two writes within one timestamp tick could otherwise go unnoticed.
const RACY_WINDOW_MS: i64 = 2_000;

/// Extensions treated as text (eligible for three-way merge). Anything else is binary.
const TEXT_EXTENSIONS: &[&str] = &[
    "md",
    "markdown",
    "txt",
    "text",
    "json",
    "yaml",
    "yml",
    "toml",
    "csv",
    "tsv",
    "css",
    "js",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "jsx",
    "html",
    "htm",
    "xml",
    "svg",
    "rs",
    "py",
    "sh",
    "ps1",
    "bat",
    "ini",
    "cfg",
    "conf",
    "log",
    "tex",
    "org",
    "adoc",
    "rst",
    "sql",
    "env",
    "properties",
    "gitignore",
];

// ---------------------------------------------------------------------------------------------
// Ignore rules
// ---------------------------------------------------------------------------------------------

/// Ignore rules for vault paths. Syntax (gitignore-like, simplified):
/// - `name` without `/` matches a file or folder with that name at any depth.
/// - `a/b` (contains `/` or starts with `/`) is anchored at the vault root.
/// - a trailing `/` makes the pattern match folders only.
/// - `*` matches within one path segment, `**` across segments, `?` one character.
/// - Lines starting with `#` and blank lines are ignored. Negation is not supported.
///
/// `.jiyue` and `.trash` components are always ignored, whatever the patterns say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRules {
    patterns: Vec<IgnorePattern>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IgnorePattern {
    text: String,
    glob: Vec<char>,
    anchored: bool,
    dir_only: bool,
}

impl IgnorePattern {
    fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        let dir_only = trimmed.ends_with('/');
        let body = trimmed.trim_end_matches('/');
        let (body, lead_slash) = match body.strip_prefix('/') {
            Some(rest) => (rest, true),
            None => (body, false),
        };
        if body.is_empty() {
            return None;
        }
        Some(Self {
            text: trimmed.to_owned(),
            glob: body.chars().collect(),
            anchored: lead_slash || body.contains('/'),
            dir_only,
        })
    }

    fn matches(&self, comps: &[&str], is_dir: bool) -> bool {
        let n = comps.len();
        // A directory-only pattern never matches the last component of a file path.
        let last_counts = is_dir || !self.dir_only;
        if self.anchored {
            (1..=n).any(|k| (k < n || last_counts) && glob_match(&self.glob, &comps[..k].join("/")))
        } else {
            comps
                .iter()
                .enumerate()
                .any(|(i, c)| (i + 1 < n || last_counts) && glob_match(&self.glob, c))
        }
    }
}

impl IgnoreRules {
    /// Built-in defaults plus the user's patterns (one per entry; blank and `#` lines skipped).
    pub fn new<S: AsRef<str>>(user_patterns: &[S]) -> Self {
        let patterns = DEFAULT_IGNORE_PATTERNS
            .iter()
            .copied()
            .chain(user_patterns.iter().map(|s| s.as_ref()))
            .filter_map(IgnorePattern::parse)
            .collect();
        Self { patterns }
    }

    /// The pattern texts in effect, defaults first.
    pub fn pattern_texts(&self) -> Vec<String> {
        self.patterns.iter().map(|p| p.text.clone()).collect()
    }

    /// Whether the vault-relative path (with `/` separators) is ignored.
    pub fn is_ignored(&self, rel: &str, is_dir: bool) -> bool {
        let comps: Vec<&str> = rel.split('/').filter(|c| !c.is_empty()).collect();
        if comps.is_empty() {
            return false;
        }
        if comps.iter().any(|c| *c == ".jiyue" || *c == ".trash") {
            return true;
        }
        self.patterns.iter().any(|p| p.matches(&comps, is_dir))
    }
}

impl Default for IgnoreRules {
    fn default() -> Self {
        Self::new::<&str>(&[])
    }
}

/// Matches `text` against a glob. `*` does not cross `/`, `**` does, `?` matches one non-`/`
/// character. `**/` may also match zero directories.
pub(crate) fn glob_match(pat: &[char], text: &str) -> bool {
    let txt: Vec<char> = text.chars().collect();
    let (m, n) = (pat.len(), txt.len());
    // dp[p][t]: the pattern suffix starting at p matches the text suffix starting at t.
    let mut dp = vec![vec![false; n + 1]; m + 1];
    dp[m][n] = true;
    for p in (0..m).rev() {
        for t in (0..=n).rev() {
            dp[p][t] = match pat[p] {
                '*' if p + 1 < m && pat[p + 1] == '*' => {
                    let q = p + 2;
                    let zero = dp[q][t] || (q < m && pat[q] == '/' && dp[q + 1][t]);
                    zero || (t < n && dp[p][t + 1])
                }
                '*' => dp[p + 1][t] || (t < n && txt[t] != '/' && dp[p][t + 1]),
                '?' => t < n && txt[t] != '/' && dp[p + 1][t + 1],
                c => t < n && txt[t] == c && dp[p + 1][t + 1],
            };
        }
    }
    dp[0][0]
}

// ---------------------------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------------------------

/// Scans the vault without a hash cache: every file is read and hashed.
pub fn scan_local(root: &Path, rules: &IgnoreRules) -> LocalSnapshot {
    let mut unused = BTreeMap::new();
    scan_inner(root, rules, None, &mut unused)
}

/// Scans the vault, reusing hashes from `cache` for files whose size and mtime did not change.
/// On return `cache` holds exactly the files that were seen in this scan.
pub fn scan_local_cached(
    root: &Path,
    rules: &IgnoreRules,
    cache: &mut BTreeMap<String, ScanCacheEntry>,
) -> LocalSnapshot {
    let old = std::mem::take(cache);
    scan_inner(root, rules, Some(&old), cache)
}

fn scan_inner(
    root: &Path,
    rules: &IgnoreRules,
    old: Option<&BTreeMap<String, ScanCacheEntry>>,
    new_cache: &mut BTreeMap<String, ScanCacheEntry>,
) -> LocalSnapshot {
    let mut snap = LocalSnapshot::default();
    let now = now_ms();
    let mut stack: Vec<String> = vec![String::new()];
    while let Some(dir_rel) = stack.pop() {
        let dir_abs = if dir_rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(&dir_rel)
        };
        let dir_key = if dir_rel.is_empty() {
            String::new()
        } else {
            format!("{dir_rel}/")
        };
        let read = match fs::read_dir(&dir_abs) {
            Ok(read) => read,
            Err(_) => {
                snap.unreadable.insert(dir_key);
                continue;
            }
        };
        for entry in read {
            // A failing entry means we do not know what is inside this folder: protect the folder.
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    snap.unreadable.insert(dir_key.clone());
                    continue;
                }
            };
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => {
                    snap.unreadable.insert(dir_key.clone());
                    continue;
                }
            };
            let rel = if dir_rel.is_empty() {
                name
            } else {
                format!("{dir_rel}/{name}")
            };
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => {
                    snap.unreadable.insert(rel);
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }
            let is_dir = file_type.is_dir();
            if rules.is_ignored(&rel, is_dir) {
                continue;
            }
            if is_dir {
                stack.push(rel);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    snap.unreadable.insert(rel);
                    continue;
                }
            };
            let size = meta.len();
            let mtime_ms = meta.modified().map(system_time_ms).unwrap_or(0);
            let cached = old
                .and_then(|c| c.get(&rel))
                .filter(|c| {
                    c.size == size && c.mtime_ms == mtime_ms && mtime_ms < now - RACY_WINDOW_MS
                })
                .map(|c| c.hash.clone());
            let hash = match cached {
                Some(hash) => hash,
                None => match fs::read(entry.path()) {
                    Ok(bytes) => hash_bytes(&bytes),
                    Err(_) => {
                        snap.unreadable.insert(rel);
                        continue;
                    }
                },
            };
            new_cache.insert(
                rel.clone(),
                ScanCacheEntry {
                    size,
                    mtime_ms,
                    hash: hash.clone(),
                },
            );
            snap.files.insert(rel.clone(), hash.clone());
            snap.entries.insert(
                rel,
                LocalEntry {
                    hash,
                    size,
                    mtime_ms,
                },
            );
        }
    }
    snap
}

// ---------------------------------------------------------------------------------------------
// State persistence
// ---------------------------------------------------------------------------------------------

/// `<root>/.jiyue/sync`.
pub fn sync_dir(root: &Path) -> PathBuf {
    root.join(".jiyue").join("sync")
}

/// `<root>/.jiyue/sync/state.json`.
pub fn state_path(root: &Path) -> PathBuf {
    sync_dir(root).join("state.json")
}

/// Loads the sync state. A missing file gives an empty state. A corrupt file or one written by a
/// newer version is an error: resetting silently would make every file look new.
pub fn load_state(root: &Path) -> io::Result<SyncState> {
    match fs::read(state_path(root)) {
        Ok(bytes) => {
            let state: SyncState = serde_json::from_slice(&bytes).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("sync state is corrupt ({e}); move .jiyue/sync/state.json aside to rebuild it"),
                )
            })?;
            if state.version > STATE_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "sync state version {} is newer than {STATE_VERSION}",
                        state.version
                    ),
                ));
            }
            Ok(state)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(SyncState {
            version: STATE_VERSION,
            ..SyncState::default()
        }),
        Err(e) => Err(e),
    }
}

/// Saves the sync state atomically (temp file, then rename).
pub fn save_state(root: &Path, state: &mut SyncState) -> io::Result<()> {
    state.version = STATE_VERSION;
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| io::Error::other(e.to_string()))?;
    let path = state_path(root);
    // Quiet syncs leave the state unchanged; skipping the rewrite saves a file replacement.
    if fs::read(&path).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    write_atomic(&path, &bytes)
}

/// Writes `data` to `path` through a temp file in the same folder, so readers never see a partial
/// file. The temp name ends in `.tmp`, which the default ignore rules skip.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(".jiyue-sync.tmp");
    path.with_file_name(name)
}

// ---------------------------------------------------------------------------------------------
// Base cache
// ---------------------------------------------------------------------------------------------

/// `<root>/.jiyue/sync/base`.
pub fn base_cache_dir(root: &Path) -> PathBuf {
    sync_dir(root).join("base")
}

/// Whether a file of this path and size should be kept as a merge base.
pub fn wants_base_cache(path: &str, len: usize) -> bool {
    is_text_path(path) && len <= BASE_CACHE_MAX_BYTES
}

/// Stores content under its hash. The caller checks [`wants_base_cache`] first.
pub fn write_base_cache(root: &Path, hash: &str, data: &[u8]) -> io::Result<()> {
    if !is_valid_hash(hash) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad hash"));
    }
    let path = base_cache_dir(root).join(hash);
    // Content-addressed: an existing file with the same bytes needs no rewrite.
    if fs::read(&path).is_ok_and(|current| current == data) {
        return Ok(());
    }
    write_atomic(&path, data)
}

/// Reads cached base content; `None` when missing or when its bytes no longer match the hash.
pub fn read_base_cache(root: &Path, hash: &str) -> Option<Vec<u8>> {
    if !is_valid_hash(hash) {
        return None;
    }
    let data = fs::read(base_cache_dir(root).join(hash)).ok()?;
    if hash_bytes(&data) == hash {
        Some(data)
    } else {
        None
    }
}

/// Deletes cached base files whose hash is not in `keep`.
pub fn prune_base_cache(root: &Path, keep: &BTreeSet<String>) -> io::Result<()> {
    let dir = base_cache_dir(root);
    let read = match fs::read_dir(&dir) {
        Ok(read) => read,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_valid_hash(&name) && !keep.contains(&name) {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Device id
// ---------------------------------------------------------------------------------------------

/// Returns this vault's device id, creating and persisting one on first use.
pub fn load_or_create_device_id(root: &Path) -> io::Result<String> {
    let path = sync_dir(root).join("device_id");
    if let Ok(text) = fs::read_to_string(&path) {
        let id = text.trim();
        if is_valid_device_id(id) {
            return Ok(id.to_owned());
        }
    }
    let id = generate_device_id();
    write_atomic(&path, id.as_bytes())?;
    Ok(id)
}

fn is_valid_device_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn generate_device_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_i64(now_ms());
    hasher.write_u32(std::process::id());
    let seed = format!("{:016x}-{}", hasher.finish(), std::process::id());
    let digest = blake3::hash(seed.as_bytes()).to_hex();
    digest.as_str()[..12].to_owned()
}

// ---------------------------------------------------------------------------------------------
// Names, timestamps, classification
// ---------------------------------------------------------------------------------------------

/// Conflict-copy name for `path`, e.g. `daily/note.md` becomes
/// `daily/note (冲突副本 PC 20261104-1530).md`. The folder and extension are kept.
/// `timestamp` is used verbatim; see [`format_timestamp`].
pub fn conflict_copy_name(path: &str, device: &str, timestamp: &str) -> String {
    conflict_copy_name_n(path, device, timestamp, 1)
}

/// Like [`conflict_copy_name`], with a counter for the `n`-th copy of the same name (`n >= 2`
/// adds ` n` before the closing parenthesis).
pub fn conflict_copy_name_n(path: &str, device: &str, timestamp: &str, n: u32) -> String {
    let (dir, name) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let device = sanitize_component(device);
    let suffix = if n <= 1 {
        format!("{device} {timestamp}")
    } else {
        format!("{device} {timestamp} {n}")
    };
    format!("{dir}{stem} ({CONFLICT_COPY_LABEL} {suffix}){ext}")
}

/// The label used inside conflict-copy names.
pub const CONFLICT_COPY_LABEL: &str = "冲突副本";

fn sanitize_component(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '(' | ')' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "device".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Whether the file is text (merged line by line). Unknown or missing extensions are binary.
pub fn is_text_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => {
            let ext = name[i + 1..].to_ascii_lowercase();
            TEXT_EXTENSIONS.contains(&ext.as_str())
        }
        _ => false,
    }
}

/// Accepts only vault-relative paths: no empty or `.`/`..` segments, no absolute or drive-letter
/// paths, no backslashes or control characters. Used for every path that comes from the remote.
pub fn is_safe_rel_path(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains('\\')
        && !rel.contains(':')
        && !rel.chars().any(|c| c.is_control())
        && rel
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// blake3 hash of the bytes, as lowercase hex.
pub fn hash_bytes(data: &[u8]) -> String {
    blake3::hash(data).to_hex().as_str().to_owned()
}

fn is_valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Formats Unix milliseconds as `YYYYMMDD-HHMM` in UTC, e.g. `20261104-1530`.
pub fn format_timestamp(ms: i64) -> String {
    let (y, mo, d, h, mi, _s, _ms) = civil_from_ms(ms);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}")
}

/// Formats Unix milliseconds as `YYYYMMDD-HHMMSS-mmm` in UTC (used for trash names).
pub fn format_timestamp_precise(ms: i64) -> String {
    let (y, mo, d, h, mi, s, milli) = civil_from_ms(ms);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}-{milli:03}")
}

fn civil_from_ms(ms: i64) -> (i64, i64, i64, i64, i64, i64, i64) {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let h = rem / 3_600_000;
    let mi = (rem / 60_000) % 60;
    let s = (rem / 1_000) % 60;
    let milli = rem % 1_000;
    (y, m, d, h, mi, s, milli)
}

/// Current time in Unix milliseconds.
pub fn now_ms() -> i64 {
    system_time_ms(SystemTime::now())
}

/// Converts a `SystemTime` to Unix milliseconds (negative before 1970).
pub fn system_time_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

impl SyncState {
    /// Base entry for `path`. Falls back to the hash-only `base` map for states written by older
    /// code.
    pub fn base_entry(&self, path: &str) -> Option<BaseEntry> {
        if let Some(entry) = self.entries.get(path) {
            return Some(entry.clone());
        }
        self.base.get(path).map(|hash| BaseEntry {
            hash: hash.clone(),
            etag: None,
            version_id: None,
            size: 0,
        })
    }

    /// Records `entry` as the base of `path` (both maps).
    pub fn set_base(&mut self, path: &str, entry: BaseEntry) {
        self.base.insert(path.to_owned(), entry.hash.clone());
        self.entries.insert(path.to_owned(), entry);
    }

    /// Forgets the base of `path`.
    pub fn remove_base(&mut self, path: &str) {
        self.base.remove(path);
        self.entries.remove(path);
    }
}

impl LocalSnapshot {
    /// Whether `path` lies under a folder or file the scan could not read.
    pub fn is_unreadable(&self, path: &str) -> bool {
        self.unreadable.iter().any(|u| {
            u.is_empty() || path == u || (u.ends_with('/') && path.starts_with(u.as_str()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A fresh empty folder under the system temp dir, removed on drop.
    struct TempVault(PathBuf);

    impl TempVault {
        fn new(tag: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "jiyue-sync-state-{tag}-{}-{n}-{}",
                std::process::id(),
                now_ms()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create temp vault");
            Self(dir)
        }

        fn write(&self, rel: &str, data: &[u8]) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(path, data).expect("write");
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempVault {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn rules(user: &[&str]) -> IgnoreRules {
        IgnoreRules::new(user)
    }

    #[test]
    fn default_ignore_rules_cover_the_builtin_list() {
        let r = IgnoreRules::default();
        for path in [
            ".jiyue/sync/state.json",
            ".trash/old.md",
            ".git/config",
            "sub/.git/HEAD",
            ".obsidian/workspace.json",
            ".obsidian/workspace-mobile.json",
            ".obsidian/cache",
            ".obsidian/cache/blob.bin",
            "draft.tmp",
            "sub/draft.tmp",
            "~$report.docx",
            ".DS_Store",
            "photos/Thumbs.db",
        ] {
            assert!(r.is_ignored(path, false), "{path} should be ignored");
        }
        for path in [
            "note.md",
            "daily/2026-11-04.md",
            ".obsidian/app.json",
            ".obsidian/workspace.md",
            "attachments/image.png",
            "tmpfile.md",
        ] {
            assert!(!r.is_ignored(path, false), "{path} should not be ignored");
        }
    }

    #[test]
    fn jiyue_and_trash_components_are_always_ignored() {
        let r = rules(&["!keep"]);
        assert!(r.is_ignored(".jiyue", true));
        assert!(r.is_ignored("a/.trash/x.md", false));
        assert!(r.is_ignored(".jiyue/sync/base/abc", false));
    }

    #[test]
    fn user_patterns_are_applied_after_defaults() {
        let r = rules(&[
            "drafts/**",
            "*.bak",
            "secret?.md",
            "**/private",
            "# comment",
            "   ",
        ]);
        assert!(r.is_ignored("drafts/x/y.md", false));
        assert!(r.is_ignored("drafts/x", true));
        assert!(!r.is_ignored("other/drafts.md", false));
        assert!(r.is_ignored("old.bak", false));
        assert!(r.is_ignored("deep/er/old.bak", false));
        assert!(r.is_ignored("secret1.md", false));
        assert!(!r.is_ignored("secret10.md", false));
        assert!(r.is_ignored("a/b/private", false));
        assert!(r.is_ignored("private", false));
        assert!(!r.is_ignored("privately.md", false));
    }

    #[test]
    fn anchored_pattern_only_matches_from_root() {
        let r = rules(&["/TODO.md", "notes/archive"]);
        assert!(r.is_ignored("TODO.md", false));
        assert!(!r.is_ignored("sub/TODO.md", false));
        assert!(r.is_ignored("notes/archive", true));
        assert!(r.is_ignored("notes/archive/2020.md", false));
        assert!(!r.is_ignored("x/notes/archive", true));
    }

    #[test]
    fn directory_only_pattern_ignores_folder_not_same_named_file() {
        let r = rules(&["build/"]);
        assert!(r.is_ignored("build", true));
        assert!(!r.is_ignored("build", false));
        assert!(r.is_ignored("build/out.md", false));
        assert!(r.is_ignored("sub/build/out.md", false));
    }

    #[test]
    fn glob_star_question_and_doublestar() {
        let cases: &[(&str, &str, bool)] = &[
            ("*.md", "a.md", true),
            ("*.md", "a/b.md", false),
            ("a?c", "abc", true),
            ("a?c", "a/c", false),
            ("a**c", "a/b/c", true),
            ("a*c", "a/b/c", false),
            ("**/x", "x", true),
            ("**/x", "p/q/x", true),
            ("**/x", "p/q/xy", false),
            ("a/**", "a/b/c", true),
            ("", "", true),
            ("*", "", true),
            ("?", "", false),
        ];
        for (pat, text, want) in cases {
            let chars: Vec<char> = pat.chars().collect();
            assert_eq!(glob_match(&chars, text), *want, "pattern {pat} vs {text}");
        }
    }

    #[test]
    fn parse_skips_blank_and_comment_lines() {
        let r = rules(&["", "  # nothing", "/", "keep-me.md"]);
        let texts = r.pattern_texts();
        assert!(texts.contains(&"keep-me.md".to_owned()));
        assert!(
            !texts
                .iter()
                .any(|t| t.starts_with('#') || t.is_empty() || t == "/")
        );
    }

    #[test]
    fn scan_hashes_files_and_skips_ignored_ones() {
        let vault = TempVault::new("scan");
        vault.write("a.md", b"hello\n");
        vault.write("dir/b.txt", b"world");
        vault.write("dir/skip.tmp", b"tmp");
        vault.write(".trash/gone.md", b"gone");
        vault.write(".jiyue/sync/state.json", b"{}");
        let snap = scan_local(vault.path(), &IgnoreRules::default());
        let keys: Vec<&String> = snap.files.keys().collect();
        assert_eq!(keys, vec!["a.md", "dir/b.txt"]);
        assert_eq!(snap.files["a.md"], hash_bytes(b"hello\n"));
        assert_eq!(snap.entries["dir/b.txt"].size, 5);
        assert_eq!(snap.entries["dir/b.txt"].hash, hash_bytes(b"world"));
        assert!(snap.unreadable.is_empty());
    }

    #[test]
    fn scan_cache_is_reused_only_when_safe() {
        let vault = TempVault::new("cache");
        vault.write("old.md", b"aaaa");
        vault.write("fresh.md", b"bbbb");
        let old_time = SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(vault.path().join("old.md"))
            .expect("open")
            .set_modified(old_time)
            .expect("set mtime");
        let mut cache = BTreeMap::new();
        let first = scan_local_cached(vault.path(), &IgnoreRules::default(), &mut cache);
        assert_eq!(cache.len(), 2);

        // Poison the cache for both files. The old file (size and mtime unchanged, outside the
        // racy window) must use the poisoned hash; the fresh one must be re-hashed.
        for entry in cache.values_mut() {
            entry.hash = "poisoned".into();
        }
        let second = scan_local_cached(vault.path(), &IgnoreRules::default(), &mut cache);
        assert_eq!(second.files["old.md"], "poisoned");
        assert_eq!(second.files["fresh.md"], first.files["fresh.md"]);
        assert_ne!(second.files["fresh.md"], "poisoned");
    }

    #[test]
    fn scan_cache_drops_entries_for_deleted_files() {
        let vault = TempVault::new("cache-drop");
        vault.write("a.md", b"a");
        vault.write("b.md", b"b");
        let mut cache = BTreeMap::new();
        scan_local_cached(vault.path(), &IgnoreRules::default(), &mut cache);
        fs::remove_file(vault.path().join("b.md")).expect("remove");
        let snap = scan_local_cached(vault.path(), &IgnoreRules::default(), &mut cache);
        assert_eq!(snap.files.len(), 1);
        assert_eq!(cache.keys().collect::<Vec<_>>(), vec!["a.md"]);
    }

    #[test]
    fn scan_of_missing_root_reports_root_unreadable() {
        let vault = TempVault::new("missing");
        let missing = vault.path().join("nope");
        let snap = scan_local(&missing, &IgnoreRules::default());
        assert!(snap.files.is_empty());
        assert!(snap.is_unreadable("anything.md"));
    }

    #[test]
    fn is_unreadable_matches_files_and_folder_prefixes() {
        let mut snap = LocalSnapshot::default();
        snap.unreadable.insert("locked.md".into());
        snap.unreadable.insert("broken/".into());
        assert!(snap.is_unreadable("locked.md"));
        assert!(snap.is_unreadable("broken/x.md"));
        assert!(!snap.is_unreadable("broken.md"));
        assert!(!snap.is_unreadable("ok.md"));
    }

    #[test]
    fn state_round_trips_through_disk() {
        let vault = TempVault::new("persist");
        let mut state = SyncState::default();
        state.set_base(
            "a.md",
            BaseEntry {
                hash: hash_bytes(b"x"),
                etag: Some("e1".into()),
                version_id: Some("v1".into()),
                size: 1,
            },
        );
        state.last_synced_ms = Some(42);
        save_state(vault.path(), &mut state).expect("save");
        assert_eq!(state.version, STATE_VERSION);
        let loaded = load_state(vault.path()).expect("load");
        assert_eq!(loaded, state);
        assert_eq!(
            loaded.base_entry("a.md").expect("entry").etag.as_deref(),
            Some("e1")
        );
        assert!(
            !sync_dir(vault.path())
                .join("state.json.jiyue-sync.tmp")
                .exists()
        );
    }

    #[test]
    fn missing_state_loads_as_empty() {
        let vault = TempVault::new("empty-state");
        let state = load_state(vault.path()).expect("load");
        assert_eq!(state.version, STATE_VERSION);
        assert!(state.entries.is_empty() && state.base.is_empty());
    }

    #[test]
    fn corrupt_or_newer_state_is_an_error_not_a_reset() {
        let vault = TempVault::new("corrupt");
        vault.write(".jiyue/sync/state.json", b"{ not json");
        assert!(load_state(vault.path()).is_err());
        vault.write(
            ".jiyue/sync/state.json",
            format!("{{\"version\": {}}}", STATE_VERSION + 1).as_bytes(),
        );
        assert!(load_state(vault.path()).is_err());
    }

    #[test]
    fn legacy_hash_only_base_is_still_readable() {
        let mut state = SyncState::default();
        state.base.insert("old.md".into(), "h".into());
        let entry = state.base_entry("old.md").expect("legacy entry");
        assert_eq!(entry.hash, "h");
        assert_eq!(entry.etag, None);
    }

    #[test]
    fn base_cache_round_trip_and_prune() {
        let vault = TempVault::new("base-cache");
        let data = b"merge base text\n";
        let hash = hash_bytes(data);
        write_base_cache(vault.path(), &hash, data).expect("write");
        assert_eq!(
            read_base_cache(vault.path(), &hash).as_deref(),
            Some(&data[..])
        );
        assert!(read_base_cache(vault.path(), "../../etc").is_none());

        // Corrupted content no longer matches its name and is not returned.
        fs::write(base_cache_dir(vault.path()).join(&hash), b"tampered").expect("tamper");
        assert!(read_base_cache(vault.path(), &hash).is_none());

        let other = hash_bytes(b"other");
        write_base_cache(vault.path(), &other, b"other").expect("write other");
        let mut keep = BTreeSet::new();
        keep.insert(other.clone());
        prune_base_cache(vault.path(), &keep).expect("prune");
        assert!(base_cache_dir(vault.path()).join(&other).exists());
        assert!(!base_cache_dir(vault.path()).join(&hash).exists());
    }

    #[test]
    fn base_cache_rules() {
        assert!(wants_base_cache("notes/a.md", 100));
        assert!(!wants_base_cache("photo.png", 100));
        assert!(!wants_base_cache("big.md", BASE_CACHE_MAX_BYTES + 1));
        assert!(wants_base_cache("exact.md", BASE_CACHE_MAX_BYTES));
    }

    #[test]
    fn device_id_is_created_once_and_reused() {
        let vault = TempVault::new("device");
        let first = load_or_create_device_id(vault.path()).expect("create");
        assert!(is_valid_device_id(&first));
        assert_eq!(first.len(), 12);
        let second = load_or_create_device_id(vault.path()).expect("reuse");
        assert_eq!(first, second);
    }

    #[test]
    fn device_id_file_with_garbage_is_replaced() {
        let vault = TempVault::new("device-bad");
        vault.write(".jiyue/sync/device_id", b"../evil id");
        let id = load_or_create_device_id(vault.path()).expect("replace");
        assert!(is_valid_device_id(&id));
    }

    #[test]
    fn conflict_copy_names_keep_folder_and_extension() {
        let ts = "20261104-1530";
        assert_eq!(
            conflict_copy_name("note.md", "PC", ts),
            "note (冲突副本 PC 20261104-1530).md"
        );
        assert_eq!(
            conflict_copy_name("daily/2026/11/04.md", "PC", ts),
            "daily/2026/11/04 (冲突副本 PC 20261104-1530).md"
        );
        assert_eq!(
            conflict_copy_name("img/photo.final.PNG", "phone", ts),
            "img/photo.final (冲突副本 phone 20261104-1530).PNG"
        );
        assert_eq!(
            conflict_copy_name("README", "PC", ts),
            "README (冲突副本 PC 20261104-1530)"
        );
        assert_eq!(
            conflict_copy_name(".gitignore", "PC", ts),
            ".gitignore (冲突副本 PC 20261104-1530)"
        );
    }

    #[test]
    fn conflict_copy_numbering_and_device_sanitizing() {
        assert_eq!(
            conflict_copy_name_n("a.md", "PC", "20261104-1530", 2),
            "a (冲突副本 PC 20261104-1530 2).md"
        );
        assert_eq!(
            conflict_copy_name("a.md", "we/ird:name", "t"),
            "a (冲突副本 we-ird-name t).md"
        );
        assert_eq!(
            conflict_copy_name("a.md", "  ", "t"),
            "a (冲突副本 device t).md"
        );
    }

    #[test]
    fn timestamps_are_utc_and_compact() {
        assert_eq!(format_timestamp(0), "19700101-0000");
        assert_eq!(format_timestamp(1_793_806_200_000), "20261104-1530");
        assert_eq!(
            format_timestamp_precise(1_793_806_200_000 + 47_042),
            "20261104-153047-042"
        );
        assert_eq!(format_timestamp(-1), "19691231-2359");
    }

    #[test]
    fn text_detection_by_extension() {
        for p in [
            "a.md",
            "dir/b.TXT",
            "c.json",
            "d.yaml",
            "e.rs",
            "f.csv",
            "x/y.ts",
        ] {
            assert!(is_text_path(p), "{p} should be text");
        }
        for p in [
            "pic.png",
            "doc.pdf",
            "archive.zip",
            "noext",
            "trailing.",
            ".hidden",
        ] {
            assert!(!is_text_path(p), "{p} should be binary");
        }
    }

    #[test]
    fn safe_relative_paths() {
        assert!(is_safe_rel_path("a/b.md"));
        assert!(is_safe_rel_path("a.md"));
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            "C:/x",
            "a\\b",
            "a/./b",
            "x\u{0}y",
        ] {
            assert!(!is_safe_rel_path(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn atomic_write_replaces_content_without_leftovers() {
        let vault = TempVault::new("atomic");
        let path = vault.path().join("deep/f.md");
        write_atomic(&path, b"one").expect("first");
        write_atomic(&path, b"two").expect("second");
        assert_eq!(fs::read(&path).expect("read"), b"two");
        let leftovers: Vec<_> = fs::read_dir(path.parent().expect("parent"))
            .expect("list")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn hash_is_stable_blake3_hex() {
        let h = hash_bytes(b"");
        assert_eq!(h.len(), 64);
        assert_eq!(h, blake3::hash(b"").to_hex().as_str());
        assert!(is_valid_hash(&h));
        assert!(!is_valid_hash("ABC"));
    }
}
