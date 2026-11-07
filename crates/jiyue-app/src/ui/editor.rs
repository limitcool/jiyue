//! The editor column: document title, properties, the markdown body (live preview),
//! and the floating 源码 / 实时预览 / 阅读 pill.
//!
//! Clicks in the preview: a block in Live mode moves the caret to its source line and an
//! estimated column (see `Caret`). A task checkbox toggles the task in the note text in both
//! Live and Read mode. Wikilink chips are handled in `inline.rs`.

use std::time::UNIX_EPOCH;

use gpui::{
    AnyElement, ClickEvent, ClipboardItem, Context, Div, ElementId, ExternalPaths, FontWeight,
    IntoElement, MouseButton, Pixels, Point, Rgba, SharedString, StyledText, TextLayout, div,
    linear_color_stop, linear_gradient, prelude::*, px, relative,
};

use super::display_family;
use super::highlight;
use super::icons::icon;
use super::inline::{rich_text, rich_text_geo};
use crate::RustdianApp;
use crate::buffer::Note;
use crate::daily::Date;
use crate::session::ViewMode;
use crate::theme::ActiveTheme;
use crate::ui::fonts;

/// Renders the whole editor area for `note`. `show_pill` adds the mode switcher.
pub fn render_editor(
    app: &RustdianApp,
    note: &Note,
    show_pill: bool,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let family = display_family(&t);
    // Only the primary pane has the mode switch; the secondary pane always shows the rendered note.
    let mode = if show_pill {
        app.view_mode
    } else {
        ViewMode::Read
    };

    let mut fade_top = t.sheet;
    fade_top.a = 0.0;

    // Files dragged from the OS onto the note are imported (see `attach_ui::drop_files`).
    let drop_tint = t.ac_soft;
    let mut area = div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_tint))
        .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
            super::attach_ui::drop_files(this, paths.paths(), cx);
            cx.notify();
        }));
    // Source and Live both edit the note in the text editor. Live hides the markers away from the
    // caret (the editor's live view); Source shows every marker. Read shows the rendered note.
    let editing = matches!(mode, ViewMode::Source | ViewMode::Live);
    // The primary pane's Read mode is the live editor in read-only mode: the same widgets, no raw
    // source and no typing (see `TextEditor::set_read_only`). The secondary pane keeps `preview`.
    let read_editor = if show_pill && mode == ViewMode::Read {
        app.editor_for(&note.path)
    } else {
        None
    };
    if editing {
        // The editor scrolls itself. The app creates it for the active note before rendering.
        if let Some(editor) = app.editor_for(&note.path) {
            editor.update(cx, |ed, cx| {
                ed.set_read_only(false, cx);
                ed.set_live(mode == ViewMode::Live, cx);
            });
            area = area.child(div().size_full().child(editor));
        }
    } else if let Some(editor) = read_editor {
        editor.update(cx, |ed, cx| {
            ed.set_read_only(true, cx);
            ed.set_live(true, cx);
        });
        area = area.child(div().size_full().child(editor));
    } else {
        let body = preview(app, note, family, mode == ViewMode::Live, cx);
        area = area.child(
            div()
                .id("doc_scroll")
                .size_full()
                .overflow_y_scroll()
                .child(body),
        );
    }

    if !editing {
        area = area.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(90.0))
                .bg(linear_gradient(
                    0.0,
                    linear_color_stop(t.sheet, 0.12),
                    linear_color_stop(fade_top, 1.0),
                )),
        );
    }

    if show_pill {
        area = area.child(mode_pill(app, cx));
    }
    area
}

/// The body of the pane that does not have the focus. A note with its own editor is edited
/// there (Live view). A note that the focused pane edits is shown as its rendered text: it
/// follows the edits, and only one editor exists for it, so the buffer and its undo are shared.
pub fn render_secondary(
    app: &RustdianApp,
    note: &Note,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let family = display_family(&t);
    let mut area = div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden();
    let own_editor = if app.active_path() != Some(note.path.as_str()) {
        app.editor_for(&note.path)
    } else {
        None
    };
    match own_editor {
        Some(editor) => {
            editor.update(cx, |ed, cx| {
                ed.set_read_only(false, cx);
                ed.set_live(true, cx);
            });
            area = area.child(div().size_full().child(editor));
        }
        None => {
            let body = preview(app, note, family, true, cx);
            area = area.child(
                div()
                    .id("doc_scroll_secondary")
                    .size_full()
                    .overflow_y_scroll()
                    .child(body),
            );
        }
    }
    area
}

