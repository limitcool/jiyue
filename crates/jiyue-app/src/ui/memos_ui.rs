//! Memos & Todo Dashboard UI module.
//!
//! Provides a sidebar section for capturing thoughts and managing todo tasks,
//! with seamless integration to Daily Notes (今日日记) and the active note editor.

use std::time::Instant;

use gpui::{
    AnyElement, Context, Entity, FontWeight, IntoElement, MouseButton,
    div, prelude::*, px,
};
use jiyue_core::config::credentials::{self, MEMOS_TABLE};
use jiyue_memos::{Memo, MemosClient, MemosConfig};

use crate::theme::RADIUS_S;
use crate::ui::ext::ToastKind;
use crate::ui::icons::icon;
use crate::ui::widgets::line_input::LineInput;
use crate::RustdianApp;

/// Which tab is active in the Memos sidebar panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Todos,
    All,
}

/// State for the Memos feature.
pub struct State {
    /// Whether the Memos section is open in the left sidebar.
    pub open: bool,
    /// Currently selected tab.
    pub tab: Tab,
    /// Locally cached list of memos.
    pub memos: Vec<Memo>,
    /// Whether a network sync is in flight.
    pub loading: bool,
    /// Last error message, if any.
    pub error: Option<String>,
    /// Fast input editor in sidebar.
    pub input: Option<Entity<LineInput>>,
    /// Last successful sync timestamp.
    pub last_sync: Option<Instant>,
    /// Settings page: server URL input.
    pub settings_url_input: Option<Entity<LineInput>>,
    /// Settings page: token input.
    pub settings_token_input: Option<Entity<LineInput>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: true,
            tab: Tab::Todos,
            memos: Vec::new(),
            loading: false,
            error: None,
            input: None,
            last_sync: None,
            settings_url_input: None,
            settings_token_input: None,
        }
    }
}

/// Initializes the Memos extension.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let input = cx.new(|cx| LineInput::new(cx, crate::t!("memos-placeholder")));
    app.ext.memos.input = Some(input);

    let cfg = load_config(&app.vault.root_dir);
    let token = load_token().unwrap_or_default();
    let url_input = cx.new(|cx| LineInput::new(cx, &cfg.base_url));
    let token_input = cx.new(|cx| LineInput::new(cx, &token));
    app.ext.memos.settings_url_input = Some(url_input);
    app.ext.memos.settings_token_input = Some(token_input);
}

/// Loads the `[memos]` section from `<vault>/.jiyue/config.toml`.
pub fn load_config(root: &std::path::Path) -> MemosConfig {
    let loaded: jiyue_core::config::Loaded<MemosConfig> = jiyue_core::config::load_section(
        &jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE),
        "memos",
    );
    loaded.value
}

/// Saves the `[memos]` section to `<vault>/.jiyue/config.toml`.
pub fn save_config(root: &std::path::Path, cfg: &MemosConfig) -> Result<(), String> {
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
    credentials::write_field(
        &path,
        MEMOS_TABLE,
        "default",
        "token",
        Some(token),
    )
    .map(|_| ())
}

/// Creates a MemosClient from vault config and credentials.
pub fn create_client(vault_root: &std::path::Path) -> Option<MemosClient> {
    let cfg = load_config(vault_root);
    if !cfg.enabled || cfg.base_url.trim().is_empty() {
        return None;
    }
    let token = load_token();
    MemosClient::new(&cfg.base_url, token.as_deref()).ok()
}

