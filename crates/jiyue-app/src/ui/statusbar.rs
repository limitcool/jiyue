//! Status bar (28px) at the bottom of the sheet.

use crate::RustdianApp;
use crate::session::{SaveStatus, ViewMode};
use gpui::{Context, IntoElement, Rgba, SharedString, div, prelude::*, px, rgb};

/// Warm amber for unsaved changes (saved uses the theme's `ok`).
const UNSAVED_AMBER: u32 = 0xd9962b;
/// Red for a failed save.
const SAVE_FAILED_RED: u32 = 0xe5484d;

/// Renders the status bar: cursor position and encoding on the left, save state on the right.
/// The cursor follows the active note's editor (the app caches it at each render).
pub fn render_statusbar(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let mode = match app.view_mode {
        ViewMode::Source => crate::t!("status-source"),
        ViewMode::Live => crate::t!("status-live-preview"),
        ViewMode::Read => crate::t!("status-reading"),
    };
    let status = app.active_save_status(cx);
    let (label, dot) = match status {
        SaveStatus::Saved => (crate::t!("status-saved"), div().size(px(6.0)).rounded_full().bg(t.ok)),
        SaveStatus::Unsaved => (
            crate::t!("status-unsaved"),
            div().size(px(6.0)).rounded_full().bg(rgb(UNSAVED_AMBER)),
        ),
        SaveStatus::Saving => (
            crate::t!("status-saving"),
            div().size(px(6.0)).rounded_full().bg(rgb(UNSAVED_AMBER)),
        ),
        SaveStatus::Failed => (
            crate::t!("status-save-failed"),
            div().size(px(6.0)).rounded_full().bg(rgb(SAVE_FAILED_RED)),
        ),
    };
    let label_color: Rgba = match status {
        SaveStatus::Failed => rgb(SAVE_FAILED_RED),
        _ => t.tx3,
    };
    // Feature modules' items sit before the save indicator, 12px apart (empty until wave 2).
    let items = crate::ui::ext::statusbar_items(app, cx);

    div()
        .flex()
        .items_center()
        .gap(px(16.0))
        .h(px(28.0))
        .flex_none()
        .px(px(16.0))
        .border_t_1()
        .border_color(t.line_soft)
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(11.5))
        .text_color(t.tx3)
        .child(
            div()
                .flex_none()
                .mr(px(16.0))
                .child(SharedString::from(crate::t!(
                    "status-cursor",
                    line = app.cursor_row,
                    col = app.cursor_col
                ))),
        )
        .child(div().flex_none().mr(px(16.0)).child("UTF-8"))
        .child(div().flex_none().mr(px(16.0)).child(mode))
        .child(
            div()
                .ml_auto()
                .flex()
                .items_center()
                .gap(px(12.0))
                .children(items)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_color(label_color)
                        .child(dot)
                        .child(label),
                ),
        )
}
