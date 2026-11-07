#![allow(dead_code)]
// Release builds on Windows use the GUI subsystem, so starting jiyue.exe from Explorer, the Start
// Menu or a shortcut does not open a console window next to the app. Debug builds keep the console
// for log output.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub use jiyue_core::{buffer, daily, editor_state, md_edit, vault};

mod config_store;
pub mod i18n;
mod jobs;
mod journal;
mod launch;
mod session;
mod theme;
mod ui;
pub mod update;
mod vaults;

use gpui::{
    AnyElement, App, Application, Bounds, Context, Entity, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Render, SharedString, Subscription, Task, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};
use session::{
    AUTOSAVE_DELAY, Debounce, GRAPH_DEBOUNCE, SAVE_ERROR_SHOW, SAVING_FLASH, SaveStatus, ViewMode,
    mode_after_setting, retain_paths, save_status,
};
use theme::*;
use ui::text_input::{MoveDocEnd, OpenWikilink, TextEditor};
use vault::Vault;

/// Viewport display mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportMode {
    Editor,
    GraphView,
}

/// How often the configuration files are checked for changes.
const CONFIG_POLL: Duration = Duration::from_secs(2);

/// Application configuration (values from mechanics.json)
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub base_font_size: f32,
    pub tab_size: usize,
    pub line_height_normal: f32,
    pub header_height: f32,
    pub sidebar_width: f32,
    pub radius_s: f32,
    pub radius_m: f32,
    pub radius_l: f32,
    pub font_ui_small: f32,
    pub font_ui_medium: f32,
    pub readable_line_length: bool,
    pub show_line_number: bool,
    pub fold_heading: bool,
    pub auto_pair_markdown: bool,
    pub auto_pair_brackets: bool,
    pub vim_mode: bool,
    pub live_preview: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_font_size: BASE_FONT_SIZE,
            tab_size: TAB_SIZE,
            line_height_normal: LINE_HEIGHT_NORMAL,
            header_height: HEADER_HEIGHT,
            sidebar_width: SIDEBAR_WIDTH,
            radius_s: RADIUS_S,
            radius_m: RADIUS_M,
            radius_l: RADIUS_L,
            font_ui_small: FONT_UI_SMALL,
            font_ui_medium: FONT_UI_MEDIUM,
            readable_line_length: READABLE_LINE_LENGTH,
            show_line_number: SHOW_LINE_NUMBER,
            fold_heading: FOLD_HEADING,
            auto_pair_markdown: AUTO_PAIR_MARKDOWN,
            auto_pair_brackets: AUTO_PAIR_BRACKETS,
            vim_mode: VIM_MODE,
            live_preview: LIVE_PREVIEW,
        }
    }
}

/// Tabs restored on start-up when the vault contains these notes (the demo vault's
/// design layout). Otherwise the vault's own default tabs are kept.
const RESTORED_TABS: &[&str] = &["沉积的想法", "GPUI 笔记", "2026-11-03"];

/// One editor per open note. Lives in `RustdianApp::editors`, keyed by the note's path.
pub struct NoteEditor {
    pub editor: Entity<TextEditor>,
    /// Copies the editor's changes into the note and schedules saves and rebuilds.
    _observe: Subscription,
    /// Ctrl+click on a wikilink in the editor opens the target note.
    _links: Subscription,
    /// Dirty flag as of the last observed change (the tab strip repaints when it flips).
    dirty_seen: bool,
    autosave_debounce: Debounce,
    /// The pending autosave timer. Replacing it cancels the previous one.
    autosave: Option<Task<()>>,
    /// The text was restored from a recovery copy and is not on disk yet. It counts as unsaved,
    /// and autosave waits for an explicit save.
    pub restored: bool,
    /// Modification time (ms) of the disk file when the current unsaved edit began.
    pub base_mtime_ms: Option<i64>,
    /// A recovery copy is queued for the next journal tick.
    journal_pending: bool,
    journal_task: Option<Task<()>>,
    /// The last save failed; the retry timer is running and the toast is already shown.
    save_failing: bool,
}

impl NoteEditor {
    fn new(editor: Entity<TextEditor>, observe: Subscription, links: Subscription) -> Self {
        Self {
            editor,
            _observe: observe,
            _links: links,
            dirty_seen: false,
            autosave_debounce: Debounce::default(),
            autosave: None,
            restored: false,
            base_mtime_ms: None,
            journal_pending: false,
            journal_task: None,
            save_failing: false,
        }
    }
}

/// Recovered text waiting for its editor to open (see `RustdianApp::restore_recovered`).
pub struct PendingRestore {
    pub text: String,
    pub base_mtime_ms: Option<i64>,
}

/// Save retry delay after a failed write.
const SAVE_RETRY: Duration = Duration::from_secs(5);
/// Delay between recovery copies while text changes (at most one copy per delay). Shorter than
/// the 1.5 s autosave, so the first copy of an edit is on disk before autosave writes the note.
const JOURNAL_DELAY: Duration = Duration::from_millis(1000);

/// The main application state
pub struct RustdianApp {
    pub config: AppConfig,
    /// Live state of the configuration files (broken files, external edits). See `config_store`.
    pub configs: config_store::ConfigStore,
    /// True while the settings poll loop runs (see `ensure_settings_watch`).
    pub settings_watch_live: bool,
    /// Re-checks the configuration files when the window is activated again (focus).
    pub activation_sub: Option<gpui::Subscription>,
    pub theme: ActiveTheme,
    pub vault: Vault,
    /// Settings value (外观/编辑器 toggle). `true` makes the rendered view the default.
    pub live_preview: bool,
    /// How the active note is shown: editor, rendered preview with click-to-edit, or read-only.
    pub view_mode: ViewMode,
    pub viewport_mode: ViewportMode,
    /// The split view (分屏) and the tab strips' state (see `ui/split.rs`).
    pub layout: ui::split::Layout,
    pub left_sidebar_open: bool,
    pub right_sidebar_open: bool,
    pub show_palette: bool,
    pub is_command_mode: bool, // true: command palette (actions), false: quick switcher (notes)
    pub palette_query: String,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub collapsed_headings: Vec<usize>,
    /// Folder keys (`想法`, `项目/子目录`) that are expanded in the file tree.
    pub expanded_folders: Vec<String>,
    /// Keyboard focus for the window (shortcuts are handled on the root element).
    pub focus: FocusHandle,
    pub settings: ui::settings::SettingsState,
    pub palette: ui::palette::PaletteState,
    /// The 快捷键 settings page (see `ui/keymap_ui.rs`).
    pub keymap_ui: ui::keymap_ui::State,
    pub graph_ui: ui::graph::GraphState,
    pub calendar: ui::calendar::CalendarState,
    /// Pointer is over the title-bar close button (its glyph turns white on the red hover).
    pub close_hovered: bool,
    /// Editors of the notes shown so far, keyed by note path (indices shift, paths do not).
    editors: HashMap<String, NoteEditor>,
    /// Path of the note that was active at the last sync (detects switches).
    last_active_path: Option<String>,
    /// `live_preview` as seen by the last sync (detects settings toggles).
    live_seen: bool,
    /// Give keyboard focus to the target (active editor or root) at the next render.
    want_focus: bool,
    /// Move the caret of the active editor to the end once it is shown.
    caret_to_end: bool,
    /// Move the caret of the active editor to (row, col) (zero based) once it is shown. Set by a
    /// click on a rendered block or on an outline entry; takes precedence over `caret_to_end`.
    caret_at: Option<(usize, usize)>,
    /// The next active note is new: open it in Source with the caret in the body.
    pending_new_note: bool,
    /// Save the active note at the next render (Ctrl+S, the save command).
    save_requested: bool,
    /// The status bar shows `保存中…` until this instant.
    saving_until: Option<Instant>,
    /// The status bar shows `保存失败` until this instant.
    save_error_until: Option<Instant>,
    graph_debounce: Debounce,
    graph_task: Option<Task<()>>,
    /// Whether the palette or settings overlay was open at the last render (focus follows it).
    overlay_open: bool,
    /// `--open widgets`: the widget catalogue is drawn as an overlay (see `ext::overlays`).
    pub open_widgets_demo: bool,
    /// State and toasts of the feature modules (see `ui/ext.rs`).
    pub ext: ui::ext::ExtState,
    /// Module commands queued by `run_command(AppCommand::Ext(..))`. `run_command` has no
    /// `Context`, so the queue is drained at the next render, which runs them with `cx`.
    /// Use `dispatch_ext` instead when a `Context` is at hand.
    pub pending_ext: Vec<ui::ext::ExtCommand>,
    /// Back and forward history of the visited notes (see `nav_history`).
    pub nav: ui::nav_history::NavHistory,
    /// `ExtEvent::Started` has been sent (once, at the first render).
    ext_started: bool,
    /// Theme as last reported to the modules (detects cycles and explicit sets).
    ext_theme_seen: ThemeKind,
    /// Writer of the recovery journal (see `journal.rs`).
    pub journals: journal::Journals,
    /// Recovered text per note, waiting for the note's editor to open.
    pending_restore: HashMap<String, PendingRestore>,
    /// Notes saved by this process; their versions are flushed on exit.
    session_saved: HashSet<String>,
    /// The window may close even though a save failed (set by the confirmation).
    pub force_close: bool,
}

