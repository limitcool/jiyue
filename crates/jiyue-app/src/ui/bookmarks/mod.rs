//! Bookmarks (书签), like Obsidian's bookmarks plugin but simpler.
//!
//! - Data: `<vault>/.jiyue/bookmarks.json`, read at start-up and written after every change
//!   (atomic write). Entries: notes, headings, saved searches, folders and groups (see `logic`).
//! - Sidebar section 书签 between the calendar and the file tree (`view::section`).
//! - Entry points: the star in the breadcrumb bar, the search panel's 书签 button, the palette rows
//!   (添加/取消当前笔记书签, 为当前标题添加书签, 打开书签…) and `toggle_path` for the file tree menu.
//! - Renames and moves reach the bookmarks through `on_rename`.
//!
//! Layout: `logic` (pure, unit tested), `view` (sidebar, context menu, modals). This file holds the
//! state, the file I/O, the actions and the hooks that `ui/ext.rs` calls.

pub(crate) mod logic;
mod view;

use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{AnyElement, AppContext, Context, Entity, Focusable, KeyDownEvent, Subscription};

// This module is a child of `ui::ext`, so `super` is the hub.
use super::{ExtCommand, ExtEvent, PaletteCommand, ToastKind, reveal_in_explorer};
use crate::AppCommand;
use crate::RustdianApp;
use crate::ui::files_ui;
use crate::ui::search_ui;
use crate::ui::widgets::{LineInput, LineInputEvent, ListAction, is_escape, menu_action};
use logic::{Bookmark, Kind, Toggle};

pub use view::section;

/// Commands of this module (sent as `ExtCommand::Bookmarks`, palette rows and buttons).
#[derive(Clone, Debug)]
pub enum Command {
    /// 添加/取消当前笔记书签.
    ToggleActiveNote,
    /// 为当前标题添加书签: the heading at the caret.
    AddHeadingAtCaret,
    /// 打开书签…
    OpenPicker,
}

/// The modal dialog that is open, if any.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Modal {
    #[default]
    None,
    /// 重命名书签 for the entry with this id.
    Rename(String),
    /// 移到分组… for the entry with this id.
    MoveTo(String),
    /// 打开书签….
    Picker,
}

/// One destination of the 移到分组 dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveOpt {
    /// Out of its group, to the top level.
    Root,
    /// An existing group (its id and path of names).
    Group { id: String, path: String },
    /// A new top-level group with this name.
    New(String),
}

/// Module state. Lives in `ExtState::bookmarks`.
#[derive(Default)]
pub struct State {
    /// The bookmarks in list order.
    pub items: Vec<Bookmark>,
    /// The file could not be read; the next save keeps a copy of it first.
    pub broken: bool,
    /// The section body is folded.
    pub collapsed: bool,
    /// Ids of the groups that are folded.
    pub collapsed_groups: Vec<String>,
    /// The right-click menu of a row.
    pub menu: Option<view::Menu>,
    pub modal: Modal,
    rename_input: Option<Entity<LineInput>>,
    move_input: Option<Entity<LineInput>>,
    picker_input: Option<Entity<LineInput>>,
    /// Picker rows of the last opening, and the indexes that match the filter.
    pub picker_rows: Vec<logic::PickRow>,
    pub picker_shown: Vec<usize>,
    /// Highlighted row of the picker or of the 移到分组 list.
    pub highlight: usize,
    seq: u64,
    subs: Vec<Subscription>,
}

// ---------------------------------------------------------------------------
// Start-up and hooks
// ---------------------------------------------------------------------------

