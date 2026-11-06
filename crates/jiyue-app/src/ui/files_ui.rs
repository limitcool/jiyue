//! File operations UI (wave 2, W2-F): file watching and external edits, the file tree's inline
//! rename and create rows, context menus, move, delete to trash with undo, the trash view and the
//! conflict bar for notes changed on disk.
//!
//! Layout: `files_ui/pure.rs` (name rules, path mapping, menu rows, diff; unit tested),
//! `files_ui/actions.rs` (file operations and the link-updating rename), `files_ui/watch.rs`
//! (watcher polling, reloads, external edits), `files_ui/menu.rs` (context menus),
//! `files_ui/modals.rs` (trash, move picker, diff, confirmations, undo toast).

mod actions;
pub mod autoname;
pub mod dnd;
mod menu;
mod modals;
mod pure;
mod recovery;
mod watch;

pub use modals::confirm_close_despite_failures;
pub use recovery::start_recovery;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, AppContext, Context, Entity, FocusHandle, KeyDownEvent, Subscription, Window,
};

use crate::ui::ext::{ExtCommand, ExtEvent, PaletteCommand, ToastKind};
use crate::ui::widgets::line_input::{LineInput, LineInputEvent};
use crate::{AppCommand, RustdianApp};

pub use pure::Target;
pub use watch::conflict_bar;

/// How often the watcher channel is drained.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// After a save, the file is read again this long after it: a real external edit made within the
/// watcher's self-write window (2 s) is then still found.
const RECONCILE_DELAY: Duration = Duration::from_millis(2500);

/// Module state. Lives in `ExtState::files`.
#[derive(Default)]
pub struct State {
    /// The vault watcher and its polling task (started on `ExtEvent::Started`).
    watch: Option<watch::WatchService>,
    /// Keyboard focus of the file tree. Delete, F2, Enter and Menu act on the tree only while it
    /// holds the focus.
    pub tree_focus: Option<FocusHandle>,
    /// The tree row last clicked (a note path or a folder key).
    pub selected: Option<String>,
    /// The inline row being typed into (new note, new folder, rename).
    inline: Option<Inline>,
    /// Open context menu.
    menu: Option<menu::Menu>,
    /// Open modal (trash, move picker, diff, confirmation).
    modal: Option<modals::Modal>,
    /// The undo toast of the last file operation.
    undo: Option<UndoToast>,
    /// Notes whose file changed on disk while their editor has unsaved changes.
    conflicts: Vec<String>,
    /// The text this process last wrote to each note's file. Watcher events that just repeat it
    /// are not external changes.
    last_written: HashMap<String, String>,
    /// The closed folder under the dragged row and when the pointer arrived (see `dnd`).
    pub drag_hover: Option<(String, Instant)>,
    /// The automatic file name of new notes (see `autoname`).
    pub autoname: autoname::Session,
}

/// Commands this module handles (sent as `ExtCommand::Files`).
#[derive(Clone, Debug)]
pub enum Command {
    NewFolder,
    RenameActive,
    DeleteActive,
    RestoreTrash,
    OpenTrash,
}

/// What the inline row creates or renames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineKind {
    /// A new note in the folder `dir` (`""` for the root).
    NewNote { dir: String },
    /// A new folder in `dir`.
    NewFolder { dir: String },
    /// Rename of a note (`is_dir` false, `path` ends with `.md`) or a folder.
    Rename { path: String, is_dir: bool },
}

/// The inline row of the file tree: an input with a red hint under it when the name is invalid.
pub struct Inline {
    pub kind: InlineKind,
    pub input: Entity<LineInput>,
    pub error: Option<String>,
    _sub: Subscription,
}

/// The undo toast: a message and an action that reverts the operation.
pub struct UndoToast {
    pub text: String,
    op: actions::UndoOp,
    until: std::time::Instant,
}

/// How long the undo toast stays.
const UNDO_LIFETIME: Duration = Duration::from_secs(8);

pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.ext.files.tree_focus = Some(cx.focus_handle());
}

/// Overlays of this module: context menu, modal, undo toast.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if let Some(el) = menu::overlay(app, cx) {
        out.push(el);
    }
    if let Some(el) = modals::overlay(app, cx) {
        out.push(el);
    }
    if let Some(el) = actions::undo_overlay(app, cx) {
        out.push(el);
    }
    out
}

