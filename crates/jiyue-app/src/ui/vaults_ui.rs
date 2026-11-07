//! Vault management screens (see `vaults.rs` for the rules and the recent list):
//! - the 笔记库 settings page (`render_library_section`),
//! - the sidebar workspace menu (`open_menu`, drawn by `overlays`),
//! - the open and create flow: a native folder picker, checks in the background, a confirmation
//!   when needed, then the switch,
//! - the switch itself: save everything, finish the exit sync (8 s cap), then start a new process
//!   for the other vault and quit (see `vaults::request_relaunch`),
//! - the first-run chooser window, shown when no vault is known (`VaultChooser`).
//!
//! Test hook: `JIYUE_TEST_PICK_VAULT=<path>` makes every folder picker return that path without a
//! dialog. It is read only when set, and only by this module.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AnyElement, App, ClipboardItem, Context, Div, Entity, Focusable, FontWeight, IntoElement,
    KeyDownEvent, MouseDownEvent, PathPromptOptions, Render, SharedString, Window, div, prelude::*,
    px,
};

use crate::AppCommand;
use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::ext::{ExtCommand, PaletteCommand, ToastKind, reveal_in_explorer};
use crate::ui::icons::icon;
use crate::ui::settings::{CAT_LIBRARY, setting_row};
use crate::ui::widgets::controls::PickIndex;
use crate::ui::widgets::{
    BadgeKind, ButtonKind, LineInput, LineInputEvent, MenuItem, badge, button, confirm_dialog,
    context_menu, is_escape, modal, section_title, tooltip,
};
use crate::vaults::{self, Action, Facts, Mode, RecentRow, RecentStore};
use jiyue_core::platform;

/// Commands of this module (sent as `ExtCommand::Vaults`, or from the menus and buttons).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// 打开其他笔记库…: native folder picker, then the checks and the switch.
    OpenPicker,
    /// 新建笔记库…: native save dialog for a new folder name, then the checks and the switch.
    NewPicker,
    /// 打开 a vault from the recent list.
    OpenRecent(String),
    /// 移除 a vault from the recent list (its files are not touched).
    RemoveRecent(String),
    /// 在资源管理器中显示 the vault folder.
    Reveal,
    /// 复制路径 of the vault folder.
    CopyPath,
    /// 管理笔记库…: opens the 笔记库 settings page.
    Manage,
    /// 重命名笔记库…: the display name (stored in the vault's config).
    RenameName,
    /// 重命名文件夹…: renames the vault folder on disk and restarts into it.
    RenameFolder,
}

/// The rename field of the display name or of the folder name, while its dialog is open.
pub struct RenameDialog {
    /// `true` for the folder name, `false` for the display name.
    pub folder: bool,
    pub input: Entity<LineInput>,
    /// The reason the typed name was refused.
    pub error: Option<String>,
    /// Keeps the event subscription alive.
    pub _sub: gpui::Subscription,
}

/// A folder rename that waits for the user's confirmation.
pub struct PendingFolder {
    pub new_name: String,
    pub target: PathBuf,
}

/// A switch waiting for the user's answer to a warning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSwitch {
    pub path: String,
    pub title: String,
    pub message: String,
    pub confirm: String,
}

/// The sidebar workspace menu while it is open.
pub struct VaultMenu {
    /// Where it opened (window px) and the window size then.
    pub pos: (f32, f32),
    pub win: (f32, f32),
    pub items: Vec<MenuItem>,
    /// The command of each row (`None` for separators and rows that cannot be picked).
    pub acts: Vec<Option<Command>>,
}

/// Module state, one field in `ExtState`.
#[derive(Default)]
pub struct State {
    pub menu: Option<VaultMenu>,
    pub confirm: Option<PendingSwitch>,
    /// The rename dialog of the display name or the folder name, while it is open.
    pub rename: Option<RenameDialog>,
    /// A folder rename waiting for the user's confirmation.
    pub folder_go: Option<PendingFolder>,
}

/// Palette rows of this module.
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "打开其他笔记库…",
            icon: "folder-open",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Vaults(Command::OpenPicker)),
        },
        PaletteCommand {
            label: "新建笔记库…",
            icon: "folder-plus",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Vaults(Command::NewPicker)),
        },
        PaletteCommand {
            label: "管理笔记库…",
            icon: "folder",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Vaults(Command::Manage)),
        },
        PaletteCommand {
            label: "重命名笔记库…",
            icon: "pencil",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Vaults(Command::RenameName)),
        },
    ]
}

