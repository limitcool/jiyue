//! The AI category of the settings dialog: master switch (with consent), providers (add, edit,
//! remove; keys go to the Windows credential store on a background thread), the active chat and
//! embedding providers, excluded folders and tags, the vector index, and the privacy note.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Div, Entity, FontWeight, MouseButton, MouseDownEvent, Window, div,
    prelude::*, px,
};
use jiyue_ai::{ProviderEntry, ProviderKind};
use jiyue_core::config::credentials::{self, AI_TABLE};

use super::errors::{self, ProviderForm};
use super::{
    Modal, chat, effective_config, host_of, indexer, input_clear, input_text, runner, worker,
};
use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::ext::ToastKind;
use crate::ui::icons::icon;
use crate::ui::widgets::controls::PickIndex;
use crate::ui::widgets::{
    ButtonKind, LineInput, button, form_row, icon_button, section_title, segmented, toggle,
};

/// Height of the scrolling part of the settings page (the dialog is 540 px high).
const SCROLL_HEIGHT: f32 = 440.0;
const RED: u32 = 0xe5484d;

/// Outcome of the connection test shown next to its button.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TestState {
    #[default]
    Idle,
    Running,
    Ok,
    Failed(String),
}

/// The provider editor, open for a new or an existing provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editing {
    /// `None` for a provider that is being added.
    pub id: Option<String>,
    pub kind: ProviderKind,
    pub error: Option<String>,
}

/// Settings-page state.
#[derive(Default)]
pub struct Ui {
    pub editing: Option<Editing>,
    pub test: TestState,
    /// Whether a key is stored, per provider id (read from the credential store off the UI thread).
    pub key_saved: HashMap<String, bool>,
    /// How the stored key of each provider is given (literal, `env:`, `cmd:`); never the value.
    pub key_display: HashMap<String, crate::ui::config_ui::SecretDisplay>,
    /// A short confirmation shown at the top of the page.
    pub notice: Option<String>,
}

/// Opens the settings dialog on the AI category.
pub fn open_settings(app: &mut RustdianApp) {
    app.settings.open = true;
    app.settings.category = crate::ui::settings::CAT_AI;
}

/// Reads the stored-key flags of all providers on a background thread.
pub fn refresh_keys(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let ids: Vec<String> = app
        .ext
        .ai
        .config
        .providers
        .iter()
        .map(|p| p.id.clone())
        .collect();
    runner::once(
        cx,
        move || {
            ids.into_iter()
                .map(|id| {
                    let saved = worker::key_saved(&id, None);
                    let display = worker::key_display(&id);
                    (id, saved, display)
                })
                .collect::<Vec<_>>()
        },
        |app, flags, cx| {
            app.ext.ai.settings.key_saved = flags.iter().map(|(i, s, _)| (i.clone(), *s)).collect();
            app.ext.ai.settings.key_display = flags.into_iter().map(|(i, _, d)| (i, d)).collect();
            cx.notify();
        },
    );
}

/// Writes the configuration file. Shows a message and returns false when it fails.
fn persist(app: &mut RustdianApp) -> bool {
    if let Err(e) = app.ext.ai.config.validate() {
        app.toast(ToastKind::Error, e.to_string());
        return false;
    }
    match super::super::ai_ui::save_config(&app.vault.root_dir, &app.ext.ai.config) {
        Ok(()) => true,
        Err(e) => {
            app.toast(ToastKind::Error, e.to_string());
            false
        }
    }
}

/// The master switch. Turning AI on first shows the consent dialog.
pub fn toggle_enabled(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.ext.ai.config.enabled {
        app.ext.ai.config.enabled = false;
        persist(app);
        runner::stop_index(app);
        chat::cancel_preview(app);
        chat::stop(app);
        app.toast(ToastKind::Info, "AI 功能已关闭，不再发送任何内容");
    } else {
        app.ext.ai.modal = Modal::Consent;
    }
    cx.notify();
}

/// The consent dialog's button: turns AI on and starts the index if an embedding service is set.
pub fn confirm_enable(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.ext.ai.modal = Modal::None;
    app.ext.ai.config.enabled = true;
    if persist(app) {
        app.toast(ToastKind::Success, "AI 功能已开启");
        if super::index_wanted(app) {
            runner::start_index(app, cx, None);
        }
    } else {
        app.ext.ai.config.enabled = false;
    }
    cx.notify();
}

