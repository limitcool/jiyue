//! AI UI: side chat (current note, selection or vault), selection actions with diff preview,
//! automatic title, tags and summary, the vector index service, and the AI settings category.
//! Owned by W2-I in wave 2.
//!
//! Layout: this file holds the state and the hooks that `ui/ext.rs` calls. The rest lives in
//! child modules: `chat` (side panel flow), `panel` (side panel view), `selection` (Ctrl+J menu,
//! result card), `notes` (title, tags, summary), `modal` (dialogs), `settings` (AI settings),
//! `runner` (pumping background results into the UI), `editing` (applying text to the editor),
//! and the pure logic in `context`, `markdown`, `diff`, `errors`, `indexer`, `worker`.
//!
//! Privacy: nothing leaves the machine unless AI is enabled, the note is not excluded, and the
//! user confirmed the preview (questions) or pressed the action (selection and note actions).

pub mod chat;
pub mod context;
pub mod diff;
pub mod editing;
pub mod errors;
pub mod indexer;
pub mod markdown;
pub mod modal;
pub mod notes;
pub mod panel;
pub mod runner;
pub mod selection;
pub mod settings;
pub mod worker;

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Context, Entity, KeyDownEvent, MouseButton, ScrollHandle, Subscription, Window,
    div, prelude::*, px,
};
use jiyue_ai::{AiConfig, CancelToken};

use crate::RustdianApp;

/// Focuses the composer in the frame the panel opens, whatever opened it (shortcut, status bar,
/// palette). Called every frame from the root render.
pub fn focus_on_open(app: &mut RustdianApp, window: &mut Window, cx: &mut Context<RustdianApp>) {
    let open = app.ext.ai.panel_open;
    if open == app.ext.ai.panel_was_open {
        return;
    }
    app.ext.ai.panel_was_open = open;
    if open && let Some(draft) = app.ext.ai.inputs.draft.clone() {
        draft.read(cx).focus(window);
    }
}

use crate::ui::UI_FONT;
use crate::ui::ext::{ExtCommand, ExtEvent, PaletteCommand};
use crate::ui::icons::icon;
use crate::ui::text_input::TextEditor;
use crate::ui::widgets::{LineInput, LineInputEvent};
use crate::{AppCommand, theme::ActiveTheme};

use context::NoteSnap;
use indexer::DebouncedQueue;

/// How often the background ticker checks the re-embedding queue.
const TICK: Duration = Duration::from_secs(2);

/// Chips under the composer: which sources the next question may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chips {
    pub note: bool,
    pub selection: bool,
    pub vault: bool,
}

impl Default for Chips {
    fn default() -> Self {
        Self {
            note: true,
            selection: false,
            vault: false,
        }
    }
}

/// Text fields owned by the AI UI. Created once in `init`.
#[derive(Default)]
pub struct Inputs {
    pub draft: Option<Entity<LineInput>>,
    pub rewrite: Option<Entity<LineInput>>,
    pub name: Option<Entity<LineInput>>,
    pub url: Option<Entity<LineInput>>,
    pub chat_model: Option<Entity<LineInput>>,
    pub embed_model: Option<Entity<LineInput>>,
    pub api_key: Option<Entity<LineInput>>,
    pub folder: Option<Entity<LineInput>>,
    pub tag: Option<Entity<LineInput>>,
}

impl Inputs {
    /// Every field that exists, for theme changes.
    pub fn all(&self) -> Vec<Entity<LineInput>> {
        [
            &self.draft,
            &self.rewrite,
            &self.name,
            &self.url,
            &self.chat_model,
            &self.embed_model,
            &self.api_key,
            &self.folder,
            &self.tag,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
    }
}

/// Which dialog is open. Only one at a time.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    #[default]
    None,
    /// Consent before the first use: what is sent, and to whom.
    Consent,
    /// Instruction for 改写 on the selection.
    Rewrite,
    /// Confirmation before a whole note is sent for 总结.
    Summary,
    /// Confirmation before a provider is removed.
    DeleteProvider(String),
}

