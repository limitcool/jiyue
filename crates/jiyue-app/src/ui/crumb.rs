//! Breadcrumb bar (40px) inside the sheet: folder, folder > note title, daily-note navigation and four action icons.
//!
//! The 更多 (dots) button opens a menu for the active note. Its rows reuse the commands of the
//! other modules (history, export, templates, file operations); the menu itself is drawn by the
//! shared `context_menu` widget through `overlay`.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClipboardItem, Context, Div, FontWeight, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, SharedString, Window, div, prelude::*, px,
};

use super::calendar::active_daily_date;
use super::ext::{ExtCommand, PaletteCommand, ToastKind};
use super::files_ui;
use super::history_ui;
use super::icons::icon;
use super::widgets::controls::{ListAction, PickIndex};
use super::widgets::overlay::{MenuItem, context_menu, menu_action};
use super::{export_ui, folder_key, path_parts, template_ui};
use crate::daily::{Date, weekday_name_zh};
use crate::theme::ActiveTheme;
use crate::{AppCommand, RustdianApp};

/// Commands of this module (sent as `ExtCommand::Crumb`, palette rows).
#[derive(Clone, Debug)]
pub enum Command {
    /// 复制文件路径 for the active note.
    CopyPath,
    /// 在资源管理器中显示 for the active note.
    Reveal,
}

/// One row of the 更多 menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    History,
    ExportHtml,
    CopyHtml,
    CopyMarkdown,
    InsertTemplate,
    SplitRight,
    SplitDown,
    CloseSplit,
    Reveal,
    CopyPath,
    Rename,
    Delete,
}

/// An open 更多 menu.
pub struct Menu {
    /// Where it opened (window px).
    pub pos: (f32, f32),
    /// Size of the window when it opened (the menu flips to stay inside it).
    pub win: (f32, f32),
    pub items: Vec<MenuItem>,
    /// The command of each row (`None` for separators).
    pub acts: Vec<Option<Act>>,
    pub highlight: Option<usize>,
}

/// Module state. Lives in `ExtState::crumb`.
#[derive(Default)]
pub struct State {
    pub menu: Option<Menu>,
}

/// Rows of the 更多 menu in order: label, icon name, command; `None` is a separator.
pub fn row_specs() -> Vec<Option<(&'static str, &'static str, Act)>> {
    vec![
        Some(("历史版本", "clock", Act::History)),
        None,
        Some(("导出为 HTML…", "file-text", Act::ExportHtml)),
        Some(("复制为 HTML", "code", Act::CopyHtml)),
        Some(("复制为 Markdown", "copy", Act::CopyMarkdown)),
        None,
        Some(("插入模板…", "file-plus", Act::InsertTemplate)),
        None,
        Some(("向右拆分", "layout-columns", Act::SplitRight)),
        Some(("向下拆分", "arrow-down", Act::SplitDown)),
        Some(("关闭分屏", "x", Act::CloseSplit)),
        None,
        Some(("在资源管理器中显示", "folder-open", Act::Reveal)),
        Some(("复制文件路径", "copy", Act::CopyPath)),
        None,
        Some(("重命名", "pencil", Act::Rename)),
        Some(("删除", "trash", Act::Delete)),
    ]
}

/// The menu rows as widgets and their commands.
fn build_rows() -> (Vec<MenuItem>, Vec<Option<Act>>) {
    row_specs()
        .into_iter()
        .map(|spec| match spec {
            None => (MenuItem::separator(), None),
            Some((label, icon_name, act)) => {
                let mut item = MenuItem::new(label).icon(icon_name);
                if act == Act::Delete {
                    item = item.danger();
                }
                (item, Some(act))
            }
        })
        .unzip()
}

/// Palette rows: the two commands of this module that have no palette entry elsewhere.
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "复制当前笔记的文件路径",
            icon: "copy",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Crumb(Command::CopyPath)),
        },
        PaletteCommand {
            label: "在资源管理器中显示当前笔记",
            icon: "folder-open",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Crumb(Command::Reveal)),
        },
    ]
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::CopyPath => copy_path(app, cx),
        Command::Reveal => reveal(app),
    }
    cx.notify();
}

/// The vault-relative path of the active note, or a toast when none is open.
fn active_rel(app: &mut RustdianApp) -> Option<String> {
    let rel = app.active_path().map(str::to_owned);
    if rel.is_none() {
        app.toast(ToastKind::Info, "没有打开的笔记");
    }
    rel
}

