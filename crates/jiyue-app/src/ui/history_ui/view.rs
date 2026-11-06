//! The 历史版本 dialog: the versions of the note on the left, the selected version's diff against
//! the editor text on the right, and 复制此版本文本 / 恢复此版本 below.

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, MouseButton, SharedString, div, prelude::*, px,
};
use jiyue_history::{DiffTag, Kind};

use super::ops::{Loaded, Row};
use super::pure;
use super::{Backend, close_modal, copy_selected, restore, select};
use crate::RustdianApp;
use crate::theme::{ActiveTheme, RADIUS_M};
use crate::ui::widgets::{BadgeKind, ButtonKind, badge, button, modal};

/// Most diff lines shown.
const DIFF_LIMIT: usize = 400;
/// Red of removed lines (the danger colour used by the other dialogs).
const DELETED_RGB: u32 = 0xE5484D;

/// The dialog.
pub fn render(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let h = &app.ext.history;
    let title = format!("历史版本 · {}", h.path);

    let body: AnyElement = match &h.backend {
        Backend::Opening => text(&t, t.tx3, "正在打开历史库…").into_any_element(),
        Backend::Unavailable(message) => text(&t, t.st, message).into_any_element(),
        Backend::Ready(_) if h.loading_list => {
            text(&t, t.tx3, "正在读取历史版本…").into_any_element()
        }
        Backend::Ready(_) => match &h.list_error {
            Some(e) => text(&t, t.st, &format!("无法读取历史版本：{e}")).into_any_element(),
            None if h.rows.is_empty() => {
                text(&t, t.tx3, "这篇笔记还没有历史版本。保存一次后就会出现。").into_any_element()
            }
            None => ready_body(&t, app, cx),
        },
    };

    let mut footer: Vec<AnyElement> = vec![
        button(
            &t,
            "关闭",
            ButtonKind::Ghost,
            None,
            cx.listener(|this, _, _, cx| {
                close_modal(this);
                cx.notify();
            }),
        )
        .into_any_element(),
    ];
    if h.loaded.is_some() {
        footer.push(
            button(
                &t,
                "复制此版本文本",
                ButtonKind::Secondary,
                None,
                cx.listener(|this, _, _, cx| {
                    copy_selected(this, cx);
                }),
            )
            .into_any_element(),
        );
        footer.push(
            button(
                &t,
                "恢复此版本",
                ButtonKind::Primary,
                None,
                cx.listener(|this, _, _, cx| {
                    restore(this, cx);
                }),
            )
            .into_any_element(),
        );
    }

    modal(
        &t,
        &title,
        div()
            .w_full()
            .font_family(crate::ui::fonts::ui_name())
            .child(body),
        footer,
        820.0,
        cx.listener(|this, _, _, cx| {
            close_modal(this);
            cx.notify();
        }),
    )
}

/// Version list on the left, the selected version's diff on the right.
fn ready_body(t: &ActiveTheme, app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let h = &app.ext.history;
    let mut list = div()
        .flex()
        .flex_col()
        .w(px(300.0))
        .flex_none()
        .max_h(px(420.0))
        .overflow_hidden()
        .gap(px(2.0));
    for (i, row) in h.rows.iter().enumerate() {
        list = list.child(version_row(t, row, h.selected == Some(i), i, cx));
    }

    let mut pane = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx3)
                .child(if h.loading_version {
                    "正在读取…"
                } else if h.loaded.is_some() {
                    "与当前笔记相比：+ 恢复后会加入，− 恢复后会去掉"
                } else {
                    "选择左侧的版本查看差异"
                }),
        );
    if !h.note.is_empty() {
        pane = pane.child(text(t, t.st, &h.note));
    }
    if let Some(loaded) = &h.loaded {
        // Dimmed while a newer pick loads. The same element stays, so the scroll position is kept.
        let dim = if h.loading_version { 0.5 } else { 1.0 };
        pane = pane.child(div().w_full().opacity(dim).child(diff_view(t, loaded)));
    }

    div()
        .flex()
        .items_start()
        .gap(px(14.0))
        .child(list)
        .child(pane)
        .into_any_element()
}

/// One row of the version list: local time, reason, size and the change marks.
fn version_row(
    t: &ActiveTheme,
    row: &Row,
    selected: bool,
    index: usize,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let when = pure::local_time_text(row.time_ms);
    let marks = match row.counts {
        Some((added, removed)) => pure::change_marks(added, removed),
        None => "—".to_owned(),
    };
    let mut flags = div().flex().items_center().gap(px(4.0));
    let manual = row.kind == Kind::Manual || row.label.is_some();
    if manual {
        flags = flags.child(badge(t, pure::kind_label(row.kind), BadgeKind::Accent));
    } else {
        flags = flags.child(
            div()
                .text_size(px(11.5))
                .text_color(t.tx3)
                .child(SharedString::from(pure::kind_label(row.kind))),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(RADIUS_M))
        .cursor_pointer()
        .bg(if selected { t.active } else { t.sheet })
        .hover(|s| s.bg(t.hover))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(t.tx)
                .child(SharedString::from(when)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(t.tx3)
                        .child(SharedString::from(format!(
                            "{} · {marks}",
                            pure::bytes_text(row.size)
                        ))),
                )
                .child(flags),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                select(this, index, cx);
            }),
        )
}

/// Changed lines of the version: additions green, removals red, unchanged lines muted.
fn diff_view(t: &ActiveTheme, loaded: &Loaded) -> AnyElement {
    let mut lines = div()
        .flex()
        .flex_col()
        .id("history_diff")
        .max_h(px(380.0))
        .overflow_y_scroll()
        .p(px(8.0))
        .rounded(px(RADIUS_M))
        .border_1()
        .border_color(t.line_soft)
        .font_family(crate::ui::MONO_FONT)
        .text_size(px(12.0));
    if loaded.diff.iter().all(|l| l.tag == DiffTag::Equal) {
        lines = lines.child(div().text_color(t.tx3).child("此版本与当前笔记相同。"));
    }
    for line in loaded.diff.iter().take(DIFF_LIMIT) {
        let (mark, color) = match line.tag {
            DiffTag::Equal => (" ", t.tx3),
            DiffTag::Insert => ("+", t.ok),
            // `st` is not a red in every theme (ink uses a green), so removals use the danger red
            // of the other dialogs (see palette.rs and right_panel.rs).
            DiffTag::Delete => ("-", gpui::rgb(DELETED_RGB)),
        };
        let shown = if line.text.is_empty() {
            " ".to_owned()
        } else {
            line.text.clone()
        };
        lines = lines.child(
            div()
                .flex()
                .gap(px(8.0))
                .text_color(color)
                .child(SharedString::from(mark.to_owned()))
                .child(div().min_w_0().child(SharedString::from(shown))),
        );
    }
    if loaded.diff.len() > DIFF_LIMIT {
        lines = lines.child(div().text_color(t.tx3).child(SharedString::from(format!(
            "…… 还有 {} 行未显示",
            loaded.diff.len() - DIFF_LIMIT
        ))));
    }
    lines.into_any_element()
}

fn text(_t: &ActiveTheme, color: gpui::Rgba, value: &str) -> gpui::Div {
    div()
        .text_size(px(13.0))
        .text_color(color)
        .child(SharedString::from(value.to_owned()))
}
