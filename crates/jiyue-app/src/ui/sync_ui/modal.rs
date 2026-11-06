//! Sync dialog: status, the last report with expandable path lists, the conflict list with its
//! per-item actions, and the confirmation dialogs of this module.

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, MouseButton, SharedString, div, prelude::*, px,
};

use super::conflicts::{self, Item, Kind, Side};
use super::service::RunKind;
use super::{Confirm, close_modal, reset_state, schedule, start_run};
use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::ext::{ExtEvent, ToastKind};
use crate::ui::widgets::controls::clamp_fraction;
use crate::ui::widgets::{
    BadgeKind, ButtonKind, badge, button, confirm_dialog, modal, progress_bar,
};

/// Maximum paths listed per report row.
const LIST_LIMIT: usize = 60;

/// Labels of the report rows, in `State::expanded` order.
const ROW_LABELS: [&str; 6] = ["上传", "下载", "合并", "冲突", "删除", "错误"];

/// The sync dialog.
pub fn render(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let running = app.ext.sync.service.running.is_some();

    let body = div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .w_full()
        .font_family(crate::ui::fonts::ui_name())
        .child(status_block(&t, app))
        .child(report_block(&t, app, cx))
        .child(conflicts_block(&t, app, cx));

    let footer: Vec<AnyElement> = vec![
        button(
            &t,
            "同步设置",
            ButtonKind::Ghost,
            None,
            cx.listener(|this, _, _, cx| {
                close_modal(this);
                super::run(this, super::Command::OpenSyncSettings, cx);
            }),
        )
        .into_any_element(),
        if running {
            button(
                &t,
                "取消同步",
                ButtonKind::Secondary,
                None,
                cx.listener(|this, _, _, cx| {
                    this.ext.sync.service.request_cancel();
                    cx.notify();
                }),
            )
            .into_any_element()
        } else {
            button(
                &t,
                "立即同步",
                ButtonKind::Primary,
                None,
                cx.listener(|this, _, _, cx| {
                    start_run(this, RunKind::Manual, cx);
                    cx.notify();
                }),
            )
            .into_any_element()
        },
    ];

    modal(
        &t,
        "同步",
        body,
        footer,
        560.0,
        cx.listener(|this, _, _, cx| {
            close_modal(this);
            cx.notify();
        }),
    )
}

/// Header line: what the sync is doing, with a bar while it runs.
fn status_block(t: &ActiveTheme, app: &RustdianApp) -> AnyElement {
    let s = &app.ext.sync;
    let mut col = div().flex().flex_col().gap(px(8.0));
    if !s.saved.enabled {
        return col
            .child(text(
                t,
                t.tx2,
                "同步未启用。在设置的 同步 中开启后即可使用。",
            ))
            .into_any_element();
    }
    if s.service.running.is_some() {
        let p = &s.service.progress;
        let fraction = if p.total == 0 {
            0.0
        } else {
            clamp_fraction(p.done as f32 / p.total as f32)
        };
        let detail = if p.path.is_empty() {
            schedule::action_label(p.action).to_owned()
        } else {
            format!("{} {}", schedule::action_label(p.action), p.path)
        };
        col = col
            .child(text(t, t.tx, &schedule::progress_text(p.done, p.total)))
            .child(progress_bar(t, fraction))
            .child(text(t, t.tx3, &detail));
    } else if let Some(err) = s.service.last.as_ref().and_then(|o| o.error.clone()) {
        col = col.child(text(t, t.st, &format!("同步失败：{err}")));
    } else {
        let line = match s.last_ok_ms {
            Some(ms) => format!(
                "已同步 · {}",
                schedule::relative_zh(jiyue_sync::state::now_ms(), ms)
            ),
            None => "尚未同步".to_owned(),
        };
        col = col.child(text(t, t.tx, &line));
    }
    col.into_any_element()
}