/// True while the sidebar menu or a confirmation is open.
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.vaults.menu.is_some()
        || app.ext.vaults.confirm.is_some()
        || app.ext.vaults.rename.is_some()
        || app.ext.vaults.folder_go.is_some()
}

/// Escape closes the sidebar menu, the rename dialog, or cancels a confirmation.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    if !is_escape(ev) {
        return false;
    }
    let closed = app.ext.vaults.menu.take().is_some()
        | app.ext.vaults.confirm.take().is_some()
        | app.ext.vaults.rename.take().is_some()
        | app.ext.vaults.folder_go.take().is_some();
    if closed {
        cx.notify();
        return true;
    }
    false
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::OpenPicker => pick(cx, false, |app, path, cx| {
            begin_candidate(app, path, Mode::Open, cx)
        }),
        Command::NewPicker => pick(cx, true, |app, path, cx| {
            begin_candidate(app, path, Mode::Create, cx)
        }),
        Command::OpenRecent(path) => begin_candidate(app, PathBuf::from(path), Mode::Open, cx),
        Command::RemoveRecent(path) => {
            match vaults::update_store(|s| s.remove(&path)) {
                Ok(()) => app.toast(ToastKind::Info, "已从列表移除，文件没有被删除"),
                Err(err) => app.toast(ToastKind::Error, format!("无法更新最近笔记库：{err}")),
            }
            cx.notify();
        }
        Command::Reveal => {
            reveal_in_explorer(&app.vault.root_dir);
        }
        Command::CopyPath => {
            let text = app.vault.root_dir.display().to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            app.toast(ToastKind::Success, "已复制笔记库路径");
            cx.notify();
        }
        Command::Manage => {
            app.settings.open = true;
            app.settings.category = CAT_LIBRARY;
            app.settings.scroll.scroll_to_item(0);
            cx.notify();
        }
        Command::RenameName => open_rename(app, false, cx),
        Command::RenameFolder => open_rename(app, true, cx),
    }
}

// ---------------------------------------------------------------------------
// Renaming the vault: the display name, and the folder on disk
// ---------------------------------------------------------------------------

/// Opens the rename dialog with the current name typed in: the display name, or the folder name.
fn open_rename(app: &mut RustdianApp, folder: bool, cx: &mut Context<RustdianApp>) {
    let current = if folder {
        app.vault
            .root_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string()
    } else {
        app.configs.vault_title()
    };
    let theme = app.theme;
    let placeholder = if folder {
        "文件夹名"
    } else {
        "笔记库名称"
    };
    let input = cx.new(|cx| LineInput::new(cx, placeholder));
    input.update(cx, |field, cx| {
        field.set_theme(theme, cx);
        field.set_text(&current, cx);
    });
    let handle = input.read(cx).focus_handle(cx);
    crate::ui::search_ui::focus_soon(cx, handle);
    let sub = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            on_rename_event(this, ev, cx);
        },
    );
    app.ext.vaults.rename = Some(RenameDialog {
        folder,
        input,
        error: None,
        _sub: sub,
    });
    cx.notify();
}

fn on_rename_event(app: &mut RustdianApp, ev: &LineInputEvent, cx: &mut Context<RustdianApp>) {
    match ev {
        LineInputEvent::Changed(_) => {}
        LineInputEvent::Cancel => app.ext.vaults.rename = None,
        LineInputEvent::Submit(raw) => submit_rename(app, raw, cx),
    }
    cx.notify();
}

/// Checks the typed name. A display name is saved at once. A folder name is checked against the
/// disk and then asks for the confirmation that restarts the app into the new folder.
fn submit_rename(app: &mut RustdianApp, raw: &str, cx: &mut Context<RustdianApp>) {
    let Some(folder) = app.ext.vaults.rename.as_ref().map(|d| d.folder) else {
        return;
    };
    if folder {
        match jiyue_core::config::vault_meta::folder_rename_target(&app.vault.root_dir, raw) {
            Ok(target) => {
                app.ext.vaults.rename = None;
                app.ext.vaults.folder_go = Some(PendingFolder {
                    new_name: raw.trim().to_string(),
                    target,
                });
            }
            Err(message) => set_rename_error(app, message),
        }
    } else {
        let root = app.vault.root_dir.clone();
        match jiyue_core::config::vault_meta::write_name(&root, raw) {
            Ok(()) => {
                app.configs.set_vault_display(raw);
                app.ext.vaults.rename = None;
                app.toast(ToastKind::Success, "笔记库名称已更新");
            }
            Err(message) => set_rename_error(app, message),
        }
    }
    cx.notify();
}

fn set_rename_error(app: &mut RustdianApp, message: String) {
    if let Some(dialog) = app.ext.vaults.rename.as_mut() {
        dialog.error = Some(message);
    }
}

