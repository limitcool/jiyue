//! Right panel (264px): segmented inspector switching between Outline/Properties, Backlinks and Local Graph.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, ClickEvent, Context, Corners, CursorStyle, Edges, ElementId, FontWeight,
    Hsla, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder,
    Pixels, Point, Rgba, SharedString, TextRun, Window, canvas, div, fill, font, point, prelude::*,
    px, relative, size, transparent_black,
};
use jiyue_md::frontmatter::{self, Property, Value};

use super::UI_FONT;
use super::attach_ui;
use super::attach_ui::props::{self, Block};
use super::ext::ToastKind;
use super::icons::icon;
use super::inline::rich_text;
use super::keymap::primary_held;
use super::sidebar::SidebarTab;
use super::widgets::controls::{empty_state, removable_tag_chip, segmented_full};
use super::widgets::{LineInput, LineInputEvent};
use crate::RustdianApp;
use crate::ViewportMode;
use crate::buffer::HeadingItem;
use crate::theme::{ActiveTheme, ICON_GAP_TIGHT, ICON_SIZE, RADIUS_M, RADIUS_S};
use crate::vault::KnowledgeGraph;

/// Active tab inside the right inspector panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RightPanelTab {
    #[default]
    Outline,
    Backlinks,
    Graph,
}

