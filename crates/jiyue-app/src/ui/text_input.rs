//! Multi-line editable text view with IME support, driven by `EditorState`.
//!
//! The document column is `COLUMN_WIDTH` wide and centred, with soft wrap. Every logical line
//! is shaped by the gpui text system at the wrap width; the last paint keeps the shaped lines
//! and the window origin in `Layout`, which hit testing, caret placement and the IME input
//! handler (`bounds_for_range`, `character_index_for_point`) read.
//!
//! Sections: pure helpers, constants and actions, state, layout, element (prepaint and paint),
//! IME input handler, editing, mouse and scroll, key bindings, render.
#![allow(dead_code)]

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, BorderStyle, Bounds, ClipboardItem, Context, Corners, CursorStyle, Edges, Element,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render,
    Rgba, ScrollWheelEvent, SharedString, StrikethroughStyle, Style, Subscription, TextAlign,
    TextRun, TransformationMatrix, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions,
    div, fill, font, point, prelude::*, px, quad, relative, rgb, size,
};
use gpui::{FontStyle, FontWeight};

use super::highlight;
use super::inline::wikilink_inner_at;
use super::keymap::{primary, primary_held};
use super::md_style::{self, Segment, SpanStyle};
/// Block widgets of the live view: code panels and frontmatter (see `ui/live/blocks.rs`).
#[path = "live/blocks.rs"]
pub mod blocks;
/// Callouts `> [!type]` of the live view (see `ui/live/callout.rs`).
#[path = "live/callout.rs"]
pub mod callout;
/// Vault data that the image and embed widgets read (see `ui/live/context.rs`).
#[path = "live/context.rs"]
pub mod context;
/// Mermaid diagrams of the live view: source, support and sizing (see `ui/live/diagram.rs`).
#[path = "live/diagram.rs"]
pub mod diagram;
/// The Mermaid block widget (see `ui/live/diagram_widget.rs`).
#[path = "live/diagram_widget.rs"]
mod diagram_widget;
/// Images and note embeds of the live view (see `ui/live/embed.rs`).
#[path = "live/embed.rs"]
pub mod embed;
/// Footnotes of the live view: references, definitions, jumps and the insert command (see
/// `ui/live/footnote.rs`).
#[path = "live/footnote.rs"]
pub mod footnote;
/// Line reuse across edits: which old lines a new line comes from (see `ui/live/invalidate.rs`).
#[path = "live/invalidate.rs"]
pub mod invalidate;
/// Display mapping of the live preview (pure logic, see `ui/live/map.rs`).
#[path = "live/map.rs"]
pub mod map;
/// GFM table parsing and column widths of the live view (see `ui/live/table.rs`).
#[path = "live/table.rs"]
pub mod table;
use crate::editor_state::{EditorState, Selection};
use crate::md_edit::{self, EditPlan};
use crate::theme::{ActiveTheme, ThemeMode};
use crate::ui::attach_ui::resolve::{basename, caption};
use crate::ui::fonts::{self, Face};
use gpui::RenderImage;
use std::cell::RefCell;
use std::sync::Arc;

// ----- pure helpers -------------------------------------------------------------------

/// UTF-16 code-unit offset -> UTF-8 byte offset. Offsets past the end clamp to the text length.
/// An offset inside a surrogate pair rounds up to the end of that character.
pub fn utf16_to_utf8(text: &str, utf16: usize) -> usize {
    let mut bytes = 0;
    let mut units = 0;
    for ch in text.chars() {
        if units >= utf16 {
            break;
        }
        units += ch.len_utf16();
        bytes += ch.len_utf8();
    }
    bytes
}

/// UTF-8 byte offset -> UTF-16 code-unit offset. An offset inside a character rounds up.
pub fn utf8_to_utf16(text: &str, bytes: usize) -> usize {
    let mut units = 0;
    let mut seen = 0;
    for ch in text.chars() {
        if seen >= bytes {
            break;
        }
        seen += ch.len_utf8();
        units += ch.len_utf16();
    }
    units
}

/// UTF-16 range (as the OS sends it) -> ordered UTF-8 byte range.
pub fn range_utf16_to_utf8(text: &str, range: &Range<usize>) -> Range<usize> {
    let a = utf16_to_utf8(text, range.start);
    let b = utf16_to_utf8(text, range.end);
    a.min(b)..a.max(b)
}

/// UTF-8 byte range -> UTF-16 range.
pub fn range_utf8_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    utf8_to_utf16(text, range.start)..utf8_to_utf16(text, range.end)
}

/// Clipboard text -> text with `\n` line endings (CRLF and lone CR both become LF).
pub fn clipboard_to_text(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// New scroll offset that keeps `[top, bottom]` inside a viewport of height `viewport`.
pub fn scroll_to_reveal(top: f32, bottom: f32, scroll: f32, viewport: f32) -> f32 {
    if top < scroll {
        top
    } else if bottom > scroll + viewport {
        bottom - viewport
    } else {
        scroll
    }
}

/// Vertical centre of the visual row `rows` below (negative: above) a row whose top is `top`.
pub fn row_centre_y(top: f32, rows: i32) -> f32 {
    top + fonts::current_metrics().line * rows as f32 + fonts::current_metrics().line / 2.0
}

/// The selection a plan leaves behind (its offsets are already in the resulting text).
pub fn plan_selection(plan: &EditPlan) -> Selection {
    Selection {
        anchor: plan.sel_anchor,
        head: plan.sel_head,
    }
}

/// The only character of a committed text, if the text is exactly one printable char.
/// Only such commits (typing, IME punctuation) may take part in auto-pairing; pastes and
/// multi-char commits (pinyin words) never do.
pub fn single_typed_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => Some(c),
        _ => None,
    }
}

// ----- constants and actions ----------------------------------------------------------

/// Width of the document column (the same as the live preview).
const COLUMN_WIDTH: f32 = 620.0;
/// The column of read mode (see `TextEditor::read_only`).
const READ_COLUMN_WIDTH: f32 = 720.0;
/// Extra height of a blank line in read mode: wider space between paragraphs.
const READ_PARA_GAP: f32 = 12.0;
/// Minimum gap kept on each side when the window is narrower than the column.
const GUTTER: f32 = 16.0;
const FONT_SIZE: f32 = 15.0;
const LINE_HEIGHT: f32 = 26.0;
/// Font size of fenced and indented code lines (source view only).
const CODE_FONT_SIZE: f32 = 13.5;
/// Font size and row height of ATX heading lines, H1 to H6. Taller rows keep the glyphs apart.
const HEADING_FONT_SIZE: [f32; 6] = [28.0, 22.0, 18.0, 16.0, 15.0, 15.0];
const HEADING_LINE_HEIGHT: [f32; 6] = [44.0, 36.0, 30.0, 28.0, LINE_HEIGHT, LINE_HEIGHT];
/// Space above the first line and below the last one (scrollable).
const PAD_TOP: f32 = 30.0;
const PAD_BOTTOM: f32 = 80.0;
const CARET_WIDTH: f32 = 2.0;
/// Width drawn for a selected newline.
const NEWLINE_MARK_WIDTH: f32 = 8.0;
const CARET_BLINK_MS: u64 = 500;
/// Side of a task box in the live view.
const CHECK_SIZE: f32 = 14.0;
/// Footnote labels: the font size of an ordinal is 0.72 of the body size, the width of one blank
/// cell is half of it (see `map::GAP_CELL`), and the top of a label is raised below its row top.
const FOOTNOTE_TOP: f32 = 3.0;
/// Width of a block quote bar in the live view.
const QUOTE_BAR_WIDTH: f32 = 3.0;
/// Code panel of the live view: header band height, side and top/bottom padding, the row height
/// of a code line, the gap under a panel, and the size of its copy button.
const CODE_HEADER_H: f32 = 30.0;
const CODE_PAD_X: f32 = 14.0;
const CODE_PAD_Y: f32 = 8.0;
const CODE_LINE_H: f32 = 20.0;
const CODE_GAP: f32 = 12.0;
const CODE_BUTTON: (f32, f32) = (46.0, 22.0);
const CODE_FONT_SIZE_LIVE: f32 = 13.0;
const PANEL_RADIUS: f32 = 8.0;
/// Table grid of the live view: font size, row height of a wrapped cell line, padding of a cell
/// and the narrowest a column shrinks to before its text wraps word by word.
const TABLE_FONT_SIZE: f32 = 14.0;
const TABLE_LINE_H: f32 = 22.0;
const TABLE_PAD_X: f32 = 10.0;
const TABLE_PAD_Y: f32 = 6.0;
const TABLE_MIN_COL: f32 = 56.0;
/// Callout panel: title row height, icon side, the x of the title and body text, the bar width,
/// the title and body font sizes, the body row height and the padding above and below the body.
const CALLOUT_TITLE_H: f32 = 30.0;
const CALLOUT_ICON: f32 = 16.0;
const CALLOUT_TEXT_X: f32 = 36.0;
const CALLOUT_BAR_W: f32 = 3.0;
const CALLOUT_TEXT_SIZE: f32 = 14.0;
const CALLOUT_BODY_SIZE: f32 = 14.5;
const CALLOUT_BODY_LH: f32 = 22.0;
const CALLOUT_PAD_Y: f32 = 4.0;
/// Height of the spacer that a hidden frontmatter block leaves (kept above zero for the layout).
const FRONTMATTER_GAP: f32 = 6.0;

// Image and note embed widgets (see `embed.rs`).
/// Space above a picture or a card, and below it.
const IMG_TOP: f32 = 8.0;
const IMG_GAP: f32 = 16.0;
/// Height of a caption line under a picture.
const IMG_CAPTION_H: f32 = 24.0;
const IMG_RADIUS: f32 = 8.0;
/// Height of a card that says why nothing is drawn (missing file, remote image, loading).
const CARD_H: f32 = 60.0;
/// Height of the title bar of a note excerpt, and the side padding of its body.
const EMBED_HEAD_H: f32 = 34.0;
const EMBED_PAD_X: f32 = 16.0;
const EMBED_BODY_SIZE: f32 = 14.5;
const EMBED_BODY_LH: f32 = 22.0;
/// Shaped lines kept between frames; the cache starts again when it grows past this.
const LINE_CACHE_MAX: usize = 65536;
/// Most search matches painted at once (a bigger result list is still searched, just not drawn).
const MAX_PAINTED_MATCHES: usize = 2000;
const TAB_TEXT: &str = "    ";
pub const CONTEXT: &str = "TextEditor";

actions!(
    text_editor,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        MoveWordLeft,
        MoveWordRight,
        SelectWordLeft,
        SelectWordRight,
        MoveLineStart,
        MoveLineEnd,
        SelectLineStart,
        SelectLineEnd,
        MoveDocStart,
        MoveDocEnd,
        SelectDocStart,
        SelectDocEnd,
        PageUp,
        PageDown,
        SelectPageUp,
        SelectPageDown,
        Backspace,
        Delete,
        DeleteWordBackward,
        DeleteWordForward,
        InsertNewline,
        InsertTab,
        OutdentTab,
        SelectAll,
        CopyText,
        CutText,
        PasteText,
        Undo,
        Redo,
        MoveLineUp,
        MoveLineDown,
        DuplicateLine,
        InsertFootnote,
    ]
);

/// Registers the editor key bindings (context `TextEditor`). Call once at app start.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, Some(CONTEXT)),
        KeyBinding::new("right", MoveRight, Some(CONTEXT)),
        KeyBinding::new("up", MoveUp, Some(CONTEXT)),
        KeyBinding::new("down", MoveDown, Some(CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("ctrl-left", MoveWordLeft, Some(CONTEXT)),
        KeyBinding::new("ctrl-right", MoveWordRight, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, Some(CONTEXT)),
        KeyBinding::new("home", MoveLineStart, Some(CONTEXT)),
        KeyBinding::new("end", MoveLineEnd, Some(CONTEXT)),
        KeyBinding::new("shift-home", SelectLineStart, Some(CONTEXT)),
        KeyBinding::new("shift-end", SelectLineEnd, Some(CONTEXT)),
        KeyBinding::new("ctrl-home", MoveDocStart, Some(CONTEXT)),
        KeyBinding::new("ctrl-end", MoveDocEnd, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-home", SelectDocStart, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-end", SelectDocEnd, Some(CONTEXT)),
        KeyBinding::new("pageup", PageUp, Some(CONTEXT)),
        KeyBinding::new("pagedown", PageDown, Some(CONTEXT)),
        KeyBinding::new("shift-pageup", SelectPageUp, Some(CONTEXT)),
        KeyBinding::new("shift-pagedown", SelectPageDown, Some(CONTEXT)),
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("ctrl-backspace", DeleteWordBackward, Some(CONTEXT)),
        KeyBinding::new("ctrl-delete", DeleteWordForward, Some(CONTEXT)),
        KeyBinding::new("enter", InsertNewline, Some(CONTEXT)),
        KeyBinding::new("tab", InsertTab, Some(CONTEXT)),
        KeyBinding::new("shift-tab", OutdentTab, Some(CONTEXT)),
        // Ctrl+letter editing keys: Cmd on macOS (`keymap::primary`).
        KeyBinding::new(&primary("ctrl-a"), SelectAll, Some(CONTEXT)),
        KeyBinding::new(&primary("ctrl-c"), CopyText, Some(CONTEXT)),
        KeyBinding::new(&primary("ctrl-x"), CutText, Some(CONTEXT)),
        KeyBinding::new(&primary("ctrl-v"), PasteText, Some(CONTEXT)),
        KeyBinding::new(&primary("ctrl-z"), Undo, Some(CONTEXT)),
        KeyBinding::new(&primary("ctrl-y"), Redo, Some(CONTEXT)),
        KeyBinding::new(&primary("ctrl-shift-z"), Redo, Some(CONTEXT)),
        KeyBinding::new("alt-up", MoveLineUp, Some(CONTEXT)),
        KeyBinding::new("alt-down", MoveLineDown, Some(CONTEXT)),
        KeyBinding::new("alt-shift-down", DuplicateLine, Some(CONTEXT)),
        // InsertFootnote is bound from the keymap registry (`keymap::rebind`).
    ]);
}

// ----- state --------------------------------------------------------------------------

/// An IME composition in progress. `base` is the document before the first composing edit and
/// `range` the part of it that composing replaces. Composing edits are applied to the live
/// document as the user types, and on commit the base is restored and the final text applied
/// once, so the whole composition is one undo step.
struct Composition {
    base: EditorState,
    range: Range<usize>,
}

/// One logical line in the last layout.
struct LineLayout {
    /// Byte offset of the line in the document.
    start: usize,
    /// Length in bytes, without the newline.
    len: usize,
    /// Top of the first visual row, in document space (no padding, no scroll).
    top: f32,
    /// Visual rows: soft wraps plus one.
    rows: usize,
    /// Height of one visual row of this line (headings are taller).
    line_h: f32,
    /// Shaped text (the display text, see `map`); `None` when the line shows nothing.
    wrapped: Option<WrappedLine>,
    /// How the source line is shown: source pieces and replaced ones. Empty for an empty line.
    pieces: Vec<map::Piece>,
    /// A block widget (code panel, hidden frontmatter) replacing the source lines of `start..start+len`.
    /// Such a unit has one visual row of the widget's height and no pieces.
    widget: Option<Box<Widget>>,
    /// Inline formulas drawn on this line (see `inline_math_pass`).
    inline: Vec<InlineShot>,
    /// False for a placeholder: a unit outside the built window, whose height is an estimate and
    /// which has no pieces or widget yet (see `prepaint`).
    built: bool,
}

/// Moves the source offsets of a widget by `delta` bytes. A widget keeps the offsets of the
/// source it was built from, so it is moved when it is reused at another place in the document.
fn shift_widget(w: &mut Widget, delta: isize) {
    if let Some((_, body)) = w.button.as_mut() {
        *body = invalidate::shift_range(body, delta);
    }
    for cell in w.cells.iter_mut() {
        cell.src = invalidate::shift_range(&cell.src, delta);
        for p in cell.pieces.iter_mut() {
            p.src = invalidate::shift_range(&p.src, delta);
        }
    }
}

impl LineLayout {
    /// Moves the unit by `delta` bytes of source: its start, pieces, formulas and widget offsets.
    fn shift(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }
        self.start = invalidate::shift(self.start, delta);
        for p in self.pieces.iter_mut() {
            p.src = invalidate::shift_range(&p.src, delta);
        }
        for s in self.inline.iter_mut() {
            s.src = invalidate::shift(s.src, delta);
        }
        if let Some(w) = self.widget.as_mut() {
            shift_widget(w, delta);
        }
    }
}

/// A rendered inline formula on a line: its source offset, CSS size, baseline distance from its
/// top and raster.
struct InlineShot {
    src: usize,
    css: (f32, f32),
    baseline: f32,
    image: Arc<RenderImage>,
}

