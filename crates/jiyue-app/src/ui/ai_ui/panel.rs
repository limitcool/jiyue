//! The AI side panel: a drawer over the right side of the editor. Header, message list (chat and
//! note results), the preview card that must be confirmed before anything is sent, and the composer
//! with source chips.

use super::chat::{Kind, Preview, Role, UiMessage};
use super::markdown::{self, Block};
use super::{editing, effective_config, errors};
use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::icons::icon;
use crate::ui::widgets::{ButtonKind, button, icon_button};
use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, Div, ElementId, FontWeight, MouseButton,
    MouseDownEvent, SharedString, Window, div, prelude::*, px,
};

/// Top and bottom offsets of the drawer, under the tab strip and above the status bar.
const PANEL_TOP: f32 = 92.0;
const PANEL_BOTTOM: f32 = 44.0;
const PANEL_WIDTH: f32 = 380.0;

/// Renders the drawer.
pub fn render(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let s = &app.ext.ai;
    let cfg = effective_config(s);
    let model_line = match cfg.chat_provider() {
        Some(p) => format!("{} · {}", p.name, p.chat_model),
        None if s.config.enabled => "尚未选择对话服务".to_string(),
        None => "AI 功能未开启".to_string(),
    };

    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(46.0))
        .px(px(14.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("AI 助手"))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .child(model_line),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(button(
                    &t,
                    "清空",
                    ButtonKind::Ghost,
                    None,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        super::chat::clear(this);
                        cx.notify();
                    }),
                ))
                .child(icon_button(
                    &t,
                    "x",
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.ext.ai.panel_open = false;
                        cx.notify();
                    }),
                )),
        );

    let mut list = div()
        .id("ai-messages")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&s.scroll)
        .px(px(12.0))
        .py(px(10.0))
        .flex()
        .flex_col()
        .gap(px(14.0));
    if !s.config.enabled {
        list = list.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(t.sheet)
                .text_color(t.tx2)
                .child("AI 功能默认关闭。开启后，提问前会先显示将发送的内容。")
                .child(button(
                    &t,
                    "打开 AI 设置",
                    ButtonKind::Secondary,
                    Some("settings"),
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        super::settings::open_settings(this);
                        cx.notify();
                    }),
                )),
        );
    } else if s.messages.is_empty() {
        list = list.child(
            div()
                .text_color(t.tx3)
                .child("问点什么。可以勾选“当前笔记”“选中文字”或“全库检索”作为依据。选中文字后按 Ctrl+J 可以改写、缩写、翻译等。"),
        );
    }
    for m in &s.messages {
        list = list.child(render_message(app, m, cx));
    }

    let mut body = div().flex().flex_col().flex_1().min_h_0().child(list);
    if let Some(preview) = &s.preview {
        body = body.child(render_preview(app, preview, cx));
    }
    if let Some(warn) = s.preview.as_ref().and_then(|p| p.warning.clone()) {
        body = body.child(
            div()
                .px(px(14.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child(warn),
        );
    }

    div()
        .absolute()
        .top(px(PANEL_TOP))
        .bottom(px(PANEL_BOTTOM))
        .right(px(12.0))
        .w(px(PANEL_WIDTH))
        .flex()
        .flex_col()
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .rounded(px(12.0))
        .shadow_lg()
        .overflow_hidden()
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .text_color(t.tx)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(header)
        .child(body)
        .child(composer(app, cx))
        .into_any_element()
}

fn composer(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let s = &app.ext.ai;
    let chips = s.chips;
    let note_excluded = app
        .vault
        .active_note()
        .map(|n| {
            let tags = super::exclusion_tags(n.content(), &n.tags);
            effective_config(s).is_note_excluded(&n.path, &tags)
        })
        .unwrap_or(false);
    let note_label = if note_excluded {
        "当前笔记（已排除）"
    } else {
        "当前笔记"
    };
    let chip_row = div()
        .flex()
        .flex_wrap()
        .gap(px(6.0))
        .child(chip(
            &t,
            note_label,
            chips.note && !note_excluded,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.ext.ai.chips.note = !this.ext.ai.chips.note;
                cx.notify();
            }),
        ))
        .child(chip(
            &t,
            "选中文字",
            chips.selection,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.ext.ai.chips.selection = !this.ext.ai.chips.selection;
                cx.notify();
            }),
        ))
        .child(chip(
            &t,
            "全库检索",
            chips.vault,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.ext.ai.chips.vault = !this.ext.ai.chips.vault;
                cx.notify();
            }),
        ));

    let send: AnyElement = if s.busy.is_some() {
        button(
            &t,
            "停止",
            ButtonKind::Secondary,
            Some("square"),
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                super::chat::stop(this);
                cx.notify();
            }),
        )
        .into_any_element()
    } else {
        let label = if s.preparing {
            "准备中…"
        } else {
            "发送"
        };
        button(
            &t,
            label,
            ButtonKind::Primary,
            Some("corner-down-left"),
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                super::chat::send_draft(this, cx);
            }),
        )
        .into_any_element()
    };

    let configured = effective_config(s)
        .chat_provider()
        .map(|p| p.chat_model.clone());
    let mut footer = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_t_1()
        .border_color(t.line_soft)
        .child(chip_row)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().flex_1().min_w_0().children(s.inputs.draft.clone()))
                .child(send),
        );
    if let Some(note) = answered_note(configured.as_deref(), s.last_answered_by.as_deref()) {
        footer = footer.child(
            div()
                .text_size(px(11.0))
                .text_color(t.tx3)
                .child(SharedString::from(note)),
        );
    }
    footer
}

