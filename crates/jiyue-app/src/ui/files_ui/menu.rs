//! Context menus of the file tree (note, folder, empty space) and of the tab strip. The rows of the
//! file tree and their commands come from `pure::menu_rows`; the tab strip's rows are built here,
//! because they depend on the split (向右拆分, 关闭分屏). This module shows the rows and runs the pick.

use std::rc::Rc;

use gpui::{AnyElement, App, Context, MouseDownEvent, Window};

use super::pure::{Act, Target};
use super::{InlineKind, actions, batch, modals, pure, selected_items, start_inline, with_window};
use crate::RustdianApp;
use crate::ui::split;
use crate::ui::split_model::SplitDir;
use crate::ui::widgets::controls::{ListAction, PickIndex};
use crate::ui::widgets::overlay::{MenuItem, context_menu, menu_action};

/// A command of a tab's menu. `usize` is the tab's position in the focused pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCmd {
    SplitRight,
    SplitDown,
    CloseSplit,
    Close(usize),
    CloseOthers(usize),
    CloseRight(usize),
    CopyPath(usize),
    Reveal(usize),
}

/// What a row of the menu does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// A row of the file tree (see `pure::menu_rows`).
    Files(Act),
    /// A row of the tab strip.
    Tab(TabCmd),
}

/// An open context menu.
pub struct Menu {
    pub target: Target,
    /// Where it opened (window px).
    pub pos: (f32, f32),
    /// Size of the window when it opened (the menu flips to stay inside it).
    pub win: (f32, f32),
    pub items: Vec<MenuItem>,
    /// The command of each row (`None` for separators).
    pub acts: Vec<Option<Cmd>>,
    pub highlight: Option<usize>,
}

/// Opens the menu for `target` at `pos`.
pub fn open(app: &mut RustdianApp, target: Target, pos: (f32, f32), win: (f32, f32)) {
    let batch_menu = match &target {
        Target::Tree { path, .. } => super::acts_on_selection(app, path),
        _ => false,
    };
    let rows: Vec<(MenuItem, Option<Cmd>)> = match &target {
        Target::Tab(p) => tab_rows(app, *p),
        _ if batch_menu => pure::batch_menu_rows()
            .into_iter()
            .map(|(item, act)| (item, act.map(Cmd::Files)))
            .collect(),
        _ => pure::menu_rows(&target)
            .into_iter()
            .map(|(item, act)| (item, act.map(Cmd::Files)))
            .collect(),
    };
    let (items, acts): (Vec<MenuItem>, Vec<Option<Cmd>>) = rows.into_iter().unzip();
    app.ext.files.menu = Some(Menu {
        target,
        pos,
        win,
        items,
        acts,
        highlight: None,
    });
}

/// Rows of a tab's menu. The split rows show when the view is one pane; 关闭分屏 only when it is
/// split.
fn tab_rows(app: &RustdianApp, pos: usize) -> Vec<(MenuItem, Option<Cmd>)> {
    let mut rows = vec![
        row(&crate::t!("menu-split-right"), "layout-columns", Cmd::Tab(TabCmd::SplitRight)),
        row(&crate::t!("menu-split-down"), "arrow-down", Cmd::Tab(TabCmd::SplitDown)),
    ];
    if app.layout.split.is_some() {
        rows.push(row(&crate::t!("menu-close-split"), "x", Cmd::Tab(TabCmd::CloseSplit)));
    }
    rows.push(sep());
    rows.push(row(&crate::t!("menu-close-tab"), "x", Cmd::Tab(TabCmd::Close(pos))));
    rows.push(row(&crate::t!("menu-close-other-tabs"), "x", Cmd::Tab(TabCmd::CloseOthers(pos))));
    rows.push(row(&crate::t!("menu-close-right-tabs"), "x", Cmd::Tab(TabCmd::CloseRight(pos))));
    rows.push(sep());
    rows.push(row(&crate::t!("menu-copy-path"), "copy", Cmd::Tab(TabCmd::CopyPath(pos))));
    rows.push(row(
        &crate::t!("menu-reveal-file"),
        "folder-open",
        Cmd::Tab(TabCmd::Reveal(pos)),
    ));
    rows
}

fn row(label: &str, icon: &'static str, cmd: Cmd) -> (MenuItem, Option<Cmd>) {
    (MenuItem::new(label).icon(icon), Some(cmd))
}

fn sep() -> (MenuItem, Option<Cmd>) {
    (MenuItem::separator(), None)
}

/// The menu layer, drawn over the window.
pub fn overlay(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    let menu = app.ext.files.menu.as_ref()?;
    let t = app.theme;
    let me = cx.entity();
    let pick_me = me.clone();
    let close_me = me.clone();
    let on_select: PickIndex = Rc::new(move |idx: &usize, window: &mut Window, cx: &mut App| {
        let idx = *idx;
        pick_me.update(cx, |app, cx| pick(app, idx, window, cx));
    });
    let on_close = move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
        close_me.update(cx, |app, cx| {
            app.ext.files.menu = None;
            cx.notify();
        });
    };
    Some(context_menu(
        &t,
        &menu.items,
        menu.highlight,
        menu.pos,
        menu.win,
        on_select,
        on_close,
    ))
}