/// A rectangle `(x, y, w, h)` in CSS pixels.
type Rect4 = (f32, f32, f32, f32);
/// An icon: its rectangle, its name in `assets/icons` (`None`: a coloured dot) and its colour.
type Icon = (Rect4, Option<&'static str>, Rgba);

/// What a block widget paints, relative to the top of its unit.
#[derive(Default, Clone)]
struct Widget {
    /// Rounded panel `(x, y, w, h)`.
    panel: Option<(f32, f32, f32, f32)>,
    /// Plain rectangles `(x, y, w, h)` in a colour, painted over the panel (grid lines, header).
    fills: Vec<((f32, f32, f32, f32), Rgba)>,
    /// Shaped text (see `WidgetText`).
    texts: Vec<WidgetText>,
    /// Copy button: its rectangle `(x, y, w, h)` and the source range of the body it copies.
    button: Option<(Rect4, Range<usize>)>,
    /// Table cells, for clicks: the caret goes to the source position of the click.
    cells: Vec<CellHit>,
    /// Fill of the panel, when it is not the raised colour (callouts are tinted).
    panel_fill: Option<Rgba>,
    /// Icons `(x, y, w, h)` with their name in `assets/icons` (`None`: a coloured dot), and colour.
    icons: Vec<Icon>,
    /// A foldable callout: its title row and fold key (for clicks).
    callout: Option<CalloutHit>,
    /// The fold state the widget was built with (a callout is rebuilt when it changes).
    open: bool,
    /// A picture painted at `(x, y, w, h)` (see `context`).
    image: Option<(Rect4, Arc<RenderImage>)>,
    /// Clickable rectangles `(x, y, w, h)` that open a link body (the title bar of a note embed).
    links: Vec<((f32, f32, f32, f32), String)>,
    /// The vault data generation the widget was built from (None: it reads no vault data).
    vault_gen: Option<u64>,
    /// The widget waits for an image decode: it is rebuilt on the next frame.
    loading: bool,
}

/// The title row of a foldable callout widget, with the key and state a click toggles.
#[derive(Clone)]
struct CalloutHit {
    title_h: f32,
    key: u64,
    open: bool,
}

/// One shaped text of a widget, at `(x, y)` relative to the unit top. A text with `align_w` is
/// aligned inside a box of that width (table cells); otherwise it starts at `x`.
#[derive(Clone)]
struct WidgetText {
    x: f32,
    y: f32,
    line: WrappedLine,
    lh: f32,
    align: TextAlign,
    align_w: Option<f32>,
}

/// A table cell of a widget: its rectangle and shaped text (relative to the unit top), with the
/// pieces that map a shaped index back to a source offset.
#[derive(Clone)]
struct CellHit {
    rect: (f32, f32, f32, f32),
    src: Range<usize>,
    pieces: Vec<map::Piece>,
    wrapped: Option<WrappedLine>,
    text_x: f32,
    text_y: f32,
}

/// A text of a widget that starts at `x`, `y` (no alignment).
fn plain_text(x: f32, y: f32, line: WrappedLine, lh: f32) -> WidgetText {
    WidgetText {
        x,
        y,
        line,
        lh,
        align: TextAlign::Left,
        align_w: None,
    }
}

impl LineLayout {
    /// Height of the whole logical line (all its visual rows).
    fn height(&self) -> f32 {
        self.rows as f32 * self.line_h
    }

    /// Absolute source offset closest to `(x, y)`, where `y` is measured from the top of this line.
    /// The shaped text is the display text, so its index goes back through the pieces.
    fn index_in(&self, x: f32, y: f32) -> usize {
        if self.pieces.is_empty() {
            return self.start;
        }
        let disp = match self.wrapped.as_ref() {
            Some(wrapped) => {
                match wrapped.closest_index_for_position(point(px(x), px(y)), px(self.line_h)) {
                    Ok(i) | Err(i) => i,
                }
            }
            None => 0,
        };
        map::to_source(&self.pieces, disp, self.start + self.len)
    }
}

/// Shaped lines from the last prepaint, in document space, plus the window origin.
#[derive(Default)]
struct Layout {
    /// Window x of document x = 0 (left edge of the column).
    origin_x: f32,
    /// Window y of document y = 0 (already includes the top padding and the scroll offset).
    origin_y: f32,
    wrap_width: f32,
    viewport_h: f32,
    lines: Vec<LineLayout>,
    /// Task boxes of the live view, in document space (no origin): `(x, y, w, h, source offset)`.
    checks: Vec<(f32, f32, f32, f32, usize)>,
    /// Copy buttons of the code panels, in document space: `(x, y, w, h, body source range)`.
    buttons: Vec<(f32, f32, f32, f32, Range<usize>)>,
    /// Title rows of the foldable callouts, in document space: `(x, y, w, h, key, open)`.
    fold_hits: Vec<(f32, f32, f32, f32, u64, bool)>,
    /// Link rectangles of the widgets, in document space: `(x, y, w, h, link body)`.
    links: Vec<(f32, f32, f32, f32, String)>,
    /// Back arrows of the footnote definitions, in document space: `(x, y, w, h, ordinal)`.
    footnote_backs: Vec<(f32, f32, f32, f32, u32)>,
}

impl Layout {
    fn content_height(&self) -> f32 {
        self.lines.last().map_or(0.0, |l| l.top + l.height())
    }

    /// The link body of the widget rectangle under document-space `(x, y)`, if any.
    fn link_at(&self, x: f32, y: f32) -> Option<String> {
        self.links.iter().find_map(|(lx, ly, lw, lh, raw)| {
            (x >= *lx && x <= lx + lw && y >= *ly && y <= ly + lh).then(|| raw.clone())
        })
    }

    /// Source range of the code body whose copy button is under document-space `(x, y)`.
    fn button_at(&self, x: f32, y: f32) -> Option<Range<usize>> {
        self.buttons.iter().find_map(|(bx, by, bw, bh, body)| {
            (x >= *bx && x <= bx + bw && y >= *by && y <= by + bh).then(|| body.clone())
        })
    }

    /// Source offset under document-space `(x, y)` when that point is in a table cell: the source
    /// position of the click inside the cell.
    fn table_source_at(&self, x: f32, y: f32) -> Option<usize> {
        let line = self.lines.iter().find(|l| {
            y >= l.top
                && y < l.top + l.height()
                && l.widget.as_ref().is_some_and(|w| !w.cells.is_empty())
        })?;
        let w = line.widget.as_ref()?;
        let yy = y - line.top;
        let cell = w.cells.iter().find(|c| {
            let (cx, cy, cw, ch) = c.rect;
            x >= cx && x < cx + cw && yy >= cy && yy < cy + ch
        })?;
        let disp = match cell.wrapped.as_ref() {
            Some(wl) => {
                let p = point(px(x - cell.text_x), px(yy - cell.text_y));
                match wl
                    .closest_index_for_position(p, px(TABLE_LINE_H * fonts::current_metrics().k))
                {
                    Ok(i) | Err(i) => i,
                }
            }
            None => 0,
        };
        Some(map::to_source(&cell.pieces, disp, cell.src.end))
    }

    /// The foldable callout whose title row is under document-space `(x, y)`: its key and state.
    fn fold_at(&self, x: f32, y: f32) -> Option<(u64, bool)> {
        self.fold_hits
            .iter()
            .find_map(|&(fx, fy, fw, fh, key, open)| {
                (x >= fx && x <= fx + fw && y >= fy && y <= fy + fh).then_some((key, open))
            })
    }

    /// The ordinal of the footnote back arrow under document-space `(x, y)`, if any.
    fn back_at(&self, x: f32, y: f32) -> Option<u32> {
        self.footnote_backs.iter().find_map(|&(bx, by, bw, bh, n)| {
            (x >= bx && x <= bx + bw && y >= by && y <= by + bh).then_some(n)
        })
    }

    /// Whether document-space `(x, y)` is on a widget a plain click acts on: a task box, a copy
    /// button, an embed title or widget link, a callout fold row or a footnote back arrow.
    fn widget_at(&self, x: f32, y: f32) -> bool {
        self.check_at(x, y).is_some()
            || self.button_at(x, y).is_some()
            || self.link_at(x, y).is_some()
            || self.fold_at(x, y).is_some()
            || self.back_at(x, y).is_some()
    }

    /// Source offset of the task box under document-space `(x, y)`, if any.
    fn check_at(&self, x: f32, y: f32) -> Option<usize> {
        self.checks.iter().find_map(|&(bx, by, bw, bh, at)| {
            (x >= bx - 3.0 && x <= bx + bw + 3.0 && y >= by - 3.0 && y <= by + bh + 3.0)
                .then_some(at)
        })
    }

    /// Height of the visual row that holds byte `index` (the caret's row).
    fn row_height_at(&self, index: usize) -> f32 {
        self.lines
            .get(self.lines.partition_point(|l| l.start + l.len < index))
            .or(self.lines.last())
            .map_or(fonts::current_metrics().line, |l| l.line_h)
    }

    /// Centre y of the visual row above or below the row that contains document-space `y`.
    /// `None` when there is no such row (the first or last row of the document).
    fn adjacent_row_centre(&self, y: f32, dir: i32) -> Option<f32> {
        let idx = self
            .lines
            .iter()
            .position(|l| y >= l.top && y < l.top + l.height())?;
        let line = &self.lines[idx];
        let row = (((y - line.top) / line.line_h).floor().max(0.0) as usize).min(line.rows - 1);
        if dir > 0 {
            if row + 1 < line.rows {
                return Some(line.top + (row + 1) as f32 * line.line_h + line.line_h / 2.0);
            }
            let next = self.lines.get(idx + 1)?;
            Some(next.top + next.line_h / 2.0)
        } else {
            if row > 0 {
                return Some(line.top + (row - 1) as f32 * line.line_h + line.line_h / 2.0);
            }
            let prev = self.lines.get(idx.checked_sub(1)?)?;
            Some(prev.top + (prev.rows - 1) as f32 * prev.line_h + prev.line_h / 2.0)
        }
    }

    /// Byte offset closest to a document-space point. Above the text gives 0, below it the end.
    fn index_at(&self, x: f32, y: f32, text_len: usize) -> usize {
        if y < 0.0 {
            return 0;
        }
        for line in &self.lines {
            if y < line.top + line.height() {
                return line.index_in(x, y - line.top);
            }
        }
        text_len
    }

    /// Document-space `(x, y)` of the caret position for byte `index` (y is the row top).
    fn point_for_index(&self, index: usize) -> (f32, f32) {
        let found = self
            .lines
            .get(self.lines.partition_point(|l| l.start + l.len < index))
            .or(self.lines.last());
        let Some(line) = found else {
            return (0.0, 0.0);
        };
        let Some(wrapped) = line.wrapped.as_ref() else {
            return (0.0, line.top);
        };
        let disp = map::to_display(&line.pieces, index);
        match wrapped.position_for_index(disp, px(line.line_h)) {
            Some(p) => (f32::from(p.x), line.top + f32::from(p.y)),
            None => (0.0, line.top),
        }
    }

    /// Selection rectangles `(x, y, w, h)` in document space, one per visual row segment.
    /// A selected newline adds a short marker at the end of its line.
    fn selection_rects(&self, sel: &Range<usize>) -> Vec<(f32, f32, f32, f32)> {
        let mut out = Vec::new();
        if sel.is_empty() {
            return out;
        }
        let last = self.lines.len().saturating_sub(1);
        // Lines are in document order, so start at the first one that can reach the range.
        let first = self.lines.partition_point(|l| l.start + l.len < sel.start);
        for (i, line) in self.lines.iter().enumerate().skip(first) {
            if line.start >= sel.end {
                break;
            }
            let line_end = line.start + line.len;
            if line.widget.is_some() {
                // A widget is selected as a whole when the selection covers any of its source.
                if sel.start < line_end && sel.end > line.start {
                    out.push((0.0, line.top, self.wrap_width, line.height()));
                }
                continue;
            }
            let newline = i < last && sel.end > line_end && sel.start <= line_end;
            let a = sel.start.max(line.start);
            let b = sel.end.min(line_end);
            if a > b || (a == b && !newline) {
                continue;
            }
            let (ax, ay) = self.point_for_index(a);
            let (bx, by) = self.point_for_index(b);
            let row_a = row_of(ay - line.top, line.line_h);
            let row_b = row_of(by - line.top, line.line_h);
            for row in row_a..=row_b {
                let x0 = if row == row_a { ax } else { 0.0 };
                let mut x1 = if row == row_b { bx } else { self.wrap_width };
                if newline && row == row_b {
                    x1 += NEWLINE_MARK_WIDTH;
                }
                if x1 - x0 <= 0.0 {
                    continue;
                }
                out.push((
                    x0,
                    line.top + row as f32 * line.line_h,
                    x1 - x0,
                    line.line_h,
                ));
            }
        }
        out
    }
}

/// Visual row index for a y offset from a line top, for rows `row_h` tall.
fn row_of(y_in_line: f32, row_h: f32) -> usize {
    (y_in_line / row_h).round().max(0.0) as usize
}

/// The font size and row height of a logical line, from its segments: headings are larger and
/// taller, code lines use the smaller code size.
fn line_metrics(segs: &[Segment]) -> (f32, f32) {
    let heading = segs.iter().map(|s| s.style.heading).max().unwrap_or(0);
    if heading > 0 {
        let i = usize::from(heading.min(6)) - 1;
        (
            fonts::current_metrics().heading[i],
            fonts::current_metrics().heading_lh[i],
        )
    } else if segs
        .iter()
        .any(|s| s.style.has(SpanStyle::CODE_BLOCK) || s.style.has(SpanStyle::CODE_FENCE))
    {
        (fonts::current_metrics().code, fonts::current_metrics().line)
    } else {
        (fonts::current_metrics().font, fonts::current_metrics().line)
    }
}

/// How one piece of text is drawn: colour, font, weight and decorations.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    color: Rgba,
    family: Face,
    weight: FontWeight,
    italic: bool,
    strike: bool,
    /// Underline in the accent colour (link text).
    underline: bool,
    background: Option<Rgba>,
}

/// The look of a segment. Colour priority, highest first: syntax markers and link URLs (tx3),
/// links, tags, list and task markers, callout titles (ac), fence and quote lines (tx2), and
/// the plain text colour (tx).
fn look_for(s: SpanStyle, t: &ActiveTheme) -> Look {
    let mut look = Look {
        color: t.tx,
        family: Face::Editor,
        weight: FontWeight::NORMAL,
        italic: false,
        strike: false,
        underline: false,
        background: None,
    };
    if s.heading > 0 {
        look.family = Face::Heading;
        look.weight = if s.heading <= 2 {
            FontWeight::BOLD
        } else {
            FontWeight::SEMIBOLD
        };
    }
    if s.has(SpanStyle::CODE) || s.has(SpanStyle::CODE_BLOCK) || s.has(SpanStyle::CODE_FENCE) {
        look.family = Face::Code;
    }
    if s.has(SpanStyle::BOLD) {
        look.weight = FontWeight::BOLD;
    }
    if s.has(SpanStyle::CALLOUT_TITLE) && look.weight == FontWeight::NORMAL {
        look.weight = FontWeight::SEMIBOLD;
    }
    look.italic = s.has(SpanStyle::ITALIC);
    look.strike = s.has(SpanStyle::STRIKE);
    look.color = if s.has(SpanStyle::MARKER)
        || s.has(SpanStyle::LINK_URL)
        || s.has(SpanStyle::BLOCK_ID)
        || s.has(SpanStyle::HR)
    {
        t.tx3
    } else if s.has(SpanStyle::LINK_TEXT)
        || s.has(SpanStyle::WIKILINK)
        || s.has(SpanStyle::TAG)
        || s.has(SpanStyle::LIST_MARKER)
        || s.has(SpanStyle::TASK_MARKER)
        || s.has(SpanStyle::CALLOUT_TITLE)
    {
        t.ac
    } else if s.has(SpanStyle::FRONTMATTER)
        || s.has(SpanStyle::CODE_FENCE)
        || s.has(SpanStyle::QUOTE)
    {
        t.tx2
    } else {
        t.tx
    };
    look.underline = s.has(SpanStyle::LINK_TEXT);
    if s.has(SpanStyle::CODE) {
        look.background = Some(t.raised);
    }
    if s.has(SpanStyle::HIGHLIGHT) {
        look.background = Some(t.ac_soft);
    }
    look
}

/// One `TextRun` for a piece of text. The marked (IME) range is underlined in the text colour.
fn make_run(look: &Look, len: usize, ime_underline: bool, t: &ActiveTheme) -> TextRun {
    let mut f = font(fonts::face_name(look.family, t.kind));
    f.weight = look.weight;
    if look.italic {
        f.style = FontStyle::Italic;
    }
    let color: Hsla = look.color.into();
    let underline = if ime_underline {
        Some(UnderlineStyle {
            color: Some(color),
            thickness: px(1.0),
            wavy: false,
        })
    } else if look.underline {
        Some(UnderlineStyle {
            color: Some(t.ac.into()),
            thickness: px(1.0),
            wavy: false,
        })
    } else {
        None
    };
    TextRun {
        len,
        font: f,
        color,
        background_color: look.background.map(Into::into),
        underline,
        strikethrough: look.strike.then(|| StrikethroughStyle {
            thickness: px(1.0),
            color: None,
        }),
    }
}

/// Text runs for one logical line, from its style segments (which tile the line). The marked
/// (IME) range is underlined. Without segments the line is one plain run.
fn line_runs(
    t: &ActiveTheme,
    segs: &[Segment],
    marked: Option<&Range<usize>>,
    line_start: usize,
    line_len: usize,
) -> Vec<TextRun> {
    let pieces = map::line_pieces(
        line_start..line_start + line_len,
        segs,
        &[],
        &(0..0),
        marked,
        false,
    );
    runs_for_pieces(t, &pieces)
}

/// Text runs for the display text of a line's pieces: one run per piece that shows text. The
/// runs tile the display text, and the IME range is underlined.
fn runs_for_pieces(t: &ActiveTheme, pieces: &[map::Piece]) -> Vec<TextRun> {
    pieces
        .iter()
        .filter_map(|p| {
            let len = map::display_len(p);
            (len > 0).then(|| make_run(&look_for(p.style, t), len, p.ime, t))
        })
        .collect()
}

/// Cache key of a shaped line: its display text, its runs and the metrics it was shaped with.
fn line_key(
    disp: &str,
    pieces: &[map::Piece],
    font_size: f32,
    wrap_w: f32,
    theme_salt: u64,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    disp.hash(&mut h);
    theme_salt.hash(&mut h);
    font_size.to_bits().hash(&mut h);
    wrap_w.to_bits().hash(&mut h);
    for p in pieces {
        let len = map::display_len(p);
        if len > 0 {
            (len, p.style.bits(), p.style.heading, p.ime).hash(&mut h);
        }
    }
    h.finish()
}

/// Hash of the colours and the theme kind (fonts differ between the paper and ink themes).
fn theme_salt(t: &ActiveTheme) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in [
        t.tx, t.tx2, t.tx3, t.ac, t.ac_soft, t.raised, t.sheet, t.line,
    ] {
        [c.r, c.g, c.b, c.a].map(f32::to_bits).hash(&mut h);
    }
    std::mem::discriminant(&t.kind).hash(&mut h);
    h.finish()
}

/// The inputs of the lines of one frame: the document, the view settings and the reveal state of
/// every record. The next frame reuses the lines of this one where nothing changed.
struct FrameKey {
    /// The document text of the frame: the next frame compares its text with this one.
    text: String,
    live: bool,
    wrap_w: u32,
    theme: u64,
    /// Footnote numbering the lines were built with (see `footnote::numbering_salt`).
    notes: u64,
    revealed: Vec<bool>,
    marked: Option<Range<usize>>,
    /// Source range of every reveal record, and a signature of the styling of every logical line.
    hide_src: Vec<Range<usize>>,
    sigs: Vec<u64>,
}

/// The caches of shaped lines and of block widgets, lent to the units built in one frame.
struct UnitCaches<'a> {
    lines: &'a mut HashMap<u64, (usize, Option<WrappedLine>)>,
    /// Built widgets with the source offset they were built at (their offsets are moved on reuse).
    widgets: &'a mut HashMap<u64, (f32, Widget, usize)>,
}

/// What a line is built from, shared by the lines of one frame.
struct LineCtx<'a> {
    text: &'a str,
    theme: &'a ActiveTheme,
    theme_key: u64,
    wrap_w: f32,
    sel: &'a Range<usize>,
    marked: Option<&'a Range<usize>>,
    live: bool,
    /// Read mode: blank lines are taller (see `READ_PARA_GAP`).
    read_only: bool,
    /// All records of the document (sorted by start).
    hides: &'a [map::Hide],
    /// The vault data of the live view, and the path of the note shown (see `context`).
    vault: Option<&'a context::LiveHandle>,
    note_path: &'a str,
    /// Images that the widgets of this frame need decoded (started after the frame is built).
    pending: &'a RefCell<Vec<context::Pending>>,
    /// Logical lines `(start, len)`, the styling of each line (None: unstyled), and the reveal state
    /// of every record.
    split: &'a [(usize, usize)],
    styles: Option<&'a Vec<Vec<Segment>>>,
    revealed: &'a [bool],
    /// Fold key and start state of each callout by block start, and the folds toggled by clicks.
    callout_keys: &'a HashMap<usize, (u64, bool)>,
    folds: &'a HashMap<u64, bool>,
    /// Column width (f32 bits) and the vault generation the widgets are checked against.
    wrap_bits: u32,
    vault_gen: Option<u64>,
}

/// Builds the unit `unit` (a placeholder of this frame) from the document: a block widget, or a
/// text line shaped at the column width. A widget comes from the cache when the same block was
/// built before, moved to its new offsets.
fn build_unit(
    window: &Window,
    ctx: &LineCtx,
    caches: &mut UnitCaches,
    unit: &LineLayout,
) -> LineLayout {
    let (start, len) = (unit.start, unit.len);
    if ctx.live
        && let Some(idx) = widget_record(ctx.hides, start, ctx.revealed)
    {
        let h = &ctx.hides[idx];
        if let map::Deco::Block(kind) = h.deco {
            let block_len = h.src.end - start;
            let fold = match kind {
                map::BlockKind::Callout => {
                    let (key, default_open) =
                        ctx.callout_keys.get(&start).copied().unwrap_or((0, true));
                    (key, ctx.folds.get(&key).copied().unwrap_or(default_open))
                }
                _ => (0, false),
            };
            let block = ctx.text.get(start..start + block_len).unwrap_or("");
            let key = widget_key(
                kind,
                block,
                fold,
                ctx.wrap_bits,
                ctx.theme_key,
                ctx.read_only,
            );
            let cached = caches
                .widgets
                .get(&key)
                .filter(|(_, w, _)| widget_fresh(w, ctx.vault_gen))
                .cloned();
            if let Some((height, mut w, built_at)) = cached {
                shift_widget(&mut w, start as isize - built_at as isize);
                return LineLayout {
                    start,
                    len: block_len,
                    top: 0.0,
                    rows: 1,
                    line_h: height,
                    wrapped: None,
                    pieces: Vec::new(),
                    widget: Some(Box::new(w)),
                    inline: Vec::new(),
                    built: true,
                };
            }
            let unit = build_widget(window, ctx, start, block_len, kind, fold);
            if let Some(w) = unit.widget.as_ref() {
                if caches.widgets.len() >= WIDGET_CACHE_MAX {
                    caches.widgets.clear();
                }
                caches
                    .widgets
                    .insert(key, (unit.line_h, (**w).clone(), start));
            }
            return unit;
        }
    }
    let li = ctx.split.partition_point(|&(s, _)| s < start);
    let segs: &[Segment] = ctx
        .styles
        .and_then(|s| s.get(li))
        .map_or(&[], |v| v.as_slice());
    build_line(window, ctx, caches.lines, start, len, segs)
}

/// Estimated visual rows of a text line that is not built yet: its characters at the body size
/// against the column width (Latin letters are narrower than CJK).
fn estimate_rows(text: &str, font_size: f32, wrap_w: f32) -> usize {
    let width: f32 = text
        .chars()
        .map(|c| {
            if c.is_ascii() {
                font_size * 0.55
            } else {
                font_size
            }
        })
        .sum();
    ((width / wrap_w.max(1.0)).ceil() as usize).max(1)
}

/// Estimated height of a block widget that is not built yet, from its kind and source lines.
fn widget_estimate(kind: map::BlockKind, block: &str) -> f32 {
    let lines = block.matches('\n').count() + 1;
    match kind {
        map::BlockKind::Code => {
            CODE_HEADER_H
                + 2.0 * CODE_PAD_Y
                + lines as f32 * fonts::current_metrics().code_row
                + CODE_GAP
        }
        map::BlockKind::Table => {
            lines as f32 * TABLE_LINE_H * fonts::current_metrics().k + CODE_GAP
        }
        map::BlockKind::Frontmatter => FRONTMATTER_GAP,
        _ => 120.0,
    }
}

