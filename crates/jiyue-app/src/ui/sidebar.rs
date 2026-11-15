//! Activity bar (42px) and modular left sidebar (236px): files/tags, search, journal, memos, bookmarks.

use super::{folder_key, path_parts};
use std::collections::BTreeMap;

use gpui::{
    AnyElement, ClickEvent, Context, DragMoveEvent, ElementId, Entity, FontWeight, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, SharedString, WindowControlArea, div, img,
    prelude::*, px,
};

use super::files_ui::{self, Target};
use super::icons::icon;
use super::widgets::controls::{IconBtnSize, icon_button_sized};
use super::widgets::line_input::LineInput;
use crate::theme::{
    ACTIVITY_BAR_WIDTH, HEADER_HEIGHT, ICON_GAP, ICON_SIZE, RADIUS_S, ROW_HEIGHT, SIDEBAR_WIDTH,
};
use crate::{AppCommand, RustdianApp, ViewportMode};

/// Which module is active inside the left sidebar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SidebarTab {
    #[default]
    Files,
    Search,
    Journal,
    Memos,
    Bookmarks,
}

/// Height of the tag block: five 28px rows, then it scrolls.
const TAG_BLOCK_MAX_H: f32 = 140.0;

/// Renders the fixed 42px left activity bar spanning the full window height.
pub fn render_activity_bar(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;

    let rail_tab_btn = |icon_name: &'static str, tab: SidebarTab| {
        let active = app.left_sidebar_open
            && app.sidebar_tab == tab
            && app.viewport_mode == ViewportMode::Editor;
        div()
            .id(ElementId::Name(SharedString::from(format!(
                "rail_tab_{icon_name}"
            ))))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .size(px(30.0))
            .rounded(px(RADIUS_S))
            .cursor_pointer()
            .when(active, |d| {
                d.bg(t.active).child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(7.0))
                        .w(px(2.0))
                        .h(px(16.0))
                        .rounded_r(px(1.0))
                        .bg(t.ac),
                )
            })
            .when(!active, |d| d.hover(|s| s.bg(t.hover)))
            .child(icon(icon_name, 16.0, if active { t.ac } else { t.tx3 }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.left_sidebar_open
                        && this.sidebar_tab == tab
                        && this.viewport_mode == ViewportMode::Editor
                    {
                        this.left_sidebar_open = false;
                        crate::config_store::sync_ui(this);
                    } else {
                        this.viewport_mode = ViewportMode::Editor;
                        this.sidebar_tab = tab;
                        this.left_sidebar_open = true;
                        crate::config_store::sync_ui(this);
                        if tab == SidebarTab::Search {
                            super::search_ui::focus_sidebar_input(this, cx);
                        }
                    }
                    cx.notify();
                }),
            )
    };

    let graph_on = app.viewport_mode == ViewportMode::GraphView;
    let graph_btn = div()
        .id("rail_graph")
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .size(px(30.0))
        .rounded(px(RADIUS_S))
        .cursor_pointer()
        .when(graph_on, |d| {
            d.bg(t.active).child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(7.0))
                    .w(px(2.0))
                    .h(px(16.0))
                    .rounded_r(px(1.0))
                    .bg(t.ac),
            )
        })
        .when(!graph_on, |d| d.hover(|s| s.bg(t.hover)))
        .child(icon("affiliate", 16.0, if graph_on { t.ac } else { t.tx3 }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.viewport_mode = if this.viewport_mode == ViewportMode::GraphView {
                    ViewportMode::Editor
                } else {
                    ViewportMode::GraphView
                };
                cx.notify();
            }),
        );

    let rail_action_btn = |id: &'static str, icon_name: &'static str| {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .size(px(30.0))
            .rounded(px(RADIUS_S))
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(icon(icon_name, 16.0, t.tx3))
    };

    div()
        .flex()
        .flex_col()
        .items_center()
        .flex_none()
        .w(px(ACTIVITY_BAR_WIDTH))
        .h_full()
        .bg(t.canvas)
        .border_r_1()
        .border_color(t.border_subtle)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .w_full()
                .h(px(HEADER_HEIGHT))
                .child(
                    div()
                        .id("rail_vault_logo")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.0))
                        .rounded(px(RADIUS_S))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .child(logo_mark())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                                super::vaults_ui::open_menu(this, pos, window);
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(6.0))
                .gap(px(4.0))
                .child(rail_tab_btn("folder", SidebarTab::Files))
                .child(rail_tab_btn("search", SidebarTab::Search))
                .child(rail_tab_btn("calendar", SidebarTab::Journal))
                .child(rail_tab_btn("checkbox", SidebarTab::Memos))
                .child(rail_tab_btn("bookmark", SidebarTab::Bookmarks))
                .child(div().w(px(20.0)).h(px(1.0)).my(px(4.0)).bg(t.border_subtle))
                .child(graph_btn),
        )
        .child(
            div()
                .mt_auto()
                .pb(px(8.0))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .child(rail_action_btn("rail_trash", "trash").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        files_ui::open_trash_view(this, cx);
                    }),
                ))
                .child(rail_action_btn("rail_theme", "palette").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.run_command(AppCommand::CycleTheme);
                        cx.notify();
                    }),
                ))
                .child(rail_action_btn("rail_settings", "settings").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.run_command(AppCommand::OpenSettings);
                        cx.notify();
                    }),
                )),
        )
}