impl RustdianApp {
    /// While a settings page is open, the configuration files are checked every few seconds. Nothing
    /// runs while settings are closed; focus re-checks the files (see `activation_sub`).
    fn ensure_settings_watch(&mut self, cx: &mut Context<Self>) {
        if !self.settings.open || self.settings_watch_live {
            return;
        }
        self.settings_watch_live = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CONFIG_POLL).await;
                let keep = this.update(cx, |app, cx| {
                    if app.settings.open {
                        app.poll_config(cx);
                        true
                    } else {
                        app.settings_watch_live = false;
                        false
                    }
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Re-checks the configuration files: reloads the shortcuts when their file changed, and tells
    /// the user about each change (or about the broken file, which the app runs without).
    pub fn poll_config(&mut self, cx: &mut Context<Self>) {
        let changed = self.configs.poll();
        if changed.is_empty() {
            return;
        }
        let user_config = jiyue_core::config::user_file(jiyue_core::config::USER_CONFIG_FILE);
        if changed.iter().any(|p| p == &user_config) {
            // Initialize i18n from user configuration
            i18n::set_locale(&config_store::saved_locale());
            ui::keymap::init_from_disk();
            ui::keymap::rebind(cx);
        }
        match self.configs.broken().first() {
            Some(issue) => self.toast(
                ui::ext::ToastKind::Error,
                format!("配置文件有错误，程序继续使用默认值：{issue}"),
            ),
            None => self.toast(ui::ext::ToastKind::Info, "配置已重新载入"),
        }
        cx.notify();
    }

    /// Loads the vault at `vault_dir` and builds the initial state with `kind` as theme.
    pub fn new(
        cx: &mut Context<Self>,
        kind: ThemeKind,
        vault_dir: &Path,
        open: Option<&str>,
        launch_notes: &[String],
    ) -> Self {
        let vault = Vault::open_or_create(vault_dir);
        let journals = journal::Journals::start(vault.root_dir.clone());
        let mut app = Self {
            journals,
            pending_restore: HashMap::new(),
            session_saved: HashSet::new(),
            force_close: false,
            config: AppConfig::default(),
            configs: Default::default(),
            settings_watch_live: false,
            activation_sub: None,
            theme: ActiveTheme::from_kind(kind),
            vault,
            live_preview: true,
            view_mode: ViewMode::Live,
            viewport_mode: ViewportMode::Editor,
            layout: Default::default(),
            left_sidebar_open: true,
            right_sidebar_open: true,
            show_palette: false,
            is_command_mode: false,
            palette_query: String::new(),
            cursor_row: 1,
            cursor_col: 1,
            collapsed_headings: Vec::new(),
            expanded_folders: Vec::new(),
            focus: cx.focus_handle(),
            settings: Default::default(),
            palette: Default::default(),
            keymap_ui: Default::default(),
            graph_ui: Default::default(),
            calendar: Default::default(),
            close_hovered: false,
            editors: HashMap::new(),
            last_active_path: None,
            live_seen: true,
            want_focus: false,
            caret_to_end: false,
            caret_at: None,
            pending_new_note: false,
            save_requested: false,
            saving_until: None,
            save_error_until: None,
            graph_debounce: Debounce::default(),
            graph_task: None,
            overlay_open: false,
            open_widgets_demo: false,
            ext: Default::default(),
            pending_ext: Vec::new(),
            nav: ui::nav_history::NavHistory::default(),
            ext_started: false,
            ext_theme_seen: kind,
        };
        ui::palette::install(&mut app, cx);
        ui::keymap_ui::install(&mut app, cx);
        ui::graph::install(&mut app, cx);
        let user_dir = jiyue_core::config::user_dir();
        app.configs.open(&user_dir, Some(&app.vault.root_dir));
        // The interface preferences of the last run (the theme comes from `--theme` or the saved
        // value, see `parse_args`). Written back by `config_store::sync_ui` when they change.
        let ui = app.configs.ui_prefs_on_disk();
        app.live_preview = ui.live_preview;
        app.left_sidebar_open = ui.left_sidebar;
        app.right_sidebar_open = ui.right_sidebar;
        if let Some(notice) = app.configs.startup_notice.clone() {
            app.toast(
                ui::ext::ToastKind::Error,
                format!("{notice}；程序使用默认值运行"),
            );
        }
        app.restore_tabs();
        if open == Some("daily") {
            app.focus_restored_daily_tab();
        }
        // Notes given on the command line open as tabs; the last one ends up focused.
        for rel in launch_notes {
            if let Some(idx) = app.note_index(rel) {
                app.open_note_by_index(idx);
            }
        }
        app.expanded_folders = app
            .vault
            .active_note()
            .map(|n| ui::sidebar::default_expanded(&n.path))
            .unwrap_or_default();
        app.view_mode = ViewMode::from_live_preview(app.live_preview);
        app.live_seen = app.live_preview;
        match open {
            Some("palette") => {
                app.show_palette = true;
                app.is_command_mode = true;
            }
            Some("switcher") => {
                app.show_palette = true;
                app.is_command_mode = false;
            }
            Some("settings") => app.settings.open = true,
            Some("library") => {
                app.settings.open = true;
                app.settings.category = ui::settings::CAT_LIBRARY;
            }
            Some("calendar") => app.calendar.collapsed = false,
            Some("graph") => app.viewport_mode = ViewportMode::GraphView,
            Some("editor") => app.view_mode = ViewMode::Source,
            Some("widgets") => app.open_widgets_demo = true,
            _ => {}
        }
        app.overlay_open = app.show_palette || app.settings.open || ui::ext::overlay_open(&app);
        app.last_active_path = app.active_path().map(str::to_string);
        if let Some(path) = app.last_active_path.clone() {
            app.ensure_editor(&path, cx);
        }
        ui::files_ui::start_recovery(&mut app, cx);
        ui::ext::init(&mut app, cx);
        app
    }

    /// Window focus at start-up: the active editor in Source mode when no overlay is open.
    pub fn initial_focus(&self, cx: &App) -> FocusHandle {
        self.focus_target(cx)
    }

    /// Restores the design's tab set when those notes exist.
    fn restore_tabs(&mut self) {
        // The tabs, their order, the active tab and the split of the last run come first.
        if ui::split::restore(self) {
            return;
        }
        let indices: Vec<usize> = RESTORED_TABS
            .iter()
            .filter_map(|title| {
                self.vault
                    .notes
                    .iter()
                    .position(|n| n.title.eq_ignore_ascii_case(title))
            })
            .collect();
        if indices.len() == RESTORED_TABS.len() {
            self.vault.open_tabs = indices;
            self.vault.active_tab_index = 0;
            self.vault.active_note_index = self.vault.open_tabs[0];
        }
    }

    /// Makes the restored daily-note tab (`RESTORED_TABS[2]`) the active note.
    fn focus_restored_daily_tab(&mut self) {
        let daily_title = RESTORED_TABS[2];
        let tab = self
            .vault
            .open_tabs
            .iter()
            .enumerate()
            .find(|(_, note_idx)| {
                self.vault
                    .notes
                    .get(**note_idx)
                    .is_some_and(|n| n.title.eq_ignore_ascii_case(daily_title))
            });
        if let Some((tab_idx, &note_idx)) = tab {
            self.vault.active_tab_index = tab_idx;
            self.vault.active_note_index = note_idx;
        }
    }

    /// Path of the active note.
    pub fn active_path(&self) -> Option<&str> {
        self.vault.active_note().map(|n| n.path.as_str())
    }

    fn note_index(&self, path: &str) -> Option<usize> {
        self.vault.notes.iter().position(|n| n.path == path)
    }

    /// The editor of the note at `path`, if one exists.
    pub fn editor_for(&self, path: &str) -> Option<Entity<TextEditor>> {
        self.editors.get(path).map(|e| e.editor.clone())
    }

    /// Whether the note at `path` has unsaved editor changes.
    pub fn is_dirty(&self, path: &str, cx: &App) -> bool {
        self.editors
            .get(path)
            .is_some_and(|e| e.restored || e.editor.read(cx).is_dirty())
    }

    /// Save state of the active note, for the status bar.
    pub fn active_save_status(&self, cx: &App) -> SaveStatus {
        let dirty = self.active_path().is_some_and(|p| self.is_dirty(p, cx));
        save_status(dirty, self.is_saving(), self.save_failed())
    }

    fn is_saving(&self) -> bool {
        self.saving_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn save_failed(&self) -> bool {
        self.save_error_until
            .is_some_and(|until| Instant::now() < until)
    }

    /// Creates the editor for the note at `path` from its current text, once.
    fn ensure_editor(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.editors.contains_key(path) {
            return;
        }
        let Some(note) = self.vault.notes.iter().find(|n| n.path == path) else {
            return;
        };
        let text = note.content().to_string();
        let theme = self.theme;
        let editor = cx.new(|cx| TextEditor::new(&text, theme, cx));
        let live = self.ext.attach.live.clone();
        let vault_path = path.to_string();
        editor.update(cx, |ed, cx| ed.set_vault(live, &vault_path, cx));
        let key = path.to_string();
        let observe = cx.observe(&editor, move |this, _, cx| this.on_editor_changed(&key, cx));
        let links = cx.subscribe(&editor, |this, _, ev: &OpenWikilink, cx| {
            ui::attach_ui::navigate_link(this, &ev.0, cx);
        });
        let mut entry = NoteEditor::new(editor, observe, links);
        // Recovered text: the note already holds it, and it stays unsaved until the user saves.
        if let Some(pending) = self.pending_restore.remove(path) {
            entry.restored = true;
            entry.base_mtime_ms = pending.base_mtime_ms;
        }
        self.editors.insert(path.to_string(), entry);
    }

    /// Runs when an editor notifies: copies its text into the note (the editor is the source of
    /// truth), schedules the autosave and the graph rebuild, and follows the caret and dirty flag.
    fn on_editor_changed(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_for(path) else {
            return;
        };
        let (text, dirty, cursor) = {
            let ed = editor.read(cx);
            (ed.text(), ed.is_dirty(), ed.cursor_row_col())
        };
        let mut changed = false;
        let mut edited = false;
        if let Some(note) = self
            .note_index(path)
            .and_then(|idx| self.vault.notes.get_mut(idx))
            && note.content() != text
        {
            note.set_content(&text);
            edited = true;
        }
        let mut turned_dirty = false;
        let mut turned_clean = false;
        if let Some(entry) = self.editors.get_mut(path) {
            let dirty = dirty || entry.restored;
            if entry.dirty_seen != dirty {
                entry.dirty_seen = dirty;
                changed = true;
                turned_dirty = dirty;
                turned_clean = !dirty;
            }
        }
        if turned_dirty {
            self.begin_unsaved_edit(path);
        }
        if turned_clean {
            self.journals.remove(path);
        }
        if edited {
            changed = true;
            self.schedule_graph_rebuild(cx);
            self.schedule_autosave(path, cx);
            self.schedule_journal(path, cx);
            ui::files_ui::autoname::on_edit(self, path, cx);
            ui::ext::on_event(self, &ui::ext::ExtEvent::NoteChanged(path.to_string()), cx);
        }
        if self.active_path() == Some(path) && cursor != (self.cursor_row, self.cursor_col) {
            self.cursor_row = cursor.0;
            self.cursor_col = cursor.1;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// Marks the start of an unsaved edit: the disk modification time at that point is kept, unless
    /// a recovered edit already recorded one.
    fn begin_unsaved_edit(&mut self, path: &str) {
        let (_, mtime) = journal::disk_state(&self.vault.root_dir, path);
        if let Some(entry) = self.editors.get_mut(path)
            && entry.base_mtime_ms.is_none()
        {
            entry.base_mtime_ms = mtime;
        }
    }

    /// Queues a recovery copy of the unsaved text: at most one per `JOURNAL_DELAY`, so typing
    /// without pauses still gets copies.
    fn schedule_journal(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.editors.get_mut(path) else {
            return;
        };
        if entry.journal_pending {
            return;
        }
        entry.journal_pending = true;
        let key = path.to_string();
        entry.journal_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JOURNAL_DELAY).await;
            let _ = this.update(cx, |app, cx| app.journal_tick(&key, cx));
        }));
    }

    /// Writes the recovery copy of a note that is still dirty (off the UI thread, see `journals`).
    fn journal_tick(&mut self, path: &str, cx: &App) {
        match self.editors.get_mut(path) {
            Some(entry) => entry.journal_pending = false,
            None => return,
        }
        if !self.is_dirty(path, cx) {
            return;
        }
        let Some(editor) = self.editor_for(path) else {
            return;
        };
        let text = editor.read(cx).text();
        let base_mtime_ms = self.editors.get(path).and_then(|e| e.base_mtime_ms);
        self.journals.write(jiyue_core::recovery::Journal {
            path: path.to_string(),
            base_mtime_ms,
            saved_at_ms: journal::now_ms(),
            text,
        });
        if journal::test_panic_requested() {
            panic!("JIYUE_TEST_PANIC: forced panic for the crash test");
        }
    }

    /// Retries a failed save after `SAVE_RETRY`, while the note is still dirty and no newer change
    /// has replaced the timer.
    fn schedule_retry(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.editors.get_mut(path) else {
            return;
        };
        let token = entry.autosave_debounce.restart();
        let key = path.to_string();
        entry.autosave = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_RETRY).await;
            let _ = this.update(cx, |app, cx| app.retry_due(&key, token, cx));
        }));
    }

    fn retry_due(&mut self, path: &str, token: u64, cx: &mut Context<Self>) {
        let current = self
            .editors
            .get(path)
            .is_some_and(|e| e.autosave_debounce.is_current(token));
        if current && self.is_dirty(path, cx) && self.save_path(path, cx) {
            let open = self.open_paths().iter().any(|p| p == path);
            let active = self.active_path() == Some(path);
            if !open && !active {
                self.editors.remove(path);
            }
        }
        cx.notify();
    }

    /// Rebuilds the graph and backlinks data after a pause in typing (one pending rebuild).
    fn schedule_graph_rebuild(&mut self, cx: &mut Context<Self>) {
        let token = self.graph_debounce.restart();
        self.graph_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(GRAPH_DEBOUNCE).await;
            let _ = this.update(cx, |app, cx| {
                if app.graph_debounce.is_current(token) {
                    // Built on a background thread from a copy of the notes; the result is applied
                    // only if no newer change was scheduled meanwhile (that change schedules its own).
                    let sources = app.vault.graph_sources();
                    let built = cx
                        .background_executor()
                        .spawn(async move { jiyue_core::vault::build_graph(&sources) });
                    cx.spawn(async move |this, cx| {
                        let graph = built.await;
                        let _ = this.update(cx, |app, cx| {
                            if app.graph_debounce.is_current(token) {
                                app.vault.graph = graph;
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
                cx.notify();
            });
        }));
    }

    /// Starts the autosave timer for `path`. Replacing the stored task cancels the old timer.
    fn schedule_autosave(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.editors.get_mut(path) else {
            return;
        };
        let token = entry.autosave_debounce.restart();
        let key = path.to_string();
        entry.autosave = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            let _ = this.update(cx, |app, cx| app.autosave_due(&key, token, cx));
        }));
    }

    fn autosave_due(&mut self, path: &str, token: u64, cx: &mut Context<Self>) {
        let current = self
            .editors
            .get(path)
            .is_some_and(|e| e.autosave_debounce.is_current(token));
        // Recovered text stays off the disk until the user saves it explicitly.
        let held = self.editors.get(path).is_some_and(|e| e.restored);
        if current && !held && self.is_dirty(path, cx) {
            self.save_path(path, cx);
        }
        cx.notify();
    }

    /// Writes the editor text of the note at `path` to its file and marks it saved.
    /// Returns false when the write failed (the status bar then shows `保存失败`).
    /// The caller repaints.
    fn save_path(&mut self, path: &str, cx: &mut Context<Self>) -> bool {
        let Some(idx) = self.note_index(path) else {
            return true;
        };
        let Some(editor) = self.editor_for(path) else {
            return true;
        };
        let text = editor.read(cx).text();
        if let Some(note) = self.vault.notes.get_mut(idx)
            && note.content() != text
        {
            note.set_content(&text);
        }
        match self.vault.save_note_to_disk(idx) {
            Ok(()) => {
                editor.update(cx, |ed, _| ed.mark_saved());
                if let Some(entry) = self.editors.get_mut(path) {
                    entry.dirty_seen = false;
                    entry.restored = false;
                    entry.base_mtime_ms = None;
                    entry.save_failing = false;
                }
                self.journals.remove(path);
                self.session_saved.insert(path.to_string());
                self.save_error_until = None;
                self.saving_until = Some(Instant::now() + SAVING_FLASH);
                Self::notify_after(cx, SAVING_FLASH);
                ui::ext::on_event(self, &ui::ext::ExtEvent::NoteSaved(path.to_string()), cx);
                true
            }
            Err(err) => {
                // One toast per failure streak; the retry timer keeps trying.
                let first = match self.editors.get_mut(path) {
                    Some(entry) => {
                        let first = !entry.save_failing;
                        entry.save_failing = true;
                        first
                    }
                    None => true,
                };
                if first {
                    self.toast(
                        ui::ext::ToastKind::Error,
                        format!("保存失败，内容已保留在恢复文件中：{path}（{err}）"),
                    );
                }
                self.save_error_until = Some(Instant::now() + SAVE_ERROR_SHOW);
                Self::notify_after(cx, SAVE_ERROR_SHOW);
                self.schedule_retry(path, cx);
                false
            }
        }
    }

    /// Saves the active note (Ctrl+S).
    fn save_active(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.active_path().map(str::to_string) {
            self.save_path(&path, cx);
        }
    }

    /// Saves every note with unsaved changes (window close, best effort).
    /// Returns the paths whose save failed (their text stays in the recovery copy).
    pub fn save_all_dirty(&mut self, cx: &mut Context<Self>) -> Vec<String> {
        let dirty: Vec<String> = {
            let app_cx: &App = cx;
            self.editors
                .keys()
                .filter(|p| self.is_dirty(p, app_cx))
                .cloned()
                .collect()
        };
        let mut failed = Vec::new();
        for path in dirty {
            if !self.save_path(&path, cx) {
                failed.push(path);
            }
        }
        failed
    }

    /// Everything that must reach disk before the window goes. Dirty notes are saved; the notes whose
    /// save failed are returned, and their recovery copies are written now, on this thread. The
    /// copies queued so far are flushed, and the notes saved in this run get a version of their text
    /// at once, whatever the history's 2-minute interval says.
    pub fn prepare_exit(&mut self, cx: &mut Context<Self>) -> Vec<String> {
        let failed = self.save_all_dirty(cx);
        let root = self.vault.root_dir.clone();
        for path in &failed {
            let text = self.editor_for(path).map(|e| e.read(cx).text());
            let base_mtime_ms = self.editors.get(path).and_then(|e| e.base_mtime_ms);
            if let Some(text) = text {
                journal::write_now(
                    &root,
                    &jiyue_core::recovery::Journal {
                        path: path.clone(),
                        base_mtime_ms,
                        saved_at_ms: journal::now_ms(),
                        text,
                    },
                );
            }
        }
        self.journals.flush(Duration::from_secs(2));
        if let Some(store) = ui::history_ui::store_of(self) {
            for path in self.session_saved.iter() {
                if let Some(note) = self.vault.notes.iter().find(|n| n.path == *path) {
                    let _ = ui::history_ui::keep_now(&store, path, note.content());
                }
            }
            if let Ok(mut history) = store.lock() {
                let _ = history.flush();
            }
        }
        failed
    }

    /// Repaints once `delay` has passed (clears the transient status texts).
    fn notify_after(cx: &mut Context<Self>, delay: Duration) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// Paths whose notes are open (the focused pane's tabs, plus the other pane's when split).
    fn open_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .vault
            .open_tabs
            .iter()
            .filter_map(|&i| self.vault.notes.get(i))
            .map(|n| n.path.clone())
            .collect();
        if let Some(split) = self.layout.split.as_ref() {
            paths.extend(split.other.tabs.iter().cloned());
        }
        paths
    }

    /// Keeps editors in step with the vault. Runs at the start of every render, where the
    /// window is available for focus.
    ///  - Ctrl+S / the save command writes the active note.
    ///  - Editors of deleted notes are dropped; editors of closed notes are saved, then dropped.
    ///  - When the active note changes, the one left is saved and the new one is focused.
    ///  - The active note always has an editor, and its theme and caret are current.
    fn sync_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.save_requested) {
            self.save_active(cx);
        }
        // The other pane's notes are kept in step first, so the open-note list below is current.
        ui::split::sync(self);
        // Theme, sidebars and live preview are saved as soon as they change (see config_store).
        config_store::sync_ui(self);
        // The window title shows the vault's name (see `config_store::sync_window_title`).
        config_store::sync_window_title(self, window);

        // The 外观/编辑器 setting picks the default mode when it changes.
        if let Some(mode) = mode_after_setting(self.live_seen, self.live_preview) {
            self.view_mode = mode;
            self.want_focus = true;
        }
        self.live_seen = self.live_preview;

        let existing: Vec<String> = self.vault.notes.iter().map(|n| n.path.clone()).collect();
        retain_paths(&mut self.editors, |p| existing.iter().any(|e| e == p));

        let open = self.open_paths();
        let active = self.active_path().map(str::to_string);
        let closed: Vec<String> = self
            .editors
            .keys()
            .filter(|p| !open.contains(*p) && active.as_deref() != Some(p.as_str()))
            .cloned()
            .collect();
        for path in closed {
            // A failed save keeps the editor, so the text is not lost. Its retry timer saves it
            // later; this loop does not retry on every frame.
            if !self.is_dirty(&path, cx)
                || (!self.editors.get(&path).is_some_and(|e| e.save_failing)
                    && self.save_path(&path, cx))
            {
                self.editors.remove(&path);
            }
        }

        if std::mem::take(&mut self.pending_new_note) {
            self.view_mode = ViewMode::Source;
            self.want_focus = true;
            self.caret_to_end = true;
        }

        if active != self.last_active_path {
            if let Some(prev) = self.last_active_path.clone()
                && self.is_dirty(&prev, cx)
            {
                self.save_path(&prev, cx);
            }
            self.last_active_path = active.clone();
            self.want_focus = true;
            if let Some(path) = active.clone() {
                ui::ext::on_event(self, &ui::ext::ExtEvent::NoteOpened(path), cx);
            }
        }

        if let Some(path) = active.clone() {
            self.ensure_editor(&path, cx);
        }
        // The other pane's note has an editor of its own when it differs from the focused note.
        let other_active = self
            .layout
            .split
            .as_ref()
            .and_then(|s| s.other.active.clone());
        if let Some(path) = other_active {
            self.ensure_editor(&path, cx);
        }
        ui::text_input::context::sync(&self.ext.attach.live, self);
        let theme = self.theme;
        if let Some(editor) = active.as_deref().and_then(|p| self.editor_for(p)) {
            editor.update(cx, |ed, cx| ed.set_theme(theme, cx));
            let (row, col) = editor.read(cx).cursor_row_col();
            self.cursor_row = row;
            self.cursor_col = col;
        }

        ui::palette::sync_open(self, cx);
        let overlay = self.show_palette || self.settings.open || ui::ext::overlay_open(self);
        if overlay != self.overlay_open {
            self.overlay_open = overlay;
            self.want_focus = true;
        }
        if std::mem::take(&mut self.want_focus) {
            let handle = self.focus_target(cx);
            window.focus(&handle);
        }
        ui::ai_ui::focus_on_open(self, window, cx);
        ui::template_ui::focus_on_open(self, window, cx);

        // After a click on the rendered text the caret goes to the end. The editor is shown in
        // this frame, so the move runs in the next one, where its focus is in the rendered tree.
        // A click on a rendered block or an outline entry sets `caret_at`; a click on empty space
        // sets `caret_to_end`. `caret_at` wins.
        let caret_at = self.caret_at.take();
        let caret_end = std::mem::take(&mut self.caret_to_end);
        if (caret_at.is_some() || caret_end)
            && self.view_mode.is_source()
            && let Some(editor) = active.as_deref().and_then(|p| self.editor_for(p))
        {
            window.refresh();
            window.on_next_frame(move |window, cx| {
                editor.read(cx).focus(window);
                match caret_at {
                    Some((row, col)) => {
                        editor.update(cx, |ed, cx| ed.set_caret_to_row_col(row, col, cx));
                    }
                    None => {
                        window.dispatch_action(Box::new(MoveDocEnd), cx);
                    }
                }
            });
        }
    }

    /// The handle that should hold keyboard focus right now.
    fn focus_target(&self, cx: &App) -> FocusHandle {
        if self.show_palette
            && let Some(input) = &self.palette.input
        {
            return input.read(cx).focus_handle(cx);
        }
        if let Some(handle) = ui::files_ui::inline_focus_handle(self, cx) {
            return handle;
        }
        let overlay = self.show_palette || self.settings.open || ui::ext::overlay_open(self);
        if !overlay
            && self.view_mode.is_source()
            && let Some(editor) = self.active_path().and_then(|p| self.editor_for(p))
        {
            return editor.read(cx).focus_handle(cx);
        }
        self.focus.clone()
    }

    /// Switches the view mode from the pill. Keyboard focus follows the mode.
    pub fn set_view_mode(&mut self, mode: ViewMode) {
        if mode != self.view_mode {
            self.view_mode = mode;
            self.want_focus = true;
        }
    }

    /// A click on empty space of the rendered note at `path`: Live becomes Source with the caret
    /// at the end. Ignored when that note is not the active one, and when a block already placed
    /// the caret for this click (see `click_rendered_line`).
    pub fn click_rendered_text(&mut self, path: &str) {
        if self.active_path() != Some(path) || self.caret_at.is_some() {
            return;
        }
        let next = self.view_mode.on_click();
        if next != self.view_mode {
            self.view_mode = next;
            self.caret_to_end = true;
            self.want_focus = true;
        }
    }

    /// A click on a rendered block of the note at `path`, which is at source `row` and `col`
    /// (zero based). Live becomes Source with the caret there. Ignored for other notes.
    pub fn click_rendered_line(&mut self, path: &str, row: usize, col: usize) {
        if self.active_path() != Some(path) {
            return;
        }
        let next = self.view_mode.on_click();
        if next != self.view_mode {
            self.view_mode = next;
            self.caret_at = Some((row, col));
            self.want_focus = true;
        }
    }

    /// Opens the active note in Source with the caret at `row` and `col` (zero based), for example
    /// from an outline entry.
    pub fn request_source_caret(&mut self, row: usize, col: usize) {
        self.set_view_mode(ViewMode::Source);
        self.caret_at = Some((row, col));
        self.want_focus = true;
        self.cursor_row = row + 1;
        self.cursor_col = col + 1;
    }

    /// Opens the note a wikilink points at. `raw` is the link body (`Title`, `Title|alias`,
    /// `Title#Heading`) or a bare title. The target is opened like a sidebar click, so the view
    /// mode stays as it was. A missing note is created in the vault root and opened in Source
    /// with the caret in its body, like Ctrl+N. An unsafe title does nothing.
    pub fn open_wikilink(&mut self, raw: &str) {
        let Some(title) = vault::safe_note_title(ui::inline::wikilink_target(raw)) else {
            return;
        };
        if let Some(idx) = self.vault.note_index_by_title(&title) {
            self.open_note_by_index(idx);
        } else if let Ok(idx) = self.vault.create_link_note(&title) {
            self.open_note_by_index(idx);
            self.pending_new_note = true;
        }
    }

    /// Flips the task on source line `line_no` (1-based) of the note at `path`, from the rendered
    /// preview. The editor is created if needed and the caret and selection are kept. The toggle is
    /// one undo step, the note becomes dirty and autosave writes it.
    pub fn toggle_task_line(&mut self, path: &str, line_no: usize, cx: &mut Context<Self>) {
        self.ensure_editor(path, cx);
        let Some(editor) = self.editor_for(path) else {
            return;
        };
        editor.update(cx, |ed, cx| {
            let (anchor, head) = ed.selection_offsets();
            let offset = ed.line_start(line_no.saturating_sub(1));
            if ed.toggle_task_at(offset, cx) {
                ed.restore_selection(anchor, head, cx);
            }
        });
    }

    /// Open note by title
    pub fn open_note_by_title(&mut self, title: &str) {
        let _ = self.vault.open_note_by_title(title);
        self.show_palette = false;
        self.viewport_mode = ViewportMode::Editor;
    }

    /// Open note by index
    pub fn open_note_by_index(&mut self, idx: usize) {
        if idx < self.vault.notes.len() {
            self.vault.active_note_index = idx;
            if !self.vault.open_tabs.contains(&idx) {
                self.vault.open_tabs.push(idx);
            }
            if let Some(tab_idx) = self.vault.open_tabs.iter().position(|&i| i == idx) {
                self.vault.active_tab_index = tab_idx;
            }
        }
        self.show_palette = false;
        self.viewport_mode = ViewportMode::Editor;
    }

    /// Close tab. The note stays active when it is still open; its editor is saved and
    /// dropped by the next sync.
    pub fn close_tab(&mut self, tab_idx: usize) {
        // In a split view the focused pane's last tab closes the pane (see `ui::split`).
        ui::split::close_tab(self, tab_idx);
    }

    /// Creates an untitled note and opens it in Source mode. The name is the first free `未命名 N`
    /// (an existing note of that name is never overwritten).
    pub fn new_note(&mut self) {
        let mut count = self.vault.notes.len() + 1;
        while self
            .vault
            .notes
            .iter()
            .any(|n| n.path == format!("未命名 {count}.md"))
        {
            count += 1;
        }
        let title = format!("未命名 {}", count);
        self.vault.create_new_note(&title);
        ui::files_ui::autoname::track_new(self, &format!("{title}.md"));
        self.viewport_mode = ViewportMode::Editor;
        self.pending_new_note = true;
    }

    /// Opens today's daily note, creating it from the template if needed.
    pub fn open_today(&mut self) {
        if let Ok(idx) = self.vault.open_or_create_daily(daily::Date::today()) {
            self.open_note_by_index(idx);
        }
    }

    /// Cycles through the five themes (Ctrl+T).
    pub fn cycle_theme(&mut self) {
        self.theme = ActiveTheme::from_kind(self.theme.kind.next());
    }

    /// Toggle heading collapse
    pub fn toggle_heading_collapse(&mut self, line_no: usize) {
        if let Some(pos) = self.collapsed_headings.iter().position(|&l| l == line_no) {
            self.collapsed_headings.remove(pos);
        } else {
            self.collapsed_headings.push(line_no);
        }
    }

    /// Keyboard shortcuts on the root element.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        if ui::keymap_ui::capture_key(self, ev, window, cx)
            || ui::ext::handle_key(self, ev, cx)
            || ui::settings::handle_key(self, ev)
            || ui::palette::handle_key(self, ev)
        {
            return;
        }
        // Customisable shortcuts (ui/keymap.rs): the Global commands of the registry.
        if let Some(id) = ui::keymap::lookup_key(&ev.keystroke, ui::keymap::When::Global) {
            self.run_registry_command(id);
            return;
        }
        if key == "escape" {
            let overlay = self.show_palette || self.settings.open || ui::ext::overlay_open(self);
            self.show_palette = false;
            if !overlay && self.view_mode.is_source() {
                self.view_mode = self.view_mode.on_escape();
                self.want_focus = true;
            }
        }
    }

    /// Runs a registry command that is not an `AppCommand` (tabs and panes, see `ui/keymap.rs`).
    /// Commands that are `AppCommand`s go through `run_command`.
    fn run_registry_command(&mut self, id: &str) {
        if let Some(cmd) = ui::keymap::action_for(id) {
            self.run_command(cmd);
            return;
        }
        match id {
            "view.tab_prev" => ui::tabbar::tab_key(self, false, false),
            "view.tab_next" => ui::tabbar::tab_key(self, true, false),
            "pane.tab_move_prev" => ui::tabbar::tab_key(self, false, true),
            "pane.tab_move_next" => ui::tabbar::tab_key(self, true, true),
            "pane.focus_prev" => ui::split::step_focus(self, false),
            "pane.focus_next" => ui::split::step_focus(self, true),
            _ => {}
        }
    }
}