/// Triggers a refresh of memos.
pub fn refresh(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let vault_root = app.vault.root_dir.clone();
    let client = match create_client(&vault_root) {
        Some(c) => c,
        None => {
            app.toast(
                ToastKind::Warn,
                crate::t!("memos-not-configured"),
            );
            return;
        }
    };

    app.ext.memos.loading = true;
    app.ext.memos.error = None;
    cx.notify();

    cx.spawn(async move |this, cx| {
        let result = client.list_memos(50, None).await;
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

/// Creates a new memo from sidebar fast input.
pub fn submit_new_memo(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let text = if let Some(i) = &app.ext.memos.input {
        i.read(cx).text().trim().to_string()
    } else {
        return;
    };

    if text.is_empty() {
        return;
    }

    let vault_root = app.vault.root_dir.clone();
    let client = match create_client(&vault_root) {
        Some(c) => c,
        None => {
            app.toast(ToastKind::Warn, crate::t!("memos-not-configured"));
            return;
        }
    };

    // Clear input immediately for quick typing
    if let Some(i) = &app.ext.memos.input {
        i.update(cx, |i, cx| i.set_text("", cx));
    }

    app.ext.memos.loading = true;
    cx.notify();

    cx.spawn(async move |this, cx| {
        let result = client.create_memo(&text, None).await;
        let _ = this.update(cx, |app, cx| {
            app.ext.memos.loading = false;
            match result {
                Ok(memo) => {
                    app.ext.memos.memos.insert(0, memo);
                    app.toast(ToastKind::Info, crate::t!("memos-toast-created"));
                }
                Err(e) => {
                    app.toast(
                        ToastKind::Error,
                        format!("创建失败: {}", e),
                    );
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// Toggles a todo checkbox in a memo.
pub fn toggle_todo(
    app: &mut RustdianApp,
    memo_id: &str,
    line_index: usize,
    cx: &mut Context<RustdianApp>,
) {
    let vault_root = app.vault.root_dir.clone();
    let client = match create_client(&vault_root) {
        Some(c) => c,
        None => return,
    };

    let memo_pos = app.ext.memos.memos.iter().position(|m| m.id == memo_id);
    let (memo_name, new_content) = if let Some(pos) = memo_pos {
        let m = &app.ext.memos.memos[pos];
        if let Some(new_c) = m.toggle_todo_content(line_index) {
            let name = m.name.clone();
            // Optimistic local update
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
        let result = client.update_memo_content(&memo_name, &new_content).await;
        let _ = this.update(cx, |app, cx| {
            if let Err(e) = result {
                app.toast(
                    ToastKind::Error,
                    format!("更新待办失败: {}", e),
                );
                // Re-fetch on error to restore consistency
                refresh(app, cx);
            }
        });
    })
    .detach();
}

/// Appends all pending todos or today's memos to today's daily note.
pub fn append_today_memos_to_daily(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    // Open or create today's daily note
    app.open_today();

    let path = match app.active_path() {
        Some(p) => p.to_string(),
        None => return,
    };

    // Collect today's memos or pending todos
    let mut lines = Vec::new();
    lines.push(format!("\n\n## Memos 闪念与待办\n"));
    for memo in &app.ext.memos.memos {
        for line in memo.content.lines() {
            lines.push(line.to_string());
        }
        lines.push("".to_string());
    }

    let append_text = lines.join("\n");
    if let Some(editor) = app.editor_for(&path) {
        editor.update(cx, |ed, cx| {
            let current = ed.text();
            let new_text = format!("{}{}", current, append_text);
            ed.set_text(&new_text, cx);
        });
        app.toast(ToastKind::Info, crate::t!("memos-toast-appended"));
    }
}

/// Inserts a memo's content into the active editor at the current cursor position.
pub fn insert_memo_to_active(app: &mut RustdianApp, memo_content: &str, cx: &mut Context<RustdianApp>) {
    let path = match app.active_path() {
        Some(p) => p.to_string(),
        None => return,
    };

    if let Some(editor) = app.editor_for(&path) {
        editor.update(cx, |ed, cx| {
            let current = ed.text();
            let new_text = format!("{}\n\n{}\n", current, memo_content);
            ed.set_text(&new_text, cx);
        });
        app.toast(ToastKind::Info, crate::t!("memos-toast-inserted"));
    }
}

/// Renders the Memos sidebar section (collapsible).
pub fn render_sidebar_section(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> impl IntoElement + use<> {
    let t = app.theme;
    let is_open = app.ext.memos.open;
    let tab = app.ext.memos.tab;

    let mut section = div().flex().flex_col().w_full().my(px(4.0));

    // Header row
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(26.0))
        .px(px(6.0))
        .rounded(px(RADIUS_S))
        .cursor_pointer()
        .hover(|d| d.bg(t.hover))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(icon("checkbox", 14.0, t.tx2))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(12.0))
                        .text_color(t.tx2)
                        .child(crate::t!("sidebar-memos")),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                // Append to Daily button
                .child(
                    div()
                        .id("memos_btn_append_daily")
                        .size(px(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(RADIUS_S))
                        .hover(|d| d.bg(t.active))
                        .child(icon("calendar-event", 12.0, t.tx3))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                            append_today_memos_to_daily(this, cx);
                        })),
                )
                // Refresh button
                .child(
                    div()
                        .id("memos_btn_refresh")
                        .size(px(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(RADIUS_S))
                        .hover(|d| d.bg(t.active))
                        .child(icon("arrows-sort", 12.0, t.tx3))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                            refresh(this, cx);
                        })),
                )
                // Toggle fold
                .child(
                    div()
                        .size(px(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(
                            if is_open { "arrow-down" } else { "arrow-up" },
                            11.0,
                            t.tx3,
                        )),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.ext.memos.open = !this.ext.memos.open;
                cx.notify();
            }),
        );

    section = section.child(header);

    if !is_open {
        return section;
    }

    // Tab switcher: Todos / All
    let tab_bar = div()
        .flex()
        .gap(px(4.0))
        .px(px(4.0))
        .py(px(4.0))
        .child(
            div()
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(RADIUS_S))
                .text_size(px(11.0))
                .cursor_pointer()
                .when(tab == Tab::Todos, |d| {
                    d.bg(t.active).text_color(t.tx).font_weight(FontWeight::MEDIUM)
                })
                .when(tab != Tab::Todos, |d| {
                    d.text_color(t.tx3).hover(|h| h.text_color(t.tx))
                })
                .child(crate::t!("memos-tab-todos"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.ext.memos.tab = Tab::Todos;
                        cx.notify();
                    }),
                ),
        )
        .child(
            div()
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(RADIUS_S))
                .text_size(px(11.0))
                .cursor_pointer()
                .when(tab == Tab::All, |d| {
                    d.bg(t.active).text_color(t.tx).font_weight(FontWeight::MEDIUM)
                })
                .when(tab != Tab::All, |d| {
                    d.text_color(t.tx3).hover(|h| h.text_color(t.tx))
                })
                .child(crate::t!("memos-tab-all"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.ext.memos.tab = Tab::All;
                        cx.notify();
                    }),
                ),
        );

    section = section.child(tab_bar);

    // Fast Input Box
    let input_el = if let Some(inp) = &app.ext.memos.input {
        div()
            .flex()
            .items_center()
            .px(px(4.0))
            .py(px(2.0))
            .child(
                div()
                    .flex_1()
                    .h(px(28.0))
                    .bg(t.sheet)
                    .border_1()
                    .border_color(t.line)
                    .rounded(px(RADIUS_S))
                    .px(px(6.0))
                    .child(inp.clone())
                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _, cx| {
                        if ev.keystroke.key == "enter" {
                            submit_new_memo(this, cx);
                        }
                    })),
            )
            .child(
                div()
                    .ml(px(4.0))
                    .px(px(8.0))
                    .h(px(28.0))
                    .bg(t.ac)
                    .text_color(t.ac_on)
                    .text_size(px(11.0))
                    .rounded(px(RADIUS_S))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(crate::t!("memos-btn-send"))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                        submit_new_memo(this, cx);
                    })),
            )
    } else {
        div().h(px(0.0))
    };

    section = section.child(input_el);

    // Memos / Todos list
    let memos_list = app.ext.memos.memos.clone();
    let mut card_container = div()
        .id("memos_card_container")
        .flex()
        .flex_col()
        .gap(px(4.0))
        .max_h(px(260.0))
        .overflow_y_scroll()
        .px(px(2.0))
        .py(px(4.0));

    let mut displayed_count = 0;
    for memo in &memos_list {
        if tab == Tab::Todos {
            if !memo.has_todos() {
                continue;
            }
            // Render each todo item in this memo
            for todo in &memo.todos {
                displayed_count += 1;
                let mid = memo.id.clone();
                let l_idx = todo.line_index;
                let is_done = todo.done;

                let row = div()
                    .flex()
                    .items_start()
                    .gap(px(6.0))
                    .p(px(4.0))
                    .rounded(px(RADIUS_S))
                    .hover(|d| d.bg(t.hover))
                    .child(
                        div()
                            .mt(px(2.0))
                            .cursor_pointer()
                            .child(icon(
                                if is_done { "checkbox" } else { "check" },
                                13.0,
                                if is_done { t.tx3 } else { t.ac },
                            ))
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                toggle_todo(this, &mid, l_idx, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.0))
                            .text_color(if is_done { t.tx3 } else { t.tx })
                            .when(is_done, |d| d.line_through())
                            .child(todo.text.clone()),
                    );
                card_container = card_container.child(row);
            }
        } else {
            // Render plain memo card
            displayed_count += 1;
            let m_content = memo.content.clone();
            let card = div()
                .flex()
                .flex_col()
                .p(px(6.0))
                .rounded(px(RADIUS_S))
                .bg(t.sheet)
                .border_1()
                .border_color(t.line_soft)
                .hover(|d| d.border_color(t.line))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(t.tx)
                        .child(memo.content.clone()),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(4.0))
                        .mt(px(4.0))
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(t.tx3)
                                .cursor_pointer()
                                .hover(|d| d.text_color(t.ac))
                                .child(crate::t!("memos-insert-note"))
                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                    insert_memo_to_active(this, &m_content, cx);
                                })),
                        ),
                );
            card_container = card_container.child(card);
        }
    }

    if displayed_count == 0 {
        let empty_tip = if tab == Tab::Todos {
            crate::t!("memos-empty-todos")
        } else {
            crate::t!("memos-empty-all")
        };
        card_container = card_container.child(
            div()
                .py(px(12.0))
                .text_size(px(11.0))
                .text_color(t.tx3)
                .child(empty_tip),
        );
    }

    section = section.child(card_container);
    section
}

