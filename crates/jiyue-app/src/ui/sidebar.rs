//! Left sidebar (248px): workspace switcher, search, quick rows, the real file tree and tags.

use super::{folder_key, path_parts};
use std::collections::BTreeMap;

use gpui::{
    AnyElement, ClickEvent, Context, Div, DragMoveEvent, ElementId, Entity, FontWeight,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, SharedString, div, img, prelude::*, px,
    rgb,
};

use super::calendar::active_daily_date;
use super::files_ui::{self, Target};
use super::icons::icon;
use super::widgets::controls::DANGER_RGB;
use super::widgets::line_input::LineInput;
use crate::daily::Date;
use crate::theme::{ActiveTheme, ICON_GAP, ICON_SIZE};
use crate::{AppCommand, RustdianApp, ViewportMode};

/// Height of the tag block: five 30px rows, then it scrolls.
const TAG_BLOCK_MAX_H: f32 = 150.0;

/// Renders the sidebar column.
pub fn render_sidebar(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let today_active = active_daily_date(app) == Some(Date::today());

    div()
        .flex()
        .flex_col()
        .w(px(248.0))
        .flex_none()
        .h_full()
        .pt(px(10.0))
        .px(px(8.0))
        .pb(px(8.0))
        .bg(t.chrome)
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .text_color(t.tx2)
        .child(workspace(app, cx))
        .children(super::vaults_ui::missing_banner(app, cx))
        .child(
            row(
                &t,
                "search",
                &crate::t!("sidebar-search"),
                Some(&crate::ui::keymap::shortcut_text("view.quick_switch")),
                true,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.show_palette = true;
                    this.is_command_mode = false;
                    this.palette_query.clear();
                    cx.notify();
                }),
            ),
        )
        .child(
            row(
                &t,
                "square-rounded-plus",
                &crate::t!("sidebar-new-note"),
                Some(&crate::ui::keymap::shortcut_text("file.new_note")),
                false,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.new_note();
                    cx.notify();
                }),
            ),
        )
        .child(
            row(&t, "affiliate", &crate::t!("sidebar-graph"), None, false).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.viewport_mode = if this.viewport_mode == ViewportMode::GraphView {
                        ViewportMode::Editor
                    } else {
                        ViewportMode::GraphView
                    };
                    cx.notify();
                }),
            ),
        )
        .child(
            row(
                &t,
                "calendar-event",
                &crate::t!("sidebar-journal"),
                None,
                false,
            )
            .when(today_active, |d| {
                d.bg(t.active)
                    .text_color(t.tx)
                    .font_weight(FontWeight::MEDIUM)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.reset_calendar_month();
                    this.open_today();
                    cx.notify();
                }),
            ),
        )
        .child(app.render_calendar(cx))
        // 书签: a collapsible section (see ui/bookmarks).
        .child(super::ext::bookmarks::section(app, cx))
        .child(section_header(&t, &crate::t!("sidebar-files"), true, cx))
        // The file tree takes the remaining height and scrolls. It has focus after a click on a
        // row, so Delete, F2, Enter and the Menu key act on the selection (files_ui::tree_key).
        .child({
            let mut tree = div()
                .id("file_tree")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(120.0))
                .overflow_y_scroll()
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    files_ui::tree_key(this, ev, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, window, _| {
                        files_ui::focus_tree(this, window);
                    }),
                )
                // Right click on empty space: the root menu. Rows stop the event first.
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        files_ui::open_menu(this, Target::Empty, pos, window);
                        cx.notify();
                    }),
                )
                // Dropped on empty space (rows take their own drops first): the vault root.
                .on_drop(
                    cx.listener(|this, drag: &files_ui::dnd::TreeDrag, window, cx| {
                        files_ui::dnd::drop_into(this, window, drag, "", cx);
                    }),
                )
                .children(file_tree(app, cx));
            if let Some(handle) = app.ext.files.tree_focus.as_ref() {
                tree = tree.track_focus(handle);
            }
            tree
        })
        .child(section_header(&t, &crate::t!("sidebar-tags"), false, cx))
        // At most five tags are visible; more scroll inside this block.
        .child(
            div().flex().flex_col().flex_none().child(
                div()
                    .id("tag_list")
                    .flex()
                    .flex_col()
                    .max_h(px(TAG_BLOCK_MAX_H))
                    .overflow_y_scroll()
                    .child(tags(app, cx)),
            ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.0))
                .child(
                    row(&t, "settings", &crate::t!("sidebar-settings"), None, false)
                        .flex_1()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.run_command(AppCommand::OpenSettings);
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_none()
                        .size(px(30.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .child(icon("trash", ICON_SIZE, t.tx3))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                files_ui::open_trash_view(this, cx);
                            }),
                        ),
                ),
        )
}

