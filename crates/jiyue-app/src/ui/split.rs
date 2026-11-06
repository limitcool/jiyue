//! Split view (分屏): two panes side by side (向右拆分) or stacked (向下拆分), the divider between
//! them, the focus, and the workspace file (`<vault>/.jiyue/workspace.json`) that keeps the tab
//! order, the active tabs and the split across starts.
//!
//! The focused pane's tabs live in `vault.open_tabs` / `active_tab_index`, so the breadcrumb, the
//! right panel, the status bar, the palette and the sidebar follow the focused note without any
//! change. The other pane is kept as note paths in `SplitState::other` and is swapped into the
//! vault when the focus moves (`focus_pane`). A note open in both panes has one editor (one
//! buffer, one undo history, one dirty flag): the focused pane edits it and the other pane shows
//! its rendered text, which follows every edit.

use std::path::PathBuf;

use gpui::{
    AnyElement, Context, DragMoveEvent, IntoElement, MouseDownEvent, Render, ScrollHandle, Window,
    div, prelude::*, px, relative,
};

use gpui::Focusable;

use super::editor;
use super::split_model::{
    self as model, CloseOutcome, DIVIDER_PX, PaneState, SplitDir, SplitState,
};
use super::tabbar::{TabDrag, TabDragState};
use crate::RustdianApp;
use crate::ViewportMode;
use crate::session::{SplitFile, WORKSPACE_VERSION, WorkspaceFile};

/// Section of `<vault>/.jiyue/state.toml` that holds the layout.
const WORKSPACE_SECTION: &str = "workspace";

/// Layout state of the window. Lives in `RustdianApp::layout`.
#[derive(Default)]
pub struct Layout {
    /// The second pane, when the view is split.
    pub split: Option<SplitState>,
    /// Scroll position of each pane's tab strip (index = physical pane).
    pub strips: [ScrollHandle; 2],
    /// A tab being dragged, and the gap it would land in.
    pub tab_drag: Option<TabDragState>,
    /// The workspace file text as last written, so an unchanged layout is not written again.
    saved: String,
}

/// Blank drag preview for the divider (the divider itself is the only feedback).
struct Blank;

impl Render for Blank {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_0()
    }
}

/// What the divider carries while it is dragged. The direction comes from the split itself.
#[derive(Clone, Copy, Debug)]
pub struct DividerDrag;

/// Physical pane with the keyboard focus (`0` when there is no split).
pub fn focused(app: &RustdianApp) -> usize {
    app.layout.split.as_ref().map_or(0, |s| s.focused)
}

/// The focused pane's tabs as they are now in the vault.
fn vault_pane(app: &RustdianApp) -> PaneState {
    let tabs: Vec<String> = app
        .vault
        .open_tabs
        .iter()
        .filter_map(|&i| app.vault.notes.get(i))
        .map(|n| n.path.clone())
        .collect();
    let active = app
        .vault
        .open_tabs
        .get(app.vault.active_tab_index)
        .and_then(|&i| app.vault.notes.get(i))
        .map(|n| n.path.clone());
    PaneState { tabs, active }
}

/// Puts `pane` into the vault as the focused pane. False (nothing changed) when none of its
/// notes exists.
fn load_vault_pane(app: &mut RustdianApp, pane: &PaneState) -> bool {
    let idx: Vec<usize> = pane.tabs.iter().filter_map(|p| app.note_index(p)).collect();
    if idx.is_empty() {
        return false;
    }
    let pos = model::active_position(pane).min(idx.len() - 1);
    app.vault.open_tabs = idx;
    app.vault.active_tab_index = pos;
    app.vault.active_note_index = app.vault.open_tabs[pos];
    true
}

/// The tabs of physical `pane` as note indices, and the position of its active tab.
pub fn pane_tabs(app: &RustdianApp, pane: usize) -> (Vec<usize>, usize) {
    match app.layout.split.as_ref() {
        Some(s) if s.focused != pane => {
            let idx = s
                .other
                .tabs
                .iter()
                .filter_map(|p| app.note_index(p))
                .collect();
            (idx, model::active_position(&s.other))
        }
        _ => (app.vault.open_tabs.clone(), app.vault.active_tab_index),
    }
}