/// Creates the three text fields, then reads the bookmarks file. Called once at start-up.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let theme = app.theme;
    let rename = cx.new(|cx| LineInput::new(cx, "书签名称（留空恢复默认）"));
    let mv = cx.new(|cx| LineInput::new(cx, "搜索分组，或输入新分组名"));
    let picker = cx.new(|cx| LineInput::new(cx, "搜索书签…"));
    for input in [&rename, &mv, &picker] {
        input.update(cx, |i, cx| i.set_theme(theme, cx));
    }
    let s1 = cx.subscribe(
        &rename,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| match ev {
            LineInputEvent::Submit(_) => commit_rename(this, cx),
            LineInputEvent::Cancel => close(this, cx),
            LineInputEvent::Changed(_) => {}
        },
    );
    let s2 = cx.subscribe(
        &mv,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| match ev {
            LineInputEvent::Submit(_) => commit_move(this, cx),
            LineInputEvent::Cancel => close(this, cx),
            LineInputEvent::Changed(_) => {
                this.ext.bookmarks.highlight = 0;
                cx.notify();
            }
        },
    );
    let s3 = cx.subscribe(
        &picker,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| match ev {
            LineInputEvent::Submit(_) => commit_picker(this, cx),
            LineInputEvent::Cancel => close(this, cx),
            LineInputEvent::Changed(query) => {
                let st = &mut this.ext.bookmarks;
                st.picker_shown = logic::picker_matches(&st.picker_rows, query);
                st.highlight = 0;
                cx.notify();
            }
        },
    );
    let st = &mut app.ext.bookmarks;
    st.rename_input = Some(rename);
    st.move_input = Some(mv);
    st.picker_input = Some(picker);
    st.subs = vec![s1, s2, s3];
    load(app);
}

/// Palette rows: the three bookmark commands.
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "添加/取消当前笔记书签",
            icon: "bookmark",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Bookmarks(Command::ToggleActiveNote)),
        },
        PaletteCommand {
            label: "为当前标题添加书签",
            icon: "hash",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Bookmarks(Command::AddHeadingAtCaret)),
        },
        PaletteCommand {
            label: "打开书签…",
            icon: "bookmark",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Bookmarks(Command::OpenPicker)),
        },
    ]
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::ToggleActiveNote => match app.active_path().map(str::to_string) {
            Some(path) => toggle_path(app, &path, cx),
            None => app.toast(ToastKind::Info, "没有打开的笔记"),
        },
        Command::AddHeadingAtCaret => add_heading_at_caret(app),
        Command::OpenPicker => open_picker(app, cx),
    }
    cx.notify();
}

/// Keeps the text fields in the theme.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    if let ExtEvent::ThemeChanged = ev {
        let theme = app.theme;
        let inputs = [
            app.ext.bookmarks.rename_input.clone(),
            app.ext.bookmarks.move_input.clone(),
            app.ext.bookmarks.picker_input.clone(),
        ];
        for input in inputs.into_iter().flatten() {
            input.update(cx, |i, cx| i.set_theme(theme, cx));
        }
    }
}

/// The menu and the dialogs, drawn over the window.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if let Some(menu) = view::menu_overlay(app, cx) {
        out.push(menu);
    }
    out.extend(view::modal_overlay(app, cx));
    out
}

/// True while a dialog of this module is open (keyboard focus goes to it).
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.bookmarks.modal != Modal::None
}

/// Keys of the open menu and dialogs. Returns true when the key was used.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    let key = ev.keystroke.key.as_str();
    let menu_act = app
        .ext
        .bookmarks
        .menu
        .as_ref()
        .map(|m| menu_action(key, &m.items, m.highlight));
    if let Some(act) = menu_act {
        match act {
            ListAction::Close => app.ext.bookmarks.menu = None,
            ListAction::Highlight(i) => {
                if let Some(menu) = app.ext.bookmarks.menu.as_mut() {
                    menu.highlight = Some(i);
                }
            }
            ListAction::Pick(i) => view::pick_menu(app, i, cx),
            ListAction::Ignore => return false,
        }
        cx.notify();
        return true;
    }
    if app.ext.bookmarks.modal == Modal::None {
        return false;
    }
    if is_escape(ev) {
        close(app, cx);
        return true;
    }
    let is_list = matches!(app.ext.bookmarks.modal, Modal::MoveTo(_) | Modal::Picker);
    if !is_list {
        return false;
    }
    let last = match app.ext.bookmarks.modal {
        Modal::Picker => app.ext.bookmarks.picker_shown.len(),
        _ => move_options(app, &move_query(app, cx)).len(),
    }
    .saturating_sub(1);
    let st = &mut app.ext.bookmarks;
    match key {
        "down" => {
            st.highlight = (st.highlight + 1).min(last);
        }
        "up" => {
            st.highlight = st.highlight.saturating_sub(1);
        }
        _ => return false,
    }
    cx.notify();
    true
}

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