/// Renders the modular 236px left sidebar panel.
pub fn render_sidebar(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let tab = app.sidebar_tab;

    let left_title = if tab == SidebarTab::Files {
        let title = app.configs.vault_title();
        div()
            .id("sidebar_vault_switcher")
            .flex()
            .items_center()
            .gap(px(4.0))
            .max_w(px(136.0))
            .px(px(4.0))
            .py(px(2.0))
            .rounded(px(RADIUS_S))
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.tx)
                    .truncate()
                    .child(SharedString::from(title)),
            )
            .child(icon("selector", 12.0, t.tx3))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                    super::vaults_ui::open_menu(this, pos, window);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .into_any_element()
    } else {
        let label = match tab {
            SidebarTab::Files => String::new(),
            SidebarTab::Search => crate::t!("sidebar-search"),
            SidebarTab::Journal => crate::t!("sidebar-journal"),
            SidebarTab::Memos => crate::t!("sidebar-memos"),
            SidebarTab::Bookmarks => String::from("书签"),
        };
        div()
            .px(px(4.0))
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.tx)
            .truncate()
            .child(SharedString::from(label))
            .into_any_element()
    };

    let drag_spacer = div()
        .flex_1()
        .min_w(px(8.0))
        .h_full()
        .occlude()
        .window_control_area(WindowControlArea::Drag)
        .when(!cfg!(windows), |d| {
            d.on_mouse_down(MouseButton::Left, super::tabbar::title_click())
        });

    let mut right_actions = div().flex().items_center().flex_none().gap(px(2.0));
    if tab == SidebarTab::Files {
        right_actions = right_actions
            .child(icon_button_sized(
                &t,
                "file-plus",
                IconBtnSize::Small,
                cx.listener(|this, _, _, cx| {
                    this.run_command(AppCommand::NewNote);
                    cx.notify();
                }),
            ))
            .child(icon_button_sized(
                &t,
                "folder-plus",
                IconBtnSize::Small,
                cx.listener(|this, _, _, cx| {
                    files_ui::run(this, files_ui::Command::NewFolder, cx);
                }),
            ));
    }
    right_actions = right_actions.child(icon_button_sized(
        &t,
        "layout-sidebar",
        IconBtnSize::Small,
        cx.listener(|this, _, _, cx| {
            this.left_sidebar_open = false;
            crate::config_store::sync_ui(this);
            cx.notify();
        }),
    ));

    let header = div()
        .flex()
        .items_center()
        .flex_none()
        .h(px(HEADER_HEIGHT))
        .px(px(8.0))
        .gap(px(4.0))
        .border_b_1()
        .border_color(t.border_subtle)
        .child(left_title)
        .child(drag_spacer)
        .child(right_actions);

    let body: AnyElement = match tab {
        SidebarTab::Files => {
            let mut tree = div()
                .id("file_tree")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(120.0))
                .px(px(6.0))
                .py(px(6.0))
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

            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(tree)
                .child(
                    div()
                        .flex_none()
                        .px(px(10.0))
                        .pt(px(8.0))
                        .pb(px(4.0))
                        .border_t_1()
                        .border_color(t.border_subtle)
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.tx3)
                        .child(SharedString::from(crate::t!("sidebar-tags"))),
                )
                .child(
                    div().flex().flex_col().flex_none().child(
                        div()
                            .id("tag_list")
                            .flex()
                            .flex_col()
                            .max_h(px(TAG_BLOCK_MAX_H))
                            .px(px(6.0))
                            .pb(px(6.0))
                            .overflow_y_scroll()
                            .child(tags(app, cx)),
                    ),
                )
                .into_any_element()
        }
        SidebarTab::Search => super::search_ui::render_sidebar_panel(app, cx),
        SidebarTab::Journal => super::calendar::render_journal_panel(app, cx),
        SidebarTab::Memos => super::memos_ui::render_sidebar_panel(app, cx).into_any_element(),
        SidebarTab::Bookmarks => super::ext::bookmarks::render_sidebar_panel(app, cx),
    };

    div()
        .flex()
        .flex_col()
        .w(px(SIDEBAR_WIDTH))
        .flex_none()
        .h_full()
        .bg(t.panel)
        .border_r_1()
        .border_color(t.border_subtle)
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .text_color(t.tx2)
        .child(header)
        .children(super::vaults_ui::missing_banner(app, cx))
        .child(div().flex().flex_col().flex_1().min_h_0().child(body))
}

