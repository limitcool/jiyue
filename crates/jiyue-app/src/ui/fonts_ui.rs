//! The font and size rows of the 外观 page: the three font pickers (searchable, each name drawn in
//! its own face), the body size, line height and code size steppers, a live
//! preview and the reset buttons. Changes apply at once; the file is written after a short pause
//! (see `schedule_save`). The rules live in `ui/fonts.rs`.

use std::time::Duration;

use gpui::{
    AnyElement, App, Context, Div, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use jiyue_core::config::appearance::{
    Appearance, CODE_SIZE_FOLLOW, CODE_SIZE_RANGE, EDITOR_SIZE_DEFAULT, EDITOR_SIZE_RANGE,
    LINE_HEIGHT_DEFAULT,
};

use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::ext::ToastKind;
use crate::ui::fonts::{self, Slot};
use crate::ui::settings::setting_row;
use crate::ui::widgets::controls::DANGER_RGB;
use crate::ui::widgets::{LineInput, LineInputEvent};

/// Pause after the last change before the file is written.
const SAVE_DELAY: Duration = Duration::from_millis(300);
/// Most families drawn in the picker at once (narrow with the search box).
const PICKER_MAX_ROWS: usize = 200;

/// A numeric field that is being typed into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumField {
    EditorSize,
    CodeSize,
    LineHeight,
}

/// The open font picker: which font it changes, and its search field.
pub struct Picker {
    pub slot: Slot,
    pub input: gpui::Entity<LineInput>,
    _sub: Subscription,
}

/// The numeric field being typed into.
pub struct NumberEdit {
    pub field: NumField,
    pub input: gpui::Entity<LineInput>,
    _sub: Subscription,
}

/// State of the font rows. Lives in `SettingsState`.
#[derive(Default)]
pub struct FontUi {
    pub picker: Option<Picker>,
    pub number: Option<NumberEdit>,
    /// Counts the changes; a pending write runs only when it is still the latest one.
    save_seq: u64,
}

/// The rows of the 字体与字号 block, below the theme and sidebar rows of the 外观 page.
pub fn render_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    fonts::ensure_installed(|| cx.text_system().all_font_names());
    let cfg = fonts::appearance();
    let snapshot = fonts::current();
    let m = fonts::current_metrics();

    let mut block = div()
        .flex()
        .flex_col()
        .mt(px(14.0))
        .pt(px(10.0))
        .border_t_1()
        .border_color(t.line_soft)
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx)
                .child("字体与字号"),
        )
        .child(
            div()
                .pb(px(4.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child("对所有笔记库生效，修改后立即显示在编辑区和界面中"),
        );

    // Font families.
    let ui_row = font_row(
        app,
        cx,
        Slot::Ui,
        &cfg.ui_font,
        "默认界面字体",
        &snapshot.ui_family,
    );
    let editor_row = font_row(
        app,
        cx,
        Slot::Editor,
        &cfg.editor_font,
        "与界面字体相同",
        &snapshot.editor_family,
    );
    let code_row = font_row(
        app,
        cx,
        Slot::Code,
        &cfg.code_font,
        "默认等宽字体",
        &snapshot.code_family,
    );
    block = block
        .child(ui_row)
        .child(editor_row)
        .child(code_row)
        .children(missing_lines(&snapshot.missing))
        .children(missing_lines(&snapshot.clamp_warnings));

    // Sizes.
    block = block
        .child(setting_row(
            &t,
            "正文字号",
            "编辑区与阅读模式的文字大小（标题按比例放大）",
            size_control(app, cx, NumField::EditorSize, &cfg),
            false,
        ))
        .child(preset_row(app, cx, &cfg))
        .child(setting_row(
            &t,
            "行高",
            "行与行之间的距离，相对于字号的倍数",
            number_stepper(app, cx, NumField::LineHeight, line_text(&cfg), -1, 1),
            false,
        ))
        .child(setting_row(
            &t,
            "代码字号",
            "代码块与行内代码；“跟随正文”时为正文的 90%",
            code_size_control(app, cx, &cfg),
            true,
        ));

    block = block
        .child(preview(
            &t,
            &snapshot.editor_family,
            &snapshot.code_family,
            &m,
        ))
        .child(reset_row(app, cx));
    block
}

// ---------------------------------------------------------------------------
// Font families
// ---------------------------------------------------------------------------

/// One font row: the title, the trigger that opens the picker, and the picker when it is open.
fn font_row(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    slot: Slot,
    value: &str,
    default_label: &str,
    resolved: &str,
) -> Div {
    let t = app.theme;
    let open = app.settings.fonts.picker.as_ref().map(|p| p.slot) == Some(slot);
    let (title, desc) = match slot {
        Slot::Ui => ("界面字体", "菜单、侧栏、标签和按钮的字体"),
        Slot::Editor => ("正文字体", "编辑区与阅读模式的字体"),
        Slot::Code => ("代码字体", "代码块与行内代码的字体"),
    };
    let shown = if value.is_empty() {
        format!("默认（{default_label}）")
    } else {
        value.to_string()
    };
    let trigger = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .w(px(240.0))
        .h(px(32.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(if open { t.ac } else { t.line })
        .bg(t.sheet)
        .cursor_pointer()
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_family(SharedString::from(resolved.to_string()))
                .text_size(px(14.0))
                .text_color(t.tx)
                .child(SharedString::from(shown)),
        )
        .child(crate::ui::icons::icon("selector", 15.0, t.tx3))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                toggle_picker(this, slot, window, cx);
                cx.notify();
            }),
        );
    let mut block = div()
        .flex()
        .flex_col()
        .child(setting_row(&t, title, desc, trigger, false));
    if open {
        block = block.children(picker_panel(app, cx, slot, value));
    }
    block
}

