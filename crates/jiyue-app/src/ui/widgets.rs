//! Shared UI widgets (IME-capable line input, buttons, toggles, dropdowns, modal, context menu).
//! Owned by W1.6. Every colour comes from `ActiveTheme` tokens (the danger red aside).
//!
//! Layout of the library:
//! - `line_input`: `LineInput` entity (text field with IME, selection, undo, password mode).
//! - `controls`: stateless controls (button, toggle, segmented, select, checkbox, kbd, badge,
//!   form_row, progress_bar, spinner, empty_state) and the keyboard helper `select_action`.
//! - `overlay`: modal, confirm_dialog, context_menu (flips inside the window), popover placement,
//!   tooltip, and the menu key helper `menu_action`.
//! - `text_edit`: pure text model behind `LineInput` (boundaries, password masks, undo).
//! - `demo`: a catalogue of all of the above, shown with `demo::render_demo`.
//!
//! Setup: call `widgets::bind_keys(cx)` once next to `ui::text_input::bind_keys(cx)`.
#![allow(dead_code)]

pub mod controls;
pub mod demo;
pub mod line_input;
pub mod overlay;
pub mod text_edit;

#[allow(unused_imports)]
pub use controls::{
    BadgeKind, ButtonKind, ListAction, badge, button, checkbox, empty_state, form_row, icon_button,
    kbd, progress_bar, section_title, segmented, select, select_action, spinner, toggle,
};
#[allow(unused_imports)]
pub use line_input::{LineInput, LineInputEvent};
#[allow(unused_imports)]
pub use overlay::{
    MenuItem, confirm_dialog, context_menu, is_escape, menu_action, modal, place_menu,
    place_popover, tooltip,
};

use gpui::App;

/// Registers the key bindings of the widgets (`LineInput`). Call once at app start.
pub fn bind_keys(cx: &mut App) {
    line_input::bind_keys(cx);
}
