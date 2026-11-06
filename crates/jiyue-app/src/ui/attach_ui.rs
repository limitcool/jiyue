//! Attachments UI: paste or drop images and files, render them in the preview (with a lightbox),
//! note and block embeds, block references and the 附件设置 modal.
//!
//! Files: `attach_ui/resolve.rs` (pure rules), `index.rs` (vault scan), `import.rs` (import job and
//! text edits), `settings.rs` (`.jiyue/attach.json` and the modal), `view.rs` (preview blocks).
//!
//! Text edits go through `TextEditor::set_text_undoable`, which changes only the bytes that differ
//! and records one undo step, so Ctrl+Z reverts a property or link edit. The note is saved at once.

pub mod import;
pub mod index;
pub mod props;
pub mod resolve;
pub mod settings;
pub mod view;

use std::collections::HashSet;
use std::path::PathBuf;

use gpui::{AnyElement, App, ClipboardEntry, ClipboardItem, Context, Entity, KeyDownEvent};
use jiyue_md::embeds::Anchor;

use crate::AppCommand;
use crate::RustdianApp;
use crate::session::ViewMode;
use crate::ui::ext::{ExtCommand, ExtEvent, PaletteCommand, ToastKind};
use crate::ui::widgets::LineInput;
use import::{Outcome, Source};
use jiyue_md::attach::AttachSettings;
use resolve::VaultFiles;

/// An open inline editor of a property value.
pub struct PropEdit {
    /// The property key being edited.
    pub key: String,
    pub kind: props::Kind,
    pub input: Entity<LineInput>,
    /// Hint under the field after a rejected value (for example an invalid date).
    pub error: Option<&'static str>,
    /// Keeps the event subscription alive.
    pub _sub: gpui::Subscription,
}

/// Module state. The index is refreshed in the background on start-up, after imports and on
/// `VaultFilesChanged`.
#[derive(Default)]
pub struct State {
    /// Files of the vault and image sizes (see `index.rs`).
    pub files: VaultFiles,
    /// The saved attachment settings.
    pub settings: AttachSettings,
    /// The settings being edited in the modal.
    pub draft: AttachSettings,
    pub settings_open: bool,
    /// Text field of the folder name in the modal.
    pub folder_input: Option<Entity<LineInput>>,
    /// The image shown in the lightbox (vault-relative path).
    pub lightbox: Option<String>,
    /// The 属性 section of the right panel is collapsed (kept for the session).
    pub props_collapsed: bool,
    /// The property value being edited inline (text, number or date), when one is open.
    pub prop_edit: Option<PropEdit>,
    /// The 添加标签 field and its subscription, while it is open.
    pub tag_input: Option<(Entity<LineInput>, gpui::Subscription)>,
    /// Keeps the keystroke interceptor for Ctrl+V alive (see `init`).
    pub key_intercept: Option<gpui::Subscription>,
    /// Vault data of the live view's image and note widgets, shared with every editor (see
    /// `ui/text_input/context.rs`). Synced once per frame by the app.
    pub live: crate::ui::text_input::context::LiveHandle,
}

impl State {
    /// The folder-name field of the modal, or empty.
    pub fn name_text(&self, cx: &App) -> String {
        self.folder_input
            .as_ref()
            .map(|input| input.read(cx).text())
            .unwrap_or_default()
    }

    /// The fixed attachment folder, when the settings have one. Used to prefer copies inside it.
    pub fn fixed_folder(&self) -> Option<String> {
        match &self.settings.folder {
            jiyue_md::attach::AttachFolder::Fixed(name) => Some(name.clone()),
            _ => None,
        }
    }
}

/// Commands this module handles (sent as `ExtCommand::Attach`).
#[derive(Clone, Debug)]
pub enum Command {
    PasteImage,
    InsertEmbed,
    ToggleProperties,
    AttachmentSettings,
    CopyBlockLink,
}

