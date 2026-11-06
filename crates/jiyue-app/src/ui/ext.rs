//! Extension hub: the single place where `main.rs` meets the wave-2 feature modules.
//!
//! Every feature module (`search_ui`, `files_ui`, `attach_ui`, `sync_ui`, `ai_ui`) exposes the same
//! hooks with the same names. This file fans each hook out to all of them. It also owns the toast
//! layer, which is fully implemented here.
//!
//! Wiring rules (kept in `main.rs`):
//!  - `init` runs once at the end of `RustdianApp::new`.
//!  - `handle_key` runs first for every key press; the first module that returns true wins.
//!  - `overlays` are the last children of the root element.
//!  - `on_event` is called with the `ExtEvent`s listed below.
//!  - `run` executes an `ExtCommand`; see `RustdianApp::dispatch_ext` and `pending_ext` in `main.rs`.

/// Bookmarks (书签): the sidebar section, the star, the dialogs. See `ui/bookmarks/mod.rs`.
#[path = "bookmarks/mod.rs"]
pub mod bookmarks;
/// The `/` insert menu: pure logic here, the popup in `slash_ui.rs`. Declared with `#[path]` so
/// that `ui/mod.rs` does not need a line for it.
#[path = "slash/mod.rs"]
pub mod slash;
#[path = "slash_ui.rs"]
pub mod slash_ui;

use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, KeyDownEvent, MouseButton, SharedString, div, prelude::*, px};

use super::{
    ai_ui, attach_ui, crumb, export_ui, files_ui, history_ui, search_ui, sync_ui, template_ui,
    vaults_ui,
};
use crate::ui::icons::icon;
use crate::{AppCommand, RustdianApp};

/// How long a toast stays on screen. Expired toasts are removed on the next render.
pub const TOAST_LIFETIME: Duration = Duration::from_secs(4);
/// Most toasts visible at once. A new toast past this limit drops the oldest one.
pub const TOAST_MAX_VISIBLE: usize = 4;

/// A command row contributed by a feature module. The palette shows it under 命令.
#[derive(Clone)]
pub struct PaletteCommand {
    pub label: &'static str,
    pub icon: &'static str,
    /// Shortcut text shown as a chip; empty when the command has none.
    pub shortcut: &'static str,
    pub command: AppCommand,
}

/// Per-module state, one field per feature module.
#[derive(Default)]
pub struct ExtState {
    pub search: search_ui::State,
    pub files: files_ui::State,
    pub attach: attach_ui::State,
    pub sync: sync_ui::State,
    pub ai: ai_ui::State,
    pub history: history_ui::State,
    pub export: export_ui::State,
    pub template: template_ui::State,
    pub crumb: crumb::State,
    pub slash: slash_ui::State,
    pub bookmarks: bookmarks::State,
    pub vaults: vaults_ui::State,
    pub toasts: Toasts,
}

/// A command for one feature module. Build it, then run it with `RustdianApp::dispatch_ext`
/// (from a listener, which has a `Context`) or `AppCommand::Ext` (from anywhere else).
#[derive(Clone, Debug)]
pub enum ExtCommand {
    Search(search_ui::Command),
    Files(files_ui::Command),
    Attach(attach_ui::Command),
    Sync(sync_ui::Command),
    Ai(ai_ui::Command),
    History(history_ui::Command),
    Export(export_ui::Command),
    Template(template_ui::Command),
    Crumb(crumb::Command),
    Bookmarks(bookmarks::Command),
    Vaults(vaults_ui::Command),
    /// 插入脚注 on the active note, from the palette. The key chord is a GPUI binding on the editor.
    InsertFootnote,
}

/// Things that happened in the app, broadcast to every module. Paths are vault-relative with
/// forward slashes.
#[derive(Clone, Debug)]
pub enum ExtEvent {
    /// Once, at the first render after `init`.
    Started,
    /// The active note changed to this path.
    NoteOpened(String),
    /// The text of this note changed (every edit; debouncing is the receiver's job).
    NoteChanged(String),
    /// This note was written to disk.
    NoteSaved(String),
    /// The theme changed (cycle or explicit set).
    ThemeChanged,
    /// Files in the vault changed on disk (paths as listed).
    VaultFilesChanged(Vec<String>),
    /// A periodic tick, if a module asks for one.
    Tick,
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

/// Colour and icon of a toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

/// One toast message.
#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub text: String,
    pub until: Instant,
    /// A button shown after the text, if any.
    pub action: Option<ToastAction>,
}