/// Signature of the styling of each logical line: a line styled the same way in two frames gets
/// the same value. Empty styling (no styles) gives zeros.
fn style_signatures(styles: Option<&Vec<Vec<Segment>>>, split: &[(usize, usize)]) -> Vec<u64> {
    use std::hash::{Hash, Hasher};
    let Some(styles) = styles else {
        return vec![0; split.len()];
    };
    // Offsets are taken from the start of each line, so a line that only moved keeps its signature.
    split
        .iter()
        .enumerate()
        .map(|(i, &(line_start, _))| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            if let Some(segs) = styles.get(i) {
                for s in segs {
                    (
                        s.range.start.saturating_sub(line_start),
                        s.range.end.saturating_sub(line_start),
                        s.style.bits(),
                    )
                        .hash(&mut h);
                }
            }
            h.finish()
        })
        .collect()
}

/// Builds one logical line: its pieces (what is shown) and its shaped text. The shaped text comes
/// from `cache` when the same display text was shaped before. `top` is set by the caller.
fn build_line(
    window: &Window,
    ctx: &LineCtx,
    cache: &mut HashMap<u64, (usize, Option<WrappedLine>)>,
    start: usize,
    len: usize,
    segs: &[Segment],
) -> LineLayout {
    let end = start + len;
    let (font_size, line_h) = line_metrics(segs);
    // Records that touch this line (sorted, so a binary search finds them).
    let (lo, hi) = if ctx.live {
        (
            ctx.hides.partition_point(|h| h.src.end <= start),
            ctx.hides.partition_point(|h| h.src.start < end),
        )
    } else {
        (0, 0)
    };
    let mut pieces = map::line_pieces(
        start..end,
        segs,
        &ctx.hides[lo..hi.max(lo)],
        ctx.sel,
        ctx.marked,
        ctx.live,
    );
    let inline = if ctx.live {
        inline_math_pass(ctx, &mut pieces, &ctx.hides[lo..hi.max(lo)])
    } else {
        Vec::new()
    };
    // A formula taller than the row makes the row taller (a few px of air above and below).
    let line_h = inline.iter().fold(line_h, |h, s| h.max(s.css.1 + 4.0));
    let line_h = if ctx.read_only && len == 0 {
        line_h + READ_PARA_GAP
    } else {
        line_h
    };
    let disp = map::display_text(&pieces, ctx.text);
    let (rows, wrapped) = if disp.is_empty() {
        (1, None)
    } else {
        let key = line_key(&disp, &pieces, font_size, ctx.wrap_w, ctx.theme_key);
        match cache.get(&key) {
            Some(hit) => hit.clone(),
            None => {
                let runs = runs_for_pieces(ctx.theme, &pieces);
                let shaped = window.text_system().shape_text(
                    SharedString::from(disp),
                    px(font_size),
                    &runs,
                    Some(px(ctx.wrap_w)),
                    None,
                );
                let value = match shaped.ok().and_then(|v| v.into_iter().next()) {
                    Some(w) => (w.wrap_boundaries.len() + 1, Some(w)),
                    None => (1, None),
                };
                cache.insert(key, value.clone());
                value
            }
        }
    };
    LineLayout {
        start,
        len,
        top: 0.0,
        rows,
        line_h,
        wrapped,
        pieces,
        widget: None,
        inline,
        built: true,
    }
}

/// The LaTeX of an inline formula's source (`$...$`), `None` when the source is not one.
fn inline_latex(src: &str) -> Option<String> {
    jiyue_math::find_math_spans(src)
        .into_iter()
        .next()
        .filter(|m| !m.display && !m.latex.is_empty())
        .map(|m| m.latex)
}

/// Draws the inline formulas of a line that the caret is not in (see `map::Deco::InlineMath`). A
/// ready formula replaces its source pieces: the first piece reserves blank cells for the image
/// width and the other pieces of the record are hidden. A formula that is loading or refused keeps
/// its source. The formula is queued for rendering the first time it is asked for.
fn inline_math_pass(
    ctx: &LineCtx,
    pieces: &mut [map::Piece],
    hides: &[map::Hide],
) -> Vec<InlineShot> {
    let mut shots = Vec::new();
    let Some(vault) = ctx.vault else {
        return shots;
    };
    for h in hides.iter().filter(|h| h.deco == map::Deco::InlineMath) {
        let Some(first) = pieces
            .iter()
            .position(|p| p.src.start == h.src.start && p.deco == map::Deco::InlineMath)
        else {
            continue;
        };
        let Some(latex) = inline_latex(ctx.text.get(h.src.clone()).unwrap_or("")) else {
            continue;
        };
        let t = ctx.theme;
        let colour = embed::hex_colour(t.tx.r, t.tx.g, t.tx.b);
        let key: context::MathKey = (
            latex,
            colour,
            (fonts::current_metrics().font * 10.0).round() as u32,
            false,
        );
        let state = {
            let mut v = vault.borrow_mut();
            v.queue_math(key.clone());
            v.math_state(&key)
        };
        if let Some(context::MathState::Ready {
            image,
            css,
            baseline,
        }) = state
        {
            let cells = (css.0 / (fonts::current_metrics().font * 0.5))
                .ceil()
                .max(1.0) as usize;
            for p in pieces
                .iter_mut()
                .filter(|p| p.src.start >= h.src.start && p.src.end <= h.src.end)
            {
                p.replace = Some("");
            }
            pieces[first].gap = cells;
            shots.push(InlineShot {
                src: h.src.start,
                css,
                baseline,
                image,
            });
        }
    }
    shots
}

/// Takes the previous frame's unit that starts at `start`. Units are in document order, so a
/// cursor walks them once per frame (no hashing). Units passed over are left in place.
fn take_prev(prev: &mut [LineLayout], cursor: &mut usize, start: usize) -> Option<LineLayout> {
    while *cursor < prev.len() && prev[*cursor].start < start {
        *cursor += 1;
    }
    if *cursor < prev.len() && prev[*cursor].start == start {
        *cursor += 1;
        return Some(std::mem::replace(
            &mut prev[*cursor - 1],
            placeholder(0, 0, 1, 0.0),
        ));
    }
    None
}

/// Plans the splice of an edit (see `prepaint`): the old units `a..b` overlap the change and are
/// replaced by the lines `lo..hi` of the new text. Returns `None` when a line outside that region
/// changed, a reveal record crosses its edge, or the styling of a line outside it differs, so the
/// whole document is walked instead.
#[allow(clippy::too_many_arguments)]
fn plan_splice(
    s: &invalidate::EditSpan,
    prev: &[LineLayout],
    split: &[(usize, usize)],
    hide_src: &[Range<usize>],
    dirty: &[bool],
    prev_sigs: &[u64],
    sigs: &[u64],
    line_delta: isize,
) -> Option<(usize, usize, usize, usize)> {
    let a = prev.partition_point(|l| l.start + l.len < s.prefix);
    let b = prev.partition_point(|l| l.start <= s.old_end);
    if a > b {
        return None;
    }
    let s0 = if a < prev.len() {
        prev[a].start.min(s.prefix)
    } else {
        s.prefix
    };
    let lo = split.partition_point(|&(st, _)| st <= s0).saturating_sub(1);
    let hi = if b < prev.len() {
        let ns = invalidate::shift(prev[b].start, s.delta());
        let i = split.partition_point(|&(st, _)| st < ns);
        if split.get(i).map(|x| x.0) != Some(ns) {
            return None;
        }
        i
    } else {
        split.len()
    };
    if lo > hi || hi > split.len() {
        return None;
    }
    if dirty[..lo].iter().chain(&dirty[hi..]).any(|&d| d) {
        return None;
    }
    let styles_same = (0..lo).all(|li| prev_sigs.get(li) == sigs.get(li))
        && (hi..split.len()).all(|li| {
            usize::try_from(li as isize - line_delta)
                .ok()
                .is_some_and(|old| prev_sigs.get(old) == sigs.get(li))
        });
    if !styles_same {
        return None;
    }
    // A reveal record that starts in the region must end in it.
    let region_start = split.get(lo).map_or(0, |x| x.0);
    let region_end = split.get(hi).map_or(usize::MAX, |x| x.0);
    if hide_src
        .iter()
        .any(|r| r.start >= region_start && r.start < region_end && r.end >= region_end)
    {
        return None;
    }
    Some((a, b, lo, hi))
}

/// A unit that is not built: its start and length, and the estimated rows and row height.
fn placeholder(start: usize, len: usize, rows: usize, line_h: f32) -> LineLayout {
    LineLayout {
        start,
        len,
        top: 0.0,
        rows,
        line_h,
        wrapped: None,
        pieces: Vec::new(),
        widget: None,
        inline: Vec::new(),
        built: false,
    }
}

/// Sets the top of every unit from the heights of the units before it.
fn place_lines(lines: &mut [LineLayout]) {
    let mut top = 0.0f32;
    for l in lines.iter_mut() {
        l.top = top;
        top += l.height();
    }
}

/// Number of newlines in `text` (the line count less one).
fn newline_count(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count()
}

/// How far the window reaches past the view, in view heights, above and below it.
const WINDOW_MARGIN: f32 = 1.5;

/// Builds the placeholders of the window around `scroll` (the view and `WINDOW_MARGIN` of a view
/// above and below it) and the `forced` units. The tops must be current. Returns how many units
/// were built.
fn build_window(
    window: &Window,
    ctx: &LineCtx,
    caches: &mut UnitCaches,
    lines: &mut [LineLayout],
    forced: &[usize],
    scroll: f32,
    view_h: f32,
) -> usize {
    let w_top = scroll - view_h * WINDOW_MARGIN;
    let w_bottom = scroll + view_h * (1.0 + WINDOW_MARGIN);
    let first = lines.partition_point(|l| l.top + l.height() < w_top);
    let end = lines.partition_point(|l| l.top <= w_bottom).max(first);
    let mut built = 0usize;
    for i in (first..end).chain(forced.iter().copied()) {
        if i < lines.len() && !lines[i].built {
            let unit = build_unit(window, ctx, caches, &lines[i]);
            lines[i] = unit;
            built += 1;
        }
    }
    built
}

/// Index of the block-widget record that starts at `start` and is not revealed, if there is one.
/// `revealed` is the reveal state of every record (see `prepaint`).
fn widget_record(hides: &[map::Hide], start: usize, revealed: &[bool]) -> Option<usize> {
    let i = hides.binary_search_by_key(&start, |h| h.src.start).ok()?;
    let is_block = matches!(hides[i].deco, map::Deco::Block(_));
    (is_block && !revealed.get(i).copied().unwrap_or(true)).then_some(i)
}

/// Shapes one line of text with the given runs. `None` when the text system refuses it.
fn shape_one(
    window: &Window,
    text: &str,
    runs: &[TextRun],
    size: f32,
    wrap_w: Option<f32>,
) -> Option<WrappedLine> {
    window
        .text_system()
        .shape_text(
            SharedString::from(text.to_string()),
            px(size),
            runs,
            wrap_w.map(px),
            None,
        )
        .ok()
        .and_then(|v| v.into_iter().next())
}

/// A single-style run list for a label.
fn label_runs(text: &str, color: Rgba, family: &str) -> Vec<TextRun> {
    let mut f = font(family.to_string());
    f.weight = FontWeight::NORMAL;
    vec![TextRun {
        len: text.len(),
        font: f,
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }]
}

/// Builds the unit of a block widget that replaces the source `start..start + len`.
fn build_widget(
    window: &Window,
    ctx: &LineCtx,
    start: usize,
    len: usize,
    kind: map::BlockKind,
    fold: (u64, bool),
) -> LineLayout {
    let block = ctx.text.get(start..start + len).unwrap_or("");
    let (height, widget) = match kind {
        map::BlockKind::Frontmatter => (FRONTMATTER_GAP, Widget::default()),
        map::BlockKind::Code => code_widget(window, ctx, start, block),
        map::BlockKind::Table => table_widget(window, ctx, start, len),
        map::BlockKind::Callout => callout_widget(window, ctx, start, len, fold.0, fold.1),
        map::BlockKind::Image => image_widget(window, ctx, block),
        map::BlockKind::Embed => embed_widget(window, ctx, block),
        map::BlockKind::Math => math_widget(window, ctx, block),
        map::BlockKind::Mermaid => diagram_widget::mermaid_widget(window, ctx, block),
    };
    LineLayout {
        start,
        len,
        top: 0.0,
        rows: 1,
        line_h: height,
        wrapped: None,
        pieces: Vec::new(),
        widget: Some(Box::new(widget)),
        inline: Vec::new(),
        built: true,
    }
}

/// The panel of a fenced code block: a header with the language and the copy button, then the
/// body with syntax colours, wrapped at the panel width. Returns the unit height (panel and gap)
/// and what to paint, relative to the unit top.
fn code_widget(window: &Window, ctx: &LineCtx, start: usize, block: &str) -> (f32, Widget) {
    let (lang, body) = blocks::split_fenced(block);
    let body_text = block.get(body.clone()).unwrap_or("");
    // The copy button copies the body, as source offsets of the document.
    let body_src = start + body.start..start + body.end;
    let inner_w = (ctx.wrap_w - 2.0 * CODE_PAD_X).max(1.0);
    let mut texts: Vec<WidgetText> = Vec::new();
    let mut y = CODE_HEADER_H + CODE_PAD_Y;
    if !body_text.is_empty() {
        for line in body_text.split('\n') {
            if line.is_empty() {
                y += fonts::current_metrics().code_row;
                continue;
            }
            let runs = highlight::line_runs(line, ctx.theme);
            match shape_one(
                window,
                line,
                &runs,
                fonts::current_metrics().code_live,
                Some(inner_w),
            ) {
                Some(wl) => {
                    let rows = wl.wrap_boundaries.len() + 1;
                    texts.push(plain_text(
                        CODE_PAD_X,
                        y,
                        wl,
                        fonts::current_metrics().code_row,
                    ));
                    y += rows as f32 * fonts::current_metrics().code_row;
                }
                None => y += fonts::current_metrics().code_row,
            }
        }
    }
    let panel_h = y + CODE_PAD_Y;
    if !lang.is_empty() {
        let label = label_runs(&lang, ctx.theme.tx3, &fonts::code_name());
        if let Some(wl) = shape_one(window, &lang, &label, 12.0, None) {
            texts.push(plain_text(
                CODE_PAD_X,
                (CODE_HEADER_H - 16.0) / 2.0,
                wl,
                16.0,
            ));
        }
    }
    let (bw, bh) = CODE_BUTTON;
    let bx = ctx.wrap_w - bw - 10.0;
    let by = (CODE_HEADER_H - bh) / 2.0;
    let copy = label_runs("复制", ctx.theme.tx2, &fonts::ui_name());
    if let Some(wl) = shape_one(window, "复制", &copy, 12.0, None) {
        texts.push(plain_text(bx + 12.0, by + 4.0, wl, 16.0));
    }
    (
        panel_h + CODE_GAP,
        Widget {
            panel: Some((0.0, 0.0, ctx.wrap_w, panel_h)),
            texts,
            button: Some(((bx, by, bw, bh), body_src)),
            ..Widget::default()
        },
    )
}

/// The grid of a GFM table block that replaces the source `start..start + len`: a rounded panel,
/// a header row on a tint, grid lines and the shaped cells. Columns share the width of the column
/// (see `table::allocate_widths`); a cell's text wraps inside its column. Returns the unit height
/// (grid and gap) and what to paint, relative to the unit top.
fn table_widget(window: &Window, ctx: &LineCtx, start: usize, len: usize) -> (f32, Widget) {
    let Some(grid) = table::parse(ctx.text, start..start + len) else {
        return (FRONTMATTER_GAP, Widget::default());
    };
    let t = ctx.theme;
    let cols = grid.aligns.len();
    // Each cell as pieces (its inline styling, header bold) with the shown text and the natural
    // (unwrapped) width. Pieces are built with no caret in the table, so nothing in it is raw.
    let untouched = usize::MAX..usize::MAX;
    let mut shown: Vec<Vec<(Vec<map::Piece>, String, f32)>> = Vec::with_capacity(grid.rows.len());
    for (r, row) in grid.rows.iter().enumerate() {
        let mut line = Vec::with_capacity(cols);
        for cell in row {
            let pieces = table::cell_pieces(ctx.text, cell.clone(), &untouched, r == 0);
            let disp = map::display_text(&pieces, ctx.text);
            let natural = if disp.is_empty() {
                0.0
            } else {
                match shape_one(
                    window,
                    &disp,
                    &runs_for_pieces(t, &pieces),
                    TABLE_FONT_SIZE * fonts::current_metrics().k,
                    None,
                ) {
                    Some(wl) => f32::from(wl.unwrapped_layout.width),
                    None => {
                        table::text_units(&disp)
                            * TABLE_FONT_SIZE
                            * fonts::current_metrics().k
                            * 0.6
                    }
                }
            };
            line.push((pieces, disp, natural));
        }
        shown.push(line);
    }
    let mut natural_cols = vec![0.0f32; cols];
    for row in &shown {
        for (c, (_, _, w)) in row.iter().enumerate() {
            natural_cols[c] = natural_cols[c].max(w + 2.0 * TABLE_PAD_X);
        }
    }
    let widths = table::allocate_widths(&natural_cols, ctx.wrap_w, TABLE_MIN_COL);
    let total_w: f32 = widths.iter().sum();

    // Wrap every cell at its column width; a row is as tall as its tallest cell.
    let mut rows_out: Vec<(usize, Vec<Option<WrappedLine>>)> = Vec::with_capacity(shown.len());
    for row in &shown {
        let mut max_rows = 1usize;
        let mut wrapped = Vec::with_capacity(cols);
        for (c, (pieces, disp, _)) in row.iter().enumerate() {
            let inner = (widths[c] - 2.0 * TABLE_PAD_X).max(1.0);
            let line = if disp.is_empty() {
                None
            } else {
                shape_one(
                    window,
                    disp,
                    &runs_for_pieces(t, pieces),
                    TABLE_FONT_SIZE * fonts::current_metrics().k,
                    Some(inner),
                )
            };
            let rows = line.as_ref().map_or(1, |w| w.wrap_boundaries.len() + 1);
            max_rows = max_rows.max(rows);
            wrapped.push(line);
        }
        rows_out.push((max_rows, wrapped));
    }

    let mut texts: Vec<WidgetText> = Vec::new();
    let mut fills: Vec<((f32, f32, f32, f32), Rgba)> = Vec::new();
    let mut cells: Vec<CellHit> = Vec::new();
    let mut y = 0.0f32;
    for (r, (max_rows, wrapped)) in rows_out.into_iter().enumerate() {
        let row_h = max_rows as f32 * TABLE_LINE_H * fonts::current_metrics().k + 2.0 * TABLE_PAD_Y;
        if r == 0 {
            fills.push(((0.0, 0.0, total_w, row_h), t.hover));
        } else {
            fills.push(((0.0, y, total_w, 1.0), t.line));
        }
        let mut x = 0.0f32;
        for (c, line) in wrapped.into_iter().enumerate() {
            let cw = widths[c];
            if c > 0 {
                fills.push(((x, y, 1.0, row_h), t.line));
            }
            let (tx, ty) = (x + TABLE_PAD_X, y + TABLE_PAD_Y);
            let inner = (cw - 2.0 * TABLE_PAD_X).max(1.0);
            let (align, align_w) = match table::align_of(&grid, c) {
                table::Align::Left => (TextAlign::Left, None),
                table::Align::Center => (TextAlign::Center, Some(inner)),
                table::Align::Right => (TextAlign::Right, Some(inner)),
            };
            if let Some(wl) = line.clone() {
                texts.push(WidgetText {
                    x: tx,
                    y: ty,
                    line: wl,
                    lh: TABLE_LINE_H * fonts::current_metrics().k,
                    align,
                    align_w,
                });
            }
            cells.push(CellHit {
                rect: (x, y, cw, row_h),
                src: grid.rows[r][c].clone(),
                pieces: shown[r][c].0.clone(),
                wrapped: line,
                text_x: tx,
                text_y: ty,
            });
            x += cw;
        }
        y += row_h;
    }
    (
        y + CODE_GAP,
        Widget {
            panel: Some((0.0, 0.0, total_w, y)),
            fills,
            texts,
            cells,
            ..Widget::default()
        },
    )
}