/// Opens the editor with the fields of `id`, or empty fields for a new provider.
pub fn begin_edit(app: &mut RustdianApp, id: Option<String>, cx: &mut Context<RustdianApp>) {
    let (name, url, chat_model, embed_model, kind) = match id.as_ref().and_then(|i| {
        app.ext
            .ai
            .config
            .providers
            .iter()
            .find(|p| &p.id == i)
            .cloned()
    }) {
        Some(p) => (
            p.name,
            p.base_url,
            p.chat_model,
            p.embedding_model.unwrap_or_default(),
            p.kind,
        ),
        None => (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            ProviderKind::OpenAiCompatible,
        ),
    };
    let inputs = &app.ext.ai.inputs;
    for (input, text) in [
        (&inputs.name, name),
        (&inputs.url, url),
        (&inputs.chat_model, chat_model),
        (&inputs.embed_model, embed_model),
        (&inputs.api_key, String::new()),
    ] {
        if let Some(i) = input {
            i.update(cx, |i, cx| i.set_text(&text, cx));
        }
    }
    app.ext.ai.settings.editing = Some(Editing {
        id,
        kind,
        error: None,
    });
    app.ext.ai.settings.test = TestState::Idle;
    cx.notify();
}

/// Closes the editor without saving.
pub fn cancel_edit(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.ext.ai.settings.editing = None;
    app.ext.ai.settings.test = TestState::Idle;
    input_clear(&app.ext.ai.inputs.api_key, cx);
    cx.notify();
}

fn form_from_inputs(app: &RustdianApp, kind: ProviderKind, cx: &App) -> ProviderForm {
    let inputs = &app.ext.ai.inputs;
    ProviderForm {
        name: input_text(&inputs.name, cx),
        kind,
        base_url: input_text(&inputs.url, cx),
        chat_model: input_text(&inputs.chat_model, cx),
        embedding_model: input_text(&inputs.embed_model, cx),
    }
}

/// Saves the editor's provider. A key typed in the form goes to the credential store on a
/// background thread; the configuration file is written at once.
pub fn save_edit(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(editing) = app.ext.ai.settings.editing.clone() else {
        return;
    };
    let form = form_from_inputs(app, editing.kind, cx);
    let is_new = editing.id.is_none();
    let id = editing
        .id
        .clone()
        .unwrap_or_else(|| errors::new_provider_id(&app.ext.ai.config));
    let entry = match errors::validate_provider(&id, &form) {
        Ok(entry) => entry,
        Err(message) => {
            if let Some(e) = app.ext.ai.settings.editing.as_mut() {
                e.error = Some(message);
            }
            cx.notify();
            return;
        }
    };
    let key = input_text(&app.ext.ai.inputs.api_key, cx)
        .trim()
        .to_string();

    let cfg = &mut app.ext.ai.config;
    match cfg.providers.iter_mut().find(|p| p.id == id) {
        Some(existing) => *existing = entry.clone(),
        None => cfg.providers.push(entry.clone()),
    }
    if is_new {
        if cfg.active_chat.is_none() {
            cfg.active_chat = Some(id.clone());
        }
        if entry.embedding_model.is_some() && cfg.active_embedding.is_none() {
            cfg.active_embedding = Some(id.clone());
        }
    }
    app.ext.ai.settings.editing = None;
    app.ext.ai.settings.test = TestState::Idle;
    input_clear(&app.ext.ai.inputs.api_key, cx);
    if !persist(app) {
        cx.notify();
        return;
    }
    app.ext.ai.settings.notice = Some(format!("已保存服务「{}」", entry.name));
    if !key.is_empty() {
        let key_id = id.clone();
        let display = crate::ui::config_ui::display_of(Some(key.as_str()));
        runner::once(
            cx,
            move || {
                credentials::write_field(
                    &credentials::path(),
                    AI_TABLE,
                    &key_id,
                    "api_key",
                    Some(&key),
                )
                .map(|o| o.permissions_ok)
            },
            move |app, result, cx| {
                match result {
                    Ok(ok) => {
                        crate::ui::config_ui::check_permissions(app, ok);
                        app.ext.ai.settings.key_saved.insert(id.clone(), true);
                        app.ext.ai.settings.key_display.insert(id.clone(), display);
                        app.toast(ToastKind::Success, "密钥已保存到 credentials.toml");
                    }
                    Err(m) => app.toast(ToastKind::Error, format!("密钥没有保存：{m}")),
                }
                cx.notify();
            },
        );
    }
    if super::index_wanted(app) {
        runner::start_index(app, cx, None);
    }
    cx.notify();
}

