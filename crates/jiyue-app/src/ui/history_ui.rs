//! Version history of notes. Owned by W3-H.
//!
//! Every save of a note takes a local version under `<vault>/.jiyue/history` (at most one
//! automatic version per note per interval; a save inside the interval is taken once the interval
//! has passed). A change that arrives from outside the app (sync, another editor) is recorded as
//! the new state. The 历史版本 dialog lists the versions of the active note, shows a diff against
//! the editor text, and restores one as a single undoable edit. Restoring keeps the current text
//! as a `BeforeRestore` version first.
//!
//! Layout: `pure` (throttling, labels, times, counts; unit tested), `ops` (blocking work on the
//! store, run through `jobs::spawn_blocking`), `view` (the dialog).
//!
//! The store is opened once at start-up in the background. Another process holding the same
//! directory makes the history unavailable (a toast and a hint in the dialog); nothing panics.

mod ops;
pub mod pure;
mod view;

use std::collections::HashMap;
use std::time::Duration;

use gpui::{AnyElement, ClipboardItem, Context, KeyDownEvent, Task, prelude::*};

use crate::AppCommand;
use crate::RustdianApp;
use crate::jobs::spawn_blocking;
use crate::ui::ext::{ExtCommand, ExtEvent, PaletteCommand, ToastKind};
use crate::ui::widgets::overlay;

pub use ops::{Loaded, OpenError, Row, SnapOutcome, Store};

/// Commands this module handles (sent as `ExtCommand::History`).
#[derive(Clone, Debug)]
pub enum Command {
    /// 历史版本…: opens the dialog for the active note.
    Open,
}

/// The store, once it is opened.
#[derive(Default)]
pub enum Backend {
    /// The background open has not finished.
    #[default]
    Opening,
    Ready(Store),
    /// Versions cannot be kept; the text is shown in the dialog.
    Unavailable(String),
}

/// Module state.
#[derive(Default)]
pub struct State {
    pub backend: Backend,
    /// The dialog is open.
    pub modal: bool,
    /// Vault-relative path of the note the dialog is about.
    pub path: String,
    pub loading_list: bool,
    pub list_error: Option<String>,
    pub rows: Vec<Row>,
    /// Index into `rows` of the version shown on the right.
    pub selected: Option<usize>,
    pub loaded: Option<Loaded>,
    /// Version (entry id) that `loaded` belongs to. While a newer pick loads, `loaded` still shows
    /// the previous version; copying uses this to copy only the text that is really shown.
    pub loaded_id: Option<u64>,
    /// Diffs already computed in this session, so flipping back and forth does not recompute them.
    pub diff_cache: pure::DiffCache<Loaded>,
    pub loading_version: bool,
    /// Message under the version view.
    pub note: String,
    /// A restore is running.
    pub busy: bool,
    /// Fingerprint of the text the app last saved for each note. A file on disk that differs from it
    /// was changed from outside; a file that matches it is the echo of our own write.
    saved_hash: HashMap<String, u64>,
    /// Automatic versions waiting for the interval to pass, per note. Replacing one cancels it.
    pending: HashMap<String, Task<()>>,
}

/// Opens the store in the background. Called once, at start-up.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let root = app.vault.root_dir.clone();
    app.ext.history.backend = Backend::Opening;
    spawn_blocking(
        cx,
        move || ops::open(&root),
        |app, result, cx| {
            let backend = match result {
                Ok(store) => Backend::Ready(store),
                Err(OpenError::Locked) => {
                    let text = pure::unavailable_text("", true);
                    app.toast(ToastKind::Warn, "历史版本已停用：历史库正被另一个窗口使用");
                    Backend::Unavailable(text)
                }
                Err(OpenError::Other(e)) => {
                    app.toast(ToastKind::Warn, "历史版本已停用，请查看历史版本窗口");
                    Backend::Unavailable(pure::unavailable_text(&e, false))
                }
            };
            app.ext.history.backend = backend;
            if app.ext.history.modal {
                load_rows(app, cx);
            }
            cx.notify();
        },
    );
}