/// Everything the UI can ask the app to do. UI modules call `app.run_command(..)`.
#[derive(Clone)]
pub enum AppCommand {
    NewNote,
    SaveNote,
    CycleTheme,
    SetTheme(ThemeKind),
    OpenGraph,
    /// Splits when there is no split, closes the split otherwise.
    ToggleSplit,
    /// 向右拆分: a second pane (side by side) with the same tabs, focused.
    SplitRight,
    /// 向下拆分: a second pane (stacked) with the same tabs, focused.
    SplitDown,
    /// 关闭分屏: the focused pane stays, the other closes.
    CloseSplit,
    OpenQuickSwitcher,
    OpenCommandPalette,
    ClosePalette,
    OpenSettings,
    CloseSettings,
    OpenToday,
    OpenDaily(daily::Date),
    DailyPrev,
    DailyNext,
    ToggleSidebar,
    ToggleRightPanel,
    OpenNote(usize),
    /// 后退 / 前进: back and forward in the note history (`nav_history`).
    NavBack,
    NavForward,
    /// A command of a feature module (`ui/ext.rs`).
    Ext(ui::ext::ExtCommand),
}

impl RustdianApp {
    /// Executes a command. Callers are responsible for `cx.notify()`.
    ///
    /// `run_command` takes no `Context`, so `AppCommand::Ext` is only queued in `pending_ext`;
    /// `render` runs the queue with a `Context` before it draws. Listeners that hold a `cx`
    /// should call `dispatch_ext` instead, which runs the command at once.
    pub fn run_command(&mut self, cmd: AppCommand) {
        match cmd {
            AppCommand::Ext(c) => self.pending_ext.push(c),
            AppCommand::NewNote => self.new_note(),
            AppCommand::SaveNote => self.save_requested = true,
            AppCommand::CycleTheme => self.cycle_theme(),
            AppCommand::SetTheme(kind) => self.theme = ActiveTheme::from_kind(kind),
            AppCommand::OpenGraph => self.viewport_mode = ViewportMode::GraphView,
            AppCommand::ToggleSplit => ui::split::toggle(self),
            AppCommand::SplitRight => ui::split::split(self, ui::split_model::SplitDir::Right),
            AppCommand::SplitDown => ui::split::split(self, ui::split_model::SplitDir::Down),
            AppCommand::CloseSplit => ui::split::close_split(self),
            AppCommand::OpenQuickSwitcher => {
                self.show_palette = true;
                self.is_command_mode = false;
                self.palette_query.clear();
            }
            AppCommand::OpenCommandPalette => {
                self.show_palette = true;
                self.is_command_mode = true;
                self.palette_query.clear();
            }
            // Focus moves on at the next render: a command that opens a module dialog keeps the
            // overlay flag set, so the flag alone would leave focus on the closed palette input.
            AppCommand::ClosePalette => {
                self.show_palette = false;
                self.want_focus = true;
            }
            AppCommand::OpenSettings => {
                self.show_palette = false;
                self.settings.open = true;
            }
            AppCommand::CloseSettings => self.settings.open = false,
            AppCommand::OpenToday => self.open_today(),
            AppCommand::OpenDaily(date) => {
                if let Ok(idx) = self.vault.open_or_create_daily(date) {
                    self.open_note_by_index(idx);
                }
            }
            AppCommand::DailyPrev => self.open_adjacent_daily(false),
            AppCommand::DailyNext => self.open_adjacent_daily(true),
            AppCommand::NavBack => ui::nav_history::go(self, false),
            AppCommand::NavForward => ui::nav_history::go(self, true),
            AppCommand::ToggleSidebar => self.left_sidebar_open = !self.left_sidebar_open,
            AppCommand::ToggleRightPanel => self.right_sidebar_open = !self.right_sidebar_open,
            AppCommand::OpenNote(idx) => self.open_note_by_index(idx),
        }
    }

