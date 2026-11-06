//! Stateless controls: each function takes the theme, the caller's values and click handlers
//! (usually `cx.listener(...)`) and returns a gpui element. Nothing here keeps state.
//!
//! Handler types: single events take `impl Fn(&MouseDownEvent, &mut Window, &mut App)`. Handlers
//! that carry an index (segment, select option) take `Rc<dyn Fn(&usize, &mut Window, &mut App)>`,
//! which `cx.listener` builds from a closure over `usize`.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, Div, FontWeight, IntoElement, MouseButton,
    MouseDownEvent, SharedString, Transformation, Window, div, percentage, prelude::*, px,
    relative, svg,
};

use crate::theme::{
    ActiveTheme, ICON_GAP_TIGHT, ICON_SIZE, RADIUS_L, RADIUS_M, RADIUS_PILL, RADIUS_S,
};
use crate::ui::icons::icon;

/// Red used for every danger control and destructive menu row.
pub const DANGER_RGB: u32 = 0xE5484D;

/// Click handler for one mouse event.
pub type Click = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;
/// Handler that receives the index of the item it belongs to.
pub type PickIndex = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeKind {
    Neutral,
    Accent,
    Ok,
    Warn,
    Danger,
}

fn danger_colour() -> gpui::Rgba {
    gpui::rgb(DANGER_RGB)
}

/// Push button, 30px high. `icon` is an embedded icon name drawn before the label.
pub fn button(
    t: &ActiveTheme,
    label: &str,
    kind: ButtonKind,
    icon_name: Option<&str>,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let (bg, fg, border, hover) = match kind {
        ButtonKind::Primary => (t.ac, t.ac_on, t.ac, t.ac),
        ButtonKind::Secondary => (t.raised, t.tx, t.line, t.hover),
        ButtonKind::Ghost => (
            gpui::rgba(0x00000000),
            t.tx2,
            gpui::rgba(0x00000000),
            t.hover,
        ),
        ButtonKind::Danger => (
            danger_colour(),
            gpui::rgb(0xFFFFFF),
            danger_colour(),
            danger_colour(),
        ),
    };
    let mut el = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(ICON_GAP_TIGHT))
        .h(px(30.0))
        .px(px(12.0))
        .rounded(px(RADIUS_M))
        .border_1()
        .border_color(border)
        .bg(bg)
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .cursor_pointer()
        .whitespace_nowrap();
    if let Some(name) = icon_name {
        el = el.child(icon(name, ICON_SIZE, fg));
    }
    el = el.child(SharedString::from(label.to_string()));
    el = match kind {
        ButtonKind::Primary | ButtonKind::Danger => el.hover(|s| s.opacity(0.9)),
        _ => el.hover(move |s| s.bg(hover)),
    };
    el.on_mouse_down(MouseButton::Left, on_click)
}

/// Square icon-only button (28px) with a hover background.
pub fn icon_button(
    t: &ActiveTheme,
    icon_name: &str,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(RADIUS_S))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .child(icon(icon_name, ICON_SIZE, t.tx2))
        .on_mouse_down(MouseButton::Left, on_click)
}

/// Switch, 36x20 as in the settings page. `on_change` is called on click; the caller flips its value.
pub fn toggle(
    t: &ActiveTheme,
    on: bool,
    on_change: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let knob_left = if on { 18.0 } else { 2.0 };
    div()
        .relative()
        .flex_none()
        .w(px(36.0))
        .h(px(20.0))
        .rounded(px(RADIUS_PILL))
        .cursor_pointer()
        .bg(if on { t.ac } else { t.active })
        .border_1()
        .border_color(if on { t.ac } else { t.line })
        .child(
            div()
                .absolute()
                .top(px(1.0))
                .left(px(knob_left))
                .size(px(16.0))
                .rounded_full()
                .bg(t.raised),
        )
        .on_mouse_down(MouseButton::Left, on_change)
}