fn copy_path(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(rel) = active_rel(app) else {
        return;
    };
    let abs = app.vault.root_dir.join(&rel);
    cx.write_to_clipboard(ClipboardItem::new_string(abs.display().to_string()));
    app.toast(ToastKind::Success, "已复制路径");
}

fn reveal(app: &mut RustdianApp) {
    let Some(rel) = active_rel(app) else {
        return;
    };
    let abs = app.vault.root_dir.join(&rel);
    super::ext::reveal_in_explorer(&abs);
}

/// Opens the 更多 menu at window point `pos`.
fn open_menu(app: &mut RustdianApp, pos: (f32, f32), window: &Window) {
    let size = window.viewport_size();
    let win = (f32::from(size.width), f32::from(size.height));
    let (items, acts) = build_rows();
    app.ext.crumb.menu = Some(Menu {
        pos,
        win,
        items,
        acts,
        highlight: None,
    });
}

/// The menu layer, drawn over the window.
pub fn overlay(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    let menu = app.ext.crumb.menu.as_ref()?;
    let t = app.theme;
    let me = cx.entity();
    let pick_me = me.clone();
    let close_me = me.clone();
    let on_select: PickIndex = Rc::new(move |idx: &usize, _window: &mut Window, cx: &mut App| {
        let idx = *idx;
        pick_me.update(cx, |app, cx| pick(app, idx, cx));
    });
    let on_close = move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
        close_me.update(cx, |app, cx| {
            app.ext.crumb.menu = None;
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

/// Keys while the menu is open: arrows move the highlight, Enter picks it, Escape closes it.
/// Other keys go on to the app.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    let key = ev.keystroke.key.as_str();
    let Some(menu) = app.ext.crumb.menu.as_ref() else {
        return false;
    };
    match menu_action(key, &menu.items, menu.highlight) {
        ListAction::Close => {
            app.ext.crumb.menu = None;
            cx.notify();
            true
        }
        ListAction::Highlight(i) => {
            if let Some(menu) = app.ext.crumb.menu.as_mut() {
                menu.highlight = Some(i);
            }
            cx.notify();
            true
        }
        ListAction::Pick(i) => {
            pick(app, i, cx);
            true
        }
        ListAction::Ignore => false,
    }
}

/// A row was chosen: the menu closes and the row's command runs.
fn pick(app: &mut RustdianApp, idx: usize, cx: &mut Context<RustdianApp>) {
    let act = app
        .ext
        .crumb
        .menu
        .take()
        .and_then(|m| m.acts.get(idx).cloned().flatten());
    cx.notify();
    if let Some(act) = act {
        run_act(app, act, cx);
    }
}

fn run_act(app: &mut RustdianApp, act: Act, cx: &mut Context<RustdianApp>) {
    match act {
        Act::History => history_ui::open_modal(app, cx),
        Act::ExportHtml => export_ui::run(app, export_ui::Command::OpenHtmlDialog, cx),
        Act::CopyHtml => export_ui::run(app, export_ui::Command::CopyHtml, cx),
        Act::CopyMarkdown => export_ui::run(app, export_ui::Command::CopyMarkdown, cx),
        Act::InsertTemplate => template_ui::run(app, template_ui::Command::Insert, cx),
        Act::SplitRight => app.run_command(AppCommand::SplitRight),
        Act::SplitDown => app.run_command(AppCommand::SplitDown),
        Act::CloseSplit => app.run_command(AppCommand::CloseSplit),
        Act::Reveal => run(app, Command::Reveal, cx),
        Act::CopyPath => run(app, Command::CopyPath, cx),
        Act::Rename => files_ui::run(app, files_ui::Command::RenameActive, cx),
        Act::Delete => files_ui::run(app, files_ui::Command::DeleteActive, cx),
    }
    cx.notify();
}

/// Renders the breadcrumb bar for the active note.
pub fn render_crumb(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> impl IntoElement + use<> {
    let t = app.theme;
    let note = app.vault.active_note();
    let daily = active_daily_date(app);
    let title = match (note, daily) {
        (Some(n), Some(date)) => daily_crumb_title(&n.title, date),
        (Some(n), None) => n.title.clone(),
        (None, _) => String::new(),
    };
    // Folder segments of the note's path, each with its folder key. The root shows the vault name
    // without a key (nothing to reveal). A click on a segment expands the tree down to it.
    let segments: Vec<(String, String)> = match note {
        Some(n) => {
            let key = folder_key(&n.path);
            if key.is_empty() {
                let root = app
                    .vault
                    .root_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("根目录")
                    .to_string();
                vec![(root, String::new())]
            } else {
                let parts = path_parts(&key);
                (0..parts.len())
                    .map(|i| (parts[i].to_string(), parts[..=i].join("/")))
                    .collect()
            }
        }
        None => Vec::new(),
    };
    let mut folder_row = div()
        .flex()
        .items_center()
        .gap(px(2.0))
        .flex_none()
        .min_w_0();
    for (i, (label, key)) in segments.into_iter().enumerate() {
        if i > 0 {
            folder_row = folder_row.child(icon("chevron-right", 11.0, t.tx3));
        }
        let mut seg = div()
            .flex_none()
            .px(px(4.0))
            .py(px(1.0))
            .rounded(px(5.0))
            .child(SharedString::from(label));
        if !key.is_empty() {
            seg = seg
                .cursor_pointer()
                .hover(|s| s.bg(t.hover).text_color(t.tx2))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        files_ui::expand_folder(this, &key);
                        cx.notify();
                    }),
                );
        }
        folder_row = folder_row.child(seg);
    }

    // Right side: daily navigation (daily notes only), then the action icons.
    let mut right = div()
        .ml_auto()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(2.0));
    if let Some(date) = daily {
        right = right.child(daily_nav(app, cx, date));
    }

    type Listener = Box<dyn Fn(&mut RustdianApp, &mut Context<RustdianApp>)>;
    let action = |name: &'static str, on: bool, listener: Listener| {
        let color = if on { t.ac } else { t.tx2 };
        let mut el = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.0))
            .rounded(px(7.0))
            .text_color(color)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover));
        if on {
            el = el.bg(t.ac_soft);
        }
        el.child(icon(name, 16.0, color)).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                listener(this, cx);
                cx.notify();
            }),
        )
    };

    right = right.child(
        div()
            .flex()
            .gap(px(2.0))
            .child(action(
                "layout-columns",
                app.layout.split.is_some(),
                Box::new(|this, _| super::split::toggle(this)),
            ))
            // Reading-mode button: not wired yet (the pill's 阅读 item is a placeholder).
            .child(action("book-2", false, Box::new(|_, _| {})))
            .child(action(
                "layout-sidebar-right",
                app.right_sidebar_open,
                Box::new(|this, _| {
                    this.right_sidebar_open = !this.right_sidebar_open;
                }),
            ))
            .child(star_button(app, cx))
            .child(dots_button(app, cx)),
    );

    let bar = div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(40.0))
        .flex_none()
        .pl(px(18.0))
        .pr(px(10.0))
        .border_b_1()
        .border_color(t.line_soft)
        .text_size(px(12.5))
        .text_color(t.tx3)
        .child(icon("folder-open", 16.0, t.tx3))
        .child(folder_row)
        .child(icon("chevron-right", 12.0, t.tx3))
        .child(
            div()
                .flex_1()
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx)
                .min_w_0()
                .truncate()
                .child(div().child(SharedString::from(title))),
        )
        .child(right);
    // The conflict bar (a note changed on disk under unsaved edits) sits under the breadcrumb.
    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(bar)
        .children(files_ui::conflict_bar(app, cx))
}

