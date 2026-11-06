//! Tab strips (42px title bar). In a split view each pane has its own strip: the left (or top)
//! strip shows the first pane's tabs, the other one the second pane's, and a 2px accent line
//! marks the strip of the focused pane. The empty space after the tabs drags the window and
//! double-click maximizes (OS behaviour); the caption buttons float over the right end
//! (`render_captions`), so the drag areas never cover a tab.
//!
//! A tab can be dragged along its strip to reorder it: a 2px marker shows the gap it would land
//! in, and the strip scrolls when the pointer rests near its ends. A tab dropped on the left or
//! right edge of the editor splits the view (`split::split_with_tab`). Ctrl+PageUp/PageDown
//! switch tabs and Ctrl+Shift+PageUp/PageDown move the active one (`tab_key`).

use super::files_ui::{self, Target};
use super::icons::icon;
use super::split;
use crate::RustdianApp;
use crate::session;
use crate::theme::ActiveTheme;
use gpui::{
    AnyElement, App, Bounds, Context, DragMoveEvent, ElementId, Entity, FontWeight, IntoElement,
    MouseButton, MouseDownEvent, Pixels, Point, Rgba, ScrollHandle, SharedString, Svg, Window,
    WindowControlArea, div, point, prelude::*, px, relative, rgb, svg,
};

const CAPTION_W: f32 = 46.0;
const CAPTION_H: f32 = 30.0;
/// Width of the three caption buttons; the strips leave this room at the right end.
const CAPTION_AREA_W: f32 = CAPTION_W * 3.0;
const CLOSE_HOVER: u32 = 0xe81123;
/// Linux draws its own title bar (client-side decorations), and gpui hit-tests the caption and
/// drag areas only on Windows. So there the buttons and the empty bar act on click instead.
const LINUX_TITLEBAR: bool = cfg!(target_os = "linux");
/// macOS keeps its native traffic lights at the top left: the strips start after them and the app
/// draws no caption buttons.
const MAC_TITLEBAR: bool = cfg!(target_os = "macos");
const MAC_TRAFFIC_LIGHT_INSET: f32 = 78.0;

/// Click action of a caption button on the platforms where the app handles clicks itself.
fn caption_click(
    area: WindowControlArea,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    move |_, window, _| match area {
        WindowControlArea::Min => window.minimize_window(),
        WindowControlArea::Max => window.zoom_window(),
        WindowControlArea::Close => window.remove_window(),
        _ => {}
    }
}

/// Click action of an empty title bar area: double click zooms, a press starts a window move on
/// Linux (where gpui does not do it for the drag area).
pub fn title_click() -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    |event, window, _| {
        if event.click_count >= 2 {
            window.zoom_window();
        } else if LINUX_TITLEBAR {
            window.start_window_move();
        }
    }
}
/// Distance from either end of a tab scroller at which a tab drag starts to scroll it.
const EDGE_PX: f32 = 32.0;
/// Pixels scrolled per drag move near an end.
const SCROLL_STEP: f32 = 12.0;

/// A tab being dragged. `pane` is the physical pane whose strip it belongs to, `from` its
/// position in that pane's tab list.
#[derive(Clone, Debug)]
pub struct TabDrag {
    pub pane: usize,
    pub from: usize,
    pub label: String,
    pub theme: ActiveTheme,
}

/// Drag state while a tab is dragged: its pane, its position and the gap it would land in
/// (`0..=len` in the pane's tab list; `None` while the pointer is not over a tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabDragState {
    pub pane: usize,
    pub from: usize,
    pub slot: Option<usize>,
}

/// The chip that follows the pointer while a tab is dragged.
pub struct TabChip {
    label: SharedString,
    theme: ActiveTheme,
}

impl gpui::Render for TabChip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .h(px(30.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(8.0))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .text_size(px(12.5))
            .text_color(t.tx)
            .child(self.label.clone())
    }
}

/// Builds the chip for a dragged tab.
pub fn tab_chip(
    value: &TabDrag,
    _pos: Point<Pixels>,
    _window: &mut Window,
    cx: &mut App,
) -> Entity<TabChip> {
    let label = SharedString::from(value.label.clone());
    let theme = value.theme;
    cx.new(|_| TabChip { label, theme })
}

/// Caption glyph. An `svg` only paints its own colour, so it is passed explicitly.
fn caption_icon(name: &str, color: Rgba) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(15.0))
        .flex_none()
        .text_color(color)
}