/// Renames the vault folder, then restarts into it: the same hand-over as a vault switch (save,
/// exit sync, a new process for the new path). Nothing is renamed when a save fails.
pub fn apply_folder_rename(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(pending) = app.ext.vaults.folder_go.take() else {
        return;
    };
    let old = app.vault.root_dir.clone();
    let failed = app.prepare_exit(cx);
    if !failed.is_empty() {
        app.toast(
            ToastKind::Error,
            format!("{} 个笔记未能保存，暂不重命名文件夹", failed.len()),
        );
        cx.notify();
        return;
    }
    if let Err(err) = std::fs::rename(&old, &pending.target) {
        app.toast(
            ToastKind::Error,
            format!("无法重命名文件夹（可能正被其他程序占用）：{err}"),
        );
        cx.notify();
        return;
    }
    let new = pending.target.display().to_string();
    if let Err(err) = vaults::update_store(|s| s.remove(&old.display().to_string())) {
        app.toast(ToastKind::Warn, format!("无法更新最近笔记库：{err}"));
    }
    if let Err(err) = vaults::record_opened(&new) {
        app.toast(ToastKind::Warn, format!("无法记录最近笔记库：{err}"));
    }
    vaults::request_relaunch(&new, app.theme.kind);
    if crate::ui::sync_ui::on_close_requested(app, cx) {
        cx.quit();
    }
    cx.notify();
}

// ---------------------------------------------------------------------------
// Picker and checks (shared by the window and the chooser)
// ---------------------------------------------------------------------------

/// Asks for a folder: an open dialog, or a save dialog for a new name when `create`. Calls
/// `on_picked` with the chosen path. The test hook returns `JIYUE_TEST_PICK_VAULT` instead.
fn pick<V: 'static>(
    cx: &mut Context<V>,
    create: bool,
    on_picked: impl FnOnce(&mut V, PathBuf, &mut Context<V>) + 'static,
) {
    if let Some(path) = std::env::var_os(vaults::TEST_PICK_ENV).filter(|v| !v.is_empty()) {
        let path = PathBuf::from(path);
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |v, cx| on_picked(v, path, cx));
        })
        .detach();
        return;
    }
    if create {
        let rx = cx.prompt_for_new_path(&platform::documents_dir(), Some("新笔记库"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let _ = this.update(cx, |v, cx| on_picked(v, path, cx));
            }
        })
        .detach();
    } else {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(SharedString::from("选择笔记库")),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |v, cx| on_picked(v, path, cx));
            }
        })
        .detach();
    }
}

/// Looks at the disk for `path` on the background executor, then calls `done` on the entity.
/// A new vault's folder is created first. The result is the facts, or the reason it failed.
fn validate_then<V: 'static>(
    cx: &mut Context<V>,
    path: PathBuf,
    mode: Mode,
    done: impl FnOnce(&mut V, String, Result<Facts, String>, &mut Context<V>) + 'static,
) {
    let target = path.display().to_string();
    let work = path.clone();
    let task = cx
        .background_executor()
        .spawn(async move { probe(&work, mode) });
    cx.spawn(async move |this, cx| {
        let facts = task.await;
        let _ = this.update(cx, |v, cx| done(v, target, facts, cx));
    })
    .detach();
}

fn probe(path: &Path, mode: Mode) -> Result<Facts, String> {
    if mode == Mode::Create && !path.exists() {
        vaults::prepare_folder(path)?;
    }
    Ok(vaults::inspect(path))
}

/// The decision for a candidate, with the system folder looked up here.
fn verdict(mode: Mode, target: &str, current: Option<&str>, facts: &Facts) -> Action {
    let home = vaults::home_string();
    let docs = vaults::documents_string();
    let system = vaults::system_folder(target, home.as_deref(), Some(&docs));
    vaults::decide(mode, target, current, facts, system)
}

/// The window's start of the flow: the same folder is answered at once, the rest is checked.
fn begin_candidate(
    app: &mut RustdianApp,
    path: PathBuf,
    mode: Mode,
    cx: &mut Context<RustdianApp>,
) {
    let current = app.vault.root_dir.display().to_string();
    if vaults::same_path(&current, &path.display().to_string()) {
        app.toast(ToastKind::Info, "已经是当前笔记库");
        cx.notify();
        return;
    }
    validate_then(cx, path, mode, move |app, target, facts, cx| {
        finish_candidate(app, target, mode, facts, cx)
    });
}

