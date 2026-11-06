//! 快捷键 settings page: a filter field, the registry grouped by category, key chips, a capture
//! mode for changing a chord, a conflict hint with 替换 / 取消, per-row and whole-page reset, and a
//! read-only list of the text-editing keys. The pure rules live in `ui/keymap.rs`.

use gpui::{
    AnyElement, Context, Div, Entity, IntoElement, KeyDownEvent, MouseDownEvent, SharedString,
    Subscription, Window, div, prelude::*, px,
};

use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::keymap::{self, Category, Chord, Conflict, Keymap, RESERVED, SPECS, Spec};
use crate::ui::widgets::{
    ButtonKind, LineInput, LineInputEvent, button, confirm_dialog, kbd, section_title,
};

/// A chord that is already used: waits for 替换 or 取消.
#[derive(Clone)]
pub struct Pending {
    pub id: &'static str,
    pub chord: Chord,
    pub conflicts: Vec<Conflict>,
}

/// State of the page. Lives in `RustdianApp::keymap_ui`.
#[derive(Default)]
pub struct State {
    /// The filter field (IME-capable), created by `install`.
    pub filter: Option<Entity<LineInput>>,
    /// Subscriptions of `filter`, kept alive here.
    pub subs: Vec<Subscription>,
    /// Text of the filter field.
    pub query: String,
    /// The command whose chord is being captured.
    pub capturing: Option<&'static str>,
    /// A chord that clashes with another command.
    pub pending: Option<Pending>,
    /// A short note under one row (an invalid key, a warning about Ctrl+Alt+letter).
    pub notice: Option<(&'static str, String)>,
    /// The 全部恢复默认 confirmation is open.
    pub confirm_reset_all: bool,
}

/// Creates the filter field and subscribes to it. Called once from `RustdianApp::new`.
pub fn install(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let input = cx.new(|cx| LineInput::new(cx, "搜索快捷键…"));
    let sub = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            if let LineInputEvent::Changed(text) = ev {
                this.keymap_ui.query = text.clone();
            }
            cx.notify();
        },
    );
    app.keymap_ui.filter = Some(input);
    app.keymap_ui.subs.push(sub);
}

/// Starts capture mode for one command.
fn begin_capture(app: &mut RustdianApp, id: &'static str) {
    app.keymap_ui.capturing = Some(id);
    app.keymap_ui.pending = None;
    app.keymap_ui.notice = None;
}

/// Keys that only change the modifier state: ignored while capturing.
fn is_modifier_key(key: &str) -> bool {
    matches!(
        key,
        "shift" | "control" | "alt" | "platform" | "function" | "capslock"
    )
}

/// Writes `chords` for one command: updates the live keymap, the file and the GPUI bindings.
fn set_chords(app: &mut RustdianApp, id: &str, chords: Vec<Chord>, cx: &mut Context<RustdianApp>) {
    let mut km = keymap::current();
    km.set(id, chords);
    keymap::apply(cx, km);
    app.keymap_ui.notice = None;
}

/// Keyboard hook. While a chord is being captured, or the reset confirmation is open, every key is
/// used here. Returns true when the key was used.
pub fn capture_key(
    app: &mut RustdianApp,
    ev: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) -> bool {
    let confirm_open = app.keymap_ui.confirm_reset_all && app.settings.open;
    let capturing = app.keymap_ui.capturing;
    if !confirm_open && capturing.is_none() {
        return false;
    }
    // Keep the keystroke away from the filter field and the editor below the dialog.
    window.prevent_default();
    cx.stop_propagation();
    let ks = &ev.keystroke;
    let key = ks.key.as_str();
    if confirm_open {
        if key == "escape" {
            app.keymap_ui.confirm_reset_all = false;
            cx.notify();
        }
        return true;
    }
    let Some(id) = capturing else {
        return true;
    };
    if is_modifier_key(key) {
        return true;
    }
    let bare =
        !(ks.modifiers.control || ks.modifiers.alt || ks.modifiers.shift || ks.modifiers.platform);
    if key == "escape" && bare {
        app.keymap_ui.capturing = None;
        app.keymap_ui.notice = None;
        cx.notify();
        return true;
    }
    if key == "backspace" && bare {
        app.keymap_ui.capturing = None;
        set_chords(app, id, Vec::new(), cx);
        cx.notify();
        return true;
    }
    let chord = Chord::from_keystroke(ks);
    let Some(when) = keymap::spec(id).map(|s| s.when) else {
        return true;
    };
    if let Some(why) = keymap::validate(when, &chord) {
        app.keymap_ui.notice = Some((id, why.to_string()));
        cx.notify();
        return true;
    }
    app.keymap_ui.capturing = None;
    let conflicts = keymap::current().conflicts(id, &chord);
    if conflicts.is_empty() {
        set_chords(app, id, vec![chord.clone()], cx);
        if chord.altgr_risk() {
            app.keymap_ui.notice = Some((
                id,
                "Ctrl+Alt+字母在部分键盘布局上会被当作 AltGr，可能没有反应".to_string(),
            ));
        }
    } else {
        app.keymap_ui.pending = Some(Pending {
            id,
            chord,
            conflicts,
        });
    }
    cx.notify();
    true
}