/// Minimize / maximize-restore button. The `area` makes the OS treat it as that control
/// (hit-test codes), so clicks come from the non-client path and hover is tracked by gpui.
fn caption_button(
    area: WindowControlArea,
    glyph: &str,
    t: &ActiveTheme,
) -> impl IntoElement + use<> {
    let hover_bg = t.hover;
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .w(px(CAPTION_W))
        .h(px(CAPTION_H))
        .occlude()
        .window_control_area(area)
        .when(LINUX_TITLEBAR, |d| {
            d.on_mouse_down(MouseButton::Left, caption_click(area))
        })
        .hover(move |s| s.bg(hover_bg))
        .child(caption_icon(glyph, t.tx2))
}

/// Close button: red background and a white glyph while hovered (`hovered` comes from
/// the app's `close_hovered` flag, set by `on_hover`).
fn close_button(
    hovered: bool,
    t: &ActiveTheme,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let glyph_color = if hovered { rgb(0xffffff) } else { t.tx2 };
    div()
        .id("caption_close")
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .w(px(CAPTION_W))
        .h(px(CAPTION_H))
        .occlude()
        .window_control_area(WindowControlArea::Close)
        .when(LINUX_TITLEBAR, |d| {
            d.on_mouse_down(MouseButton::Left, caption_click(WindowControlArea::Close))
        })
        .when(hovered, |d| d.bg(rgb(CLOSE_HOVER)))
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            this.close_hovered = *hovered;
            cx.notify();
        }))
        .child(caption_icon("x", glyph_color))
}

/// The window caption buttons (minimize, maximize-restore, close). They float over the right
/// end of the title bar, where the last strip leaves room for them.
pub fn render_captions(
    app: &RustdianApp,
    window: &Window,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let max_glyph = if window.is_maximized() {
        "copy"
    } else {
        "square"
    };
    div()
        .absolute()
        .top_0()
        .right(px(8.0))
        .h(px(42.0))
        .flex()
        .flex_row()
        .items_start()
        .pt(px(6.0))
        .when(!MAC_TITLEBAR, |d| {
            d.child(caption_button(WindowControlArea::Min, "minus", &t))
                .child(caption_button(WindowControlArea::Max, max_glyph, &t))
                .child(close_button(app.close_hovered, &t, cx))
        })
}

/// Renders the title bar: one strip per pane (one when not split). The strips span the editor
/// column, so each strip sits over its pane; above the right panel a drag area takes the room.
pub fn render_tabbar(
    app: &RustdianApp,
    _window: &Window,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let ratio = app.layout.split.as_ref().map(|s| s.ratio);
    let count = if ratio.is_some() { 2 } else { 1 };
    let right_open = app.right_sidebar_open;
    let mut strips = div().flex().flex_row().flex_1().min_w_0().h_full();
    for pane in 0..count {
        // The caption buttons sit over the right panel's column when it is open (see below).
        let reserve_captions = pane + 1 == count && !right_open;
        strips = strips.child(render_strip(app, pane, ratio, reserve_captions, cx));
    }
    let mut row = div()
        .flex()
        .flex_row()
        .flex_none()
        .w_full()
        .h(px(42.0))
        .pr(px(8.0))
        .when(MAC_TITLEBAR, |d| d.pl(px(MAC_TRAFFIC_LIGHT_INSET)))
        .bg(t.chrome)
        .child(strips);
    if right_open {
        row = row.child(
            div()
                .flex_none()
                .w(px(super::RIGHT_PANEL_W))
                .mt(px(6.0))
                .h(px(36.0))
                .occlude()
                .window_control_area(WindowControlArea::Drag)
                .when(!cfg!(windows), |d| {
                    d.on_mouse_down(MouseButton::Left, title_click())
                }),
        );
    }
    row
}

