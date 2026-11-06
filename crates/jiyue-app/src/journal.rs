//! Crash safety for unsaved text. Three pieces:
//! - `Journals`: a background thread that writes and removes the recovery journal pairs in the
//!   order they were asked for, so a removal can never overtake an older write.
//! - the in-memory copy of every journal that should exist, which the panic hook writes out
//!   synchronously when the process panics.
//! - the panic hook itself, which also appends the panic and a backtrace to `crash.log`.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jiyue_core::recovery::{self, Journal};

/// The crash log is moved aside to `crash.log.1` once it grows past this size.
const CRASH_LOG_LIMIT: u64 = 1024 * 1024;

enum Command {
    Write(Journal),
    Remove(String),
    Flush(SyncSender<()>),
}

/// The journal copies that should be on disk, keyed by note path. The panic hook reads it.
struct Shadow {
    root: Option<PathBuf>,
    entries: BTreeMap<String, Journal>,
}

static SHADOW: Mutex<Shadow> = Mutex::new(Shadow {
    root: None,
    entries: BTreeMap::new(),
});

fn lock_shadow() -> MutexGuard<'static, Shadow> {
    SHADOW
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes and removes journal pairs on a background thread, in request order.
pub struct Journals {
    tx: Sender<Command>,
}

impl Journals {
    /// Starts the writer for the vault at `root` and makes `root` the folder the panic hook writes to.
    pub fn start(root: PathBuf) -> Self {
        lock_shadow().root = Some(root.clone());
        let (tx, rx) = mpsc::channel::<Command>();
        let spawned = std::thread::Builder::new()
            .name("jiyue-journal".into())
            .spawn(move || {
                for command in rx {
                    match command {
                        Command::Write(journal) => {
                            if let Err(err) = recovery::write(&root, &journal) {
                                eprintln!(
                                    "recovery: could not write the copy of {}: {err}",
                                    journal.path
                                );
                            }
                        }
                        Command::Remove(path) => {
                            if let Err(err) = recovery::remove(&root, &path) {
                                eprintln!("recovery: could not remove the copy of {path}: {err}");
                            }
                        }
                        Command::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            });
        if spawned.is_err() {
            eprintln!(
                "recovery: the journal thread could not start; copies are kept in memory only"
            );
        }
        Self { tx }
    }

    /// Queues a write of the copy of a note's unsaved text.
    pub fn write(&self, journal: Journal) {
        lock_shadow()
            .entries
            .insert(journal.path.clone(), journal.clone());
        let _ = self.tx.send(Command::Write(journal));
    }

    /// Queues the removal of a note's copy (after a successful save, or when the text is clean).
    pub fn remove(&self, path: &str) {
        lock_shadow().entries.remove(path);
        let _ = self.tx.send(Command::Remove(path.to_string()));
    }

    /// Waits until everything queued so far is on disk. False when the wait timed out.
    pub fn flush(&self, wait: Duration) -> bool {
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        if self.tx.send(Command::Flush(done_tx)).is_err() {
            return false;
        }
        done_rx.recv_timeout(wait).is_ok()
    }
}

/// Writes one journal copy now, on the calling thread (used while the app is closing).
pub fn write_now(root: &Path, journal: &Journal) {
    lock_shadow()
        .entries
        .insert(journal.path.clone(), journal.clone());
    if let Err(err) = recovery::write(root, journal) {
        eprintln!(
            "recovery: could not write the copy of {}: {err}",
            journal.path
        );
    }
}

/// Text and modification time (ms since the Unix epoch) of a note's disk file. Both are `None`
/// when the file does not exist or cannot be read.
pub fn disk_state(root: &Path, note_path: &str) -> (Option<String>, Option<i64>) {
    let abs = root.join(note_path);
    let text = fs::read_to_string(&abs).ok();
    let mtime = fs::metadata(&abs)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(millis_of);
    (text, mtime)
}

/// Milliseconds since the Unix epoch of a time, or `None` before the epoch.
pub fn millis_of(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}

/// Now, in ms since the Unix epoch (0 if the clock is before the epoch).
pub fn now_ms() -> i64 {
    millis_of(SystemTime::now()).unwrap_or(0)
}

/// Hidden test switch: `JIYUE_TEST_PANIC=1` makes the app panic once its first journal copy is
/// queued. Only the crash test sets it.
pub fn test_panic_requested() -> bool {
    std::env::var("JIYUE_TEST_PANIC").is_ok_and(|value| value == "1")
}

/// Where the crash log goes: `<vault>/.jiyue/crash.log`, or the app data folder when no vault is
/// known yet.
pub fn crash_log_path(root: Option<&Path>) -> PathBuf {
    match root {
        Some(root) => root.join(".jiyue").join("crash.log"),
        None => jiyue_core::config::user_dir().join("crash.log"),
    }
}

/// Installs the panic hook. Before the panic is reported, the journal copies in memory are written
/// to disk and the panic with a backtrace is appended to the crash log. The previous hook then
/// runs as before. Best effort: the hook never panics itself.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        emergency(info);
        previous(info);
    }));
}

fn emergency(info: &std::panic::PanicHookInfo<'_>) {
    // A panic while the shadow lock is held must not deadlock the hook, so it is only tried.
    let guard = match SHADOW.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    };
    let root = guard.as_ref().and_then(|shadow| shadow.root.clone());
    if let (Some(shadow), Some(root)) = (guard.as_ref(), root.as_ref()) {
        for journal in shadow.entries.values() {
            let _ = recovery::write(root, journal);
        }
    }
    drop(guard);

    let location = info
        .location()
        .map(|l| format!("{}:{}", l.file(), l.line()))
        .unwrap_or_default();
    let report = format!(
        "[{}] panic at {location}: {}\n{}\n",
        now_ms(),
        payload_text(info),
        std::backtrace::Backtrace::force_capture()
    );
    append_log(&crash_log_path(root.as_deref()), &report);
}

fn payload_text(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "(panic with a non-text payload)".to_string()
    }
}

/// Appends `text` to the crash log, first moving a log over the size limit to `crash.log.1`.
fn append_log(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::metadata(path).is_ok_and(|meta| meta.len() > CRASH_LOG_LIMIT) {
        let _ = fs::rename(path, path.with_extension("log.1"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jiyue-journal-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn writer_writes_then_removes_in_order() {
        let root = scratch("writer");
        let journals = Journals::start(root.clone());
        journals.write(Journal {
            path: "a.md".into(),
            base_mtime_ms: None,
            saved_at_ms: 1,
            text: "unsaved".into(),
        });
        assert!(journals.flush(Duration::from_secs(5)));
        let loaded = recovery::load_all(&root);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text, "unsaved");

        journals.remove("a.md");
        assert!(journals.flush(Duration::from_secs(5)));
        assert!(recovery::load_all(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn crash_log_defaults_under_vault() {
        let root = PathBuf::from("vault");
        assert_eq!(
            crash_log_path(Some(&root)),
            PathBuf::from("vault").join(".jiyue").join("crash.log")
        );
    }

    #[test]
    fn append_log_rotates_when_over_limit() {
        let dir = scratch("rotate");
        let log = dir.join("crash.log");
        fs::write(&log, vec![b'x'; (CRASH_LOG_LIMIT + 1) as usize]).expect("big log");
        append_log(&log, "new panic\n");
        assert_eq!(fs::read_to_string(&log).expect("fresh log"), "new panic\n");
        assert!(dir.join("crash.log.1").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
