//! The two rows of the 日记 settings page that belong to templates.

use std::rc::Rc;

use gpui::{Context, Div, MouseDownEvent, div, prelude::*, px};
use jiyue_export::{TemplateRules, list_templates};

use super::pure;
use crate::RustdianApp;
use crate::ui::widgets::controls::{choice_chip, form_row, form_row_with_extra, select};

/// 模板文件夹: the folder the templates are read from (模板 or Templates), kept in
/// `.jiyue/templates.json`.
pub fn folder_row(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let current = TemplateRules::load(&app.vault.root_dir).templates_folder;
    let options = [pure::DEFAULT_FOLDER, pure::CHINESE_FOLDER];

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
        true,
    )
}

/// 模板 of the daily note: the built-in one, or one of the templates. The choice is saved in
/// `.jiyue/daily.json` (the same field the daily note reads).
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
    let open = app.settings.daily_template_open;

    let paths: Vec<String> = templates.into_iter().map(|tpl| tpl.path).collect();
    let current_fallback = current.clone();
    let on_pick = Rc::new(cx.listener(move |this, idx: &usize, _, cx| {
        let next = if *idx == 0 {
            String::new()
        } else if let Some(p) = paths.get(idx - 1) {
            p.clone()
        } else {
            current_fallback.clone().unwrap_or_default()
        };
        let mut settings = this
            .settings
            .daily
            .clone()
            .unwrap_or_else(|| this.vault.daily_settings());
        settings.template_path = next;
        let _ = settings.save(&this.vault.root_dir);
        this.settings.daily = Some(settings);
        this.settings.daily_template_open = false;
        cx.notify();
    }));

    let picker = select(
        &t,
        &option_refs,
        selected.min(option_refs.len().saturating_sub(1)),
        open,
        selected.min(option_refs.len().saturating_sub(1)),
        220.0,
        cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.settings.daily_template_open = true;
            this.settings.daily_format_open = false;
            cx.notify();
        }),
        cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.settings.daily_template_open = false;
            cx.notify();
        }),
        on_pick,
    );

    form_row(
        &t,
        &crate::t!("settings-daily-template"),
        &crate::t!("settings-daily-template-desc"),
        picker,
        false,
    )
}
