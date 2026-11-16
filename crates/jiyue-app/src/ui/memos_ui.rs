//! Local-first Capture & Todo Dashboard UI module, with optional Memos remote enhancement.
//!
//! Works out of the box on plain Markdown files in the vault (stream file, daily note sections,
//! or standalone cards), while optionally syncing with a self-hosted Memos server when enabled.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, Context, Div, Entity, FontWeight, IntoElement, MouseButton, SharedString, div,
    prelude::*, px,
};
use jiyue_core::config::credentials::{self, MEMOS_TABLE};
use jiyue_core::daily::{self, Date};
use jiyue_core::fsops;
use jiyue_core::inbox::{
    self, CaptureMode, InboxSettings, LocalCaptureItem, LocalTodo, TodoGroup, TodoScope,
};
use jiyue_export::TemplateRules;
use jiyue_memos::{Memo, MemosClient, MemosConfig};

use crate::RustdianApp;
use crate::theme::{RADIUS_M, RADIUS_S};
use crate::ui::ext::{ExtEvent, ToastKind};
use crate::ui::files_ui;
use crate::ui::icons::icon;
use crate::ui::widgets::controls::{
    BadgeKind, ButtonKind, IconBtnSize, badge, button, choice_chip, empty_state, icon_button_sized,
    icon_toggle_button, section_title, segmented_full, setting_card, toggle,
};
use crate::ui::widgets::form_row;
use crate::ui::widgets::line_input::{LineInput, LineInputEvent};

/// Which tab is active in the Capture & Todos sidebar panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Todos,
    All,
}

/// State for the local Capture & Todos dashboard and optional Memos sync.
pub struct State {
    /// Whether the section is open.
    pub open: bool,
    /// Currently selected tab (`Todos` or `All` captures).
    pub tab: Tab,
    /// Whether completed local/remote todos are shown in the Todos tab.
    pub show_completed_todos: bool,
    /// Cached memos fetched from optional remote Memos server.
    pub memos: Vec<Memo>,
    /// Currently selected `#tag` filter, if any.
    pub selected_tag: Option<String>,
    /// Whether fast input submits as `- [ ] ` todo (true) or timestamped note (false).
    pub as_todo: bool,
    /// Whether a remote Memos network sync is in flight.
    pub loading: bool,
    /// Last remote sync error message, if any.
    pub error: Option<String>,
    /// Fast input editor in sidebar.
    pub input: Option<Entity<LineInput>>,
    /// Subscription to fast input events.
    pub _input_sub: Option<gpui::Subscription>,
    /// Last successful remote Memos sync timestamp.
    pub last_sync: Option<Instant>,
    /// Settings page: server URL input.
    pub settings_url_input: Option<Entity<LineInput>>,
    /// Settings page: token input.
    pub settings_token_input: Option<Entity<LineInput>>,
    /// Settings page: custom stream filename input.
    pub stream_file_input: Option<Entity<LineInput>>,
    /// Subscription for stream filename input.
    pub _stream_file_sub: Option<gpui::Subscription>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: true,
            tab: Tab::Todos,
            show_completed_todos: false,
            memos: Vec::new(),
            selected_tag: None,
            as_todo: false,
            loading: false,
            error: None,
            input: None,
            _input_sub: None,
            last_sync: None,
            settings_url_input: None,
            settings_token_input: None,
            stream_file_input: None,
            _stream_file_sub: None,
        }
    }
}

/// Initializes the Capture & Todos sidebar module.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let t = app.theme;
    let input = cx.new(|cx| {
        let mut inp = LineInput::new(cx, crate::t!("memos-placeholder"));
        inp.set_theme(t, cx);
        inp.set_borderless(true, cx);
        inp
    });
    let sub = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            if let LineInputEvent::Submit(_) = ev {
                submit_new_memo(this, cx);
            }
        },
    );
    app.ext.memos._input_sub = Some(sub);
    app.ext.memos.input = Some(input);

    let cfg = load_config(&app.vault.root_dir);
    let token = load_token().unwrap_or_default();
    let url_input = cx.new(|cx| {
        let mut inp = LineInput::new(cx, "http://localhost:5230");
        inp.set_theme(t, cx);
        inp.set_text(&cfg.base_url, cx);
        inp
    });
    let token_input = cx.new(|cx| {
        let mut inp = LineInput::new(cx, "Access Token");
        inp.set_theme(t, cx);
        inp.set_password(true, cx);
        inp.set_text(&token, cx);
        inp
    });
    app.ext.memos.settings_url_input = Some(url_input);
    app.ext.memos.settings_token_input = Some(token_input);

    let inbox_cfg = InboxSettings::load(&app.vault.root_dir);
    let stream_input = cx.new(|cx| {
        let mut inp = LineInput::new(cx, inbox::DEFAULT_STREAM_FILE);
        inp.set_theme(t, cx);
        inp.set_text(&inbox_cfg.stream_file, cx);
        inp
    });
    let stream_sub = cx.subscribe(
        &stream_input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| match ev {
            LineInputEvent::Changed(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    let root = this.vault.root_dir.clone();
                    let mut cfg = InboxSettings::load(&root);
                    cfg.stream_file = trimmed.to_string();
                    let _ = cfg.save(&root);
                    cx.notify();
                }
            }
            LineInputEvent::Submit(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    let root = this.vault.root_dir.clone();
                    let mut cfg = InboxSettings::load(&root);
                    cfg.stream_file = trimmed.to_string();
                    let _ = cfg.save(&root);
                    this.toast(ToastKind::Success, format!("流文件命名已设为：{trimmed}"));
                    cx.notify();
                }
            }
            LineInputEvent::Cancel => {}
        },
    );
    app.ext.memos._stream_file_sub = Some(stream_sub);
    app.ext.memos.stream_file_input = Some(stream_input);

    // Optional remote Memos initial sync only when explicitly enabled.
    if cfg.enabled && !cfg.base_url.trim().is_empty() {
        refresh_silent(app, cx);
    }

    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(180))
                .await;
            let ok = this.update(cx, |app, cx| {
                let cfg = load_config(&app.vault.root_dir);
                if cfg.enabled && !cfg.base_url.trim().is_empty() {
                    refresh_silent(app, cx);
                }
            });
            if ok.is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Handles broadcast app events (keeps embedded `LineInput`s in sync with the active theme).
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    if matches!(ev, ExtEvent::ThemeChanged) {
        let t = app.theme;
        if let Some(inp) = &app.ext.memos.input {
            inp.update(cx, |i, cx| i.set_theme(t, cx));
        }
        if let Some(inp) = &app.ext.memos.settings_url_input {
            inp.update(cx, |i, cx| i.set_theme(t, cx));
        }
        if let Some(inp) = &app.ext.memos.settings_token_input {
            inp.update(cx, |i, cx| i.set_theme(t, cx));
        }
        if let Some(inp) = &app.ext.memos.stream_file_input {
            inp.update(cx, |i, cx| i.set_theme(t, cx));
        }
    }
}