/// Renders the right panel column.
pub fn render_right_panel(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let tab = app.right_panel_tab;

    let tab_outline = crate::t!("panel-outline");
    let tab_backlinks = crate::t!("panel-backlinks");
    let tab_graph = crate::t!("panel-local-graph");
    let selected_idx = match tab {
        RightPanelTab::Outline => 0,
        RightPanelTab::Backlinks => 1,
        RightPanelTab::Graph => 2,
    };
    let tab_bar = div()
        .flex_none()
        .px(px(10.0))
        .py(px(5.0))
        .border_b_1()
        .border_color(t.border_subtle)
        .child(segmented_full(
            &t,
            &[
                tab_outline.as_str(),
                tab_backlinks.as_str(),
                tab_graph.as_str(),
            ],
            selected_idx,
            Rc::new(cx.listener(|this, &idx, _, cx| {
                this.right_panel_tab = match idx {
                    0 => RightPanelTab::Outline,
                    1 => RightPanelTab::Backlinks,
                    _ => RightPanelTab::Graph,
                };
                cx.notify();
            })),
        ));

    let body: AnyElement = match tab {
        RightPanelTab::Outline => {
            let (headings, block) = cached_inspector_data(app);
            div()
                .id("right_panel_scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(12.0))
                .py(px(10.0))
                .flex()
                .flex_col()
                .child(properties_section(app, &block, cx))
                .child(section_title(
                    &t,
                    &crate::t!("panel-outline"),
                    headings.len(),
                    0.0,
                ))
                .child(outline(app, &headings, cx))
                .into_any_element()
        }
        RightPanelTab::Backlinks => {
            let title = app
                .vault
                .active_note()
                .map(|n| n.title.clone())
                .unwrap_or_default();
            let cached = cached_backlinks(app, &title);
            let (backlinks, unlinked) = (&cached.0, &cached.1);
            div()
                .id("right_panel_scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(12.0))
                .py(px(10.0))
                .flex()
                .flex_col()
                .child(section_title(
                    &t,
                    &crate::t!("panel-backlinks"),
                    backlinks.len(),
                    0.0,
                ))
                .child(cards(
                    app,
                    backlinks,
                    0,
                    false,
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
                    unlinked,
                    10_000,
                    true,
                    &crate::t!("panel-unlinked-empty"),
                    cx,
                ))
                .into_any_element()
        }
        RightPanelTab::Graph => render_local_graph(app, cx).into_any_element(),
    };

    div()
        .flex()
        .flex_col()
        .w(px(super::RIGHT_PANEL_W))
        .flex_none()
        .h_full()
        .overflow_hidden()
        .bg(t.panel)
        .border_l_1()
        .border_color(t.border_subtle)
        .font_family(crate::ui::fonts::ui_name())
        .child(tab_bar)
        .child(body)
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

fn cached_inspector_data(app: &RustdianApp) -> super::files_ui::InspectorData {
    let Some(note) = app.vault.active_note() else {
        return (Rc::new(Vec::new()), Rc::new(Block::None));
    };
    let arc = note.content_arc();
    let mut cache = app.ext.files.inspector_cache.borrow_mut();
    if let Some((cached_path, cached_arc, data)) = cache.as_ref()
        && cached_path == &note.path
        && std::sync::Arc::ptr_eq(cached_arc, &arc)
    {
        return (Rc::clone(&data.0), Rc::clone(&data.1));
    }
    let text: &str = &arc;
    let block_lines = props::leading_block_lines(text);
    // A `# comment` inside the frontmatter block is not a heading.
    let headings: Vec<HeadingItem> = note
        .extract_headings()
        .into_iter()
        .filter(|h| h.line_number > block_lines)
        .collect();
    let block = props::block_state(text);
    let data = (Rc::new(headings), Rc::new(block));
    *cache = Some((
        note.path.clone(),
        arc,
        (Rc::clone(&data.0), Rc::clone(&data.1)),
    ));
    data
}

/// 属性: the active note's frontmatter as rows. Checkboxes flip, tags can be removed, rows can be
/// deleted, and 添加属性 adds an empty text property. Values are edited in the source view.
/// Collapsed state is kept for the session.
fn properties_section(
    app: &RustdianApp,
    block: &Block,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let collapsed = app.ext.attach.props_collapsed;
    let count = match block {
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
    let is_tags_key = key.eq_ignore_ascii_case("tags") || key.eq_ignore_ascii_case("tag");
    let mut chips = div().flex().flex_wrap().gap(px(5.0));
    for (j, item) in items.iter().enumerate() {
        let open_item = item.clone();
        let remove_item = item.clone();
        let key_rm = key.to_string();
        let label = if is_tags_key {
            format!("#{item}")
        } else {
            item.clone()
        };
        chips = chips.child(removable_tag_chip(
            &t,
            ElementId::NamedInteger(SharedString::from("prop-chip"), (idx * 1000 + j) as u64),
            &label,
            cx.listener(move |this, _, _, cx| {
                if is_tags_key {
                    let q = format!("tag:{open_item}");
                    this.viewport_mode = ViewportMode::Editor;
                    this.sidebar_tab = SidebarTab::Search;
                    this.left_sidebar_open = true;
                    crate::config_store::sync_ui(this);
                    super::search_ui::set_sidebar_query(this, &q, cx);
                    cx.notify();
                }
            }),
            cx.listener(move |this, _, _, cx| {
                let key = key_rm.clone();
                let item_text = remove_item.clone();
                attach_ui::edit_active_text(
                    this,
                    move |text| frontmatter::remove_tag(text, &key, &item_text),
                    cx,
                );
            }),
        ));
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
            col = col.child(div().text_size(px(11.5)).text_color(t.danger).child(msg));
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
            col = col.child(div().text_size(px(11.5)).text_color(t.danger).child(msg));
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
        return empty_state(&t, "list", &crate::t!("panel-outline-empty"), "").into_any_element();
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
            .bg(t.border_subtle),
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
                        .border_color(if on { t.ac } else { t.border_default })
                        .bg(if on { t.ac } else { t.panel }),
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
    allow_link: bool,
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

    let mut col = div().flex().flex_col().gap(px(6.0)).mb(px(6.0));
    for (idx, (title, line)) in items.iter().enumerate() {
        let key = key_base + idx as u64;
        let snippet = snippet_around(line, &target);
        let open_title = title.clone();
        let jump_target = target.clone();
        let link_source = title.clone();
        let link_target = target.clone();
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(6.0))
            .mb(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(ICON_GAP_TIGHT))
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.tx)
                    .child(icon("file-text", ICON_SIZE, t.tx3))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(title.clone())),
                    ),
            );
        if allow_link && !link_target.is_empty() {
            header = header.child(
                div()
                    .id(ElementId::NamedInteger(SharedString::from("link_btn"), key))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(RADIUS_S))
                    .bg(t.ac_soft)
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.ac)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.active))
                    .child(icon("link", 11.5, t.ac))
                    .child(SharedString::from(crate::t!("panel-link-btn")))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            convert_unlinked_mention(this, &link_source, &link_target, cx);
                        }),
                    ),
            );
        }
        col = col.child(
            div()
                .id(ElementId::NamedInteger(SharedString::from("card"), key))
                .flex()
                .flex_col()
                .px(px(10.0))
                .py(px(9.0))
                .bg(t.surface)
                .border_1()
                .border_color(t.border_subtle)
                .rounded(px(RADIUS_M))
                .cursor_pointer()
                .hover(|s| s.border_color(t.border_default).bg(t.hover))
                .child(header)
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
                        let mention_row = this
                            .vault
                            .notes
                            .iter()
                            .find(|n| n.title == open_title)
                            .and_then(|n| {
                                let text = this
                                    .editor_for(&n.path)
                                    .map(|ed| ed.read(cx).text())
                                    .unwrap_or_else(|| n.content().to_string());
                                if allow_link {
                                    find_mention_line(&text, &jump_target)
                                } else {
                                    find_mention_line(&text, &format!("[[{jump_target}"))
                                        .or_else(|| find_mention_line(&text, &jump_target))
                                }
                                .map(|row| (n.path.clone(), row))
                            });
                        this.open_note_by_title(&open_title);
                        if let Some((note_path, row)) = mention_row {
                            if let Some(ed) = this.editor_for(&note_path) {
                                ed.update(cx, |ed, cx| ed.set_caret_to_line(row, cx));
                            } else {
                                this.request_caret(row, 0);
                            }
                        }
                        cx.notify();
                    }),
                ),
        );
    }
    col.into_any_element()
}

