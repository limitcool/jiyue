//! The in-note find bar (Ctrl+F) and replace bar (Ctrl+H) for the active note's editor.
//!
//! The bar floats at the top right of the document area: only the card takes pointer events, so
//! the rest of the window keeps working. It works on the Source view: opening it switches the
//! note to Source. Matches are painted by the editor (`set_search_matches`); replacements go
//! through `replace_text_range`, one undo step each.

use std::ops::Range;

use gpui::{
    AnyElement, Context, Entity, Focusable, KeyDownEvent, Subscription, div, prelude::*, px,
};

use super::focus_soon;
use super::logic::{self, MatchOpts};
use crate::RustdianApp;
use crate::session::ViewMode;
use crate::ui::ext::ToastKind;
use crate::ui::text_input::TextEditor;
use crate::ui::widgets::{ButtonKind, LineInput, LineInputEvent, controls};

const FIND_PLACEHOLDER: &str = "查找";
const REPLACE_PLACEHOLDER: &str = "替换为";
/// Width of the bar, and the gap to the window (and the right panel when open).
const BAR_WIDTH: f32 = 420.0;
const BAR_TOP: f32 = 92.0;
const BAR_RIGHT: f32 = 24.0;
const RIGHT_PANEL_W: f32 = 272.0;

/// State of the find bar (owned by `search_ui::State`).
#[derive(Default)]
pub struct FindState {
    pub open: bool,
    /// The replace row is shown (Ctrl+H, or the chevron).
    show_replace: bool,
    find_input: Option<Entity<LineInput>>,
    replace_input: Option<Entity<LineInput>>,
    _subs: Vec<Subscription>,
    case_sensitive: bool,
    whole_word: bool,
    /// Matches in the note the bar is on (byte ranges), and the current one.
    matches: Vec<Range<usize>>,
    current: usize,
    /// Path of the note the matches belong to.
    path: Option<String>,
}

impl FindState {
    fn options(&self) -> MatchOpts {
        MatchOpts {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
        }
    }
}

/// The editor of the active note.
fn active_editor(app: &RustdianApp) -> Option<Entity<TextEditor>> {
    app.active_path().and_then(|p| app.editor_for(p))
}

/// Text of a field.
fn field_text(input: &Option<Entity<LineInput>>, cx: &gpui::App) -> String {
    input
        .as_ref()
        .map(|i| i.read(cx).text())
        .unwrap_or_default()
}

/// Creates the two fields once.
fn ensure_inputs(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.ext.search.find.find_input.is_some() {
        return;
    }
    let find = cx.new(|cx| LineInput::new(cx, crate::t!("find-placeholder")));
    let replace = cx.new(|cx| LineInput::new(cx, crate::t!("find-replace-placeholder")));
    // Enter and Escape are bound by the fields, so they arrive as events: Enter in the find field
    // goes to the next match, Enter in the replace field replaces the current one.
    let find_sub = cx.subscribe(
        &find,
        |app: &mut RustdianApp, _input, ev: &LineInputEvent, cx| match ev {
            LineInputEvent::Changed(_) => {
                refresh(app, cx);
                reveal_current(app, cx);
            }
            LineInputEvent::Submit(_) => navigate(app, true, cx),
            LineInputEvent::Cancel => close(app, cx),
        },
    );
    let replace_sub = cx.subscribe(
        &replace,
        |app: &mut RustdianApp, _input, ev: &LineInputEvent, cx| match ev {
            LineInputEvent::Submit(_) => replace_current(app, cx),
            LineInputEvent::Cancel => close(app, cx),
            LineInputEvent::Changed(_) => {}
        },
    );
    let st = &mut app.ext.search.find;
    st._subs.push(find_sub);
    st._subs.push(replace_sub);
    st.find_input = Some(find);
    st.replace_input = Some(replace);
}

/// Opens the bar (`replace` also shows the replace row). The Source view is needed, so a note
/// shown rendered switches to it. The selection of one line fills the find field.
pub fn open(app: &mut RustdianApp, replace: bool, cx: &mut Context<RustdianApp>) {
    ensure_inputs(app, cx);
    if app.view_mode != ViewMode::Source {
        app.set_view_mode(ViewMode::Source);
    }
    let theme = app.theme;
    let was_open = app.ext.search.find.open;
    app.ext.search.find.open = true;
    if replace {
        app.ext.search.find.show_replace = true;
    }
    let find = app.ext.search.find.find_input.clone();
    let replace_input = app.ext.search.find.replace_input.clone();
    for input in [find.clone(), replace_input].into_iter().flatten() {
        input.update(cx, |input, cx| input.set_theme(theme, cx));
    }
    if !was_open {
        let path = app.active_path().map(str::to_string);
        app.ext.search.find.path = path;
        let selected = active_editor(app).and_then(|editor| {
            let ed = editor.read(cx);
            let (anchor, head) = ed.selection_offsets();
            let (lo, hi) = (anchor.min(head), anchor.max(head));
            let text = ed.text();
            let picked = text.get(lo..hi).unwrap_or("");
            (!picked.is_empty() && !picked.contains('\n')).then(|| picked.to_string())
        });
        if let (Some(picked), Some(input)) = (selected, find.clone()) {
            input.update(cx, |input, cx| input.set_text(&picked, cx));
        }
    }
    refresh(app, cx);
    reveal_current(app, cx);
    if let Some(input) = find {
        let handle = input.read(cx).focus_handle(cx);
        focus_soon(cx, handle);
    }
    cx.notify();
}