/// Removes a provider, its stored key, and any active reference to it.
pub fn delete_provider(app: &mut RustdianApp, id: &str, cx: &mut Context<RustdianApp>) {
    app.ext.ai.modal = Modal::None;
    app.ext.ai.config.providers.retain(|p| p.id != id);
    if app.ext.ai.config.active_chat.as_deref() == Some(id) {
        app.ext.ai.config.active_chat = None;
    }
    if app.ext.ai.config.active_embedding.as_deref() == Some(id) {
        app.ext.ai.config.active_embedding = None;
        runner::stop_index(app);
    }
    persist(app);
    app.ext.ai.settings.key_saved.remove(id);
    app.ext.ai.settings.key_display.remove(id);
    let key_id = id.to_string();
    runner::once(
        cx,
        move || {
            credentials::write_field(&credentials::path(), AI_TABLE, &key_id, "api_key", None)
                .map(|_| ())
        },
        |app, result, cx| {
            if let Err(m) = result {
                app.toast(ToastKind::Error, format!("删除密钥失败：{m}"));
            }
            cx.notify();
        },
    );
    app.ext.ai.settings.notice = Some("已删除服务".into());
    cx.notify();
}

/// Chooses the provider for chat (`None` clears it).
pub fn set_active_chat(app: &mut RustdianApp, id: Option<String>, cx: &mut Context<RustdianApp>) {
    app.ext.ai.config.active_chat = id.filter(|i| i != worker::TEST_ID);
    persist(app);
    cx.notify();
}

/// Chooses the provider for embeddings. Turning it on starts the index; turning it off stops it.
pub fn set_active_embedding(
    app: &mut RustdianApp,
    id: Option<String>,
    cx: &mut Context<RustdianApp>,
) {
    app.ext.ai.config.active_embedding = id.filter(|i| i != worker::TEST_ID);
    persist(app);
    if super::index_wanted(app) {
        runner::start_index(app, cx, None);
    } else {
        runner::stop_index(app);
    }
    cx.notify();
}

/// Which exclusion list a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Folder,
    Tag,
}

/// Adds the text of the folder or tag field to the exclusion list.
pub fn add_excluded(app: &mut RustdianApp, kind: ListKind, cx: &mut Context<RustdianApp>) {
    type Normaliser = fn(&str) -> Option<String>;
    let (input, raw, normalise): (&Option<Entity<LineInput>>, String, Normaliser) = match kind {
        ListKind::Folder => (
            &app.ext.ai.inputs.folder,
            input_text(&app.ext.ai.inputs.folder, cx),
            errors::normalize_folder,
        ),
        ListKind::Tag => (
            &app.ext.ai.inputs.tag,
            input_text(&app.ext.ai.inputs.tag, cx),
            errors::normalize_tag,
        ),
    };
    let input = input.clone();
    let Some(item) = normalise(&raw) else {
        app.toast(ToastKind::Warn, "请输入文件夹或标签名");
        return;
    };
    let list = match kind {
        ListKind::Folder => &mut app.ext.ai.config.excluded_folders,
        ListKind::Tag => &mut app.ext.ai.config.excluded_tags,
    };
    if errors::push_unique_ci(list, item) {
        persist(app);
        if let Some(i) = input {
            i.update(cx, |i, cx| i.set_text("", cx));
        }
        if super::index_wanted(app) {
            // Excluded notes leave the index on the next run.
            runner::start_index(app, cx, None);
        }
    } else {
        app.toast(ToastKind::Info, "已经在列表中");
    }
    cx.notify();
}

/// Removes one entry of an exclusion list.
pub fn remove_excluded(
    app: &mut RustdianApp,
    kind: ListKind,
    index: usize,
    cx: &mut Context<RustdianApp>,
) {
    let list = match kind {
        ListKind::Folder => &mut app.ext.ai.config.excluded_folders,
        ListKind::Tag => &mut app.ext.ai.config.excluded_tags,
    };
    if index < list.len() {
        list.remove(index);
        persist(app);
    }
    cx.notify();
}

