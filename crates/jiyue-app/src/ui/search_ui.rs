//! Global search panel (Ctrl+Shift+F) and in-note find / replace (Ctrl+F, Ctrl+H).
//! Owned by W2-S in wave 2.
//!
//! Layout of the module:
//! - `logic`: pure helpers (query building per scope, match finding, replace edits, history).
//! - `index`: the note index in `<vault>/.jiyue/index/search`, its background build and updates.
//! - `panel`: the global search overlay.
//! - `find`: the find / replace bar for the active note.
//!
//! This file wires the module to the extension hub (`ui/ext.rs`).

mod find;
mod index;
mod logic;
mod panel;

use gpui::{AnyElement, App, Context, FocusHandle, KeyDownEvent, div, prelude::*};

use crate::ui::ext::{ExtCommand, ExtEvent, PaletteCommand};
use crate::{AppCommand, RustdianApp};

/// Module state, one field per part.
#[derive(Default)]
pub struct State {
    pub service: index::Service,
    pub panel: panel::PanelState,
    pub find: find::FindState,
}

/// Commands this module handles (sent as `ExtCommand::Search`).
// The variants name the palette commands one to one; the shared `Open` prefix is the point.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug)]
pub enum Command {
    OpenGlobalSearch,
    OpenFind,
    OpenReplace,
}

pub fn init(_app: &mut RustdianApp, _cx: &mut Context<RustdianApp>) {}

/// The text of the global search field (empty when the panel was never opened).
pub fn query_text(app: &RustdianApp, cx: &App) -> String {
    panel::query_text(app, cx)
}

/// Opens the global search with `query` and runs it (a saved-search bookmark).
pub fn open_query(app: &mut RustdianApp, query: &str, cx: &mut Context<RustdianApp>) {
    panel::open_with_query(app, query, cx);
}

pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if app.ext.search.panel.open {
        out.push(panel::render(app, cx));
    }
    if app.ext.search.find.open {
        out.push(find::render(app, cx));
    }
    out
}

/// The keys of the open panel or bar. Returns true when the key was used. Keys of another open
/// overlay (palette, settings) are left to the app. The opening chords are registry commands.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    if app.show_palette || app.settings.open {
        return false;
    }
    if app.ext.search.panel.open && panel::handle_key(app, ev, cx) {
        return true;
    }
    if app.ext.search.find.open && find::handle_key(app, ev, cx) {
        return true;
    }
    // Ctrl+Shift+F, Ctrl+F and Ctrl+H are registry commands (ui/keymap.rs), run by main.rs.
    false
}

/// Reacts to the app's events: the index follows the vault, the find bar follows the note.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    match ev {
        ExtEvent::Started => index::start(app, cx),
        ExtEvent::NoteSaved(path) => index::note_saved(app, path, cx),
        ExtEvent::NoteChanged(path) => {
            index::note_changed(app, path, cx);
            find::on_note_changed(app, path, cx);
        }
        ExtEvent::NoteOpened(path) => find::on_note_opened(app, path, cx),
        ExtEvent::VaultFilesChanged(paths) => index::files_changed(app, paths, cx),
        ExtEvent::ThemeChanged | ExtEvent::Tick => {}
    }
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::OpenGlobalSearch => panel::open(app, cx),
        Command::OpenFind => find::open(app, false, cx),
        Command::OpenReplace => find::open(app, true, cx),
    }
}

/// Command rows for the palette.
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "全局搜索",
            icon: "search",
            shortcut: "Ctrl Shift F",
            command: AppCommand::Ext(ExtCommand::Search(Command::OpenGlobalSearch)),
        },
        PaletteCommand {
            label: "在笔记中查找",
            icon: "search",
            shortcut: "Ctrl F",
            command: AppCommand::Ext(ExtCommand::Search(Command::OpenFind)),
        },
        PaletteCommand {
            label: "替换",
            icon: "pencil",
            shortcut: "Ctrl H",
            command: AppCommand::Ext(ExtCommand::Search(Command::OpenReplace)),
        },
    ]
}

/// `索引中 n/m` while the index is built. Nothing when idle.
pub fn statusbar_items(app: &RustdianApp, _cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    match index::progress(app) {
        Some((done, total)) => vec![
            div()
                .flex_none()
                .child(format!("索引中 {done}/{total}"))
                .into_any_element(),
        ],
        None => Vec::new(),
    }
}

/// The global panel is modal (it takes the focus and the scrim). The find bar is not.
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.search.panel.open
}

/// Gives keyboard focus to `handle` after the overlay that holds it has been drawn. A focus set in
/// the frame the overlay appears would be undone by the app's own focus rule for that frame, so
/// the focus waits two frames.
pub fn focus_soon(cx: &mut Context<RustdianApp>, handle: FocusHandle) {
    cx.defer(move |cx: &mut App| {
        let Some(window) = cx.active_window() else {
            return;
        };
        let handle = handle.clone();
        let _ = window.update(cx, move |_, window, _| {
            window.refresh();
            window.on_next_frame(move |window, _| {
                window.refresh();
                window.on_next_frame(move |window, _| window.focus(&handle));
            });
        });
    });
}