/// What the button of a toast does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastAction {
    /// 在资源管理器中显示: selects the file in Explorer.
    Reveal(std::path::PathBuf),
}

/// The toast queue. Expired toasts are removed by `purge`, which `render` calls every frame.
#[derive(Default)]
pub struct Toasts {
    pub items: Vec<Toast>,
    next_id: u64,
}

impl Toasts {
    /// Adds a toast that lasts `TOAST_LIFETIME`. The oldest one is dropped past `TOAST_MAX_VISIBLE`.
    pub fn push(&mut self, kind: ToastKind, text: String) {
        self.push_with(kind, text, None);
    }

    /// Like `push`, with a button.
    pub fn push_with(&mut self, kind: ToastKind, text: String, action: Option<ToastAction>) {
        self.next_id += 1;
        self.items.push(Toast {
            id: self.next_id,
            kind,
            text,
            until: Instant::now() + TOAST_LIFETIME,
            action,
        });
        while self.items.len() > TOAST_MAX_VISIBLE {
            self.items.remove(0);
        }
    }

    /// Removes the toasts whose time is up.
    pub fn purge(&mut self) {
        let now = Instant::now();
        self.items.retain(|t| t.until > now);
    }

    /// Removes one toast (a click on it).
    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }
}

impl RustdianApp {
    /// Shows a short message above the status bar. It disappears after `TOAST_LIFETIME`.
    /// Callers are responsible for `cx.notify()`.
    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.ext.toasts.push(kind, text.into());
    }

    /// Like `toast`, with a button (see `ToastAction`).
    pub fn toast_action(&mut self, kind: ToastKind, text: impl Into<String>, action: ToastAction) {
        self.ext.toasts.push_with(kind, text.into(), Some(action));
    }
}

/// Shows `path` in the system file manager: Windows Explorer with the item selected, Finder with
/// the item revealed (macOS), or the parent folder in the default file manager (Linux, which has no
/// standard "select" option). Failures are ignored: the button is only a convenience.
pub fn reveal_in_explorer(path: &std::path::Path) {
    if let Some((program, args)) = reveal_command(std::env::consts::OS, path) {
        let _ = std::process::Command::new(program).args(args).spawn();
    }
}

/// Opens `path` with the default program of the system (the default editor for `.toml` when one
/// is set). Failures are ignored: the button is only a convenience.
pub fn open_with_default_app(path: &std::path::Path) {
    if let Some((program, args)) = open_command(std::env::consts::OS, path) {
        let _ = std::process::Command::new(program).args(args).spawn();
    }
}

/// The program and arguments that open `path` with the default application on the OS named `os`.
/// Pure, so each platform's form can be tested anywhere.
pub fn open_command(
    os: &str,
    path: &std::path::Path,
) -> Option<(&'static str, Vec<std::ffi::OsString>)> {
    match os {
        // `start` is a shell built-in, so it runs through cmd; the empty title keeps paths with
        // spaces from being read as the title.
        "windows" => Some((
            "cmd",
            vec![
                "/C".into(),
                "start".into(),
                "".into(),
                path.as_os_str().to_owned(),
            ],
        )),
        "macos" => Some(("open", vec![path.as_os_str().to_owned()])),
        _ => Some(("xdg-open", vec![path.as_os_str().to_owned()])),
    }
}

/// The program and arguments that reveal `path` on the OS named `os` (`std::env::consts::OS`
/// values). Pure, so each platform's form can be tested anywhere.
pub fn reveal_command(
    os: &str,
    path: &std::path::Path,
) -> Option<(&'static str, Vec<std::ffi::OsString>)> {
    match os {
        "windows" => {
            let mut arg = std::ffi::OsString::from("/select,");
            arg.push(path.as_os_str());
            Some(("explorer", vec![arg]))
        }
        "macos" => Some(("open", vec!["-R".into(), path.as_os_str().to_owned()])),
        _ => {
            let dir = path.parent().unwrap_or(path);
            Some(("xdg-open", vec![dir.as_os_str().to_owned()]))
        }
    }
}

