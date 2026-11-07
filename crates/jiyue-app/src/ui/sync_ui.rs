//! Sync UI: S3 and WebDAV settings, the status chip, manual and automatic sync, conflict handling
//! and the remote version history. Owned by W2-Y.
//!
//! Layout: `config` (settings file and rules), `secrets` (credentials.toml), `service` (background runs),
//! `schedule` (pure timing and text), `conflicts` (conflict list and resolution), `settings` (the
//! 同步 settings page), `modal` (sync status dialog), `history` (remote versions).

pub(crate) mod config;
mod conflicts;
mod history;
mod modal;
mod schedule;
pub(crate) mod secrets;
mod service;
mod settings;

use std::time::Duration;

use crate::ui::ext::{ExtCommand, ExtEvent, PaletteCommand, ToastKind};
use crate::ui::history_ui;
use crate::ui::icons::icon;
use crate::ui::widgets::overlay;
use crate::{AppCommand, RustdianApp};
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, KeyDownEvent, MouseButton, SharedString, Task,
    div, prelude::*, px,
};

use config::SyncConfig;
use service::{Job, Progress, RunKind, SyncService};

pub use settings::render_settings_section;

/// Commands this module handles (sent as `ExtCommand::Sync`).
#[derive(Clone, Debug)]
pub enum Command {
    SyncNow,
    OpenSyncSettings,
    OpenVersionHistory,
}

/// Which dialog of this module is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Modal {
    #[default]
    None,
    /// Status, last report and conflicts.
    Sync,
    /// Remote versions of the active note.
    Versions,
}

/// A confirmation asked before an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Delete `.jiyue/sync/`.
    Reset,
    /// Mark the conflict at this index as resolved (its copy moves to the trash).
    Resolve(usize),
}

/// A message above the status bar. Errors stay until the next success or until dismissed.
#[derive(Clone, Debug)]
pub struct Banner {
    pub text: String,
    pub error: bool,
}

/// Module state.
#[derive(Default)]
pub struct State {
    /// When the window asked to close and an exit sync was started (see `on_close_requested`).
    pub exit_started: Option<std::time::Instant>,
    /// Settings as saved on disk. Runs always use this.
    pub saved: SyncConfig,
    /// Settings as edited in the form (toggles change this directly; text fields are read on save).
    pub draft: SyncConfig,
    /// Why the settings file could not be read (it is then replaced by defaults on the next save).
    pub config_error: Option<String>,
    /// A secret is stored for the saved settings (its value is never kept here).
    pub secret_stored: bool,
    /// How the stored secret is given (literal, `env:`, `cmd:`); never the value.
    pub secret_display: crate::ui::config_ui::SecretDisplay,
    /// Entities of the settings form, created once in `init`.
    pub inputs: Option<settings::Inputs>,
    /// Message under the settings buttons: (ok, text).
    pub settings_msg: Option<(bool, String)>,
    /// Result of the last connection test.
    pub test: settings::TestState,
    pub service: SyncService,
    /// Open conflicts, read from `.jiyue/sync/conflicts.json`.
    pub conflicts: Vec<conflicts::Item>,
    pub modal: Modal,
    pub confirm: Option<Confirm>,
    pub banner: Option<Banner>,
    /// Expanded report lists in the sync dialog (upload, download, merge, conflict, delete, error).
    pub expanded: [bool; 6],
    /// Finish time of the last run without errors, Unix milliseconds.
    pub last_ok_ms: Option<i64>,
    pub history: history::History,
    /// The progress poller is running.
    polling: bool,
    poll_task: Option<Task<()>>,
    ticker: Option<Task<()>>,
}

/// What the status chip shows.
enum Status {
    Off,
    Running(Progress),
    Failed,
    Conflicts(usize),
    Idle(Option<i64>),
}

fn status(s: &State) -> Status {
    if !s.saved.enabled {
        return Status::Off;
    }
    if s.service.running.is_some() {
        return Status::Running(s.service.progress.clone());
    }
    if s.service.last.as_ref().is_some_and(|o| o.error.is_some()) {
        return Status::Failed;
    }
    if !s.conflicts.is_empty() {
        return Status::Conflicts(s.conflicts.len());
    }
    Status::Idle(s.last_ok_ms)
}

/// New random vault id (the key of the vault's entry in `credentials.toml`).
fn new_vault_id() -> String {
    let seed = format!("{}-{}", jiyue_sync::state::now_ms(), std::process::id());
    jiyue_sync::state::hash_bytes(seed.as_bytes())[..16].to_owned()
}

