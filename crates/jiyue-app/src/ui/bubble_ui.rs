//! Floating inline format bubble toolbar over non-empty editor selections.

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, MouseButton, SharedString, div, prelude::*, px,
};

use super::ext::{ExtCommand, PaletteCommand};
use super::icons::icon;
use super::text_input::InlineFormat;
use crate::{AppCommand, RustdianApp};

const MARGIN: f32 = 8.0;
const GAP: f32 = 6.0;
const BAR_H: f32 = 32.0;
const BAR_FULL_W: f32 = 212.0;
const BAR_AI_W: f32 = 40.0;
const BTN_W: f32 = 26.0;
const BTN_H: f32 = 24.0;

#[derive(Clone, Copy)]
struct FormatBtn {
    fmt: InlineFormat,
    kind: BtnVisual,
}

#[derive(Clone, Copy)]
enum BtnVisual {
    Bold,
    Italic,
    Strike,
    Highlight,
    Icon(&'static str),
}

const FORMAT_BTNS: [FormatBtn; 6] = [
    FormatBtn {
        fmt: InlineFormat::Bold,
        kind: BtnVisual::Bold,
    },
    FormatBtn {
        fmt: InlineFormat::Italic,
        kind: BtnVisual::Italic,
    },
    FormatBtn {
        fmt: InlineFormat::Strikethrough,
        kind: BtnVisual::Strike,
    },
    FormatBtn {
        fmt: InlineFormat::Highlight,
        kind: BtnVisual::Highlight,
    },
    FormatBtn {
        fmt: InlineFormat::InlineCode,
        kind: BtnVisual::Icon("code"),
    },
    FormatBtn {
        fmt: InlineFormat::Wikilink,
        kind: BtnVisual::Icon("link"),
    },
];

/// Renders the floating selection toolbar overlay when an editable selection is active.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    if super::ext::overlay_open(app)
        || app.ext.slash.any_open()
        || app.ext.ai.menu.is_some()
        || app.ext.ai.card.is_some()
    {
        return Vec::new();
    }

    let Some((_, editor)) = super::ai_ui::active_editor(app) else {
        return Vec::new();
    };
    let (anchor, can_wrap, active) = {
        let ed = editor.read(cx);
        (
            ed.bubble_anchor(),
            ed.can_wrap_selection(),
            ed.active_inline_formats(),
        )
    };
    let Some((wx, wy, rw, rh, view_top, _view_bottom)) = anchor else {
        return Vec::new();
    };
    let ai_enabled = app.ext.ai.config.enabled;
    if !can_wrap && !ai_enabled {
        return Vec::new();
    }

    let t = app.theme;
    let bar_w = if can_wrap { BAR_FULL_W } else { BAR_AI_W };
    let (win_w, win_h) = app.ext.slash.viewport_size();
    let ideal_x = wx + rw / 2.0 - bar_w / 2.0;
    let left = ideal_x.clamp(MARGIN, (win_w - bar_w - MARGIN).max(MARGIN));
    let above_y = wy - BAR_H - GAP;
    let below_y = wy + rh + GAP;
    let top = if above_y >= view_top + GAP {
        above_y
    } else {
        below_y
    }
    .clamp(MARGIN, (win_h - BAR_H - MARGIN).max(MARGIN));

    let mut bar = div()
        .id("bubble_toolbar")
        .absolute()
        .left(px(left))
        .top(px(top))
        .h(px(BAR_H))
        .px(px(6.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.raised)
        .shadow_md()
        .font_family(crate::ui::fonts::ui_name())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    if can_wrap {
        for (i, item) in FORMAT_BTNS.iter().enumerate() {
            let is_active = active[i];
            let fg = if is_active { t.ac } else { t.tx2 };
            let fmt = item.fmt;
            let mut btn = div()
                .id(("bubble_fmt", i))
                .w(px(BTN_W))
                .h(px(BTN_H))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(fg)
                .when(is_active, |d| d.bg(t.ac_soft))
                .when(!is_active, |d| d.hover(|s| s.bg(t.hover).text_color(t.tx)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if let Some((_, editor)) = super::ai_ui::active_editor(this) {
                            editor.update(cx, |ed, cx| {
                                ed.toggle_inline_format(fmt, cx);
                                ed.focus(window);
                            });
                        }
                        cx.notify();
                    }),
                );

            btn = match item.kind {
                BtnVisual::Bold => btn.font_weight(FontWeight::BOLD).child("B"),
                BtnVisual::Italic => btn.italic().child("I"),
                BtnVisual::Strike => btn.line_through().child("S"),
                BtnVisual::Highlight => btn
                    .font_family(crate::ui::MONO_FONT)
                    .text_size(px(11.0))
                    .child(SharedString::from("==")),
                BtnVisual::Icon(name) => btn.child(icon(name, 14.0, fg)),
            };
            bar = bar.child(btn);
        }

        bar = bar.child(div().w(px(1.0)).h(px(16.0)).mx(px(2.0)).bg(t.line_soft));
    }

    let ai_btn = div()
        .id("bubble_ai_menu")
        .w(px(BTN_W))
        .h(px(BTN_H))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.dispatch_ext(ExtCommand::Ai(super::ai_ui::Command::ActionMenu), cx);
                cx.notify();
            }),
        )
        .child(icon("sparkles", 14.0, t.ac));

    bar = bar.child(ai_btn);
    vec![bar.into_any_element()]
}

/// Format commands contributed to the command palette.
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "格式：加粗",
            icon: "pencil",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Format(InlineFormat::Bold)),
        },
        PaletteCommand {
            label: "格式：斜体",
            icon: "pencil",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Format(InlineFormat::Italic)),
        },
        PaletteCommand {
            label: "格式：删除线",
            icon: "pencil",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Format(InlineFormat::Strikethrough)),
        },
        PaletteCommand {
            label: "格式：高亮",
            icon: "pencil",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Format(InlineFormat::Highlight)),
        },
        PaletteCommand {
            label: "格式：行内代码",
            icon: "code",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Format(InlineFormat::InlineCode)),
        },
        PaletteCommand {
            label: "格式：转为双链",
            icon: "link",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Format(InlineFormat::Wikilink)),
        },
    ]
}