/// The bookmark star: filled in the accent colour while the active note is bookmarked. A click
/// adds or removes the bookmark (see `ui/bookmarks`).
fn star_button(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let on = app
        .active_path()
        .is_some_and(|p| super::ext::bookmarks::is_bookmarked(app, p));
    let color = if on { t.ac } else { t.tx2 };
    let mut el = div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(7.0))
        .text_color(color)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover));
    if on {
        el = el.bg(t.ac_soft);
    }
    el.child(icon(
        if on { "bookmark-filled" } else { "bookmark" },
        16.0,
        color,
    ))
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _: &MouseDownEvent, _, cx| {
            if let Some(path) = this.active_path().map(str::to_string) {
                super::ext::bookmarks::toggle_path(this, &path, cx);
            }
            cx.notify();
        }),
    )
}

/// The 更多 button: opens the menu at the pointer.
fn dots_button(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let open = app.ext.crumb.menu.is_some();
    let color = if open { t.ac } else { t.tx2 };
    let mut el = div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(7.0))
        .text_color(color)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover));
    if open {
        el = el.bg(t.ac_soft);
    }
    el.child(icon("dots", 16.0, color)).on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
            open_menu(
                this,
                (f32::from(ev.position.x), f32::from(ev.position.y)),
                window,
            );
            cx.notify();
        }),
    )
}