/// Renders the Memos settings panel in the Settings dialog.
pub fn render_settings_section(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let vault_root = app.vault.root_dir.clone();
    let cfg = load_config(&vault_root);

    let url_input = app.ext.memos.settings_url_input.clone();
    let token_input = app.ext.memos.settings_token_input.clone();

    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_size(px(16.0))
                .text_color(t.tx)
                .child(crate::t!("settings-memos-title")),
        )
        .child(
            div()
                .text_size(px(13.0))
                .text_color(t.tx2)
                .child(crate::t!("settings-memos-desc")),
        )
        // Enable toggle
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .py(px(8.0))
                .border_b_1()
                .border_color(t.line_soft)
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(t.tx)
                        .child(crate::t!("settings-memos-enabled")),
                )
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(px(RADIUS_S))
                        .bg(if cfg.enabled { t.ac } else { t.sheet })
                        .text_color(if cfg.enabled { t.ac_on } else { t.tx3 })
                        .cursor_pointer()
                        .child(if cfg.enabled { "已启用 / Enabled" } else { "已禁用 / Disabled" })
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            let root = this.vault.root_dir.clone();
                            let mut c = load_config(&root);
                            c.enabled = !c.enabled;
                            let _ = save_config(&root, &c);
                            cx.notify();
                        })),
                ),
        )
        // URL Input
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(t.tx)
                        .child(crate::t!("settings-memos-url")),
                )
                .child({
                    let el = div()
                        .h(px(32.0))
                        .px(px(8.0))
                        .bg(t.sheet)
                        .border_1()
                        .border_color(t.line)
                        .rounded(px(RADIUS_S))
                        .flex()
                        .items_center();
                    if let Some(inp) = url_input {
                        el.child(inp)
                    } else {
                        el
                    }
                }),
        )
        // Token Input
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(t.tx)
                        .child(crate::t!("settings-memos-token")),
                )
                .child({
                    let el = div()
                        .h(px(32.0))
                        .px(px(8.0))
                        .bg(t.sheet)
                        .border_1()
                        .border_color(t.line)
                        .rounded(px(RADIUS_S))
                        .flex()
                        .items_center();
                    if let Some(inp) = token_input {
                        el.child(inp)
                    } else {
                        el
                    }
                }),
        )
        // Save Button
        .child(
            div()
                .flex()
                .justify_end()
                .mt(px(8.0))
                .child(
                    div()
                        .px(px(14.0))
                        .py(px(6.0))
                        .bg(t.ac)
                        .text_color(t.ac_on)
                        .rounded(px(RADIUS_S))
                        .cursor_pointer()
                        .child("保存并连接 / Save & Connect")
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
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

                            this.toast(ToastKind::Info, "已保存 Memos 设置");
                            refresh(this, cx);
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}
