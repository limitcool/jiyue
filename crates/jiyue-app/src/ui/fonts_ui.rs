//! The font and size rows of the 外观 page: the three font pickers (searchable, each name drawn in
//! its own face), the body size, line height and code size steppers, a live
//! preview and the reset buttons. Changes apply at once; the file is written after a short pause
//! (see `schedule_save`). The rules live in `ui/fonts.rs`.

use std::time::Duration;

use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, MouseButton, ScrollHandle, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use jiyue_core::config::appearance::{
    Appearance, CODE_SIZE_FOLLOW, CODE_SIZE_RANGE, EDITOR_SIZE_DEFAULT, EDITOR_SIZE_RANGE,
    LINE_HEIGHT_DEFAULT,
};

use crate::RustdianApp;
use crate::theme::{ActiveTheme, CONTROL_MD, RADIUS_M};
use crate::ui::ext::ToastKind;
use crate::ui::fonts::{self, Slot};
use crate::ui::widgets::controls::{
    ButtonKind, ButtonSize, button_sized, choice_chip, form_row, form_row_with_extra,
    section_title, setting_card, stepper,
};
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
    pub scroll: ScrollHandle,
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

/// The rows of the 字体与字号 block, below the theme and sidebar card of the 外观 page.
pub fn render_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    fonts::ensure_installed(|| cx.text_system().all_font_names());
    let cfg = fonts::appearance();
    let snapshot = fonts::current();
    let m = fonts::current_metrics();

    let ui_row = font_row(
        app,
        cx,
        Slot::Ui,
        &cfg.ui_font,
        &crate::t!("settings-font-default-ui"),
        &snapshot.ui_family,
        false,
    );
    let editor_row = font_row(
        app,
        cx,
        Slot::Editor,
        &cfg.editor_font,
        &crate::t!("settings-font-same-as-ui"),
        &snapshot.editor_family,
        false,
    );
    let code_row = font_row(
        app,
        cx,
        Slot::Code,
        &cfg.code_font,
        &crate::t!("settings-font-default-code"),
        &snapshot.code_family,
        true,
    );

    let family_card = setting_card(&t)
        .child(ui_row)
        .child(editor_row)
        .child(code_row);

    let sizing_card = setting_card(&t)
        .child(form_row_with_extra(
            &t,
            &crate::t!("settings-editor-font-size"),
            &crate::t!("settings-editor-font-size-desc"),
            size_control(app, cx, NumField::EditorSize, &cfg),
            preset_row(app, cx, &cfg),
            false,
        ))
        .child(form_row(
            &t,
            &crate::t!("settings-line-height"),
            &crate::t!("settings-line-height-desc"),
            number_stepper(app, cx, NumField::LineHeight, line_text(&cfg), -1, 1, false),
            false,
        ))
        .child(form_row(
            &t,
            &crate::t!("settings-code-font-size"),
            &crate::t!("settings-code-font-size-desc"),
            code_size_control(app, cx, &cfg),
            true,
        ));

    div()
        .flex()
        .flex_col()
        .mt(px(4.0))
        .child(section_title(
            &t,
            &crate::t!("settings-font-section-family"),
        ))
        .child(family_card)
        .children(missing_lines(&t, &snapshot.missing))
        .children(missing_lines(&t, &snapshot.clamp_warnings))
        .child(section_title(
            &t,
            &crate::t!("settings-font-section-sizing"),
        ))
        .child(sizing_card)
        .child(preview(
            &t,
            &snapshot.editor_family,
            &snapshot.code_family,
            &m,
        ))
        .child(reset_row(app, cx))
}

// ---------------------------------------------------------------------------
// Font families
// ---------------------------------------------------------------------------