/// The colour of a callout tone. Blue and green are theme tokens; amber and red are fixed per
/// theme kind (dark or light) so that they stay legible on the note sheet.
fn callout_color(t: &ActiveTheme, tone: callout::Tone) -> Rgba {
    let dark = matches!(t.mode, ThemeMode::Dark);
    match tone {
        callout::Tone::Blue => t.ac,
        callout::Tone::Green => t.ok,
        callout::Tone::Neutral => t.tx2,
        callout::Tone::Amber => rgb(if dark { 0xE0A458 } else { 0xA8581A }),
        callout::Tone::Red => rgb(if dark { 0xEF7D74 } else { 0xB3322B }),
    }
}

/// Text runs of one label in a given weight and colour.
fn styled_runs(text: &str, color: Rgba, family: &str, weight: FontWeight) -> Vec<TextRun> {
    let mut f = font(family.to_string());
    f.weight = weight;
    vec![TextRun {
        len: text.len(),
        font: f,
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }]
}

/// The panel of a callout: a tinted rounded box with a bar on the left, the icon and the title
/// row (a click on it folds the callout when it is foldable), then the body lines. A closed
/// callout is its title row only. Returns the unit height (panel and gap) and what to paint.
fn callout_widget(
    window: &Window,
    ctx: &LineCtx,
    start: usize,
    len: usize,
    key: u64,
    open: bool,
) -> (f32, Widget) {
    let Some(c) = callout::parse(ctx.text, start..start + len) else {
        return (FRONTMATTER_GAP, Widget::default());
    };
    let t = ctx.theme;
    let tone = callout_color(t, c.tone);
    let tint = Rgba { a: 0.10, ..tone };
    let title = match &c.title {
        Some(r) => ctx.text.get(r.clone()).unwrap_or("").to_string(),
        None => callout::default_title(&c.kind),
    };
    let mut texts: Vec<WidgetText> = Vec::new();
    let mut icons: Vec<Icon> = Vec::new();
    let title_top = (CALLOUT_TITLE_H - CALLOUT_ICON) / 2.0;
    icons.push(((12.0, title_top, CALLOUT_ICON, CALLOUT_ICON), c.icon, tone));
    if let Some(wl) = shape_one(
        window,
        &title,
        &styled_runs(&title, tone, &fonts::ui_name(), FontWeight::SEMIBOLD),
        CALLOUT_TEXT_SIZE * fonts::current_metrics().k,
        None,
    ) {
        texts.push(plain_text(
            CALLOUT_TEXT_X,
            (CALLOUT_TITLE_H - 20.0) / 2.0,
            wl,
            20.0,
        ));
    }
    let foldable = c.fold != callout::Fold::Fixed;
    if foldable {
        let chevron = if open {
            "chevron-down"
        } else {
            "chevron-right"
        };
        let x = ctx.wrap_w - 12.0 - CALLOUT_ICON;
        icons.push((
            (x, title_top, CALLOUT_ICON, CALLOUT_ICON),
            Some(chevron),
            t.tx3,
        ));
    }
    let mut y = CALLOUT_TITLE_H;
    if open {
        y += CALLOUT_PAD_Y;
        let inner = (ctx.wrap_w - CALLOUT_TEXT_X - 12.0).max(1.0);
        for line in &c.body {
            let pieces = callout::body_pieces(ctx.text, line.clone());
            let disp = map::display_text(&pieces, ctx.text);
            if disp.is_empty() {
                y += CALLOUT_BODY_LH * fonts::current_metrics().k * 0.5;
                continue;
            }
            let runs = runs_for_pieces(t, &pieces);
            match shape_one(
                window,
                &disp,
                &runs,
                CALLOUT_BODY_SIZE * fonts::current_metrics().k,
                Some(inner),
            ) {
                Some(wl) => {
                    let rows = wl.wrap_boundaries.len() + 1;
                    texts.push(plain_text(
                        CALLOUT_TEXT_X,
                        y,
                        wl,
                        CALLOUT_BODY_LH * fonts::current_metrics().k,
                    ));
                    y += rows as f32 * CALLOUT_BODY_LH * fonts::current_metrics().k;
                }
                None => y += CALLOUT_BODY_LH * fonts::current_metrics().k,
            }
        }
        y += CALLOUT_PAD_Y;
    }
    (
        y + CODE_GAP,
        Widget {
            panel: Some((0.0, 0.0, ctx.wrap_w, y)),
            panel_fill: Some(tint),
            fills: vec![((0.0, 0.0, CALLOUT_BAR_W, y), tone)],
            texts,
            icons,
            callout: foldable.then_some(CalloutHit {
                title_h: CALLOUT_TITLE_H,
                key,
                open,
            }),
            open,
            ..Widget::default()
        },
    )
}

/// A card that says what is drawn instead of a picture or a note: an icon, a line of text and a
/// detail line. Its unit is the card height plus the picture margins.
fn card(
    window: &Window,
    ctx: &LineCtx,
    icon: &'static str,
    title: &str,
    detail: &str,
) -> (f32, Widget) {
    let t = ctx.theme;
    let mut texts: Vec<WidgetText> = Vec::new();
    if let Some(wl) = shape_one(
        window,
        title,
        &label_runs(title, t.tx2, &fonts::ui_name()),
        13.5,
        None,
    ) {
        texts.push(plain_text(42.0, 12.0, wl, 18.0));
    }
    if let Some(wl) = shape_one(
        window,
        detail,
        &label_runs(detail, t.tx3, &fonts::ui_name()),
        12.0,
        None,
    ) {
        texts.push(plain_text(42.0, 32.0, wl, 16.0));
    }
    (
        IMG_TOP + CARD_H + IMG_GAP,
        Widget {
            panel: Some((0.0, IMG_TOP, ctx.wrap_w, CARD_H)),
            texts,
            icons: vec![(
                (14.0, IMG_TOP + (CARD_H - 18.0) / 2.0, 18.0, 18.0),
                Some(icon),
                t.tx3,
            )],
            ..Widget::default()
        },
    )
}

/// The unit of an image line: the picture at its display size under its caption. Until the
/// decode ends the same box is drawn empty (its height is known from the file index, so nothing
/// moves when the picture arrives). A remote image, a missing file and an unreadable one get a
/// card that says so.
fn image_widget(window: &Window, ctx: &LineCtx, block: &str) -> (f32, Widget) {
    let Some(e) = embed::line_embed(block) else {
        return card(window, ctx, "file-text", "无法显示图片", block);
    };
    if e.class == embed::EmbedClass::Remote {
        return card(window, ctx, "link", "外部图片不加载", &e.target);
    }
    let Some(vault) = ctx.vault else {
        return card(window, ctx, "file-text", "无法显示图片", &e.target);
    };
    let resolved = vault.borrow().resolve(ctx.note_path, &e.target);
    let Some(rel) = resolved else {
        let mut w = card(
            window,
            ctx,
            "file-text",
            &format!("找不到图片：{}", basename(&e.target)),
            &e.target,
        );
        w.1.vault_gen = Some(vault.borrow().data_gen);
        return w;
    };
    let mtime = context::modified(&vault.borrow().abs(&rel));
    let (built_gen, natural, ready, loading, failed) = {
        let v = vault.borrow();
        let slot = v.slot(&rel);
        let stale = slot.is_none_or(|s| s.mtime != mtime);
        if stale {
            ctx.pending.borrow_mut().push(context::Pending {
                rel: rel.clone(),
                abs: v.abs(&rel),
                mtime,
            });
        }
        let state = slot.filter(|_| !stale).map(|s| &s.state);
        let ready = match state {
            Some(context::ImageState::Ready(img)) => Some(img.clone()),
            _ => None,
        };
        (
            v.data_gen,
            v.natural_size(&rel),
            ready,
            stale || matches!(state, Some(context::ImageState::Loading)),
            matches!(state, Some(context::ImageState::Failed)),
        )
    };
    if failed {
        let mut w = card(window, ctx, "file-text", "无法显示图片", &rel);
        w.1.vault_gen = Some(built_gen);
        return w;
    }
    let Some((bw, bh)) = natural.and_then(|n| embed::image_box(n, e.size, ctx.wrap_w)) else {
        let mut w = card(window, ctx, "file-text", "无法显示图片", &rel);
        w.1.vault_gen = Some(built_gen);
        return w;
    };
    let t = ctx.theme;
    let mut texts: Vec<WidgetText> = Vec::new();
    let cap = caption(e.alias.as_deref());
    let cap_h = if cap.is_some() { IMG_CAPTION_H } else { 0.0 };
    if let Some(text) = cap
        && let Some(wl) = shape_one(
            window,
            &text,
            &label_runs(&text, t.tx3, &fonts::ui_name()),
            12.5,
            Some(ctx.wrap_w),
        )
    {
        texts.push(plain_text(0.0, IMG_TOP + bh + 6.0, wl, 18.0));
    }
    let mut w = Widget {
        vault_gen: Some(built_gen),
        loading,
        ..Widget::default()
    };
    match ready {
        Some(img) => w.image = Some(((0.0, IMG_TOP, bw, bh), img)),
        None => {
            // Empty box while decoding: same size as the picture will be.
            w.panel = Some((0.0, IMG_TOP, bw, bh));
            let note = if loading {
                "正在加载图片"
            } else {
                "等待图片"
            };
            if let Some(wl) = shape_one(
                window,
                note,
                &label_runs(note, t.tx3, &fonts::ui_name()),
                12.5,
                None,
            ) {
                texts.push(plain_text(12.0, IMG_TOP + 10.0, wl, 18.0));
            }
        }
    }
    w.texts = texts;
    (IMG_TOP + bh + cap_h + IMG_GAP, w)
}

/// The unit of a note embed line (`![[note]]`, `![[note#Heading]]`, `![[note#^block]]`): a panel
/// with a title bar (the note and the anchor, and a click on it opens the note) and the first
/// lines of the excerpt with their inline markers hidden. A missing note or anchor is a card.
fn embed_widget(window: &Window, ctx: &LineCtx, block: &str) -> (f32, Widget) {
    let Some(e) = embed::line_embed(block) else {
        return card(window, ctx, "file-text", "无法显示笔记", block);
    };
    let Some(vault) = ctx.vault else {
        return card(window, ctx, "file-text", "找不到笔记", &e.target);
    };
    let label = embed::note_link_body(&e.target, e.anchor.as_ref());
    let (built_gen, found) = {
        let v = vault.borrow();
        let found = match v.note_by_title(&e.target) {
            None => Err(format!("找不到笔记：{}", e.target)),
            Some(note) => embed::excerpt(&note.body, e.anchor.as_ref()).map_err(|err| match err {
                embed::ExcerptError::NoHeading => "找不到引用的标题".to_string(),
                embed::ExcerptError::NoBlock => "找不到引用的块".to_string(),
            }),
        };
        (v.data_gen, found)
    };
    let ex = match found {
        Ok(ex) => ex,
        Err(msg) => {
            let mut w = card(window, ctx, "file-text", &msg, &label);
            w.1.vault_gen = Some(built_gen);
            return w;
        }
    };
    let t = ctx.theme;
    let wrap_w = ctx.wrap_w;
    let mut texts: Vec<WidgetText> = Vec::new();
    if let Some(wl) = shape_one(
        window,
        &label,
        &styled_runs(&label, t.tx2, &fonts::ui_name(), FontWeight::MEDIUM),
        12.5,
        None,
    ) {
        texts.push(plain_text(30.0, (EMBED_HEAD_H - 18.0) / 2.0, wl, 18.0));
    }
    if let Some(wl) = shape_one(
        window,
        "打开",
        &label_runs("打开", t.ac, &fonts::ui_name()),
        12.0,
        None,
    ) {
        texts.push(plain_text(
            wrap_w - 44.0,
            (EMBED_HEAD_H - 16.0) / 2.0,
            wl,
            16.0,
        ));
    }
    let mut y = EMBED_HEAD_H + 8.0;
    let inner = (wrap_w - 2.0 * EMBED_PAD_X).max(1.0);
    for pieces in embed::excerpt_pieces(&ex.text) {
        let disp = map::display_text(&pieces, &ex.text);
        if disp.is_empty() {
            y += EMBED_BODY_LH * fonts::current_metrics().k * 0.5;
            continue;
        }
        let runs = runs_for_pieces(t, &pieces);
        match shape_one(
            window,
            &disp,
            &runs,
            EMBED_BODY_SIZE * fonts::current_metrics().k,
            Some(inner),
        ) {
            Some(wl) => {
                let rows = wl.wrap_boundaries.len() + 1;
                texts.push(plain_text(
                    EMBED_PAD_X,
                    y,
                    wl,
                    EMBED_BODY_LH * fonts::current_metrics().k,
                ));
                y += rows as f32 * EMBED_BODY_LH * fonts::current_metrics().k;
            }
            None => y += EMBED_BODY_LH * fonts::current_metrics().k,
        }
    }
    if ex.more {
        if let Some(wl) = shape_one(
            window,
            "……",
            &label_runs("……", t.tx3, &fonts::ui_name()),
            EMBED_BODY_SIZE * fonts::current_metrics().k,
            None,
        ) {
            texts.push(plain_text(
                EMBED_PAD_X,
                y,
                wl,
                EMBED_BODY_LH * fonts::current_metrics().k,
            ));
        }
        y += EMBED_BODY_LH * fonts::current_metrics().k;
    }
    let panel_h = y + 10.0;
    (
        IMG_TOP + panel_h + IMG_GAP,
        Widget {
            panel: Some((0.0, IMG_TOP, wrap_w, panel_h)),
            fills: vec![((0.0, IMG_TOP + EMBED_HEAD_H, wrap_w, 1.0), t.line)],
            texts,
            links: vec![((0.0, IMG_TOP, wrap_w, EMBED_HEAD_H), label)],
            vault_gen: Some(built_gen),
            ..Widget::default()
        },
    )
}

/// The unit of a block formula: the rendered formula centred in the column. While it renders an
/// empty box of one line is drawn (the rendered height replaces it once, when it is ready). A
/// formula the renderer refuses is a card with its source, so the source is never lost.
fn math_widget(window: &Window, ctx: &LineCtx, block: &str) -> (f32, Widget) {
    let t = ctx.theme;
    let Some(latex) = embed::math_latex(block) else {
        return card(window, ctx, "file-text", "公式为空", block);
    };
    let Some(vault) = ctx.vault else {
        return card(window, ctx, "file-text", "公式无法显示", &latex);
    };
    let colour = embed::hex_colour(t.tx.r, t.tx.g, t.tx.b);
    let key: context::MathKey = (
        latex.clone(),
        colour,
        (embed::MATH_FONT_PX * 10.0).round() as u32,
        true,
    );
    let (built_gen, state) = {
        let mut v = vault.borrow_mut();
        v.queue_math(key.clone());
        (v.data_gen, v.math_state(&key))
    };
    const MATH_TOP: f32 = 8.0;
    const MATH_LOADING_H: f32 = 40.0;
    match state {
        Some(context::MathState::Ready { image, css, .. }) => {
            let (w, h) = embed::math_box(css, ctx.wrap_w);
            let x = ((ctx.wrap_w - w) / 2.0).max(0.0);
            (
                MATH_TOP + h + IMG_GAP,
                Widget {
                    image: Some(((x, MATH_TOP, w, h), image)),
                    vault_gen: Some(built_gen),
                    ..Widget::default()
                },
            )
        }
        Some(context::MathState::Failed(msg)) => {
            let mut w = card(
                window,
                ctx,
                "file-text",
                &format!("公式无法显示：{msg}"),
                &latex,
            );
            w.1.vault_gen = Some(built_gen);
            w
        }
        Some(context::MathState::Loading) | None => {
            let mut w = Widget {
                panel: Some((0.0, MATH_TOP, ctx.wrap_w, MATH_LOADING_H)),
                loading: true,
                vault_gen: Some(built_gen),
                ..Widget::default()
            };
            if let Some(wl) = shape_one(
                window,
                "正在渲染公式",
                &label_runs("正在渲染公式", t.tx3, &fonts::ui_name()),
                12.5,
                None,
            ) {
                w.texts.push(plain_text(12.0, MATH_TOP + 12.0, wl, 18.0));
            }
            (MATH_TOP + MATH_LOADING_H + IMG_GAP, w)
        }
    }
}

/// Whether a built widget can be reused as it is: it waits for no decode, and the vault data it
/// was built from is still the current data (`current` is the current `LiveContext::data_gen`).
/// The timing log of the editor's prepaint: when `JIYUE_PREPAINT_LOG` names a file, every frame
/// appends one line (lines, units built, milliseconds). Off by default.
fn prepaint_log_path() -> Option<&'static std::path::Path> {
    static PATH: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var_os("JIYUE_PREPAINT_LOG").map(Into::into))
        .as_deref()
}

thread_local! {
    /// Start of the previous prepaint (for the frame gap in the timing log).
    static LAST_PREPAINT: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Milliseconds since the previous prepaint started (0 for the first one); records `now`.
fn take_frame_gap(now: std::time::Instant) -> f64 {
    LAST_PREPAINT.with(|c| {
        let gap = c.get().map_or(0.0, |p| (now - p).as_secs_f64() * 1000.0);
        c.set(Some(now));
        gap
    })
}

/// Appends one line to the timing log (when `JIYUE_PREPAINT_LOG` is set).
fn log_timing(line: &str) {
    if let Some(path) = prepaint_log_path() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Most block widgets kept in the cache; the cache is dropped when it is full.
const WIDGET_CACHE_MAX: usize = 1024;

/// Cache key of a block widget: its kind, its source text, its fold state, the column width (as
/// `f32` bits), the theme and the read mode. Two units with the same key paint the same.
fn widget_key(
    kind: map::BlockKind,
    block: &str,
    fold: (u64, bool),
    wrap_bits: u32,
    theme: u64,
    read_only: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kind as u8, block, fold, wrap_bits, theme, read_only).hash(&mut h);
    h.finish()
}

fn widget_fresh(w: &Widget, built_gen: Option<u64>) -> bool {
    !w.loading
        && match w.vault_gen {
            None => true,
            Some(g) => built_gen == Some(g),
        }
}

/// The smallest single edit that turns `old` into `new`: the byte range of `old` to replace and
/// the text to put there. Common leading and trailing characters are kept. Ranges fall on char
/// boundaries, and an identical pair gives an empty range.
pub fn minimal_edit(old: &str, new: &str) -> (Range<usize>, String) {
    let mut prefix = 0;
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix += a.len_utf8();
    }
    let max_suffix = old.len().min(new.len()) - prefix;
    let mut suffix = 0;
    for (a, b) in old.chars().rev().zip(new.chars().rev()) {
        if a != b || suffix + a.len_utf8() > max_suffix {
            break;
        }
        suffix += a.len_utf8();
    }
    let end_old = old.len() - suffix;
    let end_new = new.len() - suffix;
    (prefix..end_old, new[prefix..end_new].to_string())
}

/// Whether the pointer shows the pointing-hand cursor. Read mode: any link or clickable widget.
/// Live preview: widgets always (a plain click acts on them), text links only while Ctrl (Cmd on
/// macOS) is held, because a plain click places the caret there. Source mode: never (I-beam).
fn hand_cursor(
    read_only: bool,
    live: bool,
    ctrl: bool,
    over_widget: bool,
    over_link: bool,
) -> bool {
    if read_only {
        return over_widget || over_link;
    }
    if !live {
        return false;
    }
    over_widget || (ctrl && over_link)
}

/// Whether byte `at` lies in a link span: a wikilink, or the text or the URL of a Markdown link.
fn link_span_at(spans: &[md_style::StyleSpan], at: usize) -> bool {
    spans.iter().any(|s| {
        s.range.contains(&at)
            && [
                SpanStyle::LINK_TEXT,
                SpanStyle::LINK_URL,
                SpanStyle::WIKILINK,
            ]
            .iter()
            .any(|&flag| s.style.has(flag))
    })
}

/// Emitted when Ctrl+click lands on a `[[wikilink]]`. The payload is the link body as written
/// between the brackets (`笔记`, `笔记#标题`, `笔记#^id`, `笔记|别名`). The app opens the note and
/// moves the caret to the anchor (see `attach_ui::navigate_link`).
#[derive(Clone, Debug)]
pub struct OpenWikilink(pub String);

impl EventEmitter<OpenWikilink> for TextEditor {}

/// The editor lost keyboard focus (emitted by the focus-out listener set in `render`).
pub struct EditorBlur;

impl EventEmitter<EditorBlur> for TextEditor {}

/// Syntax styling of the document, per logical line. Valid for `text` only: the cache is
/// rebuilt when the document text differs (a content comparison, which is cheaper than the
/// recompute and needs no version bookkeeping in every edit path).
struct StyleCache {
    text: String,
    segments: Vec<Vec<Segment>>,
    /// Markers that the live view hides or replaces, sorted (see `map::compute_hides`).
    hides: Vec<map::Hide>,
    /// The style spans of `text` (kept to update them after the next edit, see `refresh_styles`).
    spans: Vec<md_style::StyleSpan>,
    /// Footnote numbering of `hides` (see `footnote::numbering_salt`).
    notes: u64,
}