/// The live-preview document: centred 620px column with the title block and body.
fn preview(
    app: &RustdianApp,
    note: &Note,
    family: &'static str,
    live: bool,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let content = note.content();
    let mut lines: Vec<&str> = content.lines().collect();
    // The frontmatter block is shown by the 属性 panel, not in the body. Its lines are blanked so
    // the source line numbers stay the same.
    let block = super::attach_ui::props::leading_block_lines(content);
    for line in lines.iter_mut().take(block) {
        *line = "";
    }

    let props = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(14.0))
        .gap_y(px(6.0))
        .mb(px(20.0))
        .text_size(px(12.5))
        .text_color(t.tx3)
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .children(note.tags.iter().map(|tag| {
                    div()
                        .flex()
                        .items_center()
                        .h(px(22.0))
                        .px(px(8.0))
                        .rounded_full()
                        .bg(t.ac_soft)
                        .text_color(t.ac)
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("#{}", tag))
                })),
        )
        .child(meta_item("calendar", &created_label(app, note), t.tx3))
        .child(meta_item(
            "text-size",
            &format!("{} 字", super::char_count_no_ws(content)),
            t.tx3,
        ));

    let stack = [note.path.clone()];
    let clicks = Clicks {
        path: &note.path,
        caret: live,
        embed: false,
        depth: 0,
        stack: &stack,
    };
    let body = body_blocks(app, &t, &lines, &note.title, family, clicks, cx);

    // Live mode: a click on the empty space of the document goes to the end of the note. Clicks
    // on a block stop there (see `caret_block`).
    let root = div().id("live_doc").flex().justify_center().w_full();
    let root = if live {
        let path = note.path.clone();
        root.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.click_rendered_text(&path);
            cx.notify();
        }))
    } else {
        root
    };

    root.child(
        div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(620.0))
            .pt(px(30.0))
            .pb(px(80.0))
            .font_family(fonts::face_name(fonts::Face::Editor, t.kind))
            .child(
                div()
                    .font_family(family)
                    .text_size(px(32.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(relative(1.25))
                    .text_color(t.tx)
                    .mb(px(12.0))
                    .child(note.title.clone()),
            )
            .child(props)
            .children(body),
    )
}

fn meta_item(icon_name: &str, text: &str, color: Rgba) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(icon(icon_name, 14.0, color))
        .child(div().child(SharedString::from(text.to_string())))
}

/// 创建于 date, from the file's creation time (falls back to modification time).
fn created_label(app: &RustdianApp, note: &Note) -> String {
    let path = app.vault.root_dir.join(&note.path);
    let created = std::fs::metadata(&path)
        .and_then(|m| m.created().or_else(|_| m.modified()))
        .ok()
        .and_then(|st| st.duration_since(UNIX_EPOCH).ok());
    match created {
        Some(d) => {
            let date = Date::from_days((d.as_secs() / 86_400) as i64);
            format!("创建于 {:04}-{:02}-{:02}", date.year, date.month, date.day)
        }
        None => "创建于 未知".to_string(),
    }
}

// ----- click targets ---------------------------------------------------------------------

/// How the rendered blocks react to clicks. `path` is the note. `caret` is true in Live mode,
/// where a click on a block moves the caret in the source. Task checkboxes work in both modes.
#[derive(Clone, Copy)]
pub(crate) struct Clicks<'a> {
    /// The note whose text is shown (the embedded note inside an embed).
    pub(crate) path: &'a str,
    pub(crate) caret: bool,
    /// Rendered inside a note embed: no caret moves, no fold toggles, no task toggles.
    pub(crate) embed: bool,
    /// How many note embeds surround this text (0 in the editor).
    pub(crate) depth: u8,
    /// Paths of the notes being shown, outermost first (for cycle checks).
    pub(crate) stack: &'a [String],
}