fn finish_candidate(
    app: &mut RustdianApp,
    target: String,
    mode: Mode,
    facts: Result<Facts, String>,
    cx: &mut Context<RustdianApp>,
) {
    match facts {
        Err(err) => app.toast(ToastKind::Error, err),
        Ok(facts) => {
            let current = app.vault.root_dir.display().to_string();
            match verdict(mode, &target, Some(&current), &facts) {
                Action::Switch => switch_now(app, target, cx),
                Action::AlreadyCurrent => app.toast(ToastKind::Info, "已经是当前笔记库"),
                Action::Error(message) => app.toast(ToastKind::Error, message),
                Action::Confirm {
                    title,
                    message,
                    confirm,
                } => {
                    app.ext.vaults.confirm = Some(PendingSwitch {
                        path: target,
                        title,
                        message,
                        confirm,
                    });
                }
            }
        }
    }
    cx.notify();
}

/// Switches this window to the vault at `target`. Everything is saved first, as when the window
/// closes; a failed save blocks the switch. The exit sync runs (8 s cap), then the app quits and
/// `main` starts a new process for `target` after the event loop has ended.
pub fn switch_now(app: &mut RustdianApp, target: String, cx: &mut Context<RustdianApp>) {
    // A vault folder that vanished cannot be saved to; its unsaved text is in the recovery copies.
    if app.vault.root_dir.is_dir() {
        let failed = app.prepare_exit(cx);
        if !failed.is_empty() {
            app.toast(
                ToastKind::Error,
                format!("{} 个笔记未能保存，暂不切换笔记库", failed.len()),
            );
            cx.notify();
            return;
        }
    }
    if let Err(err) = vaults::record_opened(&target) {
        app.toast(ToastKind::Warn, format!("无法记录最近笔记库：{err}"));
    }
    vaults::request_relaunch(&target, app.theme.kind);
    if crate::ui::sync_ui::on_close_requested(app, cx) {
        cx.quit();
    }
    cx.notify();
}

// ---------------------------------------------------------------------------
// Settings page
// ---------------------------------------------------------------------------

/// Button that runs a command of this module on the window's state.
fn app_button(
    me: &Entity<RustdianApp>,
    t: &ActiveTheme,
    label: &str,
    kind: ButtonKind,
    icon_name: &str,
    act: impl Fn(&mut RustdianApp, &mut Context<RustdianApp>) + 'static,
) -> Div {
    let me = me.clone();
    button(t, label, kind, Some(icon_name), move |_, _, cx| {
        me.update(cx, |app, cx| act(app, cx));
    })
}

/// The 笔记库 page of the settings dialog.
pub fn render_library_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let me = cx.entity();
    let root = app.vault.root_dir.display().to_string();
    let store = vaults::load_store(&vaults::store_path()).0;
    let rows = vaults::recent_rows(&store, Some(&root), |p| Path::new(p).is_dir());

    let path_block = div()
        .id("vaults-root-path")
        .max_w(px(380.0))
        .overflow_hidden()
        .text_size(px(12.5))
        .text_color(t.tx2)
        .tooltip(tooltip(t, root.clone()))
        .child(SharedString::from(vaults::middle_truncate(&root, 46)));
    let count = div()
        .text_size(px(13.0))
        .text_color(t.tx)
        .child(SharedString::from(app.vault.notes.len().to_string()));
    let tags = div()
        .text_size(px(13.0))
        .text_color(t.tx)
        .child(SharedString::from(app.vault.all_tags().len().to_string()));

    let actions = div()
        .flex()
        .flex_wrap()
        .gap(px(8.0))
        .py(px(12.0))
        .child(app_button(
            &me,
            &t,
            &crate::t!("settings-vault-reveal"),
            ButtonKind::Secondary,
            "folder-open",
            |app, _| {
                reveal_in_explorer(&app.vault.root_dir);
            },
        ))
        .child(app_button(
            &me,
            &t,
            &crate::t!("settings-vault-copy-path"),
            ButtonKind::Secondary,
            "copy",
            |app, cx| {
                run(app, Command::CopyPath, cx);
            },
        ))
        .child(app_button(
            &me,
            &t,
            &crate::t!("settings-vault-open-other"),
            ButtonKind::Secondary,
            "folder-open",
            |app, cx| {
                run(app, Command::OpenPicker, cx);
            },
        ))
        .child(app_button(
            &me,
            &t,
            &crate::t!("settings-vault-new"),
            ButtonKind::Secondary,
            "folder-plus",
            |app, cx| {
                run(app, Command::NewPicker, cx);
            },
        ))
        .child(app_button(
            &me,
            &t,
            &crate::t!("settings-vault-rename-folder"),
            ButtonKind::Secondary,
            "folder",
            |app, cx| {
                run(app, Command::RenameFolder, cx);
            },
        ));

    let mut list = div().flex().flex_col();
    if rows.is_empty() {
        list = list.child(
            div()
                .py(px(10.0))
                .text_size(px(12.5))
                .text_color(t.tx3)
                .child(SharedString::from(crate::t!("settings-vault-recent-empty"))),
        );
    }
    for (i, row) in rows.iter().enumerate() {
        list = list.child(recent_row(&me, &t, i, row));
    }

    div()
        .flex()
        .flex_col()
        .mt(px(10.0))
        .child(setting_row(
            &t,
            &crate::t!("settings-vault-name"),
            &crate::t!("settings-vault-name-desc"),
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .max_w(px(200.0))
                        .overflow_hidden()
                        .text_size(px(13.0))
                        .text_color(t.tx)
                        .child(SharedString::from(app.configs.vault_title())),
                )
                .child(app_button(
                    &me,
                    &t,
                    &crate::t!("settings-vault-rename"),
                    ButtonKind::Secondary,
                    "pencil",
                    |app, cx| {
                        run(app, Command::RenameName, cx);
                    },
                )),
            false,
        ))
        .child(setting_row(
            &t,
            &crate::t!("settings-vault-path"),
            &crate::t!("settings-vault-path-desc"),
            path_block,
            false,
        ))
        .child(setting_row(
            &t,
            &crate::t!("settings-vault-note-count"),
            &crate::t!("settings-vault-note-count-desc"),
            count,
            false,
        ))
        .child(setting_row(
            &t,
            &crate::t!("settings-vault-tag-count"),
            &crate::t!("settings-vault-tag-count-desc"),
            tags,
            true,
        ))
        .child(actions)
        .child(section_title(&t, &crate::t!("settings-vault-recent-title")))
        .child(list)
        .into_any_element()
}

