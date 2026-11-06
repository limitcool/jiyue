//! Overlays: modal dialog, confirm dialog, context menu, popover placement and tooltip.
//!
//! Owners render these inside `ui::ext::overlays` (or wherever an overlay must sit above the
//! page). They are plain functions. The owner keeps the open state and calls the close handler.
//!
//! Escape: these elements do not get keyboard focus themselves. The owner should call
//! `is_escape(ev)` (or `menu_action`) in its `handle_key` and close the overlay. An overlay that
//! contains a `LineInput` gets Escape from the input as `LineInputEvent::Cancel` instead.

use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, FontWeight, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, Rgba, SharedString, Window, div, prelude::*, px,
};

use super::controls::{ButtonKind, ListAction, PickIndex, button, icon_button};
use crate::theme::{ActiveTheme, ICON_GAP, ICON_SIZE, RADIUS_L, RADIUS_S};
use crate::ui::icons::icon;

/// Width of a context menu, in px.
pub const MENU_WIDTH: f32 = 220.0;
/// Height of one menu row.
pub const MENU_ROW_H: f32 = 30.0;
/// Height of a menu separator.
pub const MENU_SEP_H: f32 = 9.0;
/// Padding above and below the menu rows.
pub const MENU_PAD: f32 = 4.0;
/// Tallest a menu gets before its rows scroll.
pub const MENU_MAX_H: f32 = 320.0;
/// Gap kept between an overlay and the window edge.
pub const EDGE_MARGIN: f32 = 8.0;

/// Background behind a modal: 0.55 alpha in dark themes, 0.35 in light ones.
pub fn scrim_colour(t: &ActiveTheme) -> Rgba {
    if t.kind.is_dark() {
        gpui::rgba(0x0000008C)
    } else {
        gpui::rgba(0x00000059)
    }
}

/// True for the Escape key without modifiers.
pub fn is_escape(ev: &KeyDownEvent) -> bool {
    let m = ev.keystroke.modifiers;
    ev.keystroke.key == "escape" && !m.control && !m.alt && !m.shift && !m.platform
}

/// Where a popover of size `size` goes next to an anchor rectangle `(x, y, w, h)`, in window px.
///
/// Below the anchor, left-aligned with it, when it fits; otherwise above it. Horizontally it is
/// pulled back inside the window. `window` is the window size. Edges keep `margin`.
pub fn place_popover(
    anchor: (f32, f32, f32, f32),
    size: (f32, f32),
    window: (f32, f32),
    margin: f32,
) -> (f32, f32) {
    let (ax, ay, _aw, ah) = anchor;
    let (w, h) = size;
    let (ww, wh) = window;
    let below = ay + ah;
    let y = if below + h > wh - margin && ay - h >= margin {
        ay - h
    } else {
        below
    };
    let x = ax;
    (clamp_axis(x, w, ww, margin), clamp_axis(y, h, wh, margin))
}

/// Position of a context menu opened at point `pos`: the menu's top-left corner sits on the
/// point. It flips to the left or upwards when it would leave the window, then is clamped.
pub fn place_menu(
    pos: (f32, f32),
    size: (f32, f32),
    window: (f32, f32),
    margin: f32,
) -> (f32, f32) {
    let (px_, py) = pos;
    let (w, h) = size;
    let (ww, wh) = window;
    let x = if px_ + w > ww - margin { px_ - w } else { px_ };
    let y = if py + h > wh - margin { py - h } else { py };
    (clamp_axis(x, w, ww, margin), clamp_axis(y, h, wh, margin))
}

/// Keeps `[pos, pos + len]` inside `[margin, total - margin]`. When the box is larger than the
/// space it is pinned to the start edge.
fn clamp_axis(pos: f32, len: f32, total: f32, margin: f32) -> f32 {
    let max = total - margin - len;
    if max < margin {
        margin
    } else {
        pos.clamp(margin, max)
    }
}

/// One row of a context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub icon: Option<&'static str>,
    pub shortcut: Option<String>,
    pub danger: bool,
    pub enabled: bool,
    pub separator: bool,
}

