//! The 同步 page of the settings dialog: backend, connection fields, automatic sync, device name,
//! ignore rules, connection test, and the danger zone.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Entity, FontWeight, IntoElement, SharedString, div, prelude::*, px,
};
use jiyue_sync::store::ConnectionReport;

use super::config::{
    Backend, DEBOUNCE_CHOICES, DEBOUNCE_LABELS, Interval, SyncConfig, debounce_index,
    normalize_endpoint, normalize_prefix, parse_ignore_text, test_override,
};
use super::secrets;
use super::service;
use super::{Confirm, new_vault_id};
use crate::RustdianApp;
use crate::jobs::spawn_blocking;
use crate::theme::ActiveTheme;
use crate::ui::text_input::TextEditor;
use crate::ui::widgets::line_input::LineInput;
use crate::ui::widgets::{
    BadgeKind, ButtonKind, badge, button, form_row, section_title, segmented, toggle,
};

/// Entities of the form, created once in `init`. Cheap to clone.
#[derive(Clone)]
pub struct Inputs {
    pub s3_endpoint: Entity<LineInput>,
    pub s3_region: Entity<LineInput>,
    pub s3_bucket: Entity<LineInput>,
    pub s3_prefix: Entity<LineInput>,
    pub s3_key: Entity<LineInput>,
    pub s3_secret: Entity<LineInput>,
    pub dav_endpoint: Entity<LineInput>,
    pub dav_user: Entity<LineInput>,
    pub dav_password: Entity<LineInput>,
    pub dav_root: Entity<LineInput>,
    pub device: Entity<LineInput>,
    pub ignore: Entity<TextEditor>,
}

impl Inputs {
    pub fn new(theme: ActiveTheme, cx: &mut Context<RustdianApp>) -> Self {
        let line = |cx: &mut Context<RustdianApp>, placeholder: &'static str, secret: bool| {
            cx.new(|cx| {
                let mut input = LineInput::new(cx, placeholder);
                input.set_theme(theme, cx);
                if secret {
                    input.set_password(true, cx);
                }
                input
            })
        };
        Self {
            s3_endpoint: line(cx, "https://s3.example.com", false),
            s3_region: line(cx, "us-east-1", false),
            s3_bucket: line(cx, "存储桶名称", false),
            s3_prefix: line(cx, "jiyue/我的笔记/", false),
            s3_key: line(cx, "Access Key ID", false),
            s3_secret: line(cx, "Secret Access Key", true),
            dav_endpoint: line(cx, "https://dav.example.com/dav", false),
            dav_user: line(cx, "用户名", false),
            dav_password: line(cx, "密码", true),
            dav_root: line(cx, "jiyue/我的笔记/", false),
            device: line(cx, "本机名称", false),
            ignore: cx.new(|cx| TextEditor::new("", theme, cx)),
        }
    }
}

/// Result of the connection test shown in the settings page.
#[derive(Clone, Debug, Default)]
pub enum TestState {
    #[default]
    Idle,
    Running,
    Done(ConnectionReport),
    Failed(String),
}