/// Loads the `[memos]` section from `<vault>/.jiyue/config.toml`.
pub fn load_config(root: &Path) -> MemosConfig {
    let loaded: jiyue_core::config::Loaded<MemosConfig> = jiyue_core::config::load_section(
        &jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE),
        "memos",
    );
    loaded.value
}

/// Saves the `[memos]` section to `<vault>/.jiyue/config.toml`.
pub fn save_config(root: &Path, cfg: &MemosConfig) -> Result<(), String> {
    jiyue_core::config::save_section(
        &jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE),
        "memos",
        cfg,
        jiyue_core::config::templates::VAULT_CONFIG,
        &[],
    )
}

/// Reads the Memos token from `credentials.toml`.
pub fn load_token() -> Option<String> {
    let creds = credentials::load();
    creds.value.memos.get("default").map(|c| c.token.clone())
}

/// Writes the Memos token to `credentials.toml`.
pub fn save_token(token: &str) -> Result<(), String> {
    let path = credentials::path();
    credentials::write_field(&path, MEMOS_TABLE, "default", "token", Some(token)).map(|_| ())
}

/// Creates a MemosClient from vault config and credentials when remote enhancement is enabled.
pub fn create_client(vault_root: &Path) -> Option<MemosClient> {
    let cfg = load_config(vault_root);
    if !cfg.enabled || cfg.base_url.trim().is_empty() {
        return None;
    }
    let token = load_token();
    MemosClient::new(&cfg.base_url, token.as_deref()).ok()
}

/// Writes `new_text` to `rel_path` cleanly, keeping open editors, undo history, watcher
/// `last_written`, and vault note state in sync without false conflict prompts.
fn write_vault_markdown_synced(
    app: &mut RustdianApp,
    rel_path: &str,
    new_text: &str,
    cx: &mut Context<RustdianApp>,
) -> Result<(), String> {
    let rel = rel_path.replace('\\', "/");
    fsops::safe_rel_path(&rel).map_err(|e| format!("路径无效：{e}"))?;

    if let Some(editor) = app.editor_for(&rel) {
        editor.update(cx, |ed, cx| {
            ed.set_text_undoable(new_text, cx);
        });
        if app.note_index(&rel).is_some() {
            if !app.save_path(&rel, cx) {
                return Err(format!("保存笔记失败：{rel}"));
            }
            return Ok(());
        }
    }

    let root = app.vault.root_dir.clone();
    let abs = root.join(&rel);
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    fsops::write_atomic(&abs, new_text.as_bytes()).map_err(|e| format!("写入文件失败：{e}"))?;
    app.ext
        .files
        .last_written
        .insert(rel.clone(), new_text.to_string());

    if let Some(idx) = app
        .vault
        .notes
        .iter()
        .position(|n| n.path.eq_ignore_ascii_case(&rel))
    {
        app.vault.notes[idx].set_content(new_text);
        app.vault.bump_revision();
        app.vault.rebuild_graph();
    } else {
        files_ui::actions::reload(app, &[], cx);
    }
    Ok(())
}

/// Reads the freshest text for `rel_path` (open editor text first, then vault cache, then disk).
fn read_freshest_note_text(app: &RustdianApp, rel_path: &str, cx: &Context<RustdianApp>) -> String {
    let rel = rel_path.replace('\\', "/");
    if let Some(ed) = app.editor_for(&rel) {
        return ed.read(cx).text();
    }
    if let Some(note) = app
        .vault
        .notes
        .iter()
        .find(|n| n.path.eq_ignore_ascii_case(&rel))
    {
        return note.content().to_string();
    }
    fs::read_to_string(app.vault.root_dir.join(&rel)).unwrap_or_default()
}

/// Returns today's daily note vault-relative path (`Daily/YYYY-MM-DD.md` etc.) with `/` separators.
fn today_daily_rel_path(app: &RustdianApp, date: Date) -> String {
    let daily_cfg = app.vault.daily_settings();
    let title = daily::format_date(date, &daily_cfg.date_format);
    let folder = daily_cfg.folder.trim().replace('\\', "/");
    let folder = folder.trim_matches('/');
    if folder.is_empty() {
        format!("{title}.md")
    } else {
        format!("{folder}/{title}.md")
    }
}

/// Renders today's daily note initial text if it does not yet exist, without switching open tabs.
fn initial_daily_content(app: &RustdianApp, date: Date) -> String {
    let daily_cfg = app.vault.daily_settings();
    let title = daily::format_date(date, &daily_cfg.date_format);
    let tpl_path = daily_cfg.template_path.trim().replace('\\', "/");
    let tpl = if !tpl_path.is_empty() {
        jiyue_export::templates::read_template(&app.vault.root_dir, &tpl_path)
    } else {
        None
    }
    .unwrap_or_else(|| daily::DEFAULT_TEMPLATE.to_string());
    daily::render_template(
        &tpl,
        date,
        &title,
        &daily_cfg.date_format,
        &daily::local_hhmm(),
    )
}

/// Generates a unique card note path inside `inbox.folder` for `CaptureMode::CardNote`.
fn allocate_card_rel_path(root: &Path, folder: &str, raw_text: &str) -> String {
    let clean_dir = folder.trim().replace('\\', "/");
    let clean_dir = clean_dir.trim_matches('/');
    let prefix = if clean_dir.is_empty() {
        String::new()
    } else {
        format!("{clean_dir}/")
    };

    let first_line = raw_text
        .trim()
        .lines()
        .next()
        .unwrap_or("")
        .strip_prefix("- [ ] ")
        .or_else(|| {
            raw_text
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .strip_prefix("- ")
        })
        .unwrap_or(raw_text.trim().lines().next().unwrap_or(""));
    let snippet: String = first_line
        .chars()
        .take(24)
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|#[]".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let clean_snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    let stamp = {
        let now = jiyue_history::system_now_ms();
        let m = crate::ui::template_ui::moment(now);
        match m {
            Some(m) => format!("{:04}{:02}{:02}-{:02}{:02}", m.y, m.m, m.d, m.h, m.mi),
            None => Date::today().to_days().to_string(),
        }
    };
    let base_stem = if clean_snippet.is_empty() {
        format!("随手记 {stamp}")
    } else {
        format!("{stamp} {clean_snippet}")
    };
    for n in 0..1000u32 {
        let stem = if n == 0 {
            base_stem.clone()
        } else {
            format!("{base_stem} {n}")
        };
        let rel = format!("{prefix}{stem}.md");
        if !root.join(&rel).exists() {
            return rel;
        }
    }
    format!("{prefix}随手记-{}.md", jiyue_history::system_now_ms())
}

