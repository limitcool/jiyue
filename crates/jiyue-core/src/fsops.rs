//! File operations on a vault: create, rename, move, delete (to `.trash`). Owned by W1-A.
//!
//! All functions take the vault `root` and vault-relative paths with forward slashes. Paths are
//! validated by [`safe_rel_path`], and every existing component on the way must be a real file or
//! directory: symbolic links and junctions are refused, so nothing outside `root` is ever touched.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::watch;

/// Folder under the vault root that holds deleted items.
pub const TRASH_DIR: &str = ".trash";
/// Longest accepted path component, in UTF-8 bytes.
pub const MAX_NAME_BYTES: usize = 200;

const SIDECAR_SUFFIX: &str = ".json";
const MAX_UNIQUE_ATTEMPTS: u32 = 10_000;
const IGNORED_NAMES: [&str; 4] = [".trash", ".jiyue", ".obsidian", ".git"];
const RESERVED_STEMS: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Error from a vault file operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    /// The source does not exist (or a trash entry is missing).
    NotFound,
    /// The destination already exists.
    Exists,
    /// The path or name is not allowed; the text says why.
    Invalid(String),
    /// Any other I/O failure.
    Io(String),
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::NotFound => write!(f, "not found"),
            FsError::Exists => write!(f, "already exists"),
            FsError::Invalid(reason) => write!(f, "invalid path: {reason}"),
            FsError::Io(message) => write!(f, "I/O error: {message}"),
        }
    }
}

impl std::error::Error for FsError {}

impl From<std::io::Error> for FsError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => FsError::NotFound,
            std::io::ErrorKind::AlreadyExists => FsError::Exists,
            _ => FsError::Io(err.to_string()),
        }
    }
}

fn invalid(reason: &str) -> FsError {
    FsError::Invalid(reason.to_string())
}

/// One item in the trash. `name` is the entry inside `.trash`, `original` the vault-relative path
/// it was deleted from. Stored as a `<name>.json` sidecar next to the item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashEntry {
    pub name: String,
    pub original: String,
    pub deleted_ms: i64,
    pub is_dir: bool,
}

/// Validates a vault-relative path and returns it as a relative `PathBuf`.
///
/// Rejects: empty paths, backslashes, absolute paths and drive letters, empty segments (`a//b`,
/// trailing `/`), `.` and `..`, control characters, `<>:"/\|?*`, names ending in `.` or a space,
/// reserved Windows device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9, also with an extension),
/// and components longer than [`MAX_NAME_BYTES`] bytes. Non-ASCII names are accepted.
pub fn safe_rel_path(rel: &str) -> Result<PathBuf, FsError> {
    if rel.is_empty() {
        return Err(invalid("empty path"));
    }
    if rel.contains('\\') {
        return Err(invalid("backslash separator"));
    }
    if rel.starts_with('/') {
        return Err(invalid("absolute path"));
    }
    let mut out = PathBuf::new();
    for part in rel.split('/') {
        validate_component(part)?;
        out.push(part);
    }
    Ok(out)
}

fn validate_component(part: &str) -> Result<(), FsError> {
    if part.is_empty() {
        return Err(invalid("empty path segment"));
    }
    if part == "." || part == ".." {
        return Err(invalid("relative segment"));
    }
    if part.len() > MAX_NAME_BYTES {
        return Err(invalid("name longer than 200 bytes"));
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return Err(invalid("name ends with a dot or a space"));
    }
    if part.chars().any(|c| c.is_control()) {
        return Err(invalid("control character in name"));
    }
    if part
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        return Err(invalid("illegal character in name"));
    }
    let stem = part.split('.').next().unwrap_or(part);
    if is_reserved_stem(stem) {
        return Err(invalid("reserved Windows device name"));
    }
    Ok(())
}

