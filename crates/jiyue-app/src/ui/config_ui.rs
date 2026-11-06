//! Configuration UI: the banner that names a broken configuration file, the reset that puts a
//! default file in its place, and the help line and buttons of the credentials file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{AnyElement, App, Context, Div, MouseDownEvent, Window, div, prelude::*, px};

use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::ext::{self, ToastKind};
use crate::ui::widgets::{ButtonKind, button};
use jiyue_core::config::secret::{SecretSource, source_of};
use jiyue_core::config::{self, ConfigIssue, credentials, templates};

/// Shown in place of a literal secret: the value itself is never displayed.
pub const MASKED: &str = "••••••••";

/// How a stored secret is given, as the settings page shows it. Never holds the value.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SecretDisplay {
    /// Nothing stored.
    #[default]
    None,
    /// A literal value in the file: shown masked.
    Literal,
    /// `env:NAME`: read-only, shows the variable name.
    Env(String),
    /// `cmd:...`: read-only.
    Command,
}

/// The display for the raw value stored in `credentials.toml` (`None` when nothing is stored).
pub fn display_of(stored: Option<&str>) -> SecretDisplay {
    match stored.map(str::trim).filter(|v| !v.is_empty()) {
        None => SecretDisplay::None,
        Some(raw) => match source_of(raw) {
            SecretSource::Literal => SecretDisplay::Literal,
            SecretSource::Env(name) => SecretDisplay::Env(name),
            SecretSource::Command => SecretDisplay::Command,
        },
    }
}

/// The read-only label of an `env:` or `cmd:` secret (`None` for literals and nothing).
pub fn status_label(display: &SecretDisplay) -> Option<String> {
    match display {
        SecretDisplay::Env(name) => Some(format!("来自环境变量 {name}")),
        SecretDisplay::Command => Some("来自命令".to_string()),
        SecretDisplay::None | SecretDisplay::Literal => None,
    }
}

/// Set when the owner-only permissions of `credentials.toml` could not be applied (icacls failed).
static PERMISSION_WARNING: AtomicBool = AtomicBool::new(false);

/// Records the outcome of a credentials write. Returns `true` only the first time the warning
/// appears, so the caller shows the toast once; the help line keeps the hint afterwards.
pub fn note_permissions(ok: bool) -> bool {
    if ok {
        return false;
    }
    !PERMISSION_WARNING.swap(true, Ordering::Relaxed)
}

/// The hint shown under the credentials help line when the permissions could not be set.
pub fn permission_hint() -> Option<&'static str> {
    PERMISSION_WARNING.load(Ordering::Relaxed).then_some(
        "未能设置仅当前用户可读的权限（icacls 失败），请在资源管理器中手动检查该文件的访问权限。",
    )
}

/// Tells the user once that `credentials.toml` could not be made owner-only.
pub fn check_permissions(app: &mut crate::RustdianApp, ok: bool) {
    if note_permissions(ok) {
        app.toast(
            ToastKind::Warn,
            "凭据已保存，但未能设置仅当前用户可读的权限（icacls 失败）。请在资源管理器中检查 credentials.toml。",
        );
    }
}

/// The banner text of one broken file: the problem with its position.
pub fn banner_message(issue: &ConfigIssue) -> String {
    format!(
        "配置文件有错误，程序正在使用默认值运行：{issue}。修正文件后会自动重新载入；文件未被覆盖。"
    )
}

/// The control of a secret: a read-only status row for `env:` and `cmd:` values, a masked row
/// above the editable field for literals, and the field alone when nothing is stored.
pub fn secret_control(
    t: &ActiveTheme,
    display: &SecretDisplay,
    editable: AnyElement,
    clear: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let clear_button = || button(t, "清除", ButtonKind::Secondary, None, clear);
    match display {
        SecretDisplay::None => editable,
        SecretDisplay::Literal => div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(t.tx2)
                            .child(format!("{MASKED}（已保存，明文）")),
                    )
                    .child(clear_button()),
            )
            .child(editable)
            .into_any_element(),
        SecretDisplay::Env(_) | SecretDisplay::Command => div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(32.0))
            .px(px(10.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(t.line)
            .bg(t.raised)
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(t.tx2)
                    .child(status_label(display).unwrap_or_default()),
            )
            .child(clear_button())
            .into_any_element(),
    }
}

/// The default text that replaces a broken file of this kind.
pub fn template_for(path: &Path, user_dir: Option<&Path>) -> &'static str {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let user_level = user_dir.is_some_and(|d| path.parent() == Some(d));
    match name {
        n if n == config::CREDENTIALS_FILE => templates::CREDENTIALS,
        n if n == config::USER_STATE_FILE && user_level => templates::USER_STATE,
        n if n == config::USER_CONFIG_FILE && user_level => templates::USER_CONFIG,
        n if n == config::VAULT_STATE_FILE => templates::VAULT_STATE,
        n if n == config::BOOKMARKS_FILE => templates::BOOKMARKS,
        _ => templates::VAULT_CONFIG,
    }
}