/// Menu keys (arrows, Enter, Escape) and the Escape of the modal and the inline row. Other keys
/// go on to the app.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    let key = ev.keystroke.key.as_str();
    if app.ext.files.menu.is_some() {
        return menu::handle_key(app, key, cx);
    }
    let modifiers = ev.keystroke.modifiers;
    if key == "enter"
        && !(modifiers.control || modifiers.alt || modifiers.platform)
        && matches!(app.ext.files.modal, Some(modals::Modal::Confirm(_)))
    {
        return modals::confirm_enter(app, cx);
    }
    if key == "escape" {
        if app.ext.files.modal.is_some() {
            app.ext.files.modal = None;
            cx.notify();
            return true;
        }
        if app.ext.files.inline.is_some() {
            app.ext.files.inline = None;
            cx.notify();
            return true;
        }
    }
    false
}

/// Starts the watcher on `ExtEvent::Started`; reacts to saves and theme changes.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    match ev {
        ExtEvent::Started => watch::start(app, cx),
        ExtEvent::NoteSaved(path) => watch::note_saved(app, path, cx),
        ExtEvent::ThemeChanged => {
            let theme = app.theme;
            if let Some(inline) = app.ext.files.inline.as_ref() {
                inline
                    .input
                    .update(cx, |input, cx| input.set_theme(theme, cx));
            }
            if let Some(modals::Modal::Move { filter, .. }) = app.ext.files.modal.as_ref() {
                let filter = filter.clone();
                filter.update(cx, |input, cx| input.set_theme(theme, cx));
            }
        }
        _ => {}
    }
}

/// Runs a command. Palette rows and the tree both come here.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::NewFolder => {
            let dir = app
                .active_path()
                .map(pure::parent_of)
                .unwrap_or("")
                .to_string();
            start_inline(app, InlineKind::NewFolder { dir }, cx);
        }
        Command::RenameActive => {
            if let Some(path) = app.active_path().map(str::to_string) {
                reveal_path(app, &path);
                start_inline(
                    app,
                    InlineKind::Rename {
                        path,
                        is_dir: false,
                    },
                    cx,
                );
            }
        }
        Command::DeleteActive => {
            if let Some(path) = app.active_path().map(str::to_string) {
                modals::confirm_delete(app, path, false);
            }
        }
        Command::RestoreTrash => {
            let entries = jiyue_core::fsops::list_trash(&app.vault.root_dir).unwrap_or_default();
            match entries.into_iter().next() {
                Some(entry) => {
                    if let Err(message) = actions::restore_entry(app, &entry, cx) {
                        app.toast(ToastKind::Error, message);
                    }
                }
                None => app.toast(ToastKind::Info, "回收站是空的"),
            }
        }
        Command::OpenTrash => modals::open_trash(app),
    }
    cx.notify();
}

/// Palette rows of this module.
pub fn palette_commands() -> Vec<PaletteCommand> {
    let files = |c: Command| AppCommand::Ext(ExtCommand::Files(c));
    vec![
        PaletteCommand {
            label: "新建文件夹",
            icon: "folder-plus",
            shortcut: "",
            command: files(Command::NewFolder),
        },
        PaletteCommand {
            label: "重命名当前笔记",
            icon: "pencil",
            shortcut: "F2",
            command: files(Command::RenameActive),
        },
        PaletteCommand {
            label: "删除当前笔记",
            icon: "trash",
            shortcut: "",
            command: files(Command::DeleteActive),
        },
        PaletteCommand {
            label: "回收站",
            icon: "trash",
            shortcut: "",
            command: files(Command::OpenTrash),
        },
        PaletteCommand {
            label: "恢复最近删除的项目",
            icon: "arrow-up",
            shortcut: "",
            command: files(Command::RestoreTrash),
        },
    ]
}

pub fn statusbar_items(_app: &RustdianApp, _cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    Vec::new()
}

/// True while a context menu or a modal is open (it then takes the focus).
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.files.menu.is_some() || app.ext.files.modal.is_some()
}

// ---------------------------------------------------------------------------
// Shared helpers used by the sub-modules and the sidebar, tab strip and crumb.
// ---------------------------------------------------------------------------

/// Runs `f` with the window once the current event is over. Entities that are updated from a
/// subscription or a timer have no `Window`; this gets one from the active window handle.
fn with_window(
    cx: &mut Context<RustdianApp>,
    f: impl FnOnce(&mut RustdianApp, &mut Window, &mut Context<RustdianApp>) + 'static,
) {
    let Some(handle) = cx.active_window() else {
        return;
    };
    let weak = cx.weak_entity();
    cx.spawn(async move |_, cx| {
        let _ = cx.update_window(handle, |_, window, cx| {
            let _ = weak.update(cx, |app, cx| f(app, window, cx));
        });
    })
    .detach();
}

/// Makes `path` visible in the tree: its folders are expanded and the row is selected.
pub fn reveal_path(app: &mut RustdianApp, path: &str) {
    for key in pure::ancestor_keys(path) {
        if !app.expanded_folders.contains(&key) {
            app.expanded_folders.push(key);
        }
    }
    app.ext.files.selected = Some(path.to_string());
}