    /// Runs a feature-module command now. Callers are responsible for `cx.notify()`.
    pub fn dispatch_ext(&mut self, cmd: ui::ext::ExtCommand, cx: &mut Context<Self>) {
        ui::ext::run(self, cmd, cx);
    }

    /// Previous / next existing daily note relative to the active daily note (or today).
    fn open_adjacent_daily(&mut self, forward: bool) {
        let fmt = self.vault.daily_settings().date_format;
        let base = self
            .vault
            .active_note()
            .and_then(|n| daily::parse_date(&n.title, &fmt))
            .unwrap_or_else(daily::Date::today);
        if let Some(date) = self.vault.daily_adjacent_existing(base, forward)
            && let Ok(idx) = self.vault.open_or_create_daily(date)
        {
            self.open_note_by_index(idx);
        }
    }
}

impl RustdianApp {
    /// Feature-module housekeeping at the start of every render (it has a `Context`): runs the
    /// queued `AppCommand::Ext` commands, sends `Started` once and `ThemeChanged` when the theme
    /// differs from the last report, and drops expired toasts.
    fn drive_ext(&mut self, cx: &mut Context<Self>) {
        for cmd in std::mem::take(&mut self.pending_ext) {
            ui::ext::run(self, cmd, cx);
        }
        ui::nav_history::observe(self);
        if !self.ext_started {
            self.ext_started = true;
            ui::ext::on_event(self, &ui::ext::ExtEvent::Started, cx);
        }
        if self.theme.kind != self.ext_theme_seen {
            self.ext_theme_seen = self.theme.kind;
            ui::ext::on_event(self, &ui::ext::ExtEvent::ThemeChanged, cx);
        }
        self.ext.toasts.purge();
        // Problems found while reading keymap.json (see ui/keymap.rs), reported once.
        for warning in ui::keymap::take_warnings() {
            self.toast(ui::ext::ToastKind::Warn, warning);
        }
    }
}

