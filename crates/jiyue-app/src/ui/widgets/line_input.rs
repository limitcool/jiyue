//! `LineInput`: a reusable single-line text field with full IME support.
//!
//! The entity owns a `TextState` (see `text_edit.rs`) and paints one shaped line. The IME
//! handler (`EntityInputHandler`) is registered in paint, so only the focused input receives
//! composition and committed text. Key bindings live in context `LineInput` (`bind_keys`).
//!
//! Focus and overlays: the input is focusable on its own. An overlay that owns an input must
//! call `LineInput::focus` when it opens (in a later frame, like `RustdianApp::want_focus`).
//! While an input is focused, typed text goes to it and not to the document editor. Enter and
//! Escape are reported as `LineInputEvent::Submit` and `Cancel` and are not passed on.

use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, ShapedLine, SharedString,
    Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, font, point,
    prelude::*, px, relative, size,
};

use super::text_edit::{self, EditKind, TextState};
use crate::theme::ActiveTheme;
use crate::ui::UI_FONT;
use crate::ui::keymap::primary;
use crate::ui::text_input::{
    range_utf8_to_utf16, range_utf16_to_utf8, utf8_to_utf16, utf16_to_utf8,
};

/// Key context of the input (see `bind_keys`).
pub const CONTEXT: &str = "LineInput";
const HEIGHT: f32 = 34.0;
const FONT_SIZE: f32 = 13.0;
const LINE_H: f32 = 18.0;
const CARET_WIDTH: f32 = 1.5;
const CARET_BLINK_MS: u64 = 500;

actions!(
    line_input,
    [
        Left,
        Right,
        SelectLeft,
        SelectRight,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        Home,
        End,
        SelectHome,
        SelectEnd,
        Backspace,
        Delete,
        DeleteWordBackward,
        DeleteWordForward,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Submit,
        Cancel,
    ]
);

/// Registers the input key bindings (context `LineInput`). Call once at app start, next to
/// `ui::text_input::bind_keys`.
pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("ctrl-left", WordLeft, c),
        KeyBinding::new("ctrl-right", WordRight, c),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, c),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("ctrl-backspace", DeleteWordBackward, c),
        KeyBinding::new("ctrl-delete", DeleteWordForward, c),
        KeyBinding::new(&primary("ctrl-a"), SelectAll, c),
        KeyBinding::new(&primary("ctrl-c"), Copy, c),
        KeyBinding::new(&primary("ctrl-x"), Cut, c),
        KeyBinding::new(&primary("ctrl-v"), Paste, c),
        KeyBinding::new(&primary("ctrl-z"), Undo, c),
        KeyBinding::new(&primary("ctrl-y"), Redo, c),
        KeyBinding::new(&primary("ctrl-shift-z"), Redo, c),
        KeyBinding::new("enter", Submit, c),
        KeyBinding::new("escape", Cancel, c),
    ]);
}

/// Events an owner can subscribe to with `cx.subscribe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineInputEvent {
    /// The text changed (typing, paste, undo, IME commit, `set_text` does not emit).
    Changed(String),
    /// Enter was pressed.
    Submit(String),
    /// Escape was pressed.
    Cancel,
}

/// Geometry of the last paint, read by mouse hit testing and the IME handler.
struct PaintLayout {
    line: ShapedLine,
    /// Window x of display x = 0 (already shifted by the horizontal scroll).
    origin_x: f32,
    /// Window y of the top of the text row.
    origin_y: f32,
}

/// The entity. Create with `cx.new(|cx| LineInput::new(cx, "placeholder"))`.
pub struct LineInput {
    focus_handle: FocusHandle,
    placeholder: SharedString,
    state: TextState,
    theme: ActiveTheme,
    password: bool,
    disabled: bool,
    /// Horizontal scroll in pixels, kept so the caret stays visible.
    scroll_x: f32,
    /// IME marked range in UTF-8 bytes of the text.
    marked: Option<Range<usize>>,
    selecting: bool,
    caret_on: bool,
    layout: Option<PaintLayout>,
}

impl EventEmitter<LineInputEvent> for LineInput {}