/// One row of the recent list: name and path, the state, and 打开 / 移除.
fn recent_row(
    me: &Entity<RustdianApp>,
    t: &ActiveTheme,
    index: usize,
    row: &RecentRow,
) -> gpui::Stateful<Div> {
    let path = row.path.clone();
    let mut name_line = div().flex().items_center().gap(px(8.0)).child(
        div()
            .text_size(px(13.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.tx)
            .child(SharedString::from(row.name.clone())),
    );
    if row.current {
        name_line = name_line.child(badge(t, &crate::t!("settings-vault-badge-current"), BadgeKind::Accent));
    }
    if row.missing {
        name_line = name_line.child(badge(t, &crate::t!("settings-vault-badge-missing"), BadgeKind::Warn));
    }
    let left = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .min_w_0()
        .flex_1()
        .child(name_line)
        .child(
            div()
                .id(("vaults-recent-path", index))
                .overflow_hidden()
                .text_size(px(12.0))
                .text_color(t.tx3)
                .tooltip(tooltip(*t, path.clone()))
                .child(SharedString::from(vaults::middle_truncate(&row.path, 60))),
        );
    let mut actions = div().flex().items_center().gap(px(6.0)).flex_none();
    if !row.current {
        let open_path = path.clone();
        actions = actions.child(app_button(
            me,
            t,
            &crate::t!("settings-vault-open"),
            ButtonKind::Secondary,
            "folder-open",
            move |app, cx| {
                run(app, Command::OpenRecent(open_path.clone()), cx);
            },
        ));
    }
    let remove_path = path.clone();
    actions = actions.child(app_button(
        me,
        t,
        &crate::t!("settings-vault-remove"),
        ButtonKind::Ghost,
        "x",
        move |app, cx| {
            run(app, Command::RemoveRecent(remove_path.clone()), cx);
        },
    ));
    let mut item = div()
        .id(("vaults-recent", index))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(left)
        .child(actions);
    if row.missing {
        item = item.opacity(0.5);
    }
    item
}

// ---------------------------------------------------------------------------
// Sidebar workspace menu
// ---------------------------------------------------------------------------

/// Opens the workspace menu under the chevron at window point `pos`: the recent vaults (the one
/// open is checked), then open, new and manage.
pub fn open_menu(app: &mut RustdianApp, pos: (f32, f32), window: &Window) {
    let size = window.viewport_size();
    let win = (f32::from(size.width), f32::from(size.height));
    let current = app.vault.root_dir.display().to_string();
    let store = vaults::load_store(&vaults::store_path()).0;
    let mut items = Vec::new();
    let mut acts = Vec::new();
    for row in vaults::recent_rows(&store, Some(&current), |p| Path::new(p).is_dir()) {
        // The menu is narrow: a missing folder gets a short mark here, the full text is on the page.
        let mut item = MenuItem::new(if row.missing {
            format!("{}（不存在）", row.name)
        } else {
            row.name.clone()
        });
        if row.current {
            item = item.icon("check");
        }
        let act = if row.current || row.missing {
            if row.missing {
                item = item.disabled();
            }
            None
        } else {
            Some(Command::OpenRecent(row.path.clone()))
        };
        items.push(item);
        acts.push(act);
    }
    if !items.is_empty() {
        items.push(MenuItem::separator());
        acts.push(None);
    }
    items.push(MenuItem::new("重命名笔记库…").icon("pencil"));
    acts.push(Some(Command::RenameName));
    items.push(MenuItem::new("打开其他笔记库…").icon("folder-open"));
    acts.push(Some(Command::OpenPicker));
    items.push(MenuItem::new("新建笔记库…").icon("folder-plus"));
    acts.push(Some(Command::NewPicker));
    items.push(MenuItem::new("管理笔记库…").icon("folder"));
    acts.push(Some(Command::Manage));
    app.ext.vaults.menu = Some(VaultMenu {
        pos,
        win,
        items,
        acts,
    });
}

/// The dialog that renames the display name or the folder name.
fn rename_modal(
    app: &RustdianApp,
    dialog: &RenameDialog,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let (title, hint) = if dialog.folder {
        ("重命名文件夹", "文件夹会改名，霁月随后重新打开它。")
    } else {
        (
            "重命名笔记库",
            "显示在侧栏、窗口标题和最近列表中，不会改动文件夹。",
        )
    };
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(div().text_size(px(12.5)).text_color(t.tx3).child(hint))
        .child(dialog.input.clone());
    if let Some(message) = dialog.error.as_ref() {
        body = body.child(
            div()
                .text_size(px(11.5))
                .text_color(gpui::rgb(0xE5484D))
                .child(SharedString::from(message.clone())),
        );
    }
    let footer = vec![
        button(
            &t,
            "取消",
            ButtonKind::Ghost,
            None,
            cx.listener(|this: &mut RustdianApp, _: &MouseDownEvent, _, cx| {
                this.ext.vaults.rename = None;
                cx.notify();
            }),
        )
        .into_any_element(),
        button(
            &t,
            "保存",
            ButtonKind::Primary,
            None,
            cx.listener(|this: &mut RustdianApp, _: &MouseDownEvent, _, cx| {
                let raw = this
                    .ext
                    .vaults
                    .rename
                    .as_ref()
                    .map(|d| d.input.read(cx).text())
                    .unwrap_or_default();
                submit_rename(this, &raw, cx);
            }),
        )
        .into_any_element(),
    ];
    modal(
        &t,
        title,
        body,
        footer,
        420.0,
        cx.listener(|this: &mut RustdianApp, _: &MouseDownEvent, _, cx| {
            this.ext.vaults.rename = None;
            cx.notify();
        }),
    )
}

/// The confirmation before a folder rename: the app saves, renames the folder and restarts.
fn folder_confirm(
    app: &RustdianApp,
    pending: &PendingFolder,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let message = format!(
        "文件夹将改名为「{}」。霁月会先保存所有笔记，然后关闭，并重新打开新的文件夹。",
        pending.new_name
    );
    confirm_dialog(
        &t,
        "重命名文件夹？",
        &message,
        "重命名并重新打开",
        false,
        cx.listener(|this: &mut RustdianApp, _: &MouseDownEvent, _, cx| {
            apply_folder_rename(this, cx);
        }),
        cx.listener(|this: &mut RustdianApp, _: &MouseDownEvent, _, cx| {
            this.ext.vaults.folder_go = None;
            cx.notify();
        }),
    )
}

/// The sidebar menu and the confirmation, drawn over the window.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let t = app.theme;
    let me = cx.entity();
    let mut out = Vec::new();
    if let Some(dialog) = app.ext.vaults.rename.as_ref() {
        out.push(rename_modal(app, dialog, cx));
    }
    if let Some(pending) = app.ext.vaults.folder_go.as_ref() {
        out.push(folder_confirm(app, pending, cx));
    }
    if let Some(menu) = app.ext.vaults.menu.as_ref() {
        let pick_me = me.clone();
        let close_me = me.clone();
        let on_select: PickIndex = Rc::new(move |idx: &usize, _: &mut Window, cx: &mut App| {
            let idx = *idx;
            pick_me.update(cx, |app, cx| pick_menu(app, idx, cx));
        });
        let on_close = move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            close_me.update(cx, |app, cx| {
                app.ext.vaults.menu = None;
                cx.notify();
            });
        };
        out.push(context_menu(
            &t,
            &menu.items,
            None,
            menu.pos,
            menu.win,
            on_select,
            on_close,
        ));
    }
    if let Some(pending) = app.ext.vaults.confirm.as_ref() {
        let yes_me = me.clone();
        let no_me = me.clone();
        out.push(confirm_dialog(
            &t,
            &pending.title,
            &pending.message,
            &pending.confirm,
            false,
            move |_, _, cx| {
                yes_me.update(cx, |app, cx| {
                    if let Some(p) = app.ext.vaults.confirm.take() {
                        switch_now(app, p.path, cx);
                    }
                    cx.notify();
                });
            },
            move |_, _, cx| {
                no_me.update(cx, |app, cx| {
                    app.ext.vaults.confirm = None;
                    cx.notify();
                });
            },
        ));
    }
    out
}