/// 替换: unbinds the chord from the clashing commands, then binds it to the pending command.
fn resolve_replace(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(pending) = app.keymap_ui.pending.take() else {
        return;
    };
    let mut km = keymap::current();
    for c in &pending.conflicts {
        if c.id.is_empty() {
            continue;
        }
        let mut chords = km.chords(&c.id);
        chords.retain(|x| *x != pending.chord);
        km.set(&c.id, chords);
    }
    km.set(pending.id, vec![pending.chord]);
    keymap::apply(cx, km);
    cx.notify();
}

/// Restores the default chords of one command.
fn reset_row(app: &mut RustdianApp, id: &str, cx: &mut Context<RustdianApp>) {
    let mut km = keymap::current();
    km.reset(id);
    keymap::apply(cx, km);
    app.keymap_ui.notice = None;
    app.keymap_ui.pending = None;
}

/// Restores every default (after the confirmation).
fn reset_all(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let mut km = Keymap::default();
    km.reset_all();
    keymap::apply(cx, km);
    app.keymap_ui.confirm_reset_all = false;
    app.keymap_ui.pending = None;
    app.keymap_ui.notice = None;
    app.keymap_ui.capturing = None;
}

/// The reset confirmation, drawn over the whole window by `main.rs`.
pub fn render_confirm(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    confirm_dialog(
        &t,
        "全部恢复默认",
        "所有快捷键将恢复为默认值，自定义的设置会被删除。",
        "恢复默认",
        true,
        cx.listener(|this, _: &MouseDownEvent, _, cx| {
            reset_all(this, cx);
            cx.notify();
        }),
        cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.keymap_ui.confirm_reset_all = false;
            cx.notify();
        }),
    )
}

/// The 快捷键 category body.
pub fn render_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let st = &app.keymap_ui;
    let km = keymap::current();

    let field = match &st.filter {
        Some(input) => {
            input.update(cx, |i, cx| i.set_theme(t, cx));
            input.clone().into_any_element()
        }
        None => div().into_any_element(),
    };
    let top = div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .mt(px(10.0))
        .child(div().flex_1().min_w_0().child(field))
        .child(button(
            &t,
            "全部恢复默认",
            ButtonKind::Secondary,
            None,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.keymap_ui.confirm_reset_all = true;
                cx.notify();
            }),
        ));

    let q = st.query.trim().to_lowercase();
    let mut body = div().flex().flex_col().gap(px(2.0)).child(top);
    let mut shown = 0usize;
    for cat in Category::ALL {
        let rows: Vec<&'static Spec> = SPECS
            .iter()
            .filter(|s| s.category == cat)
            .filter(|s| {
                q.is_empty()
                    || s.label.to_lowercase().contains(&q)
                    || km
                        .chords(s.id)
                        .iter()
                        .any(|c| c.format().to_lowercase().contains(&q))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        shown += rows.len();
        body = body.child(section_title(&t, cat.label()));
        let last = rows.len() - 1;
        for (i, s) in rows.iter().enumerate() {
            body = body.child(row(app, cx, s, &km, i == last));
        }
    }
    if shown == 0 {
        body = body.child(
            div()
                .py(px(18.0))
                .text_size(px(13.0))
                .text_color(t.tx3)
                .child("没有匹配的命令"),
        );
    }

    body = body
        .child(section_title(&t, "文字编辑键（不可修改）"))
        .child(
            div()
                .pb(px(6.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child("这些键用于编辑文字，不能分配给命令；设置新快捷键时会提示冲突。"),
        );
    for (i, (keys, meaning)) in RESERVED.iter().enumerate() {
        let shown_keys = Chord::parse(keys).map(|c| c.format()).unwrap_or_default();
        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(30.0))
                .when(i + 1 < RESERVED.len(), |d| {
                    d.border_b_1().border_color(t.line_soft)
                })
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(t.tx2)
                        .child(SharedString::from(meaning.to_string())),
                )
                .child(kbd(&t, &shown_keys)),
        );
    }
    body.into_any_element()
}

