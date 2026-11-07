//! Right panel (272px): 大纲 timeline, 反向链接 cards and 未链接的提及.

use gpui::{
    AnyElement, ClickEvent, Context, ElementId, FontWeight, IntoElement, MouseButton, SharedString,
    Window, div, prelude::*, px, relative,
};
use jiyue_md::frontmatter::{self, Property, Value};

use super::UI_FONT;
use super::attach_ui;
use super::attach_ui::props::{self, Block};
use super::icons::icon;
use super::inline::rich_text;
use super::widgets::{LineInput, LineInputEvent};
use crate::RustdianApp;
use crate::buffer::HeadingItem;
use crate::theme::{ActiveTheme, ICON_GAP_TIGHT, ICON_SIZE, RADIUS_M, RADIUS_S};

/// Red of the inline hint (the same red as the palette's danger rows).
const DANGER_RGB: u32 = 0xE5484D;

/// Renders the right panel column.
pub fn render_right_panel(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let (headings, title) = match app.vault.active_note() {
        Some(n) => {
            // A `# comment` inside the frontmatter block is not a heading.
            let block = attach_ui::props::leading_block_lines(n.content());
            let headings: Vec<HeadingItem> = n
                .extract_headings()
                .into_iter()
                .filter(|h| h.line_number > block)
                .collect();
            (headings, n.title.clone())
        }
        None => (Vec::new(), String::new()),
    };
    let backlinks = app.vault.get_backlinks(&title);
    let unlinked = unlinked_mentions(app, &title, &backlinks);

    div()
        .flex()
        .flex_col()
        .w(px(super::RIGHT_PANEL_W))
        .flex_none()
        .h_full()
        .overflow_hidden()
        .px(px(14.0))
        .pt(px(16.0))
        .pb(px(12.0))
        .border_l_1()
        .border_color(t.line_soft)
        .font_family(crate::ui::fonts::ui_name())
        .child(properties_section(app, cx))
        .child(section_title(
            &t,
            &crate::t!("panel-outline"),
            headings.len(),
            0.0,
        ))
        .child(outline(app, &headings, cx))
        .child(section_title(
            &t,
            &crate::t!("panel-backlinks"),
            backlinks.len(),
            0.0,
        ))
        .child(cards(
            app,
            &backlinks,
            0,
            &crate::t!("panel-backlinks-empty"),
            cx,
        ))
        .child(section_title(
            &t,
            &crate::t!("panel-unlinked"),
            unlinked.len(),
            12.0,
        ))
        .child(cards(
            app,
            &unlinked,
            10_000,
            &crate::t!("panel-unlinked-empty"),
            cx,
        ))
}

/// Section heading row: title on the left, item count on the right.
fn section_title(t: &ActiveTheme, title: &str, count: usize, mt: f32) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .justify_between()
        .mt(px(mt))
        .mb(px(8.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.tx2)
        .child(SharedString::from(title.to_string()))
        .child(
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx3)
                .child(SharedString::from(format!("{}", count))),
        )
}

/// 属性: the active note's frontmatter as rows. Checkboxes flip, tags can be removed, rows can be
/// deleted, and 添加属性 adds an empty text property. Values are edited in the source view.
/// Collapsed state is kept for the session.
fn properties_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let collapsed = app.ext.attach.props_collapsed;
    let text = app
        .vault
        .active_note()
        .map(|n| n.content().to_string())
        .unwrap_or_default();
    let block = props::block_state(&text);
    let count = match &block {
        Block::Props(p) => p.len(),
        _ => 0,
    };
    let header = div()
        .id("props-toggle")
        .flex()
        .items_center()
        .justify_between()
        .mb(px(8.0))
        .cursor_pointer()
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.tx2)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(ICON_GAP_TIGHT))
                .child(icon(
                    if collapsed {
                        "chevron-right"
                    } else {
                        "chevron-down"
                    },
                    13.0,
                    t.tx3,
                ))
                .child("属性"),
        )
        .child(
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx3)
                .child(SharedString::from(format!("{count}"))),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.ext.attach.props_collapsed = !this.ext.attach.props_collapsed;
            cx.notify();
        }));
    let mut section = div().flex().flex_col().mb(px(14.0)).child(header);
    if collapsed {
        return section.into_any_element();
    }
    match block {
        Block::None => section = section.child(muted(&t, "没有属性")),
        Block::Broken => {
            section = section.child(muted(&t, "属性块无法解析"));
            return section.into_any_element();
        }
        Block::Props(props) => {
            for (idx, prop) in props.iter().enumerate() {
                section = section.child(prop_row(app, idx, prop, cx));
            }
        }
    }
    section
        .child(
            div()
                .id("prop-add")
                .mt(px(4.0))
                .flex()
                .items_center()
                .gap(px(ICON_GAP_TIGHT))
                .h(px(26.0))
                .px(px(8.0))
                .rounded(px(RADIUS_S))
                .text_size(px(12.0))
                .text_color(t.ac)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon("plus", ICON_SIZE, t.ac))
                .child("添加属性")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    attach_ui::edit_active_text(this, props::add_property, cx);
                })),
        )
        .child(tag_add_row(app, cx))
        .into_any_element()
}