/// Opens `note_path` in the workspace and moves the caret to `line_index` if provided.
fn open_local_note_at(
    app: &mut RustdianApp,
    note_path: &str,
    line_index: Option<usize>,
    cx: &mut Context<RustdianApp>,
) {
    let Some(idx) = app
        .vault
        .notes
        .iter()
        .position(|n| n.path.eq_ignore_ascii_case(note_path))
    else {
        files_ui::actions::reload(app, &[], cx);
        let Some(idx2) = app
            .vault
            .notes
            .iter()
            .position(|n| n.path.eq_ignore_ascii_case(note_path))
        else {
            app.toast(ToastKind::Warn, format!("找不到笔记：{note_path}"));
            return;
        };
        app.open_note_by_index(idx2);
        if let Some(line) = line_index {
            app.request_caret(line, 0);
        }
        cx.notify();
        return;
    };
    app.open_note_by_index(idx);
    if let Some(line) = line_index {
        app.request_caret(line, 0);
    }
    cx.notify();
}

/// Opens the configured local capture stream file (creating it cleanly if needed).
pub fn open_stream_note(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let root = app.vault.root_dir.clone();
    let inbox = InboxSettings::load(&root);
    let today = Date::today();
    let rel = inbox.stream_rel_path(today);
    if !root.join(&rel).is_file() {
        let stem = inbox.stream_title(today);
        let date_hdr = format!(
            "## {} {}",
            daily::format_date(today, "YYYY-MM-DD"),
            daily::weekday_name_zh(today)
        );
        let body = format!("# {stem}\n\n{date_hdr}\n");
        if let Err(err) = write_vault_markdown_synced(app, &rel, &body, cx) {
            app.toast(ToastKind::Error, err);
            return;
        }
    }
    open_local_note_at(app, &rel, None, cx);
}

/// Triggers a silent refresh of remote Memos without showing any error toast.
pub fn refresh_silent(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let vault_root = app.vault.root_dir.clone();
    let Some(client) = create_client(&vault_root) else {
        return;
    };

    cx.spawn(async move |this, cx| {
        let result = cx
            .background_executor()
            .spawn(async move { client.list_memos_blocking(50, None) })
            .await;
        let _ = this.update(cx, |app, cx| {
            if let Ok((memos, _)) = result {
                app.ext.memos.memos = memos;
                app.ext.memos.last_sync = Some(Instant::now());
                app.ext.memos.error = None;
                cx.notify();
            }
        });
    })
    .detach();
}