/// Expands a folder and its parents (`key` is a folder key such as `a/b`).
pub fn expand_folder(app: &mut RustdianApp, key: &str) {
    if key.is_empty() {
        return;
    }
    for part in pure::ancestor_keys(&format!("{key}/x")) {
        if !app.expanded_folders.contains(&part) {
            app.expanded_folders.push(part);
        }
    }
}

/// Opens the context menu of the file tree or the tab strip at window point `pos`.
pub fn open_menu(app: &mut RustdianApp, target: Target, pos: (f32, f32), window: &Window) {
    let size = window.viewport_size();
    let win = (f32::from(size.width), f32::from(size.height));
    menu::open(app, target, pos, win);
}

/// Opens the trash view (the sidebar's trash button).
pub fn open_trash_view(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    modals::open_trash(app);
    cx.notify();
}

/// Focuses the tree (so that Delete, F2 and Enter act on the selection).
pub fn focus_tree(app: &RustdianApp, window: &mut Window) {
    if let Some(handle) = app.ext.files.tree_focus.as_ref() {
        window.focus(handle);
    }
}

/// Key handling of the file tree (called from the tree's own key listener).
pub fn tree_key(
    app: &mut RustdianApp,
    ev: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    // Keys typed into an inline row bubble up to the tree too; the row owns them.
    if app.ext.files.inline.is_some() {
        return;
    }
    let key = ev.keystroke.key.as_str();
    let Some(selected) = app.ext.files.selected.clone() else {
        return;
    };
    // Notes always end in `.md`; every other tree row is a folder.
    let is_dir = !selected.ends_with(".md");
    // Rename and delete are registry commands of the sidebar (ui/keymap.rs).
    let command = crate::ui::keymap::lookup_key(&ev.keystroke, crate::ui::keymap::When::Sidebar);
    if command == Some("file.rename") {
        start_inline(
            app,
            InlineKind::Rename {
                path: selected,
                is_dir,
            },
            cx,
        );
        return;
    }
    if command == Some("file.delete") {
        modals::confirm_delete(app, selected, is_dir);
        return;
    }
    match key {
        "enter" if is_dir => {
            if let Some(pos) = app.expanded_folders.iter().position(|k| *k == selected) {
                app.expanded_folders.remove(pos);
            } else {
                app.expanded_folders.push(selected);
            }
        }
        "enter" => {
            if let Some(idx) = app.note_index(&selected) {
                app.open_note_by_index(idx);
            }
        }
        "menu" => {
            let pos = (260.0, 200.0);
            open_menu(
                app,
                Target::Tree {
                    path: selected,
                    is_dir,
                },
                pos,
                window,
            );
        }
        _ => return,
    }
    cx.notify();
}

/// The focus handle of the open inline row (new note, new folder, rename), if any.
pub fn inline_focus_handle(app: &RustdianApp, cx: &gpui::App) -> Option<gpui::FocusHandle> {
    app.ext
        .files
        .inline
        .as_ref()
        .map(|i| gpui::Focusable::focus_handle(i.input.read(cx), cx))
}

/// Starts an inline row: creates the input, expands the folder it lives in and focuses it.
pub fn start_inline(app: &mut RustdianApp, kind: InlineKind, cx: &mut Context<RustdianApp>) {
    app.ext.files.menu = None;
    let theme = app.theme;
    let (placeholder, initial) = match &kind {
        InlineKind::NewNote { .. } => ("笔记名称", String::new()),
        InlineKind::NewFolder { .. } => ("文件夹名称", String::new()),
        InlineKind::Rename { path, is_dir } => {
            let name = pure::name_of(path);
            let initial = if *is_dir {
                name.to_string()
            } else {
                name.strip_suffix(".md").unwrap_or(name).to_string()
            };
            ("新名称", initial)
        }
    };
    match &kind {
        InlineKind::NewNote { dir } | InlineKind::NewFolder { dir } => {
            let dir = dir.clone();
            expand_folder(app, &dir);
        }
        InlineKind::Rename { path, .. } => reveal_path(app, path),
    }
    let input = cx.new(|cx| {
        let mut input = LineInput::new(cx, placeholder);
        input.set_theme(theme, cx);
        input.set_text(&initial, cx);
        input
    });
    let sub = cx.subscribe(
        &input,
        |app: &mut RustdianApp, _, ev: &LineInputEvent, cx| match ev {
            LineInputEvent::Changed(text) => {
                let error = inline_error(app, text);
                if let Some(inline) = app.ext.files.inline.as_mut() {
                    inline.error = error;
                }
                cx.notify();
            }
            LineInputEvent::Submit(text) => commit_inline(app, text.clone(), cx),
            LineInputEvent::Cancel => {
                app.ext.files.inline = None;
                cx.notify();
            }
        },
    );
    let focus_input = input.clone();
    app.ext.files.inline = Some(Inline {
        kind,
        input,
        error: None,
        _sub: sub,
    });
    // The focus moves on the next frame: this frame's render would otherwise hand the keyboard
    // back to the editor (the open menu's overlay has just closed).
    with_window(cx, move |_, window, _| {
        let input = focus_input.clone();
        window.on_next_frame(move |window, cx| input.read(cx).focus(window));
        window.refresh();
    });
    cx.notify();
}