impl Focusable for LineInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl LineInput {
    /// A new empty input in the dark theme (call `set_theme` to match the app). Starts the caret
    /// blink timer.
    pub fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(CARET_BLINK_MS))
                    .await;
                let alive = this.update(cx, |input, cx| {
                    input.caret_on = !input.caret_on;
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            focus_handle: cx.focus_handle(),
            placeholder: placeholder.into(),
            state: TextState::new(""),
            theme: ActiveTheme::dark(),
            password: false,
            disabled: false,
            scroll_x: 0.0,
            marked: None,
            selecting: false,
            caret_on: true,
            layout: None,
        }
    }

    /// The current text.
    pub fn text(&self) -> String {
        self.state.text().to_string()
    }

    /// Replaces the text, clears the undo history and puts the caret at the end. Emits nothing.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.marked = None;
        self.state.set_text(text);
        self.scroll_x = 0.0;
        self.touch();
        cx.notify();
    }

    /// Password mode: every character is drawn as a bullet and copy and cut are refused.
    pub fn set_password(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.password != on {
            self.password = on;
            cx.notify();
        }
    }

    /// Longest text in characters (`None` for no limit). Longer inserts are cut short.
    pub fn set_max_len(&mut self, max: Option<usize>) {
        self.state.set_max_len(max);
    }

    /// A disabled input shows its text greyed out and ignores keys and mouse.
    pub fn set_disabled(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.disabled != on {
            self.disabled = on;
            cx.notify();
        }
    }

    pub fn set_placeholder(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = text.into();
        cx.notify();
    }

    /// Colours follow this theme. Call it from the owner's render when the theme changes.
    pub fn set_theme(&mut self, theme: ActiveTheme, cx: &mut Context<Self>) {
        if self.theme.kind != theme.kind {
            self.theme = theme;
            cx.notify();
        }
    }

    /// Gives the keyboard focus to the input.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    /// True when the input holds the keyboard focus.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Selects everything (used when an overlay opens with a prefilled value).
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.state.select_all();
        self.touch();
        cx.notify();
    }

    /// The current selection as text (empty when nothing is selected).
    pub fn selected_text(&self) -> String {
        self.state.selected_text().to_string()
    }

    // ----- editing helpers -----

    /// Restarts the caret blink so the caret is visible right after an action.
    fn touch(&mut self) {
        self.caret_on = true;
    }

    /// Runs a state change and reports the new text when it changed.
    fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut TextState) -> bool) {
        if self.disabled {
            return;
        }
        let before = self.state.text().to_string();
        f(&mut self.state);
        self.touch();
        if self.state.text() != before {
            cx.emit(LineInputEvent::Changed(self.state.text().to_string()));
        }
        cx.notify();
    }

    fn move_to(&mut self, cx: &mut Context<Self>, index: usize, extend: bool) {
        if self.disabled {
            return;
        }
        self.state.move_head(index, extend);
        self.touch();
        cx.notify();
    }

    fn display_index(&self, src: usize) -> usize {
        text_edit::display_index(self.state.text(), src, self.password)
    }

    /// Byte offset of the text under window x coordinate `x` (0 without a layout).
    fn index_at_x(&self, x: f32) -> usize {
        let Some(layout) = self.layout.as_ref() else {
            return 0;
        };
        let disp = layout.line.closest_index_for_x(px(x - layout.origin_x));
        text_edit::source_index(self.state.text(), disp, self.password)
    }

    fn copy_selection(&self, cx: &mut Context<Self>) {
        if !self.password && !self.state.range().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.state.selected_text().to_string(),
            ));
        }
    }

    fn commit_marked(&mut self) {
        self.marked = None;
    }

    // ----- actions -----

    fn act_left(&mut self, _: &Left, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let target = if self.state.range().is_empty() {
            text_edit::prev_boundary(self.state.text(), self.state.cursor())
        } else {
            self.state.range().start
        };
        self.move_to(cx, target, false);
    }

    fn act_right(&mut self, _: &Right, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let target = if self.state.range().is_empty() {
            text_edit::next_boundary(self.state.text(), self.state.cursor())
        } else {
            self.state.range().end
        };
        self.move_to(cx, target, false);
    }

    fn act_select_left(&mut self, _: &SelectLeft, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let t = text_edit::prev_boundary(self.state.text(), self.state.cursor());
        self.move_to(cx, t, true);
    }

    fn act_select_right(&mut self, _: &SelectRight, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let t = text_edit::next_boundary(self.state.text(), self.state.cursor());
        self.move_to(cx, t, true);
    }

    fn act_word_left(&mut self, _: &WordLeft, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let t = text_edit::prev_word_start(self.state.text(), self.state.cursor());
        self.move_to(cx, t, false);
    }

    fn act_word_right(&mut self, _: &WordRight, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let t = text_edit::next_word_end(self.state.text(), self.state.cursor());
        self.move_to(cx, t, false);
    }

    fn act_select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let t = text_edit::prev_word_start(self.state.text(), self.state.cursor());
        self.move_to(cx, t, true);
    }

    fn act_select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let t = text_edit::next_word_end(self.state.text(), self.state.cursor());
        self.move_to(cx, t, true);
    }

    fn act_home(&mut self, _: &Home, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.move_to(cx, 0, false);
    }

    fn act_end(&mut self, _: &End, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let end = self.state.text().len();
        self.move_to(cx, end, false);
    }

    fn act_select_home(&mut self, _: &SelectHome, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.move_to(cx, 0, true);
    }

    fn act_select_end(&mut self, _: &SelectEnd, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let end = self.state.text().len();
        self.move_to(cx, end, true);
    }

    fn act_select_all(&mut self, _: &SelectAll, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.state.select_all();
        self.touch();
        cx.notify();
    }

    fn act_backspace(&mut self, _: &Backspace, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.commit_marked();
        self.change(cx, |s| s.backspace());
    }

    fn act_delete(&mut self, _: &Delete, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.commit_marked();
        self.change(cx, |s| s.delete_forward());
    }

    fn act_delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_marked();
        self.change(cx, |s| s.delete_word_backward());
    }

    fn act_delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_marked();
        self.change(cx, |s| s.delete_word_forward());
    }

    fn act_copy(&mut self, _: &Copy, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    fn act_cut(&mut self, _: &Cut, _: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.password || self.state.range().is_empty() {
            return;
        }
        self.copy_selection(cx);
        self.commit_marked();
        self.change(cx, |s| s.replace(s.range(), "", EditKind::Other));
    }

    fn act_paste(&mut self, _: &Paste, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let line = text_edit::single_line(&text);
        self.commit_marked();
        self.change(cx, |s| s.insert(&line, EditKind::Other));
    }

    fn act_undo(&mut self, _: &Undo, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.change(cx, |s| s.undo());
    }

    fn act_redo(&mut self, _: &Redo, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.change(cx, |s| s.redo());
    }

    fn act_submit(&mut self, _: &Submit, _: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.commit_marked();
        cx.emit(LineInputEvent::Submit(self.state.text().to_string()));
    }

    fn act_cancel(&mut self, _: &Cancel, _: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        cx.emit(LineInputEvent::Cancel);
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        window.focus(&self.focus_handle);
        self.commit_marked();
        let idx = self.index_at_x(f32::from(ev.position.x));
        match ev.click_count {
            0 | 1 => {
                self.state.move_head(idx, ev.modifiers.shift);
                if !ev.modifiers.shift {
                    self.state.move_to(idx);
                }
                self.selecting = true;
            }
            2 => {
                let word = text_edit::word_range_at(self.state.text(), idx);
                self.state.set_selection(word.start, word.end);
            }
            _ => self.state.select_all(),
        }
        self.touch();
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || self.disabled {
            return;
        }
        if ev.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return;
        }
        let idx = self.index_at_x(f32::from(ev.position.x));
        self.state.move_head(idx, true);
        self.touch();
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }
}

