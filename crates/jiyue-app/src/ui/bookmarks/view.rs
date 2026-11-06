//! Bookmarks view: the sidebar section, the right-click menu and the three dialogs (重命名,
//! 移到分组, 打开书签). Actions live in `super` (`bookmarks/mod.rs`).

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, FontWeight, IntoElement,
    MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Window, div, prelude::*, px,
};

use super::logic::{self, Bookmark, Kind};
use super::{Modal, MoveOpt};
use crate::RustdianApp;
use crate::theme::{ActiveTheme, ICON_GAP, ICON_GAP_TIGHT, ICON_SIZE};
use crate::ui::icons::icon;
use crate::ui::widgets::controls::PickIndex;
use crate::ui::widgets::overlay::{MenuItem, context_menu};
use crate::ui::widgets::{ButtonKind, LineInput, button, modal};

/// Height of the section body: about six rows, then it scrolls.
const BODY_MAX_H: f32 = 200.0;
/// Most picker rows drawn at once.
const ROW_LIMIT: usize = 200;

/// What a bookmark row carries while it is dragged. Dropping it on another row reorders (or
/// moves into a group); dropping it on empty space moves it to the top level.
#[derive(Clone, Debug)]
pub struct BookmarkDrag {
    pub id: String,
    pub label: String,
    pub theme: ActiveTheme,
}

/// The chip that follows the pointer while a row is dragged.
pub struct DragChip {
    label: SharedString,
    theme: ActiveTheme,
}

impl Render for DragChip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .h(px(26.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .text_size(px(12.5))
            .text_color(t.tx)
            .child(self.label.clone())
    }
}

fn drag_chip(
    value: &BookmarkDrag,
    _pos: Point<Pixels>,
    _window: &mut Window,
    cx: &mut App,
) -> Entity<DragChip> {
    let label = SharedString::from(value.label.clone());
    let theme = value.theme;
    cx.new(|_| DragChip { label, theme })
}

/// The commands of the right-click menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Rename,
    Up,
    Down,
    MoveTo,
    Ungroup,
    Reveal,
    Remove,
}

/// An open row menu.
pub struct Menu {
    /// Where it opened (window px).
    pub pos: (f32, f32),
    /// Size of the window when it opened (the menu flips to stay inside it).
    pub win: (f32, f32),
    /// The entry the menu is for.
    pub id: String,
    pub items: Vec<MenuItem>,
    /// The command of each row (`None` for separators and the rows that are only labels).
    pub acts: Vec<Option<Act>>,
    pub highlight: Option<usize>,
}

/// Rows of the menu for the entry `id`: 重命名, 上移/下移, 移到分组…, 解散分组 (groups), 在资源管理器中
/// 显示 (notes, headings, folders), 移除.
fn menu_rows(app: &RustdianApp, id: &str) -> (Vec<MenuItem>, Vec<Option<Act>>) {
    let items_all = &app.ext.bookmarks.items;
    let Some(bm) = logic::find(items_all, id) else {
        return (Vec::new(), Vec::new());
    };
    let is_group = bm.is_group();
    let has_path = matches!(
        bm.kind,
        Kind::Note { .. } | Kind::Heading { .. } | Kind::Folder { .. }
    );
    let (pos, len) = logic::position_in_list(items_all, id).unwrap_or((0, 1));

    let mut items = Vec::new();
    let mut acts = Vec::new();
    let mut add = |item: MenuItem, act: Option<Act>| {
        items.push(item);
        acts.push(act);
    };
    add(
        MenuItem::new(if is_group {
            "重命名分组"
        } else {
            "重命名书签"
        })
        .icon("pencil"),
        Some(Act::Rename),
    );
    add(MenuItem::separator(), None);
    let up = MenuItem::new("上移").icon("arrow-up");
    add(if pos == 0 { up.disabled() } else { up }, Some(Act::Up));
    let down = MenuItem::new("下移").icon("arrow-down");
    add(
        if pos + 1 >= len {
            down.disabled()
        } else {
            down
        },
        Some(Act::Down),
    );
    add(
        MenuItem::new("移到分组…").icon("folder-plus"),
        Some(Act::MoveTo),
    );
    if is_group {
        add(MenuItem::new("解散分组").icon("folder"), Some(Act::Ungroup));
    }
    add(MenuItem::separator(), None);
    if has_path {
        add(
            MenuItem::new("在资源管理器中显示").icon("folder-open"),
            Some(Act::Reveal),
        );
    }
    let remove = if is_group {
        MenuItem::new("移除分组")
    } else {
        MenuItem::new("移除书签")
    };
    add(remove.icon("trash").danger(), Some(Act::Remove));
    (items, acts)
}