/// The workspace switcher at the top: logo mark, vault folder name and note count.
fn workspace(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> impl IntoElement + use<> {
    let t = app.theme;
    // The vault's display name (`[vault] name` in its config), or the folder name.
    let title = app.configs.vault_title();
    let count = app.vault.notes.len();

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(ICON_GAP))
        .px(px(8.0))
        .py(px(6.0))
        .mb(px(6.0))
        .rounded(px(8.0))
        .occlude()
        // The drag area covers the logo and the name only: on Windows it would swallow the clicks
        // of the chevron next to it.
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(ICON_GAP))
                .flex_1()
                .min_w_0()
                .window_control_area(gpui::WindowControlArea::Drag)
                .when(!cfg!(windows), |d| {
                    d.on_mouse_down(gpui::MouseButton::Left, super::tabbar::title_click())
                })
                .child(logo_mark())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.tx)
                                .truncate()
                                .child(SharedString::from(title)),
                        )
                        .child(
                            div()
                                .mt(px(1.0))
                                .text_size(px(11.5))
                                .text_color(t.tx3)
                                .truncate()
                                .child(SharedString::from(crate::t!(
                                    "sidebar-note-count",
                                    count = count
                                ))),
                        ),
                ),
        )
        .child(
            div()
                .id("vaults-chevron")
                .flex_none()
                .size(px(24.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon("selector", 14.0, t.tx3))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        super::vaults_ui::open_menu(this, pos, window);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                ),
        )
}

/// The 30px app logo (`assets/logo/icon-64.png`; its rounded corners are in the PNG alpha).
fn logo_mark() -> impl IntoElement {
    img("logo/icon-64.png").size(px(30.0)).flex_none()
}

/// One 30px sidebar row. `kbd` draws the shortcut badge on the right.
fn row(
    t: &ActiveTheme,
    icon_name: &str,
    label: &str,
    kbd: Option<&str>,
    search: bool,
) -> gpui::Div {
    let mut r = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(ICON_GAP))
        .h(px(if search { 32.0 } else { 30.0 }))
        .pl(px(if search { 7.0 } else { 8.0 }))
        .pr(px(8.0))
        .rounded(px(6.0))
        .whitespace_nowrap()
        .text_color(t.tx2)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover).text_color(t.tx))
        .child(icon(icon_name, ICON_SIZE, t.tx3))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(SharedString::from(label.to_string())),
        );
    if search {
        r = r
            .mb(px(4.0))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .text_color(t.tx3);
    }
    if let Some(k) = kbd {
        r = r.child(
            div()
                .flex_none()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(px(5.0))
                .border_1()
                .border_color(t.line)
                .bg(t.raised)
                .text_size(px(11.0))
                .text_color(t.tx3)
                .child(SharedString::from(k.to_string())),
        );
    }
    r
}

/// Section title such as 文件 or 标签. The file section carries the new-note action.
fn section_header(
    t: &ActiveTheme,
    title: &str,
    with_actions: bool,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let mut h = div()
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .pt(px(14.0))
        .pb(px(5.0))
        .px(px(8.0))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.tx3)
        .child(SharedString::from(title.to_string()));
    if with_actions {
        h = h.child(
            div()
                .flex()
                .gap(px(2.0))
                .child(action_icon("file-plus", t).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.run_command(AppCommand::NewNote);
                        cx.notify();
                    }),
                )),
        );
    }
    h
}