/// Refreshes local view and, when remote Memos enhancement is enabled, fetches remote Memos.
pub fn refresh(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let vault_root = app.vault.root_dir.clone();
    let Some(client) = create_client(&vault_root) else {
        cx.notify();
        return;
    };

    app.ext.memos.loading = true;
    app.ext.memos.error = None;
    cx.notify();

    cx.spawn(async move |this, cx| {
        let result = cx
            .background_executor()
            .spawn(async move { client.list_memos_blocking(50, None) })
            .await;
        let _ = this.update(cx, |app, cx| {
            app.ext.memos.loading = false;
            match result {
                Ok((memos, _)) => {
                    app.ext.memos.memos = memos;
                    app.ext.memos.last_sync = Some(Instant::now());
                    app.ext.memos.error = None;
                }
                Err(e) => {
                    let err = e.to_string();
                    app.ext.memos.error = Some(err.clone());
                    app.toast(
                        ToastKind::Error,
                        format!("{}: {}", crate::t!("memos-status-error"), err),
                    );
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// Deletes a remote memo by resource name with optimistic removal from local cache.
pub fn delete_memo(app: &mut RustdianApp, memo_name: &str, cx: &mut Context<RustdianApp>) {
    let vault_root = app.vault.root_dir.clone();
    let Some(client) = create_client(&vault_root) else {
        return;
    };

    let name = memo_name.to_string();
    let original_memos = app.ext.memos.memos.clone();
    app.ext.memos.memos.retain(|m| m.name != name);
    cx.notify();

    cx.spawn(async move |this, cx| {
        let result = cx
            .background_executor()
            .spawn(async move { client.delete_memo_blocking(&name) })
            .await;
        let _ = this.update(cx, |app, cx| {
            match result {
                Ok(()) => app.toast(ToastKind::Info, "已删除远端 Memos 条目"),
                Err(e) => {
                    app.ext.memos.memos = original_memos;
                    app.toast(ToastKind::Error, format!("删除远端 Memos 失败: {e}"));
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// Captures a new thought or todo from sidebar fast input directly into the local Markdown vault,
/// and optionally mirrors it to remote Memos if configured.
pub fn submit_new_memo(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let raw_text = if let Some(i) = &app.ext.memos.input {
        i.read(cx).text().trim().to_string()
    } else {
        return;
    };
    if raw_text.is_empty() {
        return;
    }

    let as_todo = app.ext.memos.as_todo
        || raw_text.starts_with("- [ ] ")
        || raw_text.starts_with("- [x] ")
        || raw_text.starts_with("- [X] ");
    let root = app.vault.root_dir.clone();
    let inbox = InboxSettings::load(&root);
    let today = Date::today();
    let hhmm = daily::local_hhmm();

    let save_res = match inbox.capture_mode {
        CaptureMode::StreamFile => {
            let rel = inbox.stream_rel_path(today);
            let existing = read_freshest_note_text(app, &rel, cx);
            let title = inbox.stream_title(today);
            let updated = inbox::append_to_stream_markdown(
                &existing, &title, today, &hhmm, &raw_text, as_todo,
            );
            write_vault_markdown_synced(app, &rel, &updated, cx).map(|()| rel)
        }
        CaptureMode::DailyNote => {
            let rel = today_daily_rel_path(app, today);
            let existing = {
                let cur = read_freshest_note_text(app, &rel, cx);
                if cur.trim().is_empty() && !root.join(&rel).is_file() {
                    initial_daily_content(app, today)
                } else {
                    cur
                }
            };
            let updated = inbox::append_to_daily_markdown(&existing, &hhmm, &raw_text, as_todo);
            write_vault_markdown_synced(app, &rel, &updated, cx).map(|()| rel)
        }
        CaptureMode::CardNote => {
            let rel = allocate_card_rel_path(&root, &inbox.folder, &raw_text);
            let stem = Path::new(&rel)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("随手记");
            let entry =
                if as_todo && !raw_text.starts_with("- [ ] ") && !raw_text.starts_with("- [x] ") {
                    format!("- [ ] {raw_text}")
                } else {
                    raw_text.clone()
                };
            let tpl = {
                let custom = inbox.inbox_template_path.trim();
                if !custom.is_empty() {
                    jiyue_export::templates::read_template(&root, custom)
                } else {
                    None
                }
            }
            .unwrap_or_else(|| inbox::DEFAULT_INBOX_CARD_TEMPLATE.to_string());
            let body = if tpl.contains("{{cursor}}") {
                let rendered =
                    crate::ui::template_ui::render_template_for_note(app, &tpl, stem, &rel, "", cx)
                        .map(|r| r.text)
                        .unwrap_or_else(|| format!("# {stem}\n\n{entry}\n"));
                format!("{rendered}{entry}\n")
            } else {
                let rendered =
                    crate::ui::template_ui::render_template_for_note(app, &tpl, stem, &rel, "", cx)
                        .map(|r| r.text)
                        .unwrap_or_else(|| format!("# {stem}\n\n"));
                format!("{}\n\n{entry}\n", rendered.trim_end())
            };
            write_vault_markdown_synced(app, &rel, &body, cx).map(|()| rel)
        }
    };

    match save_res {
        Ok(rel) => {
            if let Some(i) = &app.ext.memos.input {
                i.update(cx, |i, cx| i.set_text("", cx));
            }
            let tip = if as_todo {
                format!("已添加待办到 {rel}")
            } else {
                format!("已记入 {rel}")
            };
            app.toast(ToastKind::Success, tip);
        }
        Err(err) => {
            app.toast(ToastKind::Error, err);
            cx.notify();
            return;
        }
    }

    // Optional enhancement: sync to Memos server in the background without blocking local flow.
    if let Some(client) = create_client(&root) {
        let remote_text =
            if as_todo && !raw_text.starts_with("- [ ] ") && !raw_text.starts_with("- [x] ") {
                format!("- [ ] {raw_text}")
            } else {
                raw_text
            };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.create_memo_blocking(&remote_text, None) })
                .await;
            let _ = this.update(cx, |app, cx| {
                if let Ok(memo) = result {
                    app.ext.memos.memos.insert(0, memo);
                } else {
                    app.toast(ToastKind::Warn, "本地已保存，但远端 Memos 同步未成功");
                }
                cx.notify();
            });
        })
        .detach();
    }

    cx.notify();
}

/// Toggles a local todo checkbox inside `note_path` at `line_index`.
pub fn toggle_local_todo(
    app: &mut RustdianApp,
    note_path: &str,
    line_index: usize,
    cx: &mut Context<RustdianApp>,
) {
    let current = read_freshest_note_text(app, note_path, cx);
    let Some(updated) = inbox::toggle_todo_line(&current, line_index) else {
        app.toast(ToastKind::Warn, "未找到对应的待办行，请刷新后重试");
        return;
    };
    if let Err(err) = write_vault_markdown_synced(app, note_path, &updated, cx) {
        app.toast(ToastKind::Error, err);
    }
    cx.notify();
}

/// Deletes a local todo or capture entry from its Markdown file, or moves a standalone card note
/// to trash.
pub fn delete_local_entry(
    app: &mut RustdianApp,
    note_path: &str,
    line_index: Option<usize>,
    cx: &mut Context<RustdianApp>,
) {
    if let Some(line) = line_index {
        let current = read_freshest_note_text(app, note_path, cx);
        let Some(updated) = inbox::delete_list_entry(&current, line) else {
            app.toast(ToastKind::Warn, "无法定位要删除的条目");
            return;
        };
        if let Err(err) = write_vault_markdown_synced(app, note_path, &updated, cx) {
            app.toast(ToastKind::Error, err);
        } else {
            app.toast(ToastKind::Info, "已移除条目");
        }
        cx.notify();
    } else if let Err(err) = files_ui::actions::delete_item(app, note_path, false, cx) {
        app.toast(ToastKind::Error, err);
    }
}

/// Toggles a todo checkbox in a remote Memos entry.
pub fn toggle_todo(
    app: &mut RustdianApp,
    memo_id: &str,
    line_index: usize,
    cx: &mut Context<RustdianApp>,
) {
    let vault_root = app.vault.root_dir.clone();
    let Some(client) = create_client(&vault_root) else {
        return;
    };

    let memo_pos = app.ext.memos.memos.iter().position(|m| m.id == memo_id);
    let (memo_name, new_content) = if let Some(pos) = memo_pos {
        let m = &app.ext.memos.memos[pos];
        if let Some(new_c) = m.toggle_todo_content(line_index) {
            let name = m.name.clone();
            app.ext.memos.memos[pos] = Memo::from_raw(
                m.name.clone(),
                new_c.clone(),
                m.create_time.clone(),
                m.update_time.clone(),
                Some(m.visibility.clone()),
                m.pinned,
            );
            cx.notify();
            (name, new_c)
        } else {
            return;
        }
    } else {
        return;
    };

    cx.spawn(async move |this, cx| {
        let result = cx
            .background_executor()
            .spawn(async move { client.update_memo_content_blocking(&memo_name, &new_content) })
            .await;
        let _ = this.update(cx, |app, cx| {
            if let Err(e) = result {
                app.toast(ToastKind::Error, format!("更新远端待办失败: {e}"));
                refresh(app, cx);
            }
        });
    })
    .detach();
}

/// Inserts a snippet into the active editor (or informs the user if none is open).
pub fn insert_memo_to_active(
    app: &mut RustdianApp,
    memo_content: &str,
    cx: &mut Context<RustdianApp>,
) {
    let Some(path) = app.active_path().map(str::to_string) else {
        app.toast(ToastKind::Warn, "请先打开一篇笔记再插入");
        return;
    };

    app.ensure_editor(&path, cx);
    if let Some(editor) = app.editor_for(&path) {
        editor.update(cx, |ed, cx| {
            let current = ed.text();
            let trimmed = current.trim_end();
            let new_text = if trimmed.is_empty() {
                format!("{memo_content}\n")
            } else {
                format!("{trimmed}\n\n{memo_content}\n")
            };
            ed.set_text_undoable(&new_text, cx);
        });
        app.save_path(&path, cx);
        app.toast(ToastKind::Success, crate::t!("memos-toast-inserted"));
    }
}

/// Appends remote Memos / local captures to today's daily note (compatibility entry point).
pub fn append_today_memos_to_daily(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.open_today();
    let Some(path) = app.active_path().map(str::to_string) else {
        return;
    };
    if !app.ext.memos.memos.is_empty() {
        let mut lines = vec!["\n\n## Memos 闪念与待办\n".to_string()];
        for memo in &app.ext.memos.memos {
            lines.extend(memo.content.lines().map(str::to_string));
            lines.push(String::new());
        }
        let append_text = lines.join("\n");
        if let Some(editor) = app.editor_for(&path) {
            editor.update(cx, |ed, cx| {
                let current = ed.text();
                ed.set_text_undoable(&format!("{current}{append_text}"), cx);
            });
            app.save_path(&path, cx);
            app.toast(ToastKind::Info, crate::t!("memos-toast-appended"));
        }
    }
    cx.notify();
}

fn collect_note_snapshots(
    app: &RustdianApp,
    cx: &Context<RustdianApp>,
) -> Vec<(String, String, String)> {
    let mut dirty_texts: HashMap<String, String> = HashMap::new();
    for path in app.editors.keys() {
        if let Some(ed) = app.editor_for(path) {
            dirty_texts.insert(path.clone(), ed.read(cx).text());
        }
    }
    app.vault
        .notes
        .iter()
        .map(|n| {
            let content = dirty_texts
                .get(&n.path)
                .cloned()
                .unwrap_or_else(|| n.content().to_string());
            (n.path.clone(), n.title.clone(), content)
        })
        .collect()
}

/// Renders the full-height Capture & Todos sidebar panel.
pub fn render_sidebar_panel(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let root = app.vault.root_dir.clone();
    let memos_cfg = load_config(&root);
    let inbox_cfg = InboxSettings::load(&root);
    let memos_active = memos_cfg.enabled && !memos_cfg.base_url.trim().is_empty();

    let mut top_left = div().flex().items_center().gap(px(4.0)).min_w_0();
    if app.ext.memos.tab == Tab::Todos {
        let core_on = inbox_cfg.todo_scope == TodoScope::CoreFolders;
        let show_done = app.ext.memos.show_completed_todos;
        top_left = top_left
            .child(choice_chip(
                &t,
                if core_on { "核心目录" } else { "全库" },
                true,
                cx.listener(move |this, _, _, cx| {
                    let root = this.vault.root_dir.clone();
                    let mut c = InboxSettings::load(&root);
                    c.todo_scope = if c.todo_scope == TodoScope::CoreFolders {
                        TodoScope::AllNotes
                    } else {
                        TodoScope::CoreFolders
                    };
                    let _ = c.save(&root);
                    cx.notify();
                }),
            ))
            .child(icon_toggle_button(
                &t,
                "check",
                show_done,
                IconBtnSize::Small,
                cx.listener(|this, _, _, cx| {
                    this.ext.memos.show_completed_todos = !this.ext.memos.show_completed_todos;
                    cx.notify();
                }),
            ));
    } else {
        top_left = top_left.child(choice_chip(
            &t,
            "打开随手记",
            false,
            cx.listener(|this, _, _, cx| {
                open_stream_note(this, cx);
            }),
        ));
    }

    let mut top_right = div()
        .flex()
        .items_center()
        .gap(px(2.0))
        .child(icon_button_sized(
            &t,
            "file-plus",
            IconBtnSize::Small,
            cx.listener(|this, _, _, cx| {
                crate::ui::template_ui::run(this, crate::ui::template_ui::Command::NewTodoNote, cx);
            }),
        ))
        .child(icon_button_sized(
            &t,
            "calendar-event",
            IconBtnSize::Small,
            cx.listener(|this, _, _, cx| {
                this.open_today();
                cx.notify();
            }),
        ));
    if memos_active {
        top_right = top_right.child(icon_button_sized(
            &t,
            "refresh",
            IconBtnSize::Small,
            cx.listener(|this, _, _, cx| {
                refresh(this, cx);
            }),
        ));
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .w_full()
        .px(px(8.0))
        .py(px(6.0))
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(4.0))
                .child(top_left)
                .child(top_right),
        )
        .child(render_memos_body(app, true, cx))
}

fn render_memos_body(app: &RustdianApp, full_panel: bool, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let tab = app.ext.memos.tab;
    let root = app.vault.root_dir.clone();
    let inbox_cfg = InboxSettings::load(&root);
    let daily_cfg = app.vault.daily_settings();
    let tpl_configured = TemplateRules::load(&root).templates_folder;
    let tpl_folder =
        crate::ui::template_ui::choose_folder(&tpl_configured, |f| root.join(f).is_dir());
    let today = Date::today();
    let today_rel = today_daily_rel_path(app, today);
    let stream_rel = inbox_cfg.stream_rel_path(today);
    let memos_cfg = load_config(&root);
    let memos_active = memos_cfg.enabled && !memos_cfg.base_url.trim().is_empty();

    let snapshots = collect_note_snapshots(app, cx);
    let snap_refs = snapshots
        .iter()
        .map(|(p, title, c)| (p.as_str(), title.as_str(), c.as_str()));
    let local_todos = inbox::collect_local_todos(
        snap_refs,
        &today_rel,
        &daily_cfg.folder,
        &tpl_folder,
        &inbox_cfg,
    );

    let snap_refs2 = snapshots
        .iter()
        .map(|(p, title, c)| (p.as_str(), title.as_str(), c.as_str()));
    let local_captures =
        inbox::collect_local_captures(snap_refs2, &today_rel, &stream_rel, &tpl_folder, &inbox_cfg);

    let pending_local = local_todos.iter().filter(|x| !x.done).count();
    let pending_remote: usize = if memos_active {
        app.ext
            .memos
            .memos
            .iter()
            .flat_map(|m| m.todos.iter())
            .filter(|t| !t.done)
            .count()
    } else {
        0
    };
    let pending_total = pending_local + pending_remote;

    let mut body = div().flex().flex_col().w_full();
    if full_panel {
        body = body.flex_1().min_h_0();
    }

    // Tab switcher: 待办 (N) | 随手记
    let tab_todos_label = if pending_total > 0 {
        format!("{} ({pending_total})", crate::t!("memos-tab-todos"))
    } else {
        crate::t!("memos-tab-todos")
    };
    let tab_all_label = crate::t!("memos-tab-all");
    let tab_bar = div().py(px(2.0)).child(segmented_full(
        &t,
        &[tab_todos_label.as_str(), tab_all_label.as_str()],
        if tab == Tab::Todos { 0 } else { 1 },
        Rc::new(cx.listener(|this, &idx, _, cx| {
            this.ext.memos.tab = if idx == 0 { Tab::Todos } else { Tab::All };
            cx.notify();
        })),
    ));
    body = body.child(tab_bar);

    // Collect tags across current view sources (local + optional remote)
    let mut tag_set = BTreeSet::new();
    for td in &local_todos {
        for tag in &td.tags {
            tag_set.insert(tag.clone());
        }
    }
    for cap in &local_captures {
        for tag in &cap.tags {
            tag_set.insert(tag.clone());
        }
    }
    if memos_active {
        for memo in &app.ext.memos.memos {
            for tag in &memo.tags {
                tag_set.insert(tag.clone());
            }
        }
    }

    if !tag_set.is_empty() {
        let is_all_sel = app.ext.memos.selected_tag.is_none();
        let mut tag_bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(5.0))
            .py(px(4.0))
            .child(choice_chip(
                &t,
                &crate::t!("memos-tab-all"),
                is_all_sel,
                cx.listener(|this, _, _, cx| {
                    this.ext.memos.selected_tag = None;
                    cx.notify();
                }),
            ));

        for tag in tag_set.into_iter().take(12) {
            let is_sel = app.ext.memos.selected_tag.as_deref() == Some(&tag);
            let t_clone = tag.clone();
            tag_bar = tag_bar.child(choice_chip(
                &t,
                &format!("#{tag}"),
                is_sel,
                cx.listener(move |this, _, _, cx| {
                    if this.ext.memos.selected_tag.as_deref() == Some(&t_clone) {
                        this.ext.memos.selected_tag = None;
                    } else {
                        this.ext.memos.selected_tag = Some(t_clone.clone());
                    }
                    cx.notify();
                }),
            ));
        }
        body = body.child(tag_bar);
    }

    // Fast Input Box with Todo-mode Toggle
    let as_todo = app.ext.memos.as_todo;
    let placeholder_text = match (as_todo, inbox_cfg.capture_mode) {
        (true, CaptureMode::DailyNote) => "输入待办，回车记入今日日记…",
        (true, _) => "输入待办，回车写入本地待办…",
        (false, CaptureMode::DailyNote) => "随手记一句，回车写入今日日记…",
        (false, CaptureMode::CardNote) => "随手记灵感，回车生成闪念卡片…",
        (false, CaptureMode::StreamFile) => "随手记灵感，回车写入本地流文件…",
    };
    let input_el = if let Some(inp) = &app.ext.memos.input {
        inp.update(cx, |i, cx| {
            i.set_theme(t, cx);
            i.set_borderless(true, cx);
            i.set_placeholder(placeholder_text, cx);
        });
        div()
            .flex()
            .items_center()
            .flex_none()
            .h(px(32.0))
            .pl(px(4.0))
            .pr(px(3.0))
            .my(px(4.0))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border_default)
            .rounded(px(RADIUS_M))
            .overflow_hidden()
            .hover(|d| d.border_color(t.border_focus))
            .child(
                div()
                    .size(px(24.0))
                    .rounded(px(RADIUS_S))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(as_todo, |d| d.bg(t.ac_soft))
                    .when(!as_todo, |d| d.hover(|h| h.bg(t.hover)))
                    .child(icon(
                        if as_todo { "checkbox" } else { "pencil" },
                        14.0,
                        if as_todo { t.ac } else { t.tx3 },
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.ext.memos.as_todo = !this.ext.memos.as_todo;
                            cx.notify();
                        }),
                    ),
            )
            .child(div().flex_1().min_w_0().h_full().child(inp.clone()))
            .child(
                div()
                    .flex_none()
                    .h(px(24.0))
                    .px(px(10.0))
                    .rounded(px(RADIUS_S))
                    .bg(t.ac)
                    .text_color(t.ac_on)
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|h| h.bg(t.ac_hover))
                    .child("记录")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            submit_new_memo(this, cx);
                        }),
                    ),
            )
    } else {
        div().h(px(0.0))
    };

    body = body.child(input_el);

    let mut card_container = div()
        .id("memos_card_container")
        .flex()
        .flex_col()
        .gap(px(4.0))
        .overflow_y_scroll()
        .px(px(2.0))
        .py(px(4.0));
    if full_panel {
        card_container = card_container.flex_1().min_h_0();
    } else {
        card_container = card_container.max_h(px(260.0));
    }

    let sel_tag = app.ext.memos.selected_tag.clone();
    let show_done = app.ext.memos.show_completed_todos;
    let mut displayed_count = 0usize;

    if tab == Tab::Todos {
        let mut today_rows: Vec<LocalTodo> = Vec::new();
        let mut core_rows: Vec<LocalTodo> = Vec::new();
        let mut other_rows: Vec<LocalTodo> = Vec::new();
        for item in local_todos {
            if !show_done && item.done {
                continue;
            }
            if let Some(ref tag) = sel_tag
                && !item.tags.contains(tag)
            {
                continue;
            }
            match item.group {
                TodoGroup::TodayDaily => today_rows.push(item),
                TodoGroup::CoreFolder => core_rows.push(item),
                TodoGroup::OtherNote => other_rows.push(item),
            }
        }

        let groups = [
            ("今日待办", today_rows),
            ("收集箱与待办目录", core_rows),
            ("其他笔记", other_rows),
        ];
        for (heading, items) in groups {
            if items.is_empty() {
                continue;
            }
            card_container = card_container.child(
                div()
                    .px(px(4.0))
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.tx3)
                    .child(SharedString::from(format!("{heading} ({})", items.len()))),
            );
            for todo in items {
                displayed_count += 1;
                card_container = card_container.child(render_local_todo_row(&t, todo, cx));
            }
        }

        if memos_active {
            let mut remote_rows = Vec::new();
            for memo in &app.ext.memos.memos {
                if let Some(ref tag) = sel_tag
                    && !memo.tags.contains(tag)
                {
                    continue;
                }
                for todo in &memo.todos {
                    if !show_done && todo.done {
                        continue;
                    }
                    remote_rows.push((memo.id.clone(), memo.name.clone(), todo.clone()));
                }
            }
            if !remote_rows.is_empty() {
                card_container = card_container.child(
                    div()
                        .px(px(4.0))
                        .pt(px(6.0))
                        .pb(px(2.0))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.tx3)
                        .child(SharedString::from(format!(
                            "Memos 远端 ({})",
                            remote_rows.len()
                        ))),
                );
                for (mid, mname, todo) in remote_rows {
                    displayed_count += 1;
                    let is_done = todo.done;
                    let l_idx = todo.line_index;
                    let m_del = mname.clone();
                    card_container = card_container.child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap(px(6.0))
                            .p(px(4.0))
                            .rounded(px(RADIUS_S))
                            .hover(|d| d.bg(t.hover))
                            .child(
                                div()
                                    .flex()
                                    .items_start()
                                    .gap(px(6.0))
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .mt(px(2.0))
                                            .cursor_pointer()
                                            .child(icon(
                                                if is_done { "checkbox" } else { "square" },
                                                13.0,
                                                if is_done { t.tx3 } else { t.ac },
                                            ))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _, _, cx| {
                                                    toggle_todo(this, &mid, l_idx, cx);
                                                }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.0))
                                            .text_color(if is_done { t.tx3 } else { t.tx })
                                            .when(is_done, |d| d.line_through())
                                            .child(todo.text.clone()),
                                    ),
                            )
                            .child(icon_button_sized(
                                &t,
                                "trash",
                                IconBtnSize::Xs,
                                cx.listener(move |this, _, _, cx| {
                                    delete_memo(this, &m_del, cx);
                                }),
                            )),
                    );
                }
            }
        }
    } else {
        // Tab::All (随手记时间流：本地笔记卡片 + 可选 Memos 远端)
        for cap in local_captures {
            if let Some(ref tag) = sel_tag
                && !cap.tags.contains(tag)
            {
                continue;
            }
            displayed_count += 1;
            card_container = card_container.child(render_local_capture_card(&t, cap, cx));
        }

        if memos_active {
            for memo in &app.ext.memos.memos {
                if let Some(ref tag) = sel_tag
                    && !memo.tags.contains(tag)
                {
                    continue;
                }
                displayed_count += 1;
                let m_content = memo.content.clone();
                let m_del_name = memo.name.clone();
                let card = div()
                    .flex()
                    .flex_col()
                    .p(px(6.0))
                    .rounded(px(RADIUS_S))
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.border_subtle)
                    .hover(|d| d.border_color(t.border_default))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb(px(3.0))
                            .child(badge(&t, "Memos 远端", BadgeKind::Accent)),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(t.tx)
                            .child(memo.content.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mt(px(4.0))
                            .child(div().flex().items_center().gap(px(4.0)).children(
                                memo.tags.iter().map(|tag| {
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(t.tx3)
                                        .child(format!("#{}", tag))
                                }),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(t.tx3)
                                            .cursor_pointer()
                                            .hover(|d| d.text_color(t.ac))
                                            .child(crate::t!("memos-insert-note"))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _, _, cx| {
                                                    insert_memo_to_active(this, &m_content, cx);
                                                }),
                                            ),
                                    )
                                    .child(icon_button_sized(
                                        &t,
                                        "trash",
                                        IconBtnSize::Xs,
                                        cx.listener(move |this, _, _, cx| {
                                            delete_memo(this, &m_del_name, cx);
                                        }),
                                    )),
                            ),
                    );
                card_container = card_container.child(card);
            }
        }
    }

    if displayed_count == 0 {
        let (title, hint) = if tab == Tab::Todos {
            (
                crate::t!("memos-empty-todos"),
                "在上方勾选方框发送待办，或在笔记中写 - [ ] 任务",
            )
        } else {
            (
                crate::t!("memos-empty-all"),
                "直接输入想法并回车，内容会保存在本地笔记库中",
            )
        };
        card_container = card_container.child(empty_state(
            &t,
            if tab == Tab::Todos {
                "check"
            } else {
                "file-text"
            },
            &title,
            hint,
        ));
    }

    body.child(card_container)
}