/// The editor entity: document, caret and selection, IME state, scroll and the last layout.
pub struct TextEditor {
    state: EditorState,
    focus_handle: FocusHandle,
    theme: ActiveTheme,
    /// Scroll offset in pixels (0 = top of the padding).
    scroll_y: f32,
    /// Scroll so that the caret is visible at the next paint.
    reveal: bool,
    /// IME marked text, in UTF-8 bytes of the live document.
    marked_range: Option<Range<usize>>,
    composition: Option<Composition>,
    layout: Layout,
    /// Mouse drag in progress, from `drag_anchor`.
    selecting: bool,
    drag_anchor: usize,
    /// Goal x for Up/Down across visual rows; cleared by every other command.
    goal_x: Option<f32>,
    /// Caret blink phase; forced on after every input or movement.
    caret_on: bool,
    /// Markdown syntax styling in the source view. Off: plain text, one colour.
    styled: bool,
    /// Live preview: markers away from the caret and selection are hidden, and block markers
    /// are replaced (bullets, task boxes, quote bars, rules). Set by the note view per frame.
    live: bool,
    style_cache: Option<StyleCache>,
    /// Shaped lines by `line_key`: the row count and the shaped text (`None` for an empty line).
    line_cache: HashMap<u64, (usize, Option<WrappedLine>)>,
    /// Built block widgets by `widget_key`: their height and what they paint. Reused while the
    /// block text, width, theme, read mode and fold state are the same, so an edit elsewhere in
    /// the note does not rebuild the tables, code panels and callouts.
    widget_cache: HashMap<u64, (f32, Widget, usize)>,
    /// The inputs of the lines of the last frame (see `prepaint`).
    frame_key: Option<FrameKey>,
    /// In-note search matches (byte ranges), painted under the text. Set by the find bar.
    search_matches: Vec<Range<usize>>,
    /// Index into `search_matches` of the current match (accent colour and outline).
    search_current: Option<usize>,
    /// Callout folds toggled by a click, by fold key: `true` open. In memory only (see `callout`).
    folds: HashMap<u64, bool>,
    /// The vault data of the live view (images, note excerpts), set by the app (see `context`).
    vault: Option<context::LiveHandle>,
    /// Vault-relative path of the note shown (image paths resolve from its folder).
    note_path: String,
    /// Read mode: the live view with no raw source (the caret is ignored), a centred wider column
    /// and no typing. Task boxes still toggle and wikilinks open on a plain click.
    read_only: bool,
    /// Pointer is over a link or widget: the pointing-hand cursor (see `hand_cursor`).
    hand: bool,
    /// Keeps the focus-out listener alive (set on the first render).
    focus_out: Option<Subscription>,
}

impl TextEditor {
    /// New editor with the caret at 0. Starts the caret blink timer.
    pub fn new(text: &str, theme: ActiveTheme, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(CARET_BLINK_MS))
                    .await;
                let alive = this.update(cx, |ed, cx| {
                    ed.caret_on = !ed.caret_on;
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        // A freshly opened note with frontmatter starts after it, so the hidden block stays hidden.
        let mut state = EditorState::new(text);
        if let Some(at) = blocks::opening_caret(text) {
            state.set_selection(at, at);
        }

        Self {
            state,
            focus_handle: cx.focus_handle(),
            theme,
            scroll_y: 0.0,
            reveal: false,
            marked_range: None,
            composition: None,
            layout: Layout::default(),
            selecting: false,
            drag_anchor: 0,
            goal_x: None,
            caret_on: true,
            styled: true,
            live: false,
            style_cache: None,
            line_cache: HashMap::new(),
            widget_cache: HashMap::new(),
            frame_key: None,
            search_matches: Vec::new(),
            search_current: None,
            folds: HashMap::new(),
            vault: None,
            note_path: String::new(),
            read_only: false,
            hand: false,
            focus_out: None,
        }
    }

    /// Gives the editor the vault data of the live view (images and note excerpts) and the path of
    /// its note. Set once when the editor is created.
    pub fn set_vault(
        &mut self,
        vault: context::LiveHandle,
        note_path: &str,
        cx: &mut Context<Self>,
    ) {
        self.vault = Some(vault);
        self.note_path = note_path.to_string();
        cx.notify();
    }

    // ----- public helpers for the app -----

    /// The whole document text.
    pub fn text(&self) -> String {
        self.state.text().to_string()
    }

    /// Replaces the document. Clears undo history and the composition.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.composition = None;
        self.marked_range = None;
        self.state.set_text(text);
        self.scroll_y = 0.0;
        self.goal_x = None;
        self.after_edit(cx);
    }

    /// Caret position as (row, column), both 1-based, column counted in characters.
    pub fn cursor_row_col(&self) -> (usize, usize) {
        let (row, col) = self.state.offset_to_row_col(self.state.cursor());
        (row + 1, col + 1)
    }

    pub fn is_dirty(&self) -> bool {
        self.state.is_dirty()
    }

    pub fn mark_saved(&mut self) {
        self.state.mark_saved();
    }

    /// Turns the Markdown syntax styling on or off (on by default). Off shows plain text.
    pub fn set_styled(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.styled != on {
            self.styled = on;
            self.style_cache = None;
            cx.notify();
        }
    }

    /// Rebuilds the style cache when the styling or the live view is on and the text changed.
    /// Documents over `md_style::MAX_STYLED_BYTES` are not styled.
    fn refresh_styles(&mut self) {
        if !(self.styled || self.live) || self.state.len() > md_style::MAX_STYLED_BYTES {
            self.style_cache = None;
            return;
        }
        let text = self.state.text();
        if self.style_cache.as_ref().is_some_and(|c| c.text == text) {
            return;
        }
        // After an edit only the paragraph around it is parsed again (see `compute_spans_after_edit`);
        // anything else is parsed as a whole.
        let spans = match self.style_cache.as_ref() {
            Some(old) => {
                let edit = invalidate::edit_span(&old.text, text);
                md_style::compute_spans_after_edit(&old.spans, text, &edit)
                    .unwrap_or_else(|| md_style::compute_spans(text))
            }
            None => md_style::compute_spans(text),
        };
        let lines = md_style::split_lines(text);
        let segments = md_style::line_segments(&spans, &lines);
        let hides = map::compute_hides(text, &spans);
        let notes = footnote::numbering_salt(&hides);
        self.style_cache = Some(StyleCache {
            text: text.to_string(),
            segments,
            hides,
            spans,
            notes,
        });
    }

    /// Updates the colours. The text, caret and selection are kept.
    pub fn set_theme(&mut self, theme: ActiveTheme, cx: &mut Context<Self>) {
        let changed = self.theme.kind != theme.kind;
        self.theme = theme;
        if changed {
            self.line_cache.clear();
            cx.notify();
        }
    }

    /// Turns the live view on or off (markers hidden away from the caret). The note view sets it
    /// from the view mode; source mode keeps every marker visible.
    pub fn set_live(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.live != on {
            self.live = on;
            cx.notify();
        }
    }

    /// Read mode on or off (see the `read_only` field). Turning it on also ends an IME composition.
    pub fn set_read_only(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.read_only != on {
            self.read_only = on;
            self.commit_composition();
            if on {
                // Read mode opens at the top of the note, not at the caret of the source view.
                self.scroll_y = 0.0;
                self.reveal = false;
            }
            cx.notify();
        }
    }

    /// Gives keyboard focus to the editor.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    /// True while an IME composition is open (the marked text is not committed yet).
    pub fn is_composing(&self) -> bool {
        self.composition.is_some() || self.marked_range.is_some()
    }

    /// Window position of the caret: `(x, y, row height)` with `y` at the top of the caret's row.
    /// None before the first paint.
    pub fn caret_origin(&self) -> Option<(f32, f32, f32)> {
        if self.layout.lines.is_empty() {
            return None;
        }
        let cursor = self.state.cursor();
        let (x, y) = self.layout.point_for_index(cursor);
        Some((
            self.layout.origin_x + x,
            self.layout.origin_y + y,
            self.layout.row_height_at(cursor),
        ))
    }

    // ----- editing core -----

    /// Runs a command on the state: finishes a composition, applies, resets the goal column.
    fn apply(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditorState)) {
        self.commit_composition();
        f(&mut self.state);
        self.goal_x = None;
        self.after_edit(cx);
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.caret_on = true;
        self.reveal = true;
        cx.notify();
    }

    /// True when no IME composition is in progress, so the markdown helpers may run.
    fn can_edit_directly(&self) -> bool {
        self.composition.is_none() && self.marked_range.is_none()
    }

    /// Applies a markdown plan as one undo step, leaving the plan's selection and revealing it.
    /// Callers must have checked `can_edit_directly`.
    fn apply_plan(&mut self, plan: EditPlan, cx: &mut Context<Self>) {
        let sel = plan_selection(&plan);
        self.state
            .replace_range(plan.start, plan.end, &plan.insert, sel);
        self.goal_x = None;
        self.after_edit(cx);
    }

    /// Runs a markdown helper on the current selection when editing is direct, else (or when the
    /// helper declines) runs `fallback` as an ordinary command.
    fn edit_or(
        &mut self,
        cx: &mut Context<Self>,
        make: impl FnOnce(&str, usize, usize) -> Option<EditPlan>,
        fallback: impl FnOnce(&mut EditorState),
    ) {
        if self.read_only {
            return;
        }
        if self.can_edit_directly() {
            let r = self.state.selection().range();
            if let Some(plan) = make(self.state.text(), r.start, r.end) {
                self.apply_plan(plan, cx);
                return;
            }
        }
        self.apply(cx, fallback);
    }

    fn move_head(&mut self, target: usize, extend: bool) {
        let anchor = if extend {
            self.state.selection().anchor
        } else {
            target
        };
        self.state.set_selection(anchor, target);
    }

    /// Up or down by visual rows, keeping the goal x (in document pixels).
    fn vertical(&mut self, rows: i32, extend: bool, cx: &mut Context<Self>) {
        self.commit_composition();
        let (caret_x, caret_y) = self.layout.point_for_index(self.state.cursor());
        let goal = *self.goal_x.get_or_insert(caret_x);
        // Step row by row, each row with its own height (headings are taller).
        let dir = rows.signum();
        let mut y = caret_y + self.layout.row_height_at(self.state.cursor()) / 2.0;
        for _ in 0..rows.unsigned_abs() {
            match self.layout.adjacent_row_centre(y, dir) {
                Some(next) => y = next,
                None => {
                    y = if dir > 0 {
                        self.layout.content_height() + 1.0
                    } else {
                        -1.0
                    };
                    break;
                }
            }
        }
        let target = self.layout.index_at(goal, y, self.state.len());
        self.move_head(target, extend);
        self.after_edit(cx);
    }

    /// Rows that fit in the viewport, for PageUp and PageDown.
    fn page_rows(&self) -> i32 {
        ((self.layout.viewport_h - PAD_TOP - PAD_BOTTOM) / fonts::current_metrics().line)
            .floor()
            .max(1.0) as i32
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let text = self.state.copy();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        self.commit_composition();
        let text = self.state.cut();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        self.goal_x = None;
        self.after_edit(cx);
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(raw) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let text = clipboard_to_text(&raw);
        // Smart paste: a URL over a selection becomes a link; otherwise the normalised text.
        self.edit_or(
            cx,
            |t, a, b| md_edit::on_paste(t, a, b, &raw),
            move |s| s.paste(&text),
        );
    }

    /// Enter, Tab and Backspace: markdown plan first, else the ordinary command.
    fn insert_newline(&mut self, cx: &mut Context<Self>) {
        self.edit_or(cx, md_edit::on_enter, |s| s.insert_text("\n"));
    }

    fn indent(&mut self, shift: bool, cx: &mut Context<Self>) {
        self.edit_or(
            cx,
            move |t, a, b| md_edit::on_tab(t, a, b, shift),
            move |s| {
                if !shift {
                    s.insert_text(TAB_TEXT);
                }
            },
        );
    }

    fn backspace_md(&mut self, cx: &mut Context<Self>) {
        self.edit_or(cx, md_edit::on_backspace, |s| s.backspace());
    }

    /// Flips the task checkbox on the line at byte `offset`. Returns false when that line is not
    /// a task item, or while an IME composition is in progress.
    pub fn toggle_task_at(&mut self, offset: usize, cx: &mut Context<Self>) -> bool {
        if !self.can_edit_directly() {
            return false;
        }
        let Some(plan) = md_edit::toggle_task(self.state.text(), offset) else {
            return false;
        };
        self.apply_plan(plan, cx);
        true
    }

    /// Moves the caret to the target of a footnote jump (see `footnote::jump_target`) and scrolls it
    /// into view. Nothing happens when there is no target (a reference without its definition), or
    /// during an IME composition.
    fn jump_footnote(&mut self, jump: footnote::Jump, cx: &mut Context<Self>) {
        if !self.can_edit_directly() {
            return;
        }
        let target = self
            .style_cache
            .as_ref()
            .and_then(|c| footnote::jump_target(&c.hides, jump));
        if let Some(at) = target {
            self.goal_x = None;
            self.state.set_selection(at, at);
            self.after_edit(cx);
        }
    }

    /// Inserts a footnote at the caret: the reference `[^n]` there and the definition `[^n]: ` at the
    /// end of the note, with `n` the next free number (see `footnote::plan_insert`). One undo step;
    /// the caret goes after the colon of the new definition. Returns false, and changes nothing, in
    /// read mode or during an IME composition. The palette and the slash menu may call it.
    pub fn insert_footnote(&mut self, cx: &mut Context<Self>) -> bool {
        if self.read_only || !self.can_edit_directly() {
            return false;
        }
        let plan = footnote::plan_insert(self.state.text(), self.state.cursor());
        self.set_text_undoable(&plan.text, cx);
        self.state.set_selection(plan.caret, plan.caret);
        self.goal_x = None;
        self.after_edit(cx);
        true
    }

    /// Moves the caret to the start of logical line `row` (zero based) and scrolls it into view.
    /// Does nothing while an IME composition is in progress.
    pub fn set_caret_to_line(&mut self, row: usize, cx: &mut Context<Self>) {
        if !self.can_edit_directly() {
            return;
        }
        let start = self.state.line_range(row).start;
        self.state.set_selection(start, start);
        self.goal_x = None;
        self.after_edit(cx);
    }

    /// Moves the caret to `col` (chars) of logical line `row` (both zero based), clamped to the
    /// document and the line, and scrolls it into view. Does nothing during an IME composition.
    pub fn set_caret_to_row_col(&mut self, row: usize, col: usize, cx: &mut Context<Self>) {
        if !self.can_edit_directly() {
            return;
        }
        let offset = self.state.row_col_to_offset(row, col);
        self.state.set_selection(offset, offset);
        self.goal_x = None;
        self.after_edit(cx);
    }

    /// Byte offset of the start of logical line `row` (zero based), clamped to the document.
    pub fn line_start(&self, row: usize) -> usize {
        self.state.line_range(row).start
    }

    /// The selection as (anchor, head) byte offsets.
    pub fn selection_offsets(&self) -> (usize, usize) {
        let sel = self.state.selection();
        (sel.anchor, sel.head)
    }

    /// Restores a selection saved with `selection_offsets`. Unlike a movement this does not
    /// scroll the view, so a change made while the editor is hidden leaves the view alone.
    pub fn restore_selection(&mut self, anchor: usize, head: usize, cx: &mut Context<Self>) {
        self.state.set_selection(anchor, head);
        self.goal_x = None;
        cx.notify();
    }

    // ----- in-note search (find bar) -----

    /// Paints `matches` (byte ranges of the document) under the text. `current` is an index into
    /// `matches` and gets the accent colour and an outline. Selection and caret are unchanged.
    pub fn set_search_matches(
        &mut self,
        matches: Vec<Range<usize>>,
        current: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.search_matches = matches;
        self.search_current = current;
        cx.notify();
    }

    /// Selects `range` (caret at its start) and scrolls it into view. Does nothing during an IME
    /// composition.
    pub fn reveal_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        if !self.can_edit_directly() {
            return;
        }
        let len = self.state.len();
        let start = range.start.min(len);
        let end = range.end.min(len);
        self.state.set_selection(end, start);
        self.goal_x = None;
        self.after_edit(cx);
    }

    /// Replaces `range` with `text` as one undo step. The caret goes to the end of the new text.
    /// Returns false (and changes nothing) during an IME composition.
    pub fn replace_text_range(
        &mut self,
        range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.can_edit_directly() {
            return false;
        }
        let start = range.start.min(range.end);
        let end = range.start.max(range.end);
        let caret = start + text.len();
        self.state
            .replace_range(start, end, text, Selection::collapsed(caret));
        self.goal_x = None;
        self.after_edit(cx);
        true
    }

    /// Replaces the whole text with `text` as ONE undo step, changing only the bytes between the
    /// common prefix and suffix (so the caret and the undo stack stay meaningful). Falls back to
    /// `set_text` while an IME composition is open.
    pub fn set_text_undoable(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.can_edit_directly() {
            self.set_text(text, cx);
            return;
        }
        let old = self.state.text().to_string();
        if old == text {
            return;
        }
        let (range, insert) = minimal_edit(&old, text);
        self.state.replace_range(
            range.start,
            range.end,
            &insert,
            Selection::collapsed(range.start + insert.len()),
        );
        self.goal_x = None;
        self.after_edit(cx);
    }

    // ----- IME composition -----

    /// Ends a composition, keeping the marked text as ordinary text.
    fn commit_composition(&mut self) {
        if self.composition.is_some() {
            let text = match self.marked_range.clone() {
                Some(r) => self.state.text().get(r).unwrap_or("").to_string(),
                None => String::new(),
            };
            self.finish_composition(&text);
        }
    }

    /// Ends the composition: restores the document from before it and applies `text` once.
    fn finish_composition(&mut self, text: &str) {
        let Some(c) = self.composition.take() else {
            return;
        };
        self.state = c.base;
        self.marked_range = None;
        if text.is_empty() {
            self.state.set_selection(c.range.start, c.range.start);
        } else {
            self.state.replace_range(
                c.range.start,
                c.range.end,
                text,
                Selection::collapsed(c.range.start + text.len()),
            );
        }
    }

    /// The UTF-8 range an input-handler call refers to: the given UTF-16 range, else the marked
    /// range, else the selection.
    fn resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        match range_utf16 {
            Some(r) => range_utf16_to_utf8(self.state.text(), &r),
            None => self
                .marked_range
                .clone()
                .unwrap_or_else(|| self.state.selection().range()),
        }
    }

    // ----- mouse and geometry -----

    /// Byte offset under a window position.
    fn index_at_window(&self, pos: Point<Pixels>) -> usize {
        let x = f32::from(pos.x) - self.layout.origin_x;
        let y = f32::from(pos.y) - self.layout.origin_y;
        self.layout.index_at(x, y, self.state.len())
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        self.commit_composition();
        // A task box of the live view toggles its task (one undo step) and leaves the caret alone.
        let bx = f32::from(ev.position.x) - self.layout.origin_x;
        let by = f32::from(ev.position.y) - self.layout.origin_y;
        if let Some(at) = self.layout.check_at(bx, by)
            && self.toggle_task_at(at, cx)
        {
            return;
        }
        // The back arrow of a footnote definition jumps to the first reference of its label.
        if let Some(n) = self.layout.back_at(bx, by) {
            self.jump_footnote(footnote::Jump::Reference(n), cx);
            return;
        }
        // The copy button of a code panel copies the body (source text) and leaves the caret alone.
        if let Some(body) = self.layout.button_at(bx, by) {
            let code = self.state.text().get(body).unwrap_or("").to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(code));
            return;
        }
        // The title bar of a note embed opens its note (the same path as a wikilink).
        if let Some(raw) = self.layout.link_at(bx, by) {
            cx.emit(OpenWikilink(raw));
            return;
        }
        // The title row of a foldable callout toggles its fold (in memory; the source is kept).
        if let Some((key, open)) = self.layout.fold_at(bx, by) {
            self.folds.insert(key, !open);
            cx.notify();
            return;
        }
        // A table cell: the caret goes to the matching source position, which reveals the table.
        if let Some(at) = self.layout.table_source_at(bx, by) {
            self.goal_x = None;
            self.state.set_selection(at, at);
            self.after_edit(cx);
            return;
        }
        let idx = self.index_at_window(ev.position);
        // Ctrl+click on a [[wikilink]] asks the app to open its note; the caret stays put.
        if (primary_held(&ev.modifiers) || self.read_only) && ev.click_count <= 1 {
            // Ctrl+click (or a click in read mode) on a footnote reference jumps to its definition.
            if let Some(n) = self
                .style_cache
                .as_ref()
                .and_then(|c| footnote::ref_at(&c.hides, idx))
            {
                self.jump_footnote(footnote::Jump::Definition(n), cx);
                return;
            }
            if let Some(inner) = wikilink_inner_at(self.state.text(), idx) {
                cx.emit(OpenWikilink(inner));
                return;
            }
        }
        self.goal_x = None;
        match ev.click_count {
            0 | 1 => {
                if ev.modifiers.shift {
                    self.move_head(idx, true);
                } else {
                    self.state.set_selection(idx, idx);
                }
                self.selecting = true;
                self.drag_anchor = self.state.selection().anchor;
            }
            2 => self.state.select_word_at(idx),
            _ => self.state.select_line_at(idx),
        }
        self.after_edit(cx);
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.update_hover(ev.position, primary_held(&ev.modifiers), cx);
        if !self.selecting {
            return;
        }
        if ev.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return;
        }
        let idx = self.index_at_window(ev.position);
        let anchor = self.drag_anchor;
        self.state.set_selection(anchor, idx);
        self.after_edit(cx);
    }

    fn on_mouse_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        self.selecting = false;
    }

    /// Sets the pointing-hand flag for the pointer at window `pos` (see `hand_cursor`) and repaints
    /// when it changes. `ctrl` is the Ctrl (Cmd on macOS) state of the event.
    fn update_hover(&mut self, pos: Point<Pixels>, ctrl: bool, cx: &mut Context<Self>) {
        let bx = f32::from(pos.x) - self.layout.origin_x;
        let by = f32::from(pos.y) - self.layout.origin_y;
        let over_widget = self.layout.widget_at(bx, by);
        let idx = self.index_at_window(pos);
        let over_link = !over_widget
            && self.style_cache.as_ref().is_some_and(|c| {
                link_span_at(&c.spans, idx) || footnote::ref_at(&c.hides, idx).is_some()
            });
        let hand = hand_cursor(self.read_only, self.live, ctrl, over_widget, over_link);
        if hand != self.hand {
            self.hand = hand;
            cx.notify();
        }
    }

    fn on_scroll_wheel(
        &mut self,
        ev: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = ev.delta.pixel_delta(px(fonts::current_metrics().line));
        self.scroll_y = (self.scroll_y - f32::from(delta.y)).max(0.0);
        cx.notify();
    }
}