fn ready_store(app: &RustdianApp) -> Option<Store> {
    match &app.ext.history.backend {
        Backend::Ready(store) => Some(store.clone()),
        _ => None,
    }
}

/// The text of a note as the app holds it: the editor text when open, else the note in memory.
fn current_text(app: &RustdianApp, path: &str, cx: &gpui::App) -> String {
    if let Some(editor) = app.editor_for(path) {
        return editor.read(cx).text();
    }
    app.vault
        .notes
        .iter()
        .find(|n| n.path == path)
        .map(|n| n.content().to_string())
        .unwrap_or_default()
}

/// Takes an automatic version of `path` from its text in memory, in the background. Waits for the
/// interval (see `schedule_retry`) when the previous version is too recent.
fn snapshot_path(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    let Some(store) = ready_store(app) else {
        return;
    };
    let Some(text) = note_text(app, path) else {
        return;
    };
    let key = path.to_string();
    let now = jiyue_history::system_now_ms();
    spawn_blocking(cx, move || ops::snapshot_auto(&store, &key, &text, now), {
        let path = path.to_string();
        move |app, outcome, cx| match outcome {
            SnapOutcome::Throttled(left) => schedule_retry(app, path, left, cx),
            SnapOutcome::Failed(e) => {
                app.toast(ToastKind::Warn, format!("历史版本未能保存：{e}"));
                cx.notify();
            }
            SnapOutcome::Created | SnapOutcome::Unchanged => {}
        }
    });
}

/// Runs one automatic version of `path` once the interval has passed, if the note is still open
/// and clean (its saved text is what the version should hold).
fn schedule_retry(
    app: &mut RustdianApp,
    path: String,
    left_ms: i64,
    cx: &mut Context<RustdianApp>,
) {
    let delay = Duration::from_millis(left_ms.max(1) as u64);
    let key = path.clone();
    let task = cx.spawn(async move |this, cx| {
        cx.background_executor().timer(delay).await;
        let _ = this.update(cx, |app, cx| {
            if !app.is_dirty(&key, cx) {
                snapshot_path(app, &key, cx);
            }
        });
    });
    app.ext.history.pending.insert(path, task);
}

/// Saves (`NoteSaved`) take an automatic version. Files that changed on disk (`VaultFilesChanged`)
/// whose text differs from the last saved text take a version of their new state, in the background.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    match ev {
        ExtEvent::NoteSaved(path) => {
            if let Some(text) = note_text(app, path) {
                app.ext
                    .history
                    .saved_hash
                    .insert(path.clone(), pure::text_hash(&text));
            }
            snapshot_path(app, path, cx);
        }
        ExtEvent::VaultFilesChanged(paths) => external_changed(app, paths, cx),
        _ => {}
    }
}

/// The text of a note in memory (the saved text for a note that is clean).
fn note_text(app: &RustdianApp, path: &str) -> Option<String> {
    app.vault
        .notes
        .iter()
        .find(|n| n.path == path)
        .map(|n| n.content().to_string())
}

/// Changed notes whose disk text is not the text the app last saved get a version of the disk text.
fn external_changed(app: &mut RustdianApp, paths: &[String], cx: &mut Context<RustdianApp>) {
    let Some(store) = ready_store(app) else {
        return;
    };
    let changed: Vec<(String, Option<u64>)> = paths
        .iter()
        .filter(|p| p.to_ascii_lowercase().ends_with(".md"))
        .map(|p| (p.clone(), app.ext.history.saved_hash.get(p).copied()))
        .collect();
    if changed.is_empty() {
        return;
    }
    let root = app.vault.root_dir.clone();
    let now = jiyue_history::system_now_ms();
    spawn_blocking(
        cx,
        move || ops::external_changes(&store, &root, &changed, now),
        |app, recorded, cx| {
            for (path, hash, outcome) in recorded {
                app.ext.history.saved_hash.insert(path.clone(), hash);
                if let SnapOutcome::Failed(e) = outcome {
                    app.toast(ToastKind::Warn, format!("历史版本未能保存：{e}"));
                }
            }
            cx.notify();
        },
    );
}