fn render_local_todo_row(
    t: &crate::theme::ActiveTheme,
    todo: LocalTodo,
    cx: &mut Context<RustdianApp>,
) -> Div {
    let is_done = todo.done;
    let path_toggle = todo.note_path.clone();
    let path_open = todo.note_path.clone();
    let path_del = todo.note_path.clone();
    let l_idx = todo.line_index;

    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(px(6.0))
        .p(px(5.0))
        .rounded(px(RADIUS_S))
        .hover(|d| d.bg(t.hover))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(6.0))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .mt(px(2.0))
                        .cursor_pointer()
                        .child(icon(
                            if is_done { "checkbox" } else { "square" },
                            13.0,
                            if is_done { t.tx3 } else { t.ac },
                        ))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                toggle_local_todo(this, &path_toggle, l_idx, cx);
                            }),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .cursor_pointer()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                open_local_note_at(this, &path_open, Some(l_idx), cx);
                            }),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(if is_done { t.tx3 } else { t.tx })
                                .when(is_done, |d| d.line_through())
                                .child(todo.text.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(t.tx3)
                                .truncate()
                                .child(SharedString::from(todo.note_title.clone())),
                        ),
                ),
        )
        .child(icon_button_sized(
            t,
            "trash",
            IconBtnSize::Xs,
            cx.listener(move |this, _, _, cx| {
                delete_local_entry(this, &path_del, Some(l_idx), cx);
            }),
        ))
}

