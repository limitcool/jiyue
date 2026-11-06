//! Vault management: the recent-vaults list, path rules, the checks before a vault is opened, and
//! the hand-over to a new process when the app switches vault (pure logic, no gpui).
//!
//! The app switches vault by restarting itself: the current window saves everything and finishes
//! its exit sync, then the process starts a new copy of the same exe with `--vault <path>` and
//! exits. The new copy waits until the old process has gone (see `wait_for_parent_exit`), so the
//! two never hold the vault at the same time. See `ui/vaults_ui.rs` for the screens.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::theme::ThemeKind;
use jiyue_core::platform;

/// Most vaults kept in the recent list.
pub const RECENT_LIMIT: usize = 10;
/// File name of the recent-vaults list (`state.toml`) in the user's config folder.
pub const RECENT_FILE: &str = jiyue_core::config::USER_STATE_FILE;
/// Test hook: when set, the folder picker returns this path without showing a dialog (tests only).
pub const TEST_PICK_ENV: &str = "JIYUE_TEST_PICK_VAULT";
/// A vault with more files than this asks for confirmation before it opens.
pub const LARGE_VAULT_FILES: usize = 20_000;
/// The file count stops here (one more than the limit is enough to decide).
const COUNT_CAP: usize = LARGE_VAULT_FILES + 1;