impl Render for RustdianApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_settings_watch(cx);
        self.drive_ext(cx);
        self.sync_editors(window, cx);

        let t = self.theme;
        let this: &RustdianApp = self;

        let main_column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .pr(px(8.0))
            .pb(px(8.0))
            .child(ui::tabbar::render_tabbar(this, window, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .bg(t.sheet)
                    .rounded_tr(px(RADIUS_L))
                    .rounded_br(px(RADIUS_L))
                    .rounded_bl(px(RADIUS_L))
                    .shadow_md()
                    .overflow_hidden()
                    .child(ui::crumb::render_crumb(this, cx))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_1()
                            .min_h_0()
                            .child(this.editor_area(cx))
                            .when(this.right_sidebar_open, |d| {
                                d.child(ui::right_panel::render_right_panel(this, cx))
                            }),
                    )
                    .child(ui::statusbar::render_statusbar(this, cx)),
            );

        div()
            .relative()
            .flex()
            .flex_row()
            .size_full()
            .bg(t.chrome)
            .text_color(t.tx)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.on_key(ev, window, cx);
                cx.notify();
            }))
            // A tab drag that ended without a drop target ends here (drops clear it themselves).
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _window, cx| {
                    if this.layout.tab_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            // Mouse buttons 4 and 5 go back and forward in the note history (see `nav_history`).
            .on_mouse_down(
                gpui::MouseButton::Navigate(gpui::NavigationDirection::Back),
                cx.listener(|this, _: &gpui::MouseDownEvent, _window, cx| {
                    this.run_command(AppCommand::NavBack);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Navigate(gpui::NavigationDirection::Forward),
                cx.listener(|this, _: &gpui::MouseDownEvent, _window, cx| {
                    this.run_command(AppCommand::NavForward);
                    cx.notify();
                }),
            )
            .when(self.left_sidebar_open, |d| {
                d.child(ui::sidebar::render_sidebar(this, cx))
            })
            .child(main_column)
            // The caption buttons float over the title bar's right end (see `ui/tabbar.rs`).
            .child(ui::tabbar::render_captions(this, window, cx))
            .when(self.show_palette, |d| {
                d.child(this.render_palette_modal(cx))
            })
            .when(self.settings.open, |d| d.child(this.render_settings(cx)))
            .when(self.keymap_ui.confirm_reset_all, |d| {
                d.child(ui::keymap_ui::render_confirm(this, cx))
            })
            // Feature-module overlays and the toast layer come last, so they paint on top.
            .children(ui::ext::overlays(this, cx))
    }
}

impl RustdianApp {
    /// The editor slot: graph view, the note, or the two panes of a split (see `ui/split.rs`).
    fn editor_area(&self, cx: &mut Context<Self>) -> AnyElement {
        ui::split::render_editor_area(self, cx)
    }
}

/// Command line options.
struct Cli {
    theme: ThemeKind,
    /// The vault to open. `None` shows the first-run chooser (see `launch::choose_startup`).
    vault: Option<PathBuf>,
    /// `--wait-parent PID`: a relaunched copy waits for that process to exit first (see `vaults.rs`).
    wait_parent: Option<u32>,
    /// `--open palette|switcher|settings|library|graph|calendar|daily|editor|widgets`: start with that screen open (for captures).
    /// `library` is the 笔记库 page of the settings dialog.
    /// `widgets` shows the widget catalogue (`ui::widgets::demo`) on top of the app.
    /// `daily` activates the restored daily-note tab. `editor` opens the active note in Source mode.
    open: Option<String>,
    /// Note files given on the command line, as vault-relative paths (see `launch.rs`).
    notes: Vec<String>,
}

fn parse_args() -> Cli {
    let args: Vec<String> = env::args().skip(1).collect();
    let cwd = env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let plan = launch::resolve_launch(&args, &cwd, &launch::RealFs);
    // `--theme` wins when it names a known theme; otherwise the theme saved in config.toml; otherwise ink.
    if let Some(name) = plan.theme.as_deref()
        && !jiyue_core::config::prefs::is_known_theme(name)
    {
        eprintln!("unknown theme '{name}', using the saved theme");
    }
    let saved = config_store::saved_theme();
    let theme_name = jiyue_core::config::prefs::startup_theme(plan.theme.as_deref(), &saved);
    let theme = ThemeKind::from_name(&theme_name).unwrap_or(ThemeKind::Ink);
    let (recent, warning) = vaults::load_store(&vaults::store_path());
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    let vault = match launch::choose_startup(&plan, recent.last_opened(), &launch::RealFs) {
        launch::Startup::Open(path) => Some(PathBuf::from(path)),
        launch::Startup::Chooser => None,
    };
    Cli {
        theme,
        vault,
        wait_parent: vaults::parse_wait_parent(&args),
        open: plan.open,
        notes: plan.notes,
    }
}

/// The first-run chooser window (no vault is known yet).
fn open_chooser(cx: &mut App, kind: ThemeKind) {
    let bounds = Bounds::centered(None, size(px(720.0), px(560.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from("霁月 Jiyue")),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(640.0), px(480.0))),
        window_decorations: cfg!(target_os = "linux").then_some(gpui::WindowDecorations::Client),
        ..Default::default()
    };
    let _ = cx.open_window(options, move |_, cx| {
        cx.new(|cx| ui::vaults_ui::VaultChooser::new(cx, kind))
    });
}

