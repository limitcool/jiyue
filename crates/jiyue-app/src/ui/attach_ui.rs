//! Attachments UI: paste or drop images and files, render them in the preview (with a lightbox),
//! note and block embeds, block references and the 附件设置 modal.
//!
//! Files: `attach_ui/resolve.rs` (pure rules), `index.rs` (vault scan), `import.rs` (import job and
//! text edits), `settings.rs` (`.jiyue/attach.json` and the modal), `view.rs` (preview blocks).
//!
//! Text edits go through `TextEditor::set_text_undoable`, which changes only the bytes that differ
//! and records one undo step, so Ctrl+Z reverts a property or link edit. The note is saved at once.

pub mod clipboard_win;
pub mod drop_win;
pub mod import;
pub mod index;
pub mod props;
pub mod resolve;
pub mod settings;
pub mod view;

pub use drop_win::install_window_drop;

use std::collections::HashSet;
use std::path::PathBuf;

use gpui::{AnyElement, App, ClipboardEntry, ClipboardItem, Context, Entity, KeyDownEvent};
use jiyue_md::embeds::Anchor;
use jiyue_md::frontmatter;

use crate::AppCommand;
use crate::RustdianApp;
use crate::session::ViewMode;
use crate::ui::ext::{ExtCommand, ExtEvent, PaletteCommand, ToastKind};
use crate::ui::widgets::LineInput;
use import::{Outcome, Source};
use jiyue_md::attach::{AttachSettings, LinkStyle};
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

/// An open inline editor of a property name (the key of the row).
pub struct KeyEdit {
    /// The property name being renamed.
    pub key: String,
    pub input: Entity<LineInput>,
    /// Hint under the field after a rejected name (empty, duplicate, ...).
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
    /// The property name being renamed inline, while its field is open.
    pub key_edit: Option<KeyEdit>,
    /// Keeps the keystroke interceptor for Ctrl+V alive (see `init`).
    pub key_intercept: Option<gpui::Subscription>,
    /// Vault data of the live view's image and note widgets, shared with every editor (see
    /// `ui/text_input/context.rs`). Synced once per frame by the app.
    pub live: crate::ui::text_input::context::LiveHandle,
    /// True while a native OS file/image drag is hovering over the window.
    pub os_drag_over: bool,
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
    app.ext.attach.key_intercept = Some(cx.intercept_keystrokes(move |ev, window, cx| {
        // Backspace in a focused empty 添加标签 field removes the note's last tag.
        if ev.keystroke.key == "backspace" {
            let m = ev.keystroke.modifiers;
            if m.control || m.alt || m.platform {
                return;
            }
            let Some(entity) = weak.upgrade() else {
                return;
            };
            if tag_field_is_focused_and_empty(entity.read(cx), window, cx) {
                cx.stop_propagation();
                entity.update(cx, remove_last_tag);
            }
            return;
        }
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
        if !paste_takes_image(entity.read(cx), window, cx) {
            return;
        }
        // Stopping here also ends the key dispatch, so the import runs now, from the interceptor.
        cx.stop_propagation();
        entity.update(cx, import_clipboard_at_caret);
    }));
}

/// True when the 添加标签 field is open, focused in `window`, and empty.
fn tag_field_is_focused_and_empty(app: &RustdianApp, window: &gpui::Window, cx: &App) -> bool {
    app.ext.attach.tag_input.as_ref().is_some_and(|(input, _)| {
        let field = input.read(cx);
        field.is_focused(window) && field.text().is_empty()
    })
}

/// Removes the last tag of the active note (Backspace in an empty 添加标签 field).
fn remove_last_tag(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let text = app
        .vault
        .active_note()
        .map(|n| n.content().to_string())
        .unwrap_or_default();
    if let Some(tag) = props::last_tag(&text) {
        edit_active_text(app, move |t| frontmatter::remove_tag(t, "tags", &tag), cx);
    }
}

/// Imports the clipboard image or copied files at the caret of the open note (Ctrl+V).
fn import_clipboard_at_caret(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let sources = clipboard_sources(cx);
    if !sources.is_empty() {
        import_sources(app, sources, true, cx);
    }
}