/// The toast stack, bottom-right above the status bar. Clicking a toast dismisses it.
fn toast_layer(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let start = app.ext.toasts.items.len().saturating_sub(TOAST_MAX_VISIBLE);
    let mut stack = div()
        .absolute()
        .right(px(16.0))
        .bottom(px(40.0))
        .flex()
        .flex_col()
        .items_end()
        .gap(px(8.0));
    for toast in &app.ext.toasts.items[start..] {
        let (name, tint) = match toast.kind {
            ToastKind::Info => ("info-circle", t.ac),
            ToastKind::Success => ("check", t.ok),
            ToastKind::Warn => ("bulb", t.ac_on),
            ToastKind::Error => ("x", t.st),
        };
        let id = toast.id;
        let action_button = toast.action.clone().map(|action| {
            let label = match &action {
                ToastAction::Reveal(_) => "在资源管理器中显示",
            };
            div()
                .flex_none()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .text_color(t.ac)
                .hover(|s| s.bg(t.hover))
                .child(SharedString::from(label))
                .on_mouse_down(MouseButton::Left, move |_, _, _| match &action {
                    ToastAction::Reveal(path) => reveal_in_explorer(path),
                })
                .into_any_element()
        });
        stack = stack.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .max_w(px(320.0))
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
                .cursor_pointer()
                .child(icon(name, 16.0, tint))
                .child(
                    div()
                        .min_w_0()
                        .child(SharedString::from(toast.text.clone())),
                )
                .children(action_button)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.ext.toasts.dismiss(id);
                        cx.notify();
                    }),
                ),
        );
    }
    stack.into_any_element()
}

// ---------------------------------------------------------------------------
// Dispatch: every hook runs every module, in this fixed order.
// ---------------------------------------------------------------------------

/// Starts the modules. Called once at the end of `RustdianApp::new`.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    search_ui::init(app, cx);
    files_ui::init(app, cx);
    attach_ui::init(app, cx);
    sync_ui::init(app, cx);
    ai_ui::init(app, cx);
    history_ui::init(app, cx);
    template_ui::init(app, cx);
    slash_ui::init(app, cx);
    bookmarks::init(app, cx);
}

/// Overlays of every module, then the toast layer. These are the last children of the root.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    out.extend(search_ui::overlays(app, cx));
    out.extend(files_ui::overlays(app, cx));
    out.extend(attach_ui::overlays(app, cx));
    out.extend(sync_ui::overlays(app, cx));
    out.extend(ai_ui::overlays(app, cx));
    out.extend(history_ui::overlays(app, cx));
    out.extend(export_ui::overlays(app, cx));
    out.extend(template_ui::overlays(app, cx));
    out.extend(crumb::overlay(app, cx));
    out.extend(slash_ui::overlays(app, cx));
    out.extend(bookmarks::overlays(app, cx));
    out.extend(vaults_ui::overlays(app, cx));
    if app.open_widgets_demo {
        out.push(super::widgets::demo::render_demo(app.theme, cx));
    }
    if !app.ext.toasts.items.is_empty() {
        out.push(toast_layer(app, cx));
    }
    out
}

/// Keyboard hook, called before the app's own shortcuts. The first module that returns true wins.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    vaults_ui::handle_key(app, ev, cx)
        || template_ui::handle_key(app, ev, cx)
        || bookmarks::handle_key(app, ev, cx)
        || crumb::handle_key(app, ev, cx)
        || history_ui::handle_key(app, ev, cx)
        || export_ui::handle_key(app, ev, cx)
        || search_ui::handle_key(app, ev, cx)
        || files_ui::handle_key(app, ev, cx)
        || attach_ui::handle_key(app, ev, cx)
        || sync_ui::handle_key(app, ev, cx)
        || ai_ui::handle_key(app, ev, cx)
}

/// Broadcasts an event to every module.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    // First, so the history sees a note's text before the files module reloads it.
    history_ui::on_event(app, ev, cx);
    template_ui::on_event(app, ev, cx);
    bookmarks::on_event(app, ev, cx);
    search_ui::on_event(app, ev, cx);
    files_ui::on_event(app, ev, cx);
    attach_ui::on_event(app, ev, cx);
    sync_ui::on_event(app, ev, cx);
    ai_ui::on_event(app, ev, cx);
    slash_ui::on_event(app, ev, cx);
}