/// 测试连接: a tiny request with the form's values (or the stored key) on a background thread.
pub fn test_connection(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(editing) = app.ext.ai.settings.editing.clone() else {
        return;
    };
    let form = form_from_inputs(app, editing.kind, cx);
    let id = editing
        .id
        .clone()
        .unwrap_or_else(|| "new-provider".to_string());
    let entry: ProviderEntry = match errors::validate_provider(&id, &form) {
        Ok(entry) => entry,
        Err(message) => {
            if let Some(e) = app.ext.ai.settings.editing.as_mut() {
                e.error = Some(message);
            }
            cx.notify();
            return;
        }
    };
    let typed = input_text(&app.ext.ai.inputs.api_key, cx)
        .trim()
        .to_string();
    let key = if typed.is_empty() { None } else { Some(typed) };
    let test = app.ext.ai.test.clone();
    app.ext.ai.settings.test = TestState::Running;
    cx.notify();
    runner::once(
        cx,
        move || {
            worker::test_connection(&entry, key, test.as_ref()).map_err(|e| errors::describe(&e))
        },
        |app, result, cx| {
            app.ext.ai.settings.test = match result {
                Ok(()) => TestState::Ok,
                Err(m) => TestState::Failed(m),
            };
            cx.notify();
        },
    );
}

/// The index button: deletes and rebuilds the vector index.
pub fn rebuild(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if !super::index_wanted(app) {
        app.toast(ToastKind::Warn, "请先选择一个嵌入服务");
        return;
    }
    runner::rebuild_index(app, cx);
    app.toast(ToastKind::Info, "开始重建向量索引");
}

fn text_field(t: &ActiveTheme, label: &str, input: Option<Entity<LineInput>>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(t.tx2)
                .child(label.to_string()),
        )
        .children(input)
}

fn choice_chip(
    t: &ActiveTheme,
    label: &str,
    on: bool,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let el = div()
        .flex()
        .items_center()
        .h(px(26.0))
        .px(px(10.0))
        .rounded(px(13.0))
        .border_1()
        .text_size(px(12.5))
        .cursor_pointer()
        .child(label.to_string())
        .on_mouse_down(MouseButton::Left, on_click);
    if on {
        el.bg(t.ac_soft).border_color(t.ac).text_color(t.ac)
    } else {
        el.border_color(t.line)
            .text_color(t.tx2)
            .hover(|s| s.bg(t.hover))
    }
}

fn status_text(app: &RustdianApp) -> (String, bool) {
    let s = &app.ext.ai;
    let cfg = effective_config(s);
    if !s.config.enabled {
        return ("AI 未开启".into(), false);
    }
    if cfg.embedding_provider().is_none() {
        return ("未选择嵌入服务：只使用关键词检索".into(), false);
    }
    if s.index.running.is_some() {
        return (
            format!("正在索引：{}/{}", s.index.done, s.index.total),
            false,
        );
    }
    if let Some(e) = &s.index.error {
        return (format!("索引出错：{e}"), true);
    }
    match s.index.stats {
        Some(stats) => (
            format!(
                "已索引 {} 篇笔记，{} 个分块，占用约 {} KB",
                stats.notes,
                stats.chunks,
                stats.bytes / 1024
            ),
            false,
        ),
        None => (
            "尚未建立索引；保存笔记后会自动更新（30 秒去抖）。".into(),
            false,
        ),
    }
}