/// Previous / today / next controls for a daily note. Previous and next are disabled
/// (dimmed, no hover) when no other daily note exists in that direction.
fn daily_nav(app: &RustdianApp, cx: &mut Context<RustdianApp>, date: Date) -> Div {
    let t = app.theme;
    let has_prev = app.vault.daily_adjacent_existing(date, false).is_some();
    let has_next = app.vault.daily_adjacent_existing(date, true).is_some();

    let prev = nav_button(&t, "chevron-left", has_prev);
    let prev = if has_prev {
        prev.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.run_command(AppCommand::DailyPrev);
                cx.notify();
            }),
        )
    } else {
        prev
    };

    let today = div()
        .flex()
        .items_center()
        .px(px(7.0))
        .h(px(24.0))
        .rounded(px(6.0))
        .text_size(px(12.0))
        .text_color(t.tx2)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover).text_color(t.tx))
        .child("今天")
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.reset_calendar_month();
                this.run_command(AppCommand::OpenToday);
                cx.notify();
            }),
        );

    let next = nav_button(&t, "chevron-right", has_next);
    let next = if has_next {
        next.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.run_command(AppCommand::DailyNext);
                cx.notify();
            }),
        )
    } else {
        next
    };

    div()
        .flex()
        .items_center()
        .gap(px(2.0))
        .mr(px(6.0))
        .child(prev)
        .child(today)
        .child(next)
}

/// A 26px icon button; disabled buttons are drawn faint and have no hover.
fn nav_button(t: &ActiveTheme, name: &'static str, enabled: bool) -> Div {
    let mut color = t.tx2;
    if !enabled {
        color.a = 0.3;
    }
    let el = div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(26.0))
        .rounded(px(7.0))
        .child(icon(name, 16.0, color));
    if enabled {
        el.cursor_pointer().hover(|s| s.bg(t.hover))
    } else {
        el
    }
}

/// Crumb title of a daily note: the note title and its weekday, e.g. "2026-11-03 · 星期二".
pub fn daily_crumb_title(title: &str, date: Date) -> String {
    format!("{} · {}", title, weekday_name_zh(date))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_title_shows_weekday() {
        let date = Date::new(2026, 11, 3).expect("valid date");
        assert_eq!(daily_crumb_title("2026-11-03", date), "2026-11-03 · 星期二");
        let sunday = Date::new(2026, 11, 8).expect("valid date");
        assert_eq!(
            daily_crumb_title("2026-11-08", sunday),
            "2026-11-08 · 星期日"
        );
    }

    #[test]
    fn menu_lists_the_twelve_rows_in_order() {
        let labels: Vec<&str> = row_specs()
            .into_iter()
            .flatten()
            .map(|(l, _, _)| l)
            .collect();
        assert_eq!(
            labels,
            vec![
                "历史版本",
                "导出为 HTML…",
                "复制为 HTML",
                "复制为 Markdown",
                "插入模板…",
                "向右拆分",
                "向下拆分",
                "关闭分屏",
                "在资源管理器中显示",
                "复制文件路径",
                "重命名",
                "删除",
            ]
        );
    }

    #[test]
    fn every_act_appears_once_and_every_icon_exists() {
        let rows: Vec<(&str, &str, Act)> = row_specs().into_iter().flatten().collect();
        let mut acts: Vec<Act> = rows.iter().map(|(_, _, a)| *a).collect();
        acts.sort_by_key(|a| format!("{a:?}"));
        acts.dedup();
        assert_eq!(acts.len(), 12, "each of the twelve acts is in the menu");
        assert_eq!(rows.len(), 12, "no act is listed twice");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        for (_, icon_name, _) in &rows {
            assert!(
                dir.join(format!("{icon_name}.svg")).is_file(),
                "missing icon {icon_name}"
            );
        }
    }
}
