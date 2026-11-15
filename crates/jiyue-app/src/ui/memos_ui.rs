//! Memos & Todo Dashboard UI module.
//!
//! Provides a sidebar section for capturing thoughts and managing todo tasks,
//! with seamless integration to Daily Notes (今日日记) and the active note editor.

use std::time::Instant;

use gpui::{
    AnyElement, Context, Entity, FontWeight, IntoElement, MouseButton, div, prelude::*, px,
};
use jiyue_core::config::credentials::{self, MEMOS_TABLE};
use jiyue_memos::{Memo, MemosClient, MemosConfig};

use crate::RustdianApp;
use crate::theme::RADIUS_S;
use crate::ui::ext::ToastKind;
use crate::ui::icons::icon;
use crate::ui::widgets::controls::{
    ButtonKind, IconBtnSize, button, icon_button_sized, toggle,
};
use crate::ui::widgets::form_row;
use crate::ui::widgets::line_input::{LineInput, LineInputEvent};

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
    /// Currently selected tag filter, if any.
    pub selected_tag: Option<String>,
    /// Whether fast input auto-prefixes `- [ ] ` to capture as a todo.
    pub as_todo: bool,
    /// Whether a network sync is in flight.
    pub loading: bool,
    /// Last error message, if any.
    pub error: Option<String>,
    /// Fast input editor in sidebar.
    pub input: Option<Entity<LineInput>>,
    /// Subscription to fast input events.
    pub _input_sub: Option<gpui::Subscription>,
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
            selected_tag: None,
            as_todo: false,
            loading: false,
            error: None,
            input: None,
            _input_sub: None,
            last_sync: None,
            settings_url_input: None,
            settings_token_input: None,
        }
    }
}

/// Initializes the Memos extension.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let t = app.theme;
    let input = cx.new(|cx| {
        let mut inp = LineInput::new(cx, crate::t!("memos-placeholder"));
        inp.set_theme(t, cx);
        inp
    });
    let sub = cx.subscribe(&input, |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
        if let LineInputEvent::Submit(_) = ev {
            submit_new_memo(this, cx);
        }
    });
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

    // Initial silent refresh if enabled
    if cfg.enabled && !cfg.base_url.trim().is_empty() {
        refresh_silent(app, cx);
    }

    // Background periodic silent sync (every 3 minutes)
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
    credentials::write_field(&path, MEMOS_TABLE, "default", "token", Some(token)).map(|_| ())
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