/// The AI category of the settings dialog.
pub fn render(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let s = &app.ext.ai;
    let cfg = &s.config;
    let mut col = div().flex().flex_col().gap(px(2.0)).pb(px(14.0));

    col = col.child(form_row(
        &t,
        "启用 AI 功能",
        "默认关闭。开启时会先说明哪些内容会被发送。",
        toggle(
            &t,
            cfg.enabled,
            cx.listener(|this, _: &MouseDownEvent, _, cx| toggle_enabled(this, cx)),
        ),
        false,
    ));
    if let Some(err) = &s.config_error {
        col = col.child(
            div()
                .py(px(6.0))
                .text_size(px(12.0))
                .text_color(gpui::rgb(RED))
                .child(format!("AI 设置文件无法读取，已使用默认设置：{err}")),
        );
    }
    if let Some(n) = &s.settings.notice {
        col = col.child(
            div()
                .py(px(4.0))
                .text_size(px(12.0))
                .text_color(t.ok)
                .child(n.clone()),
        );
    }

    col = col.child(section_title(&t, "服务商"));
    if cfg.providers.is_empty() && s.test.is_none() {
        col = col.child(div().py(px(6.0)).text_color(t.tx3).child(
            "还没有服务商。支持 OpenAI 兼容接口与 Anthropic，密钥以明文保存在用户目录的 credentials.toml 中。",
        ));
    }
    for p in &cfg.providers {
        col = col.child(provider_row(app, p, cx));
    }
    if let Some(env) = &s.test {
        col = col.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .py(px(8.0))
                .text_size(px(13.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(format!("{}（环境变量，不保存）", worker::TEST_NAME))
                        .child(div().text_size(px(11.5)).text_color(t.tx3).child(format!(
                            "{} · {}",
                            host_of(&env.base),
                            env.model
                        ))),
                ),
        );
    }
    col = col.child(div().pt(px(8.0)).child(button(
        &t,
        "添加服务",
        ButtonKind::Secondary,
        Some("plus"),
        cx.listener(|this, _: &MouseDownEvent, _, cx| begin_edit(this, None, cx)),
    )));
    if s.settings.editing.is_some() {
        col = col.child(render_form(app, cx));
    }

    // Active providers.
    let chat_options: Vec<(String, String)> = effective_config(s)
        .providers
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let active_chat = effective_config(s).active_chat;
    col = col.child(section_title(&t, "对话服务"));
    let mut chat_row = div().flex().flex_wrap().gap(px(6.0));
    if chat_options.is_empty() {
        chat_row = chat_row.child(div().text_color(t.tx3).child("先添加服务"));
    }
    for (id, name) in &chat_options {
        let on = active_chat.as_deref() == Some(id.as_str());
        let pick = id.clone();
        chat_row = chat_row.child(choice_chip(
            &t,
            name,
            on,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                set_active_chat(this, Some(pick.clone()), cx);
            }),
        ));
    }
    col = col.child(chat_row);

    let embed_options: Vec<(String, String)> = effective_config(s)
        .providers
        .iter()
        .filter(|p| p.embedding_config().is_some())
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let active_embed = effective_config(s).active_embedding;
    col = col.child(section_title(&t, "嵌入服务（用于向量检索，可选）"));
    let mut embed_row = div().flex().flex_wrap().gap(px(6.0));
    embed_row = embed_row.child(choice_chip(
        &t,
        "不使用",
        active_embed.is_none(),
        cx.listener(|this, _: &MouseDownEvent, _, cx| set_active_embedding(this, None, cx)),
    ));
    for (id, name) in &embed_options {
        let on = active_embed.as_deref() == Some(id.as_str());
        let pick = id.clone();
        embed_row = embed_row.child(choice_chip(
            &t,
            name,
            on,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                set_active_embedding(this, Some(pick.clone()), cx);
            }),
        ));
    }
    col = col.child(embed_row);

    // Exclusions.
    col = col.child(section_title(&t, "排除的文件夹"));
    col = col.child(exclusion_list(app, ListKind::Folder, cx));
    col = col.child(section_title(&t, "排除的标签"));
    col = col.child(exclusion_list(app, ListKind::Tag, cx));
    col = col.child(
        div()
            .pt(px(4.0))
            .text_size(px(12.0))
            .text_color(t.tx3)
            .child("这些笔记不会发送给任何模型，也不会进入向量索引；它们的 AI 操作会被禁用。"),
    );

    // Index.
    col = col.child(section_title(&t, "向量索引"));
    let (status, is_error) = status_text(app);
    col = col.child(
        div()
            .py(px(4.0))
            .text_size(px(12.5))
            .text_color(if is_error { gpui::rgb(RED) } else { t.tx2 })
            .child(status),
    );
    col = col.child(
        div()
            .pt(px(2.0))
            .text_size(px(12.0))
            .text_color(t.tx3)
            .child("每个笔记按约 800 字切块，每块发送一次嵌入请求，按服务商的价格计费。索引保存在 .jiyue/index，可随时重建。"),
    );
    col = col.child(div().pt(px(6.0)).child(button(
        &t,
        "重建向量索引",
        ButtonKind::Secondary,
        Some("link"),
        cx.listener(|this, _: &MouseDownEvent, _, cx| rebuild(this, cx)),
    )));

    // Privacy.
    col = col.child(section_title(&t, "隐私"));
    col = col.child(div().text_size(px(12.5)).text_color(t.tx2).child(
        "发送前一定会显示预览。API 密钥只存放在用户目录的 credentials.toml，不写入笔记库或日志。",
    ));

    div()
        .id("ai-settings-scroll")
        .h(px(SCROLL_HEIGHT))
        .overflow_y_scroll()
        .pr(px(6.0))
        .font_family(crate::ui::fonts::ui_name())
        .child(col)
        .into_any_element()
}

