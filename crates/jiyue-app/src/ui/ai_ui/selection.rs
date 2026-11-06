//! Selection actions: the Ctrl+J menu, the rewrite dialog entry point, and the result card with a
//! word-level diff. 接受 replaces the selection as one undo step; 拒绝 and Escape change nothing.
//!
//! Without a selection, 续写 appends to the end of the note and 总结 goes through `notes`.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ClipboardItem, Context, FontWeight, HighlightStyle, MouseButton,
    MouseDownEvent, SharedString, StrikethroughStyle, StyledText, Window, div, prelude::*, px, rgb,
};
use jiyue_ai::prompts::{self, Prompt};
use jiyue_ai::{AiError, CancelToken, ChatMessage};

use super::diff::{self, SegKind};
use super::{Modal, active_editor};
use super::{context, editing, effective_config, errors, input_clear, input_text, runner, worker};
use crate::RustdianApp;
use crate::ui::UI_FONT;
use crate::ui::ext::ToastKind;
use crate::ui::icons::icon;
use crate::ui::widgets::controls::PickIndex;
use crate::ui::widgets::{ButtonKind, MenuItem, button, context_menu};

/// Longest selection an action may take. Longer ones are refused with a message.
pub const MAX_ACTION_CHARS: usize = 8000;
/// Characters of the note given to 续写.
const TAIL_CHARS: usize = 4000;
/// Where the menu and the card open (window px). The selection itself is not measured.
const MENU_POS: (f32, f32) = (300.0, 140.0);
/// Window size used to keep the menu on screen. The window size is not known to this module.
const WINDOW_GUESS: (f32, f32) = (1600.0, 1000.0);
const CARD_WIDTH: f32 = 480.0;

/// Text an action works on, with where it goes back into the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub path: String,
    /// Byte range in the note. For a position (no selection) start equals end.
    pub start: usize,
    pub end: usize,
    /// The selected text, or the whole note for 续写 and 总结 without a selection.
    pub text: String,
    pub selected: bool,
}

/// One item of the selection menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Rewrite,
    Shorten,
    Expand,
    TranslateEn,
    TranslateZh,
    FixGrammar,
    Summarize,
    Continue,
    Explain,
}

/// Menu order.
pub const MENU_ITEMS: [Action; 9] = [
    Action::Rewrite,
    Action::Shorten,
    Action::Expand,
    Action::TranslateEn,
    Action::TranslateZh,
    Action::FixGrammar,
    Action::Summarize,
    Action::Continue,
    Action::Explain,
];

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Rewrite => "改写…",
            Action::Shorten => "缩写",
            Action::Expand => "扩写",
            Action::TranslateEn => "翻译为英文",
            Action::TranslateZh => "翻译为中文",
            Action::FixGrammar => "修正语法",
            Action::Summarize => "总结",
            Action::Continue => "续写",
            Action::Explain => "解释",
        }
    }

    /// Actions that need a selection. 续写 and 总结 also work on the whole note.
    pub fn needs_selection(self) -> bool {
        !matches!(self, Action::Continue | Action::Summarize)
    }
}

/// Builds the prompt for an action. `instruction` is used by 改写 only.
pub fn build_prompt(action: Action, text: &str, instruction: Option<&str>) -> Prompt {
    match action {
        Action::Rewrite => prompts::rewrite(text, instruction.unwrap_or("")),
        Action::Shorten => prompts::shorten(text),
        Action::Expand => prompts::expand(text),
        Action::TranslateEn => prompts::translate(text, "English"),
        Action::TranslateZh => prompts::translate(text, "简体中文"),
        Action::FixGrammar => prompts::fix_grammar(text),
        Action::Summarize => prompts::summarize(text),
        Action::Continue => prompts::continue_writing(&last_chars(text, TAIL_CHARS), None),
        Action::Explain => prompts::explain(text),
    }
}

/// The last `n` characters of `text`.
pub fn last_chars(text: &str, n: usize) -> String {
    let total = text.chars().count();
    if total <= n {
        text.to_string()
    } else {
        text.chars().skip(total - n).collect()
    }
}