/// Makes physical `pane` the focused one: the two panes' tab states swap between the vault and
/// the stored pane. Nothing changes when the pane already has the focus.
pub fn focus_pane(app: &mut RustdianApp, pane: usize) {
    let Some(split) = app.layout.split.as_ref() else {
        return;
    };
    if pane > 1 || split.focused == pane {
        return;
    }
    let other = split.other.clone();
    let current = vault_pane(app);
    if !load_vault_pane(app, &other) {
        return;
    }
    if let Some(split) = app.layout.split.as_mut() {
        split.other = current;
        split.focused = pane;
    }
    app.want_focus = true;
}

/// Moves the focus to the neighbouring pane (Ctrl+Alt+arrows). Forward is right or down.
pub fn step_focus(app: &mut RustdianApp, forward: bool) {
    let Some(split) = app.layout.split.as_ref() else {
        return;
    };
    if let Some(next) = model::neighbour(split.focused, forward) {
        focus_pane(app, next);
    }
}

/// 向右拆分 / 向下拆分: a second pane with the same tabs takes the focus. When the view is
/// already split nothing changes and a toast says so (close the split first).
pub fn split(app: &mut RustdianApp, dir: SplitDir) {
    if app.layout.split.is_some() {
        app.toast(crate::ui::ext::ToastKind::Info, "已经是分屏");
        return;
    }
    if app.vault.open_tabs.is_empty() {
        return;
    }
    let current = vault_pane(app);
    app.layout.split = Some(SplitState {
        dir,
        ratio: 0.5,
        focused: 1,
        other: current,
    });
    app.want_focus = true;
}

/// Splits the tab at `pos` of the single pane into a pane of its own. The new pane holds only
/// that tab and takes the focus; it is on the left (or top) when `new_first`. Used when a tab is
/// dropped on the editor's left or right edge. False when the split is not possible.
pub fn split_with_tab(app: &mut RustdianApp, pos: usize, new_first: bool) -> bool {
    if app.layout.split.is_some() {
        return false;
    }
    let vault = vault_pane(app);
    let Some((new_pane, rest, _)) = model::detach_tab(&vault.tabs, app.vault.active_tab_index, pos)
    else {
        return false;
    };
    if !load_vault_pane(app, &new_pane) {
        return false;
    }
    app.layout.split = Some(SplitState {
        dir: SplitDir::Right,
        ratio: 0.5,
        focused: if new_first { 0 } else { 1 },
        other: rest,
    });
    app.want_focus = true;
    true
}

/// 关闭分屏: the focused pane stays, the other one closes (its notes stay in the vault).
pub fn close_split(app: &mut RustdianApp) {
    if app.layout.split.take().is_some() {
        app.want_focus = true;
    }
}

/// Splits when there is no split, closes it otherwise (Ctrl+\ and the old toggle command).
pub fn toggle(app: &mut RustdianApp) {
    if app.layout.split.is_some() {
        close_split(app);
    } else {
        split(app, SplitDir::Right);
    }
}

/// The focused pane lost its last tab: the pane closes and the other pane fills the view.
/// Returns false when there is no split or the other pane has no note left.
pub fn close_focused_pane(app: &mut RustdianApp) -> bool {
    let Some(split) = app.layout.split.take() else {
        return false;
    };
    if load_vault_pane(app, &split.other) {
        app.want_focus = true;
        return true;
    }
    app.layout.split = Some(split);
    false
}

/// Closes a tab of the focused pane, applying the closing rule for splits.
pub fn close_tab(app: &mut RustdianApp, pos: usize) {
    let left = app.vault.open_tabs.len().saturating_sub(1);
    let split = app.layout.split.is_some();
    if model::after_tab_close(split, left) == CloseOutcome::ClosePane
        && pos < app.vault.open_tabs.len()
        && close_focused_pane(app)
    {
        return;
    }
    let active_note = app.vault.active_note_index;
    app.vault.close_tab(pos);
    if let Some(p) = app.vault.open_tabs.iter().position(|&i| i == active_note) {
        app.vault.active_tab_index = p;
        app.vault.active_note_index = active_note;
    }
}