// ----- IME input handler -----

impl EntityInputHandler for LineInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.state.text();
        let range = range_utf16_to_utf8(text, &range_utf16);
        actual_range.replace(range_utf8_to_utf16(text, &range));
        text.get(range).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let text = self.state.text();
        Some(UTF16Selection {
            range: range_utf8_to_utf16(text, &self.state.range()),
            reversed: self.state.reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|r| range_utf8_to_utf16(self.state.text(), r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let range = self.resolve_range(range_utf16);
        let was_marked = self.marked.take().is_some();
        let before = self.state.text().to_string();
        if was_marked {
            // The end of a composition: the marked text is replaced by the committed text.
            self.state.replace_raw(range, new_text);
        } else {
            self.state.begin_step(EditKind::Typing, new_text);
            self.state.replace_raw(range, new_text);
        }
        self.touch();
        if self.state.text() != before {
            cx.emit(LineInputEvent::Changed(self.state.text().to_string()));
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let range = self.resolve_range(range_utf16);
        if self.marked.is_none() {
            // One composition is one undo step: record it when it starts.
            self.state.begin_step(EditKind::Other, "");
        }
        let before = self.state.text().to_string();
        self.state.replace_raw(range.clone(), new_text);
        self.marked = if new_text.is_empty() {
            None
        } else {
            Some(range.start..range.start + new_text.len())
        };
        if let Some(sel) = new_selected_range_utf16 {
            let a = range.start + utf16_to_utf8(new_text, sel.start);
            let b = range.start + utf16_to_utf8(new_text, sel.end);
            self.state.set_selection(a, b);
        }
        self.touch();
        if self.state.text() != before {
            cx.emit(LineInputEvent::Changed(self.state.text().to_string()));
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = range_utf16_to_utf8(self.state.text(), &range_utf16);
        let a = self.display_index(range.start);
        let b = self.display_index(range.end);
        let x0 = layout.origin_x + f32::from(layout.line.x_for_index(a));
        let x1 = layout.origin_x + f32::from(layout.line.x_for_index(b));
        Some(Bounds::from_corners(
            point(px(x0), px(layout.origin_y)),
            point(px(x1.max(x0 + CARET_WIDTH)), px(layout.origin_y + LINE_H)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let idx = self.index_at_x(f32::from(position.x));
        Some(utf8_to_utf16(self.state.text(), idx))
    }
}

impl LineInput {
    /// Range (UTF-8) an input-handler call refers to: the given UTF-16 range, else the marked
    /// range, else the selection.
    fn resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        match range_utf16 {
            Some(r) => range_utf16_to_utf8(self.state.text(), &r),
            None => self.marked.clone().unwrap_or_else(|| self.state.range()),
        }
    }
}

// ----- element -----

struct TextElement {
    input: Entity<LineInput>,
}

struct Prepaint {
    line: ShapedLine,
    origin_x: f32,
    origin_y: f32,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        // Fills the input's height, so `prepaint` can centre the text row in it.
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let input = self.input.read(cx);
        let t = input.theme;
        let text = input.state.text().to_string();
        let range = input.state.range();
        let marked = input.marked.clone();
        let head = input.state.cursor();
        let password = input.password;
        let disabled = input.disabled;
        let focused = input.focus_handle.is_focused(window);
        let caret_on = input.caret_on;
        let placeholder = input.placeholder.clone();
        let old_scroll = input.scroll_x;

        let shown = text_edit::display_text(&text, password);
        let (display, color) = if text.is_empty() {
            (placeholder.to_string(), t.tx3)
        } else if disabled {
            (shown, t.tx3)
        } else {
            (shown, t.tx)
        };
        let base_run = TextRun {
            len: display.len(),
            font: font(UI_FONT),
            color: Hsla::from(color),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match (&marked, text.is_empty()) {
            (Some(m), false) => {
                let a = text_edit::display_index(&text, m.start, password);
                let b = text_edit::display_index(&text, m.end, password);
                vec![
                    TextRun {
                        len: a,
                        ..base_run.clone()
                    },
                    TextRun {
                        len: b - a,
                        underline: Some(UnderlineStyle {
                            color: Some(base_run.color),
                            thickness: px(1.0),
                            wavy: false,
                        }),
                        ..base_run.clone()
                    },
                    TextRun {
                        len: display.len() - b,
                        ..base_run
                    },
                ]
                .into_iter()
                .filter(|r| r.len > 0)
                .collect()
            }
            _ => vec![base_run],
        };
        let line = window.text_system().shape_line(
            SharedString::from(display.clone()),
            px(FONT_SIZE),
            &runs,
            None,
        );

        let caret_x = f32::from(line.x_for_index(text_edit::display_index(&text, head, password)));
        let total = f32::from(line.x_for_index(display.len()));
        let view_w = f32::from(bounds.size.width);
        let scroll = scroll_for_caret(caret_x, total, old_scroll, view_w);
        let origin_x = f32::from(bounds.left()) - scroll;
        let origin_y = f32::from(bounds.top()) + (f32::from(bounds.size.height) - LINE_H) / 2.0;

        let row_top = px(origin_y);
        let sel_quad = if !range.is_empty() && !text.is_empty() {
            let a =
                f32::from(line.x_for_index(text_edit::display_index(&text, range.start, password)));
            let b =
                f32::from(line.x_for_index(text_edit::display_index(&text, range.end, password)));
            Some(fill(
                Bounds::from_corners(
                    point(px(origin_x + a), row_top),
                    point(px(origin_x + b), row_top + px(LINE_H)),
                ),
                t.sel,
            ))
        } else {
            None
        };
        let caret = if focused && caret_on && !disabled && range.is_empty() {
            Some(fill(
                Bounds::new(
                    point(px(origin_x + caret_x), row_top),
                    size(px(CARET_WIDTH), px(LINE_H)),
                ),
                t.ac,
            ))
        } else {
            None
        };

        self.input.update(cx, |input, _| input.scroll_x = scroll);
        Prepaint {
            line,
            origin_x,
            origin_y,
            selection: sel_quad,
            caret,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(_bounds, self.input.clone()),
            cx,
        );
        if let Some(quad) = prepaint.selection.take() {
            window.paint_quad(quad);
        }
        let origin = point(px(prepaint.origin_x), px(prepaint.origin_y));
        let _ = prepaint.line.paint(origin, px(LINE_H), window, cx);
        if let Some(caret) = prepaint.caret.take() {
            window.paint_quad(caret);
        }
        let line = prepaint.line.clone();
        let (ox, oy) = (prepaint.origin_x, prepaint.origin_y);
        self.input.update(cx, |input, _| {
            input.layout = Some(PaintLayout {
                line,
                origin_x: ox,
                origin_y: oy,
            });
        });
    }
}

/// Horizontal scroll that keeps the caret inside a view `view_w` wide. The text is shifted only
/// as far as needed and never past its start or end.
pub fn scroll_for_caret(caret_x: f32, total_w: f32, scroll: f32, view_w: f32) -> f32 {
    let max_scroll = (total_w - view_w + CARET_WIDTH).max(0.0);
    let mut s = scroll.clamp(0.0, max_scroll);
    if caret_x - s < 0.0 {
        s = caret_x;
    } else if caret_x - s > view_w - CARET_WIDTH {
        s = caret_x - view_w + CARET_WIDTH;
    }
    s.clamp(0.0, max_scroll)
}

impl Render for LineInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let focused = self.focus_handle.is_focused(window);
        let disabled = self.disabled;
        let mut row = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(HEIGHT))
            .px(px(10.0))
            .rounded(px(8.0))
            .overflow_hidden()
            .bg(t.raised)
            .border_1()
            .border_color(if focused && !disabled { t.ac } else { t.line })
            .font_family(UI_FONT)
            .text_size(px(FONT_SIZE))
            .text_color(if disabled { t.tx3 } else { t.tx })
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(if disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::IBeam
            });
        if !disabled {
            row = row
                .on_action(cx.listener(Self::act_left))
                .on_action(cx.listener(Self::act_right))
                .on_action(cx.listener(Self::act_select_left))
                .on_action(cx.listener(Self::act_select_right))
                .on_action(cx.listener(Self::act_word_left))
                .on_action(cx.listener(Self::act_word_right))
                .on_action(cx.listener(Self::act_select_word_left))
                .on_action(cx.listener(Self::act_select_word_right))
                .on_action(cx.listener(Self::act_home))
                .on_action(cx.listener(Self::act_end))
                .on_action(cx.listener(Self::act_select_home))
                .on_action(cx.listener(Self::act_select_end))
                .on_action(cx.listener(Self::act_select_all))
                .on_action(cx.listener(Self::act_backspace))
                .on_action(cx.listener(Self::act_delete))
                .on_action(cx.listener(Self::act_delete_word_backward))
                .on_action(cx.listener(Self::act_delete_word_forward))
                .on_action(cx.listener(Self::act_copy))
                .on_action(cx.listener(Self::act_cut))
                .on_action(cx.listener(Self::act_paste))
                .on_action(cx.listener(Self::act_undo))
                .on_action(cx.listener(Self::act_redo))
                .on_action(cx.listener(Self::act_submit))
                .on_action(cx.listener(Self::act_cancel))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                .on_mouse_move(cx.listener(Self::on_mouse_move))
                .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up));
        }
        row.child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(TextElement { input: cx.entity() }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_keeps_caret_visible_and_clamped() {
        // Text fits: no scroll.
        assert_eq!(scroll_for_caret(50.0, 80.0, 0.0, 200.0), 0.0);
        // Caret past the right edge of a 100px view over 300px of text.
        let s = scroll_for_caret(250.0, 300.0, 0.0, 100.0);
        assert!((s - (250.0 - 100.0 + CARET_WIDTH)).abs() < 1e-3);
        // Caret moved left of the visible part: scroll follows it.
        assert_eq!(scroll_for_caret(10.0, 300.0, 150.0, 100.0), 10.0);
        // Never scroll past the end.
        assert!(scroll_for_caret(300.0, 300.0, 900.0, 100.0) <= 300.0 - 100.0 + CARET_WIDTH + 1e-3);
        // Never negative.
        assert_eq!(scroll_for_caret(0.0, 300.0, -20.0, 100.0), 0.0);
    }

    // Checked at compile time: a caret that is zero or wider than 3 px fails the build.
    const _: () = assert!(CARET_WIDTH > 0.0 && CARET_WIDTH < 3.0);
}