/// One pane's strip: the tabs (a scroller as wide as its content, at most the strip) and the
/// drag area that takes the rest. `reserve_captions` leaves the caption buttons' room at its end.
fn render_strip(
    app: &RustdianApp,
    pane: usize,
    split_ratio: Option<f32>,
    reserve_captions: bool,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let focused = split::focused(app);
    let (tab_list, active_pos) = split::pane_tabs(app, pane);
    let slot = app
        .layout
        .tab_drag
        .filter(|d| d.pane == pane)
        .and_then(|d| d.slot);

    // The tabs are direct children of the scroller, so gpui can scroll the active one into view
    // (`scroll_to_item`). `active_child` is its index among those children.
    let mut children: Vec<AnyElement> = Vec::new();
    let mut active_child = None;
    for (pos, &note_idx) in tab_list.iter().enumerate() {
        if slot == Some(pos) {
            children.push(insertion_marker(&t).into_any_element());
        }
        if pos == active_pos {
            active_child = Some(children.len());
        }
        children.push(tab_el(app, pane, pos, note_idx, pos == active_pos, cx));
    }
    if slot == Some(tab_list.len()) {
        children.push(insertion_marker(&t).into_any_element());
    }
    children.push(
        div()
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .size(px(30.0))
            .mb(px(2.0))
            .ml(px(4.0))
            .rounded(px(8.0))
            .text_color(t.tx3)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover).text_color(t.tx2))
            .child(icon("plus", 16.0, t.tx3))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.new_note();
                    cx.notify();
                }),
            )
            .into_any_element(),
    );

    // Keep the active tab in view when it changed or the strip was resized since the last render.
    let handle = app.layout.strips[pane].clone();
    if let Some(child) = active_child {
        let active_note = tab_list.get(active_pos).copied().unwrap_or(usize::MAX);
        let width = f32::from(handle.bounds().size.width).round().max(0.0) as u32;
        let count = children.len();
        let prev = app.layout.strip_seen[pane].replace(Some((active_note, width, count)));
        let moved = prev.map(|(n, w, _)| (n, w)) != Some((active_note, width));
        if moved {
            // The item bounds of the last frame match the children only when their number did not
            // change; otherwise gpui reveals the tab once this frame's layout is known.
            let same_layout = prev.is_some_and(|(_, _, c)| c == count);
            if !(same_layout && reveal_child(&handle, child)) {
                handle.scroll_to_item(child);
            }
        }
    }

    // The scroller: as wide as its tabs, shrinking (and scrolling) when they do not fit.
    let scroller = div()
        .id(ElementId::NamedInteger(
            SharedString::from("tabs"),
            pane as u64,
        ))
        .flex()
        .flex_row()
        .items_end()
        .gap(px(2.0))
        .flex_shrink()
        .min_w_0()
        .mt(px(6.0))
        .h(px(36.0))
        .font_family(crate::ui::fonts::ui_name())
        .overflow_x_scroll()
        .track_scroll(&handle)
        .on_drag_move::<TabDrag>(
            cx.listener(move |this, ev: &DragMoveEvent<TabDrag>, _, cx| {
                let (dpane, dfrom) = {
                    let d = ev.drag(cx);
                    (d.pane, d.from)
                };
                if dpane != pane {
                    return;
                }
                if this.layout.tab_drag.is_none() {
                    this.layout.tab_drag = Some(TabDragState {
                        pane,
                        from: dfrom,
                        slot: None,
                    });
                }
                if autoscroll(this, pane, ev.event.position.x, ev.bounds) {
                    cx.notify();
                }
            }),
        )
        .children(children);

    // Chevrons at the ends of the strip while its tabs overflow it (the wheel scrolls it too).
    let scroll_max = f32::from(handle.max_offset().width).max(0.0);
    let scrolled = -f32::from(handle.offset().x);
    let (more_left, more_right) = overflow_hints(scrolled, scroll_max);
    let chevron = |forward: bool| {
        div()
            .id(ElementId::NamedInteger(
                SharedString::from(if forward { "tabs-next" } else { "tabs-prev" }),
                pane as u64,
            ))
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .w(px(20.0))
            .h(px(36.0))
            .mt(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(icon(
                if forward {
                    "chevron-right"
                } else {
                    "chevron-left"
                },
                13.0,
                t.tx3,
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    scroll_strip(this, pane, forward);
                    cx.notify();
                }),
            )
    };

    // The drag area: the empty title bar. It stays clear of the tabs (window control areas are
    // hit-tested apart from the elements), and the last strip leaves the caption buttons' room.
    let drag_area = div()
        .flex_1()
        .min_w_0()
        .mt(px(6.0))
        .h(px(36.0))
        .occlude()
        .window_control_area(WindowControlArea::Drag)
        .when(!cfg!(windows), |d| {
            d.on_mouse_down(MouseButton::Left, title_click())
        })
        .when(reserve_captions && !MAC_TITLEBAR, |d| {
            d.mr(px(CAPTION_AREA_W))
        });

    let mut wrap = div()
        .id(ElementId::NamedInteger(
            SharedString::from("strip"),
            pane as u64,
        ))
        .relative()
        .flex()
        .flex_row()
        .min_w_0()
        .h_full()
        .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
            let state = this.layout.tab_drag.take();
            if drag.pane == pane
                && let Some(slot) = state.and_then(|s| s.slot)
            {
                move_tab(this, pane, drag.from, slot);
            }
            cx.notify();
        }))
        .when(more_left, |d| d.child(chevron(false)))
        .child(scroller)
        .when(more_right, |d| d.child(chevron(true)))
        .child(drag_area);
    wrap = match split_ratio {
        Some(r) if pane == 0 => wrap.flex_none().w(relative(r)),
        Some(_) => wrap.flex_1(),
        None => wrap.flex_1(),
    };
    if split_ratio.is_some() && pane == focused {
        wrap = wrap.child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(2.0))
                .bg(t.ac),
        );
    }
    wrap.into_any_element()
}

