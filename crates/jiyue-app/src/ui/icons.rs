//! Icon helper. Icons are alpha masks, so the colour passed here tints them.

use gpui::{IntoElement, Rgba, Styled, px, svg};

/// Draws the embedded icon `name` (for example `"search"`) at `size` px, tinted with `color`.
pub fn icon(name: &str, size: f32, color: Rgba) -> impl IntoElement + use<> {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(color)
}
