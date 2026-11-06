//! Templates. Owned by W3-T.
//!
//! - 插入模板…: a searchable picker of the notes in the templates folder. The chosen one is rendered
//!   (`{{date}}`, `{{time}}`, `{{datetime}}`, `{{title}}`, `{{cursor}}` and the rest of
//!   `jiyue-export`'s variables) and inserted at the caret of the active note as ONE undoable edit;
//!   the caret goes to `{{cursor}}`.
//! - 用模板新建笔记…: the same picker creates a new note from the template (in the active note's
//!   folder), then opens it with the caret at `{{cursor}}`.
//! - Settings 日记: the templates folder (模板 or Templates) and the template the daily note starts
//!   from (the built-in one, or one of the templates).
//!
//! Layout: `pure` (folder choice, filter, caret and cycle rules; unit tested), `settings` (the two
//! setting rows).

mod pure;
mod settings;

pub use settings::{daily_template_row, folder_row};

use gpui::{
    AnyElement, Context, Entity, FontWeight, IntoElement, KeyDownEvent, MouseButton, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use jiyue_core::fsops;
use jiyue_export::{
    Locale, TemplateCtx, TemplateInfo, TemplateRules, list_templates, render_template,
};

use crate::AppCommand;
use crate::RustdianApp;
use crate::ui::ext::{ExtCommand, ExtEvent, PaletteCommand, ToastKind};
use crate::ui::files_ui;
use crate::ui::widgets::{ButtonKind, LineInput, LineInputEvent, button, modal, overlay};

/// Most template rows shown at once; the list scrolls past it.
const ROW_LIMIT: usize = 200;

/// Commands this module handles (sent as `ExtCommand::Template`).
#[derive(Clone, Debug)]
pub enum Command {
    /// 插入模板…
    Insert,
    /// 用模板新建笔记…
    NewNote,
}

/// What the picker does with the chosen template.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Closed,
    Insert,
    NewNote,
}

/// Module state.
#[derive(Default)]
pub struct State {
    pub mode: Mode,
    /// The filter field of the picker, created in `init`.
    pub filter: Option<Entity<LineInput>>,
    /// Folder the templates are read from (vault-relative).
    pub folder: String,
    /// Every template of the folder, sorted by path.
    pub items: Vec<TemplateInfo>,
    /// Indexes into `items` that match the filter.
    pub shown: Vec<usize>,
    /// Index into `shown` of the highlighted row.
    pub highlight: usize,
    /// The field takes keyboard focus at the next frame.
    pub focus_pending: bool,
    subs: Vec<Subscription>,
}

/// Creates the filter field and its subscriptions. Called once, at start-up.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let theme = app.theme;
    let input = cx.new(|cx| LineInput::new(cx, "搜索模板…"));
    input.update(cx, |i, cx| i.set_theme(theme, cx));
    let changed = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| match ev {
            LineInputEvent::Changed(query) => {
                let s = &mut this.ext.template;
                s.shown = pure::filter(&s.items, query);
                s.highlight = 0;
                cx.notify();
            }
            LineInputEvent::Submit(_) => confirm(this, cx),
            LineInputEvent::Cancel => close(this),
        },
    );
    app.ext.template.subs.push(changed);
    app.ext.template.filter = Some(input);
}

/// Focuses the filter field in the frame the picker opens (the palette has the focus until then).
pub fn focus_on_open(app: &mut RustdianApp, window: &mut Window, cx: &mut Context<RustdianApp>) {
    if !app.ext.template.focus_pending {
        return;
    }
    app.ext.template.focus_pending = false;
    if let Some(input) = app.ext.template.filter.clone() {
        input.read(cx).focus(window);
    }
}

/// Opens the picker. Reads the templates folder now, so new templates appear at once.
fn open(app: &mut RustdianApp, mode: Mode, cx: &mut Context<RustdianApp>) {
    if app.active_path().is_none() {
        app.toast(ToastKind::Info, "没有打开的笔记");
        return;
    }
    let root = app.vault.root_dir.clone();
    let configured = TemplateRules::load(&root).templates_folder;
    let folder = pure::choose_folder(&configured, |f| root.join(f).is_dir());
    let items = list_templates(&root, &folder);
    let s = &mut app.ext.template;
    s.mode = mode;
    s.folder = folder;
    s.shown = pure::filter(&items, "");
    s.items = items;
    s.highlight = 0;
    s.focus_pending = true;
    if let Some(input) = s.filter.clone() {
        input.update(cx, |i, cx| i.set_text("", cx));
    }
    cx.notify();
}