/// The searchable list under a font row: the default entry first, then the matches.
fn picker_panel(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    slot: Slot,
    value: &str,
) -> Option<AnyElement> {
    let t = app.theme;
    let picker = app.settings.fonts.picker.as_ref()?;
    let query = picker.input.read(cx).text();
    let list = fonts::installed();
    let matches = fonts::filter_fonts(&list, &query);

    let mut rows = div()
        .id("font_picker_list")
        .flex()
        .flex_col()
        .gap(px(1.0))
        .max_h(px(240.0))
        .overflow_y_scroll();
    rows = rows.child(picker_row(
        &t,
        cx,
        slot,
        String::new(),
        "默认（系统）".to_string(),
        value.is_empty(),
        None,
    ));
    for name in matches.iter().take(PICKER_MAX_ROWS) {
        rows = rows.child(picker_row(
            &t,
            cx,
            slot,
            name.clone(),
            name.clone(),
            value.eq_ignore_ascii_case(name),
            Some(name.clone()),
        ));
    }
    let footer = if list.is_empty() {
        "正在读取字体列表…".to_string()
    } else if matches.is_empty() {
        "没有匹配的字体".to_string()
    } else if matches.len() > PICKER_MAX_ROWS {
        format!(
            "还有 {} 个，请输入更多文字缩小范围",
            matches.len() - PICKER_MAX_ROWS
        )
    } else {
        format!("共 {} 个字体", matches.len())
    };

    let search = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(8.0))
        .py(px(6.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(28.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(t.line)
                .bg(t.sheet)
                .child(picker.input.clone()),
        );
    Some(
        div()
            .mb(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(t.line)
            .bg(t.raised)
            .shadow_md()
            .overflow_hidden()
            .child(search)
            .child(rows)
            .child(
                div()
                    .px(px(10.0))
                    .py(px(6.0))
                    .border_t_1()
                    .border_color(t.line_soft)
                    .text_size(px(11.5))
                    .text_color(t.tx3)
                    .child(SharedString::from(footer)),
            )
            .into_any_element(),
    )
}

/// One picker row. `family` is the name the row is drawn in (`None`: the interface font).
fn picker_row(
    t: &ActiveTheme,
    cx: &mut Context<RustdianApp>,
    slot: Slot,
    value: String,
    label: String,
    selected: bool,
    family: Option<String>,
) -> Div {
    let mut name = div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(px(14.0))
        .text_color(t.tx)
        .child(SharedString::from(label.clone()));
    if let Some(f) = &family {
        name = name.font_family(SharedString::from(f.clone()));
    }
    let mut row = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(30.0))
        .mx(px(4.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .child(name);
    if let Some(f) = &family {
        let mut tags = div().flex().flex_none().gap(px(4.0));
        if fonts::is_cjk_font(f) {
            tags = tags.child(tag(t, "中文"));
        }
        if fonts::is_mono_font(f) {
            tags = tags.child(tag(t, "等宽"));
        }
        row = row.child(tags);
    }
    if selected {
        row = row
            .bg(t.active)
            .child(crate::ui::icons::icon("check", 13.0, t.ac));
    } else {
        row = row.hover(|s| s.bg(t.hover));
    }
    row.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            let name = value.clone();
            edit(this, cx, |a| match slot {
                Slot::Ui => a.ui_font = name,
                Slot::Editor => a.editor_font = name,
                Slot::Code => a.code_font = name,
            });
            this.settings.fonts.picker = None;
            cx.notify();
        }),
    )
}