/// The selection menu.
pub struct Menu {
    pub pos: (f32, f32),
    pub target: Target,
}

/// The floating result of one action.
pub struct Card {
    pub id: u64,
    pub action: Action,
    pub target: Target,
    pub instruction: Option<String>,
    pub output: String,
    pub streaming: bool,
    pub error: Option<AiError>,
    pub request: Vec<ChatMessage>,
    pub cancel: Option<CancelToken>,
}

impl Card {
    /// 续写 adds text at the end of the note; the other actions replace the target.
    pub fn replaces(&self) -> bool {
        self.action != Action::Continue
    }
}

/// Text of the diff with the ranges that changed, for `StyledText`.
pub fn diff_runs(segs: &[diff::Seg]) -> (String, Vec<(Range<usize>, SegKind)>) {
    let mut text = String::new();
    let mut runs = Vec::new();
    for s in segs {
        let start = text.len();
        text.push_str(&s.text);
        if s.kind != SegKind::Same {
            runs.push((start..text.len(), s.kind));
        }
    }
    (text, runs)
}

/// Ctrl+J and the palette: opens the menu at the selection (or the note's end, without one).
pub fn open_menu(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if !app.ext.ai.config.enabled {
        app.toast(ToastKind::Warn, "AI 功能未开启，请先在 设置 → AI 中开启");
        return;
    }
    let Some((path, editor)) = active_editor(app) else {
        app.toast(ToastKind::Info, "请先打开一篇笔记");
        return;
    };
    let cfg = effective_config(&app.ext.ai);
    if let Some(note) = app.vault.active_note() {
        let tags = super::exclusion_tags(note.content(), &note.tags);
        if cfg.is_note_excluded(&note.path, &tags) {
            app.toast(ToastKind::Warn, "该笔记已排除，不能使用 AI 操作");
            return;
        }
    }
    let ed = editor.read(cx);
    let text = ed.text();
    let (anchor, head) = ed.selection_offsets();
    let target = match context::selection_slice(&text, anchor, head) {
        Some((range, body)) => Target {
            path,
            start: range.start,
            end: range.end,
            text: body,
            selected: true,
        },
        None => Target {
            path,
            start: text.len(),
            end: text.len(),
            text,
            selected: false,
        },
    };
    app.ext.ai.menu = Some(Menu {
        pos: MENU_POS,
        target,
    });
    cx.notify();
}

/// Renders the menu. Actions that need a selection are greyed out without one.
pub fn render_menu(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let t = app.theme;
    let Some(menu) = app.ext.ai.menu.as_ref() else {
        return Vec::new();
    };
    let items: Vec<MenuItem> = MENU_ITEMS
        .iter()
        .map(|a| {
            let item = MenuItem::new(a.label());
            if a.needs_selection() && !menu.target.selected {
                item.disabled()
            } else {
                item
            }
        })
        .collect();
    let pick: PickIndex = Rc::new(cx.listener(|this, i: &usize, window, cx| {
        menu_pick(this, *i, window, cx);
    }));
    let close = cx.listener(|this, _: &MouseDownEvent, _window, cx| {
        this.ext.ai.menu = None;
        cx.notify();
    });
    vec![context_menu(
        &t,
        &items,
        None,
        menu.pos,
        WINDOW_GUESS,
        pick,
        close,
    )]
}

fn menu_pick(
    app: &mut RustdianApp,
    idx: usize,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    let Some(menu) = app.ext.ai.menu.take() else {
        return;
    };
    let Some(action) = MENU_ITEMS.get(idx).copied() else {
        return;
    };
    if action.needs_selection() && !menu.target.selected {
        return;
    }
    match action {
        Action::Rewrite => {
            app.ext.ai.target = Some(menu.target);
            app.ext.ai.modal = Modal::Rewrite;
            // Focus the instruction field once this click has finished (as the widget demo does).
            if let Some(input) = app.ext.ai.inputs.rewrite.clone() {
                window.on_next_frame(move |window, cx| input.read(cx).focus(window));
            }
        }
        Action::Summarize if !menu.target.selected => {
            super::notes::ask_summarize_target(app, menu.target);
        }
        _ => {
            if menu.target.selected && menu.target.text.chars().count() > MAX_ACTION_CHARS {
                app.toast(
                    ToastKind::Warn,
                    "选中的文字太长（超过 8000 字），请分段选择",
                );
                return;
            }
            start_action(app, action, menu.target, None, cx);
        }
    }
    cx.notify();
}