/// Opens the row menu for `id` at window point `pos`.
pub fn open_menu(app: &mut RustdianApp, id: &str, pos: (f32, f32), window: &Window) {
    let size = window.viewport_size();
    let win = (f32::from(size.width), f32::from(size.height));
    let (items, acts) = menu_rows(app, id);
    app.ext.bookmarks.menu = Some(Menu {
        pos,
        win,
        id: id.to_string(),
        items,
        acts,
        highlight: None,
    });
}

/// Runs the menu row `idx` and closes the menu.
pub fn pick_menu(app: &mut RustdianApp, idx: usize, cx: &mut Context<RustdianApp>) {
    let Some(menu) = app.ext.bookmarks.menu.take() else {
        return;
    };
    let act = menu.acts.get(idx).cloned().flatten();
    let id = menu.id;
    match act {
        Some(Act::Rename) => super::open_rename(app, &id, cx),
        Some(Act::Up) => super::step_item(app, &id, -1, cx),
        Some(Act::Down) => super::step_item(app, &id, 1, cx),
        Some(Act::MoveTo) => super::open_move(app, &id, cx),
        Some(Act::Ungroup) => super::ungroup_item(app, &id, cx),
        Some(Act::Reveal) => super::reveal(app, &id),
        Some(Act::Remove) => super::remove_item(app, &id, cx),
        None => {}
    }
    cx.notify();
}

/// The menu layer, drawn over the window.
pub fn menu_overlay(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    let menu = app.ext.bookmarks.menu.as_ref()?;
    let t = app.theme;
    let me = cx.entity();
    let pick_me = me.clone();
    let close_me = me.clone();
    let on_select: PickIndex = Rc::new(move |idx: &usize, _window: &mut Window, cx: &mut App| {
        let idx = *idx;
        pick_me.update(cx, |app, cx| pick_menu(app, idx, cx));
    });
    let on_close = move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
        close_me.update(cx, |app, cx| {
            app.ext.bookmarks.menu = None;
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

// ---------------------------------------------------------------------------
// Sidebar section
// ---------------------------------------------------------------------------

/// The 书签 section: a collapsible header with the count, then the rows (groups fold).
pub fn section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let st = &app.ext.bookmarks;
    let folded = st.collapsed;
    let count = logic::entry_count(&st.items);

    let header = div()
        .id("bookmark_header")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(ICON_GAP_TIGHT))
        .pt(px(14.0))
        .pb(px(5.0))
        .px(px(8.0))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.tx3)
        .cursor_pointer()
        .hover(|s| s.text_color(t.tx2))
        .child(icon(
            if folded {
                "chevron-right"
            } else {
                "chevron-down"
            },
            13.0,
            t.tx3,
        ))
        .child(icon("bookmark", 13.0, t.tx3))
        .child(SharedString::from("书签"))
        .child(
            div()
                .ml(px(4.0))
                .px(px(5.0))
                .rounded(px(5.0))
                .bg(t.hover)
                .text_size(px(11.0))
                .text_color(t.tx3)
                .child(SharedString::from(count.to_string())),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.ext.bookmarks.collapsed = !this.ext.bookmarks.collapsed;
            cx.notify();
        }));

    let mut col = div().flex().flex_col().flex_none().child(header);
    if folded {
        return col.into_any_element();
    }

    let rows = logic::visible(&st.items, &st.collapsed_groups);
    let mut list = div()
        .id("bookmark_list")
        .flex()
        .flex_col()
        .flex_none()
        .max_h(px(BODY_MAX_H))
        .overflow_y_scroll()
        // Dropped on empty space of the section: the top level, at the end.
        .on_drop(cx.listener(|this, drag: &BookmarkDrag, _, cx| {
            super::drop_on_root(this, &drag.id, cx);
        }));
    if rows.is_empty() {
        list = list.child(
            div()
                .px(px(8.0))
                .py(px(4.0))
                .text_size(px(11.5))
                .text_color(t.tx3)
                .child("在笔记上点星标，或右键加入书签"),
        );
    }
    for row in rows {
        if let Some(bm) = logic::find(&st.items, &row.id) {
            list = list.child(item_row(app, cx, bm, row.depth));
        }
    }
    col = col.child(list);
    col.into_any_element()
}