/// Counts of the last report with expandable path lists.
fn report_block(t: &ActiveTheme, app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let Some(last) = app.ext.sync.service.last.as_ref() else {
        return div().into_any_element();
    };
    let r = &last.report;
    let deleted: Vec<String> = r
        .deleted_local
        .iter()
        .chain(r.deleted_remote.iter())
        .cloned()
        .collect();
    let lists: [Vec<String>; 6] = [
        r.uploaded.clone(),
        r.downloaded.clone(),
        r.merged.clone(),
        r.conflicts.iter().map(|c| c.path.clone()).collect(),
        deleted,
        r.errors
            .iter()
            .map(|e| format!("{}：{}", e.path, e.message))
            .collect(),
    ];
    let mut block = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(t.line_soft)
        .p(px(6.0))
        .child(
            div()
                .px(px(6.0))
                .pb(px(4.0))
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child("最近一次同步"),
        );
    for (i, label) in ROW_LABELS.iter().enumerate() {
        let count = lists[i].len();
        let open = app.ext.sync.expanded[i];
        let mut row = div()
            .flex()
            .flex_col()
            .px(px(6.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(13.0))
                    .text_color(if i == 5 && count > 0 { t.st } else { t.tx })
                    .child(SharedString::from(label.to_string()))
                    .child(SharedString::from(format!("{count}"))),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.ext.sync.expanded[i] = !this.ext.sync.expanded[i];
                    cx.notify();
                }),
            );
        if open && count > 0 {
            let mut list = div().flex().flex_col().pt(px(4.0)).pl(px(8.0));
            for p in lists[i].iter().take(LIST_LIMIT) {
                list = list.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(t.tx2)
                        .min_w_0()
                        .child(SharedString::from(p.clone())),
                );
            }
            if count > LIST_LIMIT {
                list = list.child(div().text_size(px(12.0)).text_color(t.tx3).child(
                    SharedString::from(format!("……还有 {} 个", count - LIST_LIMIT)),
                ));
            }
            row = row.child(list);
        }
        block = block.child(row);
    }
    if let Some(fatal) = &r.fatal {
        block = block.child(text(t, t.st, fatal));
    }
    block.into_any_element()
}

/// Open conflicts with their resolution actions.
fn conflicts_block(
    t: &ActiveTheme,
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let items = &app.ext.sync.conflicts;
    let mut block = div().flex().flex_col().gap(px(8.0)).child(
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.tx)
                    .child("冲突"),
            )
            .child(badge(
                t,
                &format!("{}", items.len()),
                if items.is_empty() {
                    BadgeKind::Neutral
                } else {
                    BadgeKind::Accent
                },
            )),
    );
    if items.is_empty() {
        return block
            .child(text(t, t.tx3, "没有未解决的冲突。"))
            .into_any_element();
    }
    for (i, item) in items.iter().enumerate() {
        block = block.child(conflict_row(t, i, item, cx));
    }
    block.into_any_element()
}

fn conflict_row(
    t: &ActiveTheme,
    index: usize,
    item: &Item,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let kind_text = match item.kind {
        Kind::Markers => "文本冲突：笔记中有 <<<<<<< 标记",
        Kind::BothKept => "二进制文件：两边都已保留，远端版本在冲突副本中",
    };
    let mut actions = div().flex().flex_wrap().items_center().gap(px(6.0));
    actions = actions.child(button(
        t,
        "保留本机版本",
        ButtonKind::Secondary,
        None,
        cx.listener(move |this, _, _, cx| {
            resolve(this, index, Side::Local, cx);
        }),
    ));
    actions = actions.child(button(
        t,
        "保留远端版本",
        ButtonKind::Secondary,
        None,
        cx.listener(move |this, _, _, cx| {
            resolve(this, index, Side::Remote, cx);
        }),
    ));
    if item.kind == Kind::Markers {
        actions = actions.child(button(
            t,
            "打开笔记手动合并",
            ButtonKind::Ghost,
            None,
            cx.listener(move |this, _, _, cx| {
                open_for_merge(this, index, cx);
            }),
        ));
    }
    actions = actions.child(button(
        t,
        "标记已解决",
        ButtonKind::Ghost,
        None,
        cx.listener(move |this, _, _, cx| {
            this.ext.sync.confirm = Some(Confirm::Resolve(index));
            cx.notify();
        }),
    ));

    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .p(px(10.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx)
                .min_w_0()
                .child(SharedString::from(item.path.clone())),
        )
        .child(div().text_size(px(12.0)).text_color(t.tx3).child(kind_text))
        .child(actions)
        .into_any_element()
}