fn action_icon(name: &str, t: &ActiveTheme) -> Div {
    div()
        .p(px(3.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .child(icon(name, ICON_SIZE, t.tx3))
}

#[derive(Default)]
struct DirNode {
    dirs: BTreeMap<String, DirNode>,
    files: Vec<(String, usize)>,
}

/// Builds the folder tree from the folders on disk and the notes' vault-relative paths.
fn build_tree(app: &RustdianApp) -> DirNode {
    let mut root = DirNode::default();
    // Folders first, so that empty ones (no note inside) are shown as well.
    for folder in &app.vault.folders {
        let mut node = &mut root;
        for part in path_parts(folder) {
            node = node.dirs.entry(part.to_string()).or_default();
        }
    }
    for (idx, note) in app.vault.notes.iter().enumerate() {
        let parts = path_parts(&note.path);
        if parts.is_empty() {
            continue;
        }
        let mut node = &mut root;
        for folder in &parts[..parts.len() - 1] {
            node = node.dirs.entry((*folder).to_string()).or_default();
        }
        node.files.push((note.title.clone(), idx));
    }
    for_each_dir_sorted(&mut root);
    root
}

fn for_each_dir_sorted(node: &mut DirNode) {
    node.files.sort_by(|a, b| a.0.cmp(&b.0));
    for child in node.dirs.values_mut() {
        for_each_dir_sorted(child);
    }
}

/// Whether a tree row is in the multi-selection (or is the focused row while nothing is selected).
fn is_row_selected(app: &RustdianApp, path: &str) -> bool {
    let sel = &app.ext.files.selection;
    if sel.is_empty() {
        app.ext.files.selected.as_deref() == Some(path)
    } else {
        sel.contains(path)
    }
}

/// The selection mode of a click from its modifiers: Ctrl toggles, Shift selects the range.
fn click_mode(m: gpui::Modifiers) -> files_ui::selection::ClickMode {
    use files_ui::selection::ClickMode;
    if m.control {
        ClickMode::Toggle
    } else if m.shift {
        ClickMode::Range
    } else {
        ClickMode::Plain
    }
}

/// The rows of the file tree in the order they are shown (expanded folders included). Shift+click
/// and Ctrl+A work in this order.
pub fn visible_rows(app: &RustdianApp) -> Vec<String> {
    fn collect(app: &RustdianApp, node: &DirNode, prefix: &str, out: &mut Vec<String>) {
        for (name, child) in &node.dirs {
            let key = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };
            out.push(key.clone());
            if app.expanded_folders.contains(&key) {
                collect(app, child, &key, out);
            }
        }
        for (_, idx) in &node.files {
            if let Some(note) = app.vault.notes.get(*idx) {
                out.push(note.path.clone());
            }
        }
    }
    let tree = build_tree(app);
    let mut out = Vec::new();
    collect(app, &tree, "", &mut out);
    out
}

/// Visible rows of the file tree. Folders are expanded when listed in `expanded_folders`.
fn file_tree(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let tree = build_tree(app);
    let mut out = Vec::new();
    let active = app.vault.active_note_index;
    // A new note or folder at the root is typed at the top of the tree.
    if let Some((input, error)) = files_ui::inline_in_dir(app, "") {
        out.push(inline_row(app, 0, "file-text", input, error));
    }
    walk(app, &tree, "", 0, active, cx, &mut out);
    out
}

/// The input row shown in place of a tree row (new note, new folder, rename), with the red hint
/// under it when the name is not allowed.
fn inline_row(
    app: &RustdianApp,
    depth: usize,
    icon_name: &str,
    input: Entity<LineInput>,
    error: Option<String>,
) -> AnyElement {
    let t = app.theme;
    let mut col = div()
        .flex()
        .flex_col()
        .flex_none()
        .pl(px(8.0 + depth as f32 * 20.0))
        .pr(px(8.0))
        .py(px(2.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(ICON_GAP))
                .child(icon(icon_name, ICON_SIZE, t.tx3))
                .child(div().flex_1().min_w_0().child(input)),
        );
    if let Some(message) = error {
        col = col.child(
            div()
                .pl(px(25.0))
                .pt(px(2.0))
                .text_size(px(11.5))
                .text_color(rgb(DANGER_RGB))
                .child(SharedString::from(message)),
        );
    }
    col.into_any_element()
}

fn walk(
    app: &RustdianApp,
    node: &DirNode,
    prefix: &str,
    depth: usize,
    active: usize,
    cx: &mut Context<RustdianApp>,
    out: &mut Vec<AnyElement>,
) {
    let t = app.theme;
    for (name, child) in &node.dirs {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        let open = app.expanded_folders.contains(&key);
        let selected = is_row_selected(app, &key);
        if let Some((input, error)) = files_ui::inline_renaming(app, &key) {
            out.push(inline_row(
                app,
                depth,
                if open { "folder-open" } else { "folder" },
                input,
                error,
            ));
        } else {
            let toggle_key = key.clone();
            let menu_key = key.clone();
            let drag_key = key.clone();
            let drop_key = key.clone();
            let hover_key = key.clone();
            let mut r = div()
                .id(ElementId::Name(SharedString::from(format!("folder:{key}"))))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(ICON_GAP))
                .h(px(30.0))
                .pl(px(8.0 + depth as f32 * 20.0))
                .pr(px(8.0))
                .rounded(px(6.0))
                .whitespace_nowrap()
                .text_color(t.tx2)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover).text_color(t.tx));
            if selected {
                r = r.bg(t.hover).text_color(t.tx);
            }
            out.push(
                r.child(icon(
                    if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    13.0,
                    t.tx3,
                ))
                .child(icon(
                    if open { "folder-open" } else { "folder" },
                    ICON_SIZE,
                    t.tx3,
                ))
                .child(div().child(SharedString::from(name.clone())))
                // A click (not a press) toggles, so a drag from this row does not open it.
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    let mode = click_mode(ev.modifiers());
                    files_ui::select_row(this, &toggle_key, mode);
                    // Ctrl and Shift clicks only select; a plain click also opens or closes the folder.
                    if mode == files_ui::selection::ClickMode::Plain {
                        if let Some(pos) =
                            this.expanded_folders.iter().position(|k| *k == toggle_key)
                        {
                            this.expanded_folders.remove(pos);
                        } else {
                            this.expanded_folders.push(toggle_key.clone());
                        }
                    }
                    files_ui::focus_tree(this, window);
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        files_ui::context_select(this, &menu_key);
                        let target = Target::Tree {
                            path: menu_key.clone(),
                            is_dir: true,
                        };
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        files_ui::open_menu(this, target, pos, window);
                        cx.notify();
                    }),
                )
                .on_drag(
                    files_ui::dnd::TreeDrag {
                        items: files_ui::drag_items(app, &drag_key, true),
                        path: drag_key,
                        is_dir: true,
                        theme: t,
                    },
                    files_ui::dnd::preview,
                )
                .drag_over::<files_ui::dnd::TreeDrag>(move |s, _, _, _| s.bg(t.ac_soft))
                .on_drag_move::<files_ui::dnd::TreeDrag>(cx.listener(
                    move |this, ev: &DragMoveEvent<files_ui::dnd::TreeDrag>, _, cx| {
                        let inside = ev.bounds.contains(&ev.event.position);
                        files_ui::dnd::hover_folder(this, &hover_key, inside, cx);
                        cx.notify();
                    },
                ))
                .on_drop(
                    cx.listener(move |this, drag: &files_ui::dnd::TreeDrag, window, cx| {
                        files_ui::dnd::drop_into(this, window, drag, &drop_key, cx);
                    }),
                )
                .into_any_element(),
            );
        }
        if open {
            // A new note or folder created in this folder is typed right below its row.
            if let Some((input, error)) = files_ui::inline_in_dir(app, &key) {
                out.push(inline_row(app, depth + 1, "file-text", input, error));
            }
            walk(app, child, &key, depth + 1, active, cx, out);
        }
    }

    for (title, idx) in &node.files {
        let idx = *idx;
        let Some(note) = app.vault.notes.get(idx) else {
            continue;
        };
        let note_path = note.path.clone();
        if let Some((input, error)) = files_ui::inline_renaming(app, &note_path) {
            out.push(inline_row(app, depth, "file-text", input, error));
            continue;
        }
        let is_active = idx == active;
        let is_selected = is_row_selected(app, &note_path);
        let click_path = note_path.clone();
        let menu_path = note_path.clone();
        let mut r = div()
            .id(ElementId::NamedInteger(
                SharedString::from("file_row"),
                idx as u64,
            ))
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
            .text_color(if is_active { t.tx } else { t.tx2 })
            .hover(|s| s.bg(t.hover).text_color(t.tx));
        if is_active {
            r = r.bg(t.active).font_weight(FontWeight::MEDIUM);
        } else if is_selected {
            r = r.bg(t.hover).text_color(t.tx);
        }
        let is_daily = app.vault.is_daily_note(note);
        let icon_name = if is_daily {
            "calendar-event"
        } else {
            "file-text"
        };
        let icon_color = if is_active { t.ac } else { t.tx3 };
        // A dragged row dropped on a note goes into the note's folder.
        let drop_dir = files_ui::dnd::parent_dir(&note_path).to_string();
        let drag_note = files_ui::dnd::TreeDrag {
            items: files_ui::drag_items(app, &note_path, false),
            path: note_path.clone(),
            is_dir: false,
            theme: t,
        };
        out.push(
            r.child(icon(icon_name, ICON_SIZE, icon_color))
                .child(div().child(SharedString::from(title.clone())))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        let mode = click_mode(ev.modifiers);
                        files_ui::select_row(this, &click_path, mode);
                        files_ui::focus_tree(this, window);
                        // Ctrl and Shift clicks only select; a plain click opens the note.
                        if mode == files_ui::selection::ClickMode::Plain
                            && let Some(note_idx) = this.note_index(&click_path)
                        {
                            this.open_note_by_index(note_idx);
                        }
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        files_ui::context_select(this, &menu_path);
                        let target = Target::Tree {
                            path: menu_path.clone(),
                            is_dir: false,
                        };
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        files_ui::open_menu(this, target, pos, window);
                        cx.notify();
                    }),
                )
                .on_drag(drag_note, files_ui::dnd::preview)
                .on_drop(
                    cx.listener(move |this, drag: &files_ui::dnd::TreeDrag, window, cx| {
                        files_ui::dnd::drop_into(this, window, drag, &drop_dir, cx);
                    }),
                )
                .into_any_element(),
        );
    }
}