/// Opens the dialog for the active note.
pub fn open_modal(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(path) = app.active_path().map(str::to_owned) else {
        app.toast(ToastKind::Info, "没有打开的笔记");
        return;
    };
    let h = &mut app.ext.history;
    h.modal = true;
    h.path = path;
    h.rows.clear();
    h.list_error = None;
    h.selected = None;
    h.loaded = None;
    h.loaded_id = None;
    h.loading_version = false;
    h.note.clear();
    load_rows(app, cx);
    cx.notify();
}

/// Closes the dialog.
pub fn close_modal(app: &mut RustdianApp) {
    let h = &mut app.ext.history;
    h.modal = false;
    h.loaded = None;
    h.loaded_id = None;
    h.loading_version = false;
    h.selected = None;
}

/// Reads the versions of the dialog's note in the background.
fn load_rows(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(store) = ready_store(app) else {
        // Still opening: the open task reloads when it finishes.
        return;
    };
    let path = app.ext.history.path.clone();
    app.ext.history.loading_list = true;
    let asked = path.clone();
    spawn_blocking(
        cx,
        move || ops::list_rows(&store, &asked),
        move |app, result, cx| {
            if app.ext.history.path != path {
                return;
            }
            let h = &mut app.ext.history;
            h.loading_list = false;
            match result {
                Ok(rows) => {
                    h.rows = rows;
                    h.list_error = None;
                }
                Err(e) => h.list_error = Some(e),
            }
            cx.notify();
        },
    );
}

/// Shows one version and its diff against the editor text.
pub fn select(app: &mut RustdianApp, index: usize, cx: &mut Context<RustdianApp>) {
    let Some(row) = app.ext.history.rows.get(index).cloned() else {
        return;
    };
    let Some(store) = ready_store(app) else {
        return;
    };
    let path = app.ext.history.path.clone();
    let current = current_text(app, &path, cx);
    let key = pure::DiffKey::new(row.id.0, &current);
    let h = &mut app.ext.history;
    h.selected = Some(index);
    h.note.clear();
    if let Some(cached) = h.diff_cache.get(key) {
        h.loading_version = false;
        h.loaded = Some(cached);
        h.loaded_id = Some(row.id.0);
        cx.notify();
        return;
    }
    // The previous diff stays on screen until this one is ready: no empty frame in between.
    h.loading_version = true;
    let asked = path.clone();
    let id = row.id;
    spawn_blocking(
        cx,
        move || ops::load_version(&store, id, &current),
        move |app, result, cx| {
            if app.ext.history.path != asked {
                return;
            }
            let h = &mut app.ext.history;
            // A slow result for an earlier pick is still cached, but it must not replace the view.
            let is_current = h.selected == Some(index);
            match result {
                Ok(loaded) => {
                    h.diff_cache.insert(key, loaded.clone());
                    if is_current {
                        h.loaded = Some(loaded);
                        h.loaded_id = Some(id.0);
                    }
                }
                Err(e) => {
                    if is_current {
                        h.loaded = None;
                        h.loaded_id = None;
                        h.note = format!("无法读取此版本：{e}");
                    }
                }
            }
            if is_current {
                h.loading_version = false;
            }
            cx.notify();
        },
    );
    cx.notify();
}