/// The separator of this platform's paths.
const SEP: char = if cfg!(windows) { '\\' } else { '/' };

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Cleans a path for storage and comparison: strips the `\\?\` verbatim prefix (Windows), uses
/// backslashes (Windows), and removes trailing separators except in a root (`C:\`, `\\server\share\`
/// and `/`). Nothing else changes, so CJK characters and spaces are kept.
pub fn normalize_path(raw: &str) -> String {
    let mut s = raw.trim_end_matches(['\r', '\n']).to_string();
    if cfg!(windows) {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            s = format!(r"\\{rest}");
        } else if let Some(rest) = s.strip_prefix(r"\\?\") {
            s = rest.to_string();
        }
        s = s.replace('/', "\\");
    }
    let root = root_len(&s);
    while s.len() > root && s.ends_with(SEP) {
        s.pop();
    }
    s
}

/// Length of the root part of a normalised path (`C:\`, `\\server\share\`, `/`), or 0.
fn root_len(s: &str) -> usize {
    if cfg!(windows) {
        let b = s.as_bytes();
        if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
            return 3;
        }
        if let Some(rest) = s.strip_prefix(r"\\") {
            let mut seps = rest.match_indices('\\').map(|(i, _)| i);
            return match (seps.next(), seps.next()) {
                (Some(_), Some(share_end)) => 2 + share_end + 1,
                _ => s.len(),
            };
        }
        0
    } else {
        usize::from(s.starts_with('/'))
    }
}

/// The key two paths are compared by: the normalised path, lower-cased on Windows (which ignores
/// case). CJK and spaces are unaffected.
pub fn dedup_key(path: &str) -> String {
    let n = normalize_path(path);
    if cfg!(windows) {
        n.to_ascii_lowercase()
    } else {
        n
    }
}

/// True when two paths name the same folder under the rules of `dedup_key`.
pub fn same_path(a: &str, b: &str) -> bool {
    !a.is_empty() && dedup_key(a) == dedup_key(b)
}

/// The last component of a path, or the whole path for a root.
pub fn display_name(path: &str) -> String {
    let n = normalize_path(path);
    match n.rsplit(SEP).next() {
        Some(name) if !name.is_empty() && !name.ends_with(':') => name.to_string(),
        _ => n,
    }
}

/// Shortens `text` to at most `max` characters by cutting the middle: `C:\Users\…\笔记库`.
/// Counts characters, not bytes, so CJK text is cut cleanly.
pub fn middle_truncate(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if max == 0 || chars.len() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep.div_ceil(2);
    let tail = keep - head;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(chars[chars.len() - tail..].iter());
    out
}

// ---------------------------------------------------------------------------
// Recent vaults
// ---------------------------------------------------------------------------

/// The persisted list in `state.toml`: the last vault opened (`last_vault`) and the most recent
/// vaults, newest first (`recent_vaults`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentStore {
    /// The vault opened most recently (normally `recent[0]`). Cleared when that vault is removed.
    #[serde(rename = "last_vault")]
    pub last: Option<String>,
    /// Most recent first, at most `RECENT_LIMIT` entries, no duplicates.
    #[serde(rename = "recent_vaults")]
    pub recent: Vec<String>,
}

impl RecentStore {
    /// Records that `path` was opened: it becomes the last vault and the first of the list.
    pub fn record_opened(&mut self, path: &str) {
        let p = normalize_path(path);
        if p.is_empty() {
            return;
        }
        let key = dedup_key(&p);
        self.recent.retain(|r| dedup_key(r) != key);
        self.recent.insert(0, p.clone());
        self.recent.truncate(RECENT_LIMIT);
        self.last = Some(p);
    }

    /// Removes `path` from the list only. Files are never touched.
    pub fn remove(&mut self, path: &str) {
        let key = dedup_key(path);
        self.recent.retain(|r| dedup_key(r) != key);
        if self.last.as_deref().is_some_and(|l| dedup_key(l) == key) {
            self.last = None;
        }
    }

    /// The vault opened most recently, if it is still known.
    pub fn last_opened(&self) -> Option<&str> {
        self.last.as_deref()
    }

    /// Reads the file text. A broken file gives an empty store and a warning, never an error.
    pub fn parse(text: &str) -> (Self, Option<String>) {
        let loaded: jiyue_core::config::Loaded<Self> =
            jiyue_core::config::doc::load_text(std::path::Path::new(RECENT_FILE), text);
        match loaded.error {
            None => (loaded.value.sanitised(), None),
            Some(issue) => (
                RecentStore::default(),
                Some(format!("最近笔记库文件无法读取，已忽略：{issue}")),
            ),
        }
    }

    /// Normalises entries, drops empty and duplicate ones and applies the size limit.
    fn sanitised(mut self) -> Self {
        let mut seen: Vec<String> = Vec::new();
        let mut recent = Vec::new();
        for raw in self.recent.drain(..) {
            let p = normalize_path(&raw);
            let key = dedup_key(&p);
            if p.is_empty() || seen.contains(&key) {
                continue;
            }
            seen.push(key);
            recent.push(p);
            if recent.len() == RECENT_LIMIT {
                break;
            }
        }
        self.recent = recent;
        self.last = self
            .last
            .take()
            .map(|l| normalize_path(&l))
            .filter(|l| !l.is_empty());
        self
    }
}

/// One line of the recent list, ready for the screens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentRow {
    pub path: String,
    pub name: String,
    /// It is the vault open in this window.
    pub current: bool,
    /// Its folder does not exist any more.
    pub missing: bool,
}

/// The rows of the recent list. `exists` tells whether a folder is still there.
pub fn recent_rows(
    store: &RecentStore,
    current: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> Vec<RecentRow> {
    store
        .recent
        .iter()
        .map(|path| RecentRow {
            path: path.clone(),
            name: display_name(path),
            current: current.is_some_and(|c| same_path(c, path)),
            missing: !exists(path),
        })
        .collect()
}

/// Where the recent-vaults list lives: `state.toml` in the user's config folder (`JIYUE_CONFIG_DIR`).
pub fn store_path() -> PathBuf {
    jiyue_core::config::user_file(RECENT_FILE)
}

/// Reads the store at `path`. A missing file is an empty store; a broken one gives a warning.
pub fn load_store(path: &Path) -> (RecentStore, Option<String>) {
    match fs::read_to_string(path) {
        Ok(text) => RecentStore::parse(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (RecentStore::default(), None),
        Err(err) => (
            RecentStore::default(),
            Some(format!("无法读取最近笔记库文件：{err}")),
        ),
    }
}

/// Writes the store (merged into `state.toml`, comments kept). Refused while the file is broken,
/// so a file the user is fixing is never overwritten.
pub fn save_store(path: &Path, store: &RecentStore) -> Result<(), String> {
    let check: jiyue_core::config::Loaded<RecentStore> = jiyue_core::config::load_file(path);
    if let Some(issue) = check.error {
        return Err(format!("状态文件有错误，未保存：{issue}"));
    }
    jiyue_core::config::save_file(path, store, jiyue_core::config::templates::USER_STATE, &[])
}

/// Reads the store fresh from disk, applies `change` and writes it back. Reading first means two
/// windows never overwrite each other's list with a stale copy.
pub fn update_store(change: impl FnOnce(&mut RecentStore)) -> Result<(), String> {
    let path = store_path();
    let (mut store, _) = load_store(&path);
    change(&mut store);
    save_store(&path, &store)
}

/// Records that the vault at `path` was opened. Failures are returned for a warning only.
pub fn record_opened(path: &str) -> Result<(), String> {
    update_store(|s| s.record_opened(path))
}

// ---------------------------------------------------------------------------
// Checks before a vault is opened
// ---------------------------------------------------------------------------

/// What the user asked for: open an existing folder, or create a vault in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Open,
    Create,
}

/// What the disk looks like for a candidate folder (see `inspect`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub exists: bool,
    pub is_dir: bool,
    pub readable: bool,
    pub writable: bool,
    pub is_empty: bool,
    /// Files below the folder, counted up to `LARGE_VAULT_FILES + 1`.
    pub file_count: usize,
}

/// Folders that are rarely a vault and that hold other programs' or the user's own files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemFolder {
    DriveRoot,
    UserHome,
    Documents,
    Windows,
    ProgramFiles,
    ProgramData,
}

impl SystemFolder {
    /// The Chinese name used in the confirmation.
    pub fn label(self) -> &'static str {
        match self {
            SystemFolder::DriveRoot => "磁盘根目录",
            SystemFolder::UserHome => "用户主文件夹",
            SystemFolder::Documents => "文档文件夹",
            SystemFolder::Windows => "系统文件夹",
            SystemFolder::ProgramFiles => "程序文件夹",
            SystemFolder::ProgramData => "程序数据文件夹",
        }
    }
}

/// Which system folder a candidate is, if any. `home` and `documents` are the user's folders.
pub fn system_folder(
    path: &str,
    home: Option<&str>,
    documents: Option<&str>,
) -> Option<SystemFolder> {
    let n = normalize_path(path);
    let key = dedup_key(&n);
    let is_root = if cfg!(windows) {
        n.len() == 3 && n.as_bytes()[1] == b':'
    } else {
        n == "/"
    };
    if is_root {
        return Some(SystemFolder::DriveRoot);
    }
    if home.is_some_and(|h| same_path(h, &n)) {
        return Some(SystemFolder::UserHome);
    }
    if documents.is_some_and(|d| same_path(d, &n)) {
        return Some(SystemFolder::Documents);
    }
    if cfg!(windows) {
        let inside = |prefix: &str| key == prefix || key.starts_with(&format!("{prefix}\\"));
        if inside("c:\\windows") {
            return Some(SystemFolder::Windows);
        }
        if inside("c:\\program files") || inside("c:\\program files (x86)") {
            return Some(SystemFolder::ProgramFiles);
        }
        if inside("c:\\programdata") {
            return Some(SystemFolder::ProgramData);
        }
    }
    None
}

/// True when the path is, or is inside, a `.jiyue`, `.git` or `target` folder. Such a folder
/// belongs to another vault, a repository or a build, and is never opened as a vault.
pub fn is_inside_internal_folder(path: &str) -> bool {
    normalize_path(path).split(SEP).any(|part| {
        let p = if cfg!(windows) {
            part.to_ascii_lowercase()
        } else {
            part.to_string()
        };
        p == ".jiyue" || p == ".git" || p == "target"
    })
}

/// The decision for one candidate folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Switch to the folder now.
    Switch,
    /// It is the vault already open: only a note for the user.
    AlreadyCurrent,
    /// Refused, with the message to show.
    Error(String),
    /// Ask first. The warnings are shown as one message.
    Confirm {
        title: String,
        message: String,
        confirm: String,
    },
}

/// The rules for a candidate folder, in the order they are checked. Pure: the disk is in `facts`.
pub fn decide(
    mode: Mode,
    path: &str,
    current: Option<&str>,
    facts: &Facts,
    system: Option<SystemFolder>,
) -> Action {
    if current.is_some_and(|c| same_path(c, path)) {
        return Action::AlreadyCurrent;
    }
    if normalize_path(path).is_empty() {
        return Action::Error("请选择一个文件夹".into());
    }
    if mode == Mode::Open && !facts.exists {
        return Action::Error("这个文件夹不存在，可能已被移动或删除".into());
    }
    if facts.exists && !facts.is_dir {
        return Action::Error("这是一个文件，请选择一个文件夹".into());
    }
    if is_inside_internal_folder(path) {
        return Action::Error("不能把 .jiyue、.git 或 target 里面的文件夹当作笔记库".into());
    }
    if facts.is_dir && !facts.readable {
        return Action::Error("无法读取这个文件夹，请检查它的权限".into());
    }
    if facts.is_dir && !facts.writable {
        return Action::Error(
            "这个文件夹不可写入，无法保存笔记。请检查权限或选择其他文件夹".into(),
        );
    }
    let mut warnings: Vec<String> = Vec::new();
    if mode == Mode::Create && facts.exists && !facts.is_empty {
        warnings.push("这个文件夹不是空的，将作为已有笔记库打开，不会创建示例笔记。".into());
    }
    if let Some(kind) = system {
        warnings.push(format!(
            "这看起来是{}。把它当作笔记库，其中的文件都会被当作笔记。",
            kind.label()
        ));
    }
    if facts.file_count > LARGE_VAULT_FILES {
        warnings.push(format!(
            "这个文件夹里超过 {LARGE_VAULT_FILES} 个文件，打开和建立索引可能需要较长时间。"
        ));
    }
    if warnings.is_empty() {
        Action::Switch
    } else {
        Action::Confirm {
            title: "请确认".into(),
            message: warnings.join("\n"),
            confirm: "仍然打开".into(),
        }
    }
}

/// Looks at the disk for a candidate folder. For `Mode::Open` the folder must exist. Counts files
/// up to the cap, skipping hidden folders. Writability is tested by writing a probe file under
/// `.jiyue` (created if missing), which is removed again.
pub fn inspect(path: &Path) -> Facts {
    let mut facts = Facts::default();
    let Ok(meta) = fs::metadata(path) else {
        return facts;
    };
    facts.exists = true;
    facts.is_dir = meta.is_dir();
    if !facts.is_dir {
        return facts;
    }
    match fs::read_dir(path) {
        Ok(mut entries) => {
            facts.readable = true;
            facts.is_empty = entries.next().is_none();
        }
        Err(_) => return facts,
    }
    facts.writable = probe_write(path);
    facts.file_count = count_files(path, COUNT_CAP);
    facts
}

/// Creates `.jiyue` in `path` and writes and removes a probe file in it.
fn probe_write(path: &Path) -> bool {
    let dir = path.join(".jiyue");
    if fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".write-probe-{}", std::process::id()));
    let ok = fs::File::create(&probe)
        .and_then(|mut f| f.write_all(b"ok"))
        .is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// Counts the files under `root`, stopping at `cap`. Folders whose name starts with a dot are not
/// entered.
fn count_files(root: &Path, cap: usize) -> usize {
    let mut count = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                if !name.to_string_lossy().starts_with('.') {
                    stack.push(entry.path());
                }
            } else {
                count += 1;
                if count >= cap {
                    return count;
                }
            }
        }
    }
    count
}

/// Makes the folder for `Mode::Create` (its parents too). An existing folder is kept.
pub fn prepare_folder(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("无法创建文件夹：{e}"))
}

/// The user's home folder as a string, for `system_folder`.
pub fn home_string() -> Option<String> {
    let env = platform::Env::from_process();
    env.home.map(|h| normalize_path(&h.display().to_string()))
}

/// The user's Documents folder as a string, for `system_folder`.
pub fn documents_string() -> String {
    normalize_path(&platform::documents_dir().display().to_string())
}

/// The sample vault created by the first-run chooser: `Documents\Jiyue Vault`.
pub fn sample_vault_path() -> PathBuf {
    platform::documents_dir().join("Jiyue Vault")
}

// ---------------------------------------------------------------------------
// Hand-over to a new process
// ---------------------------------------------------------------------------

/// What the new process should open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relaunch {
    pub vault: String,
    pub theme: String,
}

/// The theme name the command line understands.
pub fn theme_arg(kind: ThemeKind) -> &'static str {
    match kind {
        ThemeKind::Paper => "paper",
        ThemeKind::Ink => "ink",
        ThemeKind::Moss => "moss",
        ThemeKind::Graphite => "graphite",
        ThemeKind::Fog => "fog",
    }
}

/// The arguments of the new process: the vault, the theme and the process to wait for.
pub fn relaunch_args(vault: &str, theme: &str, parent_pid: u32) -> Vec<String> {
    vec![
        "--vault".into(),
        vault.into(),
        "--theme".into(),
        theme.into(),
        "--wait-parent".into(),
        parent_pid.to_string(),
    ]
}

/// The `--wait-parent` value of a command line, when it is a number.
pub fn parse_wait_parent(args: &[String]) -> Option<u32> {
    let i = args.iter().position(|a| a == "--wait-parent")?;
    args.get(i + 1)?.parse().ok()
}

/// The hand-over request set by the window that switches vault. `main` acts on it after the
/// event loop has ended, once the old window and its locks are gone.
static RELAUNCH: Mutex<Option<Relaunch>> = Mutex::new(None);

/// Asks for the new process to start after this one has quit.
pub fn request_relaunch(vault: &str, kind: ThemeKind) {
    if let Ok(mut slot) = RELAUNCH.lock() {
        *slot = Some(Relaunch {
            vault: vault.to_string(),
            theme: theme_arg(kind).to_string(),
        });
    }
}

/// The pending hand-over, if any.
pub fn take_relaunch() -> Option<Relaunch> {
    RELAUNCH.lock().ok().and_then(|mut slot| slot.take())
}

/// Starts a new copy of this exe for `request`. Its window appears when this process has quit.
pub fn spawn_relaunch(request: &Relaunch) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .args(relaunch_args(
            &request.vault,
            &request.theme,
            std::process::id(),
        ))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Waits until the process `pid` has exited, for at most `limit`. Used by a relaunched process so
/// that its history and journal locks are free. Does nothing when the process is already gone.
pub fn wait_for_parent_exit(pid: u32, limit: Duration) {
    #[cfg(windows)]
    {
        const SYNCHRONIZE: u32 = 0x0010_0000;
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, millis: u32) -> u32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        // SAFETY: plain Win32 calls on a handle opened and closed here. A null handle means the
        // process has already gone, so there is nothing to wait for.
        unsafe {
            let handle = OpenProcess(SYNCHRONIZE, 0, pid);
            if !handle.is_null() {
                let millis = limit.as_millis().min(u32::MAX as u128) as u32;
                WaitForSingleObject(handle, millis);
                CloseHandle(handle);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        std::thread::sleep(Duration::from_millis(300).min(limit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn record_puts_the_newest_first_and_dedups() {
        let mut st = RecentStore::default();
        st.record_opened("D:\\a");
        st.record_opened("D:\\b");
        st.record_opened("D:\\a");
        assert_eq!(st.recent, s(&["D:\\a", "D:\\b"]));
        assert_eq!(st.last_opened(), Some("D:\\a"));
    }

    #[test]
    fn list_is_limited_to_ten() {
        let mut st = RecentStore::default();
        for i in 0..15 {
            st.record_opened(&format!("n{i}"));
        }
        assert_eq!(st.recent.len(), RECENT_LIMIT);
        assert_eq!(st.recent[0], "n14");
        assert_eq!(st.recent[9], "n5");
        assert_eq!(st.last_opened(), Some("n14"));
    }

    #[test]
    fn remove_only_drops_the_entry_and_clears_last_when_it_matches() {
        let mut st = RecentStore::default();
        st.record_opened("na");
        st.record_opened("nb");
        st.remove("nb");
        assert_eq!(st.recent, s(&["na"]));
        assert_eq!(st.last_opened(), None);
        st.remove("missing");
        assert_eq!(st.recent.len(), 1);
    }

    #[test]
    fn toml_round_trip_and_tolerant_reads() {
        let mut st = RecentStore::default();
        st.record_opened("一 二 笔记");
        let (back, warn) = RecentStore::parse(&toml::to_string(&st).expect("serialise"));
        assert_eq!(warn, None);
        assert_eq!(back.recent, st.recent);
        assert_eq!(back.last, st.last);

        let (broken, warn) = RecentStore::parse("recent_vaults = [");
        assert!(broken.recent.is_empty());
        assert!(warn.is_some());

        // Unknown fields and missing fields are fine; duplicates and empties are dropped.
        let (lenient, warn) = RecentStore::parse(
            "recent_vaults = [\"x\", \"\", \"x\", \"y\"]
extra = true
",
        );
        assert_eq!(warn, None);
        assert_eq!(lenient.recent, s(&["x", "y"]));
        assert_eq!(lenient.last, None);
    }

    #[test]
    fn missing_store_file_is_empty_without_warning() {
        let dir = std::env::temp_dir().join(format!("jiyue-vaults-missing-{}", std::process::id()));
        let (st, warn) = load_store(&dir.join("nope.json"));
        assert!(st.recent.is_empty());
        assert_eq!(warn, None);
    }

    #[test]
    fn rows_mark_current_and_missing() {
        let mut st = RecentStore::default();
        st.record_opened("/notes/gone");
        st.record_opened("/notes/here");
        let rows = recent_rows(&st, Some("/notes/gone"), |p| p.ends_with("here"));
        assert_eq!(rows[0].name, "here");
        assert!(!rows[0].missing);
        assert!(!rows[0].current);
        assert_eq!(rows[1].name, "gone");
        assert!(rows[1].current);
        assert!(rows[1].missing);
    }

    #[test]
    fn middle_truncation_keeps_both_ends_and_counts_characters() {
        assert_eq!(middle_truncate("short", 10), "short");
        assert_eq!(middle_truncate("abcdefghij", 5), "ab…ij");
        assert_eq!(
            middle_truncate("我的笔记库在很长很长的路径里", 6),
            "我的笔…径里"
        );
        assert_eq!(middle_truncate("abc", 0), "abc");
    }

    #[test]
    fn display_name_is_the_last_component() {
        assert_eq!(display_name("notes/vault"), "vault");
        assert_eq!(display_name("notes/我的 笔记/"), "我的 笔记");
    }

    #[test]
    fn internal_folders_are_detected_by_any_component() {
        assert!(is_inside_internal_folder("/a/.jiyue"));
        assert!(is_inside_internal_folder("/a/.jiyue/x"));
        assert!(is_inside_internal_folder("/a/.git/objects"));
        assert!(is_inside_internal_folder("/a/target/debug"));
        assert!(!is_inside_internal_folder("/a/notes"));
        assert!(!is_inside_internal_folder("/a/my.jiyue.notes"));
    }

    #[test]
    fn system_folders_are_recognised() {
        assert_eq!(
            system_folder("/home/ann", Some("/home/ann"), Some("/home/ann/Documents")),
            Some(SystemFolder::UserHome)
        );
        assert_eq!(
            system_folder(
                "/home/ann/Documents",
                Some("/home/ann"),
                Some("/home/ann/Documents")
            ),
            Some(SystemFolder::Documents)
        );
        assert_eq!(
            system_folder("/home/ann/notes", Some("/home/ann"), None),
            None
        );
        if cfg!(windows) {
            assert_eq!(
                system_folder("C:\\", None, None),
                Some(SystemFolder::DriveRoot)
            );
            assert_eq!(
                system_folder("d:/", None, None),
                Some(SystemFolder::DriveRoot)
            );
            assert_eq!(
                system_folder("c:\\WINDOWS\\Fonts", None, None),
                Some(SystemFolder::Windows)
            );
            assert_eq!(system_folder("D:\\notes", None, None), None);
        } else {
            assert_eq!(
                system_folder("/", None, None),
                Some(SystemFolder::DriveRoot)
            );
        }
    }

    fn facts(exists: bool, is_dir: bool) -> Facts {
        Facts {
            exists,
            is_dir,
            readable: is_dir,
            writable: is_dir,
            is_empty: true,
            file_count: 3,
        }
    }

    #[test]
    fn decisions_follow_the_order_of_the_rules() {
        let f = facts(true, true);
        assert_eq!(
            decide(Mode::Open, "/n/v", Some("/n/v"), &f, None),
            Action::AlreadyCurrent
        );
        assert!(matches!(
            decide(Mode::Open, "/n/missing", None, &facts(false, false), None),
            Action::Error(_)
        ));
        // Creating a new folder is fine when it does not exist yet.
        assert_eq!(
            decide(Mode::Create, "/n/new", None, &facts(false, false), None),
            Action::Switch
        );
        assert!(matches!(
            decide(Mode::Open, "/n/file.md", None, &facts(true, false), None),
            Action::Error(_)
        ));
        assert!(matches!(
            decide(Mode::Open, "/n/.jiyue/x", None, &f, None),
            Action::Error(_)
        ));
        let unreadable = Facts {
            readable: false,
            ..facts(true, true)
        };
        assert!(matches!(
            decide(Mode::Open, "/n/v", None, &unreadable, None),
            Action::Error(_)
        ));
        let readonly = Facts {
            writable: false,
            ..facts(true, true)
        };
        assert!(matches!(
            decide(Mode::Open, "/n/v", None, &readonly, None),
            Action::Error(_)
        ));
        assert_eq!(decide(Mode::Open, "/n/v", None, &f, None), Action::Switch);
    }

    #[test]
    fn non_empty_folder_for_a_new_vault_asks_to_open_it() {
        let busy = Facts {
            is_empty: false,
            ..facts(true, true)
        };
        match decide(Mode::Create, "/n/old", None, &busy, None) {
            Action::Confirm { message, .. } => assert!(message.contains("不是空的")),
            other => panic!("expected a confirmation, got {other:?}"),
        }
        // Opening a folder with notes in it (an Obsidian vault) needs no question.
        assert_eq!(
            decide(Mode::Open, "/n/old", None, &busy, None),
            Action::Switch
        );
    }

    #[test]
    fn large_and_system_folders_need_confirmation() {
        let big = Facts {
            file_count: LARGE_VAULT_FILES + 1,
            ..facts(true, true)
        };
        assert!(matches!(
            decide(Mode::Open, "/n/big", None, &big, None),
            Action::Confirm { .. }
        ));
        let at_limit = Facts {
            file_count: LARGE_VAULT_FILES,
            ..facts(true, true)
        };
        assert_eq!(
            decide(Mode::Open, "/n/ok", None, &at_limit, None),
            Action::Switch
        );
        match decide(
            Mode::Open,
            "/home/ann",
            None,
            &facts(true, true),
            Some(SystemFolder::UserHome),
        ) {
            Action::Confirm { message, .. } => assert!(message.contains("用户主文件夹")),
            other => panic!("expected a confirmation, got {other:?}"),
        }
    }

    #[test]
    fn inspect_counts_files_and_probes_writes() {
        let dir = std::env::temp_dir().join(format!("jiyue-vaults-inspect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::create_dir_all(dir.join(".hidden")).unwrap();
        fs::write(dir.join("a.md"), "x").unwrap();
        fs::write(dir.join("sub").join("b.md"), "x").unwrap();
        fs::write(dir.join(".hidden").join("c.md"), "x").unwrap();
        let f = inspect(&dir);
        assert!(f.exists && f.is_dir && f.readable && f.writable);
        assert!(!f.is_empty);
        assert_eq!(f.file_count, 2);
        assert!(dir.join(".jiyue").is_dir());
        assert!(!inspect(&dir.join("nope")).exists);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A fresh folder under the temp dir, unique per test run.
    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("jiyue-vaults-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// Create a vault in a new folder (samples appear), open it again, then refuse the bad
    /// candidates: a file, a folder inside `.jiyue`, a non-empty folder for a new vault, and the
    /// current vault itself.
    #[test]
    fn create_then_open_existing_vault_and_validation_errors() {
        let root = scratch("flow");
        let vault_dir = root.join("My 笔记");
        // Create: the folder does not exist yet, so the check makes it and finds it empty.
        let facts = probe_for_test(&vault_dir, Mode::Create).expect("folder is created");
        assert!(facts.exists && facts.is_empty && facts.writable);
        assert_eq!(
            decide(
                Mode::Create,
                &vault_dir.display().to_string(),
                None,
                &facts,
                None
            ),
            Action::Switch
        );
        // The app's own path: an empty folder gets the sample notes.
        let vault = jiyue_core::vault::Vault::open_or_create(&vault_dir);
        assert!(!vault.notes.is_empty(), "sample notes are written");

        // Open: the folder now has notes, so it opens without a question.
        let facts = inspect(&vault_dir);
        assert!(!facts.is_empty && facts.file_count >= 1);
        let current = vault_dir.display().to_string();
        assert_eq!(
            decide(Mode::Open, &current, None, &facts, None),
            Action::Switch
        );
        // Creating over a non-empty folder asks first.
        assert!(matches!(
            decide(Mode::Create, &current, None, &facts, None),
            Action::Confirm { .. }
        ));
        // The vault that is already open is only reported.
        assert_eq!(
            decide(Mode::Open, &current, Some(&current), &facts, None),
            Action::AlreadyCurrent
        );
        // A file is refused.
        let file = vault_dir.join("沉积的想法.md");
        let file_facts = inspect(&file);
        assert!(matches!(
            decide(
                Mode::Open,
                &file.display().to_string(),
                None,
                &file_facts,
                None
            ),
            Action::Error(_)
        ));
        // A folder inside `.jiyue` is refused.
        let inside = vault_dir.join(".jiyue");
        assert!(matches!(
            decide(
                Mode::Open,
                &inside.display().to_string(),
                None,
                &inspect(&inside),
                None
            ),
            Action::Error(_)
        ));
        // A missing folder cannot be opened.
        let gone = root.join("gone");
        assert!(matches!(
            decide(
                Mode::Open,
                &gone.display().to_string(),
                None,
                &inspect(&gone),
                None
            ),
            Action::Error(_)
        ));
        let _ = fs::remove_dir_all(&root);
    }

    /// The same as `probe` in the window, without gpui.
    fn probe_for_test(path: &Path, mode: Mode) -> Result<Facts, String> {
        if mode == Mode::Create && !path.exists() {
            prepare_folder(path)?;
        }
        Ok(inspect(path))
    }

    #[test]
    fn relaunch_arguments_round_trip_the_parent_pid() {
        let args = relaunch_args("/notes/My 笔记", "fog", 4242);
        assert_eq!(parse_wait_parent(&args), Some(4242));
        assert_eq!(args[1], "/notes/My 笔记");
        assert_eq!(parse_wait_parent(&s(&["--wait-parent", "x"])), None);
        assert_eq!(theme_arg(ThemeKind::Fog), "fog");
    }

    #[test]
    fn hand_over_is_taken_once() {
        request_relaunch("/notes/next", ThemeKind::Paper);
        let first = take_relaunch();
        assert_eq!(
            first,
            Some(Relaunch {
                vault: "/notes/next".into(),
                theme: "paper".into()
            })
        );
        assert_eq!(take_relaunch(), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_are_normalised_and_compared_without_case() {
        assert_eq!(normalize_path(r"\\?\D:\notes\"), r"D:\notes");
        assert_eq!(normalize_path(r"\\?\UNC\srv\share\n\"), r"\\srv\share\n");
        assert_eq!(normalize_path("d:/NOTES/"), r"d:\NOTES");
        assert_eq!(normalize_path(r"C:\"), r"C:\");
        assert_eq!(normalize_path(r"\\srv\share\"), r"\\srv\share\");
        assert!(same_path(r"D:\Notes", r"d:\notes\"));
        assert!(same_path(r"D:\我的 笔记", r"\\?\D:\我的 笔记"));
        assert!(!same_path(r"D:\a", r"D:\ab"));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_paths_keep_their_case() {
        assert_eq!(normalize_path("/notes/"), "/notes");
        assert_eq!(normalize_path("/"), "/");
        assert!(!same_path("/Notes", "/notes"));
        assert!(same_path("/我的 笔记/", "/我的 笔记"));
    }
}