/// 标签 section: each tag with the number of notes that use it.
fn tags(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> impl IntoElement + use<> {
    let t = app.theme;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for note in &app.vault.notes {
        for tag in &note.tags {
            *counts.entry(tag.clone()).or_default() += 1;
        }
    }
    // The active tags are those of the search text (an open search only).
    let search_text = if app.ext.search.panel.open {
        super::search_ui::query_text(app, cx)
    } else {
        String::new()
    };
    let mut list = div().flex().flex_col().flex_none();
    for (name, count) in counts {
        let is_active = query_has_tag(&search_text, &name);
        let click_name = name.clone();
        let mut row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(ICON_GAP))
            .h(px(30.0))
            .px(px(8.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_color(t.tx2)
            .hover(|s| s.bg(t.hover).text_color(t.tx));
        if is_active {
            row = row
                .bg(t.active)
                .text_color(t.tx)
                .font_weight(FontWeight::MEDIUM);
        }
        list = list.child(
            row.child(
                div()
                    .w(px(14.0))
                    .text_center()
                    .text_color(t.ac)
                    .font_weight(FontWeight::MEDIUM)
                    .child("#"),
            )
            .child(div().flex_1().min_w_0().child(SharedString::from(name)))
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.5))
                    .text_color(t.tx3)
                    .child(format!("{}", count)),
            )
            // A click searches `tag:<name>`; Ctrl+click adds the tag to the search.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    tag_clicked(this, &click_name, ev.modifiers.control, cx);
                }),
            ),
        );
    }
    list
}