/// The hint for an inline name, or `None` when it is fine (an empty name is fine: it means the
/// default name).
fn inline_error(app: &RustdianApp, text: &str) -> Option<String> {
    let inline = app.ext.files.inline.as_ref()?;
    if text.trim().is_empty() {
        return None;
    }
    let name = match inline.kind {
        InlineKind::NewNote { .. } | InlineKind::Rename { is_dir: false, .. } => {
            pure::note_title_from_input(text)
        }
        _ => text.trim().to_string(),
    };
    pure::validate_item_name(&name).err()
}

/// Enter in an inline row: creates or renames. An invalid name keeps the row open with the hint.
fn commit_inline(app: &mut RustdianApp, text: String, cx: &mut Context<RustdianApp>) {
    let Some(kind) = app.ext.files.inline.as_ref().map(|i| i.kind.clone()) else {
        return;
    };
    let note_like = matches!(
        kind,
        InlineKind::NewNote { .. } | InlineKind::Rename { is_dir: false, .. }
    );
    let name = if note_like {
        pure::note_title_from_input(&text)
    } else {
        text.trim().to_string()
    };
    let name = if matches!(kind, InlineKind::NewNote { .. }) && name.is_empty() {
        "未命名".to_string()
    } else {
        name
    };
    if !name.is_empty() {
        if let Err(message) = pure::validate_item_name(&name) {
            if let Some(inline) = app.ext.files.inline.as_mut() {
                inline.error = Some(message);
            }
            cx.notify();
            return;
        }
    } else {
        // Empty rename or folder name: nothing to do.
        app.ext.files.inline = None;
        cx.notify();
        return;
    }
    app.ext.files.inline = None;
    cx.notify();
    with_window(cx, move |app, window, cx| {
        match kind {
            InlineKind::NewNote { dir } => {
                if let Err(message) = actions::create_note(app, &dir, &name, cx) {
                    app.toast(ToastKind::Error, message);
                }
            }
            InlineKind::NewFolder { dir } => {
                let rel = pure::join(&dir, &name);
                if let Err(message) = actions::create_folder(app, &rel, cx) {
                    app.toast(ToastKind::Error, message);
                }
            }
            InlineKind::Rename { path, is_dir } => {
                let new_name = if is_dir { name } else { format!("{name}.md") };
                let new_path = pure::join(pure::parent_of(&path), &new_name);
                if new_path != path
                    && let Err(message) = actions::rename(app, window, &path, &new_path, is_dir, cx)
                {
                    app.toast(ToastKind::Error, message);
                }
            }
        }
        cx.notify();
    });
}

/// The inline row for the tree, when it lives in the folder `dir` as a creation row.
pub fn inline_in_dir(app: &RustdianApp, dir: &str) -> Option<(Entity<LineInput>, Option<String>)> {
    let inline = app.ext.files.inline.as_ref()?;
    match &inline.kind {
        InlineKind::NewNote { dir: d } | InlineKind::NewFolder { dir: d } if d == dir => {
            Some((inline.input.clone(), inline.error.clone()))
        }
        _ => None,
    }
}

/// The inline rename row that replaces the tree row of `path`, if one is open.
pub fn inline_renaming(
    app: &RustdianApp,
    path: &str,
) -> Option<(Entity<LineInput>, Option<String>)> {
    let inline = app.ext.files.inline.as_ref()?;
    match &inline.kind {
        InlineKind::Rename { path: p, .. } if p == path => {
            Some((inline.input.clone(), inline.error.clone()))
        }
        _ => None,
    }
}

/// Starts the watcher's undo toast for `op`.
fn set_undo(app: &mut RustdianApp, text: String, op: actions::UndoOp) {
    app.ext.files.undo = Some(UndoToast {
        text,
        op,
        until: std::time::Instant::now() + UNDO_LIFETIME,
    });
}

/// Expires the undo toast when its time is up. Returns true when it was removed.
fn purge_undo(app: &mut RustdianApp) -> bool {
    let expired = app
        .ext
        .files
        .undo
        .as_ref()
        .is_some_and(|u| std::time::Instant::now() >= u.until);
    if expired {
        app.ext.files.undo = None;
    }
    expired
}