impl<'a> Clicks<'a> {
    /// Clicks for the text of an embedded note: read only.
    pub(crate) fn embedded(path: &'a str, depth: u8, stack: &'a [String]) -> Self {
        Self {
            path,
            caret: false,
            embed: true,
            depth,
            stack,
        }
    }
}

impl Clicks<'_> {
    /// Caret target for a block whose rendered text is `content` (laid out by `text`). `line` is
    /// the source line; the marker before `content` is skipped by the estimate.
    fn caret_text(
        self,
        lineno: usize,
        line: &str,
        content: &str,
        text: TextLayout,
    ) -> Option<Caret> {
        self.caret
            .then(|| Caret::on_text(self.path, lineno, line, content, text))
    }

    /// Caret target at the end of source line `lineno`, for blocks that span several lines.
    fn caret_end(self, lineno: usize, line: &str) -> Option<Caret> {
        self.caret.then(|| Caret::at_end(self.path, lineno, line))
    }
}

/// Where a click on a rendered block puts the caret in the source.
///
/// The caret row is the block's source line. For a text block the column comes from GPUI's hit
/// test on the rendered text (the nearest character, wrapped rows included), mapped back to the
/// source with `source_index_for_display`, and it skips the marker before the text. Clicks left
/// of the text (bullets, markers) go to the start of the line. Multi-line blocks (quotes,
/// callouts, tables, code) use the end of their first line.
#[derive(Clone)]
struct Caret {
    path: String,
    /// Zero-based source line.
    row: usize,
    /// Length of the source line in chars.
    line_chars: usize,
    /// Chars of the source line before the rendered text (indent and marker).
    prefix: usize,
    /// The rendered text of the block (as in the source).
    content: String,
    /// Measured text of the block; `None` puts the caret at the end of the line.
    text: Option<TextLayout>,
}

impl Caret {
    fn on_text(path: &str, lineno: usize, line: &str, content: &str, text: TextLayout) -> Self {
        let line_chars = line.chars().count();
        Self {
            path: path.to_string(),
            row: lineno.saturating_sub(1),
            line_chars,
            prefix: line_chars.saturating_sub(content.chars().count()),
            content: content.to_string(),
            text: Some(text),
        }
    }

    fn at_end(path: &str, lineno: usize, line: &str) -> Self {
        let line_chars = line.chars().count();
        Self {
            path: path.to_string(),
            row: lineno.saturating_sub(1),
            line_chars,
            prefix: line_chars,
            content: String::new(),
            text: None,
        }
    }

    /// The (row, column) a click at window position `pos` maps to. Column is in chars.
    fn target(&self, pos: Point<Pixels>) -> (usize, usize) {
        let Some(layout) = self.text.as_ref() else {
            return (self.row, self.line_chars);
        };
        // Left of the text (a bullet or a marker): the start of the line.
        if pos.x < layout.bounds().origin.x {
            return (self.row, 0);
        }
        // GPUI's own hit test gives the nearest character of the rendered text (wrapped rows
        // included); that position is mapped back to the source text.
        let display = layout.text();
        let at = match layout.index_for_position(pos) {
            Ok(i) | Err(i) => i.min(display.len()),
        };
        let at = (0..=at)
            .rev()
            .find(|&i| display.is_char_boundary(i))
            .unwrap_or(0);
        let shown = display[..at].chars().count();
        let in_content = source_index_for_display(&self.content, &display, shown);
        (self.row, (self.prefix + in_content).min(self.line_chars))
    }
}

/// Makes `el` a click target that moves the caret in Live mode. `id` is the block's source line,
/// so it is unique among the blocks. Clicks that stop here do not reach the document-wide
/// handler (which goes to the end of the note).
fn caret_block(
    id: usize,
    caret: Option<Caret>,
    el: Div,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let el = el.id(ElementId::NamedInteger(
        SharedString::from("blk"),
        id as u64,
    ));
    match caret {
        Some(caret) => el
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                let (row, col) = caret.target(ev.position());
                cx.stop_propagation();
                this.click_rendered_line(&caret.path, row, col);
                cx.notify();
            }))
            .into_any_element(),
        None => el.into_any_element(),
    }
}