/// Loads the settings, creates the form entities and starts the scheduler.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let root = app.vault.root_dir.clone();
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vault")
        .to_owned();
    let (loaded, error) = match SyncConfig::load(&root) {
        Ok(mut cfg) => {
            secrets::fill_identity(&mut cfg);
            (cfg, None)
        }
        Err(e) => (SyncConfig::default(), Some(e)),
    };
    let saved = loaded.with_defaults(&name, new_vault_id);
    let stored = secrets::exists(&saved);
    let display = secrets::display(&saved);
    let conflict_items = conflicts::load(&root);
    let theme = app.theme;
    {
        let s = &mut app.ext.sync;
        s.draft = saved.clone();
        s.saved = saved;
        s.config_error = error;
        s.secret_stored = stored;
        s.secret_display = display;
        s.conflicts = conflict_items;
    }
    let inputs = settings::Inputs::new(theme, cx);
    app.ext.sync.inputs = Some(inputs);
    settings::fill_inputs(app, cx);

    let ticker = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if this.update(cx, scheduler_tick).is_err() {
                break;
            }
        }
    });
    app.ext.sync.ticker = Some(ticker);
}

/// Once a second: starts the automatic and save-triggered runs when they are due.
fn scheduler_tick(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let decision = {
        let s = &app.ext.sync;
        let cfg = &s.saved;
        if !cfg.enabled || s.service.running.is_some() || s.service.backing_off() {
            None
        } else if cfg.auto_sync && schedule::auto_due(s.service.last_end.elapsed(), cfg.interval) {
            Some(RunKind::Auto)
        } else if cfg.sync_on_save
            && s.service.save_pending
            && s.service
                .last_save
                .is_some_and(|t| schedule::save_due(t.elapsed(), cfg.save_debounce_secs))
        {
            Some(RunKind::AfterSave)
        } else {
            None
        }
    };
    if let Some(kind) = decision {
        app.ext.sync.service.save_pending = false;
        start_run(app, kind, cx);
    }
}

/// Starts a sync run. Saves dirty notes first so the upload has their latest text. Returns false
/// when nothing started (the reason is toasted for manual runs).
pub fn start_run(app: &mut RustdianApp, kind: RunKind, cx: &mut Context<RustdianApp>) -> bool {
    let manual = kind.is_manual();
    let cfg = app.ext.sync.saved.clone();
    if !cfg.enabled {
        if manual {
            app.toast(ToastKind::Warn, "同步未启用，请先在设置中开启");
        }
        return false;
    }
    if app.ext.sync.service.running.is_some() {
        if manual {
            app.toast(ToastKind::Info, "同步正在进行中");
        }
        return false;
    }
    if config::test_override().is_none()
        && let Err(e) = cfg.validate(secrets::exists(&cfg))
    {
        if manual {
            app.toast(ToastKind::Error, format!("同步失败：{e}"));
        }
        return false;
    }
    // Dirty notes reach the disk before the run reads it. Those saves are covered by this run.
    app.save_all_dirty(cx);
    app.ext.sync.service.save_pending = false;

    let job = Job {
        root: app.vault.root_dir.clone(),
        cfg,
        kind,
        history: history_ui::store_of(app),
    };
    if !app.ext.sync.service.start(job) {
        return false;
    }
    app.ext.sync.banner = None;
    ensure_poll(app, cx);
    cx.notify();
    true
}

/// The longest the window waits for the exit sync before it closes anyway.
pub const EXIT_SYNC_LIMIT: Duration = Duration::from_secs(8);

/// The window asked to close. Saves dirty notes first. When sync is on and `sync_on_exit` is set,
/// one run starts and the window stays open until it ends or `EXIT_SYNC_LIMIT` passes; then the app
/// quits. A second close request while that run goes cancels it and closes at once. Returns true
/// when the window may close now.
pub fn on_close_requested(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) -> bool {
    if app.force_close {
        return true;
    }
    // Every dirty note is saved first (all open tabs). A failed save blocks the close until the
    // user chooses to close anyway; the failed text is kept in its recovery copy.
    let failed = app.prepare_exit(cx);
    if !failed.is_empty() {
        app.toast(
            ToastKind::Error,
            format!("{} 个笔记未能保存，内容已保留在恢复文件中", failed.len()),
        );
        crate::ui::files_ui::confirm_close_despite_failures(app, failed.len());
        cx.notify();
        return false;
    }
    if app.ext.sync.exit_started.is_some() {
        app.ext.sync.service.request_cancel();
        app.ext.sync.exit_started = None;
        return true;
    }
    let wanted = app.ext.sync.saved.enabled && app.ext.sync.saved.sync_on_exit;
    if !wanted {
        return true;
    }
    if app.ext.sync.service.running.is_none() && !start_run(app, RunKind::Exit, cx) {
        return true;
    }
    app.ext.sync.exit_started = Some(std::time::Instant::now());
    app.toast(ToastKind::Info, "正在同步，完成后退出（再次关闭将取消）");
    cx.notify();
    let waiter = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let keep_waiting = this.update(cx, |app, cx| {
                let running = poll_tick(app, cx);
                let timed_out = app
                    .ext
                    .sync
                    .exit_started
                    .is_some_and(|t| t.elapsed() >= EXIT_SYNC_LIMIT);
                if running && !timed_out {
                    return true;
                }
                if running {
                    app.ext.sync.service.request_cancel();
                }
                app.ext.sync.exit_started = None;
                // Edits made while the sync ran are saved before the app quits.
                app.prepare_exit(cx);
                cx.quit();
                false
            });
            if !matches!(keep_waiting, Ok(true)) {
                break;
            }
        }
    });
    waiter.detach();
    false
}

