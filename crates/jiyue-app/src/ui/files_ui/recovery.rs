//! Start-up recovery. Unsaved text that an earlier run left in the recovery journal is listed in the
//! dialog 'Jiyue 上次没有正常退出'. Restoring opens the text as an unsaved edit: the disk file is not
//! written until the user saves the note. Comparing shows the difference with the disk file, and
//! discarding removes the copy.

use gpui::{AnyElement, Context, div, prelude::*, px};
use jiyue_core::buffer::Note;
use jiyue_core::recovery::{self, Journal, Verdict};

use super::modals::{self, Confirm, Modal};
use super::pure;
use crate::journal;
use crate::ui::UI_FONT;
use crate::ui::ext::ToastKind;
use crate::ui::widgets::controls::{ButtonKind, button};
use crate::{PendingRestore, RustdianApp};

/// One leftover copy shown in the dialog.
pub struct RecoveryItem {
    pub journal: Journal,
    pub verdict: Verdict,
    /// The disk file's text (`None` when the file is missing).
    pub disk: Option<String>,
    /// Line diff of the disk file against the recovered text (`None` when too large to show).
    pub rows: Option<Vec<pure::DiffRow>>,
}

impl RecoveryItem {
    fn new(journal: Journal, verdict: Verdict, disk: Option<String>) -> Self {
        let rows = pure::line_diff(disk.as_deref().unwrap_or(""), &journal.text);
        Self {
            journal,
            verdict,
            disk,
            rows,
        }
    }
}

/// Start-up scan: copies that match the disk file are removed silently; the others open the
/// dialog. Called once when the app is built.
pub fn start_recovery(app: &mut RustdianApp, _cx: &mut Context<RustdianApp>) {
    let root = app.vault.root_dir.clone();
    let mut items = Vec::new();
    for copy in recovery::load_all(&root) {
        let (disk, mtime) = journal::disk_state(&root, &copy.path);
        let verdict = recovery::judge(&copy.text, disk.as_deref(), mtime, copy.base_mtime_ms);
        match verdict {
            Verdict::Stale => app.journals.remove(&copy.path),
            _ => items.push(RecoveryItem::new(copy, verdict, disk)),
        }
    }
    if !items.is_empty() {
        app.ext.files.modal = Some(Modal::Recovery { items, diff: None });
    }
}

/// Removes the item of `path` from the open dialog and returns it.
fn take_item(app: &mut RustdianApp, path: &str) -> Option<RecoveryItem> {
    let Some(Modal::Recovery { items, .. }) = app.ext.files.modal.as_mut() else {
        return None;
    };
    let pos = items.iter().position(|i| i.journal.path == path)?;
    Some(items.remove(pos))
}

/// Closes the dialog once no item is left.
fn close_if_empty(app: &mut RustdianApp) {
    let empty = matches!(
        app.ext.files.modal.as_ref(),
        Some(Modal::Recovery { items, .. }) if items.is_empty()
    );
    if empty {
        app.ext.files.modal = None;
    }
}

/// Opens an item as an unsaved edit: the note is opened (a note whose file is gone becomes an
/// unsaved note, written only on save), and its editor shows the recovered text.
fn apply(app: &mut RustdianApp, item: RecoveryItem, cx: &mut Context<RustdianApp>) {
    let path = item.journal.path.clone();
    let text = item.journal.text.clone();
    let base = item.journal.base_mtime_ms;
    let idx = match app.vault.notes.iter().position(|n| n.path == path) {
        Some(idx) => idx,
        None => {
            let title = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .trim_end_matches(".md")
                .to_string();
            app.vault.notes.push(Note::new(&title, &path, &text));
            app.vault.rebuild_graph();
            app.vault.notes.len() - 1
        }
    };
    app.vault.notes[idx].set_content(&text);
    match app.editors.get_mut(&path) {
        Some(entry) => {
            entry.restored = true;
            entry.base_mtime_ms = base;
            entry.save_failing = false;
            let editor = entry.editor.clone();
            editor.update(cx, |ed, cx| ed.set_text(&text, cx));
        }
        None => {
            app.pending_restore.insert(
                path.clone(),
                PendingRestore {
                    text,
                    base_mtime_ms: base,
                },
            );
        }
    }
    app.open_note_by_index(idx);
    app.toast(
        ToastKind::Info,
        format!("已恢复未保存的内容：{path}（保存后才会写入磁盘）"),
    );
}

/// Restores one note from the dialog.
pub fn restore_path(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    if let Some(item) = take_item(app, path) {
        apply(app, item, cx);
    }
    close_if_empty(app);
    cx.notify();
}

/// Restores every note from the dialog and closes it.
pub fn restore_all(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let items = match app.ext.files.modal.take() {
        Some(Modal::Recovery { items, .. }) => items,
        other => {
            app.ext.files.modal = other;
            return;
        }
    };
    for item in items {
        apply(app, item, cx);
    }
    cx.notify();
}

/// Discards the copy of one note. The note's file is not touched.
pub fn discard_path(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    if take_item(app, path).is_some() {
        app.journals.remove(path);
        app.toast(ToastKind::Info, format!("已丢弃未保存的内容：{path}"));
    }
    close_if_empty(app);
    cx.notify();
}