/// Triggers a silent refresh of memos without showing error toast.
pub fn refresh_silent(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let vault_root = app.vault.root_dir.clone();
    let client = match create_client(&vault_root) {
        Some(c) => c,
        None => return,
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

/// Triggers a refresh of memos.
pub fn refresh(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let vault_root = app.vault.root_dir.clone();
    let client = match create_client(&vault_root) {
        Some(c) => c,
        None => {
            app.toast(ToastKind::Warn, crate::t!("memos-not-configured"));
            return;
        }
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

/// Deletes a memo by resource name with optimistic removal from local cache.
pub fn delete_memo(app: &mut RustdianApp, memo_name: &str, cx: &mut Context<RustdianApp>) {
    let vault_root = app.vault.root_dir.clone();
    let client = match create_client(&vault_root) {
        Some(c) => c,
        None => {
            app.toast(ToastKind::Warn, crate::t!("memos-not-configured"));
            return;
        }
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
                Ok(()) => {
                    app.toast(ToastKind::Info, "已删除微记");
                }
                Err(e) => {
                    app.ext.memos.memos = original_memos;
                    app.toast(ToastKind::Error, format!("删除失败: {}", e));
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// Creates a new memo from sidebar fast input.
pub fn submit_new_memo(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let mut text = if let Some(i) = &app.ext.memos.input {
        i.read(cx).text().trim().to_string()
    } else {
        return;
    };

    if text.is_empty() {
        return;
    }

    if app.ext.memos.as_todo && !text.starts_with("- [ ] ") && !text.starts_with("- [x] ") {
        text = format!("- [ ] {}", text);
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
        let text_for_api = text.clone();
        let result = cx
            .background_executor()
            .spawn(async move { client.create_memo_blocking(&text_for_api, None) })
            .await;
        let _ = this.update(cx, |app, cx| {
            app.ext.memos.loading = false;
            match result {
                Ok(memo) => {
                    app.ext.memos.memos.insert(0, memo);
                    app.toast(ToastKind::Info, crate::t!("memos-toast-created"));
                }
                Err(e) => {
                    // 恢复用户输入的原始文本，防止网络波动或未连通导致灵感丢失
                    if let Some(i) = &app.ext.memos.input {
                        let restore = if app.ext.memos.as_todo && text.starts_with("- [ ] ") {
                            text.strip_prefix("- [ ] ").unwrap_or(&text).to_string()
                        } else {
                            text.clone()
                        };
                        i.update(cx, |inp, cx| inp.set_text(&restore, cx));
                    }
                    app.toast(ToastKind::Error, format!("创建失败: {}", e));
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
        let result = cx
            .background_executor()
            .spawn(async move { client.update_memo_content_blocking(&memo_name, &new_content) })
            .await;
        let _ = this.update(cx, |app, cx| {
            if let Err(e) = result {
                app.toast(ToastKind::Error, format!("更新待办失败: {}", e));
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
    lines.push("\n\n## Memos 闪念与待办\n".to_string());
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
pub fn insert_memo_to_active(
    app: &mut RustdianApp,
    memo_content: &str,
    cx: &mut Context<RustdianApp>,
) {
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
                .gap(px(2.0))
                // Append to Daily button
                .child(icon_button_sized(
                    &t,
                    "calendar-event",
                    IconBtnSize::Small,
                    cx.listener(|this, _, _, cx| {
                        append_today_memos_to_daily(this, cx);
                    }),
                ))
                // Refresh button
                .child(icon_button_sized(
                    &t,
                    "arrows-sort",
                    IconBtnSize::Small,
                    cx.listener(|this, _, _, cx| {
                        refresh(this, cx);
                    }),
                ))
                // Toggle fold
                .child(icon_button_sized(
                    &t,
                    if is_open { "arrow-down" } else { "arrow-up" },
                    IconBtnSize::Small,
                    cx.listener(|this, _, _, cx| {
                        this.ext.memos.open = !this.ext.memos.open;
                        cx.notify();
                    }),
                )),
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
                    d.bg(t.active)
                        .text_color(t.tx)
                        .font_weight(FontWeight::MEDIUM)
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
                    d.bg(t.active)
                        .text_color(t.tx)
                        .font_weight(FontWeight::MEDIUM)
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

    // Tags pill filter bar
    let mut all_tags: Vec<String> = Vec::new();
    for memo in &app.ext.memos.memos {
        for tag in &memo.tags {
            if !all_tags.contains(tag) {
                all_tags.push(tag.clone());
            }
        }
    }
    all_tags.sort();

    if !all_tags.is_empty() {
        let is_all_sel = app.ext.memos.selected_tag.is_none();
        let mut tag_bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(5.0))
            .px(px(4.0))
            .py(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(22.0))
                    .px(px(8.0))
                    .rounded(px(11.0))
                    .border_1()
                    .text_size(px(11.0))
                    .cursor_pointer()
                    .when(is_all_sel, |d| {
                        d.bg(t.ac_soft)
                            .border_color(t.ac)
                            .text_color(t.ac)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!is_all_sel, |d| {
                        d.bg(t.sheet)
                            .border_color(t.line_soft)
                            .text_color(t.tx2)
                            .hover(|h| h.bg(t.hover).text_color(t.tx))
                    })
                    .child(crate::t!("memos-tab-all"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.ext.memos.selected_tag = None;
                            cx.notify();
                        }),
                    ),
            );

        for tag in all_tags {
            let is_sel = app.ext.memos.selected_tag.as_deref() == Some(&tag);
            let t_clone = tag.clone();
            tag_bar = tag_bar.child(
                div()
                    .flex()
                    .items_center()
                    .h(px(22.0))
                    .px(px(8.0))
                    .rounded(px(11.0))
                    .border_1()
                    .text_size(px(11.0))
                    .cursor_pointer()
                    .when(is_sel, |d| {
                        d.bg(t.ac_soft)
                            .border_color(t.ac)
                            .text_color(t.ac)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!is_sel, |d| {
                        d.bg(t.sheet)
                            .border_color(t.line_soft)
                            .text_color(t.tx2)
                            .hover(|h| h.bg(t.hover).text_color(t.tx))
                    })
                    .child(format!("#{}", tag))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if this.ext.memos.selected_tag.as_deref() == Some(&t_clone) {
                                this.ext.memos.selected_tag = None;
                            } else {
                                this.ext.memos.selected_tag = Some(t_clone.clone());
                            }
                            cx.notify();
                        }),
                    ),
            );
        }
        section = section.child(tag_bar);
    }

    // Fast Input Box with Todo-mode Toggle (unified streamlined modern bar)
    let as_todo = app.ext.memos.as_todo;
    let input_el = if let Some(inp) = &app.ext.memos.input {
        inp.update(cx, |i, cx| {
            i.set_theme(t, cx);
            i.set_borderless(true, cx);
        });
        div()
            .flex()
            .items_center()
            .h(px(32.0))
            .pl(px(4.0))
            .pr(px(3.0))
            .my(px(6.0))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .rounded(px(8.0))
            .shadow_sm()
            .overflow_hidden()
            .hover(|d| d.border_color(t.ac_line))
            .child(
                div()
                    .size(px(24.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(as_todo, |d| d.bg(t.ac_soft))
                    .when(!as_todo, |d| d.hover(|h| h.bg(t.hover)))
                    .child(icon(
                        if as_todo { "checkbox" } else { "list" },
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(inp.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(24.0))
                    .px(px(10.0))
                    .rounded(px(5.0))
                    .bg(t.ac)
                    .text_color(gpui::white())
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|h| h.opacity(0.85))
                    .child("发送")
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
        // Filter by selected tag if any
        if let Some(ref sel_tag) = app.ext.memos.selected_tag
            && !memo.tags.contains(sel_tag)
        {
            continue;
        }

        let m_name = memo.name.clone();
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
                let m_del_name = m_name.clone();

                let row = div()
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
                            delete_memo(this, &m_del_name, cx);
                        }),
                    ));
                card_container = card_container.child(row);
            }
        } else {
            // Render plain memo card
            displayed_count += 1;
            let m_content = memo.content.clone();
            let m_del_name = m_name.clone();
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

    if displayed_count == 0 {
        let empty_tip = if tab == Tab::Todos {
            crate::t!("memos-empty-todos")
        } else {
            crate::t!("memos-empty-all")
        };
        card_container = card_container.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .py(px(20.0))
                .gap(px(6.0))
                .child(icon(
                    if tab == Tab::Todos {
                        "check"
                    } else {
                        "file-text"
                    },
                    18.0,
                    t.tx3,
                ))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .child(empty_tip),
                ),
        );
    }

    section = section.child(card_container);
    section
}

/// Renders the Memos settings panel in the Settings dialog.
pub fn render_settings_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let vault_root = app.vault.root_dir.clone();
    let cfg = load_config(&vault_root);

    let url_input = app.ext.memos.settings_url_input.clone();
    let token_input = app.ext.memos.settings_token_input.clone();

    if let Some(inp) = &url_input {
        inp.update(cx, |i, cx| i.set_theme(t, cx));
    }
    if let Some(inp) = &token_input {
        inp.update(cx, |i, cx| i.set_theme(t, cx));
    }

    let mut body = div().flex().flex_col().mt(px(4.0));

    // 1. Enable Toggle Row
    let toggle_btn = toggle(
        &t,
        cfg.enabled,
        cx.listener(move |this, _, _, cx| {
            let root = this.vault.root_dir.clone();
            let mut c = load_config(&root);
            c.enabled = !c.enabled;
            let _ = save_config(&root, &c);
            cx.notify();
        }),
    );

    body = body.child(form_row(
        &t,
        &crate::t!("settings-memos-enabled"),
        &crate::t!("settings-memos-desc"),
        toggle_btn,
        false,
    ));

    // 2. Base URL Row
    if let Some(inp) = url_input {
        body = body.child(form_row(
            &t,
            &crate::t!("settings-memos-url"),
            "自建 Memos 服务的 Web/API 地址，如 http://192.168.1.10:5230",
            div().w(px(320.0)).flex_none().child(inp),
            false,
        ));
    }

    // 3. Token Row
    if let Some(inp) = token_input {
        body = body.child(form_row(
            &t,
            &crate::t!("settings-memos-token"),
            "在 Memos 个人设置 -> 访问令牌 (Access Token) 中获取",
            div().w(px(320.0)).flex_none().child(inp),
            false,
        ));
    }

    // 4. Save Button
    let save_btn = div().flex().justify_end().mt(px(16.0)).child(button(
        &t,
        "保存并连接 / Save & Connect",
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

            this.toast(ToastKind::Info, "已保存 Memos 设置");
            refresh(this, cx);
            cx.notify();
        }),
    ));

    body = body.child(save_btn);
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo_prefix_normalization() {
        let normalize = |text: &str, as_todo: bool| -> String {
            let mut s = text.to_string();
            if as_todo && !s.starts_with("- [ ] ") && !s.starts_with("- [x] ") {
                s = format!("- [ ] {}", s);
            }
            s
        };

        // 普通文本在 todo 模式自动加上 - [ ]
        assert_eq!(normalize("买牛奶", true), "- [ ] 买牛奶");

        // 已经带有 - [ ] 的不重复加
        assert_eq!(normalize("- [ ] 买牛奶", true), "- [ ] 买牛奶");

        // 已经带有 - [x] 的不重复加
        assert_eq!(normalize("- [x] 已完成", true), "- [x] 已完成");

        // 非 todo 模式保持原样
        assert_eq!(normalize("闪念笔记", false), "闪念笔记");
    }

    #[test]
    fn test_todo_multiline_toggle_preserves_indent() {
        let raw = "第一行普通文本\n  - [ ] 二级待办项\n    - [x] 三级已完成";
        let memo = Memo::from_raw("m1".into(), raw.into(), None, None, None, false);

        let todos = &memo.todos;
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].line_index, 1);
        assert_eq!(todos[0].text, "二级待办项");
        assert!(!todos[0].done);

        assert_eq!(todos[1].line_index, 2);
        assert_eq!(todos[1].text, "三级已完成");
        assert!(todos[1].done);

        // 切换第 1 行（二级待办）的状态 -> 变为 - [x]，保留缩进
        let toggled = memo.toggle_todo_content(1).unwrap();
        assert!(toggled.contains("  - [x] 二级待办项"));
        assert!(toggled.contains("    - [x] 三级已完成"));

        // 再次切换 -> 变回 - [ ]
        let memo2 = Memo::from_raw("m2".into(), toggled, None, None, None, false);
        let toggled_back = memo2.toggle_todo_content(1).unwrap();
        assert!(toggled_back.contains("  - [ ] 二级待办项"));
    }

    #[test]
    fn test_memo_tag_toggle_and_filter() {
        let mut state = State::default();
        assert_eq!(state.tab, Tab::Todos);
        assert!(state.selected_tag.is_none());

        // 模拟点击 tag
        let tag = "工作".to_string();
        if state.selected_tag.as_deref() == Some(&tag) {
            state.selected_tag = None;
        } else {
            state.selected_tag = Some(tag.clone());
        }
        assert_eq!(state.selected_tag.as_deref(), Some("工作"));

        // 再次点击同一 tag -> 取消筛选
        if state.selected_tag.as_deref() == Some(&tag) {
            state.selected_tag = None;
        } else {
            state.selected_tag = Some(tag.clone());
        }
        assert!(state.selected_tag.is_none());
    }

    #[test]
    fn test_append_memos_format() {
        let m1 = Memo::from_raw(
            "1".into(),
            "- [ ] 待办 A #重要".into(),
            None,
            None,
            None,
            false,
        );
        let m2 = Memo::from_raw(
            "2".into(),
            "普通灵感笔记".into(),
            None,
            None,
            None,
            false,
        );
        let memos = vec![m1, m2];

        let mut lines = Vec::new();
        lines.push("## Memos 闪念与待办\n".to_string());
        for memo in &memos {
            for line in memo.content.lines() {
                lines.push(line.to_string());
            }
            lines.push("".to_string());
        }
        let output = lines.join("\n");
        assert!(output.contains("- [ ] 待办 A #重要"));
        assert!(output.contains("普通灵感笔记"));
    }
}