/// Converts the first bare unlinked mention of `target_title` in `source_title` into `[[target_title]]`.
fn convert_unlinked_mention(
    app: &mut RustdianApp,
    source_title: &str,
    target_title: &str,
    cx: &mut Context<RustdianApp>,
) {
    let Some(src_idx) = app.vault.notes.iter().position(|n| n.title == source_title) else {
        return;
    };
    let src_path = app.vault.notes[src_idx].path.clone();
    let current = app
        .editor_for(&src_path)
        .map(|ed| ed.read(cx).text())
        .unwrap_or_else(|| app.vault.notes[src_idx].content().to_string());
    let Some((new_text, _line_idx)) = link_first_unlinked_mention(&current, target_title) else {
        return;
    };

    super::history_ui::before_change(app, std::slice::from_ref(&src_path), cx);
    if let Some(ed) = app.editor_for(&src_path) {
        ed.update(cx, |ed, cx| ed.set_text_undoable(&new_text, cx));
        app.save_path(&src_path, cx);
    } else {
        if jiyue_core::fsops::write_atomic(&app.vault.root_dir.join(&src_path), new_text.as_bytes())
            .is_err()
        {
            app.toast(ToastKind::Error, "写入笔记失败");
            cx.notify();
            return;
        }
        app.ext
            .files
            .last_written
            .insert(src_path.clone(), new_text.clone());
    }
    if let Some(note) = app.vault.notes.get_mut(src_idx) {
        note.set_content(&new_text);
    }
    app.vault.bump_revision();
    app.ext.files.non_active_rev = app.ext.files.non_active_rev.wrapping_add(1);
    *app.ext.files.backlinks_cache.borrow_mut() = None;
    app.toast(
        ToastKind::Success,
        crate::t!("panel-linked-toast", note = source_title),
    );
    cx.notify();
}

/// Finds the 0-based line index of the first mention of `target` outside leading YAML frontmatter,
/// falling back to any matching line when only frontmatter contains it.
fn find_mention_line(content: &str, target: &str) -> Option<usize> {
    if target.is_empty() {
        return None;
    }
    let fm_lines = props::leading_block_lines(content);
    let mut fallback: Option<usize> = None;
    for (idx, line) in content.lines().enumerate() {
        if line.contains(target) {
            if idx >= fm_lines {
                return Some(idx);
            }
            if fallback.is_none() {
                fallback = Some(idx);
            }
        }
    }
    fallback
}

fn collect_protected_ranges(line: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();

    // Inline code spans (`...`).
    let mut i = 0usize;
    while i < len {
        if bytes[i] == b'`' {
            let mut ticks = 0usize;
            while i + ticks < len && bytes[i + ticks] == b'`' {
                ticks += 1;
            }
            let start = i;
            let mut j = i + ticks;
            let mut closed = None;
            while j + ticks <= len {
                if bytes[j..j + ticks].iter().all(|&b| b == b'`')
                    && bytes.get(j + ticks) != Some(&b'`')
                {
                    closed = Some(j + ticks);
                    break;
                }
                j += 1;
            }
            if let Some(end) = closed {
                out.push(start..end);
                i = end;
                continue;
            }
            i += ticks;
        } else {
            i += 1;
        }
    }

    // Wikilinks `[[...]]`.
    let mut search = 0usize;
    while let Some(rel) = line.get(search..).and_then(|s| s.find("[[")) {
        let start = search + rel;
        if let Some(end_rel) = line.get(start + 2..).and_then(|s| s.find("]]")) {
            let end = start + 2 + end_rel + 2;
            out.push(start..end);
            search = end;
        } else {
            break;
        }
    }

    // Markdown links `[text](url)`.
    let mut i = 0usize;
    while i < len {
        if bytes[i] == b'['
            && bytes.get(i + 1) != Some(&b'[')
            && let Some(mid_rel) = line.get(i + 1..).and_then(|s| s.find("]("))
            && let Some(end_rel) = line.get(i + 1 + mid_rel + 2..).and_then(|s| s.find(')'))
        {
            let end = i + 1 + mid_rel + 2 + end_rel + 1;
            out.push(i..end);
            i = end;
            continue;
        }
        i += 1;
    }

    // Bare HTTP/HTTPS URLs.
    for scheme in ["https://", "http://"] {
        let mut s = 0usize;
        while let Some(rel) = line.get(s..).and_then(|sub| sub.find(scheme)) {
            let start = s + rel;
            let mut end = start + scheme.len();
            while let Some(ch) = line.get(end..).and_then(|sub| sub.chars().next()) {
                if ch.is_whitespace() || matches!(ch, ')' | '>' | ']' | '"' | '\'') {
                    break;
                }
                end += ch.len_utf8();
            }
            out.push(start..end);
            s = end;
        }
    }

    out
}

fn find_unlinked_in_line(line: &str, target: &str) -> Option<usize> {
    if target.is_empty() {
        return None;
    }
    let protected = collect_protected_ranges(line);
    let first_ascii = target
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let last_ascii = target
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphanumeric());

    for (pos, _) in line.match_indices(target) {
        let end = pos + target.len();
        if protected.iter().any(|r| r.start < end && pos < r.end) {
            continue;
        }
        if first_ascii
            && line
                .get(..pos)
                .and_then(|s| s.chars().next_back())
                .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        if last_ascii
            && line
                .get(end..)
                .and_then(|s| s.chars().next())
                .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        return Some(pos);
    }
    None
}