/// One property row: the key, its value (checkbox, chips or text) and a delete button.
fn prop_row(
    app: &RustdianApp,
    idx: usize,
    prop: &Property,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let key = prop.key.clone();
    let value: AnyElement = match &prop.value {
        Value::Bool(on) => {
            let on = *on;
            let key = key.clone();
            let mut boxed = div()
                .id(ElementId::NamedInteger(
                    SharedString::from("prop-bool"),
                    idx as u64,
                ))
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.0))
                .rounded(px(RADIUS_S))
                .border_1()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let key = key.clone();
                    attach_ui::edit_active_text(
                        this,
                        move |text| props::toggle_bool(text, &key, on),
                        cx,
                    );
                }));
            boxed = if on {
                boxed
                    .bg(t.ac)
                    .border_color(t.ac)
                    .child(icon("check", 11.0, t.ac_on))
            } else {
                boxed.border_color(t.tx3)
            };
            boxed.into_any_element()
        }
        Value::List(items) => chip_row(app, idx, &key, items, cx),
        // A tag list written as one string (`tags: a, b`) is shown as chips too.
        Value::Text(s) if props::is_tag_key(&key) => {
            chip_row(app, idx, &key, &frontmatter::split_tokens(s), cx)
        }
        other => match props::kind_of(other) {
            Some(kind) => inline_value(app, idx, prop, kind, cx),
            None => div()
                .text_size(px(12.5))
                .text_color(t.tx)
                .child(SharedString::from(props::value_text(other)))
                .into_any_element(),
        },
    };

    let del_key = prop.key.clone();
    // The name column is wider while its rename field is open.
    let editing = app
        .ext
        .attach
        .key_edit
        .as_ref()
        .is_some_and(|e| e.key == prop.key);
    div()
        .flex()
        .items_start()
        .gap(px(8.0))
        .py(px(5.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(
            div()
                .w(px(if editing { 150.0 } else { 78.0 }))
                .flex_none()
                .pt(px(2.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .overflow_hidden()
                .child(key_label(app, idx, &prop.key, cx)),
        )
        .child(div().flex_1().min_w_0().child(value))
        .child(
            div()
                .id(ElementId::NamedInteger(
                    SharedString::from("prop-del"),
                    idx as u64,
                ))
                .flex_none()
                .p(px(2.0))
                .rounded(px(RADIUS_S))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon("x", 12.0, t.tx3))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let del_key = del_key.clone();
                    attach_ui::edit_active_text(
                        this,
                        move |text| props::delete_property(text, &del_key),
                        cx,
                    );
                })),
        )
        .into_any_element()
}

/// The chips of a list of items (a tag list). The × removes that item from the property.
fn chip_row(
    app: &RustdianApp,
    idx: usize,
    key: &str,
    items: &[String],
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let mut chips = div().flex().flex_wrap().gap(px(5.0));
    for (j, item) in items.iter().enumerate() {
        let item_text = item.clone();
        let key = key.to_string();
        chips = chips.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .h(px(20.0))
                .pl(px(8.0))
                .pr(px(4.0))
                .rounded_full()
                .bg(t.ac_soft)
                .text_size(px(12.0))
                .text_color(t.ac)
                .child(SharedString::from(item.clone()))
                .child(
                    div()
                        .id(ElementId::NamedInteger(
                            SharedString::from("prop-chip"),
                            (idx * 1000 + j) as u64,
                        ))
                        .px(px(3.0))
                        .rounded_full()
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .child(icon("x", 10.0, t.ac))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let key = key.clone();
                            let item_text = item_text.clone();
                            attach_ui::edit_active_text(
                                this,
                                move |text| frontmatter::remove_tag(text, &key, &item_text),
                                cx,
                            );
                        })),
                ),
        );
    }
    chips.into_any_element()
}