/// Maps a character index of the shown (rendered) text to the matching character index of the
/// source text `source`. Markdown markers that are not shown (`*`, `` ` ``, `[`, `]`, `~`) are
/// skipped on the source side, and the padding around a wikilink chip (thin spaces) on the shown
/// side. The result is the source index of the character that follows `shown` shown characters.
pub fn source_index_for_display(source: &str, display: &str, shown: usize) -> usize {
    const HIDDEN: [char; 5] = ['*', '`', '[', ']', '~'];
    let src: Vec<char> = source.chars().collect();
    let out: Vec<char> = display.chars().collect();
    let (mut s, mut d) = (0usize, 0usize);
    while s < src.len() && d < shown && d < out.len() {
        let c = src[s];
        if c == out[d] {
            s += 1;
            d += 1;
        } else if HIDDEN.contains(&c) {
            s += 1;
        } else if out[d] == '\u{2009}' {
            d += 1;
        } else {
            s += 1;
            d += 1;
        }
    }
    s
}

// ----- blocks ----------------------------------------------------------------------------

/// Converts the markdown body into elements. The leading H1 is the title and is skipped.
pub(crate) fn body_blocks(
    app: &RustdianApp,
    t: &ActiveTheme,
    lines: &[&str],
    _title: &str,
    family: &'static str,
    clicks: Clicks,
    cx: &mut Context<RustdianApp>,
) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = Vec::new();
    let mut i = 0;
    let mut collapsed_level: Option<usize> = None;
    // An embedded note keeps its own leading H1 (it is not the document title).
    let mut seen_title = clicks.embed;
    let mut seen_heading = false;

    while i < lines.len() {
        let line = lines[i].trim_end();
        let trimmed = line.trim_start();
        let lineno = i + 1;

        // Skip the body of a collapsed heading section.
        if let Some(level) = collapsed_level {
            match heading_level(trimmed) {
                Some(hl) if hl > level => {
                    i += 1;
                    continue;
                }
                Some(_) => collapsed_level = None,
                None => {
                    i += 1;
                    continue;
                }
            }
        }

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Headings
        if let Some(level) = heading_level(trimmed) {
            if level == 1 && !seen_title {
                seen_title = true;
                i += 1;
                continue;
            }
            seen_heading = true;
            let text = trimmed[level + 1..].trim();
            let collapsed = app.collapsed_headings.contains(&lineno);
            if collapsed {
                collapsed_level = Some(level);
            }
            out.push(heading(
                t, family, level, text, lineno, line, collapsed, clicks, cx,
            ));
            i += 1;
            continue;
        }

        // Fenced code block
        if trimmed.starts_with("```") {
            let lang = trimmed.trim_start_matches('`').trim().to_string();
            let mut code: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            i += 1; // closing fence
            out.push(code_block(t, &lang, &code, clicks, lineno, line, cx));
            continue;
        }

        // Images, files and note embeds: a line that holds one of them and nothing else.
        if let Some(block) = super::attach_ui::view::embed_line(app, clicks, line, lineno, cx) {
            out.push(block);
            i += 1;
            continue;
        }

        // Table
        if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
            let mut rows: Vec<&str> = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if l.starts_with('|') && l.ends_with('|') && l.len() > 1 {
                    rows.push(l);
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(table(t, &rows, clicks, lineno, line, cx));
            continue;
        }

        // Callout: > [!note] title
        if let Some(rest) = trimmed.strip_prefix("> [!") {
            let (kind, title) = match rest.find(']') {
                Some(end) => (&rest[..end], rest[end + 1..].trim()),
                None => (rest, ""),
            };
            let label = if title.is_empty() {
                kind.to_string()
            } else {
                title.to_string()
            };
            let mut body = String::new();
            i += 1;
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let content = lines[i].trim_start().trim_start_matches('>').trim();
                if !body.is_empty() && !content.is_empty() {
                    body.push(' ');
                }
                body.push_str(content);
                i += 1;
            }
            out.push(callout(t, &label, &body, clicks, lineno, line, cx));
            continue;
        }

        // Plain blockquote
        if trimmed.starts_with('>') {
            let mut body = String::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let content = lines[i].trim_start().trim_start_matches('>').trim();
                if !body.is_empty() {
                    body.push(' ');
                }
                body.push_str(content);
                i += 1;
            }
            let quote = div()
                .mb(px(14.0))
                .pl(px(14.0))
                .border_l_2()
                .border_color(t.ac_line)
                .text_size(px(15.0))
                .line_height(relative(1.9))
                .text_color(t.tx2)
                .child(rich_text(
                    &body,
                    t,
                    t.tx2,
                    family,
                    None,
                    false,
                    lineno as u64,
                    cx,
                ));
            let caret = clicks.caret_end(lineno, line);
            out.push(caret_block(lineno, caret, quote, cx));
            continue;
        }

        // Task items
        if let Some((checked, text)) = task_item(trimmed) {
            out.push(task(t, family, checked, text, clicks, lineno, line, cx));
            i += 1;
            continue;
        }

        // Bullet and numbered list items
        if let Some((marker, text)) = list_item(trimmed) {
            let indent = (line.len() - trimmed.len()) as f32 * 8.0;
            let (rendered, layout) =
                rich_text_geo(text, t, t.tx, family, None, false, lineno as u64, cx);
            let row = div()
                .flex()
                .items_start()
                .gap(px(10.0))
                .pl(px(indent))
                .mb(px(2.0))
                .text_size(px(15.0))
                .line_height(relative(1.9))
                .child(match marker {
                    Some(n) => div()
                        .flex_none()
                        .text_color(t.tx3)
                        .child(format!("{}.", n))
                        .into_any_element(),
                    None => div()
                        .flex_none()
                        .mt(px(11.0))
                        .size(px(5.0))
                        .rounded_full()
                        .bg(t.tx3)
                        .into_any_element(),
                })
                .child(div().flex_1().min_w_0().child(rendered));
            let caret = clicks.caret_text(lineno, line, text, layout);
            out.push(caret_block(lineno, caret, row, cx));
            i += 1;
            continue;
        }

        // Horizontal rule
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            let rule = div().w_full().h(px(1.0)).my(px(16.0)).bg(t.line);
            let caret = clicks.caret_end(lineno, line);
            out.push(caret_block(lineno, caret, rule, cx));
            i += 1;
            continue;
        }

        // Tag-only line before the first heading: shown in the properties row instead.
        if !seen_heading
            && trimmed
                .split_whitespace()
                .all(|w| w.starts_with('#') && w.len() > 1)
        {
            i += 1;
            continue;
        }

        // A block id on a line of its own labels the block above: shown dim.
        if let Some(id) = standalone_block_id(trimmed) {
            out.push(block_id_label(t, id));
            i += 1;
            continue;
        }

        // Paragraph. A trailing ` ^id` is shown dim below the text.
        let (main, id_suffix) = split_inline_id(trimmed);
        let (rendered, layout) =
            rich_text_geo(main, t, t.tx, family, None, false, lineno as u64, cx);
        let mut para = div()
            .mb(px(14.0))
            .text_size(px(15.0))
            .line_height(relative(1.9))
            .text_color(t.tx)
            .child(rendered);
        if let Some(id) = id_suffix {
            para = para.child(block_id_label(t, id));
        }
        // The caret maps against the whole source line, so the id is not taken for text.
        let caret = clicks.caret_text(lineno, line, trimmed, layout);
        out.push(caret_block(lineno, caret, para, cx));
        i += 1;
    }

    out
}

