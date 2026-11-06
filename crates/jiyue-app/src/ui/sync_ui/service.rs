//! Runs one sync at a time on a plain background thread. The UI polls `SyncService::poll` on a
//! timer; the thread writes progress and the final outcome into a shared slot. No tokio runtime is
//! involved on the UI side: `jiyue-sync` builds its own runtime inside the store.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jiyue_sync::state::{load_state, now_ms};
use jiyue_sync::store::OpendalStore;
use jiyue_sync::{SyncOptions, SyncProgress, SyncReport, run_sync};

use crate::ui::history_ui;

use super::config::{SyncConfig, test_override};
use super::conflicts;
use super::schedule::backoff_secs;
use super::secrets;

/// Why a run started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    Manual,
    Auto,
    Start,
    AfterSave,
    /// The last sync before the window closes (see `sync_ui::on_close_requested`).
    Exit,
}

impl RunKind {
    /// True for runs the user asked for; their failures are toasts, not silent retries.
    pub fn is_manual(self) -> bool {
        self == RunKind::Manual
    }
}

/// Live progress of the running sync, copied from the worker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub action: &'static str,
    pub path: String,
}

/// Result of one finished run.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub kind: RunKind,
    pub report: SyncReport,
    /// A failure that stopped the run before or during the remote work (settings, secrets, network).
    pub error: Option<String>,
    /// The failure was a network problem, so the next automatic run backs off.
    pub network: bool,
    pub finished_ms: i64,
}

/// Everything the worker needs. Built on the UI thread from the saved settings.
pub struct Job {
    pub root: PathBuf,
    pub cfg: SyncConfig,
    pub kind: RunKind,
    /// The version history, when it is open. The worker keeps the text of each note it is about to
    /// overwrite or delete.
    pub history: Option<history_ui::Store>,
}

/// Before the sync changes a local note (a download, a merge or a local delete), keeps the note's
/// current text in the history, so the change can be undone from there. Notes only; a failure is
/// logged and does not stop the run.
fn keep_before_action(store: &history_ui::Store, root: &Path, action: &str, path: &str) {
    let Some(keep) = keep_for(action, path) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(root.join(path)) else {
        return;
    };
    let result = match keep {
        Keep::Text => history_ui::keep_now(store, path, &text),
        Keep::Deleted => history_ui::record_deleted_now(store, path, &text),
    };
    if let Err(e) = result {
        eprintln!("history: could not keep the text of {path} before sync: {e}");
    }
}

/// What the history keeps of a local note that a sync action is about to change.
#[derive(Debug, PartialEq, Eq)]
enum Keep {
    /// The text on disk, as a version.
    Text,
    /// The text on disk, as a `Deleted` version (the note moves to the sync trash).
    Deleted,
}

/// The kind of history entry for a sync action (`download`, `merge`, `delete-local`, ...) on
/// `path`. `None` when the action leaves the local note alone or the file is not a note.
fn keep_for(action: &str, path: &str) -> Option<Keep> {
    if !path.to_ascii_lowercase().ends_with(".md") {
        return None;
    }
    match action {
        "download" | "merge" => Some(Keep::Text),
        "delete-local" => Some(Keep::Deleted),
        _ => None,
    }
}

#[cfg(test)]
mod keep_tests {
    use super::*;

    #[test]
    fn only_local_note_changes_keep_a_version() {
        assert_eq!(keep_for("download", "a/b.md"), Some(Keep::Text));
        assert_eq!(keep_for("merge", "b.MD"), Some(Keep::Text));
        assert_eq!(keep_for("delete-local", "b.md"), Some(Keep::Deleted));
        assert_eq!(keep_for("upload", "b.md"), None);
        assert_eq!(keep_for("conflict", "b.md"), None);
        assert_eq!(keep_for("delete-remote", "b.md"), None);
        assert_eq!(keep_for("download", "image.png"), None);
    }
}

#[derive(Default)]
struct Shared {
    progress: Progress,
    outcome: Option<Outcome>,
}