/// Keys while a menu is open: arrows move the highlight, Enter picks it, Escape closes it.
/// Other keys go on to the app (typing into an inline row, for example).
pub fn handle_key(app: &mut RustdianApp, key: &str, cx: &mut Context<RustdianApp>) -> bool {
    let Some(menu) = app.ext.files.menu.as_ref() else {
        return false;
    };
    match menu_action(key, &menu.items, menu.highlight) {
        ListAction::Close => {
            app.ext.files.menu = None;
            cx.notify();
            true
        }
        ListAction::Highlight(i) => {
            if let Some(menu) = app.ext.files.menu.as_mut() {
                menu.highlight = Some(i);
            }
            cx.notify();
            true
        }
        ListAction::Pick(i) => {
            let cmd = app
                .ext
                .files
                .menu
                .take()
                .and_then(|m| m.acts.get(i).cloned().flatten());
            cx.notify();
            if let Some(cmd) = cmd {
                with_window(cx, move |app, window, cx| run_cmd(app, cmd, window, cx));
            }
            true
        }
        ListAction::Ignore => false,
    }
}

/// A click on a menu row.
fn pick(app: &mut RustdianApp, idx: usize, window: &mut Window, cx: &mut Context<RustdianApp>) {
    let cmd = app
        .ext
        .files
        .menu
        .take()
        .and_then(|m| m.acts.get(idx).cloned().flatten());
    cx.notify();
    if let Some(cmd) = cmd {
        run_cmd(app, cmd, window, cx);
    }
}

/// Runs the command of a menu row.
fn run_cmd(app: &mut RustdianApp, cmd: Cmd, window: &mut Window, cx: &mut Context<RustdianApp>) {
    match cmd {
        Cmd::Files(act) => run_act(app, act, window, cx),
        Cmd::Tab(tab) => run_tab(app, tab, cx),
    }
    cx.notify();
}

/// Runs a command of a tab's menu.
fn run_tab(app: &mut RustdianApp, cmd: TabCmd, cx: &mut Context<RustdianApp>) {
    match cmd {
        TabCmd::SplitRight => split::split(app, SplitDir::Right),
        TabCmd::SplitDown => split::split(app, SplitDir::Down),
        TabCmd::CloseSplit => split::close_split(app),
        TabCmd::Close(pos) => app.close_tab(pos),
        TabCmd::CloseOthers(pos) => {
            if let Some(&keep) = app.vault.open_tabs.get(pos) {
                app.vault.open_tabs = vec![keep];
                app.vault.active_tab_index = 0;
                app.vault.active_note_index = keep;
            }
        }
        TabCmd::CloseRight(pos) => {
            app.vault.open_tabs.truncate(pos + 1);
            if app.vault.active_tab_index > pos {
                app.vault.active_tab_index = pos;
                if let Some(&idx) = app.vault.open_tabs.get(pos) {
                    app.vault.active_note_index = idx;
                }
            }
        }
        TabCmd::CopyPath(pos) => {
            if let Some(path) = tab_path(app, pos) {
                actions::copy_path(app, &path, cx);
            }
        }
        TabCmd::Reveal(pos) => {
            if let Some(path) = tab_path(app, pos) {
                actions::reveal(app, &path);
            }
        }
    }
}

/// Runs the command of a file tree row.
fn run_act(app: &mut RustdianApp, act: Act, window: &mut Window, cx: &mut Context<RustdianApp>) {
    match act {
        Act::NewNote(dir) => start_inline(app, InlineKind::NewNote { dir }, cx),
        Act::NewFolder(dir) => start_inline(app, InlineKind::NewFolder { dir }, cx),
        Act::Rename { path, is_dir } => start_inline(app, InlineKind::Rename { path, is_dir }, cx),
        Act::Move { path, is_dir } => modals::open_move(app, path, is_dir, cx),
        Act::CopyPath(path) => actions::copy_path(app, &path, cx),
        Act::Bookmark(path) => crate::ui::ext::bookmarks::toggle_path(app, &path, cx),
        Act::Reveal(path) => actions::reveal(app, &path),
        Act::Delete { path, is_dir } => modals::confirm_delete(app, path, is_dir),
        Act::BatchMove => {
            let items = selected_items(app);
            modals::open_move_items(app, items, cx);
        }
        Act::BatchCopyPath => {
            let rels: Vec<String> = selected_items(app).into_iter().map(|(p, _)| p).collect();
            actions::copy_paths(app, &rels, cx);
        }
        Act::BatchBookmark => {
            let paths: Vec<String> = selected_items(app).into_iter().map(|(p, _)| p).collect();
            let toggles = {
                let st: &RustdianApp = app;
                batch::bookmark_toggles(&paths, &|p| {
                    crate::ui::ext::bookmarks::is_bookmarked(st, p)
                })
            };
            for path in toggles {
                crate::ui::ext::bookmarks::toggle_path(app, &path, cx);
            }
        }
        Act::BatchReveal => {
            if let Some((first, _)) = selected_items(app).into_iter().next() {
                actions::reveal(app, &first);
            }
        }
        Act::BatchDelete => {
            let items = selected_items(app);
            modals::confirm_delete_many(app, items);
        }
        Act::CloseTab(_)
        | Act::CloseOthers(_)
        | Act::CloseRight(_)
        | Act::CopyTabPath(_)
        | Act::RevealTab(_) => {}
    }
    let _ = window;
    cx.notify();
}

/// The note path of the tab at `pos`.
fn tab_path(app: &RustdianApp, pos: usize) -> Option<String> {
    app.vault
        .open_tabs
        .get(pos)
        .and_then(|&i| app.vault.notes.get(i))
        .map(|n| n.path.clone())
}