fn tag(t: &ActiveTheme, text: &str) -> Div {
    div()
        .px(px(4.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(t.line)
        .text_size(px(10.5))
        .text_color(t.tx3)
        .child(SharedString::from(text.to_string()))
}

/// Opens the picker for `slot` (or closes it when it is already open).
fn toggle_picker(
    app: &mut RustdianApp,
    slot: Slot,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    if app.settings.fonts.picker.as_ref().map(|p| p.slot) == Some(slot) {
        app.settings.fonts.picker = None;
        return;
    }
    app.settings.fonts.number = None;
    if fonts::installed().is_empty() {
        // The list is read when the editor first paints; the settings page asks for it too.
        fonts::ensure_installed(|| cx.text_system().all_font_names());
    }
    let input = cx.new(|cx| LineInput::new(cx, "搜索字体…"));
    let theme = app.theme;
    input.update(cx, |i, cx| i.set_theme(theme, cx));
    let sub = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            match ev {
                LineInputEvent::Changed(_) => {}
                LineInputEvent::Cancel => this.settings.fonts.picker = None,
                LineInputEvent::Submit(query) => {
                    let first = this
                        .settings
                        .fonts
                        .picker
                        .as_ref()
                        .map(|p| p.slot)
                        .and_then(|slot| {
                            fonts::filter_fonts(&fonts::installed(), query)
                                .into_iter()
                                .next()
                                .map(|name| (slot, name))
                        });
                    if let Some((slot, name)) = first {
                        edit(this, cx, |a| match slot {
                            Slot::Ui => a.ui_font = name,
                            Slot::Editor => a.editor_font = name,
                            Slot::Code => a.code_font = name,
                        });
                        this.settings.fonts.picker = None;
                    }
                }
            }
            cx.notify();
        },
    );
    input.update(cx, |i, _| i.focus(window));
    app.settings.fonts.picker = Some(Picker {
        slot,
        input,
        _sub: sub,
    });
}

// ---------------------------------------------------------------------------
// Sizes
// ---------------------------------------------------------------------------