/// A toggle chip under the composer.
fn chip(
    t: &ActiveTheme,
    label: &str,
    on: bool,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let mut el = div()
        .flex()
        .items_center()
        .h(px(24.0))
        .px(px(10.0))
        .rounded(px(12.0))
        .border_1()
        .text_size(px(12.0))
        .cursor_pointer()
        .child(label.to_string())
        .on_mouse_down(MouseButton::Left, on_click);
    el = if on {
        el.bg(t.ac_soft).border_color(t.ac).text_color(t.ac)
    } else {
        el.border_color(t.line)
            .text_color(t.tx3)
            .hover(|s| s.bg(t.hover))
    };
    el
}

/// Small text button used in message action rows.
fn small_button(
    t: &ActiveTheme,
    label: &str,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(24.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(t.line)
        .text_size(px(12.0))
        .text_color(t.tx2)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .child(label.to_string())
        .on_mouse_down(MouseButton::Left, on_click)
}

/// A `[[note]]` citation chip. Clicking it opens the note.
fn cite_chip(t: &ActiveTheme, title: &str, cx: &mut Context<RustdianApp>) -> Div {
    let target = title.to_string();
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .h(px(22.0))
        .px(px(8.0))
        .rounded(px(11.0))
        .bg(t.sheet)
        .border_1()
        .border_color(t.line_soft)
        .text_size(px(11.5))
        .text_color(t.ac)
        .cursor_pointer()
        .child(icon("link", 11.0, t.ac))
        .child(title.to_string())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                this.open_note_by_title(&target);
                cx.notify();
            }),
        )
}

/// 思考过程: the model's reasoning, collapsed by default. The header toggles it. It is never part of
/// the answer text.
fn reasoning_block(
    t: &ActiveTheme,
    id: u64,
    open: bool,
    reasoning: &str,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let header = div()
        .id(ElementId::NamedInteger(
            SharedString::from("reasoning-toggle"),
            id,
        ))
        .flex()
        .items_center()
        .gap(px(6.0))
        .cursor_pointer()
        .text_size(px(11.5))
        .text_color(t.tx3)
        .child(icon(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            12.0,
            t.tx3,
        ))
        .child("思考过程")
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            if let Some(m) = super::chat::message_mut(this, id) {
                m.reasoning_open = !m.reasoning_open;
            }
            cx.notify();
        }));
    let mut block = div().flex().flex_col().gap(px(4.0)).child(header);
    if open {
        block = block.child(
            div()
                .p(px(8.0))
                .rounded(px(6.0))
                .bg(t.sheet)
                .text_size(px(12.0))
                .text_color(t.tx2)
                .child(SharedString::from(reasoning.to_string())),
        );
    }
    block.into_any_element()
}