/// True when Ctrl+V should import a clipboard image or file: the active note's editor is focused
/// in Source or Live mode, no overlay is open, and the clipboard holds media.
fn paste_takes_image(app: &RustdianApp, window: &gpui::Window, cx: &App) -> bool {
    app.viewport_mode == crate::ViewportMode::Editor
        && matches!(app.view_mode, ViewMode::Source | ViewMode::Live)
        && !app.show_palette
        && !app.settings.open
        && !crate::ui::ext::overlay_open(app)
        && app
            .active_path()
            .and_then(|path| app.editor_for(path))
            .is_some_and(|editor| editor.read(cx).is_focused(window))
        && clipboard_has_pasteable(cx)
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

/// Module events: the index follows the vault files; active inline fields follow note switches and theme.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    match ev {
        ExtEvent::Started => refresh_index(app, cx),
        ExtEvent::VaultFilesChanged(paths) => {
            let md_only_existing = !app.ext.attach.files.files.is_empty()
                && paths.iter().all(|p| {
                    jiyue_md::embeds::embed_kind(p) != jiyue_md::embeds::EmbedKind::Image
                        && !p.is_empty()
                        && app.ext.attach.files.contains(p)
                        && app.vault.root_dir.join(p).is_file()
                });
            if !md_only_existing {
                refresh_index_dirty(app, paths, cx);
            }
        }
        ExtEvent::NoteOpened(_) => {
            app.ext.attach.tag_input = None;
            app.ext.attach.prop_edit = None;
            app.ext.attach.key_edit = None;
        }
        ExtEvent::ThemeChanged => {
            let theme = app.theme;
            if let Some(input) = app.ext.attach.folder_input.as_ref() {
                let input = input.clone();
                input.update(cx, |field, cx| field.set_theme(theme, cx));
            }
        }
        _ => {}
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
    refresh_index_dirty(app, &[], cx);
}

/// Scans the vault in the background, reusing cached image dimensions for paths not in `dirty_paths`.
pub fn refresh_index_dirty(
    app: &mut RustdianApp,
    dirty_paths: &[String],
    cx: &mut Context<RustdianApp>,
) {
    let root = app.vault.root_dir.clone();
    let prev_dims = app.ext.attach.files.dims.clone();
    let dirty = dirty_paths.to_vec();
    let task = cx
        .background_executor()
        .spawn(async move { index::scan_with_cache(&root, &prev_dims, &dirty) });
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

#[cfg(not(windows))]
fn is_image_path(path: &std::path::Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg"
                )
            })
}

fn clipboard_gpui_image(cx: &App) -> Option<Source> {
    let item = cx.read_from_clipboard()?;
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(Source::Clipboard {
            bytes: image.bytes.clone(),
            ext: import::ext_for_format(image.format),
        }),
        _ => None,
    })
}

/// Sources available from the clipboard (encoded images, `CF_DIB` screenshots, or copied image files).
fn clipboard_sources(cx: &App) -> Vec<Source> {
    #[cfg(windows)]
    {
        let sources = clipboard_win::read_pasteable_sources();
        if !sources.is_empty() {
            return sources;
        }
    }
    if let Some(src) = clipboard_gpui_image(cx) {
        return vec![src];
    }
    #[cfg(not(windows))]
    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
        let trimmed = text.trim();
        if !trimmed.is_empty() && !trimmed.contains(['\n', '\r']) {
            let path = PathBuf::from(trimmed);
            if is_image_path(&path) {
                return vec![Source::File(path)];
            }
        }
    }
    Vec::new()
}

/// True when the clipboard holds pasteable image bytes or files.
fn clipboard_has_pasteable(cx: &App) -> bool {
    #[cfg(windows)]
    {
        clipboard_win::has_pasteable_media() || clipboard_gpui_image(cx).is_some()
    }
    #[cfg(not(windows))]
    {
        !clipboard_sources(cx).is_empty()
    }
}

/// Ctrl+V or palette: imports the clipboard image/file at the caret when editing.
fn paste_image(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.active_path().is_none() {
        app.toast(ToastKind::Warn, "没有打开的笔记");
        return;
    }
    let sources = clipboard_sources(cx);
    if sources.is_empty() {
        app.toast(ToastKind::Warn, "剪贴板里没有图片");
        return;
    }
    let at_caret = matches!(app.view_mode, ViewMode::Source | ViewMode::Live);
    import_sources(app, sources, at_caret, cx);
}