/// One font row: the title, the trigger that opens the picker, and the inline picker when open.
fn font_row(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    slot: Slot,
    value: &str,
    default_label: &str,
    resolved: &str,
    last: bool,
) -> Div {
    let t = app.theme;
    let open = app.settings.fonts.picker.as_ref().map(|p| p.slot) == Some(slot);
    let (title, desc) = match slot {
        Slot::Ui => (
            crate::t!("settings-ui-font"),
            crate::t!("settings-ui-font-desc"),
        ),
        Slot::Editor => (
            crate::t!("settings-editor-font"),
            crate::t!("settings-editor-font-desc"),
        ),
        Slot::Code => (
            crate::t!("settings-code-font"),
            crate::t!("settings-code-font-desc"),
        ),
    };
    let shown = if value.is_empty() {
        crate::t!("settings-font-default", label = default_label)
    } else {
        value.to_string()
    };
    let mut trigger = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .w(px(240.0))
        .h(px(CONTROL_MD))
        .px(px(10.0))
        .rounded(px(RADIUS_M))
        .border_1()
        .border_color(if open { t.ac } else { t.border_default })
        .bg(t.surface)
        .hover(|s| s.bg(t.hover))
        .cursor_pointer()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .font_family(SharedString::from(resolved.to_string()))
                .text_size(px(13.0))
                .text_color(t.tx)
                .child(SharedString::from(shown)),
        )
        .child(crate::ui::icons::icon(
            "chevron-down",
            14.0,
            if open { t.ac } else { t.tx3 },
        ));
    if !open {
        trigger = trigger.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                toggle_picker(this, slot, window, cx);
                cx.notify();
            }),
        );
    }
    if open && let Some(panel) = picker_panel(app, cx, slot, value) {
        return form_row_with_extra(&t, &title, &desc, trigger, panel, last);
    }
    form_row(&t, &title, &desc, trigger, last)
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
        .overflow_y_scroll()
        .track_scroll(&picker.scroll)
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        });
    rows = rows.child(picker_row(
        &t,
        cx,
        slot,
        String::new(),
        crate::t!("settings-font-system-default"),
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
        crate::t!("settings-font-searching")
    } else if matches.is_empty() {
        crate::t!("settings-font-no-matches")
    } else if matches.len() > PICKER_MAX_ROWS {
        crate::t!(
            "settings-font-more",
            count = matches.len() - PICKER_MAX_ROWS
        )
    } else {
        crate::t!("settings-font-count", count = matches.len())
    };

    let search = div()
        .flex()
        .items_center()
        .px(px(6.0))
        .py(px(6.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(picker.input.clone());

    Some(
        div()
            .ml_auto()
            .w(px(260.0))
            .rounded(px(RADIUS_M))
            .border_1()
            .border_color(t.border_default)
            .bg(t.surface)
            .shadow_md()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.settings.fonts.picker = None;
                cx.notify();
            }))
            .child(search)
            .child(rows)
            .child(
                div()
                    .px(px(12.0))
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
    let mut right = div().flex().flex_none().items_center().gap(px(6.0));
    if let Some(f) = &family {
        if fonts::is_cjk_font(f) {
            right = right.child(tag(t, &crate::t!("settings-font-tag-cjk")));
        }
        if fonts::is_mono_font(f) {
            right = right.child(tag(t, &crate::t!("settings-font-tag-mono")));
        }
    }
    if selected {
        right = right.child(crate::ui::icons::icon("check", 13.0, t.ac));
    }
    let mut row = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .h(px(32.0))
        .mx(px(6.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .child(name)
        .child(right);
    if selected {
        row = row.bg(t.active);
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
    let input = cx.new(|cx| LineInput::new(cx, crate::t!("settings-font-search-placeholder")));
    let theme = app.theme;
    input.update(cx, |i, cx| i.set_theme(theme, cx));
    let sub = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            match ev {
                LineInputEvent::Changed(_) => {
                    if let Some(picker) = this.settings.fonts.picker.as_ref() {
                        picker.scroll.scroll_to_item(0);
                    }
                }
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
    let scroll = ScrollHandle::default();
    app.settings.fonts.picker = Some(Picker {
        slot,
        input,
        scroll,
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
        false,
    )
}

fn code_size_control(app: &RustdianApp, cx: &mut Context<RustdianApp>, cfg: &Appearance) -> Div {
    let t = app.theme;
    let following = cfg.code_font_size == CODE_SIZE_FOLLOW;
    let display_size = if following {
        fonts::current().code_size
    } else {
        cfg.code_font_size
    };

    let follow_chip = choice_chip(
        &t,
        &crate::t!("settings-font-follow-toggle"),
        following,
        cx.listener(move |this, _, _, cx| {
            let resolved = fonts::current().code_size;
            edit(this, cx, |a| {
                if a.code_font_size == CODE_SIZE_FOLLOW {
                    a.code_font_size = fonts::step_float(resolved, 0.0, fonts::STEP_CODE);
                } else {
                    a.code_font_size = CODE_SIZE_FOLLOW;
                }
            });
            cx.notify();
        }),
    );

    let step = number_stepper(
        app,
        cx,
        NumField::CodeSize,
        format!("{} px", fmt_num(display_size)),
        -1,
        1,
        following,
    );

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .child(follow_chip)
        .child(step)
}

/// Integrated stepper: `− | value | +`. The value opens a compact inline input on click.
fn number_stepper(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    field: NumField,
    label: String,
    down: i32,
    up: i32,
    muted: bool,
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
            .w(px(72.0))
            .h_full()
            .px(px(4.0))
            .child(n.input.clone())
            .into_any_element(),
        None => div()
            .flex()
            .items_center()
            .justify_center()
            .min_w(px(64.0))
            .h_full()
            .px(px(10.0))
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(if muted { t.tx2 } else { t.tx })
            .hover(|s| s.bg(t.hover))
            .whitespace_nowrap()
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
    stepper(
        &t,
        value,
        cx.listener(move |this, _, _, cx| {
            step(this, field, down, cx);
            cx.notify();
        }),
        cx.listener(move |this, _, _, cx| {
            step(this, field, up, cx);
            cx.notify();
        }),
    )
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
        NumField::CodeSize => {
            if cfg.code_font_size == CODE_SIZE_FOLLOW {
                fmt_num(fonts::current().code_size)
            } else {
                fmt_num(cfg.code_font_size)
            }
        }
        NumField::LineHeight => format!("{:.2}", cfg.line_height),
    };
    app.settings.fonts.picker = None;
    let input = cx.new(|cx| LineInput::new(cx, "数值"));
    let theme = app.theme;
    input.update(cx, |i, cx| {
        i.set_theme(theme, cx);
        i.set_borderless(true, cx);
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

/// The quick presets of the body size, rendered inside the same row container.
fn preset_row(app: &RustdianApp, cx: &mut Context<RustdianApp>, cfg: &Appearance) -> Div {
    let t = app.theme;
    let mut row = div().flex().flex_wrap().gap(px(6.0)).justify_end();
    for &p in fonts::EDITOR_PRESETS.iter() {
        let on = fonts::snap_preset(cfg.editor_font_size, &[p]).is_some();
        row = row.child(choice_chip(
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

// ---------------------------------------------------------------------------
// Preview, warnings and resets
// ---------------------------------------------------------------------------

fn missing_lines(t: &ActiveTheme, messages: &[String]) -> Vec<Div> {
    messages
        .iter()
        .map(|m| {
            div()
                .pt(px(4.0))
                .text_size(px(12.0))
                .text_color(t.danger)
                .child(SharedString::from(m.clone()))
        })
        .collect()
}

/// A short sample in the chosen body and code faces and sizes.
fn preview(t: &ActiveTheme, body: &str, code: &str, m: &fonts::Metrics) -> Div {
    let h2 = m.heading[1];
    setting_card(t)
        .mt(px(14.0))
        .py(px(14.0))
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx3)
                .child(SharedString::from(crate::t!("settings-font-preview-title"))),
        )
        .child(
            div()
                .font_family(SharedString::from(body.to_string()))
                .text_size(px(h2))
                .line_height(px(m.heading_lh[1]))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.tx)
                .child(SharedString::from(crate::t!(
                    "settings-font-preview-heading"
                ))),
        )
        .child(
            div()
                .font_family(SharedString::from(body.to_string()))
                .text_size(px(m.font))
                .line_height(px(m.line))
                .text_color(t.tx)
                .child(SharedString::from(crate::t!("settings-font-preview-body"))),
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
        .items_center()
        .justify_end()
        .gap(px(8.0))
        .pt(px(12.0))
        .child(button_sized(
            &t,
            &crate::t!("settings-font-reset-family"),
            ButtonKind::Secondary,
            ButtonSize::Small,
            Some("refresh"),
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
        .child(button_sized(
            &t,
            &crate::t!("settings-font-reset-size"),
            ButtonKind::Secondary,
            ButtonSize::Small,
            Some("refresh"),
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