fn main() {
    // Crash safety first: a panic writes the recovery copies and the crash log (see journal.rs).
    journal::install_panic_hook();
    // The Mermaid renderer (mermaid-rs-renderer) caches font metrics under $XDG_CACHE_HOME/mmdr,
    // or $HOME/.cache/mmdr when that is unset. Point it at the app's own cache folder, before any
    // thread exists, so nothing is written to the user's home cache.
    // The folder is per OS (`jiyue_core::platform::cache_dir`; `%LOCALAPPDATA%\Jiyue\cache` on Windows).
    let cache_root = jiyue_core::platform::cache_dir();
    // SAFETY: this runs first in main, before any other thread is started.
    unsafe { std::env::set_var("XDG_CACHE_HOME", cache_root) };
    let cli = parse_args();
    let kind = cli.theme;
    let vault_dir = cli.vault;
    let open = cli.open;
    let notes = cli.notes;
    // A relaunched copy (vault switch) waits until the old process has released the vault.
    if let Some(pid) = cli.wait_parent {
        vaults::wait_for_parent_exit(pid, Duration::from_secs(10));
    }
    if let Some(dir) = &vault_dir
        && let Err(err) = vaults::record_opened(&dir.display().to_string())
    {
        eprintln!("无法记录最近笔记库：{err}");
    }

    Application::new()
        .with_assets(ui::assets::Assets)
        .run(move |cx: &mut App| {
            // The place and size of the last run (state.toml [window]), or centred 1280 x 800.
            let window_bounds = config_store::startup_window(cx, (1280.0, 800.0));
            // Windows: `appears_transparent` removes the native title bar (WM_NCCALCSIZE) and the
            // tab strip becomes the title bar (see ui/tabbar.rs). The title is kept for the taskbar.
            let options = WindowOptions {
                window_bounds: Some(window_bounds),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("霁月 Jiyue")),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(960.0), px(600.0))),
                // Linux: the tab strip is the title bar, so the desktop's own bar is turned off.
                // Windows and macOS keep their defaults (the native frame is kept or hidden by
                // `appears_transparent`).
                window_decorations: cfg!(target_os = "linux")
                    .then_some(gpui::WindowDecorations::Client),
                ..Default::default()
            };
            // The shortcut registry: read keymap.json, then bind the GPUI keys from it.
            let _ = jiyue_core::config::ensure_template(
                &jiyue_core::config::user_file(jiyue_core::config::USER_CONFIG_FILE),
                jiyue_core::config::templates::USER_CONFIG,
            );
            // Initialize i18n from user configuration
            i18n::set_locale(&config_store::saved_locale());
            ui::keymap::init_from_disk();
            ui::keymap::rebind(cx);
            let Some(vault_dir) = vault_dir else {
                open_chooser(cx, kind);
                cx.activate(true);
                return;
            };
            cx.open_window(options, move |window, cx| {
                let view =
                    cx.new(|cx| RustdianApp::new(cx, kind, &vault_dir, open.as_deref(), &notes));
                // Focus: check the configuration files again (cheap, once per activation).
                let activation = view.update(cx, |_, cx| {
                    cx.observe_window_activation(window, |app: &mut RustdianApp, _, cx| {
                        app.poll_config(cx)
                    })
                });
                view.update(cx, |app, _| app.activation_sub = Some(activation));
                let focus = view.read(cx).initial_focus(cx);
                window.focus(&focus);

                // Auto update: silently check for updates in background 10 seconds after startup.
                let update_view = view.clone();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(Duration::from_secs(10)).await;
                    let _ = update_view.update(cx, |app, cx| {
                        app.trigger_check_update(false, cx);
                    });
                })
                .detach();
                // Window close: save what is unsaved (best effort), then close.
                let on_close = view.clone();
                // The sync module saves dirty notes and may keep the window open for one last
                // sync (bounded, see `sync_ui::on_close_requested`).
                window.on_window_should_close(cx, move |window, cx| {
                    config_store::save_window(window);
                    on_close.update(cx, ui::sync_ui::on_close_requested)
                });
                view
            })
            .unwrap();
            cx.activate(true);
        });
    // The event loop has ended: the old window and its locks are gone. A vault switch starts the
    // new copy now (see `vaults::request_relaunch`).
    if let Some(request) = vaults::take_relaunch()
        && let Err(err) = vaults::spawn_relaunch(&request)
    {
        eprintln!("无法打开所选笔记库：{err}");
    }
}