/// Resolves an OS file path inside `vault_root` to a vault-relative Markdown note path.
fn vault_note_rel(vault_root: &std::path::Path, file: &std::path::Path) -> Option<String> {
    let rel = if let Ok(r) = file.strip_prefix(vault_root) {
        r.to_path_buf()
    } else {
        let root_c = std::fs::canonicalize(vault_root).ok()?;
        let file_c = std::fs::canonicalize(file).ok()?;
        file_c.strip_prefix(&root_c).ok()?.to_path_buf()
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    rel_str.ends_with(".md").then_some(rel_str)
}

/// Handles external files or clipboard images dropped onto the editor window.
/// Moves the caret to `at_pos` when editing in Source or Live mode before importing.
pub fn drop_sources_at(
    app: &mut RustdianApp,
    sources: Vec<Source>,
    at_pos: Option<gpui::Point<gpui::Pixels>>,
    window: &mut gpui::Window,
    cx: &mut Context<RustdianApp>,
) {
    if sources.is_empty()
        || app.show_palette
        || app.settings.open
        || crate::ui::ext::overlay_open(app)
    {
        return;
    }
    let Some(note_path) = app.active_path().map(str::to_string) else {
        let mut opened = false;
        for src in &sources {
            if let Source::File(path) = src
                && let Some(rel) = vault_note_rel(&app.vault.root_dir, path)
                && let Some(idx) = app.note_index(&rel)
            {
                app.open_note_by_index(idx);
                opened = true;
            }
        }
        if !opened {
            app.toast(ToastKind::Warn, "请先打开一篇笔记再拖入图片或附件");
        }
        cx.notify();
        return;
    };

    let at_caret = app.viewport_mode == crate::ViewportMode::Editor
        && matches!(app.view_mode, ViewMode::Source | ViewMode::Live);
    if at_caret && let Some(pos) = at_pos {
        app.ensure_editor(&note_path, cx);
        if let Some(editor) = app.editor_for(&note_path) {
            editor.update(cx, |ed, cx| {
                ed.place_caret_at_window(pos, window, cx);
            });
        }
    }
    import_sources(app, sources, at_caret, cx);
}

/// Files dropped from the OS via GPUI `ExternalPaths` (macOS / Linux).
pub fn drop_files(
    app: &mut RustdianApp,
    paths: &[PathBuf],
    at_pos: Option<gpui::Point<gpui::Pixels>>,
    window: &mut gpui::Window,
    cx: &mut Context<RustdianApp>,
) {
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
    drop_sources_at(app, sources, at_pos, window, cx);
}

/// Percent-encodes one path segment matching `jiyue_md::attach` link conventions.
fn encode_link_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for b in seg.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Relative, percent-encoded href from folder `from_dir` to vault-relative file `target_rel`.
pub(crate) fn relative_href(from_dir: &str, target_rel: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let target: Vec<&str> = target_rel.split('/').filter(|s| !s.is_empty()).collect();
    let common = from
        .iter()
        .zip(target.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts = Vec::new();
    for _ in common..from.len() {
        parts.push("..".to_string());
    }
    for seg in &target[common..] {
        parts.push(encode_link_segment(seg));
    }
    parts.join("/")
}

/// Formats a vault-relative file dropped from the sidebar tree as a Markdown/Wiki link.
pub(crate) fn format_tree_link(
    target_rel: &str,
    active_note_path: &str,
    link_style: LinkStyle,
    dup_note_stem: bool,
    dup_file_name: bool,
) -> String {
    let name = resolve::basename(target_rel);
    if let Some(stem_rel) = target_rel.strip_suffix(".md") {
        let stem = name.strip_suffix(".md").unwrap_or(name);
        let target = if dup_note_stem { stem_rel } else { stem };
        return format!("[[{target}]]");
    }
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    match link_style {
        LinkStyle::WikiEmbed => {
            let target = if dup_file_name { target_rel } else { name };
            format!("![[{target}]]")
        }
        LinkStyle::Markdown => {
            let href = relative_href(resolve::dir_of(active_note_path), target_rel);
            if jiyue_md::embeds::embed_kind(target_rel) == jiyue_md::embeds::EmbedKind::Image {
                format!("![{stem}]({href})")
            } else {
                format!("[{name}]({href})")
            }
        }
    }
}

/// Handles items dragged from the left file tree onto the editor at `pos`, inserting links.
pub fn drop_tree_items_at(
    app: &mut RustdianApp,
    items: &[(String, bool)],
    pos: gpui::Point<gpui::Pixels>,
    window: &mut gpui::Window,
    cx: &mut Context<RustdianApp>,
) {
    app.ext.files.drag_hover = None;
    if app.show_palette || app.settings.open || crate::ui::ext::overlay_open(app) {
        return;
    }
    let Some(note_path) = app.active_path().map(str::to_string) else {
        let mut opened = false;
        for (path, is_dir) in items {
            if !*is_dir
                && path.ends_with(".md")
                && let Some(idx) = app.note_index(path)
            {
                app.open_note_by_index(idx);
                opened = true;
            }
        }
        if !opened {
            app.toast(ToastKind::Warn, "请先打开一篇笔记");
        }
        cx.notify();
        return;
    };

    let link_style = app.ext.attach.settings.link_style;
    let mut links = Vec::new();
    for (path, is_dir) in items {
        if *is_dir {
            continue;
        }
        let name = resolve::basename(path);
        let dup_note_stem = if let Some(stem) = name.strip_suffix(".md") {
            app.vault
                .notes
                .iter()
                .filter(|n| resolve::basename(&n.path).strip_suffix(".md") == Some(stem))
                .count()
                > 1
        } else {
            false
        };
        let dup_file_name = app
            .ext
            .attach
            .files
            .files
            .iter()
            .filter(|f| resolve::basename(f) == name)
            .count()
            > 1;
        links.push(format_tree_link(
            path,
            &note_path,
            link_style,
            dup_note_stem,
            dup_file_name,
        ));
    }
    if links.is_empty() {
        return;
    }

    if app.view_mode == ViewMode::Read {
        app.set_view_mode(ViewMode::Live);
    }
    app.ensure_editor(&note_path, cx);
    let Some(editor) = app.editor_for(&note_path) else {
        return;
    };
    let live_mode = app.view_mode == ViewMode::Live;
    let (text, row, col) = editor.update(cx, |ed, cx| {
        ed.set_read_only(false, cx);
        ed.set_live(live_mode, cx);
        ed.place_caret_at_window(pos, window, cx);
        let (r, c) = ed.cursor_row_col();
        (ed.text(), r, c)
    });
    let joined = links.join(" ");
    let (new_text, r, c) = import::insert_at(&text, row - 1, col - 1, &joined);
    editor.update(cx, |ed, cx| {
        ed.set_text_undoable(&new_text, cx);
        ed.set_caret_to_row_col(r, c, cx);
    });
    app.save_path(&note_path, cx);
    cx.notify();
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
    if is_active && app.view_mode == ViewMode::Read {
        app.set_view_mode(ViewMode::Live);
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

/// Parses an anchor fragment (`^block-id` or `Heading`) after `#`.
fn parse_anchor(raw_anchor: &str) -> Option<Anchor> {
    let anchor = raw_anchor.trim();
    match anchor.strip_prefix('^') {
        Some(id) if !id.is_empty() => Some(Anchor::Block(id.to_string())),
        Some(_) => None,
        None if !anchor.is_empty() => Some(Anchor::Heading(anchor.to_string())),
        None => None,
    }
}

/// Splits a link body (`笔记#Heading`, `笔记#^id`, `#Heading`) into the note target and the anchor.
pub fn split_target(inner: &str) -> (&str, Option<Anchor>) {
    match inner.split_once('#') {
        None => (inner.trim(), None),
        Some((title, anchor)) => (title.trim(), parse_anchor(anchor)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FolderResolution {
    EntryNote(usize),
    FolderOnly(String),
    None,
}

fn find_note_path_ci(notes: &[jiyue_core::buffer::Note], candidate: &str) -> Option<usize> {
    notes
        .iter()
        .position(|n| n.path.replace('\\', "/").eq_ignore_ascii_case(candidate))
}

fn strip_index_prefix(stem: &str) -> &str {
    let bytes = stem.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return stem;
    }
    while i < bytes.len() && matches!(bytes[i], b'-' | b'_' | b'.' | b' ') {
        i += 1;
    }
    let rest = stem[i..].trim();
    if rest.is_empty() { stem } else { rest }
}

/// Matches a link target against vault folders and known index/entry notes (`<dir>/<dir>.md`,
/// `<dir>/README.md`, `<dir>/index.md`, `_indexes/<name>.md`).
pub(crate) fn resolve_folder_or_index(
    notes: &[jiyue_core::buffer::Note],
    folders: &[String],
    target_raw: &str,
    from_path: &str,
) -> FolderResolution {
    let normalized = target_raw.trim().replace('\\', "/");
    let clean = normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .trim_matches('/');
    if clean.is_empty() {
        return FolderResolution::None;
    }
    let base = clean.rsplit('/').next().unwrap_or(clean).trim();

    let from_dir = from_path
        .replace('\\', "/")
        .rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or_default();

    let mut matched_folder: Option<String> = None;
    if !from_dir.is_empty() {
        let rel_cand = format!("{from_dir}/{clean}");
        matched_folder = folders
            .iter()
            .find(|f| f.eq_ignore_ascii_case(&rel_cand))
            .cloned();
    }
    if matched_folder.is_none() {
        matched_folder = folders
            .iter()
            .find(|f| f.eq_ignore_ascii_case(clean))
            .cloned();
    }
    if matched_folder.is_none() && !clean.contains('/') {
        let mut leaf_matches: Vec<&String> = folders
            .iter()
            .filter(|f| {
                f.rsplit('/')
                    .next()
                    .is_some_and(|seg| seg.eq_ignore_ascii_case(clean))
            })
            .collect();
        leaf_matches.sort_by_key(|f| (f.chars().count(), f.as_str()));
        if let Some(first) = leaf_matches.first() {
            matched_folder = Some((*first).clone());
        }
    }

    if let Some(folder) = matched_folder.as_deref() {
        let folder_base = folder.rsplit('/').next().unwrap_or(folder);
        for candidate in [
            format!("{folder}/{folder_base}.md"),
            format!("{folder}/README.md"),
            format!("{folder}/index.md"),
        ] {
            if let Some(idx) = find_note_path_ci(notes, &candidate) {
                return FolderResolution::EntryNote(idx);
            }
        }
    }

    if !base.is_empty() {
        if let Some(idx) = find_note_path_ci(notes, &format!("_indexes/{base}.md")) {
            return FolderResolution::EntryNote(idx);
        }
        let mut idx_matches: Vec<usize> = notes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                let p = n.path.replace('\\', "/");
                let prefix = "_indexes/";
                let Some(file) = p
                    .get(..prefix.len())
                    .filter(|s| s.eq_ignore_ascii_case(prefix))
                    .and_then(|_| p.get(prefix.len()..))
                else {
                    return false;
                };
                if file.is_empty() || file.contains('/') {
                    return false;
                }
                let stem = if file.len() >= 3
                    && file.as_bytes()[file.len() - 3..].eq_ignore_ascii_case(b".md")
                {
                    &file[..file.len() - 3]
                } else {
                    file
                };
                strip_index_prefix(stem).eq_ignore_ascii_case(base)
            })
            .map(|(i, _)| i)
            .collect();
        if idx_matches.len() == 1 {
            return FolderResolution::EntryNote(idx_matches.remove(0));
        }
    }

    match matched_folder {
        Some(folder) => FolderResolution::FolderOnly(folder),
        None => FolderResolution::None,
    }
}

fn note_snippet(note: &jiyue_core::buffer::Note, label: &str) -> String {
    let snippet: Vec<&str> = note
        .content()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .take(3)
        .collect();
    if snippet.is_empty() {
        format!("[[{label}]]\n(空笔记)")
    } else {
        format!("[[{label}]]\n{}", snippet.join("\n"))
    }
}

/// Returns hover tooltip text for a wikilink target (`Title`, path, alias, or folder).
pub fn tooltip_preview(app: &RustdianApp, clean_title: &str) -> String {
    let from_path = app.active_path().unwrap_or("");
    let refs = crate::ui::files_ui::cached_note_refs(app);
    if let Some(idx) = jiyue_core::links::resolve_link(&refs, clean_title, from_path) {
        return note_snippet(&app.vault.notes[idx], clean_title);
    }
    match resolve_folder_or_index(&app.vault.notes, &app.vault.folders, clean_title, from_path) {
        FolderResolution::EntryNote(idx) => note_snippet(&app.vault.notes[idx], clean_title),
        FolderResolution::FolderOnly(folder) => {
            format!("📁 文件夹: {folder} · 点击在侧栏展开")
        }
        FolderResolution::None => format!("未创建笔记: [[{clean_title}]] · 点击立即新建"),
    }
}

fn open_resolved_note(app: &mut RustdianApp, idx: usize, anchor: Option<&Anchor>) {
    app.open_note_by_index(idx);
    if let Some(anchor) = anchor {
        jump_to_anchor(app, idx, anchor);
    }
}

fn reveal_folder_in_sidebar(app: &mut RustdianApp, folder: &str) {
    app.viewport_mode = crate::ViewportMode::Editor;
    app.sidebar_tab = crate::ui::sidebar::SidebarTab::Files;
    app.left_sidebar_open = true;
    crate::config_store::sync_ui(app);
    crate::ui::files_ui::expand_folder(app, folder);
    crate::ui::files_ui::reveal_path(app, folder);
}

/// Opens the note a wikilink points at and moves the caret to its heading or block. `raw` is the
/// link body as written (`笔记`, `笔记|别名`, `笔记#标题`, `笔记#^id`, `目录/笔记`, `#标题`).
/// Matches relative paths, frontmatter aliases, and folders before falling back to note creation.
pub fn navigate_link(app: &mut RustdianApp, raw: &str, cx: &mut Context<RustdianApp>) {
    let inner = raw.split('|').next().unwrap_or("").trim();
    let (target_raw, anchor) = split_target(inner);
    if target_raw.is_empty() {
        // `[[#Heading]]`: a link inside the open note.
        if let (Some(anchor), Some(path)) = (anchor, app.active_path().map(str::to_string))
            && let Some(idx) = app.vault.notes.iter().position(|n| n.path == path)
        {
            jump_to_anchor(app, idx, &anchor);
        }
        cx.notify();
        return;
    }

    let from_path = app.active_path().unwrap_or("").to_string();
    let refs = crate::ui::files_ui::cached_note_refs(app);
    if let Some(idx) = jiyue_core::links::resolve_link(&refs, target_raw, &from_path) {
        open_resolved_note(app, idx, anchor.as_ref());
        cx.notify();
        return;
    }

    match resolve_folder_or_index(&app.vault.notes, &app.vault.folders, target_raw, &from_path) {
        FolderResolution::EntryNote(idx) => open_resolved_note(app, idx, anchor.as_ref()),
        FolderResolution::FolderOnly(folder) => reveal_folder_in_sidebar(app, &folder),
        FolderResolution::None => match app.vault.create_link_note_at(target_raw) {
            Ok(idx) => {
                app.open_note_by_index(idx);
                app.pending_new_note = true;
            }
            Err(err) => app.toast(ToastKind::Warn, format!("无法打开或创建链接：{err}")),
        },
    }
    cx.notify();
}

/// Opens an internal Markdown link `[text](rel_path#anchor)`, decoding percent-escapes and
/// falling back cleanly to matching folders without creating stray files.
pub fn navigate_md_link(app: &mut RustdianApp, raw_dest: &str, cx: &mut Context<RustdianApp>) {
    let trimmed = raw_dest.trim();
    let (raw_path, raw_anchor) = match trimmed.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (trimmed, None),
    };
    let decoded_path = jiyue_core::links::percent_decode(raw_path)
        .trim()
        .to_string();
    let anchor = raw_anchor
        .map(jiyue_core::links::percent_decode)
        .and_then(|s| parse_anchor(&s));

    if decoded_path.is_empty() {
        if let (Some(anchor), Some(path)) = (anchor, app.active_path().map(str::to_string))
            && let Some(idx) = app.vault.notes.iter().position(|n| n.path == path)
        {
            jump_to_anchor(app, idx, &anchor);
        }
        cx.notify();
        return;
    }

    let from_path = app.active_path().unwrap_or("").to_string();
    let refs = crate::ui::files_ui::cached_note_refs(app);
    let occ = jiyue_core::links::LinkOccurrence {
        range: 0..0,
        target_range: 0..0,
        kind: jiyue_core::links::LinkKind::MdLink,
        target: decoded_path.clone(),
        anchor: None,
        alias: None,
    };
    if let Some(idx) = jiyue_core::links::resolve_occurrence(&refs, &occ, &from_path)
        .or_else(|| jiyue_core::links::resolve_link(&refs, &decoded_path, &from_path))
    {
        open_resolved_note(app, idx, anchor.as_ref());
        cx.notify();
        return;
    }

    match resolve_folder_or_index(
        &app.vault.notes,
        &app.vault.folders,
        &decoded_path,
        &from_path,
    ) {
        FolderResolution::EntryNote(idx) => open_resolved_note(app, idx, anchor.as_ref()),
        FolderResolution::FolderOnly(folder) => reveal_folder_in_sidebar(app, &folder),
        FolderResolution::None => {
            app.toast(ToastKind::Warn, format!("未找到链接目标：{decoded_path}"));
        }
    }
    cx.notify();
}

/// Opens the note `title` and, when there is an anchor, moves the caret to it. A missing note is
/// created like a plain wikilink.
pub fn open_target(
    app: &mut RustdianApp,
    title: &str,
    anchor: Option<Anchor>,
    cx: &mut Context<RustdianApp>,
) {
    let raw = match anchor {
        Some(Anchor::Heading(h)) => format!("{title}#{h}"),
        Some(Anchor::Block(id)) => format!("{title}#^{id}"),
        None => title.to_string(),
    };
    navigate_link(app, &raw, cx);
}

/// Moves the caret of note `idx` to the line of `anchor` without forcing Source mode. Does nothing
/// when the anchor is not in the note.
fn jump_to_anchor(app: &mut RustdianApp, idx: usize, anchor: &Anchor) {
    let Some(note) = app.vault.notes.get(idx) else {
        return;
    };
    if let Some(row) = anchor_row(note.content(), anchor) {
        app.request_caret(row, 0);
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

pub fn open_settings(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
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

#[cfg(test)]
mod link_resolution_tests {
    use super::*;
    use jiyue_core::buffer::Note;
    use jiyue_core::links::{self, NoteRef};

    #[test]
    fn folder_and_index_resolution_handles_readme_indexes_and_folder_only() {
        let notes = vec![
            Note::new(
                "后端与数据库",
                "💻 后端与数据库/README.md",
                "# README\n正文",
            ),
            Note::new("01-安全与逆向", "_indexes/01-安全与逆向.md", "# 索引\n正文"),
            Note::new(
                "工作流指南",
                "docs/workflow.md",
                "---\naliases:\n  - \"🧠 AI 与智能工作流\"\n---\n# 内容\n",
            ),
        ];
        let folders = vec![
            "_indexes".to_string(),
            "💻 后端与数据库".to_string(),
            "🏗️ 运维与基础设施".to_string(),
            "🔒 账号与隐私".to_string(),
        ];

        assert_eq!(
            resolve_folder_or_index(&notes, &folders, "💻 后端与数据库", "README.md"),
            FolderResolution::EntryNote(0)
        );
        assert_eq!(
            resolve_folder_or_index(&notes, &folders, "安全与逆向", "README.md"),
            FolderResolution::EntryNote(1)
        );
        assert_eq!(
            resolve_folder_or_index(&notes, &folders, "🏗️ 运维与基础设施", "README.md"),
            FolderResolution::FolderOnly("🏗️ 运维与基础设施".to_string())
        );
        assert_eq!(
            resolve_folder_or_index(&notes, &folders, "不存在的笔记", "README.md"),
            FolderResolution::None
        );

        let refs = vec![
            NoteRef::new(notes[0].path.clone()),
            NoteRef::new(notes[1].path.clone()),
            NoteRef::with_aliases(notes[2].path.clone(), &["🧠 AI 与智能工作流"]),
        ];
        assert_eq!(
            links::resolve_link(&refs, "🧠 AI 与智能工作流", "README.md"),
            Some(2)
        );
        assert_eq!(
            links::resolve_link(&refs, "💻 后端与数据库/README", "README.md"),
            Some(0)
        );
    }

    #[test]
    fn tree_drop_formats_notes_and_attachments() {
        assert_eq!(
            format_tree_link(
                "笔记/随笔.md",
                "首页.md",
                LinkStyle::WikiEmbed,
                false,
                false
            ),
            "[[随笔]]"
        );
        assert_eq!(
            format_tree_link("笔记/随笔.md", "首页.md", LinkStyle::Markdown, true, false),
            "[[笔记/随笔]]"
        );
        assert_eq!(
            format_tree_link(
                "附件/截图 1.png",
                "笔记/首页.md",
                LinkStyle::WikiEmbed,
                false,
                false
            ),
            "![[截图 1.png]]"
        );
        assert_eq!(
            format_tree_link(
                "附件/pic 1.png",
                "笔记/sub/首页.md",
                LinkStyle::Markdown,
                false,
                false
            ),
            "![pic 1](../../%E9%99%84%E4%BB%B6/pic%201.png)"
        );
    }
}