/// A block id: letters, digits and `-`.
fn valid_block_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `^id` alone on a line: the label of the block above.
fn standalone_block_id(trimmed: &str) -> Option<&str> {
    let id = trimmed.strip_prefix('^')?;
    valid_block_id(id).then_some(id)
}

/// `text ^id` -> (`text`, Some(`id`)). Other lines come back whole.
fn split_inline_id(trimmed: &str) -> (&str, Option<&str>) {
    if let Some((head, last)) = trimmed.rsplit_once(' ')
        && let Some(id) = last.strip_prefix('^')
        && valid_block_id(id)
        && !head.trim().is_empty()
    {
        return (head.trim_end(), Some(id));
    }
    (trimmed, None)
}

/// The dim `^id` label of a block.
fn block_id_label(t: &ActiveTheme, id: &str) -> AnyElement {
    div()
        .mb(px(8.0))
        .font_family(fonts::code_name())
        .text_size(px(11.5))
        .text_color(t.tx3)
        .child(SharedString::from(format!("^{id}")))
        .into_any_element()
}

/// Returns the heading level (1..=6) of a trimmed line that is an ATX heading.
fn heading_level(trimmed: &str) -> Option<usize> {
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&level) && trimmed.chars().nth(level) == Some(' ') {
        Some(level)
    } else {
        None
    }
}