/// One line per broken file (at most two): the problem with its position, and the two actions.
/// Empty when every file loads.
pub fn banner(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let t = app.theme;
    let user_dir = app.configs.user_dir().map(Path::to_path_buf);
    let mut out = Vec::new();
    for issue in app.configs.broken().into_iter().take(2) {
        let file = issue.file.clone();
        let template = template_for(&file, user_dir.as_deref());
        let open_file = file.clone();
        let reset = cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            this.reset_config_file(&file, template, cx);
        });
        out.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .p(px(12.0))
                .mb(px(12.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(t.ac_line)
                .bg(t.ac_soft)
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(t.tx)
                        .child(banner_message(issue)),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(button(
                            &t,
                            "打开配置文件",
                            ButtonKind::Secondary,
                            None,
                            move |_, _, _| ext::open_with_default_app(&open_file),
                        ))
                        .child(button(&t, "重置为默认", ButtonKind::Danger, None, reset)),
                )
                .into_any_element(),
        );
    }
    out
}

/// The help line under the secret fields and the two buttons of the credentials file.
pub fn credentials_help(t: &ActiveTheme) -> Div {
    let path: PathBuf = credentials::path();
    let open_path = path.clone();
    let reveal_path = path.clone();
    let hint = permission_hint();
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .children(hint.map(|h| div().text_size(px(11.5)).text_color(t.ac).child(h)))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.tx3)
                .child(format!(
                    "密钥以明文保存在 {}（仅当前用户可读），不会写入笔记库，也不会同步。也可以写 env:变量名 或 cmd:命令 代替明文。",
                    path.display()
                )),
        )
        .child(
            div()
                .flex()
                .gap(px(8.0))
                .child(button(
                    t,
                    "打开凭据文件",
                    ButtonKind::Secondary,
                    None,
                    move |_, _, _| ext::open_with_default_app(&open_path),
                ))
                .child(button(
                    t,
                    "在资源管理器中显示",
                    ButtonKind::Secondary,
                    None,
                    move |_, _, _| ext::reveal_in_explorer(&reveal_path),
                )),
        )
}

impl RustdianApp {
    /// Renames the broken file aside and puts the default text in its place, then re-checks the
    /// files so the banner goes away.
    pub fn reset_config_file(&mut self, path: &Path, template: &str, cx: &mut Context<Self>) {
        match config::reset_to_default(path, template) {
            Ok(Some(bad)) => self.toast(
                ToastKind::Warn,
                format!("已重置为默认值；损坏的文件保存为 {}", bad.display()),
            ),
            Ok(None) => self.toast(ToastKind::Info, "已写入默认文件"),
            Err(err) => self.toast(ToastKind::Error, format!("重置失败：{err}")),
        }
        self.configs.poll();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_follows_the_stored_form_and_never_holds_a_literal() {
        assert_eq!(display_of(None), SecretDisplay::None);
        assert_eq!(display_of(Some("  ")), SecretDisplay::None);
        assert_eq!(display_of(Some("sk-real-value")), SecretDisplay::Literal);
        assert_eq!(
            display_of(Some("env:OPENAI_API_KEY")),
            SecretDisplay::Env("OPENAI_API_KEY".into())
        );
        assert_eq!(display_of(Some("cmd:pass show x")), SecretDisplay::Command);
        assert!(!format!("{:?}", display_of(Some("sk-real-value"))).contains("sk-real"));
    }

    #[test]
    fn status_labels_are_read_only_text_for_env_and_cmd_only() {
        assert_eq!(
            status_label(&SecretDisplay::Env("K".into())).as_deref(),
            Some("来自环境变量 K")
        );
        assert_eq!(
            status_label(&SecretDisplay::Command).as_deref(),
            Some("来自命令")
        );
        assert_eq!(status_label(&SecretDisplay::Literal), None);
        assert_eq!(status_label(&SecretDisplay::None), None);
    }

    #[test]
    fn permission_warning_is_announced_once() {
        // The flag is process-wide; this test only checks the transitions it makes itself.
        PERMISSION_WARNING.store(false, Ordering::Relaxed);
        assert!(!note_permissions(true), "a good write never warns");
        assert!(note_permissions(false), "first failure is announced");
        assert!(
            !note_permissions(false),
            "later failures are not announced again"
        );
        assert!(permission_hint().is_some());
        PERMISSION_WARNING.store(false, Ordering::Relaxed);
        assert!(permission_hint().is_none());
    }

    #[test]
    fn banner_message_names_file_line_and_column() {
        let issue = ConfigIssue {
            file: PathBuf::from("/vault/.jiyue/config.toml"),
            line: 3,
            column: 9,
            key: Some("editor.auto_name_from_heading".into()),
            message: "invalid type".into(),
        };
        let text = banner_message(&issue);
        assert!(text.contains("config.toml:3:9"), "{text}");
        assert!(text.contains("auto_name_from_heading"), "{text}");
        assert!(text.contains("未被覆盖"));
    }
}