/// Writes the saved values into the form entities (also the placeholder of the secret field).
pub fn fill_inputs(app: &RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(inputs) = app.ext.sync.inputs.clone() else {
        return;
    };
    let d = &app.ext.sync.draft;
    let display = app.ext.sync.secret_display.clone();
    let set = |entity: &Entity<LineInput>, text: &str, cx: &mut Context<RustdianApp>| {
        entity.update(cx, |input, cx| input.set_text(text, cx));
    };
    set(&inputs.s3_endpoint, &d.s3_endpoint, cx);
    set(&inputs.s3_region, &d.s3_region, cx);
    set(&inputs.s3_bucket, &d.s3_bucket, cx);
    set(&inputs.s3_prefix, &d.s3_root_prefix, cx);
    set(&inputs.s3_key, &d.s3_access_key_id, cx);
    set(&inputs.dav_endpoint, &d.webdav_endpoint, cx);
    set(&inputs.dav_user, &d.webdav_username, cx);
    set(&inputs.dav_root, &d.webdav_root, cx);
    set(&inputs.device, &d.device_name, cx);
    let ignore_text = d.ignore_patterns.join("\n");
    inputs
        .ignore
        .update(cx, |editor, cx| editor.set_text(&ignore_text, cx));
    for secret in [&inputs.s3_secret, &inputs.dav_password] {
        secret.update(cx, |input, cx| {
            input.set_text("", cx);
            let placeholder = match &display {
                crate::ui::config_ui::SecretDisplay::Literal => {
                    format!("{}（已保存，明文）", crate::ui::config_ui::MASKED)
                }
                crate::ui::config_ui::SecretDisplay::None => "未保存".to_string(),
                _ => secrets::STORED_PLACEHOLDER.to_string(),
            };
            input.set_placeholder(&placeholder, cx);
        });
    }
}

/// Removes the vault's stored sync secret and refreshes the form.
pub fn clear_secret(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let cfg = app.ext.sync.saved.clone();
    match secrets::delete(&cfg) {
        Ok(ok) => {
            crate::ui::config_ui::check_permissions(app, ok);
            app.toast(
                crate::ui::ext::ToastKind::Info,
                "已清除同步密钥（credentials.toml）",
            );
        }
        Err(e) => app.toast(
            crate::ui::ext::ToastKind::Error,
            format!("无法清除同步密钥：{e}"),
        ),
    }
    app.ext.sync.secret_stored = secrets::exists(&cfg);
    app.ext.sync.secret_display = secrets::display(&cfg);
    fill_inputs(app, cx);
    cx.notify();
}

/// Applies the theme to the form entities (after a theme change).
pub fn set_theme(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let theme = app.theme;
    let Some(inputs) = app.ext.sync.inputs.clone() else {
        return;
    };
    for input in [
        &inputs.s3_endpoint,
        &inputs.s3_region,
        &inputs.s3_bucket,
        &inputs.s3_prefix,
        &inputs.s3_key,
        &inputs.s3_secret,
        &inputs.dav_endpoint,
        &inputs.dav_user,
        &inputs.dav_password,
        &inputs.dav_root,
        &inputs.device,
    ] {
        input.update(cx, |i, cx| i.set_theme(theme, cx));
    }
    inputs
        .ignore
        .update(cx, |editor, cx| editor.set_theme(theme, cx));
}

/// The form as the user sees it now: the draft with every text field read, normalised, and the
/// typed secret (empty when nothing was typed).
pub fn read_form(app: &RustdianApp, cx: &App) -> Option<(SyncConfig, String)> {
    let inputs = app.ext.sync.inputs.as_ref()?;
    let mut cfg = app.ext.sync.draft.clone();
    cfg.s3_endpoint = normalize_endpoint(&inputs.s3_endpoint.read(cx).text());
    cfg.s3_region = inputs.s3_region.read(cx).text().trim().to_owned();
    cfg.s3_bucket = inputs.s3_bucket.read(cx).text().trim().to_owned();
    cfg.s3_root_prefix = normalize_prefix(&inputs.s3_prefix.read(cx).text());
    cfg.s3_access_key_id = inputs.s3_key.read(cx).text().trim().to_owned();
    cfg.webdav_endpoint = normalize_endpoint(&inputs.dav_endpoint.read(cx).text());
    cfg.webdav_username = inputs.dav_user.read(cx).text().trim().to_owned();
    cfg.webdav_root = normalize_prefix(&inputs.dav_root.read(cx).text());
    cfg.device_name = inputs.device.read(cx).text().trim().to_owned();
    cfg.ignore_patterns = parse_ignore_text(&inputs.ignore.read(cx).text());
    let typed = match cfg.backend {
        Backend::S3 => inputs.s3_secret.read(cx).text(),
        Backend::WebDav => inputs.dav_password.read(cx).text(),
    };
    Some((cfg, typed))
}