/// Starts the 200 ms progress poller unless it is already running.
fn ensure_poll(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.ext.sync.polling {
        return;
    }
    app.ext.sync.polling = true;
    let task = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let keep = this.update(cx, |app, cx| {
                let running = poll_tick(app, cx);
                if !running {
                    app.ext.sync.polling = false;
                }
                running
            });
            if !matches!(keep, Ok(true)) {
                break;
            }
        }
    });
    app.ext.sync.poll_task = Some(task);
}

/// Copies progress and applies a finished run. Returns true while a run is going.
fn poll_tick(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) -> bool {
    if let Some(outcome) = app.ext.sync.service.poll() {
        finish_run(app, outcome, cx);
    }
    let running = app.ext.sync.service.running.is_some();
    cx.notify();
    running
}

/// Applies the result of a run: banner, toasts, conflict list and changed files.
fn finish_run(app: &mut RustdianApp, outcome: service::Outcome, cx: &mut Context<RustdianApp>) {
    let root = app.vault.root_dir.clone();
    let manual = outcome.kind.is_manual();
    let report = outcome.report.clone();
    app.ext.sync.conflicts = conflicts::load(&root);

    let mut toast: Option<(ToastKind, String)> = None;
    let mut banner: Option<Banner> = None;
    match &outcome.error {
        Some(err) => {
            let text = format!("同步失败：{err}");
            if manual {
                toast = Some((ToastKind::Error, text.clone()));
            }
            banner = Some(Banner { text, error: true });
        }
        None if report.cancelled => {
            if manual {
                toast = Some((ToastKind::Info, "同步已取消".into()));
            }
        }
        None if report.errors.is_empty() => {
            app.ext.sync.last_ok_ms = Some(outcome.finished_ms);
            if manual {
                toast = Some((
                    ToastKind::Success,
                    format!("同步完成 · {}", schedule::report_summary(&report)),
                ));
            }
        }
        None => {
            let first = report
                .errors
                .first()
                .map(|e| format!("{}：{}", e.path, e.message))
                .unwrap_or_default();
            let text = format!("同步完成，{} 个文件失败：{first}", report.errors.len());
            if manual {
                toast = Some((ToastKind::Warn, text.clone()));
            }
            banner = Some(Banner { text, error: true });
        }
    }
    app.ext.sync.banner = banner;
    if let Some((kind, text)) = toast {
        app.toast(kind, text);
    }
    let changed = schedule::changed_paths(&report);
    if !changed.is_empty() {
        crate::ui::ext::on_event(app, &ExtEvent::VaultFilesChanged(changed), cx);
    }
    cx.notify();
}

/// Opens the sync dialog.
pub fn open_modal(app: &mut RustdianApp, which: Modal) {
    app.ext.sync.modal = which;
}

/// Closes the dialog (and any confirmation).
pub fn close_modal(app: &mut RustdianApp) {
    app.ext.sync.modal = Modal::None;
    app.ext.sync.confirm = None;
}

/// Deletes `.jiyue/sync/` (state, base cache, conflict list, device id) after confirmation.
pub fn reset_state(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.ext.sync.service.running.is_some() {
        app.toast(ToastKind::Info, "同步正在进行中，稍后再重置");
        return;
    }
    let dir = jiyue_sync::state::sync_dir(&app.vault.root_dir);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            app.toast(ToastKind::Error, format!("重置失败：{e}"));
            cx.notify();
            return;
        }
    }
    let s = &mut app.ext.sync;
    s.conflicts.clear();
    s.service.last = None;
    s.service.failures = 0;
    s.service.next_auto_at = None;
    s.last_ok_ms = None;
    s.banner = None;
    app.toast(ToastKind::Success, "已重置同步状态");
    cx.notify();
}