/// Background sync runner and its bookkeeping (one run at a time, backoff, save debounce).
pub struct SyncService {
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    /// The run in progress, if any.
    pub running: Option<RunKind>,
    /// Last progress copied from the worker.
    pub progress: Progress,
    /// Last finished run.
    pub last: Option<Outcome>,
    /// When the last run ended (monotonic), for the interval timer. The app start counts at first.
    pub last_end: Instant,
    /// When a note was last saved, for the save-triggered run.
    pub last_save: Option<Instant>,
    /// A save happened and no save-triggered run has started since.
    pub save_pending: bool,
    /// Network failures in a row.
    pub failures: u32,
    /// No automatic run before this instant (backoff after network failures).
    pub next_auto_at: Option<Instant>,
}

impl Default for SyncService {
    fn default() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            running: None,
            progress: Progress::default(),
            last: None,
            last_end: Instant::now(),
            last_save: None,
            save_pending: false,
            failures: 0,
            next_auto_at: None,
        }
    }
}

impl SyncService {
    /// Starts a run on a background thread. Returns false (and does nothing) when a run is going.
    pub fn start(&mut self, job: Job) -> bool {
        if self.running.is_some() {
            return false;
        }
        self.cancel.store(false, Ordering::SeqCst);
        if let Ok(mut shared) = self.shared.lock() {
            shared.progress = Progress::default();
            shared.outcome = None;
        }
        self.running = Some(job.kind);
        self.progress = Progress::default();
        let shared = self.shared.clone();
        let cancel = self.cancel.clone();
        let kind = job.kind;
        let spawned = std::thread::Builder::new()
            .name("jiyue-sync".into())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| execute(job, &shared, &cancel)))
                    .unwrap_or_else(|_| Outcome {
                        kind,
                        report: SyncReport::default(),
                        error: Some("同步意外中断".into()),
                        network: false,
                        finished_ms: now_ms(),
                    });
                if let Ok(mut slot) = shared.lock() {
                    slot.outcome = Some(outcome);
                }
            });
        if spawned.is_err() {
            self.running = None;
            return false;
        }
        true
    }

    /// Asks the running sync to stop before its next action.
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Copies progress from the worker. Returns the outcome when the run has just finished.
    pub fn poll(&mut self) -> Option<Outcome> {
        let finished = {
            let Ok(mut shared) = self.shared.lock() else {
                return None;
            };
            self.progress = shared.progress.clone();
            shared.outcome.take()
        };
        let outcome = finished?;
        self.running = None;
        self.last_end = Instant::now();
        if outcome.network {
            self.failures = self.failures.saturating_add(1);
            self.next_auto_at =
                Some(Instant::now() + Duration::from_secs(backoff_secs(self.failures)));
        } else if outcome.error.is_none() {
            self.failures = 0;
            self.next_auto_at = None;
        }
        self.last = Some(outcome.clone());
        Some(outcome)
    }

    /// Records a note save (starts or restarts the save debounce).
    pub fn note_saved(&mut self) {
        self.save_pending = true;
        self.last_save = Some(Instant::now());
    }

    /// True while the automatic runs must wait for a backoff.
    pub fn backing_off(&self) -> bool {
        self.next_auto_at.is_some_and(|t| Instant::now() < t)
    }
}

/// Opens the store for `cfg`. `typed` is a secret typed in the form (empty to use the stored one).
/// The test-only environment override replaces the S3 fields. Blocking: call from a background thread.
pub fn open_store(cfg: &SyncConfig, typed: &str) -> Result<OpendalStore, String> {
    let over = test_override();
    let secret = if over.is_some() {
        String::new()
    } else if !typed.is_empty() {
        typed.to_owned()
    } else {
        secrets::load(cfg)?.ok_or_else(|| "未找到密钥，请在同步设置中填写并保存".to_owned())?
    };
    OpendalStore::from_config(&cfg.store_config(&secret, over.as_ref())).map_err(|e| e.to_string())
}