/// Palette command 改写选区: needs a selection, then asks for the instruction.
pub fn start_rewrite(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if !app.ext.ai.config.enabled {
        app.toast(ToastKind::Warn, "AI 功能未开启，请先在 设置 → AI 中开启");
        return;
    }
    match editing::selection(app, cx) {
        Some(target) => {
            app.ext.ai.target = Some(target);
            app.ext.ai.modal = Modal::Rewrite;
            cx.notify();
        }
        None => app.toast(ToastKind::Info, "请先选中要改写的文字"),
    }
}

/// The 确认 button of the rewrite dialog (also Enter in its field).
pub fn confirm_rewrite_from_modal(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let instruction = input_text(&app.ext.ai.inputs.rewrite, cx)
        .trim()
        .to_string();
    if instruction.is_empty() {
        app.toast(ToastKind::Warn, "请输入改写要求");
        return;
    }
    let Some(target) = app.ext.ai.target.take() else {
        app.ext.ai.modal = Modal::None;
        return;
    };
    app.ext.ai.modal = Modal::None;
    input_clear(&app.ext.ai.inputs.rewrite, cx);
    start_action(app, Action::Rewrite, target, Some(instruction), cx);
    cx.notify();
}

/// Starts an action: builds the prompt, shows the card, and streams the answer into it.
pub fn start_action(
    app: &mut RustdianApp,
    action: Action,
    target: Target,
    instruction: Option<String>,
    cx: &mut Context<RustdianApp>,
) {
    if !app.ext.ai.config.enabled || effective_config(&app.ext.ai).chat_provider().is_none() {
        app.toast(ToastKind::Warn, "请先在 设置 → AI 中开启 AI 并选择对话服务");
        return;
    }
    if let Some(prev) = app.ext.ai.card.as_mut()
        && let Some(c) = prev.cancel.take()
    {
        c.cancel();
    }
    let prompt = build_prompt(action, &target.text, instruction.as_deref());
    let id = app.ext.ai.take_id();
    app.ext.ai.card = Some(Card {
        id,
        action,
        target,
        instruction,
        output: String::new(),
        streaming: false,
        error: None,
        request: prompt.messages.clone(),
        cancel: None,
    });
    stream_card(app, id, prompt.messages, cx);
    cx.notify();
}

/// Streams the answer into card `id`.
fn stream_card(
    app: &mut RustdianApp,
    id: u64,
    messages: Vec<ChatMessage>,
    cx: &mut Context<RustdianApp>,
) {
    let cfg = effective_config(&app.ext.ai);
    let Some(entry) = cfg.chat_provider().cloned() else {
        return;
    };
    let cancel = CancelToken::new();
    if let Some(card) = card_mut(app, id) {
        card.streaming = true;
        card.error = None;
        card.output.clear();
        card.cancel = Some(cancel.clone());
    }
    let rx = worker::spawn_chat(entry, app.ext.ai.test.clone(), messages, cancel);
    runner::pump(cx, rx, move |app, msg, _cx| match msg {
        worker::ChatMsg::Delta(d) => {
            if let Some(card) = card_mut(app, id) {
                card.output.push_str(&d);
            }
            true
        }
        worker::ChatMsg::Done(result) => {
            finish_card(app, id, result);
            false
        }
    });
}

fn card_mut(app: &mut RustdianApp, id: u64) -> Option<&mut Card> {
    app.ext.ai.card.as_mut().filter(|c| c.id == id)
}

fn finish_card(app: &mut RustdianApp, id: u64, result: Result<jiyue_ai::ChatResponse, AiError>) {
    let Some(card) = card_mut(app, id) else {
        return;
    };
    card.streaming = false;
    card.cancel = None;
    match result {
        Ok(resp) => {
            let full = if resp.content.trim().is_empty() {
                std::mem::take(&mut card.output)
            } else {
                resp.content
            };
            card.output = prompts::clean_output(&full);
        }
        // Keep the partial output so the user can still accept it.
        Err(AiError::Cancelled) => {}
        Err(e) => card.error = Some(e),
    }
}