impl Focusable for TextEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// ----- element: prepaint (shape, layout, quads) and paint ------------------------------

/// Paints the document: selection, shaped lines, caret. Installs the input handler.
struct TextElement {
    editor: Entity<TextEditor>,
}

struct PrepaintState {
    /// Icons of the widgets (path in `assets/icons`, colour), painted after the decorations.
    svgs: Vec<(Bounds<Pixels>, SharedString, Hsla)>,
    /// Search match highlights (behind the selection and the text).
    matches: Vec<PaintQuad>,
    selection: Vec<PaintQuad>,
    /// Rules, quote bars and task boxes of the live view (above the selection, below the text).
    decos: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    /// Visible shaped lines with their window origins, row heights, alignment and the box width
    /// an aligned line is set in (see `WidgetText`).
    lines: Vec<VisibleText>,
    /// Pictures of the image widgets, painted after the decorations.
    images: Vec<(Bounds<Pixels>, Arc<RenderImage>)>,
    /// Inline formulas, painted after the text backgrounds (they sit in the text row).
    inline_images: Vec<(Bounds<Pixels>, Arc<RenderImage>)>,
}

/// One visible shaped line: origin, line, row height, alignment and the aligned box width.
type VisibleText = (Point<Pixels>, WrappedLine, f32, TextAlign, Option<Pixels>);

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

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
    ) -> Self::PrepaintState {
        let frame_started = std::time::Instant::now();
        // The installed fonts are listed once, so a missing configured font is noticed at start.
        fonts::ensure_installed(|| cx.text_system().all_font_names());
        let gap_ms = take_frame_gap(frame_started);
        let mut phases: Vec<(&'static str, f64)> = Vec::new();
        #[allow(unused_assignments)]
        let mut last = frame_started;
        macro_rules! phase {
            ($name:expr) => {{
                let now = std::time::Instant::now();
                phases.push(($name, (now - last).as_secs_f64() * 1000.0));
                last = now;
            }};
        }
        self.editor.update(cx, |ed, _| ed.refresh_styles());
        // The shaped-line cache, the previous frame's lines and its key move out for this frame
        // and back in at the end.
        let (mut line_cache, mut widget_cache, prev_lines, mut prev_key) =
            self.editor.update(cx, |ed, _| {
                (
                    std::mem::take(&mut ed.line_cache),
                    std::mem::take(&mut ed.widget_cache),
                    std::mem::take(&mut ed.layout.lines),
                    ed.frame_key.take(),
                )
            });
        phase!("take");
        let editor = self.editor.read(cx);
        let theme = editor.theme;
        let text = editor.state.text().to_string();
        let (styles, hides, notes): (Option<&Vec<Vec<Segment>>>, &[map::Hide], u64) =
            match editor.style_cache.as_ref() {
                Some(c) => (Some(&c.segments), c.hides.as_slice(), c.notes),
                None => (None, &[], 0),
            };
        let live = editor.live;
        let read_only = editor.read_only;
        let sel = editor.state.selection();
        let marked = editor.marked_range.clone();
        let scroll_in = editor.scroll_y;
        let reveal = editor.reveal;
        let caret_on = editor.caret_on;
        let focused = editor.focus_handle.is_focused(window);
        let folds = editor.folds.clone();
        let vault = editor.vault.clone();
        let note_path = editor.note_path.clone();
        let pending: RefCell<Vec<context::Pending>> = RefCell::new(Vec::new());
        let vault_gen = vault.as_ref().map(|v| v.borrow().data_gen);

        let view_w = f32::from(bounds.size.width);
        let view_h = f32::from(bounds.size.height);
        let column = if read_only {
            READ_COLUMN_WIDTH
        } else {
            COLUMN_WIDTH
        };
        let wrap_w = (view_w - 2.0 * GUTTER).min(column).max(1.0);
        let col_left = ((view_w - wrap_w) / 2.0).max(0.0);

        // Shape every logical line at the wrap width, with its style runs. Headings and code
        // lines get their own font size and row height.
        let sel_range = sel.range();
        // The font generation is part of the key: a font change re-shapes every line.
        let theme_key = fonts::shape_salt(theme_salt(&theme), fonts::generation());
        if line_cache.len() > LINE_CACHE_MAX {
            line_cache.clear();
        }
        // Which lines to rebuild. The reveal state of every record is compared with the last
        // frame: a frame where only the caret or selection moved rebuilds the lines whose records
        // changed (and the lines of the IME range). Text, view mode, width and theme changes
        // rebuild all lines.
        // A block widget that holds the current find match is shown raw too (the find fallback).
        let found = editor
            .search_current
            .and_then(|i| editor.search_matches.get(i))
            .cloned();
        let revealed: Vec<bool> = hides
            .iter()
            .map(|h| {
                let found_here = map::found_reveals(h.deco, &h.src, found.as_ref());
                map::shows_source(live, read_only, &h.reveal, &sel_range, found_here)
            })
            .collect();
        phase!("reveal");
        // Fold keys and start states of the callouts, by block start. The ordinal counts the
        // callouts with the same header in document order, whether or not each one is a widget.
        let mut callout_keys: HashMap<usize, (u64, bool)> = HashMap::new();
        {
            let mut ordinals: HashMap<u64, usize> = HashMap::new();
            for h in hides.iter() {
                if h.deco != map::Deco::Block(map::BlockKind::Callout) {
                    continue;
                }
                let Some(c) = callout::parse(&text, h.src.clone()) else {
                    continue;
                };
                let header = text.get(c.header.clone()).unwrap_or("");
                let n = ordinals.entry(callout::header_hash(header)).or_insert(0);
                let key = callout::fold_key(header, *n);
                *n += 1;
                callout_keys.insert(h.src.start, (key, c.fold != callout::Fold::Closed));
            }
        }
        phase!("callouts");
        // Logical lines as (start, len); a block widget may cover several of them.
        let split: Vec<(usize, usize)> = {
            let mut start = 0usize;
            text.split('\n')
                .map(|line| {
                    let item = (start, line.len());
                    start += line.len() + 1;
                    item
                })
                .collect()
        };
        phase!("split");
        let line_count = split.len();
        // Index of the logical line that holds byte `offset`.
        let line_of = |offset: usize| {
            split
                .partition_point(|&(s, _)| s <= offset)
                .saturating_sub(1)
        };
        let wrap_bits = wrap_w.to_bits();
        // Pass 1: every unit of the document in order. A unit keeps its built state from the last
        // frame when the edit (see `invalidate`) left its text and styling alone; the others are
        // placeholders with an estimated height. The reveal state of a record, and the IME range,
        // rebuild the lines they touch.
        let hide_src: Vec<Range<usize>> = hides.iter().map(|h| h.src.clone()).collect();
        let sigs = style_signatures(styles, &split);
        let mut dirty = vec![false; line_count];
        let mut prev: Vec<LineLayout> = Vec::new();
        let mut prev_sigs: Vec<u64> = Vec::new();
        let mut span: Option<invalidate::EditSpan> = None;
        let mut line_delta = 0isize;
        let mut reuse = false;
        // The unit at the top of the view in the last frame: the view keeps it at the top.
        let anchor: Option<(usize, f32)> = {
            let i = prev_lines.partition_point(|l| l.top + l.height() <= scroll_in);
            prev_lines.get(i).map(|l| (l.start, scroll_in - l.top))
        };
        if let Some(k) = prev_key.as_mut() {
            let same_view = k.live == live
                && k.wrap_w == wrap_bits
                && k.theme == theme_key
                && k.notes == notes
                && k.revealed.len() == revealed.len();
            if same_view {
                let es = (k.text != text).then(|| invalidate::edit_span(&k.text, &text));
                if invalidate::records_match(&k.hide_src, &hide_src, es.as_ref()) {
                    reuse = true;
                    line_delta = newline_count(&text) as isize - newline_count(&k.text) as isize;
                    for (idx, (&now, &was)) in revealed.iter().zip(&k.revealed).enumerate() {
                        if now != was {
                            let h = &hides[idx];
                            for d in &mut dirty[line_of(h.src.start)..=line_of(h.src.end)] {
                                *d = true;
                            }
                        }
                    }
                    for m in [k.marked.as_ref(), marked.as_ref()].into_iter().flatten() {
                        dirty[line_of(m.start)] = true;
                        dirty[line_of(m.end)] = true;
                    }
                    span = es;
                    prev_sigs = std::mem::take(&mut k.sigs);
                    prev = prev_lines;
                }
            }
        }
        // A line with an inline formula is rebuilt every frame: the formula may have become ready.
        for h in hides.iter().filter(|h| h.deco == map::Deco::InlineMath) {
            dirty[line_of(h.src.start)] = true;
        }
        let ctx = LineCtx {
            text: &text,
            theme: &theme,
            theme_key,
            wrap_w,
            sel: &sel_range,
            marked: marked.as_ref(),
            live,
            read_only,
            hides,
            vault: vault.as_ref(),
            note_path: &note_path,
            pending: &pending,
            split: &split,
            styles,
            revealed: &revealed,
            callout_keys: &callout_keys,
            folds: &folds,
            wrap_bits,
            vault_gen,
        };
        phase!("ctx");
        // Nothing changed in the text, the records or the styling: the last frame's units stand.
        let fast = reuse && span.is_none() && !dirty.iter().any(|&d| d);
        // An edit that leaves the units around it alone rebuilds only the lines of its region; the
        // units before the region stay in place and the units after it move by the change.
        let plan = match span.as_ref() {
            Some(s) if reuse && !fast => plan_splice(
                s, &prev, &split, &hide_src, &dirty, &prev_sigs, &sigs, line_delta,
            ),
            _ => None,
        };
        let (lo, hi, splice) = match plan {
            Some((a, b, lo, hi)) => (lo, hi, Some((a, b))),
            None => (0, line_count, None),
        };
        let mid = if splice.is_some() { lo..hi } else { 0..0 };
        // Whether the unit at new line `li` can take the last frame's unit at its start.
        let reusable = |start: usize, len: usize, li: usize| -> Option<invalidate::Source> {
            if !reuse || mid.contains(&li) {
                return None;
            }
            let src = invalidate::source(start, len, li, span.as_ref(), line_delta)?;
            (prev_sigs.get(src.old_line) == sigs.get(li)).then_some(src)
        };
        let mut lines: Vec<LineLayout> = Vec::with_capacity(hi - lo);
        let mut cursor = 0usize;
        let mut li = lo;
        if fast {
            lines = std::mem::take(&mut prev);
            li = line_count;
        }
        while li < hi {
            let (start, len) = split[li];
            // A block widget that the caret does not touch replaces all of its source lines.
            if let Some(idx) = live
                .then(|| widget_record(hides, start, &revealed))
                .flatten()
            {
                let h = &hides[idx];
                let map::Deco::Block(kind) = h.deco else {
                    li += 1;
                    continue;
                };
                let block_len = h.src.end - start;
                let last = line_of(h.src.end);
                let touched = dirty[li..=last].iter().any(|&d| d);
                // A callout's fold: its key and whether it is open now (toggled, else its start state).
                let fold = match kind {
                    map::BlockKind::Callout => {
                        let (key, default_open) =
                            callout_keys.get(&start).copied().unwrap_or((0, true));
                        (key, folds.get(&key).copied().unwrap_or(default_open))
                    }
                    _ => (0, false),
                };
                let block = text.get(start..start + block_len).unwrap_or("");
                let src = reusable(start, block_len, li);
                let old = src.and_then(|s| take_prev(&mut prev, &mut cursor, s.old_start));
                let unit = match old {
                    Some(mut l)
                        if !touched
                            && l.built
                            && l.len == block_len
                            && l.widget.as_ref().is_some_and(|w| {
                                w.open == fold.1 && widget_fresh(w, vault_gen)
                            }) =>
                    {
                        l.shift(src.map_or(0, |s| s.delta));
                        l
                    }
                    other => {
                        let height = other
                            .as_ref()
                            .map_or_else(|| widget_estimate(kind, block), |l| l.height());
                        placeholder(start, block_len, 1, height)
                    }
                };
                lines.push(unit);
                li = last + 1;
                continue;
            }
            let src = if dirty[li] {
                None
            } else {
                reusable(start, len, li)
            };
            let old = src.and_then(|s| take_prev(&mut prev, &mut cursor, s.old_start));
            let entry = match old {
                Some(mut l) if l.built && l.widget.is_none() && l.len == len => {
                    l.shift(src.map_or(0, |s| s.delta));
                    l
                }
                other => {
                    let (rows, line_h) = match other.as_ref() {
                        Some(l) if l.widget.is_none() => (l.rows, l.line_h),
                        _ => {
                            let segs: &[Segment] =
                                styles.and_then(|s| s.get(li)).map_or(&[], |v| v.as_slice());
                            let (font_size, lh) = line_metrics(segs);
                            let text_line = text.get(start..start + len).unwrap_or("");
                            let gap = if read_only && len == 0 {
                                READ_PARA_GAP
                            } else {
                                0.0
                            };
                            (estimate_rows(text_line, font_size, wrap_w), lh + gap)
                        }
                    };
                    placeholder(start, len, rows, line_h)
                }
            };
            lines.push(entry);
            li += 1;
        }
        if let Some((a, b)) = splice {
            let made = lines.len();
            prev.splice(a..b, lines);
            let delta = span.map_or(0, |s| s.delta());
            for l in prev[a + made..].iter_mut() {
                l.shift(delta);
            }
            lines = prev;
        }
        phase!("prev");
        let mut layout = Layout {
            origin_x: 0.0,
            origin_y: 0.0,
            wrap_width: wrap_w,
            viewport_h: view_h,
            lines,
            checks: Vec::new(),
            buttons: Vec::new(),
            fold_hits: Vec::new(),
            links: Vec::new(),
            footnote_backs: Vec::new(),
        };
        place_lines(&mut layout.lines);
        phase!("build");
        // Pass 2: the units in the window around the view are built, and the units the caret, the
        // current find match and the IME range need. The view then keeps its anchor unit at the top
        // (heights of units above it may have become exact), or goes to the caret when it moved.
        let mut forced: Vec<usize> = Vec::new();
        let n = layout.lines.len();
        if n > 0 {
            let unit_at = |lines: &[LineLayout], offset: usize| {
                lines
                    .partition_point(|l| l.start + l.len < offset)
                    .min(lines.len() - 1)
            };
            let c = unit_at(&layout.lines, sel.head);
            forced.extend(c.saturating_sub(1)..=(c + 1).min(n - 1));
            if let Some(r) = found.as_ref() {
                forced.push(unit_at(&layout.lines, r.start));
            }
            if let Some(m) = marked.as_ref() {
                forced.push(unit_at(&layout.lines, m.start));
            }
        }
        let mut built = 0usize;
        let mut caches = UnitCaches {
            lines: &mut line_cache,
            widgets: &mut widget_cache,
        };
        let mut scroll = scroll_in;
        for _ in 0..4 {
            let max_scroll = (PAD_TOP + layout.content_height() + PAD_BOTTOM - view_h).max(0.0);
            scroll = scroll.clamp(0.0, max_scroll);
            built += build_window(
                window,
                &ctx,
                &mut caches,
                &mut layout.lines,
                &forced,
                scroll,
                view_h,
            );
            place_lines(&mut layout.lines);
            let target = if reveal {
                let (_, caret_y) = layout.point_for_index(sel.head);
                scroll_to_reveal(
                    PAD_TOP + caret_y,
                    PAD_TOP + caret_y + layout.row_height_at(sel.head),
                    scroll,
                    view_h,
                )
            } else if let Some((old_start, offset)) = anchor {
                let new_start = invalidate::map_offset(old_start, span.as_ref());
                let i = layout.lines.partition_point(|l| l.start < new_start);
                match layout.lines.get(i) {
                    Some(l) if l.start == new_start => l.top + offset,
                    _ => scroll,
                }
            } else {
                scroll
            };
            let settled = (target - scroll).abs() < 0.5;
            scroll = target;
            if settled {
                break;
            }
        }
        let max_scroll = (PAD_TOP + layout.content_height() + PAD_BOTTOM - view_h).max(0.0);
        scroll = scroll.clamp(0.0, max_scroll);
        built += build_window(
            window,
            &ctx,
            &mut caches,
            &mut layout.lines,
            &forced,
            scroll,
            view_h,
        );
        place_lines(&mut layout.lines);
        let requests = pending.into_inner();
        let frame_key = FrameKey {
            text,
            live,
            wrap_w: wrap_bits,
            theme: theme_key,
            notes,
            revealed,
            marked: marked.clone(),
            hide_src,
            sigs,
        };
        layout.origin_x = f32::from(bounds.origin.x) + col_left;
        layout.origin_y = f32::from(bounds.origin.y) + PAD_TOP - scroll;
        let (ox, oy) = (layout.origin_x, layout.origin_y);
        phase!("scroll");
        // Decorations of replaced markers: rules, quote bars and task boxes. The boxes are also
        // kept in document space for clicks.
        let mut decos: Vec<PaintQuad> = Vec::new();
        let mut svgs: Vec<(Bounds<Pixels>, SharedString, Hsla)> = Vec::new();
        let mut checks: Vec<(f32, f32, f32, f32, usize)> = Vec::new();
        // Footnote labels (ordinals, arrows, unused labels) are text painted with the lines, and
        // the back arrows are hit boxes in document space.
        let mut label_texts: Vec<VisibleText> = Vec::new();
        let mut footnote_backs: Vec<(f32, f32, f32, f32, u32)> = Vec::new();
        let mut images: Vec<(Bounds<Pixels>, Arc<RenderImage>)> = Vec::new();
        let mut inline_images: Vec<(Bounds<Pixels>, Arc<RenderImage>)> = Vec::new();
        let deco_top = f32::from(bounds.origin.y);
        let deco_bottom = deco_top + view_h;
        for line in &layout.lines {
            let row_top = oy + line.top;
            if row_top + line.height() < deco_top || row_top > deco_bottom {
                continue;
            }
            if let Some(w) = &line.widget {
                if let Some(((x, y, iw, ih), img)) = &w.image {
                    let bounds =
                        Bounds::new(point(px(ox + x), px(row_top + y)), size(px(*iw), px(*ih)));
                    images.push((bounds, img.clone()));
                }
                if let Some((x, y, pw, ph)) = w.panel {
                    let bounds =
                        Bounds::new(point(px(ox + x), px(row_top + y)), size(px(pw), px(ph)));
                    let (panel_bg, panel_edge) = match w.panel_fill {
                        Some(tint) => (tint, 0.0),
                        None => (theme.raised, 1.0),
                    };
                    decos.push(quad(
                        bounds,
                        Corners::all(px(PANEL_RADIUS)),
                        panel_bg,
                        Edges::all(px(panel_edge)),
                        theme.line,
                        BorderStyle::default(),
                    ));
                }
                for &((x, y, iw, ih), name, color) in &w.icons {
                    match name {
                        Some(name) => {
                            let bounds = Bounds::new(
                                point(px(ox + x), px(row_top + y)),
                                size(px(iw), px(ih)),
                            );
                            let path = SharedString::from(format!("icons/{name}.svg"));
                            svgs.push((bounds, path, color.into()));
                        }
                        None => {
                            // An icon that the assets do not have: a coloured dot.
                            let bounds = Bounds::new(
                                point(
                                    px(ox + x + iw / 2.0 - 4.0),
                                    px(row_top + y + ih / 2.0 - 4.0),
                                ),
                                size(px(8.0), px(8.0)),
                            );
                            decos.push(quad(
                                bounds,
                                Corners::all(px(4.0)),
                                color,
                                Edges::all(px(0.0)),
                                color,
                                BorderStyle::default(),
                            ));
                        }
                    }
                }
                for &((x, y, fw, fh), color) in &w.fills {
                    let bounds =
                        Bounds::new(point(px(ox + x), px(row_top + y)), size(px(fw), px(fh)));
                    decos.push(fill(bounds, color));
                }
                if let Some(((x, y, bw, bh), _)) = &w.button {
                    let bounds =
                        Bounds::new(point(px(ox + x), px(row_top + y)), size(px(*bw), px(*bh)));
                    decos.push(quad(
                        bounds,
                        Corners::all(px(4.0)),
                        theme.sheet,
                        Edges::all(px(1.0)),
                        theme.line,
                        BorderStyle::default(),
                    ));
                }
                continue;
            }
            for p in line.pieces.iter().filter(|p| p.deco != map::Deco::None) {
                let (x, y) = layout.point_for_index(p.src.start);
                match p.deco {
                    map::Deco::Checkbox { checked } => {
                        let (bx, by) = (x + 2.0, y + (line.line_h - CHECK_SIZE) / 2.0);
                        checks.push((bx, by, CHECK_SIZE, CHECK_SIZE, p.src.start));
                        let bounds = Bounds::new(
                            point(px(ox + bx), px(oy + by)),
                            size(px(CHECK_SIZE), px(CHECK_SIZE)),
                        );
                        decos.push(if checked {
                            fill(bounds, theme.ac)
                        } else {
                            quad(
                                bounds,
                                Corners::all(px(3.0)),
                                theme.sheet,
                                Edges::all(px(1.5)),
                                theme.tx3,
                                BorderStyle::default(),
                            )
                        });
                    }
                    map::Deco::QuoteBar => {
                        let bounds = Bounds::new(
                            point(px(ox + x), px(oy + y)),
                            size(px(QUOTE_BAR_WIDTH), px(line.line_h)),
                        );
                        decos.push(fill(bounds, theme.tx3));
                    }
                    map::Deco::Rule => {
                        let bounds = Bounds::new(
                            point(px(ox), px(oy + line.top + line.line_h / 2.0)),
                            size(px(wrap_w), px(1.0)),
                        );
                        decos.push(fill(bounds, theme.line));
                    }
                    map::Deco::FootnoteRef { ordinal, missing } => {
                        let color = if missing {
                            callout_color(&theme, callout::Tone::Amber)
                        } else {
                            theme.ac
                        };
                        let num = ordinal.to_string();
                        if let Some(wl) = shape_one(
                            window,
                            &num,
                            &label_runs(&num, color, &fonts::ui_name()),
                            fonts::current_metrics().font * 0.72,
                            None,
                        ) {
                            label_texts.push((
                                point(px(ox + x), px(oy + y + FOOTNOTE_TOP)),
                                wl,
                                (fonts::current_metrics().font * 0.72 * 1.4),
                                TextAlign::Left,
                                None,
                            ));
                        }
                    }
                    map::Deco::FootnoteDef {
                        ordinal: Some(n), ..
                    } => {
                        let num = n.to_string();
                        let arrow_x =
                            x + footnote::digits(n) as f32 * (fonts::current_metrics().font * 0.5);
                        if let Some(wl) = shape_one(
                            window,
                            &num,
                            &label_runs(&num, theme.ac, &fonts::ui_name()),
                            fonts::current_metrics().font * 0.72,
                            None,
                        ) {
                            label_texts.push((
                                point(px(ox + x), px(oy + y + FOOTNOTE_TOP)),
                                wl,
                                (fonts::current_metrics().font * 0.72 * 1.4),
                                TextAlign::Left,
                                None,
                            ));
                        }
                        let arrow = "\u{21A9}";
                        if let Some(wl) = shape_one(
                            window,
                            arrow,
                            &label_runs(arrow, theme.tx2, &fonts::ui_name()),
                            fonts::current_metrics().font * 0.72,
                            None,
                        ) {
                            label_texts.push((
                                point(px(ox + arrow_x), px(oy + y + FOOTNOTE_TOP)),
                                wl,
                                (fonts::current_metrics().font * 0.72 * 1.4),
                                TextAlign::Left,
                                None,
                            ));
                        }
                        footnote_backs.push((
                            arrow_x,
                            y,
                            (fonts::current_metrics().font * 0.5) * 2.0,
                            line.line_h,
                            n,
                        ));
                    }
                    map::Deco::FootnoteDef { ordinal: None, .. } => {
                        // An unused definition shows its label, dimmed.
                        if let Some(label) =
                            footnote::def_label_at(editor.state.text(), p.src.start)
                            && let Some(wl) = shape_one(
                                window,
                                label,
                                &label_runs(label, theme.tx3, &fonts::ui_name()),
                                fonts::current_metrics().font * 0.72,
                                None,
                            )
                        {
                            label_texts.push((
                                point(px(ox + x), px(oy + y + FOOTNOTE_TOP)),
                                wl,
                                (fonts::current_metrics().font * 0.72 * 1.4),
                                TextAlign::Left,
                                None,
                            ));
                        }
                    }
                    map::Deco::None | map::Deco::Block(_) | map::Deco::InlineMath => {}
                }
            }
            // Inline formulas sit on the text baseline of their row.
            for s in &line.inline {
                let (x, y) = layout.point_for_index(s.src);
                let text_base = (line.line_h - fonts::current_metrics().font * 1.2) / 2.0
                    + fonts::current_metrics().font * 0.95;
                let top = y + text_base - s.baseline;
                let bounds = Bounds::new(
                    point(px(ox + x), px(oy + top)),
                    size(px(s.css.0), px(s.css.1)),
                );
                inline_images.push((bounds, s.image.clone()));
            }
        }
        phase!("decos");
        layout.checks = checks;
        layout.footnote_backs = footnote_backs;
        layout.buttons = layout
            .lines
            .iter()
            .filter_map(|l| {
                let ((x, y, bw, bh), body) = l.widget.as_ref()?.button.clone()?;
                Some((x, l.top + y, bw, bh, body))
            })
            .collect();
        let mut links: Vec<(f32, f32, f32, f32, String)> = Vec::new();
        for l in &layout.lines {
            if let Some(w) = &l.widget {
                for ((x, y, lw, lh), raw) in &w.links {
                    links.push((*x, l.top + *y, *lw, *lh, raw.clone()));
                }
            }
        }
        layout.links = links;
        layout.fold_hits = layout
            .lines
            .iter()
            .filter_map(|l| {
                let w = l.widget.as_ref()?;
                let c = w.callout.as_ref()?;
                let width = w.panel.map_or(wrap_w, |p| p.2);
                Some((0.0, l.top, width, c.title_h, c.key, c.open))
            })
            .collect();

        phase!("hits");
        let selection = layout
            .selection_rects(&sel.range())
            .into_iter()
            .map(|(x, y, w, h)| {
                fill(
                    Bounds::new(point(px(ox + x), px(oy + y)), size(px(w), px(h))),
                    theme.sel,
                )
            })
            .collect::<Vec<_>>();

        // Search matches: a translucent fill each; the current one gets the accent colour and an
        // outline. Painted before the selection so the text stays on top of them.
        let matches = {
            let mut quads = Vec::new();
            let current = editor.search_current;
            for (i, range) in editor
                .search_matches
                .iter()
                .enumerate()
                .take(MAX_PAINTED_MATCHES)
            {
                for (x, y, w, h) in layout.selection_rects(range) {
                    let bounds = Bounds::new(point(px(ox + x), px(oy + y)), size(px(w), px(h)));
                    quads.push(if current == Some(i) {
                        quad(
                            bounds,
                            Corners::all(px(3.0)),
                            theme.ac_soft,
                            Edges::all(px(1.0)),
                            theme.ac,
                            BorderStyle::default(),
                        )
                    } else {
                        fill(bounds, theme.sel)
                    });
                }
            }
            quads
        };

        let caret = if focused && caret_on && !read_only {
            let (x, y) = layout.point_for_index(sel.head);
            Some(fill(
                Bounds::new(
                    point(px(ox + x), px(oy + y)),
                    size(px(CARET_WIDTH), px(layout.row_height_at(sel.head))),
                ),
                theme.ac,
            ))
        } else {
            None
        };

        phase!("sel");
        // Only lines that overlap the viewport are painted.
        let view_top = f32::from(bounds.origin.y);
        let view_bottom = view_top + view_h;
        let mut visible: Vec<VisibleText> = label_texts;
        for l in layout.lines.iter() {
            let y = oy + l.top;
            let bottom = y + l.height();
            if bottom < view_top || y > view_bottom {
                continue;
            }
            if let Some(w) = &l.widget {
                for t in &w.texts {
                    let origin = point(px(ox + t.x), px(y + t.y));
                    let align_w = t.align_w.map(px);
                    visible.push((origin, t.line.clone(), t.lh, t.align, align_w));
                }
                continue;
            }
            if let Some(wrapped) = l.wrapped.clone() {
                visible.push((
                    point(px(ox), px(y)),
                    wrapped,
                    l.line_h,
                    TextAlign::Left,
                    None,
                ));
            }
        }

        phase!("visible");
        self.editor.update(cx, move |ed, _| {
            ed.layout = layout;
            ed.scroll_y = scroll;
            ed.reveal = false;
            ed.line_cache = line_cache;
            ed.widget_cache = widget_cache;
            ed.frame_key = Some(frame_key);
        });
        // The images the widgets asked for start decoding (after the last read of the editor).
        if let Some(handle) = vault.as_ref() {
            if !requests.is_empty() {
                context::ensure_images(handle, requests, cx);
            }
            context::ensure_math(handle, cx);
            context::ensure_diagrams(handle, cx);
        }

        if prepaint_log_path().is_some() {
            phase!("write");
            let _ = last;
            let ms = frame_started.elapsed().as_secs_f64() * 1000.0;
            let ph: Vec<String> = phases.iter().map(|(n, t)| format!("{n}:{t:.2}")).collect();
            log_timing(&format!(
                "lines={line_count} built={built} gap_ms={gap_ms:.3} prepaint_ms={ms:.3} ph={}",
                ph.join(",")
            ));
        }
        PrepaintState {
            svgs,
            matches,
            selection,
            decos,
            caret,
            lines: visible,
            images,
            inline_images,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let paint_started = std::time::Instant::now();
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        // Run backgrounds (inline code, highlights) first, then the selection, then the text.
        for (origin, line, line_h, align, align_w) in &prepaint.lines {
            let bounds = align_w.map(|w| Bounds::new(*origin, size(w, px(*line_h))));
            let _ = line.paint_background(*origin, px(*line_h), *align, bounds, window, cx);
        }
        for quad in std::mem::take(&mut prepaint.matches) {
            window.paint_quad(quad);
        }
        for quad in std::mem::take(&mut prepaint.selection) {
            window.paint_quad(quad);
        }
        for quad in std::mem::take(&mut prepaint.decos) {
            window.paint_quad(quad);
        }
        for (bounds, path, color) in std::mem::take(&mut prepaint.svgs) {
            let _ = window.paint_svg(bounds, path, TransformationMatrix::unit(), color, cx);
        }
        for (bounds, image) in std::mem::take(&mut prepaint.inline_images) {
            let _ = window.paint_image(bounds, Corners::all(px(0.0)), image, 0, false);
        }
        for (bounds, image) in std::mem::take(&mut prepaint.images) {
            let _ = window.paint_image(bounds, Corners::all(px(IMG_RADIUS)), image, 0, false);
        }
        for (origin, line, line_h, align, align_w) in &prepaint.lines {
            let bounds = align_w.map(|w| Bounds::new(*origin, size(w, px(*line_h))));
            let _ = line.paint(*origin, px(*line_h), *align, bounds, window, cx);
        }
        if let Some(caret) = prepaint.caret.take() {
            window.paint_quad(caret);
        }
        if prepaint_log_path().is_some() {
            let ms = paint_started.elapsed().as_secs_f64() * 1000.0;
            log_timing(&format!("paint_ms={ms:.3}"));
        }
    }
}

// ----- IME input handler ----------------------------------------------------------------

impl EntityInputHandler for TextEditor {
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
        let sel = self.state.selection();
        Some(UTF16Selection {
            range: range_utf8_to_utf16(self.state.text(), &sel.range()),
            reversed: sel.head < sel.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|r| range_utf8_to_utf16(self.state.text(), r))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.commit_composition();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.composition.is_some() {
            // Commit of composed text: ends the composition with exactly this text.
            self.finish_composition(new_text);
        } else {
            let range = self.resolve_range(range_utf16);
            // A single typed char (keyboard or IME punctuation such as the full-width brackets)
            // may auto-pair or wrap the selection; anything else is inserted as it is.
            let plan = single_typed_char(new_text)
                .filter(|_| self.can_edit_directly())
                .and_then(|ch| {
                    md_edit::on_typed_char(self.state.text(), range.start, range.end, ch)
                });
            if let Some(plan) = plan {
                self.state
                    .replace_range(plan.start, plan.end, &plan.insert, plan_selection(&plan));
            } else if range == self.state.selection().range() {
                self.state.insert_text(new_text);
            } else {
                self.state.replace_range(
                    range.start,
                    range.end,
                    new_text,
                    Selection::collapsed(range.start + new_text.len()),
                );
            }
        }
        self.marked_range = None;
        self.goal_x = None;
        self.after_edit(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = self.resolve_range(range_utf16);
        if self.composition.is_none() {
            self.composition = Some(Composition {
                base: self.state.clone(),
                range: range.clone(),
            });
        }
        self.state.replace_range(
            range.start,
            range.end,
            new_text,
            Selection::collapsed(range.start + new_text.len()),
        );
        self.marked_range = if new_text.is_empty() {
            None
        } else {
            Some(range.start..range.start + new_text.len())
        };
        if let Some(sel16) = new_selected_range_utf16 {
            let a = range.start + utf16_to_utf8(new_text, sel16.start);
            let b = range.start + utf16_to_utf8(new_text, sel16.end);
            self.state.set_selection(a, b);
        }
        self.goal_x = None;
        self.after_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if self.layout.lines.is_empty() {
            return None;
        }
        let range = range_utf16_to_utf8(self.state.text(), &range_utf16);
        let (x0, y0) = self.layout.point_for_index(range.start);
        let (x1, y1) = self.layout.point_for_index(range.end);
        let x1 = if (y1 - y0).abs() < 0.5 {
            x1
        } else {
            x0 + CARET_WIDTH
        };
        let top = self.layout.origin_y + y0;
        Some(Bounds::from_corners(
            point(px(self.layout.origin_x + x0), px(top)),
            point(
                px(self.layout.origin_x + x1),
                px(top + self.layout.row_height_at(range.start)),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let idx = self.index_at_window(position);
        Some(utf8_to_utf16(self.state.text(), idx))
    }
}

// ----- actions ----------------------------------------------------------------------------

impl Render for TextEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_out.is_none() {
            let sub = cx.on_focus_out(&self.focus_handle, window, |_, _, _, cx| {
                cx.emit(EditorBlur)
            });
            self.focus_out = Some(sub);
        }
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(self.theme.sheet)
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(if self.hand {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            })
            // Horizontal
            .on_action(
                cx.listener(|this, _: &MoveLeft, _, cx| this.apply(cx, |s| s.move_left(false))),
            )
            .on_action(
                cx.listener(|this, _: &MoveRight, _, cx| this.apply(cx, |s| s.move_right(false))),
            )
            .on_action(
                cx.listener(|this, _: &SelectLeft, _, cx| this.apply(cx, |s| s.move_left(true))),
            )
            .on_action(
                cx.listener(|this, _: &SelectRight, _, cx| this.apply(cx, |s| s.move_right(true))),
            )
            .on_action(cx.listener(|this, _: &MoveWordLeft, _, cx| {
                this.apply(cx, |s| s.move_word_left(false))
            }))
            .on_action(cx.listener(|this, _: &MoveWordRight, _, cx| {
                this.apply(cx, |s| s.move_word_right(false))
            }))
            .on_action(cx.listener(|this, _: &SelectWordLeft, _, cx| {
                this.apply(cx, |s| s.move_word_left(true))
            }))
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| {
                this.apply(cx, |s| s.move_word_right(true))
            }))
            .on_action(cx.listener(|this, _: &MoveLineStart, _, cx| {
                this.apply(cx, |s| s.move_line_start(false))
            }))
            .on_action(cx.listener(|this, _: &MoveLineEnd, _, cx| {
                this.apply(cx, |s| s.move_line_end(false))
            }))
            .on_action(cx.listener(|this, _: &SelectLineStart, _, cx| {
                this.apply(cx, |s| s.move_line_start(true))
            }))
            .on_action(cx.listener(|this, _: &SelectLineEnd, _, cx| {
                this.apply(cx, |s| s.move_line_end(true))
            }))
            .on_action(cx.listener(|this, _: &MoveDocStart, _, cx| {
                this.apply(cx, |s| s.move_doc_start(false))
            }))
            .on_action(
                cx.listener(|this, _: &MoveDocEnd, _, cx| {
                    this.apply(cx, |s| s.move_doc_end(false))
                }),
            )
            .on_action(cx.listener(|this, _: &SelectDocStart, _, cx| {
                this.apply(cx, |s| s.move_doc_start(true))
            }))
            .on_action(
                cx.listener(|this, _: &SelectDocEnd, _, cx| {
                    this.apply(cx, |s| s.move_doc_end(true))
                }),
            )
            // Vertical (visual rows, with goal x)
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.vertical(-1, false, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.vertical(1, false, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.vertical(-1, true, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.vertical(1, true, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| {
                let rows = this.page_rows();
                this.vertical(-rows, false, cx)
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| {
                let rows = this.page_rows();
                this.vertical(rows, false, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectPageUp, _, cx| {
                let rows = this.page_rows();
                this.vertical(-rows, true, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectPageDown, _, cx| {
                let rows = this.page_rows();
                this.vertical(rows, true, cx)
            }))
            // Editing
            .on_action(cx.listener(|this, _: &Backspace, _, cx| this.backspace_md(cx)))
            .on_action(
                cx.listener(|this, _: &Delete, _, cx| this.apply(cx, |s| s.delete_forward())),
            )
            .on_action(cx.listener(|this, _: &DeleteWordBackward, _, cx| {
                this.apply(cx, |s| s.delete_word_backward())
            }))
            .on_action(cx.listener(|this, _: &DeleteWordForward, _, cx| {
                this.apply(cx, |s| s.delete_word_forward())
            }))
            .on_action(cx.listener(|this, _: &InsertNewline, _, cx| this.insert_newline(cx)))
            .on_action(cx.listener(|this, _: &InsertTab, _, cx| this.indent(false, cx)))
            .on_action(cx.listener(|this, _: &OutdentTab, _, cx| this.indent(true, cx)))
            .on_action(
                cx.listener(|this, _: &MoveLineUp, _, cx| this.apply(cx, |s| s.move_line_up())),
            )
            .on_action(
                cx.listener(|this, _: &MoveLineDown, _, cx| this.apply(cx, |s| s.move_line_down())),
            )
            .on_action(
                cx.listener(|this, _: &DuplicateLine, _, cx| {
                    this.apply(cx, |s| s.duplicate_line())
                }),
            )
            .on_action(cx.listener(|this, _: &InsertFootnote, _, cx| {
                this.insert_footnote(cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.apply(cx, |s| s.select_all())))
            .on_action(cx.listener(|this, _: &Undo, _, cx| {
                this.apply(cx, |s| {
                    s.undo();
                })
            }))
            .on_action(cx.listener(|this, _: &Redo, _, cx| {
                this.apply(cx, |s| {
                    s.redo();
                })
            }))
            .on_action(cx.listener(|this, _: &CopyText, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &CutText, _, cx| this.cut(cx)))
            .on_action(cx.listener(|this, _: &PasteText, _, cx| this.paste(cx)))
            // Mouse and wheel
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(TextElement {
                editor: cx.entity(),
            })
    }
}

// ----- tests -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_ascii() {
        assert_eq!(utf16_to_utf8("abc", 0), 0);
        assert_eq!(utf16_to_utf8("abc", 2), 2);
        assert_eq!(utf16_to_utf8("abc", 99), 3);
        assert_eq!(utf8_to_utf16("abc", 2), 2);
    }

    #[test]
    fn utf16_offsets_cjk() {
        // 你 is 3 bytes and 1 UTF-16 unit.
        let t = "你好吗";
        assert_eq!(utf16_to_utf8(t, 1), 3);
        assert_eq!(utf16_to_utf8(t, 2), 6);
        assert_eq!(utf8_to_utf16(t, 6), 2);
        assert_eq!(utf8_to_utf16(t, 9), 3);
    }

    #[test]
    fn utf16_offsets_emoji_surrogate_pairs() {
        // 😀 is 4 bytes and 2 UTF-16 units. a=1 byte/1 unit, b=1/1.
        let t = "a😀b";
        assert_eq!(utf16_to_utf8(t, 1), 1);
        assert_eq!(utf16_to_utf8(t, 3), 5);
        assert_eq!(utf16_to_utf8(t, 4), 6);
        assert_eq!(utf16_to_utf8(t, 5), 6);
        assert_eq!(utf8_to_utf16(t, 5), 3);
        assert_eq!(utf8_to_utf16(t, 6), 4);
    }

    #[test]
    fn utf16_inside_emoji_rounds_up() {
        // Offset 1 is between the two surrogate halves of the emoji and rounds up past it.
        assert_eq!(utf16_to_utf8("😀x", 1), 4);
        // Byte offset 2 is inside the 3-byte char 你, so it rounds up past it.
        assert_eq!(utf8_to_utf16("你x", 2), 1);
    }

    #[test]
    fn utf16_round_trip_on_char_boundaries() {
        let t = "中a😀文b";
        let mut bytes = 0;
        for ch in t.chars() {
            let units = utf8_to_utf16(t, bytes);
            assert_eq!(utf16_to_utf8(t, units), bytes);
            bytes += ch.len_utf8();
        }
        assert_eq!(utf16_to_utf8(t, utf8_to_utf16(t, t.len())), t.len());
    }

    #[test]
    fn utf16_range_conversions() {
        let t = "你好😀";
        // UTF-16 1..3 covers 好 and the emoji (its end at 3 rounds up to the emoji's end).
        let r = range_utf16_to_utf8(t, &(1..3));
        assert_eq!(r, 3..10);
        assert_eq!(&t[r], "好😀");
        assert_eq!(range_utf8_to_utf16(t, &(3..10)), 1..4);
        // Reversed input is ordered.
        let (start, end) = (3usize, 1usize);
        assert_eq!(range_utf16_to_utf8(t, &(start..end)), 3..10);
    }

    #[test]
    fn clipboard_line_endings_become_lf() {
        assert_eq!(clipboard_to_text("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(clipboard_to_text(""), "");
    }

    #[test]
    fn scroll_reveal_cases() {
        // Already visible: unchanged.
        assert_eq!(scroll_to_reveal(100.0, 126.0, 0.0, 200.0), 0.0);
        // Below the viewport: scroll so the bottom sits at the bottom edge.
        assert_eq!(scroll_to_reveal(100.0, 126.0, 0.0, 110.0), 16.0);
        // Above the viewport: scroll up to the top.
        assert_eq!(scroll_to_reveal(10.0, 36.0, 50.0, 200.0), 10.0);
    }

    #[test]
    fn vertical_row_centres() {
        // Row 1 top is 26, so its centre is 39; down one row gives row 2 centre 65.
        assert_eq!(row_centre_y(26.0, 1), 65.0);
        assert_eq!(row_centre_y(26.0, -1), 13.0);
        assert_eq!(row_of(0.0, LINE_HEIGHT), 0);
        assert_eq!(row_of(52.0, LINE_HEIGHT), 2);
        assert_eq!(row_of(40.0, 44.0), 1);
    }

    #[test]
    fn single_typed_char_accepts_one_printable_char() {
        assert_eq!(single_typed_char("("), Some('('));
        assert_eq!(single_typed_char("（"), Some('（'));
        assert_eq!(single_typed_char("你"), Some('你'));
    }

    #[test]
    fn single_typed_char_rejects_multi_char_and_control() {
        assert_eq!(single_typed_char(""), None);
        assert_eq!(single_typed_char("ab"), None);
        assert_eq!(single_typed_char("你好"), None);
        assert_eq!(single_typed_char("\n"), None);
        assert_eq!(single_typed_char("\t"), None);
    }

    #[test]
    fn plan_selection_copies_offsets() {
        let plan = EditPlan {
            start: 1,
            end: 2,
            insert: "xy".to_string(),
            sel_anchor: 3,
            sel_head: 5,
        };
        assert_eq!(plan_selection(&plan), Selection { anchor: 3, head: 5 });
    }

    #[test]
    fn layout_index_for_empty_layout_is_zero() {
        let layout = Layout::default();
        assert_eq!(layout.index_at(10.0, 10.0, 5), 5);
        assert_eq!(layout.index_at(10.0, -1.0, 5), 0);
        assert_eq!(layout.point_for_index(3), (0.0, 0.0));
    }

    /// A logical line without shaped text, for geometry tests.
    fn bare_line(start: usize, len: usize, top: f32, rows: usize, line_h: f32) -> LineLayout {
        LineLayout {
            start,
            len,
            top,
            rows,
            line_h,
            wrapped: None,
            pieces: Vec::new(),
            widget: None,
            inline: Vec::new(),
            built: true,
        }
    }

    /// Three lines: plain (26 px), H2 (36 px tall rows), plain. Starts 0, 4 and 9.
    fn mixed_layout() -> Layout {
        Layout {
            origin_x: 0.0,
            origin_y: 0.0,
            wrap_width: 500.0,
            viewport_h: 400.0,
            checks: Vec::new(),
            buttons: Vec::new(),
            fold_hits: Vec::new(),
            links: Vec::new(),
            footnote_backs: Vec::new(),
            lines: vec![
                bare_line(0, 3, 0.0, 1, LINE_HEIGHT),
                bare_line(4, 4, 26.0, 1, 36.0),
                bare_line(9, 2, 62.0, 2, LINE_HEIGHT),
            ],
        }
    }

    fn unit_at(start: usize) -> LineLayout {
        LineLayout {
            start,
            len: 1,
            top: 0.0,
            rows: 1,
            line_h: 10.0,
            wrapped: None,
            pieces: Vec::new(),
            widget: None,
            inline: Vec::new(),
            built: true,
        }
    }

    /// A widget built at offset `at` with a copy button over `at + 5..at + 9` and one table cell
    /// whose source is `at + 2..at + 7` (what `code_widget` and `table_widget` record).
    fn widget_at(at: usize) -> Widget {
        Widget {
            button: Some(((0.0, 0.0, 10.0, 10.0), at + 5..at + 9)),
            cells: vec![CellHit {
                rect: (0.0, 0.0, 1.0, 1.0),
                src: at + 2..at + 7,
                pieces: vec![map::Piece {
                    src: at + 2..at + 7,
                    style: SpanStyle::default(),
                    replace: None,
                    deco: map::Deco::None,
                    ime: false,
                    gap: 0,
                }],
                wrapped: None,
                text_x: 0.0,
                text_y: 0.0,
            }],
            ..Widget::default()
        }
    }

    #[test]
    fn widget_reused_at_another_offset_points_at_its_own_source() {
        // Built at 0 and reused at 10 (an edit of 10 bytes above it): the copy button and the table
        // cell must name the text at 10, not the text at 0.
        let mut w = widget_at(0);
        shift_widget(&mut w, 10);
        assert_eq!(w.button.as_ref().map(|(_, b)| b.clone()), Some(15..19));
        assert_eq!(w.cells[0].src, 12..17);
        assert_eq!(w.cells[0].pieces[0].src, 12..17);
        // Moved back: the same offsets as at the start.
        shift_widget(&mut w, -10);
        assert_eq!(w.button.as_ref().map(|(_, b)| b.clone()), Some(5..9));
    }

    #[test]
    fn shifted_unit_moves_its_pieces_and_widget() {
        let mut l = unit_at(4);
        l.pieces.push(map::Piece {
            src: 5..6,
            style: SpanStyle::default(),
            replace: None,
            deco: map::Deco::None,
            ime: false,
            gap: 0,
        });
        l.widget = Some(Box::new(widget_at(4)));
        l.shift(-2);
        assert_eq!(l.start, 2);
        assert_eq!(l.pieces[0].src, 3..4);
        assert_eq!(
            l.widget
                .as_ref()
                .and_then(|w| w.button.clone())
                .map(|(_, b)| b),
            Some(7..11)
        );
    }

    #[test]
    fn placeholder_is_not_built_and_places_below_its_predecessor() {
        let mut lines = vec![placeholder(0, 3, 2, 10.0), placeholder(4, 1, 1, 26.0)];
        place_lines(&mut lines);
        assert_eq!(lines[1].top, 20.0);
        assert!(!lines[0].built);
    }

    /// Three placeholder units of two bytes (`ab`, `cd`, `ef`) and one row of 10 px each.
    fn three_units() -> Vec<LineLayout> {
        [0usize, 3, 6]
            .into_iter()
            .map(|s| placeholder(s, 2, 1, 10.0))
            .collect()
    }

    #[test]
    fn splice_region_of_an_insert_is_its_line() {
        let old = "ab
cd
ef";
        let new = "ab
cXd
ef";
        let span = invalidate::edit_span(old, new);
        let split = [(0usize, 2usize), (3, 3), (7, 2)];
        let plan = plan_splice(
            &span,
            &three_units(),
            &split,
            &[],
            &[false; 3],
            &[0; 3],
            &[0; 3],
            0,
        );
        // Only line 1 is rebuilt: unit 1 is replaced by it and unit 2 moves by one byte.
        assert_eq!(plan, Some((1, 2, 1, 2)));
    }

    #[test]
    fn splice_region_of_a_merge_covers_both_lines() {
        let old = "ab
cd
ef";
        let new = "abcd
ef";
        let span = invalidate::edit_span(old, new);
        let split = [(0usize, 4usize), (5, 2)];
        // The old text had three lines, the new one has two: the signatures are per line of each.
        let plan = plan_splice(
            &span,
            &three_units(),
            &split,
            &[],
            &[false; 2],
            &[0; 3],
            &[0; 2],
            -1,
        );
        assert_eq!(plan, Some((0, 2, 0, 1)));
    }

    #[test]
    fn take_prev_finds_units_once_in_order() {
        let mut prev: Vec<LineLayout> = [0, 2, 5, 9].into_iter().map(unit_at).collect();
        let mut cursor = 0usize;
        // A start that no unit has (unit at 1 was skipped) finds nothing and moves nothing.
        assert!(take_prev(&mut prev, &mut cursor, 1).is_none());
        assert_eq!(
            take_prev(&mut prev, &mut cursor, 2).map(|l| l.start),
            Some(2)
        );
        // Taking the same start twice gives nothing the second time.
        assert!(take_prev(&mut prev, &mut cursor, 2).is_none());
        assert_eq!(
            take_prev(&mut prev, &mut cursor, 5).map(|l| l.start),
            Some(5)
        );
        // Units passed over by a later start are skipped, not returned.
        assert_eq!(
            take_prev(&mut prev, &mut cursor, 9).map(|l| l.start),
            Some(9)
        );
        assert!(take_prev(&mut prev, &mut cursor, 100).is_none());
    }

    #[test]
    fn heights_follow_each_line() {
        let l = mixed_layout();
        assert_eq!(l.content_height(), 62.0 + 2.0 * LINE_HEIGHT);
        assert_eq!(l.row_height_at(0), LINE_HEIGHT);
        assert_eq!(l.row_height_at(5), 36.0);
        assert_eq!(l.row_height_at(10), LINE_HEIGHT);
        // A click in the middle of the H2 row (26..62) lands on its line, not the next one.
        assert_eq!(l.index_at(0.0, 50.0, 11), 4);
        assert_eq!(l.index_at(0.0, 30.0, 11), 4);
        assert_eq!(l.index_at(0.0, 65.0, 11), 9);
        assert_eq!(l.index_at(0.0, 999.0, 11), 11);
    }

    #[test]
    fn vertical_steps_cross_rows_of_different_heights() {
        let l = mixed_layout();
        // Plain row centre 13, H2 centre 26 + 18 = 44, the two-row line starts at 62 with its
        // rows centred at 75 and 101.
        assert_eq!(l.adjacent_row_centre(13.0, 1), Some(44.0));
        assert_eq!(l.adjacent_row_centre(44.0, 1), Some(75.0));
        assert_eq!(l.adjacent_row_centre(75.0, 1), Some(101.0));
        assert_eq!(l.adjacent_row_centre(101.0, 1), None);
        assert_eq!(l.adjacent_row_centre(75.0, -1), Some(44.0));
        assert_eq!(l.adjacent_row_centre(44.0, -1), Some(13.0));
        assert_eq!(l.adjacent_row_centre(13.0, -1), None);
    }

    #[test]
    fn vertical_steps_inside_a_wrapped_line() {
        let l = Layout {
            lines: vec![bare_line(0, 10, 0.0, 3, 36.0)],
            ..mixed_layout()
        };
        assert_eq!(l.adjacent_row_centre(18.0, 1), Some(54.0));
        assert_eq!(l.adjacent_row_centre(54.0, 1), Some(90.0));
        assert_eq!(l.adjacent_row_centre(90.0, 1), None);
        assert_eq!(l.adjacent_row_centre(90.0, -1), Some(54.0));
    }

    #[test]
    fn line_metrics_for_headings_and_code() {
        use md_style::SpanStyle as S;
        let seg = |style: S| Segment { range: 0..1, style };
        assert_eq!(line_metrics(&[]), (FONT_SIZE, LINE_HEIGHT));
        assert_eq!(line_metrics(&[seg(S::heading(1))]), (28.0, 44.0));
        assert_eq!(line_metrics(&[seg(S::heading(2))]), (22.0, 36.0));
        assert_eq!(line_metrics(&[seg(S::heading(3))]), (18.0, 30.0));
        assert_eq!(line_metrics(&[seg(S::heading(4))]), (16.0, 28.0));
        assert_eq!(line_metrics(&[seg(S::heading(6))]), (15.0, LINE_HEIGHT));
        assert_eq!(
            line_metrics(&[seg(S::of(S::CODE_BLOCK))]),
            (CODE_FONT_SIZE, LINE_HEIGHT)
        );
    }

    #[test]
    fn look_colours_follow_the_theme_tokens() {
        use md_style::SpanStyle as S;
        let t = ActiveTheme::from_kind(crate::theme::ThemeKind::Ink);
        let plain = look_for(S::default(), &t);
        assert_eq!(plain.color, t.tx);
        assert_eq!(plain.family, Face::Editor);
        assert_eq!(look_for(S::of(S::MARKER), &t).color, t.tx3);
        assert_eq!(look_for(S::of(S::LINK_URL), &t).color, t.tx3);
        assert_eq!(look_for(S::of(S::LINK_TEXT), &t).color, t.ac);
        assert!(look_for(S::of(S::LINK_TEXT), &t).underline);
        assert_eq!(look_for(S::of(S::WIKILINK), &t).color, t.ac);
        assert_eq!(look_for(S::of(S::TAG), &t).color, t.ac);
        assert_eq!(look_for(S::of(S::LIST_MARKER), &t).color, t.ac);
        assert_eq!(look_for(S::of(S::QUOTE), &t).color, t.tx2);
        assert_eq!(look_for(S::of(S::CODE_FENCE), &t).color, t.tx2);
        assert_eq!(look_for(S::of(S::CODE_BLOCK), &t).family, Face::Code);
        let code = look_for(S::of(S::CODE), &t);
        assert_eq!(code.family, Face::Code);
        assert_eq!(code.background, Some(t.raised));
        assert_eq!(look_for(S::of(S::BOLD), &t).weight, FontWeight::BOLD);
        assert!(look_for(S::of(S::ITALIC), &t).italic);
        assert!(look_for(S::of(S::STRIKE), &t).strike);
        let h1 = look_for(S::heading(1), &t);
        assert_eq!(h1.family, Face::Heading);
        assert_eq!(h1.weight, FontWeight::BOLD);
        assert_eq!(look_for(S::heading(4), &t).weight, FontWeight::SEMIBOLD);
    }

    #[test]
    fn line_runs_cover_the_line_exactly() {
        use md_style::SpanStyle as S;
        let t = ActiveTheme::from_kind(crate::theme::ThemeKind::Fog);
        // "**ab** c" with bold on 2..4 and a marker on 0..2 and 4..6.
        let segs = vec![
            Segment {
                range: 0..2,
                style: S::of(S::MARKER),
            },
            Segment {
                range: 2..4,
                style: S::of(S::BOLD),
            },
            Segment {
                range: 4..6,
                style: S::of(S::MARKER),
            },
            Segment {
                range: 6..8,
                style: S::default(),
            },
        ];
        let runs = line_runs(&t, &segs, None, 0, 8);
        let total: usize = runs.iter().map(|r| r.len).sum();
        assert_eq!(total, 8);
        assert_eq!(runs.len(), 4);
        assert_eq!(runs[1].font.weight, FontWeight::BOLD);
        assert_eq!(runs[0].color, Hsla::from(t.tx3));
    }

    #[test]
    fn line_runs_split_at_ime_range() {
        let t = ActiveTheme::from_kind(crate::theme::ThemeKind::Fog);
        let runs = line_runs(&t, &[], Some(&(2..5)), 0, 8);
        let lens: Vec<usize> = runs.iter().map(|r| r.len).collect();
        assert_eq!(lens, vec![2, 3, 3]);
        assert!(runs[1].underline.is_some());
        assert!(runs[0].underline.is_none());
    }

    #[test]
    fn line_runs_respect_a_line_offset() {
        use md_style::SpanStyle as S;
        let t = ActiveTheme::from_kind(crate::theme::ThemeKind::Fog);
        // The line starts at byte 10; its segment covers absolute 10..13.
        let segs = vec![Segment {
            range: 10..13,
            style: S::of(S::TAG),
        }];
        let runs = line_runs(&t, &segs, None, 10, 3);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, 3);
        assert_eq!(runs[0].color, Hsla::from(t.ac));
    }

    /// Applies a `minimal_edit` the way the editor does and checks the result.
    fn apply(old: &str, new: &str) -> String {
        let (range, insert) = minimal_edit(old, new);
        let mut out = String::from(&old[..range.start]);
        out.push_str(&insert);
        out.push_str(&old[range.end..]);
        out
    }

    #[test]
    fn minimal_edit_round_trips_and_keeps_the_common_parts() {
        let cases = [
            ("", ""),
            ("abc", "abc"),
            ("abc", ""),
            ("", "abc"),
            ("- [ ] 任务\n", "- [x] 任务\n"),
            ("tags: [a]", "tags: [a, b]"),
            ("你好世界", "你好啊世界"),
            ("aaa", "aaaa"),
            ("abab", "ab"),
            ("完全不同", "另一段"),
        ];
        for (old, new) in cases {
            assert_eq!(apply(old, new), new, "{old:?} -> {new:?}");
        }
        // A checkbox flip changes exactly one character.
        let (range, insert) = minimal_edit("- [ ] 任务\n", "- [x] 任务\n");
        assert_eq!(range, 3..4);
        assert_eq!(insert, "x");
        // Identical text is an empty edit.
        let (range, insert) = minimal_edit("同样", "同样");
        assert!(range.is_empty());
        assert!(insert.is_empty());
    }

    #[test]
    fn hand_cursor_follows_the_mode() {
        // Read mode: any link or clickable widget.
        assert!(hand_cursor(true, true, false, false, true));
        assert!(hand_cursor(true, true, false, true, false));
        assert!(!hand_cursor(true, true, false, false, false));
        // Live preview: text links need Ctrl, widgets do not.
        assert!(!hand_cursor(false, true, false, false, true));
        assert!(hand_cursor(false, true, true, false, true));
        assert!(hand_cursor(false, true, false, true, false));
        // Source mode: always the I-beam.
        assert!(!hand_cursor(false, false, true, true, true));
    }

    #[test]
    fn link_spans_cover_wikilinks_and_markdown_links() {
        let t = "see [site](https://x.y) and [[Note]] end";
        let spans = md_style::compute_spans(t);
        assert!(link_span_at(&spans, t.find("site").unwrap()));
        assert!(link_span_at(&spans, t.find("https").unwrap()));
        assert!(link_span_at(&spans, t.find("Note").unwrap()));
        assert!(!link_span_at(&spans, t.find("see").unwrap()));
        assert!(!link_span_at(&spans, t.find("end").unwrap()));
    }
}