/// Closes the picker.
fn close(app: &mut RustdianApp) {
    app.ext.template.mode = Mode::Closed;
    app.ext.template.focus_pending = false;
}

/// The template at the highlighted row was chosen (Enter or a click).
fn confirm(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let s = &app.ext.template;
    let Some(item) = s
        .shown
        .get(s.highlight)
        .and_then(|&i| s.items.get(i))
        .cloned()
    else {
        return;
    };
    let mode = s.mode;
    let root = app.vault.root_dir.clone();
    let Some(text) = jiyue_export::templates::read_template(&root, &item.path) else {
        app.toast(ToastKind::Error, format!("无法读取模板：{}", item.name));
        close(app);
        cx.notify();
        return;
    };
    match mode {
        Mode::Insert => insert_at_caret(app, &text, cx),
        Mode::NewNote => new_note_from(app, &root, &text, cx),
        Mode::Closed => {}
    }
    close(app);
    cx.notify();
}

/// Variables of a template for the note `title` at `path`.
fn context(
    app: &RustdianApp,
    title: &str,
    path: &str,
    selection: &str,
    cx: &gpui::App,
) -> Option<TemplateCtx> {
    let now = pure::moment(jiyue_history::system_now_ms())?;
    let clipboard = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    let vault = app
        .vault
        .root_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut ctx = TemplateCtx::new(now, title, path, &vault, Locale::Zh);
    ctx.clipboard = clipboard;
    ctx.selection = selection.to_owned();
    ctx.uuid_seed = jiyue_history::system_now_ms() as u64;
    Some(ctx)
}

/// Inserts the rendered template at the caret of the active note (replacing the selection), as one
/// undoable edit, and puts the caret at `{{cursor}}`.
fn insert_at_caret(app: &mut RustdianApp, template: &str, cx: &mut Context<RustdianApp>) {
    let Some(path) = app.active_path().map(str::to_owned) else {
        return;
    };
    let Some(editor) = app.editor_for(&path) else {
        app.toast(ToastKind::Info, "笔记未打开，无法插入模板");
        return;
    };
    let (anchor, head) = editor.read(cx).selection_offsets();
    let selection = {
        let text = editor.read(cx).text();
        let (a, b) = (anchor.min(head), anchor.max(head));
        text.get(a..b).unwrap_or("").to_owned()
    };
    let title = app
        .vault
        .notes
        .iter()
        .find(|n| n.path == path)
        .map(|n| n.title.clone())
        .unwrap_or_default();
    let Some(ctx) = context(app, &title, &path, &selection, cx) else {
        app.toast(ToastKind::Error, "无法读取当前时间");
        return;
    };
    let rendered = render_template(template, &ctx);
    let inserted = editor.update(cx, |ed, cx| {
        let (a, b) = {
            let (a, h) = ed.selection_offsets();
            (a.min(h), a.max(h))
        };
        if !ed.replace_text_range(a..b, &rendered.text, cx) {
            return false;
        }
        let caret = pure::caret_after_insert(a, rendered.text.len(), rendered.cursor_offset);
        ed.restore_selection(caret, caret, cx);
        true
    });
    if !inserted {
        app.toast(ToastKind::Info, "请先结束输入法的输入，再插入模板");
    }
}

/// Creates `无标题.md` (renamed by the file system when taken) in the folder of the active note,
/// writes the rendered template into it and opens it with the caret at `{{cursor}}`.
fn new_note_from(
    app: &mut RustdianApp,
    root: &std::path::Path,
    template: &str,
    cx: &mut Context<RustdianApp>,
) {
    let dir = app
        .active_path()
        .map(crate::ui::folder_key)
        .unwrap_or_default();
    let rel = match fsops::create_note(root, &dir, pure::NEW_NOTE_TITLE) {
        Ok(rel) => rel,
        Err(e) => {
            app.toast(ToastKind::Error, format!("无法新建笔记：{e:?}"));
            return;
        }
    };
    let stem = std::path::Path::new(&rel)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(ctx) = context(app, &stem, &rel, "", cx) else {
        app.toast(ToastKind::Error, "无法读取当前时间");
        return;
    };
    let rendered = render_template(template, &ctx);
    if let Err(e) = fsops::write_atomic(&root.join(&rel), rendered.text.as_bytes()) {
        app.toast(ToastKind::Error, format!("无法写入模板内容：{e:?}"));
        return;
    }

    // Dirty tabs keep their text; everything else is read from disk again.
    let keep: Vec<String> = app
        .vault
        .open_tabs
        .iter()
        .filter_map(|&i| app.vault.notes.get(i))
        .map(|n| n.path.clone())
        .filter(|p| app.is_dirty(p, cx))
        .collect();
    app.vault.reload_from_disk(&keep, &[]);
    files_ui::expand_folder(app, &dir);
    files_ui::reveal_path(app, &rel);
    if let Some(idx) = app.vault.notes.iter().position(|n| n.path == rel) {
        app.open_note_by_index(idx);
    }
    let offset = rendered.cursor_offset.unwrap_or(rendered.text.len());
    let (row, col) = pure::row_col(&rendered.text, offset);
    app.request_source_caret(row, col);
    app.toast(ToastKind::Success, format!("已用模板新建笔记：{stem}"));
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::Insert => open(app, Mode::Insert, cx),
        Command::NewNote => open(app, Mode::NewNote, cx),
    }
}