/// Runs the search for a clicked tag. Ctrl+click adds the tag to the text of the open search.
fn tag_clicked(app: &mut RustdianApp, name: &str, extend: bool, cx: &mut Context<RustdianApp>) {
    let open = app.ext.search.panel.open;
    let current = if extend && open {
        super::search_ui::query_text(app, cx)
    } else {
        String::new()
    };
    let query = tag_query_text(&current, name, extend && open);
    super::search_ui::open_query(app, &query, cx);
    cx.notify();
}

/// The search text after a tag click. A plain click searches only the tag. Ctrl+click appends the
/// tag to the text the user has typed; a tag that is already in the text is not added twice.
fn tag_query_text(current: &str, name: &str, extend: bool) -> String {
    let token = format!("tag:{name}");
    let current = current.trim();
    if !extend || current.is_empty() {
        return token;
    }
    if query_has_tag(current, name) {
        return current.to_string();
    }
    format!("{current} {token}")
}

/// Whether the search text has the token `tag:<name>` (a negated `-tag:` does not count).
fn query_has_tag(text: &str, name: &str) -> bool {
    let token = format!("tag:{name}");
    text.split_whitespace().any(|t| t == token)
}

/// Helper used by the folder expansion default: the folder key of a note path.
pub fn default_expanded(path: &str) -> Vec<String> {
    let key = folder_key(path);
    if key.is_empty() {
        Vec::new()
    } else {
        let parts: Vec<&str> = key.split('/').collect();
        (1..=parts.len()).map(|n| parts[..n].join("/")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{query_has_tag, tag_query_text};

    #[test]
    fn plain_click_searches_only_the_tag() {
        assert_eq!(tag_query_text("rust 笔记", "c", false), "tag:c");
        assert_eq!(tag_query_text("", "c", false), "tag:c");
    }

    #[test]
    fn ctrl_click_appends_to_the_typed_text() {
        assert_eq!(tag_query_text("沉积 岩", "b", true), "沉积 岩 tag:b");
        assert_eq!(tag_query_text("  ", "b", true), "tag:b");
    }

    #[test]
    fn ctrl_click_does_not_repeat_a_tag() {
        assert_eq!(tag_query_text("tag:a 沉积", "a", true), "tag:a 沉积");
        assert!(!query_has_tag("-tag:a", "a"));
        assert!(query_has_tag("x tag:a", "a"));
    }
}