/// Closes the bar: the highlights go, and the editor gets the keyboard back.
pub fn close(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let st = &mut app.ext.search.find;
    st.open = false;
    st.matches.clear();
    st.current = 0;
    let path = st.path.take();
    if let Some(editor) = path.and_then(|p| app.editor_for(&p)) {
        editor.update(cx, |ed, cx| ed.set_search_matches(Vec::new(), None, cx));
        let handle = editor.read(cx).focus_handle(cx);
        focus_soon(cx, handle);
    }
    cx.notify();
}

/// Finds the matches of the find field in the active note. The current one is the first match at
/// or after the caret (or the first one). The editor is repainted with them.
pub fn refresh(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if !app.ext.search.find.open {
        return;
    }
    let Some(editor) = active_editor(app) else {
        app.ext.search.find.matches.clear();
        return;
    };
    let needle = field_text(&app.ext.search.find.find_input, cx);
    let opts = app.ext.search.find.options();
    let (text, caret) = {
        let ed = editor.read(cx);
        (ed.text(), ed.selection_offsets().1)
    };
    let matches = logic::find_matches(&text, &needle, opts);
    let current = if matches.is_empty() {
        0
    } else {
        matches.iter().position(|m| m.start >= caret).unwrap_or(0)
    };
    let path = app.active_path().map(str::to_string);
    let st = &mut app.ext.search.find;
    st.path = path;
    st.matches = matches.clone();
    st.current = current;
    let shown = (!matches.is_empty()).then_some(current);
    editor.update(cx, |ed, cx| ed.set_search_matches(matches, shown, cx));
    cx.notify();
}

/// Moves the editor's selection to the current match and scrolls to it.
fn reveal_current(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let st = &app.ext.search.find;
    let Some(range) = st.matches.get(st.current).cloned() else {
        return;
    };
    let matches = st.matches.clone();
    let current = st.current;
    let Some(editor) = active_editor(app) else {
        return;
    };
    editor.update(cx, |ed, cx| {
        ed.set_search_matches(matches, Some(current), cx);
        ed.reveal_range(range, cx);
    });
}

/// Next (`forward`) or previous match, wrapping at the ends.
pub fn navigate(app: &mut RustdianApp, forward: bool, cx: &mut Context<RustdianApp>) {
    let st = &mut app.ext.search.find;
    let n = st.matches.len();
    if n == 0 {
        return;
    }
    st.current = if forward {
        (st.current + 1) % n
    } else {
        (st.current + n - 1) % n
    };
    reveal_current(app, cx);
    cx.notify();
}

/// Replaces the current match with the replace field's text (one undo step), then moves on to the
/// next match.
fn replace_current(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let replacement = field_text(&app.ext.search.find.replace_input, cx);
    let Some(range) = app
        .ext
        .search
        .find
        .matches
        .get(app.ext.search.find.current)
        .cloned()
    else {
        return;
    };
    let Some(editor) = active_editor(app) else {
        return;
    };
    editor.update(cx, |ed, cx| {
        ed.replace_text_range(range, &replacement, cx);
    });
    // The caret now follows the new text, so the next match after it becomes current.
    refresh(app, cx);
}

/// Replaces every match with the replace field's text. One undo step.
fn replace_all(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let replacement = field_text(&app.ext.search.find.replace_input, cx);
    let needle = field_text(&app.ext.search.find.find_input, cx);
    let opts = app.ext.search.find.options();
    let Some(editor) = active_editor(app) else {
        return;
    };
    let text = editor.read(cx).text();
    let matches = logic::find_matches(&text, &needle, opts);
    let Some((start, end, new_text)) = logic::replace_all_edit(&text, &matches, &replacement)
    else {
        app.toast(ToastKind::Info, crate::t!("find-no-match-to-replace"));
        return;
    };
    let count = matches.len();
    editor.update(cx, |ed, cx| {
        ed.replace_text_range(start..end, &new_text, cx);
    });
    refresh(app, cx);
    app.toast(
        ToastKind::Success,
        crate::t!("find-replaced-count", count = count),
    );
}

/// The active note's text changed: refresh the matches of the open bar.
pub fn on_note_changed(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    if app.ext.search.find.open && app.active_path() == Some(path) {
        refresh(app, cx);
    }
}