fn render_local_capture_card(
    t: &crate::theme::ActiveTheme,
    cap: LocalCaptureItem,
    cx: &mut Context<RustdianApp>,
) -> Div {
    let path_open = cap.note_path.clone();
    let path_del = cap.note_path.clone();
    let line_opt = cap.line_index;
    let insert_content = cap.content.clone();
    let meta = match cap.time_label.as_deref() {
        Some(tm) => format!("{} · {tm}", cap.date_label),
        None => cap.date_label.clone(),
    };

    div()
        .flex()
        .flex_col()
        .p(px(7.0))
        .rounded(px(RADIUS_S))
        .bg(t.surface)
        .border_1()
        .border_color(t.border_subtle)
        .hover(|d| d.border_color(t.border_default))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(4.0))
                .mb(px(4.0))
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(t.tx3)
                        .truncate()
                        .child(SharedString::from(meta)),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(t.ac)
                        .cursor_pointer()
                        .truncate()
                        .child(SharedString::from(cap.note_title.clone()))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                open_local_note_at(this, &path_open, line_opt, cx);
                            }),
                        ),
                ),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(t.tx)
                .child(cap.content.clone()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mt(px(5.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .children(cap.tags.iter().map(|tag| {
                            div()
                                .text_size(px(10.0))
                                .text_color(t.tx3)
                                .child(format!("#{tag}"))
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(t.tx3)
                                .cursor_pointer()
                                .hover(|d| d.text_color(t.ac))
                                .child(crate::t!("memos-insert-note"))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        insert_memo_to_active(this, &insert_content, cx);
                                    }),
                                ),
                        )
                        .child(icon_button_sized(
                            t,
                            "trash",
                            IconBtnSize::Xs,
                            cx.listener(move |this, _, _, cx| {
                                delete_local_entry(this, &path_del, line_opt, cx);
                            }),
                        )),
                ),
        )
}