/// The worker body. Runs on the background thread; `shared` gets progress and the result.
fn execute(job: Job, shared: &Mutex<Shared>, cancel: &AtomicBool) -> Outcome {
    let kind = job.kind;
    let finish = |report: SyncReport, error: Option<String>, network: bool| Outcome {
        kind,
        report,
        error,
        network,
        finished_ms: now_ms(),
    };
    let root = job.root.clone();
    let history = job.history.clone();
    let override_s3 = test_override();

    let secret = if override_s3.is_some() {
        String::new()
    } else {
        match secrets::load(&job.cfg) {
            Ok(Some(s)) => s,
            Ok(None) => {
                return finish(
                    SyncReport::default(),
                    Some("未找到密钥，请在同步设置中填写并保存".into()),
                    false,
                );
            }
            Err(e) => return finish(SyncReport::default(), Some(e), false),
        }
    };
    if override_s3.is_none()
        && let Err(e) = job.cfg.validate(true)
    {
        return finish(SyncReport::default(), Some(e), false);
    }

    let store =
        match OpendalStore::from_config(&job.cfg.store_config(&secret, override_s3.as_ref())) {
            Ok(store) => store,
            Err(e) => return finish(SyncReport::default(), Some(e.to_string()), false),
        };
    let mut state = match load_state(&root) {
        Ok(state) => state,
        Err(e) => {
            return finish(
                SyncReport::default(),
                Some(format!("同步状态无法读取：{e}")),
                false,
            );
        }
    };

    let mut on_progress = |p: SyncProgress| {
        // Called before each action: the text on disk is still the one the action replaces.
        if let Some(store) = history.as_ref() {
            keep_before_action(store, &root, p.action, &p.path);
        }
        if let Ok(mut slot) = shared.lock() {
            slot.progress = Progress {
                done: p.done,
                total: p.total,
                action: p.action,
                path: p.path,
            };
        }
    };
    let device = if job.cfg.device_name.trim().is_empty() {
        super::config::default_device_name()
    } else {
        job.cfg.device_name.trim().to_owned()
    };
    let mut opts = SyncOptions::new(device);
    opts.ignore = job.cfg.ignore_rules();
    opts.cancel = Some(cancel);
    opts.progress = Some(&mut on_progress);

    let mut report = run_sync(&root, &store, &mut state, &mut opts);

    // Record the conflicts of this run next to the ones still open.
    let mut items = conflicts::load(&root);
    let mut changed = conflicts::add_records(&mut items, &report.conflicts);
    changed |= conflicts::prune(&mut items, |copy| root.join(copy).exists());
    if changed && let Err(e) = conflicts::save(&root, &items) {
        report.errors.push(jiyue_sync::FileError {
            path: "conflicts.json".into(),
            message: format!("无法保存冲突列表：{e}"),
        });
    }
    let network = report
        .fatal
        .as_deref()
        .is_some_and(|f| f.contains("network error") || f.contains("cannot reach"));
    let error = report.fatal.clone();
    finish(report, error, network)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_start_is_refused_while_running() {
        let mut svc = SyncService {
            running: Some(RunKind::Manual),
            ..SyncService::default()
        };
        let job = Job {
            root: std::env::temp_dir(),
            cfg: SyncConfig::default(),
            kind: RunKind::Auto,
            history: None,
        };
        assert!(!svc.start(job));
        assert_eq!(svc.running, Some(RunKind::Manual));
    }

    #[test]
    fn network_failures_back_off_and_success_resets() {
        let mut svc = SyncService::default();
        for _ in 0..3 {
            svc.running = Some(RunKind::Auto);
            svc.shared.lock().unwrap().outcome = Some(Outcome {
                kind: RunKind::Auto,
                report: SyncReport::default(),
                error: Some("cannot list the remote: network error: timeout".into()),
                network: true,
                finished_ms: 1,
            });
            assert!(svc.poll().is_some());
        }
        assert_eq!(svc.failures, 3);
        assert!(svc.backing_off());
        svc.running = Some(RunKind::Auto);
        svc.shared.lock().unwrap().outcome = Some(Outcome {
            kind: RunKind::Auto,
            report: SyncReport::default(),
            error: None,
            network: false,
            finished_ms: 2,
        });
        svc.poll();
        assert_eq!(svc.failures, 0);
        assert!(!svc.backing_off());
    }

    #[test]
    fn poll_copies_progress_without_outcome() {
        let mut svc = SyncService::default();
        svc.shared.lock().unwrap().progress = Progress {
            done: 3,
            total: 9,
            action: "upload",
            path: "a.md".into(),
        };
        assert!(svc.poll().is_none());
        assert_eq!(svc.progress.done, 3);
        assert_eq!(svc.progress.total, 9);
    }

    fn write_file(root: &std::path::Path, rel: &str, text: &str) {
        let path = root.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("create dir");
        }
        std::fs::write(path, text).expect("write note");
    }

    fn read_file(root: &std::path::Path, rel: &str) -> String {
        std::fs::read_to_string(root.join(rel)).expect("read note")
    }

    /// Runs the production worker (`execute`) for one vault. Secrets and the endpoint come from
    /// the `JIYUE_TEST_S3_*` environment (see `rustfs_run.py`).
    fn run_vault(root: &std::path::Path, cfg: &SyncConfig) -> Outcome {
        let shared = Mutex::new(Shared::default());
        let cancel = AtomicBool::new(false);
        let job = Job {
            root: root.to_path_buf(),
            cfg: cfg.clone(),
            kind: RunKind::Manual,
            history: None,
        };
        execute(job, &shared, &cancel)
    }

    fn assert_clean(o: &Outcome) {
        assert!(o.error.is_none(), "run error: {:?}", o.error);
        assert!(
            o.report.errors.is_empty(),
            "file errors: {:?}",
            o.report.errors
        );
    }

    /// Two vault copies (A and B) sync through the real RustFS test bucket: upload, download,
    /// clean merge, conflict with markers, resolution, versions, delete and restore.
    /// Run with: `python rustfs_run.py -- cargo test -p jiyue rustfs_two_devices -- --ignored`.
    #[test]
    #[ignore = "needs the RustFS test bucket; run through rustfs_run.py (see the report)"]
    fn rustfs_two_devices_round_trip() {
        use jiyue_sync::RemoteStore;
        use jiyue_sync::merge::MARKER_LOCAL;
        use jiyue_sync::{ConflictKind, state::write_atomic};

        use super::super::config::test_override;
        use super::conflicts;

        assert!(
            test_override().is_some(),
            "JIYUE_TEST_S3_* is not set; run through rustfs_run.py"
        );
        let stamp = now_ms();
        let base = std::env::temp_dir().join(format!("jiyue-it-{}-{stamp}", std::process::id()));
        let dir_a = base.join("A");
        let dir_b = base.join("B");
        std::fs::create_dir_all(&dir_a).expect("A dir");
        std::fs::create_dir_all(&dir_b).expect("B dir");
        let cfg_a = SyncConfig {
            enabled: true,
            s3_root_prefix: format!("jiyue-it/{stamp}/"),
            device_name: "DEVA".into(),
            ..SyncConfig::default()
        };
        let cfg_b = SyncConfig {
            device_name: "DEVB".into(),
            ..cfg_a.clone()
        };

        let store = open_store(&cfg_a, "").expect("store");
        let check = store.check_connection();
        assert!(
            check.reachable && check.can_list && check.can_write,
            "{check:?}"
        );
        assert!(check.can_delete && check.supports_versions, "{check:?}");
        assert!(check.conditional_put_supported, "{check:?}");

        // 1. A creates a note, B downloads it.
        write_file(&dir_a, "a.md", "one\ntwo\nthree\nfour\nfive\n");
        let o = run_vault(&dir_a, &cfg_a);
        assert_clean(&o);
        assert!(
            o.report.uploaded.contains(&"a.md".to_owned()),
            "{:?}",
            o.report
        );
        let o = run_vault(&dir_b, &cfg_b);
        assert_clean(&o);
        assert!(
            o.report.downloaded.contains(&"a.md".to_owned()),
            "{:?}",
            o.report
        );
        assert_eq!(read_file(&dir_b, "a.md"), "one\ntwo\nthree\nfour\nfive\n");

        // 2. Edits to different lines, made on both sides before syncing: merged cleanly.
        write_file(&dir_a, "a.md", "ONE\ntwo\nthree\nfour\nfive\n");
        assert_clean(&run_vault(&dir_a, &cfg_a));
        write_file(&dir_b, "a.md", "one\ntwo\nthree\nfour\nFIVE\n");
        let o = run_vault(&dir_b, &cfg_b);
        assert_clean(&o);
        assert!(o.report.conflicts.is_empty(), "{:?}", o.report.conflicts);
        assert_eq!(read_file(&dir_b, "a.md"), "ONE\ntwo\nthree\nfour\nFIVE\n");
        assert_clean(&run_vault(&dir_a, &cfg_a));
        assert_eq!(read_file(&dir_a, "a.md"), "ONE\ntwo\nthree\nfour\nFIVE\n");

        // 3. The same line edited on both sides: conflict markers and a conflict copy.
        write_file(&dir_a, "a.md", "ONE\nA-two\nthree\nfour\nFIVE\n");
        assert_clean(&run_vault(&dir_a, &cfg_a));
        write_file(&dir_b, "a.md", "ONE\nB-two\nthree\nfour\nFIVE\n");
        let o = run_vault(&dir_b, &cfg_b);
        assert_clean(&o);
        assert_eq!(o.report.conflicts.len(), 1, "{:?}", o.report);
        let rec = o.report.conflicts[0].clone();
        assert_eq!(rec.kind, ConflictKind::Markers);
        assert!(dir_b.join(&rec.copy_path).exists(), "copy missing");
        assert!(
            read_file(&dir_b, "a.md").contains(MARKER_LOCAL),
            "no markers"
        );
        let items = conflicts::load(&dir_b);
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].copy_path, rec.copy_path);

        // 4. The user keeps the local text; the resolution reaches A.
        conflicts::keep_local(&dir_b, &items[0]).expect("keep local");
        assert_eq!(read_file(&dir_b, "a.md"), "ONE\nB-two\nthree\nfour\nFIVE\n");
        let mut items = items;
        items.clear();
        conflicts::save(&dir_b, &items).expect("save list");
        assert_clean(&run_vault(&dir_b, &cfg_b));
        assert_clean(&run_vault(&dir_a, &cfg_a));
        assert_eq!(read_file(&dir_a, "a.md"), "ONE\nB-two\nthree\nfour\nFIVE\n");
        assert!(conflicts::load(&dir_b).is_empty());

        // 5. Version history: every sync above left a version; the oldest is the first text.
        let versions = store.list_version_details("a.md").expect("versions");
        assert!(versions.len() >= 4, "{versions:?}");
        let oldest = versions.last().expect("oldest version");
        let oldest_id = oldest.entry.version_id.clone().expect("version id");
        let bytes = store
            .get("a.md", Some(oldest_id.as_str()))
            .expect("old version");
        assert_eq!(
            String::from_utf8(bytes).expect("utf8"),
            "one\ntwo\nthree\nfour\nfive\n"
        );

        // 6. Delete on A reaches B (moved to the trash there); the delete marker is listed.
        std::fs::remove_file(dir_a.join("a.md")).expect("remove");
        let o = run_vault(&dir_a, &cfg_a);
        assert_clean(&o);
        assert!(
            o.report.deleted_remote.contains(&"a.md".to_owned()),
            "{:?}",
            o.report
        );
        let o = run_vault(&dir_b, &cfg_b);
        assert_clean(&o);
        assert!(
            o.report.deleted_local.contains(&"a.md".to_owned()),
            "{:?}",
            o.report
        );
        assert!(!dir_b.join("a.md").exists());
        let versions = store.list_version_details("a.md").expect("versions");
        assert!(versions.iter().any(|v| v.is_delete_marker), "{versions:?}");

        // 7. Restoring an old version: write it on A, sync, B receives it.
        let restored = store
            .get("a.md", Some(oldest_id.as_str()))
            .expect("old version again");
        write_atomic(&dir_a.join("a.md"), &restored).expect("write restored");
        assert_clean(&run_vault(&dir_a, &cfg_a));
        assert_clean(&run_vault(&dir_b, &cfg_b));
        assert_eq!(read_file(&dir_b, "a.md"), "one\ntwo\nthree\nfour\nfive\n");

        std::fs::remove_dir_all(&base).ok();
    }
}