/// One command row: label, chips (or the capture prompt), 修改 and 恢复默认, plus the hints.
fn row(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    s: &'static Spec,
    km: &Keymap,
    last: bool,
) -> Div {
    let t = app.theme;
    let st = &app.keymap_ui;
    let capturing = st.capturing == Some(s.id);

    let mut right = div().flex().items_center().gap(px(6.0)).flex_none();
    if capturing {
        right = right.child(
            div()
                .text_size(px(12.5))
                .text_color(t.ac)
                .child("按下新的快捷键…"),
        );
    } else {
        let chords = km.chords(s.id);
        if chords.is_empty() {
            right = right.child(div().text_size(px(12.5)).text_color(t.tx3).child("未设置"));
        }
        for c in &chords {
            right = right.child(kbd(&t, &c.format()));
        }
    }
    right = right.child(button(
        &t,
        if capturing { "取消" } else { "修改" },
        ButtonKind::Secondary,
        Some("pencil"),
        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            if this.keymap_ui.capturing == Some(s.id) {
                this.keymap_ui.capturing = None;
            } else {
                begin_capture(this, s.id);
            }
            cx.notify();
        }),
    ));
    if !km.is_default(s.id) {
        right = right.child(button(
            &t,
            "恢复默认",
            ButtonKind::Secondary,
            None,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                reset_row(this, s.id, cx);
                cx.notify();
            }),
        ));
    }

    let mut block = div().flex().flex_col().gap(px(4.0)).py(px(7.0)).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .min_w_0()
                    .text_size(px(13.5))
                    .text_color(t.tx)
                    .child(SharedString::from(s.label.to_string())),
            )
            .child(right),
    );
    if capturing {
        block = block.child(hint_text(&t, "Esc 取消 · Backspace 清除为未设置", t.tx3));
    }
    if let Some((id, text)) = &st.notice
        && *id == s.id
    {
        block = block.child(hint_text(&t, text, t.st));
    }
    if let Some(pending) = &st.pending
        && pending.id == s.id
    {
        block = block.child(conflict_hint(app, cx, pending));
    }
    if !last {
        block = block.border_b_1().border_color(t.line_soft);
    }
    block
}

fn hint_text(_t: &ActiveTheme, text: &str, color: gpui::Rgba) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(color)
        .child(SharedString::from(text.to_string()))
}

/// Red hint under a row whose chord is used by another command, with 替换 and 取消.
fn conflict_hint(app: &RustdianApp, cx: &mut Context<RustdianApp>, pending: &Pending) -> Div {
    let t = app.theme;
    let names: Vec<String> = pending.conflicts.iter().map(|c| c.label.clone()).collect();
    let replaceable = pending.conflicts.iter().all(|c| !c.id.is_empty());
    let text = format!(
        "{} 已被「{}」使用",
        pending.chord.format(),
        names.join("」「")
    );
    let mut actions = div().flex().items_center().gap(px(6.0));
    if replaceable {
        actions = actions.child(button(
            &t,
            "替换",
            ButtonKind::Danger,
            None,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                resolve_replace(this, cx);
            }),
        ));
    }
    actions = actions.child(button(
        &t,
        "取消",
        ButtonKind::Secondary,
        None,
        cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.keymap_ui.pending = None;
            cx.notify();
        }),
    ));
    let mut hint = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .py(px(4.0))
        .child(hint_text(&t, &text, gpui::rgb(0xE5484D)))
        .child(actions);
    if !replaceable {
        hint = hint.child(hint_text(&t, "文字编辑键不能被替换", t.tx3));
    }
    hint
}