/// Keeps one side of a conflict, moves the other copy to the trash and tells the other modules.
fn resolve(app: &mut RustdianApp, index: usize, keep: Side, cx: &mut Context<RustdianApp>) {
    let Some(item) = app.ext.sync.conflicts.get(index).cloned() else {
        return;
    };
    let root = app.vault.root_dir.clone();
    let result = match keep {
        Side::Local => conflicts::keep_local(&root, &item),
        Side::Remote => conflicts::keep_remote(&root, &item),
    };
    match result {
        Ok(()) => {
            remove_item(app, index);
            app.toast(
                ToastKind::Success,
                match keep {
                    Side::Local => "已保留本机版本，等待下次同步上传",
                    Side::Remote => "已保留远端版本，等待下次同步",
                },
            );
            crate::ui::ext::on_event(app, &ExtEvent::VaultFilesChanged(vec![item.path]), cx);
        }
        Err(e) => app.toast(ToastKind::Error, format!("冲突处理失败：{e}")),
    }
    cx.notify();
}

/// Marks a conflict resolved by hand after the confirmation.
pub fn mark_resolved(app: &mut RustdianApp, index: usize, cx: &mut Context<RustdianApp>) {
    let Some(item) = app.ext.sync.conflicts.get(index).cloned() else {
        return;
    };
    let root = app.vault.root_dir.clone();
    match conflicts::mark_resolved(&root, &item) {
        Ok(()) => {
            remove_item(app, index);
            app.toast(ToastKind::Success, "冲突已标记为已解决");
        }
        Err(e) => app.toast(ToastKind::Error, format!("标记失败：{e}")),
    }
    cx.notify();
}

fn remove_item(app: &mut RustdianApp, index: usize) {
    if index < app.ext.sync.conflicts.len() {
        app.ext.sync.conflicts.remove(index);
    }
    let root = app.vault.root_dir.clone();
    if let Err(e) = conflicts::save(&root, &app.ext.sync.conflicts) {
        app.toast(ToastKind::Error, format!("无法保存冲突列表：{e}"));
    }
}

/// Opens the note with the conflict markers so the user can merge by hand.
fn open_for_merge(app: &mut RustdianApp, index: usize, cx: &mut Context<RustdianApp>) {
    let Some(path) = app.ext.sync.conflicts.get(index).map(|i| i.path.clone()) else {
        return;
    };
    match app.vault.notes.iter().position(|n| n.path == path) {
        Some(idx) => {
            close_modal(app);
            app.open_note_by_index(idx);
        }
        None => app.toast(ToastKind::Info, "笔记不在当前笔记库中，请在文件列表中打开"),
    }
    cx.notify();
}

/// Confirmation dialogs of this module.
pub fn render_confirm(
    app: &RustdianApp,
    confirm: Confirm,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    match confirm {
        Confirm::Reset => confirm_dialog(
            &t,
            "重置同步状态",
            "删除 .jiyue/sync/ 下的基准记录、冲突列表和设备标识。笔记本身不会改动。",
            "重置",
            true,
            cx.listener(|this, _, _, cx| {
                this.ext.sync.confirm = None;
                reset_state(this, cx);
            }),
            cx.listener(|this, _, _, cx| {
                this.ext.sync.confirm = None;
                cx.notify();
            }),
        ),
        Confirm::Resolve(index) => confirm_dialog(
            &t,
            "标记为已解决",
            "冲突副本会移到 .trash/sync/，之后可以从那里找回。",
            "标记已解决",
            false,
            cx.listener(move |this, _, _, cx| {
                this.ext.sync.confirm = None;
                mark_resolved(this, index, cx);
            }),
            cx.listener(|this, _, _, cx| {
                this.ext.sync.confirm = None;
                cx.notify();
            }),
        ),
    }
}

fn text(_t: &ActiveTheme, color: gpui::Rgba, value: &str) -> gpui::Div {
    div()
        .text_size(px(13.0))
        .text_color(color)
        .min_w_0()
        .child(SharedString::from(value.to_owned()))
}
