//! The two rows of the 日记 settings page that belong to templates.

use std::rc::Rc;

use gpui::{Context, Div, MouseDownEvent, div, prelude::*, px};
use jiyue_export::{TemplateRules, list_templates};

use super::pure;
use crate::RustdianApp;
use crate::ui::widgets::controls::{choice_chip, form_row, form_row_with_extra, select};

/// 模板文件夹: the folder the templates are read from (`_templates`, `Templates`, or `模板`), kept in
/// `.jiyue/config.toml` (`[templates]`).
pub fn folder_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let current = TemplateRules::load(&app.vault.root_dir).templates_folder;
    let options = [
        pure::OBSIDIAN_FOLDER,
        pure::DEFAULT_FOLDER,
        pure::CHINESE_FOLDER,
    ];

    let mut chips = div().flex().items_center().justify_end().gap(px(6.0));
    for name in options {
        let on = name == current;
        chips = chips.child(choice_chip(
            &t,
            name,
            on,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                let root = this.vault.root_dir.clone();
                let mut rules = TemplateRules::load(&root);
                rules.templates_folder = name.to_owned();
                let _ = rules.save(&root);
                crate::ui::memos_ui::invalidate_cache(this);
                if let Some(inp) = &this.settings.template_folder_input {
                    inp.update(cx, |i, cx| i.set_text(name, cx));
                }
                cx.notify();
            }),
        ));
    }

    let input_box = div()
        .w(px(200.0))
        .h(px(30.0))
        .when_some(app.settings.template_folder_input.clone(), |d, inp| {
            d.child(inp)
        });

    form_row_with_extra(
        &t,
        &crate::t!("settings-template-folder"),
        &crate::t!("settings-template-folder-desc"),
        input_box,
        chips,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn template_picker_row(
    app: &RustdianApp,
    title: &str,
    description: &str,
    current: Option<String>,
    open: bool,
    on_open: impl Fn(&mut RustdianApp, &mut Context<RustdianApp>) + 'static,
    on_close: impl Fn(&mut RustdianApp, &mut Context<RustdianApp>) + 'static,
    on_pick: impl Fn(&mut RustdianApp, String, &mut Context<RustdianApp>) + 'static,
    last: bool,
    cx: &mut Context<RustdianApp>,
) -> Div {
    let t = app.theme;
    let root = app.vault.root_dir.clone();
    let configured = TemplateRules::load(&root).templates_folder;
    let folder = pure::choose_folder(&configured, |f| root.join(f).is_dir());
    let templates = list_templates(&root, &folder);
    let selected = pure::selected_daily_template_index(current.as_deref(), &templates);

    let mut labels: Vec<String> = Vec::with_capacity(templates.len() + 2);
    labels.push(crate::t!("settings-template-builtin"));
    for tpl in &templates {
        labels.push(tpl.name.clone());
    }
    if let Some(path) = &current
        && !templates.iter().any(|tpl| &tpl.path == path)
    {
        labels.push(path.clone());
    }
    let option_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let paths: Vec<String> = templates.into_iter().map(|tpl| tpl.path).collect();
    let current_fallback = current.unwrap_or_default();

    let picker = select(
        &t,
        &option_refs,
        selected.min(option_refs.len().saturating_sub(1)),
        open,
        selected.min(option_refs.len().saturating_sub(1)),
        220.0,
        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            on_open(this, cx);
            cx.notify();
        }),
        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            on_close(this, cx);
            cx.notify();
        }),
        Rc::new(cx.listener(move |this, idx: &usize, _, cx| {
            let next = if *idx == 0 {
                String::new()
            } else if let Some(p) = paths.get(idx - 1) {
                p.clone()
            } else {
                current_fallback.clone()
            };
            on_pick(this, next, cx);
            cx.notify();
        })),
    );

    form_row(&t, title, description, picker, last)
}

/// 模板 of the daily note: the built-in one, or one of the templates. The choice is saved in
/// `.jiyue/config.toml` (`[daily]`).
pub fn daily_template_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let daily = app
        .settings
        .daily
        .clone()
        .unwrap_or_else(|| app.vault.daily_settings());
    let current = Some(daily.template_path.trim().to_string()).filter(|p| !p.is_empty());
    template_picker_row(
        app,
        &crate::t!("settings-daily-template"),
        &crate::t!("settings-daily-template-desc"),
        current,
        app.settings.daily_template_open,
        |this, _| {
            this.settings.daily_template_open = true;
            this.settings.daily_format_open = false;
            this.settings.todo_template_open = false;
            this.settings.inbox_template_open = false;
        },
        |this, _| {
            this.settings.daily_template_open = false;
        },
        |this, next, _| {
            let mut settings = this
                .settings
                .daily
                .clone()
                .unwrap_or_else(|| this.vault.daily_settings());
            settings.template_path = next;
            let _ = settings.save(&this.vault.root_dir);
            this.settings.daily = Some(settings);
            this.settings.daily_template_open = false;
        },
        false,
        cx,
    )
}

/// 待办清单模板选择器（存入 `[inbox].todo_template_path`）。
pub fn todo_template_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let inbox = jiyue_core::inbox::InboxSettings::load(&app.vault.root_dir);
    let current = Some(inbox.todo_template_path.trim().to_string()).filter(|p| !p.is_empty());
    template_picker_row(
        app,
        "待办清单模板",
        "在待办目录新建清单或使用「新建待办清单」时自动套用的模板",
        current,
        app.settings.todo_template_open,
        |this, _| {
            this.settings.todo_template_open = true;
            this.settings.daily_format_open = false;
            this.settings.daily_template_open = false;
            this.settings.inbox_template_open = false;
        },
        |this, _| {
            this.settings.todo_template_open = false;
        },
        |this, next, _| {
            let root = this.vault.root_dir.clone();
            let mut inbox = jiyue_core::inbox::InboxSettings::load(&root);
            inbox.todo_template_path = next;
            let _ = inbox.save(&root);
            crate::ui::memos_ui::invalidate_cache(this);
            this.settings.todo_template_open = false;
        },
        false,
        cx,
    )
}

/// 随手记卡片模板选择器（存入 `[inbox].inbox_template_path`）。
pub fn inbox_template_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let inbox = jiyue_core::inbox::InboxSettings::load(&app.vault.root_dir);
    let current = Some(inbox.inbox_template_path.trim().to_string()).filter(|p| !p.is_empty());
    template_picker_row(
        app,
        "随手记模板",
        "在随手记目录新建笔记或发送独立闪念卡片时自动套用的模板",
        current,
        app.settings.inbox_template_open,
        |this, _| {
            this.settings.inbox_template_open = true;
            this.settings.daily_format_open = false;
            this.settings.daily_template_open = false;
            this.settings.todo_template_open = false;
        },
        |this, _| {
            this.settings.inbox_template_open = false;
        },
        |this, next, _| {
            let root = this.vault.root_dir.clone();
            let mut inbox = jiyue_core::inbox::InboxSettings::load(&root);
            inbox.inbox_template_path = next;
            let _ = inbox.save(&root);
            crate::ui::memos_ui::invalidate_cache(this);
            this.settings.inbox_template_open = false;
        },
        false,
        cx,
    )
}