impl MenuItem {
    /// An enabled row with a label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            shortcut: None,
            danger: false,
            enabled: true,
            separator: false,
        }
    }

    /// A thin line between groups of rows.
    pub fn separator() -> Self {
        Self {
            separator: true,
            enabled: false,
            ..Self::new("")
        }
    }

    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn shortcut(mut self, text: impl Into<String>) -> Self {
        self.shortcut = Some(text.into());
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    fn row_height(&self) -> f32 {
        if self.separator {
            MENU_SEP_H
        } else {
            MENU_ROW_H
        }
    }
}

/// Height of a menu with these rows, capped at `MENU_MAX_H` (the rest scrolls).
pub fn menu_height(items: &[MenuItem]) -> f32 {
    let rows: f32 = items.iter().map(MenuItem::row_height).sum();
    (rows + 2.0 * MENU_PAD).min(MENU_MAX_H)
}

/// Key handling for a context menu. `highlight` is the highlighted row, if any. Up and Down move
/// over enabled rows only and wrap around. Enter picks the highlighted row.
pub fn menu_action(key: &str, items: &[MenuItem], highlight: Option<usize>) -> ListAction {
    match key {
        "escape" => ListAction::Close,
        "down" | "up" => {
            let enabled: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(_, it)| it.enabled && !it.separator)
                .map(|(i, _)| i)
                .collect();
            if enabled.is_empty() {
                return ListAction::Ignore;
            }
            let down = key == "down";
            let next = match highlight.and_then(|h| enabled.iter().position(|&i| i == h)) {
                Some(pos) if down => enabled[(pos + 1) % enabled.len()],
                Some(pos) => enabled[(pos + enabled.len() - 1) % enabled.len()],
                None if down => enabled[0],
                None => enabled[enabled.len() - 1],
            };
            ListAction::Highlight(next)
        }
        "enter" => match highlight {
            Some(h) if items.get(h).is_some_and(|it| it.enabled && !it.separator) => {
                ListAction::Pick(h)
            }
            _ => ListAction::Ignore,
        },
        _ => ListAction::Ignore,
    }
}

/// Dimmed full-window layer with a centred card. A click on the dimmed area calls `on_close`;
/// a click on the card does not. The header close button and Escape (see `is_escape`) also close.
pub fn modal(
    t: &ActiveTheme,
    title: &str,
    body: impl IntoElement,
    footer: Vec<AnyElement>,
    width: f32,
    on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let close = Rc::new(on_close);
    let close_btn = close.clone();
    let scrim_close = close.clone();

    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(52.0))
        .px(px(20.0))
        .child(
            div()
                .font_family(crate::ui::fonts::ui_name())
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.tx)
                .child(SharedString::from(title.to_string())),
        )
        .child(icon_button(t, "x", move |e, w, cx| close_btn(e, w, cx)));

    let mut footer_row = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.0))
        .h(px(56.0))
        .px(px(16.0))
        .border_t_1()
        .border_color(t.line_soft)
        .bg(t.hover)
        // gpui clips children to the rectangle, so the footer rounds its own bottom corners.
        .rounded_b(px(RADIUS_L));
    for button_el in footer {
        footer_row = footer_row.child(button_el);
    }

    let card = div()
        .flex()
        .flex_col()
        .w(px(width))
        .max_h(px(560.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .rounded(px(RADIUS_L))
        .shadow_lg()
        .overflow_hidden()
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .text_color(t.tx)
        .child(header)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .px(px(20.0))
                .pb(px(20.0))
                .child(body),
        )
        .child(footer_row)
        // A click on the card stops here, so the dimmed layer below does not see it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(scrim_colour(t))
        // Blocks the mouse for everything below the dimmed layer.
        .occlude()
        .on_mouse_down(MouseButton::Left, move |e, w, cx| scrim_close(e, w, cx))
        .child(card)
        .into_any_element()
}