/// Renders the Capture & Todos settings page inside the Settings dialog.
pub fn render_settings_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let vault_root = app.vault.root_dir.clone();
    let inbox_cfg = InboxSettings::load(&vault_root);
    let memos_cfg = load_config(&vault_root);

    let url_input = app.ext.memos.settings_url_input.clone();
    let token_input = app.ext.memos.settings_token_input.clone();
    if let Some(inp) = &url_input {
        inp.update(cx, |i, cx| i.set_theme(t, cx));
    }
    if let Some(inp) = &token_input {
        inp.update(cx, |i, cx| i.set_theme(t, cx));
    }
    if let Some(inp) = &app.ext.memos.stream_file_input {
        inp.update(cx, |i, cx| i.set_theme(t, cx));
    }

    // 1. Local capture & todo rules card
    let mode_chips = div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_end()
        .gap(px(6.0))
        .child(choice_chip(
            &t,
            "随手记流文件",
            inbox_cfg.capture_mode == CaptureMode::StreamFile,
            cx.listener(|this, _, _, cx| {
                let root = this.vault.root_dir.clone();
                let mut c = InboxSettings::load(&root);
                c.capture_mode = CaptureMode::StreamFile;
                let _ = c.save(&root);
                cx.notify();
            }),
        ))
        .child(choice_chip(
            &t,
            "今日日记区块",
            inbox_cfg.capture_mode == CaptureMode::DailyNote,
            cx.listener(|this, _, _, cx| {
                let root = this.vault.root_dir.clone();
                let mut c = InboxSettings::load(&root);
                c.capture_mode = CaptureMode::DailyNote;
                let _ = c.save(&root);
                cx.notify();
            }),
        ))
        .child(choice_chip(
            &t,
            "独立闪念卡片",
            inbox_cfg.capture_mode == CaptureMode::CardNote,
            cx.listener(|this, _, _, cx| {
                let root = this.vault.root_dir.clone();
                let mut c = InboxSettings::load(&root);
                c.capture_mode = CaptureMode::CardNote;
                let _ = c.save(&root);
                cx.notify();
            }),
        ));

    let scope_chips = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(6.0))
        .child(choice_chip(
            &t,
            "仅核心目录",
            inbox_cfg.todo_scope == TodoScope::CoreFolders,
            cx.listener(|this, _, _, cx| {
                let root = this.vault.root_dir.clone();
                let mut c = InboxSettings::load(&root);
                c.todo_scope = TodoScope::CoreFolders;
                let _ = c.save(&root);
                cx.notify();
            }),
        ))
        .child(choice_chip(
            &t,
            "整个笔记库",
            inbox_cfg.todo_scope == TodoScope::AllNotes,
            cx.listener(|this, _, _, cx| {
                let root = this.vault.root_dir.clone();
                let mut c = InboxSettings::load(&root);
                c.todo_scope = TodoScope::AllNotes;
                let _ = c.save(&root);
                cx.notify();
            }),
        ));

    let is_stream = inbox_cfg.capture_mode == CaptureMode::StreamFile;
    let mut local_card = setting_card(&t).child(form_row(
        &t,
        "快速记录保存方式",
        "发送随手记与待办时的默认本地 Markdown 落盘位置，无需外部服务即可使用",
        mode_chips,
        false,
    ));

    if is_stream {
        let preview = inbox_cfg.stream_rel_path(Date::today());
        let stream_presets = [
            ("随手记", "单文件"),
            ("YYYY-MM", "按月 YYYY-MM"),
            ("YYYY", "按年 YYYY"),
        ];
        let mut preset_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(t.tx3)
                    .child(SharedString::from(format!("当前写入：{preview}"))),
            );
        let mut chips = div().flex().items_center().gap(px(6.0));
        for (pattern, label) in stream_presets {
            let on = inbox_cfg.stream_file == pattern;
            chips = chips.child(choice_chip(
                &t,
                label,
                on,
                cx.listener(move |this, _, _, cx| {
                    let root = this.vault.root_dir.clone();
                    let mut c = InboxSettings::load(&root);
                    c.stream_file = pattern.to_string();
                    let _ = c.save(&root);
                    if let Some(inp) = &this.ext.memos.stream_file_input {
                        inp.update(cx, |i, cx| i.set_text(pattern, cx));
                    }
                    cx.notify();
                }),
            ));
        }
        preset_row = preset_row.child(chips);

        let stream_input_box = div()
            .w(px(180.0))
            .when_some(app.ext.memos.stream_file_input.clone(), |d, inp| {
                d.child(inp)
            });
        local_card = local_card.child(crate::ui::widgets::controls::form_row_with_extra(
            &t,
            "流文件命名",
            "保存到「随手记目录」下的文件名（可在「目录与模板」设置目录路径）",
            stream_input_box,
            preset_row,
            false,
        ));
    }

    local_card = local_card.child(form_row(
        &t,
        "待办面板扫描范围",
        "默认仅扫描日记、随手记与待办三个核心目录，避免历史归档笔记刷屏",
        scope_chips,
        true,
    ));

    // 2. Optional Memos enhancement card
    let toggle_btn = toggle(
        &t,
        memos_cfg.enabled,
        cx.listener(move |this, _, _, cx| {
            let root = this.vault.root_dir.clone();
            let mut c = load_config(&root);
            c.enabled = !c.enabled;
            let _ = save_config(&root, &c);
            cx.notify();
        }),
    );

    let mut memos_card = setting_card(&t).child(form_row(
        &t,
        &crate::t!("settings-memos-enabled"),
        "可选增强：不开启即可在本地完整使用随手记与待办；开启后可额外同步自建 Memos 服务",
        toggle_btn,
        !memos_cfg.enabled,
    ));

    if memos_cfg.enabled {
        let has_url = url_input.is_some();
        let has_token = token_input.is_some();
        if let Some(inp) = url_input {
            memos_card = memos_card.child(form_row(
                &t,
                &crate::t!("settings-memos-url"),
                "自建 Memos 服务的 Web/API 地址，如 http://192.168.1.10:5230",
                div().w(px(280.0)).flex_none().child(inp),
                !has_token,
            ));
        }
        if let Some(inp) = token_input {
            memos_card = memos_card.child(form_row(
                &t,
                &crate::t!("settings-memos-token"),
                "在 Memos 个人设置 -> 访问令牌 (Access Token) 中获取",
                div().w(px(280.0)).flex_none().child(inp),
                true,
            ));
        }
        if !has_url && !has_token {
            memos_card = memos_card.child(div());
        }
    }

    let mut root_col = div()
        .flex()
        .flex_col()
        .child(section_title(&t, "本地工作流").pt(px(4.0)))
        .child(local_card)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .pt(px(14.0))
                .pb(px(6.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.tx2)
                        .child("Memos 远端增强（可选）"),
                )
                .child(if memos_cfg.enabled {
                    badge(&t, "已启用", BadgeKind::Ok)
                } else {
                    badge(&t, "未启用 · 纯本地模式", BadgeKind::Neutral)
                }),
        )
        .child(memos_card);

    if memos_cfg.enabled {
        let footer = div()
            .flex()
            .items_center()
            .justify_end()
            .mt(px(12.0))
            .child(button(
                &t,
                "保存并同步 Memos",
                ButtonKind::Primary,
                Some("check"),
                cx.listener(move |this, _, _, cx| {
                    let root = this.vault.root_dir.clone();
                    let mut c = load_config(&root);
                    if let Some(inp) = &this.ext.memos.settings_url_input {
                        c.base_url = inp.read(cx).text().trim().to_string();
                    }
                    let _ = save_config(&root, &c);

                    if let Some(inp) = &this.ext.memos.settings_token_input {
                        let tok = inp.read(cx).text().trim().to_string();
                        let _ = save_token(&tok);
                    }

                    this.toast(ToastKind::Info, "已保存 Memos 远端增强设置");
                    refresh(this, cx);
                    cx.notify();
                }),
            ));
        root_col = root_col.child(footer);
    }

    root_col.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo_prefix_normalization() {
        assert_eq!(
            inbox::format_capture_entry("09:00", "买牛奶", true),
            "- [ ] 买牛奶"
        );
        assert_eq!(
            inbox::format_capture_entry("09:00", "- [ ] 买牛奶", true),
            "- [ ] 买牛奶"
        );
        assert_eq!(
            inbox::format_capture_entry("09:00", "闪念笔记", false),
            "- 09:00 闪念笔记"
        );
    }

    #[test]
    fn test_memo_tag_toggle_and_filter() {
        let mut state = State::default();
        assert_eq!(state.tab, Tab::Todos);
        assert!(state.selected_tag.is_none());
        let tag = "工作".to_string();
        state.selected_tag = Some(tag.clone());
        assert_eq!(state.selected_tag.as_deref(), Some("工作"));
    }
}