fn pick_menu(app: &mut RustdianApp, idx: usize, cx: &mut Context<RustdianApp>) {
    let cmd = app
        .ext
        .vaults
        .menu
        .take()
        .and_then(|m| m.acts.get(idx).cloned().flatten());
    cx.notify();
    if let Some(cmd) = cmd {
        run(app, cmd, cx);
    }
}

/// A banner in the sidebar when the vault folder cannot be found any more (deleted or renamed
/// while the app runs). The app keeps running; the user can choose another folder.
pub fn missing_banner(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    if app.vault.root_dir.is_dir() {
        return None;
    }
    let t = app.theme;
    let me = cx.entity();
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .mb(px(8.0))
            .px(px(10.0))
            .py(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(t.st)
            .bg(t.hover)
            .child(icon("alert-triangle", 15.0, t.st))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .text_color(t.tx)
                    .child("笔记库文件夹不可用"),
            )
            .child(app_button(
                &me,
                &t,
                "重新选择",
                ButtonKind::Secondary,
                "folder-open",
                |_, cx| {
                    pick(cx, false, |app, path, cx| {
                        begin_candidate(app, path, Mode::Open, cx)
                    });
                },
            ))
            .into_any_element(),
    )
}

// ---------------------------------------------------------------------------
// First-run chooser window
// ---------------------------------------------------------------------------