/// Modal asking to confirm an action. `danger` makes the confirm button red. Both buttons and the
/// dimmed area call handlers; Escape should be routed to `on_cancel` by the owner.
pub fn confirm_dialog(
    t: &ActiveTheme,
    title: &str,
    message: &str,
    confirm_label: &str,
    danger: bool,
    on_confirm: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let cancel = Rc::new(on_cancel);
    let cancel_btn = cancel.clone();
    let cancel_x = cancel.clone();
    let body = div()
        .pt(px(4.0))
        .text_size(px(13.5))
        .text_color(t.tx2)
        .child(SharedString::from(message.to_string()));
    let footer = vec![
        button(t, "取消", ButtonKind::Secondary, None, move |e, w, cx| {
            cancel_btn(e, w, cx)
        })
        .into_any_element(),
        button(
            t,
            confirm_label,
            if danger {
                ButtonKind::Danger
            } else {
                ButtonKind::Primary
            },
            None,
            on_confirm,
        )
        .into_any_element(),
    ];
    modal(t, title, body, footer, 380.0, move |e, w, cx| {
        cancel_x(e, w, cx)
    })
}

/// Context menu at window point `pos` (px from the window's top-left). Clicking a row calls
/// `on_select` with its index and the owner closes it. A click outside calls `on_close`.
/// `highlight` is the keyboard row (see `menu_action`). Flips to stay inside `window`.
pub fn context_menu(
    t: &ActiveTheme,
    items: &[MenuItem],
    highlight: Option<usize>,
    pos: (f32, f32),
    window: (f32, f32),
    on_select: PickIndex,
    on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let size = (MENU_WIDTH, menu_height(items));
    let (x, y) = place_menu(pos, size, window, EDGE_MARGIN);

    let mut list = div()
        .id("widgets-context-menu")
        .flex()
        .flex_col()
        .py(px(MENU_PAD))
        .max_h(px(MENU_MAX_H))
        .overflow_y_scroll();
    for (i, item) in items.iter().enumerate() {
        if item.separator {
            list = list.child(
                div()
                    .h(px(MENU_SEP_H))
                    .mx(px(8.0))
                    .flex()
                    .items_center()
                    .child(div().w_full().h(px(1.0)).bg(t.line_soft)),
            );
            continue;
        }
        let fg = if !item.enabled {
            t.tx3
        } else if item.danger {
            gpui::rgb(super::controls::DANGER_RGB)
        } else {
            t.tx
        };
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(ICON_GAP))
            .h(px(MENU_ROW_H))
            .mx(px(4.0))
            .px(px(10.0))
            .rounded(px(RADIUS_S))
            .text_color(fg)
            .child(match item.icon {
                Some(name) => icon(name, ICON_SIZE, fg).into_any_element(),
                None => div().w(px(ICON_SIZE)).into_any_element(),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap()
                    .child(SharedString::from(item.label.clone())),
            );
        if let Some(text) = item.shortcut.as_ref() {
            row = row.child(
                div()
                    .text_size(px(11.5))
                    .text_color(t.tx3)
                    .child(SharedString::from(text.clone())),
            );
        }
        if !item.enabled {
            row = row.opacity(0.6);
        } else {
            row = row.cursor_pointer();
            if highlight == Some(i) {
                row = row.bg(t.active);
            } else {
                row = row.hover(|s| s.bg(t.hover));
            }
            let pick = on_select.clone();
            row = row.on_mouse_down(MouseButton::Left, move |_, w, cx| pick(&i, w, cx));
        }
        list = list.child(row);
    }

    let card = div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(MENU_WIDTH))
        .rounded(px(RADIUS_L))
        .border_1()
        .border_color(t.line)
        .bg(t.raised)
        .shadow_lg()
        .overflow_hidden()
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .text_color(t.tx)
        .child(list)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    div()
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(MouseButton::Left, move |e, w, cx| on_close(e, w, cx))
        .child(card)
        .into_any_element()
}

/// Text bubble for `div().tooltip(...)`: `.tooltip(widgets::overlay::tooltip(t, "text"))`.
pub fn tooltip(
    t: ActiveTheme,
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| {
        cx.new(|_| TooltipView {
            theme: t,
            text: text.clone(),
        })
        .into()
    }
}

struct TooltipView {
    theme: ActiveTheme,
    text: SharedString,
}

