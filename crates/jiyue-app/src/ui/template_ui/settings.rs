//! The two rows of the 日记 settings page that belong to templates.

use gpui::{Context, Div, IntoElement, MouseButton, SharedString, div, prelude::*, px};
use jiyue_export::{TemplateRules, list_templates};

use super::pure;
use crate::RustdianApp;
use crate::ui::settings::setting_row;

/// 模板文件夹: the folder the templates are read from (模板 or Templates), kept in
/// `.jiyue/templates.json`.
pub fn folder_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let current = TemplateRules::load(&app.vault.root_dir).templates_folder;
    let options = [pure::DEFAULT_FOLDER, pure::ENGLISH_FOLDER];
    let mut control = div()
        .flex()
        .h(px(30.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .overflow_hidden();
    for name in options {
        let on = name == current;
        control = control.child(
            div()
                .flex()
                .items_center()
                .px(px(12.0))
                .h_full()
                .text_size(px(12.5))
                .cursor_pointer()
                .bg(if on { t.active } else { t.sheet })
                .text_color(if on { t.tx } else { t.tx2 })
                .child(SharedString::from(name))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        let root = this.vault.root_dir.clone();
                        let mut rules = TemplateRules::load(&root);
                        rules.templates_folder = name.to_owned();
                        let _ = rules.save(&root);
                        cx.notify();
                    }),
                ),
        );
    }
    setting_row(
        &t,
        &crate::t!("settings-template-folder"),
        &crate::t!("settings-template-folder-desc"),
        control,
        false,
    )
}

/// 模板 of the daily note: the built-in one, or one of the templates. The button steps through
/// them. The choice is saved in `.jiyue/daily.json` (the same field the daily note reads).
pub fn daily_template_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let root = app.vault.root_dir.clone();
    let daily = app
        .settings
        .daily
        .clone()
        .unwrap_or_else(|| app.vault.daily_settings());
    let configured = TemplateRules::load(&root).templates_folder;
    let folder = pure::choose_folder(&configured, |f| root.join(f).is_dir());
    let templates = list_templates(&root, &folder);

    let current = Some(daily.template_path.trim().to_string()).filter(|p| !p.is_empty());
    let shown = match &current {
        Some(path) => templates
            .iter()
            .find(|tpl| &tpl.path == path)
            .map(|tpl| tpl.name.clone())
            .unwrap_or_else(|| path.clone()),
        None => crate::t!("settings-template-builtin"),
    };
    let next_button = div()
        .flex()
        .items_center()
        .h(px(28.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .text_size(px(12.5))
        .text_color(t.tx2)
        .child(SharedString::from(crate::t!("settings-template-change")))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                let next = pure::next_daily_template(current.as_deref(), &templates);
                let mut settings = this
                    .settings
                    .daily
                    .clone()
                    .unwrap_or_else(|| this.vault.daily_settings());
                settings.template_path = next.unwrap_or_default();
                let _ = settings.save(&this.vault.root_dir);
                this.settings.daily = Some(settings);
                cx.notify();
            }),
        );
    let control = div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .text_size(px(12.5))
                .text_color(t.tx2)
                .child(SharedString::from(shown)),
        )
        .child(next_button.into_any_element());
    setting_row(
        &t,
        &crate::t!("settings-daily-template"),
        &crate::t!("settings-daily-template-desc"),
        control,
        true,
    )
}