/// Segmented control: one segment per option, the selected one highlighted. `on_select` gets the
/// index of the clicked segment.
pub fn segmented(t: &ActiveTheme, options: &[&str], selected: usize, on_select: PickIndex) -> Div {
    let mut control = div()
        .flex()
        .flex_none()
        .h(px(30.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .overflow_hidden()
        .font_family(crate::ui::fonts::ui_name());
    for (i, name) in options.iter().enumerate() {
        let on = i == selected;
        let mut seg = div()
            .flex()
            .items_center()
            .px(px(12.0))
            .h_full()
            .text_size(px(12.5))
            .cursor_pointer()
            .text_color(if on { t.tx } else { t.tx2 })
            .font_weight(if on {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            })
            .child(SharedString::from(name.to_string()));
        // The container clips only its rectangle, so the end segments round their own outer corners
        // (inside the 1px border: radius minus the border width).
        let inner = RADIUS_M - 1.0;
        if i == 0 {
            seg = seg.rounded_l(px(inner));
        }
        if i + 1 == options.len() {
            seg = seg.rounded_r(px(inner));
        }
        if on {
            seg = seg.bg(t.active);
        } else {
            seg = seg.hover(|s| s.bg(t.hover));
        }
        if i > 0 {
            seg = seg.border_l_1().border_color(t.line_soft);
        }
        let pick = on_select.clone();
        seg = seg.on_mouse_down(MouseButton::Left, move |_, window, cx| pick(&i, window, cx));
        control = control.child(seg);
    }
    control
}

/// Dropdown. The trigger shows `options[selected]`. While closed the trigger opens the list
/// (`on_open`); while open the list closes (`on_close`) on any mouse down outside it, and an option
/// click calls `on_pick` with its index. The trigger has no listener while the list is open, so one
/// click never both closes and reopens it. Keyboard: `select_action` on Up/Down/Enter/Escape.
///
/// The returned element is `relative`; place it where the dropdown should sit. The list opens
/// below the trigger (it is not flipped; keep the control away from the bottom edge).
#[allow(clippy::too_many_arguments)]
pub fn select(
    t: &ActiveTheme,
    options: &[&str],
    selected: usize,
    open: bool,
    highlight: usize,
    width: f32,
    on_open: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_pick: PickIndex,
) -> Div {
    let current = options.get(selected).copied().unwrap_or("");
    let mut trigger = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .w(px(width))
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(RADIUS_M))
        .border_1()
        .border_color(if open { t.ac } else { t.line })
        .bg(t.raised)
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .text_color(t.tx)
        .cursor_pointer()
        .child(SharedString::from(current.to_string()))
        .child(icon("chevron-down", 14.0, t.tx3));
    if !open {
        trigger = trigger.on_mouse_down(MouseButton::Left, on_open);
    }

    let mut wrap = div().relative().w(px(width)).child(trigger);
    if open {
        let mut list = div()
            .absolute()
            .top(px(34.0))
            .left_0()
            .w_full()
            .py(px(4.0))
            .rounded(px(RADIUS_L))
            .border_1()
            .border_color(t.line)
            .bg(t.raised)
            .shadow_md()
            .font_family(crate::ui::fonts::ui_name())
            .text_size(px(13.0))
            .text_color(t.tx)
            .on_mouse_down_out(on_close);
        for (i, name) in options.iter().enumerate() {
            let on = i == highlight;
            let pick = on_pick.clone();
            let mut row = div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(30.0))
                .mx(px(4.0))
                .px(px(10.0))
                .rounded(px(RADIUS_S))
                .cursor_pointer()
                .child(SharedString::from(name.to_string()));
            if on {
                row = row.bg(t.active);
            } else {
                row = row.hover(|s| s.bg(t.hover));
            }
            if i == selected {
                row = row.child(icon("check", 13.0, t.ac));
            }
            row = row.on_mouse_down(MouseButton::Left, move |_, window, cx| pick(&i, window, cx));
            list = list.child(row);
        }
        wrap = wrap.child(list);
    }
    wrap
}

/// Checkbox, 16px. `on_change` is called on click; the caller flips its value.
pub fn checkbox(
    t: &ActiveTheme,
    checked: bool,
    on_change: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let mut boxed = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(16.0))
        .rounded(px(RADIUS_S))
        .border_1()
        .border_color(if checked { t.ac } else { t.line })
        .bg(if checked { t.ac } else { t.raised })
        .cursor_pointer();
    if checked {
        boxed = boxed.child(icon("check", 11.0, t.ac_on));
    }
    boxed.on_mouse_down(MouseButton::Left, on_change)
}

/// Key cap chip, for example `Ctrl` or `Esc`.
pub fn kbd(t: &ActiveTheme, text: &str) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .px(px(6.0))
        .h(px(18.0))
        .rounded(px(RADIUS_S))
        .border_1()
        .border_color(t.line)
        .bg(t.hover)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.tx2)
        .whitespace_nowrap()
        .child(SharedString::from(text.to_string()))
}

/// Small status label.
pub fn badge(t: &ActiveTheme, text: &str, kind: BadgeKind) -> Div {
    let (fg, bg) = match kind {
        BadgeKind::Neutral => (t.tx2, t.hover),
        BadgeKind::Accent => (t.ac, t.ac_soft),
        BadgeKind::Ok => (t.ok, t.ok_soft),
        BadgeKind::Warn => (t.st, t.hover),
        BadgeKind::Danger => (danger_colour(), gpui::rgba(0xE5484D1F)),
    };
    div()
        .flex_none()
        .px(px(7.0))
        .py(px(1.0))
        .rounded(px(RADIUS_PILL))
        .bg(bg)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .whitespace_nowrap()
        .child(SharedString::from(text.to_string()))
}