/// The name of a property row. A click opens the rename field (Enter renames, Esc cancels).
fn key_label(
    app: &RustdianApp,
    idx: usize,
    key: &str,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    if let Some(edit) = app.ext.attach.key_edit.as_ref().filter(|e| e.key == key) {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(edit.input.clone());
        if let Some(msg) = edit.error {
            col = col.child(
                div()
                    .text_size(px(11.5))
                    .text_color(gpui::rgb(DANGER_RGB))
                    .child(msg),
            );
        }
        return col.into_any_element();
    }
    let owned = key.to_string();
    div()
        .id(ElementId::NamedInteger(
            SharedString::from("prop-key"),
            idx as u64,
        ))
        .rounded(px(RADIUS_S))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .child(SharedString::from(key.to_string()))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            open_key_edit(this, &owned, window, cx);
        }))
        .into_any_element()
}

/// Opens the rename field of property `key` with its name selected for typing.
fn open_key_edit(
    this: &mut RustdianApp,
    key: &str,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    let theme = this.theme;
    let input = cx.new(|cx| LineInput::new(cx, "属性名"));
    input.update(cx, |field, cx| {
        field.set_theme(theme, cx);
        field.set_text(key, cx);
    });
    input.read(cx).focus(window);
    let old = key.to_string();
    let sub = cx.subscribe(
        &input,
        move |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            on_key_event(this, &old, ev, cx);
        },
    );
    this.ext.attach.key_edit = Some(attach_ui::KeyEdit {
        key: key.to_string(),
        input,
        error: None,
        _sub: sub,
    });
    cx.notify();
}

/// Enter renames the property (one undo step; value, order and comments stay), Esc closes the
/// field, and a rejected name keeps the field open with a hint.
fn on_key_event(
    this: &mut RustdianApp,
    old: &str,
    ev: &LineInputEvent,
    cx: &mut Context<RustdianApp>,
) {
    match ev {
        LineInputEvent::Changed(_) => {}
        LineInputEvent::Cancel => this.ext.attach.key_edit = None,
        LineInputEvent::Submit(raw) => {
            let note = this
                .vault
                .active_note()
                .map(|n| n.content().to_string())
                .unwrap_or_default();
            let others: Vec<String> = props::keys(&note)
                .into_iter()
                .filter(|k| k != old)
                .collect();
            match props::check_key(raw, old, &others) {
                Ok(new) => {
                    this.ext.attach.key_edit = None;
                    let old = old.to_string();
                    attach_ui::edit_active_text(
                        this,
                        move |text| frontmatter::rename_property(text, &old, &new),
                        cx,
                    );
                }
                Err(msg) => {
                    if let Some(edit) = this.ext.attach.key_edit.as_mut() {
                        edit.error = Some(msg);
                    }
                }
            }
        }
    }
    cx.notify();
}

/// A text, number or date value: a click opens the inline field (Enter saves, Esc cancels). The
/// open field shows a red hint under it when the typed value is not valid.
fn inline_value(
    app: &RustdianApp,
    idx: usize,
    prop: &Property,
    kind: props::Kind,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    if let Some(edit) = app
        .ext
        .attach
        .prop_edit
        .as_ref()
        .filter(|e| e.key == prop.key)
    {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(edit.input.clone());
        if let Some(msg) = edit.error {
            col = col.child(
                div()
                    .text_size(px(11.5))
                    .text_color(gpui::rgb(DANGER_RGB))
                    .child(msg),
            );
        }
        return col.into_any_element();
    }
    let text = props::value_text(&prop.value);
    let empty = text.is_empty();
    let key = prop.key.clone();
    div()
        .id(ElementId::NamedInteger(
            SharedString::from("prop-val"),
            idx as u64,
        ))
        .min_h(px(20.0))
        .rounded(px(RADIUS_S))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .text_size(px(12.5))
        .text_color(if empty { t.tx3 } else { t.tx })
        .child(SharedString::from(if empty {
            "点击输入".to_string()
        } else {
            text.clone()
        }))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            open_prop_edit(this, &key, kind, &text, window, cx);
        }))
        .into_any_element()
}

/// Opens the inline field of property `key` with its current text and focuses it.
fn open_prop_edit(
    this: &mut RustdianApp,
    key: &str,
    kind: props::Kind,
    current: &str,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    let theme = this.theme;
    let input = cx.new(|cx| LineInput::new(cx, "输入值"));
    input.update(cx, |field, cx| {
        field.set_theme(theme, cx);
        field.set_text(current, cx);
    });
    input.read(cx).focus(window);
    let owned_key = key.to_string();
    let sub = cx.subscribe(
        &input,
        move |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            on_prop_event(this, &owned_key, kind, ev, cx);
        },
    );
    this.ext.attach.prop_edit = Some(attach_ui::PropEdit {
        key: key.to_string(),
        kind,
        input,
        error: None,
        _sub: sub,
    });
    cx.notify();
}