/// Scans `content` for the first bare mention of `target` outside YAML frontmatter,
/// fenced code blocks, inline code, links, and URLs, returning `(abs_start, abs_end, line_idx, trimmed_line)`.
fn scan_first_unlinked_mention<'a>(
    content: &'a str,
    target: &str,
) -> Option<(usize, usize, usize, &'a str)> {
    if target.is_empty() {
        return None;
    }
    let fm_lines = props::leading_block_lines(content);
    let mut in_fence = false;
    let mut offset = 0usize;

    for (line_idx, raw_line) in content.split_inclusive('\n').enumerate() {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim_start();
        if line_idx >= fm_lines && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
            in_fence = !in_fence;
            offset += raw_line.len();
            continue;
        }
        if line_idx >= fm_lines
            && !in_fence
            && let Some(col) = find_unlinked_in_line(line, target)
        {
            let abs_start = offset + col;
            let abs_end = abs_start + target.len();
            return Some((abs_start, abs_end, line_idx, line.trim()));
        }
        offset += raw_line.len();
    }
    None
}

/// Replaces the first bare unlinked mention of `target` in `content` with `[[target]]`,
/// returning `(new_content, 0_based_line_index)`.
pub(crate) fn link_first_unlinked_mention(content: &str, target: &str) -> Option<(String, usize)> {
    let (start, end, line_idx, _) = scan_first_unlinked_mention(content, target)?;
    let mut out = String::with_capacity(content.len() + 4);
    out.push_str(&content[..start]);
    out.push_str("[[");
    out.push_str(target);
    out.push_str("]]");
    out.push_str(&content[end..]);
    Some((out, line_idx))
}