fn is_reserved_stem(stem: &str) -> bool {
    let upper = stem.trim_end_matches(' ').to_ascii_uppercase();
    if RESERVED_STEMS.contains(&upper.as_str()) {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && (b'1'..=b'9').contains(&bytes[3])
}

/// True when a vault-relative path must never be indexed, watched or reported: the internal
/// folders (`.trash`, `.jiyue`, `.obsidian`, `.git`), any hidden dot-file or dot-folder, and
/// `*.tmp` files. Backslashes are accepted as separators.
pub fn is_ignored(rel: &str) -> bool {
    rel.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .any(|part| {
            part.starts_with('.')
                || IGNORED_NAMES
                    .iter()
                    .any(|name| part.eq_ignore_ascii_case(name))
                || ends_with_ignore_case(part, ".tmp")
        })
}

pub(crate) fn ends_with_ignore_case(s: &str, suffix: &str) -> bool {
    s.len()
        .checked_sub(suffix.len())
        .and_then(|i| s.get(i..))
        .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
}

/// Writes `bytes` to `path` atomically: a temp file in the same folder is written and fsynced,
/// then renamed over `path`. The write is registered with the watch module so that the resulting
/// file events are not reported back as external changes. Parent folders are created.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FsError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("path has no parent folder"))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("file name is not UTF-8"))?;
    // A read-only file is refused before anything is written, so it fails fast and clearly.
    if fs::metadata(path).is_ok_and(|meta| meta.permissions().readonly()) {
        return Err(FsError::Io(format!("文件是只读的，未能保存：{name}")));
    }
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = write_then_rename(&tmp, path, bytes);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Number of rename attempts when another program holds the target (see `rename_with_retry`).
const RENAME_ATTEMPTS: u32 = 5;
/// Wait before the second rename attempt; doubles after every failed attempt.
const RENAME_FIRST_WAIT: Duration = Duration::from_millis(25);

fn write_then_rename(tmp: &Path, dest: &Path, bytes: &[u8]) -> Result<(), FsError> {
    {
        let mut file = OpenOptions::new().write(true).create_new(true).open(tmp)?;
        injected_failure("tmp write")?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    injected_failure("rename")?;
    watch::expect_write(dest);
    rename_with_retry(tmp, dest).map_err(|err| {
        let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_busy(&err) {
            FsError::Io(format!("文件正被其他程序占用，未能保存：{name}"))
        } else {
            FsError::Io(format!("未能保存 {name}：{err}"))
        }
    })
}

/// Renames `from` over `to`. A file held by another program (sharing or lock violation, access
/// denied) is retried a few times with a short wait; every other error is returned at once.
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut wait = RENAME_FIRST_WAIT;
    let mut attempt = 1;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err) if is_busy(&err) && attempt < RENAME_ATTEMPTS => {
                std::thread::sleep(wait);
                wait *= 2;
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// Windows error codes for a file another program holds: 5 access denied, 32 sharing violation,
/// 33 lock violation.
fn is_busy(err: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        matches!(err.raw_os_error(), Some(5 | 32 | 33))
    }
    #[cfg(not(windows))]
    {
        let _ = err;
        false
    }
}

#[cfg(test)]
thread_local! {
    /// Test hook: the next `write_atomic` on this thread fails at the named stage.
    static FAIL_AT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// Returns an error when a test asked for a failure at `stage`. Nothing happens in normal builds.
#[cfg(test)]
fn injected_failure(stage: &'static str) -> Result<(), FsError> {
    if FAIL_AT.with(|c| c.get()) == Some(stage) {
        FAIL_AT.with(|c| c.set(None));
        return Err(FsError::Io(format!("injected failure at {stage}")));
    }
    Ok(())
}

#[cfg(not(test))]
fn injected_failure(_stage: &'static str) -> Result<(), FsError> {
    Ok(())
}

/// Creates an empty note `<dir_rel>/<title>.md`, or `<title> 1.md`, `<title> 2.md`, ... when the
/// name is taken. `dir_rel` is `""` for the vault root and must otherwise be an existing folder.
/// Returns the vault-relative path of the new note.
pub fn create_note(root: &Path, dir_rel: &str, title: &str) -> Result<String, FsError> {
    ensure_root(root)?;
    let title = title.trim();
    if title.is_empty() {
        return Err(invalid("empty title"));
    }
    if title.starts_with('.') {
        return Err(invalid("title starts with a dot"));
    }
    validate_component(title)?;
    let dir_abs = if dir_rel.is_empty() {
        root.to_path_buf()
    } else {
        resolve_in_root(root, dir_rel)?
    };
    if !fs::metadata(&dir_abs)?.is_dir() {
        return Err(invalid("parent is not a folder"));
    }
    let prefix = if dir_rel.is_empty() {
        String::new()
    } else {
        format!("{dir_rel}/")
    };
    for n in 0..MAX_UNIQUE_ATTEMPTS {
        let file = if n == 0 {
            format!("{title}.md")
        } else {
            format!("{title} {n}.md")
        };
        validate_component(&file)?;
        let abs = dir_abs.join(&file);
        watch::expect_write(&abs);
        match OpenOptions::new().write(true).create_new(true).open(&abs) {
            Ok(_) => return Ok(format!("{prefix}{file}")),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Err(FsError::Io("no free file name".to_string()))
}

/// Creates a folder (and missing parents). Fails with `Exists` if anything is already there.
pub fn create_folder(root: &Path, rel: &str) -> Result<(), FsError> {
    ensure_root(root)?;
    let abs = resolve_in_root(root, rel)?;
    if exists_no_follow(&abs)? {
        return Err(FsError::Exists);
    }
    fs::create_dir_all(&abs)?;
    Ok(())
}

/// Renames or moves a file or folder. Fails with `Exists` when `to` is taken, except for a
/// case-only rename of the same item (`a.md` to `A.md`). Parent folders of `to` are created.
/// Renaming an item to its own path is a no-op. A folder cannot move into itself.
pub fn rename(root: &Path, from: &str, to: &str) -> Result<(), FsError> {
    ensure_root(root)?;
    let from_abs = existing_in_root(root, from)?;
    let to_abs = resolve_in_root(root, to)?;
    if from == to {
        return Ok(());
    }
    if is_inside(from, to) {
        return Err(invalid("cannot move a folder into itself"));
    }
    let case_only = from.to_lowercase() == to.to_lowercase();
    if exists_no_follow(&to_abs)? {
        let same_item = case_only && fs::canonicalize(&from_abs)? == fs::canonicalize(&to_abs)?;
        if !same_item {
            return Err(FsError::Exists);
        }
    }
    if let Some(parent) = to_abs.parent() {
        fs::create_dir_all(parent)?;
    }
    if case_only {
        // Two steps: some file systems treat a direct case-only rename as a no-op.
        let stage = stage_path(&to_abs);
        fs::rename(&from_abs, &stage)?;
        if let Err(err) = fs::rename(&stage, &to_abs) {
            let _ = fs::rename(&stage, &from_abs);
            return Err(err.into());
        }
        return Ok(());
    }
    fs::rename(&from_abs, &to_abs)?;
    Ok(())
}

/// Moves `from` into the folder `to_dir` (`""` for the root), keeping its name. Returns the new
/// vault-relative path. Fails like [`rename`].
pub fn move_to(root: &Path, from: &str, to_dir: &str) -> Result<String, FsError> {
    ensure_root(root)?;
    existing_in_root(root, from)?;
    if !to_dir.is_empty() {
        let dir = existing_in_root(root, to_dir)?;
        if !fs::metadata(&dir)?.is_dir() {
            return Err(invalid("destination is not a folder"));
        }
    }
    let name = from.rsplit('/').next().unwrap_or(from);
    let to = if to_dir.is_empty() {
        name.to_string()
    } else {
        format!("{to_dir}/{name}")
    };
    rename(root, from, &to)?;
    Ok(to)
}

/// Moves a file or folder into `<root>/.trash/<unix-ms>-<name>` and writes a `.json` sidecar that
/// records the original path. If the sidecar cannot be written the move is undone.
pub fn delete_to_trash(root: &Path, rel: &str) -> Result<TrashEntry, FsError> {
    ensure_root(root)?;
    if is_trash_rel(rel) {
        return Err(invalid("item is already in the trash"));
    }
    let src = existing_in_root(root, rel)?;
    let is_dir = fs::symlink_metadata(&src)?.is_dir();
    let name_src = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("file name is not UTF-8"))?;
    let trash = root.join(TRASH_DIR);
    fs::create_dir_all(&trash)?;
    let mut stamp = now_ms();
    for _ in 0..MAX_UNIQUE_ATTEMPTS {
        let name = format!("{stamp}-{name_src}");
        let dest = trash.join(&name);
        let sidecar = sidecar_path(&trash, &name);
        if exists_no_follow(&dest)? || exists_no_follow(&sidecar)? {
            stamp += 1;
            continue;
        }
        fs::rename(&src, &dest)?;
        let entry = TrashEntry {
            name,
            original: rel.to_string(),
            deleted_ms: stamp,
            is_dir,
        };
        if let Err(err) = write_sidecar(&sidecar, &entry) {
            let _ = fs::rename(&dest, &src);
            return Err(err);
        }
        return Ok(entry);
    }
    Err(FsError::Io("no free trash name".to_string()))
}

/// Lists restorable trash entries, newest first. Items without a readable sidecar are skipped.
pub fn list_trash(root: &Path) -> Result<Vec<TrashEntry>, FsError> {
    ensure_root(root)?;
    let trash = root.join(TRASH_DIR);
    let read = match fs::read_dir(&trash) {
        Ok(read) => read,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut out = Vec::new();
    for dir_entry in read {
        let dir_entry = dir_entry?;
        let file_name = dir_entry.file_name();
        let Some(sidecar_name) = file_name.to_str() else {
            continue;
        };
        let Some(item) = sidecar_name.strip_suffix(SIDECAR_SUFFIX) else {
            continue;
        };
        let Ok(bytes) = fs::read(dir_entry.path()) else {
            continue;
        };
        let Ok(mut entry) = serde_json::from_slice::<TrashEntry>(&bytes) else {
            continue;
        };
        if !exists_no_follow(&trash.join(item))? {
            continue;
        }
        entry.name = item.to_string();
        out.push(entry);
    }
    out.sort_by(|a, b| {
        b.deleted_ms
            .cmp(&a.deleted_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Moves a trash entry back to its original path (read from the sidecar). If that path is taken
/// the item gets a unique name (`name 1.md`). Returns the new vault-relative path.
pub fn restore_from_trash(root: &Path, entry: &TrashEntry) -> Result<String, FsError> {
    ensure_root(root)?;
    validate_component(&entry.name)?;
    let trash = root.join(TRASH_DIR);
    let item = trash.join(&entry.name);
    let sidecar = sidecar_path(&trash, &entry.name);
    let bytes = fs::read(&sidecar)?;
    let stored: TrashEntry =
        serde_json::from_slice(&bytes).map_err(|err| FsError::Io(err.to_string()))?;
    if !exists_no_follow(&item)? {
        return Err(FsError::NotFound);
    }
    let target = unique_rel(root, &stored.original)?;
    let target_abs = resolve_in_root(root, &target)?;
    if let Some(parent) = target_abs.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&item, &target_abs)?;
    let _ = fs::remove_file(&sidecar);
    Ok(target)
}

/// Permanently removes everything in `.trash`. Returns how many restorable entries were removed.
pub fn empty_trash(root: &Path) -> Result<usize, FsError> {
    let count = list_trash(root)?.len();
    let trash = root.join(TRASH_DIR);
    let read = match fs::read_dir(&trash) {
        Ok(read) => read,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.into()),
    };
    for dir_entry in read {
        let path = dir_entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            fs::remove_file(&path).or_else(|_| fs::remove_dir(&path))?;
        } else if meta.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(count)
}

fn write_sidecar(path: &Path, entry: &TrashEntry) -> Result<(), FsError> {
    let bytes = serde_json::to_vec_pretty(entry).map_err(|err| FsError::Io(err.to_string()))?;
    write_atomic(path, &bytes)
}

fn sidecar_path(trash: &Path, name: &str) -> PathBuf {
    trash.join(format!("{name}{SIDECAR_SUFFIX}"))
}

fn is_trash_rel(rel: &str) -> bool {
    rel.split('/')
        .next()
        .is_some_and(|first| first.eq_ignore_ascii_case(TRASH_DIR))
}

fn is_inside(from: &str, to: &str) -> bool {
    let prefix = format!("{}/", from.to_lowercase());
    to.to_lowercase().starts_with(&prefix)
}

fn stage_path(to_abs: &Path) -> PathBuf {
    let name = to_abs
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("item");
    to_abs.with_file_name(format!(
        ".{name}.{}.{}.stage",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Returns a restore target: `rel` itself, or `stem N.ext` for the first free N.
fn unique_rel(root: &Path, rel: &str) -> Result<String, FsError> {
    if !exists_no_follow(&resolve_in_root(root, rel)?)? {
        return Ok(rel.to_string());
    }
    let (dir, file) = match rel.rfind('/') {
        Some(i) => (&rel[..i], &rel[i + 1..]),
        None => ("", rel),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    for n in 1..MAX_UNIQUE_ATTEMPTS {
        let candidate = if dir.is_empty() {
            format!("{stem} {n}{ext}")
        } else {
            format!("{dir}/{stem} {n}{ext}")
        };
        if !exists_no_follow(&resolve_in_root(root, &candidate)?)? {
            return Ok(candidate);
        }
    }
    Err(FsError::Io("no free name for restore".to_string()))
}

fn ensure_root(root: &Path) -> Result<(), FsError> {
    if fs::metadata(root)?.is_dir() {
        Ok(())
    } else {
        Err(invalid("vault root is not a folder"))
    }
}

fn exists_no_follow(path: &Path) -> Result<bool, FsError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

/// Joins a validated vault-relative path onto `root`. Refuses any existing component that is a
/// symbolic link or junction, so the result always stays inside `root`.
fn resolve_in_root(root: &Path, rel: &str) -> Result<PathBuf, FsError> {
    let rel_path = safe_rel_path(rel)?;
    let mut current = root.to_path_buf();
    for component in rel_path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || fs::read_link(&current).is_ok() {
                    return Err(invalid("path passes through a symbolic link"));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(current)
}

fn existing_in_root(root: &Path, rel: &str) -> Result<PathBuf, FsError> {
    let path = resolve_in_root(root, rel)?;
    if exists_no_follow(&path)? {
        Ok(path)
    } else {
        Err(FsError::NotFound)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch vault under the system temp dir, removed on drop.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("jiyue-test-{}-fsops-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create sandbox");
            Sandbox(dir)
        }

        fn root(&self) -> &Path {
            &self.0
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create fixture folder");
            }
            fs::write(path, text).expect("write fixture");
        }

        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.0.join(rel)).unwrap_or_default()
        }

        fn exists(&self, rel: &str) -> bool {
            self.0.join(rel).exists()
        }

        fn names_in(&self, rel: &str) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.0.join(rel))
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn is_invalid(result: &Result<impl std::fmt::Debug, FsError>) -> bool {
        matches!(result, Err(FsError::Invalid(_)))
    }

    #[test]
    fn safe_rel_path_accepts_normal_paths() {
        for ok in [
            "a.md",
            "folder/note.md",
            "a/b/c/d.md",
            "notes/2026-11-04 log.md",
        ] {
            assert_eq!(
                safe_rel_path(ok).expect(ok),
                PathBuf::from(ok.replace('/', std::path::MAIN_SEPARATOR_STR)),
                "{ok}"
            );
        }
    }

    #[test]
    fn safe_rel_path_accepts_cjk_and_emoji() {
        for ok in [
            "想法/沉积的想法.md",
            "日记 📝/今天.md",
            "中文 文件.md",
            "é/ünï.md",
        ] {
            assert!(safe_rel_path(ok).is_ok(), "{ok} should be accepted");
        }
    }

    #[test]
    fn safe_rel_path_accepts_dot_folders_for_internal_use() {
        assert!(safe_rel_path(".obsidian/app.json").is_ok());
        assert!(safe_rel_path(".trash/1-a.md").is_ok());
    }

    #[test]
    fn safe_rel_path_rejects_empty() {
        assert!(is_invalid(&safe_rel_path("")));
    }

    #[test]
    fn safe_rel_path_rejects_absolute_and_drive_paths() {
        for bad in [
            "/etc/passwd",
            "C:/notes/a.md",
            "C:a.md",
            "D:\\x.md",
            "\\\\server\\share",
        ] {
            assert!(is_invalid(&safe_rel_path(bad)), "{bad} should be rejected");
        }
    }

    #[test]
    fn safe_rel_path_rejects_backslashes() {
        for bad in ["a\\b.md", "folder\\note.md"] {
            assert!(is_invalid(&safe_rel_path(bad)), "{bad} should be rejected");
        }
    }

    #[test]
    fn safe_rel_path_rejects_traversal() {
        for bad in [
            "..",
            "../x.md",
            "a/../b.md",
            "a/..",
            "./a.md",
            "a/./b.md",
            ".",
        ] {
            assert!(is_invalid(&safe_rel_path(bad)), "{bad} should be rejected");
        }
    }

    #[test]
    fn safe_rel_path_rejects_empty_segments() {
        for bad in ["a//b.md", "a/", "a/b/", "//x"] {
            assert!(is_invalid(&safe_rel_path(bad)), "{bad} should be rejected");
        }
    }

    #[test]
    fn safe_rel_path_rejects_trailing_dot_and_space() {
        for bad in ["trailing.", "dir./x.md", "name ", "dir /x.md", "a/b. "] {
            assert!(
                is_invalid(&safe_rel_path(bad)),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn safe_rel_path_rejects_illegal_characters() {
        for bad in ["a<b", "a>b", "a:b", "a\"b", "a|b", "a?b", "a*b", "x/y*z.md"] {
            assert!(is_invalid(&safe_rel_path(bad)), "{bad} should be rejected");
        }
    }

    #[test]
    fn safe_rel_path_rejects_control_characters() {
        for bad in ["tab\tname", "new\nline.md", "bell\u{7}.md", "del\u{7f}.md"] {
            assert!(
                is_invalid(&safe_rel_path(bad)),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn safe_rel_path_rejects_reserved_device_names() {
        for bad in [
            "CON", "con", "PRN.md", "aux/x.md", "NUL.txt", "nul", "COM1", "com9.md", "LPT1",
            "lpt5/x", "Com3.txt",
        ] {
            assert!(is_invalid(&safe_rel_path(bad)), "{bad} should be rejected");
        }
    }

    #[test]
    fn safe_rel_path_accepts_names_that_only_look_reserved() {
        for ok in [
            "com10.md",
            "LPT0.md",
            "console.md",
            "nullable.md",
            "COM.md",
            "auxiliary.md",
        ] {
            assert!(safe_rel_path(ok).is_ok(), "{ok} should be accepted");
        }
    }

    #[test]
    fn safe_rel_path_enforces_200_byte_limit() {
        let ok = "a".repeat(200);
        let too_long = "a".repeat(201);
        assert!(safe_rel_path(&ok).is_ok());
        assert!(is_invalid(&safe_rel_path(&too_long)));
    }

    #[test]
    fn safe_rel_path_counts_bytes_not_chars() {
        // Each CJK char is 3 bytes: 66 chars = 198 bytes (ok), 67 chars = 201 bytes (rejected).
        let ok = "中".repeat(66);
        let too_long = "中".repeat(67);
        assert!(safe_rel_path(&ok).is_ok());
        assert!(is_invalid(&safe_rel_path(&too_long)));
    }

    #[test]
    fn safe_rel_path_limit_applies_per_component() {
        let long_dir = format!("{}/x.md", "d".repeat(150));
        assert!(safe_rel_path(&long_dir).is_ok());
    }

    #[test]
    fn is_ignored_table() {
        for ignored in [
            ".trash/1-a.md",
            ".jiyue/index",
            ".obsidian/app.json",
            ".git/HEAD",
            ".TRASH/x.md",
            "a/.hidden/x.md",
            ".DS_Store",
            "a/x.tmp",
            "a/X.TMP",
            "a\\.git\\config",
        ] {
            assert!(is_ignored(ignored), "{ignored} should be ignored");
        }
    }

    #[test]
    fn is_ignored_keeps_normal_notes() {
        for kept in [
            "a.md",
            "想法/沉积的想法.md",
            "folder/tmpnote.md",
            "a/b.tmpl",
            "x.md.bak",
        ] {
            assert!(!is_ignored(kept), "{kept} should not be ignored");
        }
    }

    #[test]
    fn write_atomic_creates_parents_and_overwrites() {
        let sb = Sandbox::new("write-atomic");
        let path = sb.root().join("deep/er/note.md");
        write_atomic(&path, "first".as_bytes()).expect("first write");
        assert_eq!(sb.read("deep/er/note.md"), "first");
        write_atomic(&path, "second".as_bytes()).expect("second write");
        assert_eq!(sb.read("deep/er/note.md"), "second");
    }

    #[test]
    fn write_atomic_leaves_no_temp_files() {
        let sb = Sandbox::new("write-atomic-tmp");
        write_atomic(&sb.root().join("n.md"), b"body").expect("write");
        assert_eq!(sb.names_in(""), vec!["n.md".to_string()]);
    }

    #[test]
    fn write_atomic_writes_cjk_and_empty_content() {
        let sb = Sandbox::new("write-atomic-cjk");
        write_atomic(&sb.root().join("笔记.md"), "内容 📝".as_bytes()).expect("write");
        assert_eq!(sb.read("笔记.md"), "内容 📝");
        write_atomic(&sb.root().join("笔记.md"), b"").expect("truncate");
        assert_eq!(sb.read("笔记.md"), "");
    }

    #[test]
    fn write_atomic_keeps_original_when_temp_write_fails() {
        let sb = Sandbox::new("write-atomic-fail-tmp");
        write_atomic(&sb.root().join("n.md"), b"original").expect("seed");
        fail_at("tmp write");
        let err = write_atomic(&sb.root().join("n.md"), b"replacement");
        assert!(err.is_err());
        assert_eq!(sb.read("n.md"), "original");
        assert_eq!(sb.names_in(""), vec!["n.md".to_string()]);
    }

    #[test]
    fn write_atomic_keeps_original_when_rename_fails() {
        let sb = Sandbox::new("write-atomic-fail-rename");
        write_atomic(&sb.root().join("n.md"), b"original").expect("seed");
        fail_at("rename");
        assert!(write_atomic(&sb.root().join("n.md"), b"replacement").is_err());
        assert_eq!(sb.read("n.md"), "original");
        assert_eq!(sb.names_in(""), vec!["n.md".to_string()]);
    }

    #[test]
    // Clearing the read-only flag is the point of the test: the file is restored, not left open.
    #[allow(clippy::permissions_set_readonly_false)]
    fn write_atomic_refuses_read_only_target_and_keeps_it() {
        let sb = Sandbox::new("write-atomic-readonly");
        let path = sb.root().join("n.md");
        write_atomic(&path, b"original").expect("seed");
        let mut perms = fs::metadata(&path).expect("meta").permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms.clone()).expect("readonly");
        let err = write_atomic(&path, b"replacement").expect_err("read-only must fail");
        assert!(err.to_string().contains("只读"), "{err}");
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).expect("writable again");
        assert_eq!(sb.read("n.md"), "original");
        assert_eq!(sb.names_in(""), vec!["n.md".to_string()]);
    }

    #[test]
    fn write_atomic_fails_when_parent_is_a_file_and_keeps_others() {
        let sb = Sandbox::new("write-atomic-parent-file");
        write_atomic(&sb.root().join("blocker"), b"keep me").expect("seed");
        assert!(write_atomic(&sb.root().join("blocker/n.md"), b"x").is_err());
        assert_eq!(sb.read("blocker"), "keep me");
    }

    #[cfg(windows)]
    #[test]
    fn write_atomic_keeps_original_when_another_program_holds_it() {
        use std::os::windows::fs::OpenOptionsExt;
        let sb = Sandbox::new("write-atomic-locked");
        let path = sb.root().join("n.md");
        write_atomic(&path, b"original").expect("seed");
        // Open without FILE_SHARE_DELETE or FILE_SHARE_WRITE, like an editor holding the file.
        let _held = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .expect("hold");
        let err = write_atomic(&path, b"replacement").expect_err("held file must fail");
        assert!(err.to_string().contains("占用"), "{err}");
        assert_eq!(sb.read("n.md"), "original");
        assert_eq!(sb.names_in(""), vec!["n.md".to_string()]);
    }

    fn fail_at(stage: &'static str) {
        FAIL_AT.with(|c| c.set(Some(stage)));
    }

    #[test]
    fn create_note_creates_at_root_and_in_folder() {
        let sb = Sandbox::new("create-note");
        fs::create_dir_all(sb.root().join("想法")).expect("folder");
        assert_eq!(
            create_note(sb.root(), "", "Inbox").expect("root"),
            "Inbox.md"
        );
        assert_eq!(
            create_note(sb.root(), "想法", "沉积").expect("folder"),
            "想法/沉积.md"
        );
        assert!(sb.exists("Inbox.md"));
        assert_eq!(sb.read("想法/沉积.md"), "");
    }

    #[test]
    fn create_note_numbers_duplicates() {
        let sb = Sandbox::new("create-note-dup");
        assert_eq!(create_note(sb.root(), "", "x").expect("1"), "x.md");
        assert_eq!(create_note(sb.root(), "", "x").expect("2"), "x 1.md");
        assert_eq!(create_note(sb.root(), "", "x").expect("3"), "x 2.md");
        assert_eq!(create_note(sb.root(), "", "x 1").expect("4"), "x 1 1.md");
    }

    #[test]
    fn create_note_trims_title() {
        let sb = Sandbox::new("create-note-trim");
        assert_eq!(
            create_note(sb.root(), "", "  spaced  ").expect("ok"),
            "spaced.md"
        );
    }

    #[test]
    fn create_note_accepts_cjk_and_emoji_titles() {
        let sb = Sandbox::new("create-note-cjk");
        assert_eq!(
            create_note(sb.root(), "", "全新笔记").expect("cjk"),
            "全新笔记.md"
        );
        assert_eq!(
            create_note(sb.root(), "", "idea 💡").expect("emoji"),
            "idea 💡.md"
        );
    }

    #[test]
    fn create_note_rejects_bad_titles() {
        let sb = Sandbox::new("create-note-bad");
        for bad in [
            "",
            "   ",
            "a/b",
            "a\\b",
            "..",
            ".hidden",
            "trailing.",
            "a:b",
            "CON",
            "x|y",
        ] {
            assert!(
                is_invalid(&create_note(sb.root(), "", bad)),
                "title {bad:?} should be rejected"
            );
        }
        assert!(sb.names_in("").is_empty());
    }

    #[test]
    fn create_note_rejects_missing_or_file_parent() {
        let sb = Sandbox::new("create-note-parent");
        sb.write("file.md", "x");
        assert_eq!(
            create_note(sb.root(), "missing", "n"),
            Err(FsError::NotFound)
        );
        assert!(is_invalid(&create_note(sb.root(), "file.md", "n")));
    }

    #[test]
    fn create_note_refuses_paths_outside_root() {
        let sb = Sandbox::new("create-note-outside");
        assert!(is_invalid(&create_note(sb.root(), "../evil", "n")));
        assert!(is_invalid(&create_note(sb.root(), "/abs", "n")));
    }

    #[test]
    fn create_folder_creates_nested_and_reports_existing() {
        let sb = Sandbox::new("create-folder");
        create_folder(sb.root(), "a/b/c").expect("nested");
        assert!(sb.root().join("a/b/c").is_dir());
        assert_eq!(create_folder(sb.root(), "a/b"), Err(FsError::Exists));
        sb.write("file", "x");
        assert_eq!(create_folder(sb.root(), "file"), Err(FsError::Exists));
    }

    #[test]
    fn rename_file_moves_and_creates_parents() {
        let sb = Sandbox::new("rename-file");
        sb.write("old.md", "body");
        rename(sb.root(), "old.md", "new/deep/name.md").expect("rename");
        assert!(!sb.exists("old.md"));
        assert_eq!(sb.read("new/deep/name.md"), "body");
    }

    #[test]
    fn rename_refuses_existing_target() {
        let sb = Sandbox::new("rename-exists");
        sb.write("a.md", "a");
        sb.write("b.md", "b");
        assert_eq!(rename(sb.root(), "a.md", "b.md"), Err(FsError::Exists));
        assert_eq!(sb.read("a.md"), "a");
        assert_eq!(sb.read("b.md"), "b");
    }

    #[test]
    fn rename_case_only_file() {
        let sb = Sandbox::new("rename-case");
        sb.write("note.md", "keep me");
        rename(sb.root(), "note.md", "Note.md").expect("case rename");
        assert_eq!(sb.names_in(""), vec!["Note.md".to_string()]);
        assert_eq!(sb.read("Note.md"), "keep me");
    }

    #[test]
    fn rename_case_only_folder() {
        let sb = Sandbox::new("rename-case-folder");
        sb.write("ideas/one.md", "1");
        rename(sb.root(), "ideas", "Ideas").expect("case rename folder");
        assert_eq!(sb.names_in(""), vec!["Ideas".to_string()]);
        assert_eq!(sb.read("Ideas/one.md"), "1");
    }

    #[test]
    fn rename_folder_moves_children() {
        let sb = Sandbox::new("rename-folder");
        sb.write("old/a.md", "a");
        sb.write("old/sub/b.md", "b");
        rename(sb.root(), "old", "renamed/new").expect("folder rename");
        assert!(!sb.exists("old"));
        assert_eq!(sb.read("renamed/new/a.md"), "a");
        assert_eq!(sb.read("renamed/new/sub/b.md"), "b");
    }

    #[test]
    fn rename_folder_into_itself_is_invalid() {
        let sb = Sandbox::new("rename-self");
        sb.write("a/x.md", "x");
        assert!(is_invalid(&rename(sb.root(), "a", "a/inner")));
        assert!(sb.exists("a/x.md"));
    }

    #[test]
    fn rename_missing_source_is_not_found() {
        let sb = Sandbox::new("rename-missing");
        assert_eq!(rename(sb.root(), "nope.md", "x.md"), Err(FsError::NotFound));
    }

    #[test]
    fn rename_to_same_path_is_noop() {
        let sb = Sandbox::new("rename-same");
        sb.write("same.md", "s");
        rename(sb.root(), "same.md", "same.md").expect("noop");
        assert_eq!(sb.read("same.md"), "s");
    }

    #[test]
    fn rename_cjk_names() {
        let sb = Sandbox::new("rename-cjk");
        sb.write("想法/沉积的想法.md", "x");
        rename(sb.root(), "想法/沉积的想法.md", "想法/新的想法 📝.md").expect("rename");
        assert_eq!(sb.read("想法/新的想法 📝.md"), "x");
    }

    #[test]
    fn move_to_moves_into_folder_and_root() {
        let sb = Sandbox::new("move-to");
        sb.write("a.md", "a");
        sb.write("dest/keep.md", "k");
        let moved = move_to(sb.root(), "a.md", "dest").expect("into folder");
        assert_eq!(moved, "dest/a.md");
        assert_eq!(sb.read("dest/a.md"), "a");
        let back = move_to(sb.root(), "dest/a.md", "").expect("to root");
        assert_eq!(back, "a.md");
        assert_eq!(sb.read("a.md"), "a");
    }

    #[test]
    fn move_to_same_folder_is_noop() {
        let sb = Sandbox::new("move-same");
        sb.write("dir/x.md", "x");
        assert_eq!(
            move_to(sb.root(), "dir/x.md", "dir").expect("noop"),
            "dir/x.md"
        );
        assert_eq!(sb.read("dir/x.md"), "x");
    }

    #[test]
    fn move_to_collision_is_exists() {
        let sb = Sandbox::new("move-collision");
        sb.write("a.md", "root");
        sb.write("dest/a.md", "dest");
        assert_eq!(move_to(sb.root(), "a.md", "dest"), Err(FsError::Exists));
        assert_eq!(sb.read("a.md"), "root");
    }

    #[test]
    fn move_to_missing_destination_is_not_found() {
        let sb = Sandbox::new("move-missing-dest");
        sb.write("a.md", "a");
        assert_eq!(
            move_to(sb.root(), "a.md", "nowhere"),
            Err(FsError::NotFound)
        );
    }

    #[test]
    fn move_folder_into_descendant_is_invalid() {
        let sb = Sandbox::new("move-descendant");
        sb.write("a/b/c.md", "c");
        assert!(is_invalid(&move_to(sb.root(), "a", "a/b")));
        assert_eq!(sb.read("a/b/c.md"), "c");
    }

    #[test]
    fn delete_to_trash_moves_file_and_writes_sidecar() {
        let sb = Sandbox::new("trash-file");
        sb.write("gone.md", "bye");
        let entry = delete_to_trash(sb.root(), "gone.md").expect("delete");
        assert!(!sb.exists("gone.md"));
        assert_eq!(entry.original, "gone.md");
        assert!(!entry.is_dir);
        assert!(entry.name.ends_with("-gone.md"));
        assert!(sb.exists(&format!(".trash/{}", entry.name)));
        assert!(sb.exists(&format!(".trash/{}.json", entry.name)));
        assert_eq!(sb.read(&format!(".trash/{}", entry.name)), "bye");
    }

    #[test]
    fn delete_to_trash_folder_keeps_contents() {
        let sb = Sandbox::new("trash-folder");
        sb.write("proj/a.md", "a");
        sb.write("proj/deep/b.md", "b");
        let entry = delete_to_trash(sb.root(), "proj").expect("delete folder");
        assert!(entry.is_dir);
        assert!(!sb.exists("proj"));
        assert_eq!(sb.read(&format!(".trash/{}/deep/b.md", entry.name)), "b");
    }

    #[test]
    fn delete_to_trash_missing_is_not_found() {
        let sb = Sandbox::new("trash-missing");
        assert_eq!(
            delete_to_trash(sb.root(), "nope.md"),
            Err(FsError::NotFound)
        );
    }

    #[test]
    fn delete_inside_trash_is_invalid() {
        let sb = Sandbox::new("trash-inside");
        sb.write("a.md", "a");
        let entry = delete_to_trash(sb.root(), "a.md").expect("delete");
        let inside = format!(".trash/{}", entry.name);
        assert!(is_invalid(&delete_to_trash(sb.root(), &inside)));
    }

    #[test]
    fn list_trash_newest_first_and_skips_broken_sidecars() {
        let sb = Sandbox::new("trash-list");
        sb.write("one.md", "1");
        let first = delete_to_trash(sb.root(), "one.md").expect("first");
        sb.write("two.md", "2");
        let second = delete_to_trash(sb.root(), "two.md").expect("second");
        sb.write(".trash/broken-item.md", "x");
        sb.write(".trash/broken-item.md.json", "{ not json");
        sb.write(".trash/orphan.md.json", "{}");
        let listed = list_trash(sb.root()).expect("list");
        let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&first.name.as_str()));
        assert!(names.contains(&second.name.as_str()));
        assert!(!names.contains(&"broken-item.md"));
        assert!(!names.contains(&"orphan.md"));
        assert_eq!(listed.len(), 2);
        assert!(listed[0].deleted_ms >= listed[1].deleted_ms);
    }

    #[test]
    fn list_trash_without_folder_is_empty() {
        let sb = Sandbox::new("trash-none");
        assert_eq!(list_trash(sb.root()).expect("list"), Vec::new());
    }

    #[test]
    fn restore_round_trip_returns_to_original_path() {
        let sb = Sandbox::new("restore");
        sb.write("folder/note.md", "content");
        let entry = delete_to_trash(sb.root(), "folder/note.md").expect("delete");
        let restored = restore_from_trash(sb.root(), &entry).expect("restore");
        assert_eq!(restored, "folder/note.md");
        assert_eq!(sb.read("folder/note.md"), "content");
        assert!(list_trash(sb.root()).expect("list").is_empty());
        assert!(!sb.exists(&format!(".trash/{}.json", entry.name)));
    }

    #[test]
    fn restore_round_trip_for_folder() {
        let sb = Sandbox::new("restore-folder");
        sb.write("proj/x.md", "x");
        let entry = delete_to_trash(sb.root(), "proj").expect("delete");
        assert_eq!(
            restore_from_trash(sb.root(), &entry).expect("restore"),
            "proj"
        );
        assert_eq!(sb.read("proj/x.md"), "x");
    }

    #[test]
    fn restore_uses_unique_name_when_original_is_taken() {
        let sb = Sandbox::new("restore-taken");
        sb.write("note.md", "old");
        let entry = delete_to_trash(sb.root(), "note.md").expect("delete");
        sb.write("note.md", "new");
        let restored = restore_from_trash(sb.root(), &entry).expect("restore");
        assert_eq!(restored, "note 1.md");
        assert_eq!(sb.read("note.md"), "new");
        assert_eq!(sb.read("note 1.md"), "old");
    }

    #[test]
    fn restore_unknown_entry_is_not_found() {
        let sb = Sandbox::new("restore-unknown");
        let entry = TrashEntry {
            name: "1-ghost.md".to_string(),
            original: "ghost.md".to_string(),
            deleted_ms: 1,
            is_dir: false,
        };
        assert_eq!(
            restore_from_trash(sb.root(), &entry),
            Err(FsError::NotFound)
        );
    }

    #[test]
    fn restore_rejects_hostile_entry_names() {
        let sb = Sandbox::new("restore-hostile");
        let entry = TrashEntry {
            name: "../../escape".to_string(),
            original: "x.md".to_string(),
            deleted_ms: 1,
            is_dir: false,
        };
        assert!(is_invalid(&restore_from_trash(sb.root(), &entry)));
    }

    #[test]
    fn empty_trash_removes_everything_and_counts() {
        let sb = Sandbox::new("empty-trash");
        sb.write("a.md", "a");
        sb.write("dir/b.md", "b");
        delete_to_trash(sb.root(), "a.md").expect("a");
        delete_to_trash(sb.root(), "dir").expect("dir");
        assert_eq!(empty_trash(sb.root()).expect("empty"), 2);
        assert!(sb.names_in(".trash").is_empty());
        assert!(list_trash(sb.root()).expect("list").is_empty());
    }

    #[test]
    fn empty_trash_without_folder_returns_zero() {
        let sb = Sandbox::new("empty-trash-none");
        assert_eq!(empty_trash(sb.root()).expect("empty"), 0);
    }

    #[test]
    fn operations_refuse_missing_root() {
        let missing = std::env::temp_dir().join(format!(
            "jiyue-test-{}-fsops-no-such-root",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&missing);
        assert_eq!(create_folder(&missing, "x"), Err(FsError::NotFound));
        assert_eq!(list_trash(&missing), Err(FsError::NotFound));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn symlinked_folder_cannot_escape_root() {
        let sb = Sandbox::new("symlink-escape");
        let outside = Sandbox::new("symlink-escape-outside");
        let link = sb.root().join("link");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(outside.root(), &link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(outside.root(), &link);
        if made.is_err() {
            eprintln!("symlink creation not permitted here; skipping");
            return;
        }
        assert!(is_invalid(&create_note(sb.root(), "link", "escape")));
        assert!(is_invalid(&create_folder(sb.root(), "link/inner")));
        assert!(is_invalid(&delete_to_trash(sb.root(), "link")));
        assert!(!outside.exists("escape.md"));
        assert!(!outside.exists("inner"));
        assert!(outside.root().is_dir());
    }

    #[test]
    fn trash_entry_serializes_round_trip() {
        let entry = TrashEntry {
            name: "1-a.md".to_string(),
            original: "folder/a.md".to_string(),
            deleted_ms: 1,
            is_dir: false,
        };
        let text = serde_json::to_string(&entry).expect("serialize");
        let back: TrashEntry = serde_json::from_str(&text).expect("parse");
        assert_eq!(back, entry);
    }

    #[test]
    fn error_display_and_io_conversion() {
        assert_eq!(FsError::NotFound.to_string(), "not found");
        assert!(FsError::Invalid("x".into()).to_string().contains("x"));
        let io = std::io::Error::new(std::io::ErrorKind::AlreadyExists, "dup");
        assert_eq!(FsError::from(io), FsError::Exists);
        let io = std::io::Error::other("boom");
        assert!(matches!(FsError::from(io), FsError::Io(_)));
    }
}