/// Index service state, shown in the status bar and settings.
pub struct IndexUi {
    pub queue: DebouncedQueue,
    /// Counts runs. A finished run only changes the state when its number is still current.
    pub generation: u64,
    /// Set while a run is in progress. Cancelling it pauses the run.
    pub running: Option<CancelToken>,
    pub done: usize,
    pub total: usize,
    pub stats: Option<indexer::IndexStats>,
    pub error: Option<String>,
}

impl Default for IndexUi {
    fn default() -> Self {
        Self {
            queue: DebouncedQueue::new(indexer::REEMBED_DELAY),
            generation: 0,
            running: None,
            done: 0,
            total: 0,
            stats: None,
            error: None,
        }
    }
}

/// Module state, one field per concern.
#[derive(Default)]
pub struct State {
    pub config: AiConfig,
    /// Why `ai.json` could not be read (the defaults are used then).
    pub config_error: Option<String>,
    /// Debug builds only: the provider from `JIYUE_TEST_OPENAI_*`.
    pub test: Option<worker::TestEnv>,
    pub panel_open: bool,
    /// The panel state of the last frame (see `focus_on_open`).
    pub panel_was_open: bool,
    pub chips: Chips,
    pub inputs: Inputs,
    pub messages: Vec<chat::UiMessage>,
    pub next_id: u64,
    /// A prepared question waiting for the user to confirm the send.
    pub preview: Option<chat::Preview>,
    /// Retrieval for a preview is running.
    pub preparing: bool,
    /// A chat stream is running for this message.
    pub busy: Option<chat::Busy>,
    pub modal: Modal,
    /// Text the open dialog acts on (the selection for 改写, the note for 总结).
    pub target: Option<selection::Target>,
    pub menu: Option<selection::Menu>,
    pub card: Option<selection::Card>,
    pub index: IndexUi,
    pub settings: settings::Ui,
    pub scroll: ScrollHandle,
    pub subs: Vec<Subscription>,
    /// The model name the server reported for the last answer (shown in the panel footer when it
    /// differs from the configured model: gateways alias model names).
    pub last_answered_by: Option<String>,
}

impl State {
    /// A fresh message id.
    pub fn take_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

/// Commands this module handles (sent as `ExtCommand::Ai`).
#[derive(Clone, Debug)]
pub enum Command {
    TogglePanel,
    AskVault,
    RewriteSelection,
    SummarizeNote,
    SuggestTitle,
    SuggestTags,
    ActionMenu,
}

/// Creates a themed single-line input.
pub fn new_input(
    cx: &mut Context<RustdianApp>,
    theme: ActiveTheme,
    placeholder: &str,
) -> Entity<LineInput> {
    let input = cx.new(|cx| LineInput::new(cx, placeholder.to_string()));
    input.update(cx, |i, cx| i.set_theme(theme, cx));
    input
}

pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let theme = app.theme;
    let inputs = Inputs {
        draft: Some(new_input(cx, theme, "问点什么…（Enter 发送）")),
        rewrite: Some(new_input(cx, theme, "改写要求，例如：更简洁、改成口语")),
        name: Some(new_input(cx, theme, "服务名称，例如 本地模型")),
        url: Some(new_input(cx, theme, "https://api.example.com/v1")),
        chat_model: Some(new_input(cx, theme, "对话模型，例如 gpt-4o-mini")),
        embed_model: Some(new_input(cx, theme, "嵌入模型（可留空）")),
        api_key: Some(new_input(cx, theme, "API 密钥（保存后不再显示）")),
        folder: Some(new_input(cx, theme, "文件夹，例如 私人/日记")),
        tag: Some(new_input(cx, theme, "标签，例如 secret")),
    };
    if let Some(input) = &inputs.api_key {
        input.update(cx, |i, cx| i.set_password(true, cx));
    }
    app.ext.ai.inputs = inputs;