/// Starts the module: loads the settings, creates the folder field and scans the vault.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.ext.attach.settings = settings::load(&app.vault.root_dir);
    app.ext.attach.folder_input = Some(settings::new_name_input(cx));
    refresh_index(app, cx);
    // The editor binds Ctrl+V to its paste action, and key bindings run before key listeners.
    // This interceptor runs before the bindings: when Ctrl+V would import a clipboard image, the
    // binding is skipped and the key listener (`handle_key`) does the import.
    let weak = cx.entity().downgrade();
    app.ext.attach.key_intercept = Some(cx.intercept_keystrokes(move |ev, _window, cx| {
        let m = ev.keystroke.modifiers;
        // Ctrl+V, or Cmd+V on macOS (`keymap::primary_held`).
        if ev.keystroke.key != "v" || !crate::ui::keymap::primary_held(&m) || m.alt || m.shift {
            return;
        }
        if cfg!(target_os = "macos") {
            if m.control {
                return;
            }
        } else if m.platform {
            return;
        }
        let Some(entity) = weak.upgrade() else {
            return;
        };
        if !paste_takes_image(entity.read(cx), cx) {
            return;
        }
        // Stopping here also ends the key dispatch, so the import runs now, from the interceptor.
        cx.stop_propagation();
        entity.update(cx, import_clipboard_at_caret);
    }));
}

/// Imports the clipboard image at the caret of the open note (Ctrl+V in Source).
fn import_clipboard_at_caret(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if let Some((ext, bytes)) = clipboard_image(cx) {
        import_sources(app, vec![Source::Clipboard { bytes, ext }], true, cx);
    }
}

/// True when Ctrl+V should import a clipboard image: a note is open in Source, no overlay is
/// open, and the clipboard holds an image.
fn paste_takes_image(app: &RustdianApp, cx: &App) -> bool {
    app.view_mode.is_source()
        && app.active_path().is_some()
        && !app.show_palette
        && !app.settings.open
        && !app.ext.attach.settings_open
        && app.ext.attach.lightbox.is_none()
        && clipboard_has_image(cx)
}

/// Lightbox and the settings modal.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if let Some(rel) = app.ext.attach.lightbox.as_deref() {
        out.push(view::lightbox(app, rel, cx));
    }
    out.extend(settings::render_modal(app, cx));
    out
}

/// Escape closes the lightbox and the modal. (Ctrl+V is handled before the key bindings, see
/// `init`.)
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    if ev.keystroke.key.as_str() != "escape" {
        return false;
    }
    if app.ext.attach.lightbox.take().is_some() {
        cx.notify();
        return true;
    }
    if app.ext.attach.settings_open {
        app.ext.attach.settings_open = false;
        cx.notify();
        return true;
    }
    false
}

/// Module events: the index follows the vault files.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    if matches!(ev, ExtEvent::Started | ExtEvent::VaultFilesChanged(_)) {
        refresh_index(app, cx);
    }
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::PasteImage => paste_image(app, cx),
        Command::InsertEmbed => insert_embed(app, cx),
        Command::ToggleProperties => {
            app.ext.attach.props_collapsed = !app.ext.attach.props_collapsed;
        }
        Command::AttachmentSettings => open_settings(app, cx),
        Command::CopyBlockLink => copy_block_link(app, cx),
    }
    cx.notify();
}

/// Command rows for the command palette.
pub fn palette_commands() -> Vec<PaletteCommand> {
    let cmd = |c: Command| AppCommand::Ext(ExtCommand::Attach(c));
    vec![
        PaletteCommand {
            label: "粘贴图片",
            icon: "plus",
            shortcut: "Ctrl+V",
            command: cmd(Command::PasteImage),
        },
        PaletteCommand {
            label: "插入嵌入",
            icon: "square-rounded-plus",
            shortcut: "",
            command: cmd(Command::InsertEmbed),
        },
        PaletteCommand {
            label: "复制块链接",
            icon: "link",
            shortcut: "",
            command: cmd(Command::CopyBlockLink),
        },
        PaletteCommand {
            label: "附件设置",
            icon: "settings",
            shortcut: "",
            command: cmd(Command::AttachmentSettings),
        },
        PaletteCommand {
            label: "显示或隐藏属性",
            icon: "tag",
            shortcut: "",
            command: cmd(Command::ToggleProperties),
        },
    ]
}

/// No status bar items.
pub fn statusbar_items(_app: &RustdianApp, _cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    Vec::new()
}