/// Discards every copy and closes the dialog (after the confirmation).
pub fn discard_all(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if let Some(Modal::Recovery { items, .. }) = app.ext.files.modal.take() {
        for item in &items {
            app.journals.remove(&item.journal.path);
        }
        app.toast(
            ToastKind::Info,
            format!("已丢弃 {} 个笔记的未保存内容", items.len()),
        );
    }
    cx.notify();
}

/// Shows the difference of one item in the dialog, or the list again when `path` is `None`.
pub fn show_diff(app: &mut RustdianApp, path: Option<String>, cx: &mut Context<RustdianApp>) {
    if let Some(Modal::Recovery { diff, .. }) = app.ext.files.modal.as_mut() {
        *diff = path;
    }
    cx.notify();
}

/// Title, body and footer of the dialog: the list of copies, or the diff of one of them.
pub fn view(
    app: &RustdianApp,
    items: &[RecoveryItem],
    diff: Option<&str>,
    cx: &mut Context<RustdianApp>,
) -> (String, AnyElement, Vec<AnyElement>) {
    let t = app.theme;
    let me = cx.entity();
    if let Some(item) = diff.and_then(|p| items.iter().find(|i| i.journal.path == p)) {
        let body = modals::diff_body(&t, item.rows.as_deref());
        let restore_me = me.clone();
        let restore_path_key = item.journal.path.clone();
        let back_me = me.clone();
        let footer = vec![
            button(
                &t,
                "恢复这份内容",
                ButtonKind::Secondary,
                None,
                move |_, _, cx| {
                    let path = restore_path_key.clone();
                    restore_me.update(cx, |app, cx| restore_path(app, &path, cx));
                },
            )
            .into_any_element(),
            button(
                &t,
                "返回列表",
                ButtonKind::Ghost,
                None,
                move |_, _, cx| {
                    back_me.update(cx, |app, cx| show_diff(app, None, cx));
                },
            )
            .into_any_element(),
        ];
        return (
            format!("对比：{}", pure::name_of(&item.journal.path)),
            body,
            footer,
        );
    }

    let mut list = div()
        .id("recovery-list")
        .flex()
        .flex_col()
        .gap(px(8.0))
        .max_h(px(420.0))
        .overflow_y_scroll();
    for item in items {
        let path = item.journal.path.clone();
        let note = match item.verdict {
            Verdict::DiskMissing => "磁盘上的文件已不存在".to_string(),
            Verdict::Differs { disk_changed: true } => {
                "磁盘上的文件在这次编辑之后已被修改，恢复会覆盖外部修改".to_string()
            }
            _ => "与磁盘上的文件不同".to_string(),
        };
        let (diff_me, restore_me, discard_me) = (me.clone(), me.clone(), me.clone());
        let (diff_path, restore_path_key, discard_path_key) =
            (path.clone(), path.clone(), path.clone());
        list = list.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(10.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(t.line_soft)
                .child(div().text_color(t.tx).child(path.clone()))
                .child(div().text_color(t.tx3).text_size(px(12.0)).child(note))
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(button(
                            &t,
                            "对比",
                            ButtonKind::Ghost,
                            None,
                            move |_, _, cx| {
                                let path = diff_path.clone();
                                diff_me.update(cx, |app, cx| show_diff(app, Some(path), cx));
                            },
                        ))
                        .child(button(
                            &t,
                            "恢复",
                            ButtonKind::Secondary,
                            None,
                            move |_, _, cx| {
                                let path = restore_path_key.clone();
                                restore_me.update(cx, |app, cx| restore_path(app, &path, cx));
                            },
                        ))
                        .child(button(
                            &t,
                            "丢弃",
                            ButtonKind::Danger,
                            None,
                            move |_, _, cx| {
                                let path = discard_path_key.clone();
                                discard_me.update(cx, |app, cx| discard_path(app, &path, cx));
                            },
                        )),
                ),
        );
    }
    let body = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .font_family(UI_FONT)
        .child(
            div()
                .text_color(t.tx2)
                .child("以下笔记有未保存的内容。恢复后它们会在编辑器中显示为未保存，磁盘文件要等你保存后才会改变。"),
        )
        .child(list);

    let (all_me, discard_all_me, later_me) = (me.clone(), me.clone(), me.clone());
    let paths: Vec<String> = items.iter().map(|i| i.journal.path.clone()).collect();
    let footer = vec![
        button(
            &t,
            "全部恢复",
            ButtonKind::Secondary,
            None,
            move |_, _, cx| {
                all_me.update(cx, restore_all);
            },
        )
        .into_any_element(),
        button(
            &t,
            "全部丢弃",
            ButtonKind::Danger,
            None,
            move |_, _, cx| {
                discard_all_me.update(cx, |app, cx| {
                    app.ext.files.modal =
                        Some(Modal::Confirm(Confirm::DiscardRecovery(paths.clone())));
                    cx.notify();
                });
            },
        )
        .into_any_element(),
        button(
            &t,
            "稍后处理",
            ButtonKind::Ghost,
            None,
            move |_, _, cx| {
                later_me.update(cx, |app, cx| {
                    app.ext.files.modal = None;
                    app.toast(ToastKind::Info, "未保存的内容仍保留，下次启动时会再次提示");
                    cx.notify();
                });
            },
        )
        .into_any_element(),
    ];
    (
        "Jiyue 上次没有正常退出".to_string(),
        body.into_any_element(),
        footer,
    )
}