    if let Some(draft) = app.ext.ai.inputs.draft.clone() {
        let sub = cx.subscribe(
            &draft,
            |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
                if let LineInputEvent::Submit(_) = ev {
                    chat::send_draft(this, cx);
                }
            },
        );
        app.ext.ai.subs.push(sub);
    }
    if let Some(rewrite) = app.ext.ai.inputs.rewrite.clone() {
        let sub = cx.subscribe(
            &rewrite,
            |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
                if let LineInputEvent::Submit(_) = ev {
                    selection::confirm_rewrite_from_modal(this, cx);
                }
            },
        );
        app.ext.ai.subs.push(sub);
    }

    app.ext.ai.test = worker::test_env_from_process();
    match load_config(&app.vault.root_dir) {
        Ok(cfg) => app.ext.ai.config = cfg,
        Err(e) => app.ext.ai.config_error = Some(e.to_string()),
    }
    let ticker = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            let alive = this.update(cx, tick);
            if alive.is_err() {
                break;
            }
        }
    });
    ticker.detach();

    // Debug builds only: JIYUE_TEST_AI_PANEL=1 opens the side panel at start (for GUI tests).
    if cfg!(debug_assertions) && std::env::var("JIYUE_TEST_AI_PANEL").as_deref() == Ok("1") {
        app.ext.ai.panel_open = true;
    }
    settings::refresh_keys(app, cx);
    if index_wanted(app) {
        indexer_start_full(app, cx);
    }
}

/// Note title for a citation chip: the file name without folder and `.md`.
pub fn title_of(path: &str) -> String {
    jiyue_ai::prompts::title_from_path(path)
}

/// The configuration as the features see it: the saved file, plus the debug-only test provider.
pub fn effective_config(state: &State) -> AiConfig {
    let mut cfg = state.config.clone();
    if let Some(env) = &state.test {
        if !cfg.providers.iter().any(|p| p.id == worker::TEST_ID) {
            cfg.providers.push(worker::test_entry(env));
        }
        if cfg.active_chat.is_none() {
            cfg.active_chat = Some(worker::TEST_ID.to_string());
        }
        if cfg.active_embedding.is_none() && env.embed.is_some() {
            cfg.active_embedding = Some(worker::TEST_ID.to_string());
        }
    }
    cfg
}

/// Host part of a base URL, for "sent to" notices. Never includes a path or credentials.
pub fn host_of(url: &str) -> String {
    let rest = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = rest.split('/').next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    if host.is_empty() {
        "未知服务器".into()
    } else {
        host.to_string()
    }
}

/// Tags of a note for the exclusion rule: the tags the vault found (inline `#tags`) plus the
/// `tags` of its frontmatter, which the vault does not index.
pub fn exclusion_tags(content: &str, tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = tags.to_vec();
    for tag in jiyue_md::frontmatter::tags(content) {
        if !out
            .iter()
            .any(|x| x.trim_start_matches('#').eq_ignore_ascii_case(&tag))
        {
            out.push(tag);
        }
    }
    out
}

/// Snapshot of every note in the vault with its exclusion decision.
pub fn snapshot_notes(app: &RustdianApp, cfg: &AiConfig) -> Vec<NoteSnap> {
    app.vault
        .notes
        .iter()
        .map(|n| {
            let tags = exclusion_tags(n.content(), &n.tags);
            NoteSnap::new(cfg, &n.path, &n.title, n.content(), &tags)
        })
        .collect()
}

/// Whether the index service should run: AI on, an embedding provider chosen.
pub fn index_wanted(app: &RustdianApp) -> bool {
    effective_config(&app.ext.ai).embedding_provider().is_some()
}

/// Starts a full index run (every note; removed notes are dropped from the index).
pub fn indexer_start_full(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    runner::start_index(app, cx, None);
}

/// Periodic work: starts an index run for the notes whose delay has passed.
fn tick(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if !index_wanted(app) {
        return;
    }
    if app.ext.ai.index.running.is_some() {
        return;
    }
    let due = app.ext.ai.index.queue.take_due(Instant::now());
    if !due.is_empty() {
        runner::start_index(app, cx, Some(due));
    }
    cx.notify();
}

/// Keyboard hook. Ctrl+Shift+A toggles the side panel, Ctrl+J opens the selection actions, and
/// Escape closes whatever AI overlay is open.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    let key = ev.keystroke.key.as_str();
    if key == "escape" {
        if app.ext.ai.modal != Modal::None {
            app.ext.ai.modal = Modal::None;
            cx.notify();
            return true;
        }
        if app.ext.ai.menu.is_some() || app.ext.ai.card.is_some() {
            selection::close_overlays(app);
            cx.notify();
            return true;
        }
        return false;
    }
    // Ctrl+Shift+A and Ctrl+J are registry commands (ui/keymap.rs), run by main.rs.
    false
}