impl gpui::Render for TooltipView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(RADIUS_S))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .shadow_md()
            .font_family(crate::ui::fonts::ui_name())
            .text_size(px(12.0))
            .text_color(t.tx)
            .child(self.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_opens_inside_when_there_is_room() {
        let p = place_menu((100.0, 100.0), (220.0, 200.0), (1280.0, 800.0), 8.0);
        assert_eq!(p, (100.0, 100.0));
    }

    #[test]
    fn menu_flips_left_and_up_near_bottom_right() {
        let p = place_menu((1270.0, 790.0), (220.0, 200.0), (1280.0, 800.0), 8.0);
        assert_eq!(p, (1270.0 - 220.0, 790.0 - 200.0));
    }

    #[test]
    fn menu_clamps_when_flip_is_not_enough() {
        // Taller than the window: pinned to the top margin.
        let p = place_menu((10.0, 700.0), (220.0, 900.0), (1280.0, 800.0), 8.0);
        assert_eq!(p.1, 8.0);
        // Anchor left of the margin is pulled in.
        let q = place_menu((-50.0, 10.0), (100.0, 50.0), (1280.0, 800.0), 8.0);
        assert_eq!(q.0, 8.0);
    }

    #[test]
    fn popover_goes_below_then_above() {
        // Anchor near the top: below.
        let below = place_popover(
            (50.0, 100.0, 180.0, 30.0),
            (180.0, 200.0),
            (1280.0, 800.0),
            8.0,
        );
        assert_eq!(below, (50.0, 130.0));
        // Anchor near the bottom: above the anchor.
        let above = place_popover(
            (50.0, 700.0, 180.0, 30.0),
            (180.0, 200.0),
            (1280.0, 800.0),
            8.0,
        );
        assert_eq!(above, (50.0, 700.0 - 200.0));
        // Right edge: pulled back inside.
        let edge = place_popover(
            (1200.0, 100.0, 180.0, 30.0),
            (180.0, 50.0),
            (1280.0, 800.0),
            8.0,
        );
        assert_eq!(edge.0, 1280.0 - 8.0 - 180.0);
    }

    #[test]
    fn menu_height_counts_rows_and_caps() {
        let items = vec![
            MenuItem::new("a"),
            MenuItem::separator(),
            MenuItem::new("b"),
        ];
        assert_eq!(
            menu_height(&items),
            2.0 * MENU_ROW_H + MENU_SEP_H + 2.0 * MENU_PAD
        );
        let many: Vec<MenuItem> = (0..100).map(|i| MenuItem::new(i.to_string())).collect();
        assert_eq!(menu_height(&many), MENU_MAX_H);
    }

    #[test]
    fn menu_keys_skip_disabled_and_separators() {
        let items = vec![
            MenuItem::new("a"),
            MenuItem::new("b").disabled(),
            MenuItem::separator(),
            MenuItem::new("c"),
        ];
        assert_eq!(menu_action("down", &items, None), ListAction::Highlight(0));
        assert_eq!(
            menu_action("down", &items, Some(0)),
            ListAction::Highlight(3)
        );
        assert_eq!(
            menu_action("down", &items, Some(3)),
            ListAction::Highlight(0)
        );
        assert_eq!(menu_action("up", &items, Some(0)), ListAction::Highlight(3));
        assert_eq!(menu_action("up", &items, None), ListAction::Highlight(3));
        assert_eq!(menu_action("enter", &items, Some(3)), ListAction::Pick(3));
        assert_eq!(menu_action("enter", &items, Some(1)), ListAction::Ignore);
        assert_eq!(menu_action("enter", &items, None), ListAction::Ignore);
        assert_eq!(menu_action("escape", &items, None), ListAction::Close);
        assert_eq!(menu_action("x", &items, None), ListAction::Ignore);
    }

    #[test]
    fn menu_with_nothing_enabled_ignores_arrows() {
        let items = vec![MenuItem::new("a").disabled()];
        assert_eq!(menu_action("down", &items, None), ListAction::Ignore);
    }

    #[test]
    fn scrim_alpha_depends_on_mode() {
        assert!(scrim_colour(&ActiveTheme::dark()).a > scrim_colour(&ActiveTheme::light()).a);
    }
}