/// True while the lightbox or the settings modal is open.
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.attach.lightbox.is_some() || app.ext.attach.settings_open
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// Scans the vault in the background and stores the result.
pub fn refresh_index(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let root = app.vault.root_dir.clone();
    let task = cx
        .background_executor()
        .spawn(async move { index::scan(&root) });
    cx.spawn(async move |this, cx| {
        let files = task.await;
        let _ = this.update(cx, |app, cx| {
            app.ext.attach.files = files;
            cx.notify();
        });
    })
    .detach();
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// The clipboard's image bytes, if it holds an image.
fn clipboard_image(cx: &App) -> Option<(Option<&'static str>, Vec<u8>)> {
    let item = cx.read_from_clipboard()?;
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => {
            Some((import::ext_for_format(image.format), image.bytes.clone()))
        }
        _ => None,
    })
}

/// True when the clipboard holds an image (without copying it).
fn clipboard_has_image(cx: &App) -> bool {
    cx.read_from_clipboard().is_some_and(|item| {
        item.entries()
            .iter()
            .any(|entry| matches!(entry, ClipboardEntry::Image(_)))
    })
}

/// Ctrl+V from the palette: switches to Source and imports the clipboard image at the end of the note.
fn paste_image(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.active_path().is_none() {
        app.toast(ToastKind::Warn, "没有打开的笔记");
        return;
    }
    let Some((ext, bytes)) = clipboard_image(cx) else {
        app.toast(ToastKind::Warn, "剪贴板里没有图片");
        return;
    };
    import_sources(app, vec![Source::Clipboard { bytes, ext }], false, cx);
}

/// Files dropped on the editor. Inserted at the caret in Source mode, else appended.
pub fn drop_files(app: &mut RustdianApp, paths: &[PathBuf], cx: &mut Context<RustdianApp>) {
    let sources: Vec<Source> = paths
        .iter()
        .filter(|p| p.is_file())
        .cloned()
        .map(Source::File)
        .collect();
    if sources.is_empty() {
        app.toast(ToastKind::Warn, "拖入的不是文件");
        return;
    }
    let at_caret = app.view_mode.is_source();
    import_sources(app, sources, at_caret, cx);
}

/// Imports `sources` for the active note: the files are copied on a background thread, then the
/// links are inserted at the caret (`at_caret`) or at the end of the note.
pub fn import_sources(
    app: &mut RustdianApp,
    sources: Vec<Source>,
    at_caret: bool,
    cx: &mut Context<RustdianApp>,
) {
    let Some(note_path) = app.active_path().map(str::to_string) else {
        app.toast(ToastKind::Warn, "没有打开的笔记");
        return;
    };
    if sources.is_empty() {
        return;
    }
    let name_taken: HashSet<String> = app
        .ext
        .attach
        .files
        .files
        .iter()
        .map(|f| resolve::basename(f).to_string())
        .collect();
    let job = import::Job {
        root: app.vault.root_dir.clone(),
        settings: app.ext.attach.settings.clone(),
        note_path: note_path.clone(),
        sources,
        now: import::now_stamp(),
        name_taken,
    };
    let task = cx
        .background_executor()
        .spawn(async move { import::run_job(&job) });
    cx.spawn(async move |this, cx| {
        let outcomes = task.await;
        let _ = this.update(cx, |app, cx| {
            finish_import(app, &note_path, at_caret, outcomes, cx);
        });
    })
    .detach();
}

/// Inserts the links of finished imports into `note_path` and reports the result.
fn finish_import(
    app: &mut RustdianApp,
    note_path: &str,
    at_caret: bool,
    outcomes: Vec<Outcome>,
    cx: &mut Context<RustdianApp>,
) {
    let mut links = Vec::new();
    let mut saved = Vec::new();
    let mut errors = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Saved { dest_rel, link } => {
                links.push(link);
                saved.push(dest_rel);
            }
            Outcome::Failed(error) => errors.push(error),
        }
    }
    refresh_index(app, cx);
    if links.is_empty() {
        let text = errors
            .first()
            .cloned()
            .unwrap_or_else(|| "导入失败".to_string());
        app.toast(ToastKind::Error, text);
        cx.notify();
        return;
    }

    app.ensure_editor(note_path, cx);
    let Some(editor) = app.editor_for(note_path) else {
        app.toast(ToastKind::Error, "笔记没有打开，无法插入链接");
        return;
    };
    let joined = links.join(" ");
    let is_active = app.active_path() == Some(note_path);
    let (text, row, col) = {
        let ed = editor.read(cx);
        (ed.text(), ed.cursor_row_col().0, ed.cursor_row_col().1)
    };
    let (new_text, r, c) = if at_caret && is_active {
        import::insert_at(&text, row - 1, col - 1, &joined)
    } else {
        import::append(&text, &joined)
    };
    editor.update(cx, |ed, cx| {
        ed.set_text_undoable(&new_text, cx);
        ed.set_caret_to_row_col(r, c, cx);
    });
    app.save_path(note_path, cx);
    if is_active && !app.view_mode.is_source() {
        app.set_view_mode(ViewMode::Source);
    }

    let message = if saved.len() == 1 {
        format!("已保存图片到 {}", saved[0])
    } else {
        format!("已导入 {} 个文件", saved.len())
    };
    app.toast(ToastKind::Success, message);
    if !errors.is_empty() {
        app.toast(ToastKind::Error, errors[0].clone());
    }
    cx.notify();
}