/// Runs at the start of every render: drops the other pane's notes that no longer exist (a pane
/// without notes closes) and writes the workspace file when the layout changed.
pub fn sync(app: &mut RustdianApp) {
    let mut empty = false;
    if let Some(split) = app.layout.split.as_mut() {
        let notes = &app.vault.notes;
        empty = model::prune_pane(&mut split.other, |p| notes.iter().any(|n| n.path == p));
    }
    if empty {
        app.layout.split = None;
        app.want_focus = true;
    }
    save(app);
}

/// The layout as it is saved: the focused pane's tabs, and the split when there is one.
pub fn snapshot(app: &RustdianApp) -> WorkspaceFile {
    let vault = vault_pane(app);
    WorkspaceFile {
        version: WORKSPACE_VERSION,
        tabs: vault.tabs,
        active: vault.active,
        split: app.layout.split.as_ref().map(|s| SplitFile {
            down: s.dir == SplitDir::Down,
            ratio: s.ratio,
            focused: s.focused,
            other_tabs: s.other.tabs.clone(),
            other_active: s.other.active.clone(),
        }),
    }
}

fn workspace_path(app: &RustdianApp) -> PathBuf {
    jiyue_core::config::vault_file(&app.vault.root_dir, jiyue_core::config::VAULT_STATE_FILE)
}

/// Writes the layout when it differs from the last write. Written to a temporary file first, so
/// a crash never leaves half a file. Failures are ignored: the layout is only a convenience.
fn save(app: &mut RustdianApp) {
    let file = snapshot(app);
    let text = toml::to_string(&file).unwrap_or_default();
    if text == app.layout.saved {
        return;
    }
    let _ = jiyue_core::config::save_section(
        &workspace_path(app),
        WORKSPACE_SECTION,
        &file,
        jiyue_core::config::templates::VAULT_STATE,
        &["workspace.split"],
    );
    app.layout.saved = text;
}

/// Restores the tabs (and the split) saved by the last run. False when there is no usable file,
/// and the caller keeps its default tabs.
pub fn restore(app: &mut RustdianApp) -> bool {
    let loaded: jiyue_core::config::Loaded<WorkspaceFile> =
        jiyue_core::config::load_section(&workspace_path(app), WORKSPACE_SECTION);
    if loaded.error.is_some() {
        return false;
    }
    let file = loaded.value;
    if file.tabs.is_empty() {
        return false;
    }
    let mut vault = PaneState {
        tabs: file.tabs,
        active: file.active,
    };
    let notes = &app.vault.notes;
    if model::prune_pane(&mut vault, |p| notes.iter().any(|n| n.path == p)) {
        return false;
    }
    if !load_vault_pane(app, &vault) {
        return false;
    }
    let split = file.split.and_then(|sf| {
        let mut other = PaneState {
            tabs: sf.other_tabs,
            active: sf.other_active,
        };
        let notes = &app.vault.notes;
        if model::prune_pane(&mut other, |p| notes.iter().any(|n| n.path == p)) {
            return None;
        }
        Some(SplitState {
            dir: if sf.down {
                SplitDir::Down
            } else {
                SplitDir::Right
            },
            ratio: if sf.ratio.is_finite() {
                sf.ratio.clamp(0.1, 0.9)
            } else {
                0.5
            },
            focused: sf.focused.min(1),
            other,
        })
    });
    app.layout.split = split;
    true
}