/// The footer note naming the model that answered, when the server reports a name other than the
/// configured one (gateways alias model names). `None` when they match or nothing answered yet.
pub(crate) fn answered_note(configured: Option<&str>, answered: Option<&str>) -> Option<String> {
    let answered = answered.map(str::trim).filter(|a| !a.is_empty())?;
    if configured.map(str::trim) == Some(answered) {
        return None;
    }
    Some(format!("实际回答的模型：{answered}"))
}

fn render_message(app: &RustdianApp, m: &UiMessage, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    if m.role == Role::User {
        return div()
            .flex()
            .justify_end()
            .child(
                div()
                    .max_w(px(300.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(10.0))
                    .bg(t.ac_soft)
                    .text_color(t.tx)
                    .child(m.text.clone()),
            )
            .into_any_element();
    }
    let id = m.id;
    let mut col = div().flex().flex_col().gap(px(8.0));
    match &m.kind {
        Kind::Chat => {
            if m.text.is_empty() && m.streaming {
                col = col.child(div().text_color(t.tx3).child("生成中…"));
            } else {
                col = col.child(render_blocks(app, &m.text, cx));
            }
            if m.streaming && !m.text.is_empty() {
                col = col.child(div().text_size(px(11.5)).text_color(t.tx3).child("生成中…"));
            }
            if let Some(reasoning) = m.reasoning.as_ref().filter(|r| !r.trim().is_empty()) {
                col = col.child(reasoning_block(&t, id, m.reasoning_open, reasoning, cx));
            }
            if m.stopped {
                col = col.child(div().text_size(px(11.5)).text_color(t.tx3).child("已停止"));
            }
            if let Some(e) = &m.error {
                col = col.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .p(px(10.0))
                        .rounded(px(8.0))
                        .bg(t.sheet)
                        .child(
                            div()
                                .text_color(gpui::rgb(0xe5484d))
                                .child(errors::describe(e)),
                        )
                        .when(errors::can_retry(e), |d| {
                            d.child(div().flex().child(small_button(
                                &t,
                                "重试",
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    super::chat::retry(this, id, cx);
                                }),
                            )))
                        }),
                );
            }
            // The notes this answer was built from, unless the answer cites them inline already.
            if !m.sources.is_empty() && markdown::citations(&m.text).is_empty() {
                let mut row = div().flex().flex_wrap().gap(px(6.0));
                for path in m.sources.iter().take(8) {
                    row = row.child(cite_chip(&t, &crate::ui::ai_ui::title_of(path), cx));
                }
                col = col.child(row);
            }
            if !m.streaming && !m.text.is_empty() {
                let text = m.text.clone();
                let mut row = div().flex().flex_wrap().gap(px(6.0));
                let copy_text = text.clone();
                row = row.child(small_button(
                    &t,
                    "复制",
                    cx.listener(move |_, _: &MouseDownEvent, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                    }),
                ));
                let append_text = text.clone();
                row = row.child(small_button(
                    &t,
                    "插入到笔记末尾",
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        if let Some(path) = this.active_path().map(str::to_string) {
                            let result = editing::append_to_note(this, &path, &append_text, cx);
                            if let Err(msg) = result {
                                this.toast(crate::ui::ext::ToastKind::Error, msg);
                            }
                        }
                        cx.notify();
                    }),
                ));
                let replace_text = text.clone();
                row = row.child(small_button(
                    &t,
                    "替换选区",
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        match editing::selection(this, cx) {
                            Some(target) => {
                                let expected = target.text.clone();
                                if let Err(msg) = editing::apply_target(
                                    this,
                                    &target,
                                    Some(&expected),
                                    &replace_text,
                                    cx,
                                ) {
                                    this.toast(crate::ui::ext::ToastKind::Error, msg);
                                }
                            }
                            None => this.toast(
                                crate::ui::ext::ToastKind::Info,
                                "请先在笔记里选中要替换的文字",
                            ),
                        }
                        cx.notify();
                    }),
                ));
                row = row.child(small_button(
                    &t,
                    "重新生成",
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        super::chat::regenerate(this, id, cx);
                    }),
                ));
                col = col.child(row);
            }
        }
        Kind::Title => {
            col = col.child(
                div()
                    .text_size(px(11.5))
                    .text_color(t.tx3)
                    .child("建议标题"),
            );
            col = col.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(15.0))
                    .child(m.text.clone()),
            );
            let text = m.text.clone();
            let path = m.path.clone();
            let mut row = div().flex().flex_wrap().gap(px(6.0));
            let copy = text.clone();
            row = row.child(small_button(
                &t,
                "复制",
                cx.listener(move |_, _: &MouseDownEvent, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                }),
            ));
            row = row.child(small_button(
                &t,
                "插入为标题",
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    if let Some(p) = path.clone() {
                        super::notes::insert_title(this, &p, &text, cx);
                    }
                }),
            ));
            col = col.child(row);
        }
        Kind::Tags(tags) => {
            col = col.child(
                div()
                    .text_size(px(11.5))
                    .text_color(t.tx3)
                    .child("建议标签（点击添加到笔记属性）"),
            );
            let mut row = div().flex().flex_wrap().gap(px(6.0));
            for tag in tags {
                let tag_name = tag.clone();
                let path = m.path.clone();
                row = row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .h(px(24.0))
                        .px(px(10.0))
                        .rounded(px(12.0))
                        .bg(t.ac_soft)
                        .text_color(t.ac)
                        .text_size(px(12.0))
                        .cursor_pointer()
                        .child(icon("tag", 12.0, t.ac))
                        .child(format!("#{tag}"))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                if let Some(p) = path.clone() {
                                    super::notes::add_tag(this, &p, &tag_name, cx);
                                }
                            }),
                        ),
                );
            }
            col = col.child(row);
        }
        Kind::Summary => {
            col = col.child(div().text_size(px(11.5)).text_color(t.tx3).child("摘要"));
            col = col.child(render_blocks(app, &m.text, cx));
            let text = m.text.clone();
            let path = m.path.clone();
            let mut row = div().flex().flex_wrap().gap(px(6.0));
            let copy = text.clone();
            row = row.child(small_button(
                &t,
                "复制",
                cx.listener(move |_, _: &MouseDownEvent, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                }),
            ));
            row = row.child(small_button(
                &t,
                "插入到笔记顶部",
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    if let Some(p) = path.clone() {
                        super::notes::insert_summary(this, &p, &text, cx);
                    }
                }),
            ));
            col = col.child(row);
        }
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(col)
        .into_any_element()
}