/// Inserts `![[]]` at the caret (Source mode) and puts the caret between the brackets.
fn insert_embed(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(path) = app.active_path().map(str::to_string) else {
        app.toast(ToastKind::Warn, "没有打开的笔记");
        return;
    };
    app.set_view_mode(ViewMode::Source);
    app.ensure_editor(&path, cx);
    let Some(editor) = app.editor_for(&path) else {
        return;
    };
    let (text, row, col) = {
        let ed = editor.read(cx);
        let (r, c) = ed.cursor_row_col();
        (ed.text(), r, c)
    };
    let (new_text, r, c) = import::insert_at(&text, row - 1, col - 1, "![[]]");
    editor.update(cx, |ed, cx| {
        ed.set_text_undoable(&new_text, cx);
        ed.set_caret_to_row_col(r, c.saturating_sub(2), cx);
    });
    app.save_path(&path, cx);
}

/// Replaces the text of the active note with `edit(text)` through the editor, then saves it.
/// Nothing happens when `edit` leaves the text unchanged. Used by the 属性 panel.
pub fn edit_active_text(
    app: &mut RustdianApp,
    edit: impl FnOnce(&str) -> String,
    cx: &mut Context<RustdianApp>,
) {
    let Some(path) = app.active_path().map(str::to_string) else {
        return;
    };
    app.ensure_editor(&path, cx);
    let Some(editor) = app.editor_for(&path) else {
        return;
    };
    let (text, row, col) = {
        let ed = editor.read(cx);
        let (r, c) = ed.cursor_row_col();
        (ed.text(), r, c)
    };
    let next = edit(&text);
    if next == text {
        return;
    }
    editor.update(cx, |ed, cx| {
        ed.set_text_undoable(&next, cx);
        ed.set_caret_to_row_col(row - 1, col - 1, cx);
    });
    app.save_path(&path, cx);
    cx.notify();
}

// ---------------------------------------------------------------------------
// Block links and navigation
// ---------------------------------------------------------------------------

/// 复制块链接: gives the caret's block an id (adds ` ^id` to its line when needed) and copies
/// `[[笔记#^id]]` to the clipboard.
pub fn copy_block_link(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(path) = app.active_path().map(str::to_string) else {
        app.toast(ToastKind::Warn, "没有打开的笔记");
        return;
    };
    app.ensure_editor(&path, cx);
    let Some(editor) = app.editor_for(&path) else {
        return;
    };
    let (text, row, col) = {
        let ed = editor.read(cx);
        let (r, c) = ed.cursor_row_col();
        (ed.text(), r, c)
    };
    let Some((new_text, id)) = import::ensure_block_id(&text, row - 1, import::time_seed()) else {
        app.toast(ToastKind::Warn, "光标所在行是空行，没有可以引用的块");
        return;
    };
    let title = app
        .vault
        .notes
        .iter()
        .find(|n| n.path == path)
        .map(|n| n.title.clone())
        .unwrap_or_default();
    if new_text != text {
        editor.update(cx, |ed, cx| {
            ed.set_text_undoable(&new_text, cx);
            ed.set_caret_to_row_col(row - 1, col - 1, cx);
        });
        app.save_path(&path, cx);
    }
    cx.write_to_clipboard(ClipboardItem::new_string(format!("[[{title}#^{id}]]")));
    app.toast(ToastKind::Success, "已复制块链接");
    cx.notify();
}