/// `- [ ] text` / `- [x] text` -> (checked, text).
fn task_item(trimmed: &str) -> Option<(bool, &str)> {
    for prefix in ["- ", "* "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if let Some(text) = rest.strip_prefix("[ ] ") {
                return Some((false, text.trim()));
            }
            if let Some(text) = rest
                .strip_prefix("[x] ")
                .or_else(|| rest.strip_prefix("[X] "))
            {
                return Some((true, text.trim()));
            }
        }
    }
    None
}

/// `- item` -> (None, item); `1. item` -> (Some(1), item).
fn list_item(trimmed: &str) -> Option<(Option<usize>, &str)> {
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return Some((None, rest.trim()));
    }
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && trimmed[digits..].starts_with(". ") {
        let n: usize = trimmed[..digits].parse().ok()?;
        return Some((Some(n), trimmed[digits + 2..].trim()));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn heading(
    t: &ActiveTheme,
    family: &'static str,
    level: usize,
    text: &str,
    lineno: usize,
    line: &str,
    collapsed: bool,
    clicks: Clicks,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let (size, mt, mb) = match level {
        2 => (21.0, 20.0, 8.0),
        3 => (17.0, 16.0, 6.0),
        _ => (15.0, 14.0, 4.0),
    };
    let fold_icon = if collapsed {
        "chevron-right"
    } else {
        "chevron-down"
    };

    let rendered = StyledText::new(SharedString::from(text.to_string()));
    let caret = clicks.caret_text(lineno, line, text, rendered.layout().clone());

    let mut node = div()
        .relative()
        .mt(px(mt))
        .mb(px(mb))
        .font_family(family)
        .text_size(px(size))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.tx)
        .child(rendered);

    if level >= 2 && !clicks.embed {
        node = node.child(
            div()
                .id(ElementId::NamedInteger(
                    SharedString::from("fold"),
                    lineno as u64,
                ))
                .absolute()
                .left(px(-26.0))
                .top(px(4.0))
                .p(px(3.0))
                .rounded(px(5.0))
                .cursor_pointer()
                .text_color(t.tx3)
                .hover(|s| s.bg(t.hover))
                .child(icon(fold_icon, 15.0, t.tx3))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.toggle_heading_collapse(lineno);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                ),
        );
    }
    caret_block(lineno, caret, node, cx)
}

#[allow(clippy::too_many_arguments)]
fn callout(
    t: &ActiveTheme,
    label: &str,
    body: &str,
    clicks: Clicks,
    lineno: usize,
    line: &str,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let block = div()
        .flex()
        .gap(px(11.0))
        .px(px(15.0))
        .py(px(12.0))
        .mt(px(6.0))
        .mb(px(10.0))
        .rounded(px(10.0))
        .bg(t.ac_soft)
        .child(div().mt(px(2.0)).child(icon("bulb", 18.0, t.ac)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.ac)
                        .mb(px(2.0))
                        .child(div().child(SharedString::from(label.to_string()))),
                )
                .child(
                    div()
                        .text_size(px(14.5))
                        .line_height(relative(1.75))
                        .text_color(t.tx)
                        .child(rich_text(
                            body,
                            t,
                            t.tx,
                            &fonts::ui_name(),
                            None,
                            false,
                            lineno as u64,
                            cx,
                        )),
                ),
        );
    let caret = clicks.caret_end(lineno, line);
    caret_block(lineno, caret, block, cx)
}