/// The 2px accent bar that shows where a dragged tab would land.
fn insertion_marker(t: &ActiveTheme) -> impl IntoElement + use<> {
    div()
        .flex_none()
        .w(px(2.0))
        .h(px(22.0))
        .mb(px(4.0))
        .rounded(px(1.0))
        .bg(t.ac)
}

/// One tab. The pane is focused first, so the tab's position refers to the vault's tab list.
fn tab_el(
    app: &RustdianApp,
    pane: usize,
    pos: usize,
    note_idx: usize,
    on: bool,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let Some(note) = app.vault.notes.get(note_idx) else {
        return div().into_any_element();
    };
    let is_daily = app.vault.is_daily_note(note);
    let title = note.title.clone();
    let icon_name = if is_daily {
        "calendar-event"
    } else {
        "file-text"
    };
    let dragging = app
        .layout
        .tab_drag
        .is_some_and(|d| d.pane == pane && d.from == pos);

    let mut tab = div()
        .id(ElementId::NamedInteger(
            SharedString::from(format!("tab{pane}")),
            pos as u64,
        ))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .h(px(34.0))
        .max_w(px(176.0))
        .pl(px(12.0))
        .pr(px(10.0))
        .rounded_t(px(9.0))
        .text_size(px(12.5))
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(if on { t.tx } else { t.tx3 })
        .hover(|s| s.bg(t.hover).text_color(t.tx2))
        .when(dragging, |d| d.opacity(0.45))
        .child(icon(icon_name, 15.0, if on { t.ac } else { t.tx3 }))
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(SharedString::from(title.clone())),
        );

    // Close slot. A dirty note shows a dot there (any tab); the x replaces it on hover.
    // The active clean tab always shows the x.
    let dirty = app.is_dirty(&note.path, cx);
    if on {
        tab = tab.bg(t.sheet).font_weight(FontWeight::MEDIUM);
    }
    if on || dirty {
        let group = SharedString::from(format!("tab-close-{pane}-{pos}"));
        let x_always = on && !dirty;
        let mut slot = div()
            .id(ElementId::NamedInteger(
                SharedString::from(format!("close_tab{pane}")),
                pos as u64,
            ))
            .group(group.clone())
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .ml(px(6.0))
            .size(px(16.0))
            .rounded(px(4.0))
            .hover(|s| s.bg(t.hover))
            .child(
                div()
                    .when(!x_always, |d| {
                        d.invisible().group_hover(group.clone(), |s| s.visible())
                    })
                    .child(icon("x", 13.0, t.tx2)),
            );
        if dirty {
            slot = slot.child(
                div()
                    .absolute()
                    .left(px(5.0))
                    .top(px(5.0))
                    .size(px(6.0))
                    .rounded_full()
                    .bg(t.tx2)
                    .group_hover(group.clone(), |s| s.invisible()),
            );
        }
        slot = slot.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                split::focus_pane(this, pane);
                split::close_tab(this, pos);
                cx.notify();
            }),
        );
        tab = tab.child(slot);
    }

    let drag = TabDrag {
        pane,
        from: pos,
        label: title,
        theme: t,
    };
    tab = tab
        .on_drag(drag, tab_chip)
        .on_drag_move::<TabDrag>(
            cx.listener(move |this, ev: &DragMoveEvent<TabDrag>, _, cx| {
                if !ev.bounds.contains(&ev.event.position) {
                    return;
                }
                let dpane = ev.drag(cx).pane;
                if dpane != pane {
                    return;
                }
                // The left half of a tab is the gap in front of it, the right half the gap after.
                let mid = ev.bounds.center();
                let slot = if ev.event.position.x < mid.x {
                    pos
                } else {
                    pos + 1
                };
                if let Some(state) = this.layout.tab_drag.as_mut()
                    && state.pane == pane
                    && state.slot != Some(slot)
                {
                    state.slot = Some(slot);
                    cx.notify();
                }
            }),
        );
    // A click on the tab shows it. Middle click closes it; right click opens its menu (close,
    // close others and the right-hand tabs, copy path, reveal in Explorer).
    tab = tab
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                split::focus_pane(this, pane);
                activate_tab(this, pos);
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _, _, cx| {
                split::focus_pane(this, pane);
                split::close_tab(this, pos);
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                split::focus_pane(this, pane);
                let p = (f32::from(ev.position.x), f32::from(ev.position.y));
                files_ui::open_menu(this, Target::Tab(pos), p, window);
                cx.notify();
            }),
        );
    tab.into_any_element()
}