/// The window shown when no vault is known. Choosing one starts the app again for that vault.
pub struct VaultChooser {
    kind: crate::theme::ThemeKind,
    store: RecentStore,
    message: Option<String>,
    pending: Option<PendingSwitch>,
}

impl VaultChooser {
    pub fn new(_cx: &mut Context<Self>, kind: crate::theme::ThemeKind) -> Self {
        let (store, warning) = vaults::load_store(&vaults::store_path());
        Self {
            kind,
            store,
            message: warning,
            pending: None,
        }
    }

    fn check(&mut self, path: PathBuf, mode: Mode, cx: &mut Context<Self>) {
        validate_then(cx, path, mode, move |this, target, facts, cx| {
            match facts {
                Err(err) => this.message = Some(err),
                Ok(facts) => match verdict(mode, &target, None, &facts) {
                    Action::Switch => this.relaunch(target, cx),
                    Action::AlreadyCurrent => {}
                    Action::Error(message) => this.message = Some(message),
                    Action::Confirm {
                        title,
                        message,
                        confirm,
                    } => {
                        this.pending = Some(PendingSwitch {
                            path: target,
                            title,
                            message,
                            confirm,
                        });
                    }
                },
            }
            cx.notify();
        });
    }

    fn relaunch(&mut self, target: String, cx: &mut Context<Self>) {
        if let Err(err) = vaults::record_opened(&target) {
            self.message = Some(format!("无法记录最近笔记库：{err}"));
        }
        vaults::request_relaunch(&target, self.kind);
        cx.quit();
    }

    fn remove(&mut self, path: &str, cx: &mut Context<Self>) {
        match vaults::update_store(|s| s.remove(path)) {
            Ok(()) => self.store = vaults::load_store(&vaults::store_path()).0,
            Err(err) => self.message = Some(format!("无法更新最近笔记库：{err}")),
        }
        cx.notify();
    }
}