/// 恢复此版本: the editor text becomes the version as ONE undoable edit (Ctrl+Z goes back). The
/// current text is kept as a `BeforeRestore` version first. Autosave then writes the note.
pub fn restore(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.ext.history.busy {
        return;
    }
    let Some(index) = app.ext.history.selected else {
        return;
    };
    let Some(row) = app.ext.history.rows.get(index).cloned() else {
        return;
    };
    let path = app.ext.history.path.clone();
    let Some(editor) = app.editor_for(&path) else {
        app.toast(ToastKind::Info, "笔记未打开，无法恢复");
        cx.notify();
        return;
    };
    let Some(store) = ready_store(app) else {
        return;
    };
    let current = editor.read(cx).text();
    app.ext.history.busy = true;
    let id = row.id;
    spawn_blocking(
        cx,
        move || ops::restore(&store, id, &current, jiyue_history::system_now_ms()),
        move |app, result, cx| {
            app.ext.history.busy = false;
            match result {
                Ok(text) => {
                    if let Some(editor) = app.editor_for(&path) {
                        editor.update(cx, |ed, cx| ed.set_text_undoable(&text, cx));
                    }
                    close_modal(app);
                    app.toast(ToastKind::Success, "已恢复此版本，可按 Ctrl+Z 撤销");
                }
                Err(e) => app.toast(ToastKind::Error, format!("恢复失败：{e}")),
            }
            cx.notify();
        },
    );
    cx.notify();
}

/// 复制此版本文本: the text of the selected version goes to the clipboard.
pub fn copy_selected(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    // Copy only the version the pane shows: while a newer pick loads, `loaded` is the old one.
    let h = &app.ext.history;
    let selected_id = h.selected.and_then(|i| h.rows.get(i)).map(|r| r.id.0);
    let shown = h.loaded_id.filter(|id| Some(*id) == selected_id);
    let Some(text) = h
        .loaded
        .as_ref()
        .filter(|_| shown.is_some())
        .map(|l| l.text.clone())
    else {
        return;
    };
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    app.toast(ToastKind::Success, "已复制此版本文本");
    cx.notify();
}

/// Call before notes are renamed or moved (`pairs` are the old and new vault-relative paths of the
/// notes that move; a folder gives one pair per note). Keeps a version of each note as it is now
/// and moves the history to the new path. The text is read here, so call it before the change.
pub fn before_rename(
    app: &mut RustdianApp,
    pairs: &[(String, String)],
    cx: &mut Context<RustdianApp>,
) {
    let Some(store) = ready_store(app) else {
        return;
    };
    let work: Vec<(String, String, Option<String>)> = pairs
        .iter()
        .map(|(old, new)| (old.clone(), new.clone(), note_text(app, old)))
        .collect();
    let now = jiyue_history::system_now_ms();
    spawn_blocking(
        cx,
        move || {
            let mut first_error = None;
            for (old, new, text) in &work {
                if let Some(text) = text {
                    let _ = ops::snapshot_now(&store, old, text, now);
                }
                if let Err(e) = ops::follow_rename(&store, old, new) {
                    first_error.get_or_insert(e);
                }
            }
            first_error
        },
        |app, error: Option<String>, cx| {
            if let Some(e) = error {
                app.toast(ToastKind::Warn, format!("历史版本未能跟随改名：{e}"));
            }
            cx.notify();
        },
    );
}

/// Call before notes are deleted (to the trash). Their last text stays in the history as a
/// `Deleted` version. Pass every note that goes, folders included.
pub fn before_delete(app: &mut RustdianApp, paths: &[String], cx: &mut Context<RustdianApp>) {
    let Some(store) = ready_store(app) else {
        return;
    };
    let work: Vec<(String, String)> = paths
        .iter()
        .filter_map(|p| note_text(app, p).map(|t| (p.clone(), t)))
        .collect();
    if work.is_empty() {
        return;
    }
    let now = jiyue_history::system_now_ms();
    spawn_blocking(
        cx,
        move || {
            let mut first_error = None;
            for (path, text) in &work {
                if let Err(e) = ops::record_deleted(&store, path, text, now) {
                    first_error.get_or_insert(e);
                }
            }
            first_error
        },
        |app, error: Option<String>, cx| {
            if let Some(e) = error {
                app.toast(ToastKind::Warn, format!("历史版本未能记录删除：{e}"));
            }
            cx.notify();
        },
    );
}