/// The 24px app logo (`assets/logo/icon-64.png`; its rounded corners are in the PNG alpha).
fn logo_mark() -> impl IntoElement {
    img("logo/icon-64.png").size(px(24.0)).flex_none()
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
                .text_color(t.danger)
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
                .h(px(ROW_HEIGHT))
                .pl(px(8.0 + depth as f32 * 20.0))
                .pr(px(8.0))
                .rounded(px(RADIUS_S))
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
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(ICON_GAP))
            .h(px(ROW_HEIGHT))
            .pl(px(8.0 + depth as f32 * 20.0))
            .pr(px(8.0))
            .rounded(px(RADIUS_S))
            .whitespace_nowrap()
            .cursor_pointer()
            .text_color(if is_active { t.tx } else { t.tx2 })
            .hover(|s| s.bg(t.hover).text_color(t.tx));
        if is_active {
            r = r.bg(t.active).font_weight(FontWeight::MEDIUM).child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(6.0))
                    .w(px(2.0))
                    .h(px(16.0))
                    .rounded_r(px(1.0))
                    .bg(t.ac),
            );
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
    // The active tags are those of the search text (sidebar search tab or modal search).
    let search_open = (app.left_sidebar_open && app.sidebar_tab == SidebarTab::Search)
        || app.ext.search.panel.open;
    let search_text = if search_open {
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
            .h(px(ROW_HEIGHT))
            .px(px(8.0))
            .rounded(px(RADIUS_S))
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

/// Runs the search for a clicked tag inside the modular Search sidebar tab. Ctrl+click adds the tag.
fn tag_clicked(app: &mut RustdianApp, name: &str, extend: bool, cx: &mut Context<RustdianApp>) {
    let open = (app.left_sidebar_open && app.sidebar_tab == SidebarTab::Search)
        || app.ext.search.panel.open;
    let current = if extend && open {
        super::search_ui::query_text(app, cx)
    } else {
        String::new()
    };
    let query = tag_query_text(&current, name, extend && open);
    app.viewport_mode = ViewportMode::Editor;
    app.sidebar_tab = SidebarTab::Search;
    app.left_sidebar_open = true;
    crate::config_store::sync_ui(app);
    super::search_ui::set_sidebar_query(app, &query, cx);
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