/// 重试 on the card: the same request, replacing the output.
pub fn retry(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(card) = app.ext.ai.card.as_ref() else {
        return;
    };
    if card.streaming {
        return;
    }
    let (id, request) = (card.id, card.request.clone());
    stream_card(app, id, request, cx);
    cx.notify();
}

/// 停止 on the card.
pub fn stop(app: &mut RustdianApp) {
    if let Some(cancel) = app.ext.ai.card.as_ref().and_then(|c| c.cancel.clone()) {
        cancel.cancel();
    }
}

/// 拒绝, the close button, and Escape: nothing in the note changes.
pub fn close_overlays(app: &mut RustdianApp) {
    app.ext.ai.menu = None;
    if let Some(card) = app.ext.ai.card.take()
        && let Some(c) = card.cancel
    {
        c.cancel();
    }
}

/// 接受: replaces the target as one undo step (续写 appends at the end of the note).
pub fn accept(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(card) = app.ext.ai.card.as_ref() else {
        return;
    };
    if card.streaming || card.error.is_some() || card.output.trim().is_empty() {
        app.toast(ToastKind::Warn, "结果还没有生成完成");
        return;
    }
    let output = card.output.clone();
    let target = card.target.clone();
    let replaces = card.replaces();
    let original = target.text.clone();
    let outcome = if replaces {
        let expected = if target.selected {
            Some(original.as_str())
        } else {
            None
        };
        editing::apply_target(app, &target, expected, &output, cx)
    } else {
        editing::append_to_note(app, &target.path, &output, cx)
    };
    match outcome {
        Ok(()) => {
            if let Some(card) = app.ext.ai.card.take()
                && let Some(c) = card.cancel
            {
                c.cancel();
            }
            app.toast(ToastKind::Success, "已应用到笔记");
        }
        Err(msg) => app.toast(ToastKind::Error, msg),
    }
    cx.notify();
}

/// 复制 on the card.
pub fn copy_output(app: &RustdianApp, cx: &mut App) {
    if let Some(card) = app.ext.ai.card.as_ref() {
        cx.write_to_clipboard(ClipboardItem::new_string(card.output.clone()));
    }
}