/// Lightweight Markdown: paragraphs, headings, lists, code blocks (with 复制) and citation chips.
fn render_blocks(app: &RustdianApp, text: &str, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let mut col = div().flex().flex_col().gap(px(6.0)).line_height(px(20.0));
    for block in markdown::parse(text) {
        match block {
            Block::Heading { level, text } => {
                let size = if level <= 2 { 15.0 } else { 13.5 };
                col = col.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(size))
                        .child(markdown::plain_inline(&text)),
                );
            }
            Block::Paragraph { lines } => {
                for line in lines {
                    col = col.child(div().child(markdown::plain_inline(&line)));
                    let cites = markdown::citations(&line);
                    if !cites.is_empty() {
                        let mut row = div().flex().flex_wrap().gap(px(6.0));
                        for c in &cites {
                            row = row.child(cite_chip(&t, c, cx));
                        }
                        col = col.child(row);
                    }
                }
            }
            Block::Bullets(items) => {
                for item in items {
                    col = col.child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(div().text_color(t.tx3).child("•"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(markdown::plain_inline(&item)),
                            ),
                    );
                }
            }
            Block::Numbered(items) => {
                for (i, item) in items.iter().enumerate() {
                    col = col.child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(div().text_color(t.tx3).child(format!("{}.", i + 1)))
                            .child(div().flex_1().min_w_0().child(markdown::plain_inline(item))),
                    );
                }
            }
            Block::Code { lang, text } => {
                let copy = text.clone();
                col = col.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .p(px(8.0))
                        .rounded(px(6.0))
                        .bg(t.sheet)
                        .border_1()
                        .border_color(t.line_soft)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .text_size(px(11.0))
                                .text_color(t.tx3)
                                .child(if lang.is_empty() {
                                    "代码".to_string()
                                } else {
                                    lang
                                })
                                .child(
                                    div()
                                        .cursor_pointer()
                                        .px(px(6.0))
                                        .rounded(px(4.0))
                                        .hover(|s| s.bg(t.hover))
                                        .child("复制")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |_, _: &MouseDownEvent, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    copy.clone(),
                                                ));
                                            }),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .font_family(crate::ui::fonts::code_name())
                                .text_size(px(12.0))
                                .child(text),
                        ),
                );
            }
        }
    }
    col
}