/// Another note became active: its matches replace the old note's.
pub fn on_note_opened(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    if !app.ext.search.find.open {
        return;
    }
    if let Some(old) = app.ext.search.find.path.clone()
        && old != path
    {
        if let Some(editor) = app.editor_for(&old) {
            editor.update(cx, |ed, cx| ed.set_search_matches(Vec::new(), None, cx));
        }
        app.ext.search.find.matches.clear();
    }
    // The editor of the new note is created later in this render, so the refresh runs after it.
    cx.spawn(async move |this, cx| {
        let _ = this.update(cx, |app, cx| {
            if app.ext.search.find.open {
                refresh(app, cx);
            }
        });
    })
    .detach();
}

/// Keys while the bar is open. Returns true when the key was used.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    let m = ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();
    match key {
        "escape" => {
            close(app, cx);
            true
        }
        // Plain Enter is the field's Submit event; only Shift+Enter reaches this handler.
        "enter" if m.shift && !m.control && !m.alt && !m.platform => {
            navigate(app, false, cx);
            true
        }
        "f3" if !m.control && !m.alt && !m.platform => {
            navigate(app, !m.shift, cx);
            true
        }
        _ => false,
    }
}

fn toggle_option(app: &mut RustdianApp, which: Option<bool>, cx: &mut Context<RustdianApp>) {
    match which {
        Some(true) => app.ext.search.find.case_sensitive = !app.ext.search.find.case_sensitive,
        Some(false) => app.ext.search.find.whole_word = !app.ext.search.find.whole_word,
        None => app.ext.search.find.show_replace = !app.ext.search.find.show_replace,
    }
    refresh(app, cx);
    reveal_current(app, cx);
}

/// A switch with its label.
fn option_item(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    label: &str,
    on: bool,
    which: bool,
) -> AnyElement {
    let t = app.theme;
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(12.0))
        .text_color(t.tx3)
        .child(controls::toggle(
            &t,
            on,
            cx.listener(move |this, _, _, cx| toggle_option(this, Some(which), cx)),
        ))
        .child(label.to_string())
        .into_any_element()
}

/// Renders the bar. Only the card has a box, so it is the only part that takes pointer events.
pub fn render(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let st = &app.ext.search.find;
    let (Some(find), Some(replace)) = (st.find_input.clone(), st.replace_input.clone()) else {
        return div().into_any_element();
    };
    let typed = !field_text(&st.find_input, cx).is_empty();
    let count = if st.matches.is_empty() {
        if typed {
            crate::t!("find-no-results")
        } else {
            String::new()
        }
    } else {
        format!("{} / {}", st.current + 1, st.matches.len())
    };

    let row_find = div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(controls::icon_button(
            &t,
            "chevron-down",
            cx.listener(|this, _, _, cx| toggle_option(this, None, cx)),
        ))
        .child(div().flex_1().min_w_0().child(find))
        .child(
            div()
                .flex_none()
                .min_w(px(40.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child(count),
        )
        .child(controls::icon_button(
            &t,
            "arrow-up",
            cx.listener(|this, _, _, cx| navigate(this, false, cx)),
        ))
        .child(controls::icon_button(
            &t,
            "arrow-down",
            cx.listener(|this, _, _, cx| navigate(this, true, cx)),
        ))
        .child(controls::icon_button(
            &t,
            "x",
            cx.listener(|this, _, _, cx| close(this, cx)),
        ));

    let mut card = div()
        .occlude()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .w(px(BAR_WIDTH))
        .p(px(8.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .rounded(px(12.0))
        .shadow_lg()
        .text_color(t.tx)
        .child(row_find);

    if st.show_replace {
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(div().flex_1().min_w_0().child(replace))
                .child(controls::button(
                    &t,
                    &crate::t!("find-replace"),
                    ButtonKind::Secondary,
                    None,
                    cx.listener(|this, _, _, cx| replace_current(this, cx)),
                ))
                .child(controls::button(
                    &t,
                    &crate::t!("find-replace-all"),
                    ButtonKind::Secondary,
                    None,
                    cx.listener(|this, _, _, cx| replace_all(this, cx)),
                )),
        );
    }

    card = card.child(
        div()
            .flex()
            .items_center()
            .gap(px(14.0))
            .px(px(2.0))
            .child(option_item(
                app,
                cx,
                &crate::t!("find-case-sensitive"),
                st.case_sensitive,
                true,
            ))
            .child(option_item(
                app,
                cx,
                &crate::t!("find-whole-word"),
                st.whole_word,
                false,
            ))
            .child(
                div()
                    .ml_auto()
                    .text_size(px(11.0))
                    .text_color(t.tx3)
                    .whitespace_nowrap()
                    .child(gpui::SharedString::from(crate::t!("find-shortcut-hint"))),
            ),
    );

    let right = if app.right_sidebar_open {
        BAR_RIGHT + RIGHT_PANEL_W
    } else {
        BAR_RIGHT
    };
    div()
        .absolute()
        .top(px(BAR_TOP))
        .right(px(right))
        .child(card)
        .into_any_element()
}