/// Runs a command on the module that owns it.
pub fn run(app: &mut RustdianApp, cmd: ExtCommand, cx: &mut Context<RustdianApp>) {
    match cmd {
        ExtCommand::Search(c) => search_ui::run(app, c, cx),
        ExtCommand::Files(c) => files_ui::run(app, c, cx),
        ExtCommand::Attach(c) => attach_ui::run(app, c, cx),
        ExtCommand::Sync(c) => sync_ui::run(app, c, cx),
        ExtCommand::Ai(c) => ai_ui::run(app, c, cx),
        ExtCommand::History(c) => history_ui::run(app, c, cx),
        ExtCommand::Export(c) => export_ui::run(app, c, cx),
        ExtCommand::Template(c) => template_ui::run(app, c, cx),
        ExtCommand::Crumb(c) => crumb::run(app, c, cx),
        ExtCommand::Bookmarks(c) => bookmarks::run(app, c, cx),
        ExtCommand::Vaults(c) => vaults_ui::run(app, c, cx),
        ExtCommand::InsertFootnote => insert_footnote(app, cx),
    }
}

/// Inserts a footnote into the active note's editor (`TextEditor::insert_footnote`, one undo step).
fn insert_footnote(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some((_, editor)) = ai_ui::active_editor(app) else {
        app.toast(ToastKind::Warn, "没有打开的笔记".to_string());
        return;
    };
    let done = editor.update(cx, |editor, cx| editor.insert_footnote(cx));
    if !done {
        app.toast(
            ToastKind::Warn,
            "当前不能插入脚注（阅读模式或输入法组合中）".to_string(),
        );
    }
    cx.notify();
}

/// Command rows of every module, in module order. The palette appends them after its own rows.
pub fn palette_commands() -> Vec<PaletteCommand> {
    let mut out = Vec::new();
    out.extend(search_ui::palette_commands());
    out.extend(files_ui::palette_commands());
    out.extend(attach_ui::palette_commands());
    out.extend(sync_ui::palette_commands());
    out.extend(ai_ui::palette_commands());
    out.extend(history_ui::palette_commands());
    out.extend(export_ui::palette_commands());
    out.extend(template_ui::palette_commands());
    out.extend(crumb::palette_commands());
    out.extend(bookmarks::palette_commands());
    out.extend(vaults_ui::palette_commands());
    out
}

/// Status bar items of every module, placed before the save-status indicator.
pub fn statusbar_items(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    out.extend(history_ui::statusbar_items(app, cx));
    out.extend(search_ui::statusbar_items(app, cx));
    out.extend(files_ui::statusbar_items(app, cx));
    out.extend(attach_ui::statusbar_items(app, cx));
    out.extend(sync_ui::statusbar_items(app, cx));
    out.extend(ai_ui::statusbar_items(app, cx));
    out
}

/// True when any module has a modal overlay open. Counts as an open overlay for focus.
pub fn overlay_open(app: &RustdianApp) -> bool {
    history_ui::overlay_open(app)
        || template_ui::overlay_open(app)
        || export_ui::overlay_open(app)
        || search_ui::overlay_open(app)
        || files_ui::overlay_open(app)
        || attach_ui::overlay_open(app)
        || sync_ui::overlay_open(app)
        || ai_ui::overlay_open(app)
        || bookmarks::overlay_open(app)
        || vaults_ui::overlay_open(app)
        || app.open_widgets_demo
}

#[cfg(test)]
mod reveal_tests {
    use super::{open_command, reveal_command};
    use std::path::Path;

    #[test]
    fn open_command_uses_the_default_application_per_os() {
        let p = std::path::Path::new("/cfg/credentials.toml");
        assert_eq!(open_command("macos", p).expect("mac").0, "open");
        assert_eq!(open_command("linux", p).expect("linux").0, "xdg-open");
        let (program, args) = open_command("windows", p).expect("windows");
        assert_eq!(program, "cmd");
        assert_eq!(args[1], "start");
    }

    #[test]
    fn windows_selects_the_item_in_explorer() {
        let (program, args) = reveal_command("windows", Path::new(r"D:\notes\a.md")).unwrap();
        assert_eq!(program, "explorer");
        assert_eq!(
            args,
            vec![std::ffi::OsString::from(r"/select,D:\notes\a.md")]
        );
    }

    #[test]
    fn macos_reveals_in_finder() {
        let (program, args) = reveal_command("macos", Path::new("/Users/a/notes/a.md")).unwrap();
        assert_eq!(program, "open");
        assert_eq!(
            args,
            vec![
                std::ffi::OsString::from("-R"),
                std::ffi::OsString::from("/Users/a/notes/a.md")
            ]
        );
    }

    #[test]
    fn linux_opens_the_parent_folder() {
        let (program, args) = reveal_command("linux", Path::new("/home/a/notes/a.md")).unwrap();
        assert_eq!(program, "xdg-open");
        assert_eq!(args, vec![std::ffi::OsString::from("/home/a/notes")]);
    }
}