fn file_path(app: &RustdianApp) -> std::path::PathBuf {
    jiyue_core::config::vault_file(&app.vault.root_dir, jiyue_core::config::BOOKMARKS_FILE)
}

/// Reads `bookmarks.toml`. A missing file is an empty list. A broken file is ignored (the list
/// stays empty) and is never overwritten: the user is told where the problem is.
fn load(app: &mut RustdianApp) {
    let path = file_path(app);
    if !path.exists() {
        return;
    }
    let loaded: jiyue_core::config::Loaded<logic::BookmarksDoc> =
        jiyue_core::config::load_file(&path);
    match loaded.error {
        None => app.ext.bookmarks.items = logic::from_doc(loaded.value),
        Some(issue) => {
            app.ext.bookmarks.broken = true;
            app.toast(ToastKind::Warn, format!("书签文件有错误，已忽略：{issue}"));
        }
    }
}

/// Writes `bookmarks.toml`. Refused while the file on disk is broken, so a file the user is
/// fixing is never overwritten.
fn save(app: &mut RustdianApp) {
    let path = file_path(app);
    let check: jiyue_core::config::Loaded<logic::BookmarksDoc> =
        jiyue_core::config::load_file(&path);
    if let Some(issue) = check.error {
        app.ext.bookmarks.broken = true;
        app.toast(ToastKind::Error, format!("书签未保存，文件有错误：{issue}"));
        return;
    }
    let result = jiyue_core::config::save_file(
        &path,
        &logic::to_doc(&app.ext.bookmarks.items),
        jiyue_core::config::templates::BOOKMARKS,
        &[],
    );
    app.ext.bookmarks.broken = false;
    if let Err(err) = result {
        app.toast(ToastKind::Error, format!("无法保存书签：{err}"));
    }
}

fn next_id(app: &mut RustdianApp) -> String {
    let st = &mut app.ext.bookmarks;
    st.seq += 1;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    logic::new_id(now, st.seq)
}

// ---------------------------------------------------------------------------
// Lookups used by the view
// ---------------------------------------------------------------------------

/// Title of the note at `path`, if the note exists.
pub fn note_title(app: &RustdianApp, path: &str) -> Option<String> {
    app.vault
        .notes
        .iter()
        .find(|n| n.path == path)
        .map(|n| n.title.clone())
}

fn note_exists(app: &RustdianApp, path: &str) -> bool {
    app.vault.notes.iter().any(|n| n.path == path)
}

fn folder_exists(app: &RustdianApp, path: &str) -> bool {
    app.vault.folders.iter().any(|f| f == path)
}

/// True when the bookmark's target is gone from the vault.
pub fn is_missing(app: &RustdianApp, kind: &Kind) -> bool {
    logic::is_missing(kind, &|p: &str| note_exists(app, p), &|p: &str| {
        folder_exists(app, p)
    })
}