/// Returns cached backlinks and unlinked mentions for `title`, recomputing only when vault
/// structure, other notes, vault root, or active note title changes.
fn cached_backlinks(app: &RustdianApp, title: &str) -> Rc<super::files_ui::BacklinksData> {
    let struct_rev = app.vault.structure_revision;
    let non_active_rev = app.ext.files.non_active_rev;
    let mut cache = app.ext.files.backlinks_cache.borrow_mut();
    if let Some((cached_struct, cached_other, cached_root, cached_title, data)) = cache.as_ref()
        && *cached_struct == struct_rev
        && *cached_other == non_active_rev
        && *cached_root == app.vault.root_dir
        && cached_title == title
    {
        return Rc::clone(data);
    }
    let backlinks = app.vault.get_backlinks(title);
    let unlinked = unlinked_mentions(app, title, &backlinks);
    let data = Rc::new((backlinks, unlinked));
    *cache = Some((
        struct_rev,
        non_active_rev,
        app.vault.root_dir.clone(),
        title.to_string(),
        Rc::clone(&data),
    ));
    data
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
        if let Some((_, _, _, line)) = scan_first_unlinked_mention(content, title) {
            out.push((note.title.clone(), line.to_string()));
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

// ---------------------------------------------------------------------------
// Local graph inspector tab
// ---------------------------------------------------------------------------

const LOCAL_GRAPH_MAX_NODES: usize = 40;
const LOCAL_LABEL_PX: f32 = 11.5;
const LOCAL_LABEL_LINE_PX: f32 = 15.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalNode {
    pub vault_idx: usize,
    pub note_index: usize,
    pub id: String,
    pub title: String,
    pub hop: u8,
    pub link_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalSubgraph {
    pub nodes: Vec<LocalNode>,
    pub edges: Vec<(usize, usize)>,
}

#[derive(Clone)]
struct LocalGraphCache {
    key: u64,
    sub: LocalSubgraph,
    norm: Vec<(f32, f32)>,
}

/// Interactive state for the right-panel local knowledge graph.
pub struct LocalGraphState {
    pub depth: u8,
    pub hovered: Option<usize>,
    canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    cache: RefCell<Option<LocalGraphCache>>,
}

impl Default for LocalGraphState {
    fn default() -> Self {
        Self {
            depth: 1,
            hovered: None,
            canvas_bounds: Rc::new(Cell::new(None)),
            cache: RefCell::new(None),
        }
    }
}

fn with_alpha(c: Rgba, a: f32) -> Rgba {
    Rgba { a, ..c }
}

fn truncate_label(title: &str, max_chars: usize) -> String {
    let mut chars = title.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Extracts a BFS neighborhood subgraph up to `depth` hops centered on `center_idx` (placed at index 0).
pub(crate) fn build_local_subgraph(
    graph: &KnowledgeGraph,
    center_idx: usize,
    depth: u8,
    max_nodes: usize,
) -> LocalSubgraph {
    if center_idx >= graph.nodes.len() || max_nodes == 0 {
        return LocalSubgraph {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
    }
    let n = graph.nodes.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &graph.edges {
        if e.from_index < n && e.to_index < n && e.from_index != e.to_index {
            adj[e.from_index].push(e.to_index);
            adj[e.to_index].push(e.from_index);
        }
    }

    let max_hop = depth.clamp(1, 2);
    let mut dist: HashMap<usize, u8> = HashMap::new();
    let mut queue = VecDeque::new();
    dist.insert(center_idx, 0);
    queue.push_back(center_idx);
    while let Some(u) = queue.pop_front() {
        let d = dist[&u];
        if d >= max_hop {
            continue;
        }
        for &v in &adj[u] {
            if let std::collections::hash_map::Entry::Vacant(e) = dist.entry(v) {
                e.insert(d + 1);
                queue.push_back(v);
            }
        }
    }

    let mut candidates: Vec<(usize, u8)> = dist
        .into_iter()
        .filter(|&(idx, _)| idx != center_idx)
        .collect();
    candidates.sort_by(|&(i1, h1), &(i2, h2)| {
        h1.cmp(&h2)
            .then_with(|| graph.nodes[i2].link_count.cmp(&graph.nodes[i1].link_count))
            .then_with(|| i1.cmp(&i2))
    });

    let mut selected = Vec::with_capacity(max_nodes.min(candidates.len() + 1));
    selected.push((center_idx, 0u8));
    for (idx, h) in candidates.into_iter().take(max_nodes - 1) {
        selected.push((idx, h));
    }

    let mut to_local: HashMap<usize, usize> = HashMap::with_capacity(selected.len());
    let mut nodes = Vec::with_capacity(selected.len());
    for (local_i, (vault_i, hop)) in selected.into_iter().enumerate() {
        to_local.insert(vault_i, local_i);
        let node = &graph.nodes[vault_i];
        nodes.push(LocalNode {
            vault_idx: vault_i,
            note_index: node.note_index,
            id: node.id.clone(),
            title: node.title.clone(),
            hop,
            link_count: node.link_count,
        });
    }

    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut edges = Vec::new();
    for e in &graph.edges {
        if let (Some(&a), Some(&b)) = (to_local.get(&e.from_index), to_local.get(&e.to_index))
            && a != b
        {
            let key = if a < b { (a, b) } else { (b, a) };
            if seen.insert(key) {
                edges.push(key);
            }
        }
    }
    edges.sort();
    LocalSubgraph { nodes, edges }
}

fn centered_layout(sub: &LocalSubgraph) -> Vec<(f32, f32)> {
    let n = sub.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![(0.5, 0.5)];
    }
    let raw = super::graph::force_layout(n, &sub.edges);
    let c = raw[0];
    let max_dx = raw
        .iter()
        .map(|&(x, _)| (x - c.0).abs())
        .fold(0.0f32, f32::max)
        .max(1e-4);
    let max_dy = raw
        .iter()
        .map(|&(_, y)| (y - c.1).abs())
        .fold(0.0f32, f32::max)
        .max(1e-4);
    let scale = (0.38 / max_dx).min(0.38 / max_dy);
    raw.into_iter()
        .map(|(x, y)| {
            (
                (0.5 + (x - c.0) * scale).clamp(0.10, 0.90),
                (0.5 + (y - c.1) * scale).clamp(0.10, 0.90),
            )
        })
        .collect()
}

fn local_graph_key(graph: &KnowledgeGraph, center_idx: usize, depth: u8) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    center_idx.hash(&mut h);
    depth.hash(&mut h);
    for node in &graph.nodes {
        node.id.hash(&mut h);
        node.title.hash(&mut h);
    }
    for edge in &graph.edges {
        edge.from_index.hash(&mut h);
        edge.to_index.hash(&mut h);
    }
    h.finish()
}

fn local_layout(
    st: &LocalGraphState,
    graph: &KnowledgeGraph,
    center_idx: usize,
) -> (LocalSubgraph, Vec<(f32, f32)>) {
    let key = local_graph_key(graph, center_idx, st.depth);
    let mut cache = st.cache.borrow_mut();
    if let Some(cached) = cache.as_ref()
        && cached.key == key
    {
        return (cached.sub.clone(), cached.norm.clone());
    }
    let sub = build_local_subgraph(graph, center_idx, st.depth, LOCAL_GRAPH_MAX_NODES);
    let norm = centered_layout(&sub);
    *cache = Some(LocalGraphCache {
        key,
        sub: sub.clone(),
        norm: norm.clone(),
    });
    (sub, norm)
}

fn local_screen_positions(norm: &[(f32, f32)], w: f32, h: f32) -> Vec<(f32, f32)> {
    norm.iter().map(|&(nx, ny)| (nx * w, ny * h)).collect()
}

fn local_node_radius(hop: u8, link_count: usize, hovered: bool) -> f32 {
    let base = match hop {
        0 => 6.5,
        1 => (4.2 + (link_count as f32).sqrt() * 0.55).min(6.0),
        _ => 3.5,
    };
    if hovered { base + 1.2 } else { base }
}

fn active_center_node_idx(app: &RustdianApp) -> Option<usize> {
    let active_id = app.vault.active_note().map(|n| n.id.as_str())?;
    app.vault.graph.nodes.iter().position(|n| n.id == active_id)
}

fn local_node_at(app: &RustdianApp, pos: Point<Pixels>) -> Option<usize> {
    let center_idx = active_center_node_idx(app)?;
    let bounds = app.local_graph.canvas_bounds.get()?;
    let lx = f32::from(pos.x) - f32::from(bounds.origin.x);
    let ly = f32::from(pos.y) - f32::from(bounds.origin.y);
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if lx < 0.0 || ly < 0.0 || lx > w || ly > h {
        return None;
    }
    let (sub, norm) = local_layout(&app.local_graph, &app.vault.graph, center_idx);
    let sp = local_screen_positions(&norm, w, h);
    let mut best: Option<(f32, usize)> = None;
    for (i, node) in sub.nodes.iter().enumerate() {
        let Some(&(nx, ny)) = sp.get(i) else {
            continue;
        };
        let r = local_node_radius(node.hop, node.link_count, false) + 5.0;
        let d = ((nx - lx).powi(2) + (ny - ly).powi(2)).sqrt();
        if d <= r && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

fn open_local_node(app: &mut RustdianApp, node: &LocalNode, background: bool) {
    let in_sync = app
        .vault
        .notes
        .get(node.note_index)
        .is_some_and(|n| n.id == node.id);
    if in_sync {
        if background && !app.vault.open_tabs.is_empty() {
            if !app.vault.open_tabs.contains(&node.note_index) {
                app.vault.open_tabs.push(node.note_index);
            }
        } else {
            app.open_note_by_index(node.note_index);
        }
    } else if !background {
        app.open_note_by_title(&node.title);
    }
}

struct LocalLabelDraw {
    idx: usize,
    text: String,
    emphasized: bool,
}

struct LocalScene {
    theme: ActiveTheme,
    bg: Rgba,
    norm: Vec<(f32, f32)>,
    radii: Vec<f32>,
    hops: Vec<u8>,
    hovered: Option<usize>,
    edges: Vec<(usize, usize)>,
    labels: Vec<LocalLabelDraw>,
}

fn paint_local_scene(
    scene: &LocalScene,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    if w <= 1.0 || h <= 1.0 {
        return;
    }
    let t = scene.theme;
    let sp = local_screen_positions(&scene.norm, w, h);
    let at = |(x, y): (f32, f32)| point(px(ox + x), px(oy + y));

    let mut plain_edges = Vec::new();
    let mut hot_edges = Vec::new();
    for &(a, b) in &scene.edges {
        let hot = a == 0 || b == 0 || Some(a) == scene.hovered || Some(b) == scene.hovered;
        if hot {
            hot_edges.push((a, b));
        } else {
            plain_edges.push((a, b));
        }
    }
    for (edges, width, color) in [
        (&plain_edges, 1.0, with_alpha(t.tx3, 0.25)),
        (&hot_edges, 1.25, with_alpha(t.ac, 0.65)),
    ] {
        if edges.is_empty() {
            continue;
        }
        let mut pb = PathBuilder::stroke(px(width));
        for &(a, b) in edges {
            if let (Some(&pa), Some(&pb_pt)) = (sp.get(a), sp.get(b)) {
                pb.move_to(at(pa));
                pb.line_to(at(pb_pt));
            }
        }
        if let Ok(path) = pb.build() {
            window.paint_path(path, Hsla::from(color));
        }
    }

    for (i, &(x, y)) in sp.iter().enumerate() {
        let r = scene.radii[i];
        let (cx_pt, cy_pt) = (ox + x, oy + y);
        let hop = scene.hops[i];
        let is_hot = Some(i) == scene.hovered;
        if hop == 0 {
            let outer = r + 6.0;
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(px(cx_pt - outer), px(cy_pt - outer)),
                        size(px(outer * 2.0), px(outer * 2.0)),
                    ),
                    transparent_black(),
                )
                .corner_radii(Corners::all(px(outer)))
                .border_widths(Edges::all(px(4.0)))
                .border_color(Hsla::from(with_alpha(t.ac, 0.30))),
            );
        }
        let color = if hop == 0 || is_hot {
            t.ac
        } else if hop == 1 {
            with_alpha(t.ac, 0.78)
        } else {
            with_alpha(t.tx3, 0.55)
        };
        window.paint_quad(
            fill(
                Bounds::new(
                    point(px(cx_pt - r), px(cy_pt - r)),
                    size(px(r * 2.0), px(r * 2.0)),
                ),
                Hsla::from(color),
            )
            .corner_radii(Corners::all(px(r))),
        );
    }

    let mut placed: Vec<(f32, f32, f32, f32)> = Vec::new();
    for lab in &scene.labels {
        let Some(&(x, y)) = sp.get(lab.idx) else {
            continue;
        };
        let r = scene.radii[lab.idx];
        let mut f = font(UI_FONT);
        if lab.emphasized {
            f.weight = FontWeight::SEMIBOLD;
        }
        let color: Hsla = if lab.emphasized { t.tx } else { t.tx2 }.into();
        let run = TextRun {
            len: lab.text.len(),
            font: f,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(lab.text.clone()),
            px(LOCAL_LABEL_PX),
            &[run],
            None,
        );
        let text_w = f32::from(shaped.width);
        let pad_x = 4.0;
        let pad_y = 1.5;
        let box_w = text_w + pad_x * 2.0;
        let box_h = LOCAL_LABEL_LINE_PX + pad_y * 2.0;

        let cx_pt = ox + x;
        let cy_pt = oy + y;
        let candidates = [
            (
                (cx_pt - text_w / 2.0).clamp(ox + pad_x + 2.0, ox + w - text_w - pad_x - 2.0),
                cy_pt + r + 5.0,
            ),
            (
                (cx_pt - text_w / 2.0).clamp(ox + pad_x + 2.0, ox + w - text_w - pad_x - 2.0),
                cy_pt - r - LOCAL_LABEL_LINE_PX - 5.0,
            ),
        ];
        let chosen = candidates.into_iter().find(|&(tx, ty)| {
            let rect = (tx - pad_x, ty - pad_y, box_w, box_h);
            let inside = rect.0 >= ox
                && rect.0 + rect.2 <= ox + w
                && rect.1 >= oy
                && rect.1 + rect.3 <= oy + h;
            let overlaps = placed.iter().any(|p| {
                rect.0 < p.0 + p.2
                    && p.0 < rect.0 + rect.2
                    && rect.1 < p.1 + p.3
                    && p.1 < rect.1 + rect.3
            });
            inside && !overlaps
        });
        let Some((tx, ty)) = chosen else {
            continue;
        };
        let rect = (tx - pad_x, ty - pad_y, box_w, box_h);
        placed.push(rect);
        window.paint_quad(
            fill(
                Bounds::new(point(px(rect.0), px(rect.1)), size(px(rect.2), px(rect.3))),
                Hsla::from(with_alpha(scene.bg, 0.88)),
            )
            .corner_radii(Corners::all(px(4.0))),
        );
        let _ = shaped.paint(point(px(tx), px(ty)), px(LOCAL_LABEL_LINE_PX), window, cx);
    }
}

fn render_local_graph(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let st = &app.local_graph;
    let depth = st.depth;
    let center_idx = active_center_node_idx(app);
    let link_peers = center_idx
        .map(|idx| {
            local_layout(st, &app.vault.graph, idx)
                .0
                .nodes
                .len()
                .saturating_sub(1)
        })
        .unwrap_or(0);

    let depth_chip = |hop: u8, label: String| {
        let active = depth == hop;
        div()
            .id(("local_graph_depth", hop as usize))
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(RADIUS_S))
            .text_size(px(11.5))
            .font_weight(if active {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .cursor_pointer()
            .text_color(if active { t.ac } else { t.tx2 })
            .when(active, |d| d.bg(t.ac_soft))
            .when(!active, |d| d.hover(|s| s.bg(t.hover).text_color(t.tx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.local_graph.depth = hop;
                    this.local_graph.hovered = None;
                    cx.notify();
                }),
            )
            .child(SharedString::from(label))
    };

    let open_full = div()
        .id("local_graph_open_full")
        .flex()
        .items_center()
        .gap(px(4.0))
        .px(px(7.0))
        .py(px(3.0))
        .rounded(px(RADIUS_S))
        .text_size(px(11.5))
        .text_color(t.tx2)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover).text_color(t.tx))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.run_command(crate::AppCommand::OpenGraph);
                cx.notify();
            }),
        )
        .child(icon("affiliate", 12.0, t.tx2))
        .child(SharedString::from(crate::t!("panel-local-graph-open-full")));

    let toolbar = div()
        .flex_none()
        .px(px(12.0))
        .pt(px(8.0))
        .pb(px(6.0))
        .border_b_1()
        .border_color(t.line_soft)
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(depth_chip(1, crate::t!("panel-local-graph-depth-1")))
                .child(depth_chip(2, crate::t!("panel-local-graph-depth-2")))
                .when(link_peers > 0, |d| {
                    d.child(
                        div()
                            .pl(px(4.0))
                            .text_size(px(11.0))
                            .text_color(t.tx3)
                            .child(SharedString::from(format!("{link_peers}"))),
                    )
                }),
        )
        .child(open_full);

    let Some(center_idx) = center_idx else {
        return div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(16.0))
                    .child(empty_state(
                        &t,
                        "affiliate",
                        &crate::t!("panel-local-graph-no-note"),
                        "",
                    )),
            )
            .into_any_element();
    };

    let (sub, norm) = local_layout(st, &app.vault.graph, center_idx);
    if sub.nodes.len() <= 1 {
        let subtitle = sub
            .nodes
            .first()
            .map(|n| n.title.clone())
            .unwrap_or_default();
        return div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(16.0))
                    .child(empty_state(
                        &t,
                        "affiliate",
                        &crate::t!("panel-local-graph-empty"),
                        &subtitle,
                    )),
            )
            .into_any_element();
    }

    let hovered = st.hovered.filter(|&i| i < sub.nodes.len());
    let radii: Vec<f32> = sub
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| local_node_radius(n.hop, n.link_count, Some(i) == hovered))
        .collect();
    let hops: Vec<u8> = sub.nodes.iter().map(|n| n.hop).collect();

    let mut label_order: Vec<(u8, usize, usize)> = sub
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let prio = if i == 0 {
                0
            } else if Some(i) == hovered {
                1
            } else if n.hop == 1 {
                2
            } else {
                3
            };
            (prio, usize::MAX - n.link_count, i)
        })
        .collect();
    label_order.sort();
    let labels = label_order
        .into_iter()
        .map(|(prio, _, i)| LocalLabelDraw {
            idx: i,
            text: truncate_label(&sub.nodes[i].title, 14),
            emphasized: prio <= 1,
        })
        .collect();

    let scene = LocalScene {
        theme: t,
        bg: t.panel,
        norm,
        radii,
        hops,
        hovered,
        edges: sub.edges,
        labels,
    };
    let bounds_cell = st.canvas_bounds.clone();
    let cursor_pointer = hovered.is_some();

    let plot = div()
        .id("local_graph_canvas")
        .flex_1()
        .min_h_0()
        .w_full()
        .when(cursor_pointer, |d| d.cursor(CursorStyle::PointingHand))
        .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
            let hit = local_node_at(this, ev.position);
            if hit != this.local_graph.hovered {
                this.local_graph.hovered = hit;
                cx.notify();
            }
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                if let Some(hit) = local_node_at(this, ev.position)
                    && let Some(center) = active_center_node_idx(this)
                {
                    let (sub, _) = local_layout(&this.local_graph, &this.vault.graph, center);
                    if let Some(node) = sub.nodes.get(hit) {
                        open_local_node(this, node, primary_held(&ev.modifiers));
                        cx.notify();
                    }
                }
            }),
        )
        .on_mouse_up(
            MouseButton::Middle,
            cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                if let Some(hit) = local_node_at(this, ev.position)
                    && let Some(center) = active_center_node_idx(this)
                {
                    let (sub, _) = local_layout(&this.local_graph, &this.vault.graph, center);
                    if let Some(node) = sub.nodes.get(hit) {
                        open_local_node(this, node, true);
                        cx.notify();
                    }
                }
            }),
        )
        .child(
            canvas(
                move |bounds, _, _| {
                    bounds_cell.set(Some(bounds));
                },
                move |bounds, (), window, cx| {
                    paint_local_scene(&scene, bounds, window, cx);
                },
            )
            .size_full(),
        );

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(toolbar)
        .child(plot)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{GraphEdge, GraphNode};

    fn sample_graph() -> KnowledgeGraph {
        let mk_node = |idx: usize, title: &str, lc: usize| GraphNode {
            id: format!("id-{idx}"),
            title: title.to_string(),
            note_index: idx,
            x: 0.0,
            y: 0.0,
            radius: 4.0,
            link_count: lc,
        };
        KnowledgeGraph {
            nodes: vec![
                mk_node(0, "Center", 2),
                mk_node(1, "Hop1A", 3),
                mk_node(2, "Hop1B", 1),
                mk_node(3, "Hop2", 2),
                mk_node(4, "Hop3", 1),
                mk_node(5, "Isolated", 0),
            ],
            edges: vec![
                GraphEdge {
                    from_index: 0,
                    to_index: 1,
                },
                GraphEdge {
                    from_index: 0,
                    to_index: 2,
                },
                GraphEdge {
                    from_index: 1,
                    to_index: 3,
                },
                GraphEdge {
                    from_index: 3,
                    to_index: 4,
                },
            ],
        }
    }

    #[test]
    fn local_subgraph_respects_depth_1_and_2_and_centers_root() {
        let g = sample_graph();

        let sub1 = build_local_subgraph(&g, 0, 1, 40);
        assert_eq!(sub1.nodes[0].title, "Center");
        assert_eq!(sub1.nodes[0].hop, 0);
        let titles1: Vec<&str> = sub1.nodes.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles1, vec!["Center", "Hop1A", "Hop1B"]);
        assert_eq!(sub1.edges.len(), 2);

        let sub2 = build_local_subgraph(&g, 0, 2, 40);
        let titles2: Vec<&str> = sub2.nodes.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles2, vec!["Center", "Hop1A", "Hop1B", "Hop2"]);
        assert_eq!(sub2.edges.len(), 3);
    }

    #[test]
    fn local_subgraph_handles_isolated_and_max_nodes_cap() {
        let g = sample_graph();
        let iso = build_local_subgraph(&g, 5, 2, 40);
        assert_eq!(iso.nodes.len(), 1);
        assert_eq!(iso.nodes[0].title, "Isolated");
        assert!(iso.edges.is_empty());

        let capped = build_local_subgraph(&g, 0, 2, 2);
        assert_eq!(capped.nodes.len(), 2);
        assert_eq!(capped.nodes[0].title, "Center");
        assert_eq!(capped.nodes[1].title, "Hop1A");
    }

    #[test]
    fn link_mention_skips_frontmatter_code_and_existing_links() {
        let md = "---\ntitle: Rust\n---\n\n`Rust` 代码\n```\nRust\n```\n参见 https://example.com/Rust 与 [Rust](r.md)\n已有 [[Rust]]，这里还有真正的 Rust 笔记。\n";
        let (updated, line_idx) =
            link_first_unlinked_mention(md, "Rust").expect("should find bare mention");
        assert_eq!(line_idx, 9);
        assert!(updated.contains("这里还有真正的 [[Rust]] 笔记。"));
        assert!(updated.contains("title: Rust"));
        assert!(updated.contains("`Rust` 代码"));
        assert!(updated.contains("https://example.com/Rust"));
    }

    #[test]
    fn link_mention_respects_ascii_boundaries_and_cjk() {
        let ascii = "Rustdian 不是 Rust 语言";
        let (updated, line_idx) = link_first_unlinked_mention(ascii, "Rust").unwrap();
        assert_eq!(line_idx, 0);
        assert_eq!(updated, "Rustdian 不是 [[Rust]] 语言");

        let cjk = "今天整理了知识图谱相关内容";
        let (updated_cjk, _) = link_first_unlinked_mention(cjk, "知识图谱").unwrap();
        assert_eq!(updated_cjk, "今天整理了[[知识图谱]]相关内容");
    }

    #[test]
    fn find_mention_line_prefers_body_over_frontmatter() {
        let md = "---\nref: 架构设计\n---\n正文提到架构设计。\n";
        assert_eq!(find_mention_line(md, "架构设计"), Some(3));
    }
}