/// Keyboard hook: Escape closes the confirmation or the dialog.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    if !overlay::is_escape(ev) {
        return false;
    }
    if app.ext.sync.confirm.is_some() {
        app.ext.sync.confirm = None;
        cx.notify();
        return true;
    }
    if app.ext.sync.modal != Modal::None {
        close_modal(app);
        cx.notify();
        return true;
    }
    false
}

/// Notes saved while sync-on-save is on start the debounce.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    match ev {
        ExtEvent::Started => {
            let cfg = &app.ext.sync.saved;
            if cfg.enabled && cfg.sync_on_start {
                start_run(app, RunKind::Start, cx);
            }
        }
        ExtEvent::NoteSaved(_) => {
            if app.ext.sync.saved.sync_on_save {
                app.ext.sync.service.note_saved();
            }
        }
        ExtEvent::ThemeChanged => settings::set_theme(app, cx),
        _ => {}
    }
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::SyncNow => {
            open_modal(app, Modal::Sync);
            start_run(app, RunKind::Manual, cx);
        }
        Command::OpenSyncSettings => {
            app.run_command(AppCommand::OpenSettings);
            app.settings.category = 3;
        }
        Command::OpenVersionHistory => history::open(app, cx),
    }
    cx.notify();
}

/// Palette rows: 立即同步, 同步设置, 查看远端历史版本.
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "立即同步",
            icon: "arrows-sort",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Sync(Command::SyncNow)),
        },
        PaletteCommand {
            label: "同步设置",
            icon: "settings",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Sync(Command::OpenSyncSettings)),
        },
        PaletteCommand {
            label: "查看远端历史版本",
            icon: "clock",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Sync(Command::OpenVersionHistory)),
        },
    ]
}

/// The status chip, left of the save indicator. Click opens the sync dialog.
pub fn statusbar_items(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let t = app.theme;
    let s = &app.ext.sync;
    let (text, dot, color) = match status(s) {
        Status::Off => (crate::t!("status-sync-off"), t.tx3, t.tx3),
        Status::Running(p) => (schedule::progress_text(p.done, p.total), t.ac, t.tx2),
        Status::Failed => (crate::t!("status-sync-failed"), t.st, t.st),
        Status::Conflicts(n) => (crate::t!("status-sync-conflicts", count = n), t.ac, t.ac),
        Status::Idle(None) => (crate::t!("status-sync-never"), t.tx3, t.tx3),
        Status::Idle(Some(ms)) => {
            let rel = schedule::relative_zh(jiyue_sync::state::now_ms(), ms);
            (crate::t!("status-sync-synced", time = rel.as_str()), t.ok, t.tx3)
        }
    };
    let chip = div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(6.0))
        .h(px(20.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .text_color(color)
        .font_family(crate::ui::fonts::ui_name())
        .child(div().size(px(6.0)).rounded_full().bg(dot))
        .child(SharedString::from(text))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                open_modal(this, Modal::Sync);
                cx.notify();
            }),
        );
    vec![chip.into_any_element()]
}

/// True while a dialog or confirmation of this module is open.
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.sync.modal != Modal::None || app.ext.sync.confirm.is_some()
}

/// Dialogs, confirmations and the banner.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    match app.ext.sync.modal {
        Modal::None => {}
        Modal::Sync => out.push(modal::render(app, cx)),
        Modal::Versions => out.push(history::render(app, cx)),
    }
    if let Some(confirm) = app.ext.sync.confirm {
        out.push(modal::render_confirm(app, confirm, cx));
    }
    if let Some(banner) = &app.ext.sync.banner {
        out.push(render_banner(app, banner, cx));
    }
    out
}

/// Error banner, bottom-left above the status bar. 重试 runs a manual sync.
fn render_banner(app: &RustdianApp, banner: &Banner, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let retry = crate::ui::widgets::button(
        &t,
        "重试",
        crate::ui::widgets::ButtonKind::Secondary,
        None,
        cx.listener(|this, _, _, cx| {
            start_run(this, RunKind::Manual, cx);
        }),
    );
    let close = crate::ui::widgets::icon_button(
        &t,
        "x",
        cx.listener(|this, _, _, cx| {
            this.ext.sync.banner = None;
            cx.notify();
        }),
    );
    div()
        .absolute()
        .left(px(16.0))
        .bottom(px(40.0))
        .max_w(px(420.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(14.0))
        .py(px(10.0))
        .rounded(px(10.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .shadow_md()
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .text_color(t.tx)
        .child(icon("x", 16.0, t.st))
        .child(
            div()
                .min_w_0()
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(banner.text.clone())),
        )
        .child(retry)
        .child(close)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_ids_are_short_hex() {
        let id = new_vault_id();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