/// Enter saves the typed value through the frontmatter (one undo step), Esc closes the field, and
/// an invalid number or date keeps the field open with a hint.
fn on_prop_event(
    this: &mut RustdianApp,
    key: &str,
    kind: props::Kind,
    ev: &LineInputEvent,
    cx: &mut Context<RustdianApp>,
) {
    match ev {
        LineInputEvent::Changed(_) => {}
        LineInputEvent::Cancel => this.ext.attach.prop_edit = None,
        LineInputEvent::Submit(raw) => match props::parse_value(kind, raw) {
            Ok(value) => {
                this.ext.attach.prop_edit = None;
                let key = key.to_string();
                attach_ui::edit_active_text(
                    this,
                    move |text| frontmatter::set_property(text, &key, &value),
                    cx,
                );
            }
            Err(msg) => {
                if let Some(edit) = this.ext.attach.prop_edit.as_mut() {
                    edit.error = Some(msg);
                }
            }
        },
    }
    cx.notify();
}

/// The 添加标签 button, and the chip field under it while it is open.
fn tag_add_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let mut row = div().flex().flex_col().gap(px(4.0));
    match app.ext.attach.tag_input.as_ref() {
        Some((input, _)) => {
            row =
                row.child(input.clone())
                    .child(div().text_size(px(11.5)).text_color(t.tx3).child(
                    "Enter 添加；逗号或空格可一次添加多个；空白时 Backspace 删除最后一个；Esc 关闭",
                ));
        }
        None => {
            row = row.child(
                div()
                    .id("tag-add")
                    .mt(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(ICON_GAP_TIGHT))
                    .h(px(26.0))
                    .px(px(8.0))
                    .rounded(px(RADIUS_S))
                    .text_size(px(12.0))
                    .text_color(t.ac)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(icon("plus", ICON_SIZE, t.ac))
                    .child("添加标签")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        open_tag_input(this, window, cx);
                    })),
            );
        }
    }
    row.into_any_element()
}

/// Opens the 添加标签 field. Enter adds the typed tags to the `tags` property (created when missing)
/// and clears the field, so the next tag can be typed at once; the field stays open until Esc.
fn open_tag_input(this: &mut RustdianApp, window: &mut Window, cx: &mut Context<RustdianApp>) {
    let theme = this.theme;
    let input = cx.new(|cx| LineInput::new(cx, "标签名"));
    input.update(cx, |field, cx| field.set_theme(theme, cx));
    input.read(cx).focus(window);
    let sub = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            match ev {
                LineInputEvent::Changed(_) => {}
                LineInputEvent::Cancel => this.ext.attach.tag_input = None,
                LineInputEvent::Submit(raw) => {
                    let tags = props::tags_from_input(raw);
                    if !tags.is_empty() {
                        if let Some((input, _)) = this.ext.attach.tag_input.as_ref() {
                            let input = input.clone();
                            input.update(cx, |field, cx| field.set_text("", cx));
                        }
                        attach_ui::edit_active_text(
                            this,
                            move |text| frontmatter::add_tags(text, "tags", &tags),
                            cx,
                        );
                    }
                }
            }
            cx.notify();
        },
    );
    this.ext.attach.tag_input = Some((input, sub));
    cx.notify();
}

fn muted(t: &ActiveTheme, text: &str) -> impl IntoElement + use<> {
    div()
        .mb(px(8.0))
        .text_size(px(12.5))
        .text_color(t.tx3)
        .child(SharedString::from(text.to_string()))
}