/// Splits a link body (`笔记#Heading`, `笔记#^id`, `#Heading`) into the note title and the anchor.
pub fn split_target(inner: &str) -> (&str, Option<Anchor>) {
    match inner.split_once('#') {
        None => (inner.trim(), None),
        Some((title, anchor)) => {
            let anchor = anchor.trim();
            let parsed = match anchor.strip_prefix('^') {
                Some(id) if !id.is_empty() => Some(Anchor::Block(id.to_string())),
                Some(_) => None,
                None if !anchor.is_empty() => Some(Anchor::Heading(anchor.to_string())),
                None => None,
            };
            (title.trim(), parsed)
        }
    }
}

/// Opens the note a wikilink points at and moves the caret to its heading or block. `raw` is the
/// link body as written (`笔记`, `笔记|别名`, `笔记#标题`, `笔记#^id`, `#标题`). A missing note is
/// created, like a plain wikilink.
pub fn navigate_link(app: &mut RustdianApp, raw: &str, cx: &mut Context<RustdianApp>) {
    let inner = raw.split('|').next().unwrap_or("").trim();
    let (title, anchor) = split_target(inner);
    if title.is_empty() {
        // `[[#Heading]]`: a link inside the open note.
        if let (Some(anchor), Some(path)) = (anchor, app.active_path().map(str::to_string))
            && let Some(idx) = app.vault.notes.iter().position(|n| n.path == path)
        {
            jump_to_anchor(app, idx, &anchor);
        }
        cx.notify();
        return;
    }
    open_target(app, title, anchor, cx);
}

/// Opens the note `title` and, when there is an anchor, moves the caret to it. A missing note is
/// created like a plain wikilink.
pub fn open_target(
    app: &mut RustdianApp,
    title: &str,
    anchor: Option<Anchor>,
    cx: &mut Context<RustdianApp>,
) {
    match anchor {
        None => app.open_wikilink(title),
        Some(anchor) => match app.vault.note_index_by_title(title) {
            Some(idx) => {
                app.open_note_by_index(idx);
                jump_to_anchor(app, idx, &anchor);
            }
            None => app.open_wikilink(title),
        },
    }
    cx.notify();
}

/// Moves the caret of note `idx` to the line of `anchor`, opening it in Source. Does nothing when
/// the anchor is not in the note.
fn jump_to_anchor(app: &mut RustdianApp, idx: usize, anchor: &Anchor) {
    let Some(note) = app.vault.notes.get(idx) else {
        return;
    };
    if let Some(row) = anchor_row(note.content(), anchor) {
        app.request_source_caret(row, 0);
    }
}

/// Zero-based line of a heading or block in `text`.
pub fn anchor_row(text: &str, anchor: &Anchor) -> Option<usize> {
    match anchor {
        Anchor::Heading(heading) => {
            jiyue_md::embeds::find_heading(text, heading).map(|r| resolve::line_of(text, r.start))
        }
        Anchor::Block(id) => jiyue_md::embeds::find_block(text, id).map(|b| b.line_start),
    }
}

// ---------------------------------------------------------------------------
// Settings modal
// ---------------------------------------------------------------------------

fn open_settings(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.show_palette = false;
    app.ext.attach.draft = app.ext.attach.settings.clone();
    let name = settings::folder_name(&app.ext.attach.draft.folder);
    let name = if name.is_empty() {
        settings::DEFAULT_FOLDER.to_string()
    } else {
        name
    };
    if let Some(input) = app.ext.attach.folder_input.clone() {
        input.update(cx, |field, cx| field.set_text(&name, cx));
    }
    app.ext.attach.settings_open = true;
}

/// Saves the modal's settings to `.jiyue/attach.json` and closes it.
pub fn save_settings(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let name = app.ext.attach.name_text(cx);
    let mut next = app.ext.attach.draft.clone();
    next.folder = settings::folder_for(settings::mode_index(&next.folder), &name);
    match settings::save(&app.vault.root_dir, &next) {
        Ok(()) => {
            app.ext.attach.settings = next;
            app.ext.attach.settings_open = false;
            app.toast(ToastKind::Success, "附件设置已保存");
        }
        Err(e) => app.toast(ToastKind::Error, format!("保存附件设置失败：{e}")),
    }
    cx.notify();
}