/// The preview card: everything that will be sent, with sizes, and the confirm/cancel buttons.
fn render_preview(app: &RustdianApp, p: &Preview, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let total = p.prompt.preview.total_chars;
    let mut items = div()
        .id("ai-preview-items")
        .flex()
        .flex_col()
        .gap(px(3.0))
        .max_h(px(110.0))
        .overflow_y_scroll();
    for (label, n) in &p.prompt.preview.items {
        items = items.child(
            div()
                .flex()
                .justify_between()
                .text_size(px(12.0))
                .child(div().min_w_0().child(label.clone()))
                .child(div().flex_none().text_color(t.tx3).child(format!("{n} 字"))),
        );
    }
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .mx(px(12.0))
        .mb(px(8.0))
        .p(px(10.0))
        .rounded(px(10.0))
        .bg(t.sheet)
        .border_1()
        .border_color(t.ac_line)
        .child(
            div()
                .flex()
                .justify_between()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("将发送的内容"),
                )
                .child(div().text_color(t.tx3).child(format!("共 {total} 字"))),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(t.tx2)
                .child(format!("发送至 {}（{}）", p.provider, p.host)),
        );
    if p.vector_used {
        card = card.child(
            div()
                .text_size(px(11.5))
                .text_color(t.tx3)
                .child("检索时问题文本已先发送给嵌入服务。"),
        );
    }
    if !p.sources.is_empty() {
        let mut row = div().flex().flex_wrap().gap(px(6.0));
        for path in p.sources.iter().take(8) {
            row = row.child(cite_chip(&t, &crate::ui::ai_ui::title_of(path), cx));
        }
        card = card.child(row);
    }
    card.child(items).child(
        div()
            .flex()
            .justify_end()
            .gap(px(8.0))
            .child(button(
                &t,
                "取消",
                ButtonKind::Secondary,
                None,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    super::chat::cancel_preview(this);
                    cx.notify();
                }),
            ))
            .child(button(
                &t,
                "确认发送",
                ButtonKind::Primary,
                Some("corner-down-left"),
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    super::chat::confirm_send(this, cx);
                }),
            )),
    )
}

#[cfg(test)]
mod answered_note_tests {
    use super::answered_note;

    #[test]
    fn shows_only_when_the_answering_model_differs() {
        assert_eq!(answered_note(Some("gpt-4o"), None), None);
        assert_eq!(answered_note(Some("gpt-4o"), Some("gpt-4o")), None);
        assert_eq!(answered_note(Some("gpt-4o"), Some("  ")), None);
        assert_eq!(
            answered_note(Some("gpt-4o"), Some("openai/gpt-4o-2024")).as_deref(),
            Some("实际回答的模型：openai/gpt-4o-2024")
        );
        assert!(answered_note(None, Some("alias")).is_some());
    }
}
