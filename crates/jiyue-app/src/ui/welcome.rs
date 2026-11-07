//! VSCode-style Welcome Page (欢迎页).
//!
//! Displayed when no note tabs are open or on startup, providing quick start actions,
//! recently edited notes, keyboard shortcuts cheat sheet, and knowledge base tips.

use gpui::{
    Context, Div, FontWeight, MouseButton, MouseDownEvent, ScrollHandle, SharedString,
    div, prelude::*, px,
};

use crate::theme::ActiveTheme;
use crate::ui::display_family;
use crate::ui::icons::icon;
use crate::{AppCommand, RustdianApp, ViewportMode, t};

/// Renders the VSCode-style welcome page.
pub fn render_welcome_page(
    app: &RustdianApp,
    scroll: &ScrollHandle,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement {
    let t = app.theme;

    // Header / Hero
    let hero = div()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(40.0))
        .pb(px(28.0))
        .gap(px(8.0))
        .child(
            div()
                .font_family(display_family(&t))
                .text_size(px(32.0))
                .font_weight(FontWeight::BOLD)
                .text_color(t.tx)
                .child(t!("welcome-title")),
        )
        .child(
            div()
                .text_size(px(14.0))
                .text_color(t.tx3)
                .child(t!("welcome-subtitle")),
        );

    // Left Column: Start & Recent
    let left_col = render_left_column(app, &t, cx);

    // Right Column: Shortcuts & Tips
    let right_col = render_right_column(&t, cx);

    // Grid container
    let columns = div()
        .flex()
        .flex_row()
        .justify_center()
        .gap(px(48.0))
        .w_full()
        .max_w(px(880.0))
        .px(px(24.0))
        .child(left_col)
        .child(right_col);

    // Footer note
    let footer = div()
        .flex()
        .items_center()
        .justify_center()
        .pt(px(40.0))
        .pb(px(32.0))
        .text_size(px(12.0))
        .text_color(t.tx3)
        .child(format!("霁月 Jiyue v{}", env!("CARGO_PKG_VERSION")));

    div()
        .id("welcome_page")
        .flex_1()
        .min_h_0()
        .h_full()
        .w_full()
        .bg(t.sheet)
        .overflow_y_scroll()
        .track_scroll(scroll)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .w_full()
                .child(hero)
                .child(columns)
                .child(footer),
        )
}

fn render_left_column(
    app: &RustdianApp,
    t: &ActiveTheme,
    cx: &mut Context<RustdianApp>,
) -> Div {
    // 1. Start actions
    let start_title = section_title(t, &t!("welcome-start"));
    let action_new = action_button(
        t,
        "file-plus",
        &t!("welcome-new-file"),
        "Ctrl+N",
        cx.listener(|this, _, _, cx| {
            this.run_command(AppCommand::NewNote);
            cx.notify();
        }),
    );
    let action_search = action_button(
        t,
        "search",
        &t!("welcome-shortcut-quick-open"),
        "Ctrl+P",
        cx.listener(|this, _, _, cx| {
            this.show_palette = true;
            this.is_command_mode = false;
            this.palette_query.clear();
            cx.notify();
        }),
    );
    let action_graph = action_button(
        t,
        "share-2",
        &t!("sidebar-graph"),
        "Ctrl+G",
        cx.listener(|this, _, _, cx| {
            this.viewport_mode = ViewportMode::GraphView;
            cx.notify();
        }),
    );

    // 2. Recent notes list (up to 5 recent notes)
    let recent_title = section_title(t, &t!("welcome-recent"));
    let mut recent_list = div().flex().flex_col().gap(px(4.0)).w_full();

    if app.vault.notes.is_empty() {
        recent_list = recent_list.child(
            div()
                .py(px(8.0))
                .text_size(px(13.0))
                .text_color(t.tx3)
                .child(t!("welcome-no-recent")),
        );
    } else {
        // Collect latest notes (up to 5)
        for (idx, note) in app.vault.notes.iter().enumerate().take(5) {
            let note_title = if note.title.trim().is_empty() {
                t!("tab-untitled")
            } else {
                note.title.clone()
            };

            let row = div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.open_note_by_index(idx);
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .min_w_0()
                        .child(icon("file-text", 15.0, t.tx2))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(t.tx)
                                .font_weight(FontWeight::MEDIUM)
                                .child(note_title),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .child(note.path.clone()),
                );
            recent_list = recent_list.child(row);
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .w(px(380.0))
        .child(start_title)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .w_full()
                .child(action_new)
                .child(action_search)
                .child(action_graph),
        )
        .child(div().h(px(12.0)))
        .child(recent_title)
        .child(recent_list)
}

fn render_right_column(t: &ActiveTheme, _cx: &mut Context<RustdianApp>) -> Div {
    let shortcuts_title = section_title(t, &t!("welcome-shortcuts"));
    let tips_title = section_title(t, &t!("welcome-tips"));

    let shortcut_rows = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .w_full()
        .child(shortcut_item(t, &t!("welcome-shortcut-palette"), "Ctrl+P"))
        .child(shortcut_item(t, &t!("welcome-new-file"), "Ctrl+N"))
        .child(shortcut_item(t, &t!("welcome-shortcut-search"), "Ctrl+Shift+F"))
        .child(shortcut_item(t, &t!("welcome-shortcut-settings"), "Ctrl+,"));

    let tip_cards = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .w_full()
        .child(tip_card(t, "link", &t!("welcome-tip-wikilink")))
        .child(tip_card(t, "sparkles", &t!("welcome-tip-markdown")))
        .child(tip_card(t, "check", &t!("welcome-tip-local")));

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .w(px(380.0))
        .child(shortcuts_title)
        .child(shortcut_rows)
        .child(div().h(px(12.0)))
        .child(tips_title)
        .child(tip_cards)
}

fn section_title(t: &ActiveTheme, text: &str) -> Div {
    div()
        .text_size(px(14.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.tx)
        .pb(px(4.0))
        .child(SharedString::from(text.to_string()))
}

fn action_button(
    t: &ActiveTheme,
    ic: &str,
    label: &str,
    badge: &str,
    listener: impl Fn(&MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .hover(|s| s.bg(t.hover))
        .on_mouse_down(MouseButton::Left, listener)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(icon(ic, 16.0, t.ac))
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.tx)
                        .child(SharedString::from(label.to_string())),
                ),
        )
        .child(
            div()
                .px(px(6.0))
                .py(px(1.5))
                .rounded(px(4.0))
                .bg(t.chrome)
                .text_size(px(11.0))
                .text_color(t.tx3)
                .child(SharedString::from(badge.to_string())),
        )
}

fn shortcut_item(t: &ActiveTheme, name: &str, chord: &str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .py(px(3.0))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(t.tx2)
                .child(SharedString::from(name.to_string())),
        )
        .child(
            div()
                .px(px(6.0))
                .py(px(1.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(t.line)
                .bg(t.raised)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx2)
                .child(SharedString::from(chord.to_string())),
        )
}

fn tip_card(t: &ActiveTheme, ic: &str, content: &str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(10.0))
        .p(px(10.0))
        .rounded(px(6.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .child(div().pt(px(2.0)).child(icon(ic, 15.0, t.ac)))
        .child(
            div()
                .flex_1()
                .text_size(px(12.5))
                .line_height(px(18.0))
                .text_color(t.tx2)
                .child(SharedString::from(content.to_string())),
        )
}