/// Palette rows: 插入模板…, 用模板新建笔记….
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "插入模板…",
            icon: "file-plus",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Template(Command::Insert)),
        },
        PaletteCommand {
            label: "用模板新建笔记…",
            icon: "square-rounded-plus",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Template(Command::NewNote)),
        },
    ]
}

/// Keeps the filter field in the theme.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    if let ExtEvent::ThemeChanged = ev
        && let Some(input) = app.ext.template.filter.clone()
    {
        let theme = app.theme;
        input.update(cx, |i, cx| i.set_theme(theme, cx));
    }
}

/// True while the picker is open.
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.template.mode != Mode::Closed
}

/// Escape closes the picker; the arrow keys move the highlight. Enter is the field's Submit.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    if app.ext.template.mode == Mode::Closed {
        return false;
    }
    if overlay::is_escape(ev) {
        close(app);
        cx.notify();
        return true;
    }
    let s = &mut app.ext.template;
    let last = s.shown.len().saturating_sub(1);
    match ev.keystroke.key.as_str() {
        "down" => {
            s.highlight = (s.highlight + 1).min(last);
            cx.notify();
            true
        }
        "up" => {
            s.highlight = s.highlight.saturating_sub(1);
            cx.notify();
            true
        }
        _ => false,
    }
}

/// The picker: the filter field, the matching templates, 取消.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let s = &app.ext.template;
    if s.mode == Mode::Closed {
        return Vec::new();
    }
    let t = app.theme;
    let title = match s.mode {
        Mode::NewNote => "用模板新建笔记",
        _ => "插入模板",
    };

    let mut list = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .max_h(px(320.0))
        .overflow_hidden();
    if s.items.is_empty() {
        list = list.child(
            div()
                .py(px(12.0))
                .text_size(px(13.0))
                .text_color(t.tx3)
                .child(SharedString::from(format!(
                    "「{}」文件夹里没有模板。把笔记放进这个文件夹即可。可在 设置 → 日记 中修改文件夹。",
                    s.folder
                ))),
        );
    } else if s.shown.is_empty() {
        list = list.child(
            div()
                .py(px(12.0))
                .text_size(px(13.0))
                .text_color(t.tx3)
                .child("没有匹配的模板"),
        );
    }
    for (row, &idx) in s.shown.iter().take(ROW_LIMIT).enumerate() {
        let Some(item) = s.items.get(idx) else {
            continue;
        };
        let on = row == s.highlight;
        list = list.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .px(px(10.0))
                .py(px(7.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .bg(if on { t.active } else { t.sheet })
                .hover(|st| st.bg(t.hover))
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.tx)
                        .child(SharedString::from(item.name.clone())),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .child(SharedString::from(item.path.clone())),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.ext.template.highlight = row;
                        confirm(this, cx);
                    }),
                ),
        );
    }

    let mut field = div().mb(px(10.0));
    if let Some(input) = &s.filter {
        field = field.child(input.clone());
    }
    let body = div()
        .flex()
        .flex_col()
        .font_family(crate::ui::fonts::ui_name())
        .child(field)
        .child(list);

    let footer = vec![
        button(
            &t,
            "取消",
            ButtonKind::Ghost,
            None,
            cx.listener(|this, _, _, cx| {
                close(this);
                cx.notify();
            }),
        )
        .into_any_element(),
    ];
    let dismiss = cx.listener(|this: &mut RustdianApp, _, _, cx| {
        close(this);
        cx.notify();
    });
    vec![modal(&t, title, body, footer, 520.0, dismiss)]
}