fn fmt_num(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn line_text(cfg: &Appearance) -> String {
    format!("{:.2}", cfg.line_height)
}

fn size_control(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    field: NumField,
    cfg: &Appearance,
) -> Div {
    number_stepper(
        app,
        cx,
        field,
        format!("{} px", fmt_num(cfg.editor_font_size)),
        -1,
        1,
    )
}

fn code_size_control(app: &RustdianApp, cx: &mut Context<RustdianApp>, cfg: &Appearance) -> Div {
    let label = if cfg.code_font_size == CODE_SIZE_FOLLOW {
        let resolved = fonts::current().code_size;
        format!("跟随正文 {} px", fmt_num(resolved))
    } else {
        format!("{} px", fmt_num(cfg.code_font_size))
    };
    number_stepper(app, cx, NumField::CodeSize, label, -1, 1)
}

/// Stepper: − / value / +. The value is typed into when clicked (Enter applies, Escape cancels).
/// `down` and `up` are the raw steps given to `step`: `±1` for sizes (and `±1` tenth for the line
/// height).
fn number_stepper(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    field: NumField,
    label: String,
    down: i32,
    up: i32,
) -> Div {
    let t = app.theme;
    let editing = app
        .settings
        .fonts
        .number
        .as_ref()
        .filter(|n| n.field == field);
    let value: AnyElement = match editing {
        Some(n) => div()
            .w(px(96.0))
            .h(px(28.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(t.ac)
            .bg(t.sheet)
            .child(n.input.clone())
            .into_any_element(),
        None => div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(96.0))
            .h(px(28.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(13.0))
            .text_color(t.tx)
            .hover(|s| s.bg(t.hover))
            .child(SharedString::from(label))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    open_number(this, field, window, cx);
                    cx.notify();
                }),
            )
            .into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(step_button(
            &t,
            "−",
            cx.listener(move |this, _, _, cx| {
                step(this, field, down, cx);
                cx.notify();
            }),
        ))
        .child(value)
        .child(step_button(
            &t,
            "+",
            cx.listener(move |this, _, _, cx| {
                step(this, field, up, cx);
                cx.notify();
            }),
        ))
}

fn step_button(
    t: &ActiveTheme,
    label: &str,
    on: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .cursor_pointer()
        .text_size(px(15.0))
        .text_color(t.tx2)
        .hover(|s| s.bg(t.hover))
        .child(SharedString::from(label.to_string()))
        .on_mouse_down(MouseButton::Left, on)
}

/// One step of a field. `delta` is the stepper's unit: sizes and the line height use `±1`, which
/// the rules in `fonts::step_*` turn into the next value.
fn step(app: &mut RustdianApp, field: NumField, delta: i32, cx: &mut Context<RustdianApp>) {
    let body = fonts::appearance().editor_font_size;
    edit(app, cx, |a| match field {
        NumField::EditorSize => {
            a.editor_font_size =
                fonts::step_float(a.editor_font_size, delta as f32, fonts::STEP_EDITOR);
        }
        NumField::CodeSize => {
            a.code_font_size =
                fonts::step_code(a.code_font_size, body, delta as f32, fonts::STEP_CODE);
        }
        NumField::LineHeight => {
            a.line_height = fonts::step_float(a.line_height, delta as f32 / 10.0, fonts::STEP_LINE);
        }
    });
}

/// Starts typing into a numeric field.
fn open_number(
    app: &mut RustdianApp,
    field: NumField,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    let cfg = fonts::appearance();
    let text = match field {
        NumField::EditorSize => fmt_num(cfg.editor_font_size),
        NumField::CodeSize => fmt_num(cfg.code_font_size),
        NumField::LineHeight => format!("{:.2}", cfg.line_height),
    };
    app.settings.fonts.picker = None;
    let input = cx.new(|cx| LineInput::new(cx, "输入数字后按 Enter"));
    let theme = app.theme;
    input.update(cx, |i, cx| {
        i.set_theme(theme, cx);
        i.set_text(&text, cx);
    });
    let sub = cx.subscribe(
        &input,
        move |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            match ev {
                LineInputEvent::Changed(_) => {}
                LineInputEvent::Cancel => this.settings.fonts.number = None,
                LineInputEvent::Submit(text) => {
                    if let Some(v) = fonts::parse_number(text) {
                        apply_number(this, field, v, cx);
                    }
                    this.settings.fonts.number = None;
                }
            }
            cx.notify();
        },
    );
    input.update(cx, |i, _| i.focus(window));
    app.settings.fonts.number = Some(NumberEdit {
        field,
        input,
        _sub: sub,
    });
}

/// Applies a typed number. Out-of-range values are clamped by the rules (the file keeps the
/// clamped value, see `Appearance::clamped`).
fn apply_number(app: &mut RustdianApp, field: NumField, v: f32, cx: &mut Context<RustdianApp>) {
    edit(app, cx, |a| match field {
        NumField::EditorSize => {
            a.editor_font_size = v.clamp(EDITOR_SIZE_RANGE.0, EDITOR_SIZE_RANGE.1)
        }
        NumField::CodeSize => {
            a.code_font_size = if v == 0.0 {
                CODE_SIZE_FOLLOW
            } else {
                v.clamp(CODE_SIZE_RANGE.0, CODE_SIZE_RANGE.1)
            };
        }
        NumField::LineHeight => a.line_height = v,
    });
}

/// The quick presets of the body size, under its stepper.
fn preset_row(app: &RustdianApp, cx: &mut Context<RustdianApp>, cfg: &Appearance) -> Div {
    let t = app.theme;
    let mut row = div().flex().gap(px(6.0)).pb(px(6.0)).justify_end();
    for &p in fonts::EDITOR_PRESETS.iter() {
        let on = fonts::snap_preset(cfg.editor_font_size, &[p]).is_some();
        row = row.child(chip(
            &t,
            &fmt_num(p),
            on,
            cx.listener(move |this, _, _, cx| {
                edit(this, cx, |a| a.editor_font_size = p);
                cx.notify();
            }),
        ));
    }
    row
}