impl Render for VaultChooser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = ActiveTheme::from_kind(self.kind);
        let me = cx.entity();
        let rows = vaults::recent_rows(&self.store, None, |p| Path::new(p).is_dir());

        let mut list = div().flex().flex_col();
        for (i, row) in rows.iter().enumerate() {
            let open_path = row.path.clone();
            let remove_path = row.path.clone();
            let mut actions = div().flex().items_center().gap(px(6.0)).flex_none();
            actions = actions.child(choose_button(
                &me,
                &t,
                &crate::t!("settings-vault-open"),
                ButtonKind::Secondary,
                "folder-open",
                move |this, cx| {
                    this.check(PathBuf::from(open_path.clone()), Mode::Open, cx);
                },
            ));
            actions = actions.child(choose_button(
                &me,
                &t,
                &crate::t!("settings-vault-remove"),
                ButtonKind::Ghost,
                "x",
                move |this, cx| {
                    this.remove(&remove_path, cx);
                },
            ));
            let mut name = div().flex().items_center().gap(px(8.0)).child(
                div()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.tx)
                    .child(SharedString::from(row.name.clone())),
            );
            if row.missing {
                name = name.child(badge(&t, "文件夹不存在", BadgeKind::Warn));
            }
            let left = div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .min_w_0()
                .flex_1()
                .child(name)
                .child(
                    div()
                        .id(("chooser-recent-path", i))
                        .overflow_hidden()
                        .text_size(px(12.0))
                        .text_color(t.tx3)
                        .tooltip(tooltip(t, row.path.clone()))
                        .child(SharedString::from(vaults::middle_truncate(&row.path, 58))),
                );
            let mut item = div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .py(px(8.0))
                .border_b_1()
                .border_color(t.line_soft)
                .child(left)
                .child(actions);
            if row.missing {
                item = item.opacity(0.5);
            }
            list = list.child(item);
        }
        if rows.is_empty() {
            list = list.child(
                div()
                    .py(px(10.0))
                    .text_size(px(12.5))
                    .text_color(t.tx3)
                    .child(SharedString::from(crate::t!("settings-vault-recent-empty"))),
            );
        }

        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .px(px(40.0))
            .pt(px(36.0))
            .pb(px(24.0))
            .bg(t.raised)
            .font_family(crate::ui::fonts::ui_name())
            .text_size(px(13.0))
            .text_color(t.tx)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(22.0))
                    .text_color(t.tx)
                    .child("霁月 Jiyue"),
            )
            .child(
                div()
                    .text_color(t.tx2)
                    .child("选择一个笔记库开始。笔记就是文件夹里的 Markdown 文件。"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(choose_button(
                        &me,
                        &t,
                        "打开笔记库…",
                        ButtonKind::Primary,
                        "folder-open",
                        |_, cx| {
                            pick(cx, false, |v, path, cx| v.check(path, Mode::Open, cx));
                        },
                    ))
                    .child(choose_button(
                        &me,
                        &t,
                        &crate::t!("settings-vault-new"),
                        ButtonKind::Secondary,
                        "folder-plus",
                        |_, cx| {
                            pick(cx, true, |v, path, cx| v.check(path, Mode::Create, cx));
                        },
                    ))
                    .child(choose_button(
                        &me,
                        &t,
                        "使用示例笔记库",
                        ButtonKind::Secondary,
                        "file-plus",
                        |this, cx| {
                            this.check(vaults::sample_vault_path(), Mode::Create, cx);
                        },
                    )),
            )
            .child(section_title(&t, &crate::t!("settings-vault-recent-title")))
            .child(
                div()
                    .id("chooser-recent")
                    .overflow_y_scroll()
                    .flex_1()
                    .min_h_0()
                    .child(list),
            );

        if let Some(message) = self.message.clone() {
            root = root.child(
                div()
                    .text_size(px(12.5))
                    .text_color(t.st)
                    .child(SharedString::from(message)),
            );
        }
        if let Some(pending) = self.pending.clone() {
            let yes = me.clone();
            let no = me.clone();
            root = root.child(confirm_dialog(
                &t,
                &pending.title,
                &pending.message,
                &pending.confirm,
                false,
                move |_, _, cx| {
                    yes.update(cx, |this, cx| {
                        if let Some(p) = this.pending.take() {
                            this.relaunch(p.path, cx);
                        }
                    });
                },
                move |_, _, cx| {
                    no.update(cx, |this, cx| {
                        this.pending = None;
                        cx.notify();
                    });
                },
            ));
        }
        root
    }
}

/// Button of the chooser that runs `act` on the chooser.
fn choose_button(
    me: &Entity<VaultChooser>,
    t: &ActiveTheme,
    label: &str,
    kind: ButtonKind,
    icon_name: &str,
    act: impl Fn(&mut VaultChooser, &mut Context<VaultChooser>) + 'static,
) -> Div {
    let me = me.clone();
    button(t, label, kind, Some(icon_name), move |_, _, cx| {
        me.update(cx, |this, cx| act(this, cx));
    })
}
