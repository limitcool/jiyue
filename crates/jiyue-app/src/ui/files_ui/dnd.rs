//! Drag and drop in the file tree. A note or folder row dragged onto a folder row moves into that
//! folder; the folder opens when the pointer rests on it for 600 ms. Dropped on a note it moves into
//! the note's folder, and dropped on empty space in the tree it moves to the vault root. The move
//! goes through `actions::move_item`, so links are updated the same way a rename updates them.

use std::time::{Duration, Instant};

use gpui::{
    App, Context, Entity, IntoElement, Pixels, Point, Render, SharedString, Window, div,
    prelude::*, px,
};

use super::actions;
use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::ext::ToastKind;

/// How long the pointer must rest on a closed folder before the folder opens.
pub const EXPAND_AFTER: Duration = Duration::from_millis(600);

/// What a tree row carries while it is dragged.
#[derive(Clone, Debug)]
pub struct TreeDrag {
    /// Vault-relative path of the note, or the folder key.
    pub path: String,
    pub is_dir: bool,
    pub theme: ActiveTheme,
}

/// The chip that follows the pointer while a row is dragged.
pub struct DragPreview {
    label: SharedString,
    theme: ActiveTheme,
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .h(px(26.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .text_size(px(12.5))
            .text_color(t.tx)
            .child(self.label.clone())
    }
}

/// Builds the chip for a dragged row.
pub fn preview(
    value: &TreeDrag,
    _pos: Point<Pixels>,
    _window: &mut Window,
    cx: &mut App,
) -> Entity<DragPreview> {
    let label = name_of(&value.path).to_string();
    let theme = value.theme;
    cx.new(|_| DragPreview {
        label: SharedString::from(label),
        theme,
    })
}

/// The last segment of a vault-relative path.
fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The folder key that holds `path` (`""` for a note or folder at the root).
pub fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Moves the dragged row into `dest_dir` (`""` for the root). A refused move becomes an error
/// toast; a move that changes nothing is silent.
pub fn drop_into(
    app: &mut RustdianApp,
    window: &mut Window,
    drag: &TreeDrag,
    dest_dir: &str,
    cx: &mut Context<RustdianApp>,
) {
    app.ext.files.drag_hover = None;
    if let Err(message) = actions::move_item(app, window, &drag.path, drag.is_dir, dest_dir, cx) {
        app.toast(ToastKind::Error, message);
    }
    cx.notify();
}

/// Called on every drag move with whether the pointer is inside the closed folder `key`. Starts the
/// timer that opens the folder once the pointer has rested there for `EXPAND_AFTER`.
pub fn hover_folder(app: &mut RustdianApp, key: &str, inside: bool, cx: &mut Context<RustdianApp>) {
    let hovering_this = app
        .ext
        .files
        .drag_hover
        .as_ref()
        .is_some_and(|(k, _)| k == key);
    if !inside {
        if hovering_this {
            app.ext.files.drag_hover = None;
        }
        return;
    }
    if hovering_this || app.expanded_folders.iter().any(|k| k == key) {
        return;
    }
    let started = Instant::now();
    app.ext.files.drag_hover = Some((key.to_string(), started));
    let key = key.to_string();
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(EXPAND_AFTER).await;
        let _ = this.update(cx, |app, cx| {
            // Only the same resting pointer opens the folder: a move away cleared the hover.
            let still = app
                .ext
                .files
                .drag_hover
                .as_ref()
                .is_some_and(|(k, at)| *k == key && *at == started);
            if still {
                app.ext.files.drag_hover = None;
                super::expand_folder(app, &key);
                cx.notify();
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_dir_is_the_folder_key_or_root() {
        assert_eq!(parent_dir("a/b/x.md"), "a/b");
        assert_eq!(parent_dir("x.md"), "");
        assert_eq!(parent_dir("a/folder"), "a");
    }

    #[test]
    fn name_is_the_last_segment() {
        assert_eq!(name_of("a/b/x.md"), "x.md");
        assert_eq!(name_of("solo"), "solo");
    }
}