/// One row of the section: icon, title, the 文件已缺失 hint for a missing target.
fn item_row(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    bm: &Bookmark,
    depth: usize,
) -> AnyElement {
    let t = app.theme;
    let missing = super::is_missing(app, &bm.kind);
    let title = logic::display_title(bm, &|p: &str| super::note_title(app, p));
    let is_group = bm.is_group();
    let group_open = is_group && !app.ext.bookmarks.collapsed_groups.contains(&bm.id);
    let icon_name = match &bm.kind {
        Kind::Note { .. } => "file-text",
        Kind::Heading { .. } => "hash",
        Kind::Search { .. } => "search",
        Kind::Folder { .. } => "folder",
        Kind::Group { .. } => {
            if group_open {
                "folder-open"
            } else {
                "folder"
            }
        }
    };

    let id = bm.id.clone();
    let open_id = id.clone();
    let menu_id = id.clone();
    let drop_id = id.clone();
    let drag = BookmarkDrag {
        id: id.clone(),
        label: title.clone(),
        theme: t,
    };

    let mut row = div()
        .id(ElementId::Name(SharedString::from(format!("bm:{id}"))))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(ICON_GAP))
        .h(px(30.0))
        .pl(px(8.0 + depth as f32 * 20.0))
        .pr(px(8.0))
        .rounded(px(6.0))
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(if missing { t.tx3 } else { t.tx2 })
        .hover(|s| s.bg(t.hover).text_color(t.tx));
    if is_group {
        row = row.child(icon(
            if group_open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            13.0,
            t.tx3,
        ));
    }
    row = row.child(icon(icon_name, ICON_SIZE, t.tx3)).child(
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .when(missing, |d| d.line_through())
            .child(SharedString::from(title)),
    );
    if missing {
        row = row.child(
            div()
                .flex_none()
                .text_size(px(11.0))
                .text_color(t.tx3)
                .child("文件已缺失"),
        );
    }
    row.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
        super::open_item(this, &open_id, cx);
    }))
    .on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
            open_menu(this, &menu_id, pos, window);
            cx.notify();
        }),
    )
    .on_drag(drag, drag_chip)
    .drag_over::<BookmarkDrag>(move |s, _, _, _| s.bg(t.ac_soft))
    .on_drop(cx.listener(move |this, drag: &BookmarkDrag, _, cx| {
        super::drop_on(this, &drag.id, &drop_id, cx);
    }))
    .into_any_element()
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// A field in a dialog body.
fn field(input: Option<Entity<LineInput>>) -> Div {
    let mut f = div().mb(px(10.0));
    if let Some(input) = input {
        f = f.child(input);
    }
    f
}

/// A row of a dialog list: a label and a dimmer line under it.
fn list_row(
    t: &ActiveTheme,
    on: bool,
    label: String,
    sub: String,
    listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .px(px(10.0))
        .py(px(7.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if on { t.active } else { t.sheet })
        .hover(|s| s.bg(t.hover))
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx)
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.tx3)
                .child(SharedString::from(sub)),
        )
        .on_mouse_down(MouseButton::Left, listener)
}