/// Small caption above a group of rows.
pub fn section_title(t: &ActiveTheme, text: &str) -> Div {
    div()
        .pt(px(12.0))
        .pb(px(6.0))
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.tx3)
        .child(SharedString::from(text.to_string()))
}

/// Settings row: title and description on the left, `control` on the right, a thin separator
/// below unless `last`.
pub fn form_row(
    t: &ActiveTheme,
    title: &str,
    description: &str,
    control: impl IntoElement,
    last: bool,
) -> Div {
    let left = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .min_w_0()
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx)
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child(SharedString::from(description.to_string())),
        );
    let mut row = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(20.0))
        .py(px(8.0))
        .font_family(crate::ui::fonts::ui_name())
        .child(left)
        .child(control);
    if !last {
        row = row.border_b_1().border_color(t.line_soft);
    }
    row
}

/// Determinate progress bar, 6px high. `fraction` is clamped to 0..=1.
pub fn progress_bar(t: &ActiveTheme, fraction: f32) -> Div {
    let f = clamp_fraction(fraction);
    div()
        .w_full()
        .h(px(6.0))
        .rounded(px(RADIUS_PILL))
        .bg(t.active)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(f))
                .rounded(px(RADIUS_PILL))
                .bg(t.ac),
        )
}

/// Clamps a progress value to 0..=1 (NaN counts as 0).
pub fn clamp_fraction(f: f32) -> f32 {
    if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) }
}

/// Rotating spinner, `size` px. Uses the `dots` icon turned by a 900 ms repeating animation.
pub fn spinner(t: &ActiveTheme, size: f32) -> AnyElement {
    svg()
        .path("icons/dots.svg")
        .size(px(size))
        .flex_none()
        .text_color(t.ac)
        .with_animation(
            "widgets-spinner",
            Animation::new(Duration::from_millis(900)).repeat(),
            |el, delta| el.with_transformation(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
}

/// Centred placeholder for an empty list or panel.
pub fn empty_state(t: &ActiveTheme, icon_name: &str, title: &str, text: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .py(px(32.0))
        .px(px(16.0))
        .font_family(crate::ui::fonts::ui_name())
        .child(icon(icon_name, 28.0, t.tx3))
        .child(
            div()
                .text_size(px(14.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx2)
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .max_w(px(320.0))
                .text_size(px(12.5))
                .text_color(t.tx3)
                .child(SharedString::from(text.to_string())),
        )
}

// ----- pure helpers for keyboard navigation -----

/// What a dropdown or menu key press asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListAction {
    /// Highlight this index.
    Highlight(usize),
    /// Choose this index and close.
    Pick(usize),
    Close,
    /// Not a list key.
    Ignore,
}

/// Dropdown keys. `key` is the keystroke key name (`up`, `down`, `enter`, `escape`). Up and Down
/// stop at the ends of the list. With no `len`, only Escape closes.
pub fn select_action(key: &str, highlight: usize, len: usize) -> ListAction {
    match key {
        "escape" => ListAction::Close,
        _ if len == 0 => ListAction::Ignore,
        "down" => ListAction::Highlight((highlight + 1).min(len - 1)),
        "up" => ListAction::Highlight(highlight.saturating_sub(1)),
        "enter" => ListAction::Pick(highlight.min(len - 1)),
        _ => ListAction::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_navigation_clamps_at_both_ends() {
        assert_eq!(select_action("down", 0, 3), ListAction::Highlight(1));
        assert_eq!(select_action("down", 2, 3), ListAction::Highlight(2));
        assert_eq!(select_action("up", 0, 3), ListAction::Highlight(0));
        assert_eq!(select_action("up", 2, 3), ListAction::Highlight(1));
    }

    #[test]
    fn select_enter_and_escape() {
        assert_eq!(select_action("enter", 1, 3), ListAction::Pick(1));
        assert_eq!(select_action("enter", 9, 3), ListAction::Pick(2));
        assert_eq!(select_action("escape", 0, 0), ListAction::Close);
        assert_eq!(select_action("a", 0, 3), ListAction::Ignore);
        assert_eq!(select_action("down", 0, 0), ListAction::Ignore);
        assert_eq!(select_action("enter", 0, 0), ListAction::Ignore);
    }

    #[test]
    fn progress_fraction_is_clamped() {
        assert_eq!(clamp_fraction(-1.0), 0.0);
        assert_eq!(clamp_fraction(0.25), 0.25);
        assert_eq!(clamp_fraction(3.0), 1.0);
        assert_eq!(clamp_fraction(f32::NAN), 0.0);
    }
}
