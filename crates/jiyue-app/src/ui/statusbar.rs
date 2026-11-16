//! Status bar (24px) at the bottom of the workspace column.

use crate::RustdianApp;
use crate::session::{SaveStatus, ViewMode};
use crate::theme::STATUSBAR_HEIGHT;
use gpui::{Context, IntoElement, Rgba, SharedString, div, prelude::*, px};

/// Renders the status bar: cursor position and encoding on the left, save state on the right.
/// The cursor follows the active note's editor (the app caches it at each render).
pub fn render_statusbar(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let welcome = app.is_welcome_view();
    // Feature modules' items sit before the save indicator, 12px apart.
    let items = crate::ui::ext::statusbar_items(app, cx);

    let mut bar = div()
        .flex()
        .items_center()
        .gap(px(16.0))
        .h(px(STATUSBAR_HEIGHT))
        .flex_none()
        .px(px(14.0))
        .bg(t.panel)
        .border_t_1()
        .border_color(t.border_subtle)
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(11.0))
        .text_color(t.tx3);

    if !welcome {
        let mode = match app.view_mode {
            ViewMode::Source => crate::t!("status-source"),
            ViewMode::Live => crate::t!("status-live-preview"),
            ViewMode::Read => crate::t!("status-reading"),
        };
        bar = bar
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
            .child(div().flex_none().mr(px(16.0)).child(mode));
    }

    let mut right = div()
        .ml_auto()
        .flex()
        .items_center()
        .gap(px(12.0))
        .children(items);
    if !welcome {
        let status = app.active_save_status(cx);
        let (label, dot) = match status {
            SaveStatus::Saved => (
                crate::t!("status-saved"),
                div().size(px(6.0)).rounded_full().bg(t.ok),
            ),
            SaveStatus::Unsaved => (
                crate::t!("status-unsaved"),
                div().size(px(6.0)).rounded_full().bg(t.warn),
            ),
            SaveStatus::Saving => (
                crate::t!("status-saving"),
                div().size(px(6.0)).rounded_full().bg(t.warn),
            ),
            SaveStatus::Failed => (
                crate::t!("status-save-failed"),
                div().size(px(6.0)).rounded_full().bg(t.danger),
            ),
        };
        let label_color: Rgba = match status {
            SaveStatus::Failed => t.danger,
            _ => t.tx3,
        };
        right = right.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_color(label_color)
                .child(dot)
                .child(label),
        );
    }

    bar.child(right)
}