/// The open dialog (重命名, 移到分组, 打开书签), if any.
pub fn modal_overlay(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let t = app.theme;
    let st = &app.ext.bookmarks;
    let dismiss = cx.listener(|this: &mut RustdianApp, _, _, cx| super::close(this, cx));
    match st.modal.clone() {
        Modal::None => Vec::new(),
        Modal::Rename(_) => {
            let body = div()
                .flex()
                .flex_col()
                .font_family(crate::ui::fonts::ui_name())
                .child(field(st.rename_input.clone()));
            let footer = vec![
                button(
                    &t,
                    "取消",
                    ButtonKind::Ghost,
                    None,
                    cx.listener(|this, _, _, cx| super::close(this, cx)),
                )
                .into_any_element(),
                button(
                    &t,
                    "保存",
                    ButtonKind::Primary,
                    None,
                    cx.listener(|this, _, _, cx| super::commit_rename(this, cx)),
                )
                .into_any_element(),
            ];
            vec![modal(&t, "重命名书签", body, footer, 380.0, dismiss)]
        }
        Modal::MoveTo(_) => {
            let query = super::move_query(app, cx);
            let options = super::move_options(app, &query);
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .max_h(px(320.0))
                .overflow_hidden();
            if options.is_empty() {
                list = list.child(
                    div()
                        .py(px(12.0))
                        .text_size(px(13.0))
                        .text_color(t.tx3)
                        .child("没有可用的分组"),
                );
            }
            for (row, opt) in options.iter().enumerate() {
                let (label, sub) = match opt {
                    MoveOpt::Root => ("移出分组".to_string(), "放到顶层".to_string()),
                    MoveOpt::Group { path, .. } => (path.clone(), "分组".to_string()),
                    MoveOpt::New(name) => {
                        (format!("新建分组「{name}」"), "回车即可创建".to_string())
                    }
                };
                let on = row == st.highlight;
                list = list.child(list_row(
                    &t,
                    on,
                    label,
                    sub,
                    cx.listener(move |this, _, _, cx| {
                        this.ext.bookmarks.highlight = row;
                        super::commit_move(this, cx);
                    }),
                ));
            }
            let body = div()
                .flex()
                .flex_col()
                .font_family(crate::ui::fonts::ui_name())
                .child(field(st.move_input.clone()))
                .child(list);
            let footer = vec![
                button(
                    &t,
                    "取消",
                    ButtonKind::Ghost,
                    None,
                    cx.listener(|this, _, _, cx| super::close(this, cx)),
                )
                .into_any_element(),
            ];
            vec![modal(&t, "移到分组", body, footer, 420.0, dismiss)]
        }
        Modal::Picker => {
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .max_h(px(320.0))
                .overflow_hidden();
            if st.picker_rows.is_empty() {
                list = list.child(
                    div()
                        .py(px(12.0))
                        .text_size(px(13.0))
                        .text_color(t.tx3)
                        .child("还没有书签。在笔记上点星标，或右键加入书签。"),
                );
            } else if st.picker_shown.is_empty() {
                list = list.child(
                    div()
                        .py(px(12.0))
                        .text_size(px(13.0))
                        .text_color(t.tx3)
                        .child("没有匹配的书签"),
                );
            }
            for (row, &idx) in st.picker_shown.iter().take(ROW_LIMIT).enumerate() {
                let Some(r) = st.picker_rows.get(idx) else {
                    continue;
                };
                let on = row == st.highlight;
                list = list.child(list_row(
                    &t,
                    on,
                    r.label.clone(),
                    r.sub.clone(),
                    cx.listener(move |this, _, _, cx| {
                        this.ext.bookmarks.highlight = row;
                        super::commit_picker(this, cx);
                    }),
                ));
            }
            let body = div()
                .flex()
                .flex_col()
                .font_family(crate::ui::fonts::ui_name())
                .child(field(st.picker_input.clone()))
                .child(list);
            let footer = vec![
                button(
                    &t,
                    "取消",
                    ButtonKind::Ghost,
                    None,
                    cx.listener(|this, _, _, cx| super::close(this, cx)),
                )
                .into_any_element(),
            ];
            vec![modal(&t, "打开书签", body, footer, 520.0, dismiss)]
        }
    }
}