/// Scrolls the tab scroller of `pane` while a dragged pointer rests near one of its ends. Returns
/// true when the scroll position changed.
fn autoscroll(app: &RustdianApp, pane: usize, x: Pixels, bounds: Bounds<Pixels>) -> bool {
    let left = f32::from(x - bounds.origin.x);
    let width = f32::from(bounds.size.width);
    let step = if left < EDGE_PX {
        SCROLL_STEP
    } else if left > width - EDGE_PX {
        -SCROLL_STEP
    } else {
        return false;
    };
    let handle = &app.layout.strips[pane];
    let off = handle.offset();
    let max = f32::from(handle.max_offset().width);
    let next = (f32::from(off.x) + step).clamp(-max, 0.0);
    if (next - f32::from(off.x)).abs() < 0.5 {
        return false;
    }
    handle.set_offset(point(px(next), off.y));
    true
}

/// Space kept between a revealed tab and the edge of the strip.
pub const REVEAL_MARGIN: f32 = 24.0;

/// Scroll amount that shows the content range `start..end` in a viewport of `viewport` px, keeping
/// `margin` px clear at both edges. `current` is the scroll amount now and `max` the most the strip
/// can scroll. A range that is wider than the viewport shows its start.
pub fn reveal_scroll(
    current: f32,
    viewport: f32,
    start: f32,
    end: f32,
    margin: f32,
    max: f32,
) -> f32 {
    let want_start = start - margin;
    let want_end = end + margin;
    let next = if want_start < current || want_end - want_start > viewport {
        want_start
    } else if want_end > current + viewport {
        want_end - viewport
    } else {
        current
    };
    next.clamp(0.0, max.max(0.0))
}

/// Scrolls the strip so that the child `child` is in view with `REVEAL_MARGIN`. Uses the item bounds
/// of the last frame, so the caller must know that the children did not change. Returns false when
/// the bounds are not known.
fn reveal_child(handle: &ScrollHandle, child: usize) -> bool {
    let Some(item) = handle.bounds_for_item(child) else {
        return false;
    };
    let view = handle.bounds();
    let offset = handle.offset();
    let current = -f32::from(offset.x);
    let max = f32::from(handle.max_offset().width);
    // The item's position in the viewport moves with the scroll: content = viewport + scroll.
    let start = f32::from(item.left() - view.left()) + current;
    let end = start + f32::from(item.size.width);
    let next = reveal_scroll(
        current,
        f32::from(view.size.width),
        start,
        end,
        REVEAL_MARGIN,
        max,
    );
    if (next - current).abs() > 0.5 {
        handle.set_offset(point(px(-next), offset.y));
    }
    true
}

/// Pixels a chevron scrolls the strip by.
const STRIP_STEP: f32 = 160.0;

/// Scroll amount after moving by `delta` (negative goes left), kept between 0 and `max` (the most
/// the strip can scroll: its content width minus its width).
pub fn scroll_by(current: f32, delta: f32, max: f32) -> f32 {
    (current + delta).clamp(0.0, max.max(0.0))
}

/// Which chevrons are shown for a scroll amount: (more to the left, more to the right).
pub fn overflow_hints(current: f32, max: f32) -> (bool, bool) {
    let max = max.max(0.0);
    (current > 0.5, current < max - 0.5)
}