/// Call before notes change in bulk (a sync overwrite, an AI apply). Keeps a version of each note
/// as it is now, whatever the interval says, so the change can be undone from the history.
pub fn before_change(app: &mut RustdianApp, paths: &[String], cx: &mut Context<RustdianApp>) {
    let Some(store) = ready_store(app) else {
        return;
    };
    let work: Vec<(String, String)> = paths
        .iter()
        .filter_map(|p| note_text(app, p).map(|t| (p.clone(), t)))
        .collect();
    if work.is_empty() {
        return;
    }
    let now = jiyue_history::system_now_ms();
    spawn_blocking(
        cx,
        move || {
            let mut first_error = None;
            for (path, text) in &work {
                if let SnapOutcome::Failed(e) = ops::snapshot_now(&store, path, text, now) {
                    first_error.get_or_insert(e);
                }
            }
            first_error
        },
        |app, error: Option<String>, cx| {
            if let Some(e) = error {
                app.toast(ToastKind::Warn, format!("历史版本未能保存：{e}"));
            }
            cx.notify();
        },
    );
}

/// The open store, for background workers (the sync worker takes a clone of it).
pub fn store_of(app: &RustdianApp) -> Option<Store> {
    ready_store(app)
}

/// Keeps `text` as a version of `path` now, without the interval. For worker threads: the caller
/// gives the text it is about to replace.
pub fn keep_now(store: &Store, path: &str, text: &str) -> Result<(), String> {
    match ops::snapshot_now(store, path, text, jiyue_history::system_now_ms()) {
        SnapOutcome::Failed(e) => Err(e),
        SnapOutcome::Created | SnapOutcome::Unchanged | SnapOutcome::Throttled(_) => Ok(()),
    }
}

/// Records the last text of a note that a worker deleted (as a `Deleted` version).
pub fn record_deleted_now(store: &Store, path: &str, text: &str) -> Result<(), String> {
    ops::record_deleted(store, path, text, jiyue_history::system_now_ms())
}

/// Keeps the text an edit of an open note is about to replace (AI apply). Runs on a short thread,
/// so it needs no `Context`; a failure is logged to stderr only.
pub fn keep_before_edit(app: &RustdianApp, path: &str, text: &str) {
    let Some(store) = ready_store(app) else {
        return;
    };
    let path = path.to_string();
    let text = text.to_string();
    let spawned = std::thread::Builder::new()
        .name("jiyue-history".into())
        .spawn(move || {
            if let Err(e) = keep_now(&store, &path, &text) {
                eprintln!("history: could not keep the text before an edit: {e}");
            }
        });
    if spawned.is_err() {
        eprintln!("history: could not start the thread that keeps the text before an edit");
    }
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::Open => open_modal(app, cx),
    }
}

/// Palette row: 历史版本…
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![PaletteCommand {
        label: "历史版本…",
        icon: "clock",
        shortcut: "",
        command: AppCommand::Ext(ExtCommand::History(Command::Open)),
    }]
}

/// Status bar entry: 历史版本, left of the sync chip. Click opens the dialog.
pub fn statusbar_items(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let t = app.theme;
    let dim = matches!(app.ext.history.backend, Backend::Unavailable(_));
    let color = if dim { t.tx3 } else { t.tx2 };
    let chip = gpui::div()
        .flex()
        .items_center()
        .gap(gpui::px(6.0))
        .px(gpui::px(6.0))
        .h(gpui::px(20.0))
        .rounded(gpui::px(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .text_color(color)
        .font_family(crate::ui::UI_FONT)
        .child(crate::ui::icons::icon("clock", 13.0, color))
        .child(gpui::SharedString::from(crate::t!("status-history")))
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                open_modal(this, cx);
            }),
        );
    vec![chip.into_any_element()]
}

/// True while the dialog is open.
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.history.modal
}

/// The dialog, when open.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    if !app.ext.history.modal {
        return Vec::new();
    }
    vec![view::render(app, cx)]
}

/// Escape closes the dialog.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    if !app.ext.history.modal || !overlay::is_escape(ev) {
        return false;
    }
    close_modal(app);
    cx.notify();
    true
}