/// Renders the card near the editor.
pub fn render_card(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let Some(card) = app.ext.ai.card.as_ref() else {
        return div().into_any_element();
    };
    let status: (String, gpui::Rgba) = if card.streaming {
        ("生成中…".into(), t.tx3)
    } else if let Some(e) = &card.error {
        (errors::describe(e), rgb(0xe5484d))
    } else if card.output.trim().is_empty() {
        ("没有结果".into(), t.tx3)
    } else {
        ("完成：接受后写入笔记".into(), t.tx3)
    };
    let title = match card.action {
        Action::Continue => format!("{}（末尾）", card.action.label()),
        Action::Rewrite => match &card.instruction {
            Some(i) => format!("改写：{}", i.chars().take(20).collect::<String>()),
            None => card.action.label().to_string(),
        },
        other => other.label().to_string(),
    };

    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(40.0))
        .px(px(14.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(icon("bulb", 15.0, t.ac))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(title)),
                ),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(status.1)
                .child(SharedString::from(status.0)),
        );

    let body_text: AnyElement = if card.output.is_empty() {
        div()
            .text_color(t.tx3)
            .child(if card.streaming {
                "等待回答…"
            } else {
                ""
            })
            .into_any_element()
    } else {
        let segs = diff::word_diff(&card.target.text, &card.output);
        let (text, runs) = diff_runs(&segs);
        let highlights: Vec<(Range<usize>, HighlightStyle)> = runs
            .into_iter()
            .map(|(range, kind)| {
                let style = match kind {
                    SegKind::Removed => HighlightStyle {
                        color: Some(rgb(0xe5484d).into()),
                        strikethrough: Some(StrikethroughStyle {
                            thickness: px(1.0),
                            color: Some(rgb(0xe5484d).into()),
                        }),
                        ..Default::default()
                    },
                    _ => HighlightStyle {
                        background_color: Some(t.ok_soft.into()),
                        ..Default::default()
                    },
                };
                (range, style)
            })
            .collect();
        let styled = StyledText::new(SharedString::from(text)).with_highlights(highlights);
        div()
            .text_color(t.tx)
            .line_height(px(20.0))
            .child(styled)
            .into_any_element()
    };

    let mut footer = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.0))
        .h(px(52.0))
        .px(px(12.0))
        .border_t_1()
        .border_color(t.line_soft)
        .bg(t.hover);
    footer = footer.child(button(
        &t,
        "拒绝",
        ButtonKind::Secondary,
        None,
        cx.listener(|this, _: &MouseDownEvent, _, cx| {
            close_overlays(this);
            cx.notify();
        }),
    ));
    if card.streaming {
        footer = footer.child(button(
            &t,
            "停止",
            ButtonKind::Secondary,
            Some("square"),
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                stop(this);
                cx.notify();
            }),
        ));
    } else {
        footer = footer.child(button(
            &t,
            "重试",
            ButtonKind::Ghost,
            None,
            cx.listener(|this, _: &MouseDownEvent, _, cx| retry(this, cx)),
        ));
        footer = footer.child(button(
            &t,
            "复制",
            ButtonKind::Ghost,
            Some("copy"),
            cx.listener(|this, _: &MouseDownEvent, _, cx| copy_output(this, cx)),
        ));
        footer = footer.child(button(
            &t,
            "接受",
            ButtonKind::Primary,
            Some("check"),
            cx.listener(|this, _: &MouseDownEvent, _, cx| accept(this, cx)),
        ));
    }

    div()
        .absolute()
        .left(px(300.0))
        .top(px(120.0))
        .w(px(CARD_WIDTH))
        .flex()
        .flex_col()
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .rounded(px(12.0))
        .shadow_lg()
        .overflow_hidden()
        .font_family(UI_FONT)
        .text_size(px(13.0))
        .text_color(t.tx)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(header)
        .child(
            div()
                .id("ai-card-body")
                .max_h(px(300.0))
                .overflow_y_scroll()
                .px(px(14.0))
                .py(px(10.0))
                .child(body_text),
        )
        .child(footer)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_marks_selection_actions_and_keeps_continue_and_summary_open() {
        assert!(Action::Shorten.needs_selection());
        assert!(!Action::Continue.needs_selection());
        assert!(!Action::Summarize.needs_selection());
        assert_eq!(MENU_ITEMS.len(), 9);
        let labels: Vec<&str> = MENU_ITEMS.iter().map(|a| a.label()).collect();
        assert_eq!(labels[0], "改写…");
    }

    #[test]
    fn last_chars_takes_the_tail() {
        assert_eq!(last_chars("一二三四五", 2), "四五");
        assert_eq!(last_chars("abc", 10), "abc");
    }

    #[test]
    fn prompts_carry_the_selected_text_and_the_instruction() {
        let p = build_prompt(Action::Rewrite, "原文内容", Some("更简洁"));
        let user = p
            .messages
            .iter()
            .find(|m| m.role == jiyue_ai::ChatRole::User)
            .expect("user");
        assert!(user.content.contains("原文内容"));
        assert!(user.content.contains("更简洁"));
        let c = build_prompt(Action::Continue, &"字".repeat(TAIL_CHARS + 50), None);
        assert!(c.preview.total_chars > 0);
    }

    #[test]
    fn diff_runs_point_at_changed_text_only() {
        let segs = diff::word_diff("今天很好", "今天非常好");
        let (text, runs) = diff_runs(&segs);
        // Removed text stays in the run text, followed by the added text.
        assert_eq!(text, "今天很非常好");
        let changed: Vec<&str> = runs.iter().map(|(r, _)| &text[r.clone()]).collect();
        assert_eq!(changed, vec!["很", "非常"]);
        for (r, kind) in &runs {
            assert!(r.end <= text.len());
            assert_ne!(*kind, SegKind::Same);
        }
    }
}