/// The editor slot below the tab strips: the graph view, the single note with its edge drop
/// zones, or the two panes with the divider.
pub fn render_editor_area(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    if app.viewport_mode == ViewportMode::GraphView {
        return div()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(app.render_graph_view(cx))
            .into_any_element();
    }
    let Some(split) = app.layout.split.as_ref() else {
        let Some(note) = app.vault.active_note() else {
            return div().flex_1().into_any_element();
        };
        let editor = editor::render_editor(app, note, true, cx);
        // Dragging a tab onto the left or right edge of the editor splits the view.
        return div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(editor)
            .child(edge_zone(app, true, cx))
            .child(edge_zone(app, false, cx))
            .into_any_element();
    };

    let dir = split.dir;
    let ratio = split.ratio;
    let focused_pane = split.focused;
    let focused_el = match app.vault.active_note() {
        Some(note) => editor::render_editor(app, note, true, cx).into_any_element(),
        None => div().flex_1().into_any_element(),
    };
    let other_note = split
        .other
        .active
        .as_ref()
        .and_then(|p| app.vault.notes.iter().find(|n| &n.path == p));
    let other_el = match other_note {
        Some(note) => editor::render_secondary(app, note, cx).into_any_element(),
        None => div().flex_1().into_any_element(),
    };
    let (first, second) = if focused_pane == 0 {
        (focused_el, other_el)
    } else {
        (other_el, focused_el)
    };
    let t = app.theme;
    let down = dir == SplitDir::Down;

    let first_pane = div()
        .id(gpui::ElementId::NamedInteger("split_pane".into(), 0))
        .relative()
        .flex_none()
        .overflow_hidden()
        .when(!down, |d| d.w(relative(ratio)).h_full())
        .when(down, |d| d.h(relative(ratio)).w_full())
        .when(focused_pane != 0, |d| {
            d.capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                focus_pane(this, 0);
                cx.notify();
            }))
        })
        .child(first);
    let second_pane = div()
        .id(gpui::ElementId::NamedInteger("split_pane".into(), 1))
        .relative()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .when(!down, |d| d.h_full())
        .when(down, |d| d.w_full())
        .when(focused_pane != 1, |d| {
            d.capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                focus_pane(this, 1);
                cx.notify();
            }))
        })
        .child(second);
    // A click in the focused pane gives the keyboard to its editor (the editor takes the focus
    // itself too; this keeps it there whatever the click landed on).
    let editor_focus = app
        .active_path()
        .and_then(|p| app.editor_for(p))
        .map(|e| e.read(cx).focus_handle(cx));
    let (first_pane, second_pane) = if focused_pane == 0 {
        (
            first_pane.on_mouse_down(gpui::MouseButton::Left, move |_, window, _| {
                if let Some(h) = &editor_focus {
                    window.focus(h);
                }
            }),
            second_pane,
        )
    } else {
        (
            first_pane,
            second_pane.on_mouse_down(gpui::MouseButton::Left, move |_, window, _| {
                if let Some(h) = &editor_focus {
                    window.focus(h);
                }
            }),
        )
    };
    let divider = div()
        .id("split_divider")
        .flex_none()
        .when(!down, |d| d.w(px(DIVIDER_PX)).h_full().cursor_col_resize())
        .when(down, |d| d.h(px(DIVIDER_PX)).w_full().cursor_row_resize())
        .hover(move |s| s.bg(t.ac_soft))
        .on_drag(DividerDrag, |_, _, _, cx| cx.new(|_| Blank));

    div()
        .id("split_container")
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .when(!down, |d| d.flex_row())
        .when(down, |d| d.flex_col())
        .on_drag_move::<DividerDrag>(cx.listener(
            move |this, ev: &DragMoveEvent<DividerDrag>, _, cx| {
                let Some(dir) = this.layout.split.as_ref().map(|s| s.dir) else {
                    return;
                };
                let (offset, total) = match dir {
                    SplitDir::Right => (
                        f32::from(ev.event.position.x - ev.bounds.origin.x),
                        f32::from(ev.bounds.size.width),
                    ),
                    SplitDir::Down => (
                        f32::from(ev.event.position.y - ev.bounds.origin.y),
                        f32::from(ev.bounds.size.height),
                    ),
                };
                let next = model::ratio_from_offset(offset - DIVIDER_PX / 2.0, total);
                if let Some(s) = this.layout.split.as_mut() {
                    s.ratio = next;
                }
                cx.notify();
            },
        ))
        .child(first_pane)
        .child(divider)
        .child(second_pane)
        .into_any_element()
}

/// The transparent strip at the left (`left`) or right edge of the editor that takes a dropped
/// tab. It shows a tint while a tab is dragged over it.
fn edge_zone(
    app: &RustdianApp,
    left: bool,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let zone = div()
        .id(if left {
            "split_zone_left"
        } else {
            "split_zone_right"
        })
        .absolute()
        .top_0()
        .bottom_0()
        .w(relative(0.14))
        .drag_over::<TabDrag>(move |s, _, _, _| s.bg(t.ac_soft))
        .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
            this.layout.tab_drag = None;
            if this.layout.split.is_none() && drag.pane == 0 {
                split_with_tab(this, drag.from, left);
            }
            cx.notify();
        }));
    if left { zone.left_0() } else { zone.right_0() }
}