fn chip(
    t: &ActiveTheme,
    label: &str,
    on: bool,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let mut chip = div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(24.0))
        .px(px(9.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(if on { t.ac } else { t.line })
        .text_size(px(12.0))
        .cursor_pointer()
        .text_color(if on { t.ac } else { t.tx2 })
        .child(SharedString::from(label.to_string()));
    if on {
        chip = chip.bg(t.active);
    } else {
        chip = chip.hover(|s| s.bg(t.hover));
    }
    chip.on_mouse_down(MouseButton::Left, on_click)
}

// ---------------------------------------------------------------------------
// Preview, warnings and resets
// ---------------------------------------------------------------------------

fn missing_lines(messages: &[String]) -> Vec<Div> {
    messages
        .iter()
        .map(|m| {
            div()
                .pt(px(4.0))
                .text_size(px(12.0))
                .text_color(gpui::rgb(DANGER_RGB))
                .child(SharedString::from(m.clone()))
        })
        .collect()
}

/// A short sample in the chosen body and code faces and sizes.
fn preview(t: &ActiveTheme, body: &str, code: &str, m: &fonts::Metrics) -> Div {
    let h2 = m.heading[1];
    div()
        .mt(px(12.0))
        .mb(px(8.0))
        .p(px(14.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(div().text_size(px(11.5)).text_color(t.tx3).child("预览"))
        .child(
            div()
                .font_family(SharedString::from(body.to_string()))
                .text_size(px(h2))
                .line_height(px(m.heading_lh[1]))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.tx)
                .child("标题示例 Heading"),
        )
        .child(
            div()
                .font_family(SharedString::from(body.to_string()))
                .text_size(px(m.font))
                .line_height(px(m.line))
                .text_color(t.tx)
                .child("霁月是一款本地优先的 Markdown 笔记应用。The quick brown fox 1234567890"),
        )
        .child(
            div()
                .font_family(SharedString::from(code.to_string()))
                .text_size(px(m.code))
                .line_height(px(m.code_row))
                .text_color(t.tx2)
                .child("fn main() { println!(\"你好，世界\"); }"),
        )
}

/// Buttons that put the font names and the sizes back to their defaults.
fn reset_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    div()
        .flex()
        .gap(px(8.0))
        .pt(px(4.0))
        .child(reset_button(
            &t,
            "恢复默认字体",
            cx.listener(|this, _, _, cx| {
                edit(this, cx, |a| {
                    a.ui_font.clear();
                    a.editor_font.clear();
                    a.code_font.clear();
                });
                this.settings.fonts.picker = None;
                cx.notify();
            }),
        ))
        .child(reset_button(
            &t,
            "恢复默认字号",
            cx.listener(|this, _, _, cx| {
                edit(this, cx, |a| {
                    a.editor_font_size = EDITOR_SIZE_DEFAULT;
                    a.code_font_size = CODE_SIZE_FOLLOW;
                    a.line_height = LINE_HEIGHT_DEFAULT;
                });
                this.settings.fonts.number = None;
                cx.notify();
            }),
        ))
}

fn reset_button(
    t: &ActiveTheme,
    label: &str,
    on: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(28.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .cursor_pointer()
        .text_size(px(12.5))
        .text_color(t.tx2)
        .hover(|s| s.bg(t.hover))
        .child(SharedString::from(label.to_string()))
        .on_mouse_down(MouseButton::Left, on)
}

// ---------------------------------------------------------------------------
// Changes and saving
// ---------------------------------------------------------------------------

/// Applies one change: the new settings take effect at once, and the file is written after a
/// short pause (so a stepper held down writes once).
pub fn edit(
    app: &mut RustdianApp,
    cx: &mut Context<RustdianApp>,
    change: impl FnOnce(&mut Appearance),
) {
    let mut value = fonts::appearance();
    change(&mut value);
    let (clean, _) = value.clamped();
    fonts::set_appearance(clean);
    schedule_save(app, cx);
}

fn schedule_save(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.settings.fonts.save_seq += 1;
    let seq = app.settings.fonts.save_seq;
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(SAVE_DELAY).await;
        let _ = this.update(cx, |app, cx| {
            if app.settings.fonts.save_seq == seq {
                save_now(app);
                cx.notify();
            }
        });
    })
    .detach();
}

fn save_now(app: &mut RustdianApp) {
    let value = fonts::appearance();
    if let Err(message) = app.configs.write_appearance(&value) {
        app.toast(ToastKind::Error, message);
    }
}

/// Keyboard for the font rows: Escape closes an open picker or number field first. Returns true
/// when the key was used.
pub fn handle_escape(app: &mut RustdianApp) -> bool {
    if app.settings.fonts.picker.is_some() {
        app.settings.fonts.picker = None;
        return true;
    }
    if app.settings.fonts.number.is_some() {
        app.settings.fonts.number = None;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_without_trailing_zeros() {
        assert_eq!(fmt_num(15.0), "15");
        assert_eq!(fmt_num(17.5), "17.5");
    }

    #[test]
    fn line_height_shows_two_decimals() {
        let cfg = Appearance::default();
        assert_eq!(line_text(&cfg), "1.73");
    }
}