/// Saves the form: validates when the sync is on, writes the secret to the credential manager and
/// the settings file. A disabled sync may be saved incomplete.
pub fn save(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some((cfg, typed)) = read_form(app, cx) else {
        return;
    };
    let root = app.vault.root_dir.clone();
    let folder = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vault")
        .to_owned();
    let cfg = cfg.with_defaults(&folder, new_vault_id);
    let has_secret = !typed.is_empty() || secrets::exists(&cfg);
    let mut note: Option<String> = None;
    if let Err(e) = cfg.validate(has_secret) {
        if cfg.enabled {
            app.ext.sync.settings_msg = Some((false, e));
            cx.notify();
            return;
        }
        note = Some(format!("已保存，未启用同步。{e}"));
    }
    if !typed.is_empty() {
        match secrets::save(&cfg, &typed) {
            Ok(ok) => crate::ui::config_ui::check_permissions(app, ok),
            Err(e) => {
                app.ext.sync.settings_msg = Some((false, e));
                cx.notify();
                return;
            }
        }
    }
    match secrets::save_identity(&cfg) {
        Ok(ok) => crate::ui::config_ui::check_permissions(app, ok),
        Err(e) => {
            app.ext.sync.settings_msg = Some((false, e));
            cx.notify();
            return;
        }
    }
    if let Err(e) = cfg.save(&root) {
        app.ext.sync.settings_msg = Some((false, format!("无法保存同步设置：{e}")));
        cx.notify();
        return;
    }
    let stored = secrets::exists(&cfg);
    let display = secrets::display(&cfg);
    let s = &mut app.ext.sync;
    s.secret_display = display;
    s.saved = cfg.clone();
    s.draft = cfg;
    s.secret_stored = stored;
    s.config_error = None;
    s.settings_msg = Some(match note {
        Some(text) => (true, text),
        None => (true, "已保存".into()),
    });
    fill_inputs(app, cx);
    cx.notify();
}

/// Connection test on a background thread. Uses the typed secret, or the stored one.
pub fn test_connection(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some((cfg, typed)) = read_form(app, cx) else {
        return;
    };
    let over = test_override();
    if over.is_none() {
        let has_secret = !typed.is_empty() || secrets::exists(&cfg);
        if let Err(e) = cfg.validate(has_secret) {
            app.ext.sync.test = TestState::Failed(e);
            cx.notify();
            return;
        }
    }
    app.ext.sync.test = TestState::Running;
    cx.notify();
    spawn_blocking(
        cx,
        move || check_remote(cfg, typed),
        |app, result, cx| {
            app.ext.sync.test = match result {
                Ok(report) => TestState::Done(report),
                Err(e) => TestState::Failed(e),
            };
            cx.notify();
        },
    );
}

/// Blocking part of the connection test (runs on a background thread).
fn check_remote(cfg: SyncConfig, typed: String) -> Result<ConnectionReport, String> {
    Ok(service::open_store(&cfg, &typed)?.check_connection())
}