/// True when the note at `path` has a bookmark.
pub fn is_bookmarked(app: &RustdianApp, path: &str) -> bool {
    logic::note_bookmarked(&app.ext.bookmarks.items, path)
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// Adds a bookmark for the note or folder at `path`, or removes the one it has. The file tree's
/// menu calls this for its 添加书签 row (`files_ui\menu.rs`, owned elsewhere).
pub fn toggle_path(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    let kind = if note_exists(app, path) || !folder_exists(app, path) {
        Kind::Note {
            path: path.to_string(),
        }
    } else {
        Kind::Folder {
            path: path.to_string(),
        }
    };
    let title = match &kind {
        Kind::Note { path } => note_title(app, path).unwrap_or_else(|| logic::stem_of(path)),
        Kind::Folder { path } => path.rsplit('/').next().unwrap_or(path).to_string(),
        _ => String::new(),
    };
    let id = next_id(app);
    match logic::toggle(&mut app.ext.bookmarks.items, kind, id) {
        Toggle::Added => app.toast(ToastKind::Success, format!("已加入书签：{title}")),
        Toggle::Removed => app.toast(ToastKind::Info, format!("已取消书签：{title}")),
    }
    save(app);
    cx.notify();
}

/// 为当前标题添加书签: the nearest heading above the caret of the active note.
fn add_heading_at_caret(app: &mut RustdianApp) {
    let Some(note) = app.vault.active_note() else {
        app.toast(ToastKind::Info, "没有打开的笔记");
        return;
    };
    let path = note.path.clone();
    let Some((level, text)) = logic::heading_at_line(note.content(), app.cursor_row) else {
        app.toast(ToastKind::Info, "光标前没有标题");
        return;
    };
    let kind = Kind::Heading {
        path,
        text: text.clone(),
        level,
    };
    if logic::has_duplicate(&app.ext.bookmarks.items, &kind) {
        app.toast(ToastKind::Info, format!("该标题已有书签：{text}"));
        return;
    }
    let id = next_id(app);
    logic::add_unique(&mut app.ext.bookmarks.items, kind, id);
    save(app);
    app.toast(ToastKind::Success, format!("已加入书签：{text}"));
}

/// Saves a search as a bookmark (the search panel's 书签 button).
pub fn add_search(app: &mut RustdianApp, query: &str) {
    let q = query.trim();
    if q.is_empty() {
        return;
    }
    let kind = Kind::Search {
        query: q.to_string(),
    };
    if logic::has_duplicate(&app.ext.bookmarks.items, &kind) {
        app.toast(ToastKind::Info, "该搜索已在书签中");
        return;
    }
    let id = next_id(app);
    logic::add_unique(&mut app.ext.bookmarks.items, kind, id);
    save(app);
    app.toast(ToastKind::Success, format!("已加入书签：搜索「{q}」"));
}

/// Opens a bookmark: the note (and the heading), the saved search, or the folder in the tree.
/// A group is folded or unfolded instead.
pub fn open_item(app: &mut RustdianApp, id: &str, cx: &mut Context<RustdianApp>) {
    let Some(bm) = logic::find(&app.ext.bookmarks.items, id).cloned() else {
        return;
    };
    match bm.kind {
        Kind::Note { path } => open_note(app, &path),
        Kind::Heading { path, text, level } => {
            if let Some(idx) = note_index(app, &path) {
                app.open_note_by_index(idx);
                let content = app.vault.notes[idx].content().to_string();
                match logic::heading_row(&content, &text, level) {
                    Some(row) => app.request_source_caret(row, 0),
                    None => app.toast(ToastKind::Warn, format!("笔记里已没有标题：{text}")),
                }
            } else {
                app.toast(ToastKind::Warn, "文件已不存在，可移除这个书签");
            }
        }
        Kind::Search { query } => search_ui::open_query(app, &query, cx),
        Kind::Folder { path } => {
            if folder_exists(app, &path) {
                files_ui::expand_folder(app, &path);
                app.ext.files.selected = Some(path);
            } else {
                app.toast(ToastKind::Warn, "文件夹已不存在，可移除这个书签");
            }
        }
        Kind::Group { .. } => toggle_group(app, id),
    }
    cx.notify();
}

fn note_index(app: &RustdianApp, path: &str) -> Option<usize> {
    app.vault.notes.iter().position(|n| n.path == path)
}

fn open_note(app: &mut RustdianApp, path: &str) {
    match note_index(app, path) {
        Some(idx) => {
            app.ext.files.selected = Some(path.to_string());
            app.open_note_by_index(idx);
        }
        None => app.toast(ToastKind::Warn, "文件已不存在，可移除这个书签"),
    }
}

/// Folds or unfolds a group.
pub fn toggle_group(app: &mut RustdianApp, id: &str) {
    let st = &mut app.ext.bookmarks;
    if let Some(pos) = st.collapsed_groups.iter().position(|g| g == id) {
        st.collapsed_groups.remove(pos);
    } else {
        st.collapsed_groups.push(id.to_string());
    }
}

/// Removes an entry (a group with its entries).
pub fn remove_item(app: &mut RustdianApp, id: &str, cx: &mut Context<RustdianApp>) {
    if logic::remove(&mut app.ext.bookmarks.items, id) {
        save(app);
        app.toast(ToastKind::Info, "已移除书签");
    }
    cx.notify();
}

/// Moves an entry one step up (-1) or down (+1) in its list.
pub fn step_item(app: &mut RustdianApp, id: &str, delta: isize, cx: &mut Context<RustdianApp>) {
    if logic::move_step(&mut app.ext.bookmarks.items, id, delta) {
        save(app);
    }
    cx.notify();
}

/// Dissolves a group: its entries take its place.
pub fn ungroup_item(app: &mut RustdianApp, id: &str, cx: &mut Context<RustdianApp>) {
    if logic::ungroup(&mut app.ext.bookmarks.items, id) {
        save(app);
    }
    cx.notify();
}

/// A drag ended on an entry: a group takes the dragged entry inside, any other entry is
/// reordered to the dragged entry's new place just before it.
pub fn drop_on(app: &mut RustdianApp, dragged: &str, target: &str, cx: &mut Context<RustdianApp>) {
    if dragged == target {
        return;
    }
    let into_group = logic::find(&app.ext.bookmarks.items, target).is_some_and(|b| b.is_group());
    let items = &mut app.ext.bookmarks.items;
    let moved = if into_group {
        logic::move_into_group(items, dragged, target)
    } else {
        logic::move_before(items, dragged, target)
    };
    if moved {
        save(app);
    } else {
        app.toast(ToastKind::Info, "无法移到这里");
    }
    cx.notify();
}

/// A drag ended on empty space of the section: the entry goes to the end of the top level.
pub fn drop_on_root(app: &mut RustdianApp, dragged: &str, cx: &mut Context<RustdianApp>) {
    if logic::move_to_root(&mut app.ext.bookmarks.items, dragged) {
        save(app);
    }
    cx.notify();
}

/// The file was renamed or moved (`renames` are the note pairs, `moved` the folder that moved).
/// Called from the rename path of `files_ui` (see the report for the call site).
pub fn on_rename(app: &mut RustdianApp, renames: &[(String, String)], moved: Option<(&str, &str)>) {
    if logic::remap(&mut app.ext.bookmarks.items, renames, moved) {
        save(app);
    }
}

/// Reveals the note or folder of a bookmark in Windows Explorer.
pub fn reveal(app: &RustdianApp, id: &str) {
    let Some(bm) = logic::find(&app.ext.bookmarks.items, id) else {
        return;
    };
    let path = match &bm.kind {
        Kind::Note { path } | Kind::Heading { path, .. } | Kind::Folder { path } => path,
        _ => return,
    };
    reveal_in_explorer(&app.vault.root_dir.join(path));
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// Opens 重命名书签 for `id`, with the current title (or the default one) in the field.
pub fn open_rename(app: &mut RustdianApp, id: &str, cx: &mut Context<RustdianApp>) {
    let Some(bm) = logic::find(&app.ext.bookmarks.items, id).cloned() else {
        return;
    };
    let current = match bm.title.clone() {
        Some(t) => t,
        None => {
            let a: &RustdianApp = app;
            logic::display_title(&bm, &|p: &str| note_title(a, p))
        }
    };
    app.ext.bookmarks.modal = Modal::Rename(id.to_string());
    if let Some(input) = app.ext.bookmarks.rename_input.clone() {
        input.update(cx, |i, cx| i.set_text(&current, cx));
        focus_input(&input, cx);
    }
    cx.notify();
}

fn commit_rename(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Modal::Rename(id) = app.ext.bookmarks.modal.clone() else {
        return;
    };
    let text = app
        .ext
        .bookmarks
        .rename_input
        .as_ref()
        .map(|i| i.read(cx).text())
        .unwrap_or_default();
    if logic::set_title(&mut app.ext.bookmarks.items, &id, &text) {
        save(app);
    }
    close(app, cx);
}

/// Opens 移到分组… for `id`.
pub fn open_move(app: &mut RustdianApp, id: &str, cx: &mut Context<RustdianApp>) {
    app.ext.bookmarks.modal = Modal::MoveTo(id.to_string());
    app.ext.bookmarks.highlight = 0;
    if let Some(input) = app.ext.bookmarks.move_input.clone() {
        input.update(cx, |i, cx| i.set_text("", cx));
        focus_input(&input, cx);
    }
    cx.notify();
}

/// Text of the 移到分组 field.
pub fn move_query(app: &RustdianApp, cx: &gpui::App) -> String {
    app.ext
        .bookmarks
        .move_input
        .as_ref()
        .map(|i| i.read(cx).text())
        .unwrap_or_default()
}

/// The destinations of 移到分组 for the open dialog, filtered by its field text `query`.
pub fn move_options(app: &RustdianApp, query: &str) -> Vec<MoveOpt> {
    let Modal::MoveTo(id) = &app.ext.bookmarks.modal else {
        return Vec::new();
    };
    let items = &app.ext.bookmarks.items;
    let q = query.trim().to_lowercase();
    let mut out = Vec::new();
    if q.is_empty() {
        out.push(MoveOpt::Root);
    }
    let mut exact = false;
    for g in logic::group_choices(items, None) {
        if !logic::can_move_into(items, id, &g.id) {
            continue;
        }
        if g.path.to_lowercase() == q {
            exact = true;
        }
        if q.is_empty() || g.path.to_lowercase().contains(&q) {
            out.push(MoveOpt::Group {
                id: g.id,
                path: g.path,
            });
        }
    }
    if !q.is_empty() && !exact {
        out.push(MoveOpt::New(query.trim().to_string()));
    }
    out
}

fn commit_move(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Modal::MoveTo(id) = app.ext.bookmarks.modal.clone() else {
        return;
    };
    let query = move_query(app, cx);
    let options = move_options(app, &query);
    let Some(opt) = options.get(app.ext.bookmarks.highlight).cloned() else {
        return;
    };
    apply_move(app, &id, opt);
    close(app, cx);
}

/// Carries out one destination of the 移到分组 dialog.
pub fn apply_move(app: &mut RustdianApp, id: &str, opt: MoveOpt) {
    // A new group gets its id before the list is borrowed.
    let new_gid = match &opt {
        MoveOpt::New(_) => Some(next_id(app)),
        _ => None,
    };
    let items = &mut app.ext.bookmarks.items;
    let moved = match opt {
        MoveOpt::Root => logic::move_to_root(items, id),
        MoveOpt::Group { id: gid, .. } => logic::move_into_group(items, id, &gid),
        MoveOpt::New(name) => {
            let gid = new_gid.unwrap_or_default();
            logic::add_group(items, &name, gid.clone());
            logic::move_into_group(items, id, &gid)
        }
    };
    if moved {
        save(app);
    } else {
        app.toast(ToastKind::Info, "无法移到这个分组");
    }
}

/// Opens 打开书签….
pub fn open_picker(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let rows = {
        let a: &RustdianApp = app;
        logic::picker_rows(&a.ext.bookmarks.items, &|p: &str| note_title(a, p))
    };
    let shown = logic::picker_matches(&rows, "");
    let st = &mut app.ext.bookmarks;
    st.picker_rows = rows;
    st.picker_shown = shown;
    st.highlight = 0;
    st.modal = Modal::Picker;
    if let Some(input) = st.picker_input.clone() {
        input.update(cx, |i, cx| i.set_text("", cx));
        focus_input(&input, cx);
    }
    cx.notify();
}

fn commit_picker(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let st = &app.ext.bookmarks;
    let id = st
        .picker_shown
        .get(st.highlight)
        .and_then(|&i| st.picker_rows.get(i))
        .map(|r| r.id.clone());
    close(app, cx);
    if let Some(id) = id {
        open_item(app, &id, cx);
    }
}

/// Closes the open dialog (the menu is closed by its own owner).
pub fn close(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.ext.bookmarks.modal = Modal::None;
    cx.notify();
}

/// Gives the field keyboard focus once the dialog is drawn.
fn focus_input(input: &Entity<LineInput>, cx: &mut Context<RustdianApp>) {
    let handle = input.read(cx).focus_handle(cx);
    search_ui::focus_soon(cx, handle);
}

#[cfg(test)]
mod tests {
    #[test]
    fn module_registers_three_palette_rows() {
        assert_eq!(super::palette_commands().len(), 3);
    }
}