/// Module events. Saved notes are queued for re-embedding; a theme change restyles the fields.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    match ev {
        ExtEvent::NoteSaved(path) => {
            if index_wanted(app) {
                app.ext.ai.index.queue.touch(path, Instant::now());
            }
        }
        ExtEvent::ThemeChanged => {
            let theme = app.theme;
            for input in app.ext.ai.inputs.all() {
                input.update(cx, |i, cx| i.set_theme(theme, cx));
            }
        }
        _ => {}
    }
}

/// Runs a command from the palette or a shortcut.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::TogglePanel => app.ext.ai.panel_open = !app.ext.ai.panel_open,
        Command::AskVault => {
            app.ext.ai.panel_open = true;
            app.ext.ai.chips.vault = true;
        }
        Command::RewriteSelection => selection::start_rewrite(app, cx),
        Command::SummarizeNote => notes::ask_summarize(app, cx),
        Command::SuggestTitle => notes::suggest_title(app, cx),
        Command::SuggestTags => notes::suggest_tags(app, cx),
        Command::ActionMenu => selection::open_menu(app, cx),
    }
    cx.notify();
}

pub fn palette_commands() -> Vec<PaletteCommand> {
    let cmd = |c: Command| AppCommand::Ext(ExtCommand::Ai(c));
    vec![
        PaletteCommand {
            label: "AI：打开或关闭助手",
            icon: "bulb",
            shortcut: "Ctrl+Shift+A",
            command: cmd(Command::TogglePanel),
        },
        PaletteCommand {
            label: "AI：选区操作…",
            icon: "pencil",
            shortcut: "Ctrl+J",
            command: cmd(Command::ActionMenu),
        },
        PaletteCommand {
            label: "AI：改写选区",
            icon: "pencil",
            shortcut: "",
            command: cmd(Command::RewriteSelection),
        },
        PaletteCommand {
            label: "AI：总结当前笔记",
            icon: "file-text",
            shortcut: "",
            command: cmd(Command::SummarizeNote),
        },
        PaletteCommand {
            label: "AI：建议标题",
            icon: "hash",
            shortcut: "",
            command: cmd(Command::SuggestTitle),
        },
        PaletteCommand {
            label: "AI：建议标签",
            icon: "tag",
            shortcut: "",
            command: cmd(Command::SuggestTags),
        },
        PaletteCommand {
            label: "AI：询问整个笔记库",
            icon: "search",
            shortcut: "",
            command: cmd(Command::AskVault),
        },
    ]
}

/// Status bar: the `AI` button that opens the panel, and the index progress while it runs.
pub fn statusbar_items(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let t = app.theme;
    let on = app.ext.ai.panel_open;
    let enabled = app.ext.ai.config.enabled;
    let color = if on { t.ac } else { t.tx3 };
    let mut out: Vec<AnyElement> = vec![
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(6.0))
            .h(px(20.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .text_color(color)
            .child(icon("bulb", 13.0, color))
            .child(if enabled { "AI" } else { "AI 已关闭" })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
                    this.ext.ai.panel_open = !this.ext.ai.panel_open;
                    cx.notify();
                }),
            )
            .into_any_element(),
    ];
    if app.ext.ai.index.running.is_some() {
        out.push(
            div()
                .font_family(UI_FONT)
                .text_color(t.tx2)
                .child(indexer::progress_label(
                    app.ext.ai.index.done,
                    app.ext.ai.index.total,
                ))
                .into_any_element(),
        );
    }
    out
}

/// Modal dialogs block the rest of the window, like the palette and the settings dialog.
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.ai.modal != Modal::None
}