#[allow(clippy::too_many_arguments)]
fn task(
    t: &ActiveTheme,
    family: &'static str,
    checked: bool,
    text: &str,
    clicks: Clicks,
    lineno: usize,
    line: &str,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    // The checkbox flips the task in the note text (both Live and Read mode). The press stops
    // here so that the row does not also move the caret.
    let path = clicks.path.to_string();
    let mut boxed = div()
        .id(ElementId::NamedInteger(
            SharedString::from("chk"),
            lineno as u64,
        ))
        .mt(px(6.0))
        .flex_none()
        .size(px(17.0))
        .rounded(px(5.0))
        .border_1()
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    if !clicks.embed {
        // Inside an embed the box shows the state but does not change the note.
        boxed = boxed
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_task_line(&path, lineno, cx);
                cx.notify();
            }));
    }
    if checked {
        boxed = boxed
            .bg(t.ac)
            .border_color(t.ac)
            .text_color(t.ac_on)
            .child(icon("check", 12.0, t.ac_on));
    } else {
        boxed = boxed.border_color(t.tx3);
    }

    let (rendered, layout) = rich_text_geo(
        text,
        t,
        if checked { t.tx3 } else { t.tx },
        family,
        None,
        checked,
        lineno as u64,
        cx,
    );
    let row = div()
        .flex()
        .items_start()
        .gap(px(10.0))
        .mb(px(2.0))
        .text_size(px(15.0))
        .line_height(relative(1.9))
        .child(boxed)
        .child(div().flex_1().min_w_0().child(rendered));
    let caret = clicks.caret_text(lineno, line, text, layout);
    caret_block(lineno, caret, row, cx)
}

#[allow(clippy::too_many_arguments)]
fn code_block(
    t: &ActiveTheme,
    lang: &str,
    code: &[&str],
    clicks: Clicks,
    lineno: usize,
    line: &str,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let copy_text = code.join("\n");
    let mut body = div()
        .flex()
        .flex_col()
        .px(px(14.0))
        .py(px(12.0))
        .font_family(fonts::code_name())
        .text_size(px(12.5))
        .line_height(px(22.0))
        .text_color(t.tx);
    for line in code {
        body = body.child(highlight::code_line(line, t));
    }

    let block = div()
        .flex()
        .flex_col()
        .mt(px(8.0))
        .mb(px(16.0))
        .rounded(px(10.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(32.0))
                .pl(px(14.0))
                .pr(px(8.0))
                .border_b_1()
                .border_color(t.line_soft)
                .child(
                    div()
                        .font_family(fonts::code_name())
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .child(SharedString::from(lang.to_string())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .px(px(7.0))
                        .py(px(3.0))
                        .rounded(px(6.0))
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .hover(|s| s.bg(t.hover).text_color(t.tx))
                        .child(icon("copy", 13.0, t.tx3))
                        .child(div().child("复制"))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        }),
                ),
        )
        .child(body);
    let caret = clicks.caret_end(lineno, line);
    caret_block(lineno, caret, block, cx)
}

/// Status word -> pill kind used in table cells.
fn pill(t: &ActiveTheme, label: &str) -> Option<AnyElement> {
    let (fg, bg) = match label {
        "已完成" => (t.ok, t.ok_soft),
        "进行中" => (t.ac, t.ac_soft),
        "待规划" => (t.tx2, t.hover),
        _ => return None,
    };
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(22.0))
            .px(px(9.0))
            .rounded_full()
            .text_size(px(12.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg)
            .bg(bg)
            .child(div().size(px(6.0)).rounded_full().bg(fg))
            .child(div().child(SharedString::from(label.to_string())))
            .into_any_element(),
    )
}

#[allow(clippy::too_many_arguments)]
fn table(
    t: &ActiveTheme,
    rows: &[&str],
    clicks: Clicks,
    lineno: usize,
    line: &str,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let parsed: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            row.trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_string())
                .collect::<Vec<String>>()
        })
        .filter(|cells| {
            !cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
        })
        .collect();

    let cols = parsed.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let weights: Vec<f32> = if cols == 3 {
        vec![1.3, 1.0, 0.6]
    } else {
        vec![1.0; cols]
    };
    let total: f32 = weights.iter().sum();

    let mut table = div()
        .flex()
        .flex_col()
        .mt(px(8.0))
        .mb(px(8.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(t.line)
        .bg(t.raised)
        .overflow_hidden();

    for (ri, cells) in parsed.iter().enumerate() {
        let header = ri == 0;
        let mut row = div()
            .flex()
            .items_center()
            .px(px(14.0))
            .h(px(if header { 34.0 } else { 36.0 }))
            .when(header, |d| {
                d.bg(t.hover)
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.tx2)
            })
            .when(!header, |d| {
                d.text_size(px(13.5))
                    .text_color(t.tx)
                    .border_t_1()
                    .border_color(t.line_soft)
            });
        for (ci, cell) in cells.iter().enumerate() {
            let w = weights.get(ci).copied().unwrap_or(1.0) / total;
            let content: AnyElement = if header {
                SharedString::from(cell.clone()).into_any_element()
            } else {
                match pill(t, cell) {
                    Some(p) => p,
                    None => SharedString::from(cell.clone()).into_any_element(),
                }
            };
            row = row.child(
                div()
                    .w(relative(w))
                    .flex_none()
                    .overflow_hidden()
                    .child(content),
            );
        }
        table = table.child(row);
    }
    let caret = clicks.caret_end(lineno, line);
    caret_block(lineno, caret, table, cx)
}