fn provider_row(app: &RustdianApp, p: &ProviderEntry, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let key_text = match app.ext.ai.settings.key_saved.get(&p.id) {
        Some(true) => "已保存到 credentials.toml",
        Some(false) => "未设置密钥",
        None => "正在读取密钥状态…",
    };
    let kind = match p.kind {
        ProviderKind::OpenAiCompatible => "OpenAI 兼容",
        ProviderKind::Anthropic => "Anthropic",
    };
    let edit_id = p.id.clone();
    let delete_id = p.id.clone();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(2.0))
                .child(div().font_weight(FontWeight::MEDIUM).child(p.name.clone()))
                .child(div().text_size(px(11.5)).text_color(t.tx3).child(format!(
                    "{kind} · {} · {}",
                    host_of(&p.base_url),
                    p.chat_model
                )))
                .child(div().text_size(px(11.5)).text_color(t.tx3).child(key_text)),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(6.0))
                .child(button(
                    &t,
                    "编辑",
                    ButtonKind::Ghost,
                    None,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        begin_edit(this, Some(edit_id.clone()), cx);
                    }),
                ))
                .child(icon_button(
                    &t,
                    "trash",
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        this.ext.ai.modal = Modal::DeleteProvider(delete_id.clone());
                        cx.notify();
                    }),
                )),
        )
}

fn render_form(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let s = &app.ext.ai;
    let Some(editing) = s.settings.editing.as_ref() else {
        return div();
    };
    let inputs = &s.inputs;
    let kind_index = match editing.kind {
        ProviderKind::OpenAiCompatible => 0,
        ProviderKind::Anthropic => 1,
    };
    let kind_pick: PickIndex = Rc::new(cx.listener(|this, i: &usize, _window, cx| {
        if let Some(e) = this.ext.ai.settings.editing.as_mut() {
            e.kind = if *i == 1 {
                ProviderKind::Anthropic
            } else {
                ProviderKind::OpenAiCompatible
            };
        }
        cx.notify();
    }));
    let saved = editing
        .id
        .as_ref()
        .and_then(|id| s.settings.key_saved.get(id).copied())
        .unwrap_or(false);
    let key_display = editing
        .id
        .as_ref()
        .and_then(|id| s.settings.key_display.get(id).cloned())
        .unwrap_or_default();
    let clear_id = editing.id.clone().unwrap_or_default();
    let key_hint = if saved {
        "已保存到 credentials.toml（留空则保留原密钥）"
    } else {
        "输入后保存到 credentials.toml（明文）"
    };
    let test_line = match &s.settings.test {
        TestState::Idle => None,
        TestState::Running => Some(("正在测试连接…".to_string(), t.tx3)),
        TestState::Ok => Some(("连接正常".to_string(), t.ok)),
        TestState::Failed(m) => Some((m.clone(), gpui::rgb(RED))),
    };
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .mt(px(8.0))
        .p(px(12.0))
        .rounded(px(10.0))
        .bg(t.sheet)
        .border_1()
        .border_color(t.line)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(if editing.id.is_some() {
                    "编辑服务"
                } else {
                    "添加服务"
                }),
        )
        .child(text_field(&t, "名称", inputs.name.clone()))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(t.tx2)
                        .child("接口类型"),
                )
                .child(segmented(
                    &t,
                    &["OpenAI 兼容", "Anthropic"],
                    kind_index,
                    kind_pick,
                )),
        )
        .child(text_field(&t, "服务地址（Base URL）", inputs.url.clone()))
        .child(text_field(&t, "对话模型", inputs.chat_model.clone()))
        .child(text_field(
            &t,
            "嵌入模型（可留空）",
            inputs.embed_model.clone(),
        ))
        .child(crate::ui::config_ui::secret_control(
            &t,
            &key_display,
            text_field(&t, "API 密钥", inputs.api_key.clone()).into_any_element(),
            cx.listener(move |this, _: &gpui::MouseDownEvent, _, cx| {
                clear_key(this, &clear_id, cx)
            }),
        ))
        .child(crate::ui::config_ui::credentials_help(&t))
        .child(div().text_size(px(11.5)).text_color(t.tx3).child(key_hint));
    if let Some((text, color)) = test_line {
        col = col.child(div().text_size(px(12.5)).text_color(color).child(text));
    }
    if let Some(err) = &editing.error {
        col = col.child(
            div()
                .text_size(px(12.5))
                .text_color(gpui::rgb(RED))
                .child(err.clone()),
        );
    }
    col.child(
        div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(px(8.0))
            .child(button(
                &t,
                "取消",
                ButtonKind::Ghost,
                None,
                cx.listener(|this, _: &MouseDownEvent, _, cx| cancel_edit(this, cx)),
            ))
            .child(button(
                &t,
                "测试连接",
                ButtonKind::Secondary,
                Some("link"),
                cx.listener(|this, _: &MouseDownEvent, _, cx| test_connection(this, cx)),
            ))
            .child(button(
                &t,
                "保存",
                ButtonKind::Primary,
                Some("check"),
                cx.listener(|this, _: &MouseDownEvent, _, cx| save_edit(this, cx)),
            )),
    )
}