/// Scrolls the strip of `pane` one step to the left or right (the chevrons).
fn scroll_strip(app: &RustdianApp, pane: usize, forward: bool) {
    let handle = &app.layout.strips[pane];
    let offset = handle.offset();
    let max = f32::from(handle.max_offset().width);
    let delta = if forward { STRIP_STEP } else { -STRIP_STEP };
    let next = scroll_by(-f32::from(offset.x), delta, max);
    handle.set_offset(point(px(-next), offset.y));
}

/// Makes the tab at `pos` of the focused pane the active one.
pub fn activate_tab(app: &mut RustdianApp, pos: usize) {
    if let Some(&note_idx) = app.vault.open_tabs.get(pos) {
        app.vault.active_tab_index = pos;
        app.vault.active_note_index = note_idx;
    }
}

/// Moves the tab at `from` of `pane` to the gap `slot` of that pane. The pane is focused first.
fn move_tab(app: &mut RustdianApp, pane: usize, from: usize, slot: usize) {
    split::focus_pane(app, pane);
    let len = app.vault.open_tabs.len();
    let Some(to) = session::reorder_dest(len, from, slot) else {
        return;
    };
    let active = app.vault.active_tab_index;
    if session::move_item(&mut app.vault.open_tabs, from, to) {
        app.vault.active_tab_index = session::active_after_move(active, from, to);
        if let Some(&idx) = app.vault.open_tabs.get(app.vault.active_tab_index) {
            app.vault.active_note_index = idx;
        }
    }
}

/// Ctrl+PageUp / Ctrl+PageDown (`moving == false`) switch to the previous / next tab of the
/// focused pane, wrapping around. With Shift (`moving == true`) the active tab itself moves one
/// place, and stops at the ends.
pub fn tab_key(app: &mut RustdianApp, forward: bool, moving: bool) {
    let len = app.vault.open_tabs.len();
    let active = app.vault.active_tab_index;
    if moving {
        if let Some(to) = session::step_index(len, active, forward)
            && session::move_item(&mut app.vault.open_tabs, active, to)
        {
            app.vault.active_tab_index = to;
        }
    } else if let Some(next) = session::cycle_index(len, active, forward) {
        activate_tab(app, next);
    }
}

#[cfg(test)]
mod tests {
    use super::{overflow_hints, reveal_scroll, scroll_by};

    #[test]
    fn reveal_keeps_the_margin_at_the_left_edge() {
        // Tab at 200..260, scrolled to 180: the tab starts at 20 px, inside the margin.
        assert_eq!(
            reveal_scroll(180.0, 300.0, 200.0, 260.0, 24.0, 900.0),
            176.0
        );
    }

    #[test]
    fn reveal_keeps_the_margin_at_the_right_edge() {
        // Tab at 380..440 in a 300 px view scrolled to 0: it ends past the edge less the margin.
        assert_eq!(reveal_scroll(0.0, 300.0, 380.0, 440.0, 24.0, 900.0), 164.0);
    }

    #[test]
    fn visible_tab_does_not_move() {
        assert_eq!(
            reveal_scroll(100.0, 300.0, 200.0, 260.0, 24.0, 900.0),
            100.0
        );
    }

    #[test]
    fn reveal_is_clamped_to_the_strip() {
        // The first tab: the margin would go below zero.
        assert_eq!(reveal_scroll(50.0, 300.0, 0.0, 60.0, 24.0, 900.0), 0.0);
        // The last tab: the margin would go past the strip's end.
        assert_eq!(reveal_scroll(0.0, 300.0, 880.0, 940.0, 24.0, 620.0), 620.0);
    }

    #[test]
    fn wide_tab_shows_its_start() {
        assert_eq!(reveal_scroll(0.0, 100.0, 300.0, 600.0, 24.0, 900.0), 276.0);
    }

    #[test]
    fn scroll_stays_within_the_strip() {
        assert_eq!(scroll_by(0.0, -160.0, 500.0), 0.0);
        assert_eq!(scroll_by(0.0, 160.0, 500.0), 160.0);
        assert_eq!(scroll_by(400.0, 160.0, 500.0), 500.0);
    }

    #[test]
    fn no_overflow_means_no_scrolling() {
        assert_eq!(scroll_by(0.0, 160.0, 0.0), 0.0);
        assert_eq!(overflow_hints(0.0, 0.0), (false, false));
    }

    #[test]
    fn chevrons_follow_the_scroll_position() {
        assert_eq!(overflow_hints(0.0, 300.0), (false, true));
        assert_eq!(overflow_hints(150.0, 300.0), (true, true));
        assert_eq!(overflow_hints(300.0, 300.0), (true, false));
    }
}