/// The 源码 / 实时预览 / 阅读 pill at the bottom centre of the editor.
fn mode_pill(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> impl IntoElement + use<> {
    let t = app.theme;
    let mut bg = t.raised;
    bg.a = 0.92;
    let current = app.view_mode;

    let item = |name: &'static str, label: String, mode: ViewMode| {
        let on = current == mode;
        let color = if on { t.ac_on } else { t.tx2 };
        let mut node = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(26.0))
            .px(px(12.0))
            .rounded_full()
            .text_size(px(12.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .cursor_pointer()
            .when(on, |d| d.bg(t.ac))
            .when(!on, |d| d.hover(|s| s.bg(t.hover)))
            .child(icon(name, 14.0, color))
            .child(div().child(SharedString::from(label)));
        node = node.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.set_view_mode(mode);
                cx.notify();
            }),
        );
        node
    };

    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(14.0))
        .flex()
        .justify_center()
        .child(
            div()
                .flex()
                .p(px(3.0))
                .gap(px(2.0))
                .rounded_full()
                .bg(bg)
                .border_1()
                .border_color(t.line)
                .shadow_md()
                .child(item("code", crate::t!("status-source"), ViewMode::Source))
                .child(item("eye", crate::t!("status-live-preview"), ViewMode::Live))
                .child(item("book-2", crate::t!("status-reading"), ViewMode::Read)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_maps_one_to_one() {
        // The source here is the rendered content (the marker before it is added by the caller).
        assert_eq!(source_index_for_display("本周", "本周", 0), 0);
        assert_eq!(source_index_for_display("本周要做", "本周要做", 2), 2);
        assert_eq!(source_index_for_display("abc", "abc", 3), 3);
        // Past the end clamps to the end of the source.
        assert_eq!(source_index_for_display("abc", "abc", 99), 3);
    }

    #[test]
    fn bold_and_code_markers_are_skipped() {
        // "**粗**和`x`": shown "粗和x".
        let source = "**粗**和`x`";
        let display = "粗和x";
        // Before 粗 (shown 0) is after the opening marker.
        assert_eq!(source_index_for_display(source, display, 0), 0);
        // After 粗 (shown 1) is after the closing marker, before 和 (source index 3).
        assert_eq!(source_index_for_display(source, display, 1), 3);
        // Before x (shown 2) is the source position before the opening backtick (index 6).
        assert_eq!(source_index_for_display(source, display, 2), 6);
        // After x (shown 3) is before the closing backtick (source index 8).
        assert_eq!(source_index_for_display(source, display, 3), 8);
    }

    #[test]
    fn wikilink_brackets_and_chip_padding_are_skipped() {
        // "见[[A]]。" is shown as "见\u{2009}A\u{2009}。".
        let source = "见[[A]]。";
        let display = "见\u{2009}A\u{2009}。";
        assert_eq!(source_index_for_display(source, display, 1), 1);
        // Inside the chip (after the padding) the caret lands before the link's text.
        assert_eq!(source_index_for_display(source, display, 2), 3);
        assert_eq!(source_index_for_display(source, display, 3), 4);
        // After the chip (shown 4) is before the full stop, past the closing brackets (index 6).
        assert_eq!(source_index_for_display(source, display, 4), 6);
    }

    #[test]
    fn empty_inputs_are_safe() {
        assert_eq!(source_index_for_display("", "", 3), 0);
        assert_eq!(source_index_for_display("abc", "", 2), 0);
    }
}