/// The 同步 category body.
pub fn render_settings_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let s = &app.ext.sync;
    let Some(inputs) = s.inputs.clone() else {
        return div().into_any_element();
    };
    let d = &s.draft;
    let s3 = d.backend == Backend::S3;

    let backend = segmented(
        &t,
        &Backend::LABELS,
        d.backend.index(),
        Rc::new(cx.listener(|this, i: &usize, _, cx| {
            this.ext.sync.draft.backend = Backend::from_index(*i);
            cx.notify();
        })),
    );

    let mut body = div()
        .id("sync_settings")
        .flex()
        .flex_col()
        .h(px(440.0))
        .pr(px(10.0))
        .overflow_y_scroll()
        .font_family(crate::ui::fonts::ui_name())
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .py(px(8.0))
                .child(section_title(&t, "存储后端"))
                .child(backend),
        );

    if let Some(err) = &s.config_error {
        body = body.child(note_line(&t, false, err));
    }

    body = body.child(section_title(&t, "连接"));
    if s3 {
        body = body
            .child(form_row(
                &t,
                "服务器地址",
                "http:// 或 https:// 开头，RustFS / MinIO 填写其地址",
                field(&t, &inputs.s3_endpoint),
                false,
            ))
            .child(form_row(
                &t,
                "区域",
                "兼容服务可填 us-east-1",
                field(&t, &inputs.s3_region),
                false,
            ))
            .child(form_row(
                &t,
                "存储桶",
                "需要事先在服务器上创建",
                field(&t, &inputs.s3_bucket),
                false,
            ))
            .child(form_row(
                &t,
                "路径前缀",
                "桶内保存本库的文件夹，留空则为 jiyue/<库名>/",
                field(&t, &inputs.s3_prefix),
                false,
            ))
            .child(form_row(
                &t,
                "Access Key ID",
                "访问密钥 ID",
                field(&t, &inputs.s3_key),
                false,
            ))
            .child(form_row(
                &t,
                "Secret Access Key",
                "明文保存在 credentials.toml 中，不写入笔记库",
                crate::ui::config_ui::secret_control(
                    &t,
                    &s.secret_display,
                    field(&t, &inputs.s3_secret).into_any_element(),
                    cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| clear_secret(this, cx)),
                ),
                false,
            ))
            .child(form_row(
                &t,
                "路径风格寻址",
                "RustFS、MinIO 等自建服务通常需要开启",
                toggle(
                    &t,
                    d.s3_path_style,
                    cx.listener(|this, _, _, cx| {
                        this.ext.sync.draft.s3_path_style = !this.ext.sync.draft.s3_path_style;
                        cx.notify();
                    }),
                ),
                true,
            ));
    } else {
        body = body
            .child(form_row(
                &t,
                "服务器地址",
                "WebDAV 根地址，如 https://dav.example.com/dav",
                field(&t, &inputs.dav_endpoint),
                false,
            ))
            .child(form_row(
                &t,
                "用户名",
                "WebDAV 账户",
                field(&t, &inputs.dav_user),
                false,
            ))
            .child(form_row(
                &t,
                "密码",
                "明文保存在 credentials.toml 中，不写入笔记库",
                crate::ui::config_ui::secret_control(
                    &t,
                    &s.secret_display,
                    field(&t, &inputs.dav_password).into_any_element(),
                    cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| clear_secret(this, cx)),
                ),
                false,
            ))
            .child(form_row(
                &t,
                "远端目录",
                "保存本库的文件夹",
                field(&t, &inputs.dav_root),
                true,
            ));
    }

    body = body.child(section_title(&t, "自动同步"));
    body = body
        .child(form_row(
            &t,
            "启用同步",
            "关闭时不访问远端；修改后需要点击保存设置",
            toggle(
                &t,
                d.enabled,
                cx.listener(|this, _, _, cx| {
                    this.ext.sync.draft.enabled = !this.ext.sync.draft.enabled;
                    cx.notify();
                }),
            ),
            false,
        ))
        .child(form_row(
            &t,
            "定时同步",
            "按间隔自动同步",
            toggle(
                &t,
                d.auto_sync,
                cx.listener(|this, _, _, cx| {
                    this.ext.sync.draft.auto_sync = !this.ext.sync.draft.auto_sync;
                    cx.notify();
                }),
            ),
            false,
        ))
        .child(form_row(
            &t,
            "同步间隔",
            "定时同步时使用",
            segmented(
                &t,
                &Interval::LABELS,
                d.interval.index(),
                Rc::new(cx.listener(|this, i: &usize, _, cx| {
                    this.ext.sync.draft.interval = Interval::from_index(*i);
                    cx.notify();
                })),
            ),
            false,
        ))
        .child(form_row(
            &t,
            "保存后同步",
            "笔记保存后等待一段时间再同步",
            toggle(
                &t,
                d.sync_on_save,
                cx.listener(|this, _, _, cx| {
                    this.ext.sync.draft.sync_on_save = !this.ext.sync.draft.sync_on_save;
                    cx.notify();
                }),
            ),
            false,
        ))
        .child(form_row(
            &t,
            "等待时间",
            "最后一次保存之后的等待时间",
            segmented(
                &t,
                &DEBOUNCE_LABELS,
                debounce_index(d.save_debounce_secs),
                Rc::new(cx.listener(|this, i: &usize, _, cx| {
                    let secs = DEBOUNCE_CHOICES.get(*i).copied().unwrap_or(10);
                    this.ext.sync.draft.save_debounce_secs = secs;
                    cx.notify();
                })),
            ),
            false,
        ))
        .child(form_row(
            &t,
            "启动时同步",
            "打开笔记库时同步一次",
            toggle(
                &t,
                d.sync_on_start,
                cx.listener(|this, _, _, cx| {
                    this.ext.sync.draft.sync_on_start = !this.ext.sync.draft.sync_on_start;
                    cx.notify();
                }),
            ),
            false,
        ))
        .child(form_row(
            &t,
            "退出时同步",
            "关闭窗口前同步一次，最多等待 8 秒",
            toggle(
                &t,
                d.sync_on_exit,
                cx.listener(|this, _, _, cx| {
                    this.ext.sync.draft.sync_on_exit = !this.ext.sync.draft.sync_on_exit;
                    cx.notify();
                }),
            ),
            false,
        ))
        .child(form_row(
            &t,
            "本机名称",
            "冲突副本文件名中的设备标识",
            field(&t, &inputs.device),
            true,
        ));

    body = body.child(section_title(&t, "忽略规则"));
    body = body.child(
        div()
            .pb(px(10.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(t.tx3)
                    .pb(px(6.0))
                    .child("每行一条，如 *.tmp、drafts/，与内置规则合并"),
            )
            .child(
                div()
                    .h(px(96.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(t.line)
                    .overflow_hidden()
                    .child(inputs.ignore.clone()),
            ),
    );

    let (msg_ok, msg) = match &s.settings_msg {
        Some((ok, text)) => (*ok, Some(text.clone())),
        None => (true, None),
    };
    let last = last_line(app);
    body = body.child(
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .pt(px(6.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(t.tx3)
                    .child(SharedString::from(last)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(button(
                        &t,
                        "测试连接",
                        ButtonKind::Secondary,
                        None,
                        cx.listener(|this, _, _, cx| {
                            test_connection(this, cx);
                        }),
                    ))
                    .child(button(
                        &t,
                        "立即同步",
                        ButtonKind::Secondary,
                        None,
                        cx.listener(|this, _, _, cx| {
                            super::open_modal(this, super::Modal::Sync);
                            super::start_run(this, super::service::RunKind::Manual, cx);
                            cx.notify();
                        }),
                    ))
                    .child(button(
                        &t,
                        "保存设置",
                        ButtonKind::Primary,
                        None,
                        cx.listener(|this, _, _, cx| {
                            save(this, cx);
                        }),
                    )),
            )
            .children(msg.map(|text| note_line(&t, msg_ok, &text))),
    );

    if let Some(card) = test_card(&t, &s.test) {
        body = body.child(card);
    }

    body = body.child(section_title(&t, "危险区"));
    body = body.child(form_row(
        &t,
        "重置同步状态",
        "删除 .jiyue/sync/（基准记录、冲突列表、设备标识）。下次同步会重新比对本地与远端，可能产生冲突副本。",
        button(&t, "重置", ButtonKind::Danger, Some("trash"), cx.listener(|this, _, _, cx| {
            this.ext.sync.confirm = Some(Confirm::Reset);
            cx.notify();
        })),
        true,
    ));

    body.into_any_element()
}

/// Line of text under the settings buttons. Success is quiet, errors use the error colour.
fn note_line(t: &ActiveTheme, ok: bool, text: &str) -> AnyElement {
    div()
        .text_size(px(12.5))
        .text_color(if ok { t.tx2 } else { t.st })
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

/// One text field in a form row (30px high).
fn field(t: &ActiveTheme, input: &Entity<LineInput>) -> impl IntoElement + use<> {
    div()
        .w(px(300.0))
        .h(px(30.0))
        .flex_none()
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .overflow_hidden()
        .child(input.clone())
}

/// "Last sync" summary above the buttons.
fn last_line(app: &RustdianApp) -> String {
    let s = &app.ext.sync;
    if !s.saved.enabled {
        return "同步未启用".into();
    }
    match (s.last_ok_ms, s.service.last.as_ref()) {
        (Some(ms), Some(last)) => format!(
            "上次同步：{} · {}",
            super::schedule::relative_zh(jiyue_sync::state::now_ms(), ms),
            super::schedule::report_summary(&last.report)
        ),
        (Some(ms), None) => format!(
            "上次同步：{}",
            super::schedule::relative_zh(jiyue_sync::state::now_ms(), ms)
        ),
        _ => "尚未同步".into(),
    }
}

/// Report card for the connection test, or `None` before the first test.
fn test_card(t: &ActiveTheme, test: &TestState) -> Option<AnyElement> {
    let report = match test {
        TestState::Idle => return None,
        TestState::Running => {
            return Some(
                div()
                    .text_size(px(12.5))
                    .text_color(t.tx2)
                    .child("正在测试连接…")
                    .into_any_element(),
            );
        }
        TestState::Failed(e) => {
            return Some(note_line(t, false, &format!("连接测试失败：{e}")));
        }
        TestState::Done(report) => report,
    };
    let row = |label: &str, value: &str, kind: BadgeKind| {
        div()
            .flex()
            .items_center()
            .justify_between()
            .py(px(4.0))
            .border_b_1()
            .border_color(t.line_soft)
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(t.tx)
                    .child(SharedString::from(label.to_owned())),
            )
            .child(badge(t, value, kind))
    };
    let mut card = div()
        .mt(px(8.0))
        .p(px(12.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .flex()
        .flex_col()
        .font_weight(FontWeight::NORMAL)
        .child(row(
            "可连接",
            yes_no(report.reachable),
            ok_kind(report.reachable),
        ))
        .child(row(
            "可列出",
            yes_no(report.can_list),
            ok_kind(report.can_list),
        ))
        .child(row(
            "可写入",
            yes_no(report.can_write),
            ok_kind(report.can_write),
        ))
        .child(row(
            "可删除",
            yes_no(report.can_delete),
            ok_kind(report.can_delete),
        ))
        .child(row(
            "版本控制",
            if report.supports_versions {
                "已开启"
            } else {
                "未开启"
            },
            ok_kind(report.supports_versions),
        ))
        .child(row(
            "条件写入",
            if report.conditional_put_supported {
                "支持"
            } else {
                "不支持"
            },
            ok_kind(report.conditional_put_supported),
        ));
    if !report.supports_versions {
        card = card.child(
            div()
                .pt(px(6.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child("在桶上开启版本控制后，才能恢复历史版本。应用不会修改桶的设置。"),
        );
    }
    if !report.conditional_put_supported {
        card = card.child(
            div()
                .pt(px(4.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child("服务器不支持条件写入，两台设备同时修改时可能互相覆盖。"),
        );
    }
    if !report.message.is_empty() {
        card = card.child(
            div()
                .pt(px(4.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child(SharedString::from(report.message.clone())),
        );
    }
    Some(card.into_any_element())
}

fn yes_no(v: bool) -> &'static str {
    if v { "是" } else { "否" }
}

fn ok_kind(v: bool) -> BadgeKind {
    if v { BadgeKind::Ok } else { BadgeKind::Warn }
}