/// 大纲: vertical timeline of the note's headings. The heading at the cursor is active.
fn outline(
    app: &RustdianApp,
    headings: &[HeadingItem],
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    if headings.is_empty() {
        return div()
            .mb(px(14.0))
            .text_size(px(12.5))
            .text_color(t.tx3)
            .child(SharedString::from(crate::t!("panel-outline-empty")))
            .into_any_element();
    }
    let active_line = headings
        .iter()
        .filter(|h| h.line_number <= app.cursor_row)
        .map(|h| h.line_number)
        .last()
        .unwrap_or(headings[0].line_number);

    let mut list = div().relative().flex().flex_col().mb(px(14.0)).child(
        div()
            .absolute()
            .left(px(5.0))
            .top(px(6.0))
            .bottom(px(6.0))
            .w(px(1.0))
            .bg(t.line),
    );
    for (idx, h) in headings.iter().enumerate() {
        let on = h.line_number == active_line;
        let line_no = h.line_number;
        let text_color = if on { t.ac } else { t.tx2 };
        list = list.child(
            div()
                .id(ElementId::NamedInteger(
                    SharedString::from("outline"),
                    idx as u64,
                ))
                .relative()
                .flex()
                .items_center()
                .h(px(26.0))
                .pl(px(20.0))
                .pr(px(8.0))
                .rounded(px(RADIUS_S))
                .text_size(px(12.5))
                .text_color(text_color)
                .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                .cursor_pointer()
                .hover(|s| s.text_color(t.tx))
                .child(
                    div()
                        .absolute()
                        .left(px(2.0))
                        .top(px(9.5))
                        .size(px(7.0))
                        .rounded_full()
                        .border_1()
                        .border_color(if on { t.ac } else { t.line })
                        .bg(if on { t.ac } else { t.sheet }),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(h.text.clone())),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        // A heading opens the source with the caret on its line.
                        this.request_source_caret(line_no.saturating_sub(1), 0);
                        cx.notify();
                    }),
                ),
        );
    }
    list.into_any_element()
}

/// Cards for backlinks or unlinked mentions: `(note title, snippet line)` pairs.
fn cards(
    app: &RustdianApp,
    items: &[(String, String)],
    key_base: u64,
    empty: &str,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    if items.is_empty() {
        return div()
            .px(px(10.0))
            .text_size(px(12.0))
            .text_color(t.tx3)
            .child(SharedString::from(empty.to_string()))
            .into_any_element();
    }

    let target = app
        .vault
        .active_note()
        .map(|n| n.title.clone())
        .unwrap_or_default();

    let mut col = div().flex().flex_col().gap(px(2.0)).mb(px(6.0));
    for (idx, (title, line)) in items.iter().enumerate() {
        let key = key_base + idx as u64;
        let snippet = snippet_around(line, &target);
        let open_title = title.clone();
        col = col.child(
            div()
                .id(ElementId::NamedInteger(SharedString::from("card"), key))
                .flex()
                .flex_col()
                .px(px(10.0))
                .py(px(9.0))
                .rounded(px(RADIUS_M))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(ICON_GAP_TIGHT))
                        .mb(px(4.0))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.tx)
                        .child(icon("file-text", ICON_SIZE, t.tx3))
                        .child(div().child(SharedString::from(title.clone()))),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .line_height(relative(1.65))
                        .text_color(t.tx2)
                        .line_clamp(2)
                        .child(rich_text(
                            &snippet,
                            &t,
                            t.tx2,
                            UI_FONT,
                            Some(&target),
                            false,
                            key,
                            cx,
                        )),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.open_note_by_title(&open_title);
                        cx.notify();
                    }),
                ),
        );
    }
    col.into_any_element()
}

/// Notes that mention the title in plain text without linking to it.
fn unlinked_mentions(
    app: &RustdianApp,
    title: &str,
    backlinks: &[(String, String)],
) -> Vec<(String, String)> {
    if title.is_empty() {
        return Vec::new();
    }
    let link = format!("[[{}]]", title);
    let mut out = Vec::new();
    for note in &app.vault.notes {
        if note.title == title || backlinks.iter().any(|(t, _)| *t == note.title) {
            continue;
        }
        let content = note.content();
        if !content.contains(title) || content.contains(&link) {
            continue;
        }
        if let Some(line) = content.lines().find(|l| l.contains(title)) {
            out.push((note.title.clone(), line.trim().to_string()));
        }
    }
    out
}

/// A short excerpt of `line` centred on `target`, with ellipses where cut.
fn snippet_around(line: &str, target: &str) -> String {
    const CONTEXT: usize = 24;
    let Some(pos) = line.find(target) else {
        return line.chars().take(60).collect();
    };
    let before: Vec<char> = line[..pos].chars().collect();
    let after: Vec<char> = line[pos + target.len()..].chars().collect();
    let pre_cut = before.len() > CONTEXT;
    let post_cut = after.len() > CONTEXT;

    let mut s = String::new();
    if pre_cut {
        s.push('…');
    }
    s.extend(before.iter().skip(before.len().saturating_sub(CONTEXT)));
    s.push_str(target);
    s.extend(after.iter().take(CONTEXT));
    if post_cut {
        s.push('…');
    }
    s
}