/// Overlays: side panel, selection menu, result card, dialog.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    // The settings dialog is drawn before these overlays, so they would cover it: hide them then.
    let settings_open = app.settings.open;
    if app.ext.ai.panel_open && !settings_open {
        out.push(panel::render(app, cx));
    }
    if app.ext.ai.menu.is_some() && !settings_open {
        out.extend(selection::render_menu(app, cx));
    }
    if app.ext.ai.card.is_some() && !settings_open {
        out.push(selection::render_card(app, cx));
    }
    if app.ext.ai.modal != Modal::None {
        out.push(modal::render(app, cx));
    }
    out
}

/// The AI category of the settings dialog.
pub fn render_settings_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    settings::render(app, cx)
}

/// Reads the text of an input entity.
pub fn input_text(input: &Option<Entity<LineInput>>, cx: &gpui::App) -> String {
    input
        .as_ref()
        .map(|i| i.read(cx).text())
        .unwrap_or_default()
}

/// Clears an input entity.
pub fn input_clear(input: &Option<Entity<LineInput>>, cx: &mut Context<RustdianApp>) {
    if let Some(i) = input {
        i.update(cx, |i, cx| i.set_text("", cx));
    }
}

/// The active note's editor, with its path.
pub fn active_editor(app: &RustdianApp) -> Option<(String, Entity<TextEditor>)> {
    let path = app.active_path()?.to_string();
    let editor = app.editor_for(&path)?;
    Some((path, editor))
}

/// Loads the `[ai]` section of the vault's configuration file. A broken file is an error; the
/// settings page shows it and nothing is overwritten.
pub fn load_config(root: &std::path::Path) -> Result<AiConfig, String> {
    let loaded: jiyue_core::config::Loaded<AiConfig> = jiyue_core::config::load_section(
        &jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE),
        "ai",
    );
    match loaded.error {
        Some(issue) => Err(format!("AI 设置有错误：{issue}")),
        None => Ok(loaded.value),
    }
}

/// Validates and writes the `[ai]` section, keeping the rest of the vault's configuration file.
pub fn save_config(root: &std::path::Path, cfg: &AiConfig) -> Result<(), String> {
    cfg.validate().map_err(|e| e.to_string())?;
    jiyue_core::config::save_section(
        &jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE),
        "ai",
        cfg,
        jiyue_core::config::templates::VAULT_CONFIG,
        &["ai.providers"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_is_extracted_without_path_or_credentials() {
        assert_eq!(
            host_of("https://api.example.com/v1/chat"),
            "api.example.com"
        );
        assert_eq!(
            host_of("http://user:pw@127.0.0.1:8080/v1"),
            "127.0.0.1:8080"
        );
        assert_eq!(host_of(""), "未知服务器");
    }

    #[test]
    fn effective_config_adds_the_test_provider_only_when_set() {
        let mut state = State::default();
        let plain = effective_config(&state);
        assert!(plain.providers.is_empty());
        assert!(plain.chat_provider().is_none());

        state.test = Some(worker::TestEnv {
            base: "http://127.0.0.1:9/v1".into(),
            key: "k".into(),
            model: "m".into(),
            embed: Some("e".into()),
        });
        let with_env = effective_config(&state);
        assert_eq!(with_env.providers.len(), 1);
        assert_eq!(with_env.active_chat.as_deref(), Some(worker::TEST_ID));
        assert_eq!(with_env.active_embedding.as_deref(), Some(worker::TEST_ID));
        // The saved file is untouched.
        assert!(state.config.providers.is_empty());
        assert!(state.config.active_chat.is_none());
    }

    #[test]
    fn frontmatter_tags_count_for_exclusion() {
        let text = "---\ntags: [secret]\n---\n正文";
        let cfg = AiConfig {
            excluded_tags: vec!["secret".into()],
            ..AiConfig::default()
        };
        // The vault does not index frontmatter tags, so the vault's list is empty here.
        assert!(!cfg.is_note_excluded("x.md", &[]));
        assert!(cfg.is_note_excluded("x.md", &exclusion_tags(text, &[])));
        assert_eq!(exclusion_tags(text, &["secret".to_string()]).len(), 1);
    }

    #[test]
    fn ai_is_off_by_default() {
        let state = State::default();
        assert!(!state.config.enabled);
        assert!(effective_config(&state).chat_provider().is_none());
        assert!(!state.chips.vault);
        assert!(state.chips.note);
    }
}