fn exclusion_list(app: &RustdianApp, kind: ListKind, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let s = &app.ext.ai;
    let (items, input, placeholder_hint) = match kind {
        ListKind::Folder => (
            &s.config.excluded_folders,
            s.inputs.folder.clone(),
            "例如 私人/日记",
        ),
        ListKind::Tag => (&s.config.excluded_tags, s.inputs.tag.clone(), "例如 secret"),
    };
    let mut chips = div().flex().flex_wrap().gap(px(6.0));
    if items.is_empty() {
        chips = chips.child(div().text_size(px(12.0)).text_color(t.tx3).child("（无）"));
    }
    for (i, item) in items.iter().enumerate() {
        chips = chips.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .h(px(24.0))
                .pl(px(10.0))
                .pr(px(4.0))
                .rounded(px(12.0))
                .bg(t.sheet)
                .border_1()
                .border_color(t.line_soft)
                .text_size(px(12.0))
                .child(match kind {
                    ListKind::Tag => format!("#{item}"),
                    ListKind::Folder => item.clone(),
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.0))
                        .rounded_full()
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .child(icon("x", 11.0, t.tx3))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                remove_excluded(this, kind, i, cx);
                            }),
                        ),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(chips)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().flex_1().min_w_0().children(input))
                .child(button(
                    &t,
                    "添加",
                    ButtonKind::Secondary,
                    Some("plus"),
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        add_excluded(this, kind, cx);
                    }),
                )),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.tx3)
                .child(format!("{placeholder_hint}（文件夹相对于笔记库根目录）")),
        )
}

/// Index state label for tests and the status bar.
pub fn index_label(done: usize, total: usize) -> String {
    indexer::progress_label(done, total)
}

/// Removes the stored key of a provider (a literal or a reference) and refreshes the form.
pub fn clear_key(app: &mut RustdianApp, id: &str, cx: &mut Context<RustdianApp>) {
    app.ext.ai.settings.key_saved.remove(id);
    app.ext.ai.settings.key_display.remove(id);
    let key_id = id.to_string();
    runner::once(
        cx,
        move || {
            credentials::write_field(&credentials::path(), AI_TABLE, &key_id, "api_key", None)
                .map(|o| o.permissions_ok)
        },
        |app, result, cx| {
            match result {
                Ok(ok) => {
                    crate::ui::config_ui::check_permissions(app, ok);
                    app.toast(ToastKind::Info, "已清除 API 密钥（credentials.toml）");
                }
                Err(m) => app.toast(ToastKind::Error, format!("无法清除 API 密钥：{m}")),
            }
            cx.notify();
        },
    );
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiyue_ai::AiConfig;

    #[test]
    fn default_config_keeps_ai_off() {
        let cfg = AiConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.chat_provider().is_none());
    }

    #[test]
    fn test_state_starts_idle() {
        assert_eq!(TestState::default(), TestState::Idle);
    }

    #[test]
    fn index_label_matches_status_bar() {
        assert_eq!(index_label(2, 5), "AI 索引 2/5");
    }
}
