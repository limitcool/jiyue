//! Modals of the file operations: the trash, the move picker, the diff of an externally changed
//! note, and the confirmations for delete, permanent delete and emptying the trash.

#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    AnyElement, App, AppContext, Context, Entity, FontWeight, IntoElement, MouseButton,
    MouseDownEvent, Subscription, Window, div, prelude::*, px,
};
#[cfg(windows)]
use jiyue_core::daily::Date;
use jiyue_core::fsops::{self, TRASH_DIR, TrashEntry};

use super::pure::{self, DiffKind, DiffRow, MoveCheck};
use super::recovery::{self, RecoveryItem};
use super::{actions, watch};
use crate::RustdianApp;
use crate::ui::ext::ToastKind;
use crate::ui::icons::icon;
use crate::ui::widgets::controls::{ButtonKind, button, empty_state, icon_button};
use crate::ui::widgets::line_input::{LineInput, LineInputEvent};
use crate::ui::widgets::overlay::modal;

/// Rows of the diff view shown at most.
const DIFF_ROW_LIMIT: usize = 400;

/// The open modal.
pub enum Modal {
    /// The trash: the entries, and the local UTC offset in seconds (for the delete times).
    Trash {
        entries: Vec<TrashEntry>,
        offset: i64,
    },
    /// Pick a folder to move `path` into.
    Move {
        /// The rows to move (`(path, is_dir)`): one row, or the effective roots of a selection.
        items: Vec<(String, bool)>,
        filter: Entity<LineInput>,
        query: String,
        _sub: Subscription,
    },
    /// Disk copy against the editor text. `None` when the texts are too large to align.
    Diff {
        path: String,
        rows: Option<Vec<DiffRow>>,
    },
    Confirm(Confirm),
    /// The start-up recovery dialog. `diff` is the path whose comparison is shown, if any.
    Recovery {
        items: Vec<RecoveryItem>,
        diff: Option<String>,
    },
}

/// A question with a red button.
pub enum Confirm {
    /// Move a note or folder to the trash.
    Delete { path: String, is_dir: bool },
    /// Move several rows (`(path, is_dir)`) to the trash at once.
    DeleteMany(Vec<(String, bool)>),
    /// Delete a trash entry for good.
    Purge(TrashEntry),
    /// Empty the trash (the number of entries).
    Empty(usize),
    /// Discard the recovered text of these notes (their vault-relative paths).
    DiscardRecovery(Vec<String>),
    /// Close the window although these saves failed (the number of notes).
    CloseDespiteFailures(usize),
}

/// Asks whether the window may close although `count` saves failed. The window stays open until
/// the answer.
pub fn confirm_close_despite_failures(app: &mut RustdianApp, count: usize) {
    app.ext.files.modal = Some(Modal::Confirm(Confirm::CloseDespiteFailures(count)));
}

/// Opens the trash view.
pub fn open_trash(app: &mut RustdianApp) {
    app.ext.files.menu = None;
    let entries = fsops::list_trash(&app.vault.root_dir).unwrap_or_default();
    app.ext.files.modal = Some(Modal::Trash {
        entries,
        offset: local_offset_secs(),
    });
}

/// Asks before a note or folder goes to the trash.
pub fn confirm_delete(app: &mut RustdianApp, path: String, is_dir: bool) {
    app.ext.files.menu = None;
    app.ext.files.modal = Some(Modal::Confirm(Confirm::Delete { path, is_dir }));
}

/// One confirmation for the rows of a multi-selection.
pub fn confirm_delete_many(app: &mut RustdianApp, items: Vec<(String, bool)>) {
    app.ext.files.menu = None;
    if items.is_empty() {
        return;
    }
    app.ext.files.modal = Some(Modal::Confirm(Confirm::DeleteMany(items)));
}

/// Opens the folder picker for moving `path`.
pub fn open_move(app: &mut RustdianApp, path: String, is_dir: bool, cx: &mut Context<RustdianApp>) {
    open_move_items(app, vec![(path, is_dir)], cx);
}

/// Opens the folder picker for moving several rows (`(path, is_dir)`).
pub fn open_move_items(
    app: &mut RustdianApp,
    items: Vec<(String, bool)>,
    cx: &mut Context<RustdianApp>,
) {
    app.ext.files.menu = None;
    let theme = app.theme;
    let filter = cx.new(|cx| {
        let mut input = LineInput::new(cx, "搜索文件夹");
        input.set_theme(theme, cx);
        input
    });
    let sub = cx.subscribe(
        &filter,
        |app: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            match ev {
                LineInputEvent::Changed(text) => {
                    if let Some(Modal::Move { query, .. }) = app.ext.files.modal.as_mut() {
                        *query = text.clone();
                    }
                    cx.notify();
                }
                LineInputEvent::Submit(_) => {
                    // Enter moves into the first folder that matches the filter.
                    let first = match app.ext.files.modal.as_ref() {
                        Some(Modal::Move { items, query, .. }) => {
                            move_candidates(app, items, query).into_iter().next()
                        }
                        _ => None,
                    };
                    if let Some(dest) = first {
                        move_from_picker(app, dest, cx);
                    }
                }
                LineInputEvent::Cancel => {
                    app.ext.files.modal = None;
                    cx.notify();
                }
            }
        },
    );
    let focus = filter.clone();
    app.ext.files.modal = Some(Modal::Move {
        items,
        filter,
        query: String::new(),
        _sub: sub,
    });
    // Next frame: the overlay that just opened gives the keyboard to the root in this frame.
    super::with_window(cx, move |_, window, _| {
        let focus = focus.clone();
        window.on_next_frame(move |window, cx| focus.read(cx).focus(window));
        window.refresh();
    });
    cx.notify();
}

/// Folders a move of `path` can go to, matching `query` (case-insensitive). The root is `""`.
fn move_candidates(app: &RustdianApp, items: &[(String, bool)], query: &str) -> Vec<String> {
    let needle = query.trim().to_lowercase();
    std::iter::once(String::new())
        .chain(app.vault.folders.iter().cloned())
        .filter(|dest| {
            items.iter().any(|(path, is_dir)| {
                matches!(pure::move_target(path, *is_dir, dest), MoveCheck::Move(_))
            })
        })
        .filter(|dest| needle.is_empty() || dest.to_lowercase().contains(&needle))
        .collect()
}

/// Moves the item of the open picker into `dest` and closes the picker.
fn move_from_picker(app: &mut RustdianApp, dest: String, cx: &mut Context<RustdianApp>) {
    let Some(Modal::Move { items, .. }) = app.ext.files.modal.take() else {
        return;
    };
    super::with_window(cx, move |app, window, cx| {
        actions::move_items(app, window, &items, &dest, cx);
        cx.notify();
    });
    cx.notify();
}

/// Opens the two-column diff of the active note: the file on disk and the editor text.
pub fn open_diff(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(path) = app.active_path().map(str::to_string) else {
        return;
    };
    let disk = std::fs::read_to_string(app.vault.root_dir.join(&path)).unwrap_or_default();
    let mine = match app.editor_for(&path) {
        Some(editor) => editor.read(cx).text(),
        None => app
            .vault
            .notes
            .iter()
            .find(|n| n.path == path)
            .map(|n| n.content().to_string())
            .unwrap_or_default(),
    };
    let rows = pure::line_diff(&disk, &mine);
    app.ext.files.menu = None;
    app.ext.files.modal = Some(Modal::Diff { path, rows });
    cx.notify();
}

/// Refreshes the entries of an open trash view.
fn refresh_trash(app: &mut RustdianApp) {
    let entries = fsops::list_trash(&app.vault.root_dir).unwrap_or_default();
    if let Some(Modal::Trash { entries: list, .. }) = app.ext.files.modal.as_mut() {
        *list = entries;
    }
}

/// Puts a trash entry back from the trash view.
fn restore_from_trash(app: &mut RustdianApp, entry: &TrashEntry, cx: &mut Context<RustdianApp>) {
    match actions::restore_entry(app, entry, cx) {
        Ok(rel) => app.toast(ToastKind::Success, format!("已恢复：{rel}")),
        Err(message) => app.toast(ToastKind::Error, message),
    }
    refresh_trash(app);
    cx.notify();
}

/// Deletes one trash entry (its item and its sidecar) for good.
fn purge_entry(app: &mut RustdianApp, entry: &TrashEntry) -> Result<(), String> {
    if entry.name.contains(['/', '\\']) || entry.name == ".." || entry.name == "." {
        return Err("回收站项目名称无效".to_string());
    }
    let trash = app.vault.root_dir.join(TRASH_DIR);
    let item = trash.join(&entry.name);
    let sidecar = trash.join(format!("{}.json", entry.name));
    let removed = if entry.is_dir {
        std::fs::remove_dir_all(&item)
    } else {
        std::fs::remove_file(&item)
    };
    removed.map_err(|err| format!("无法删除：{err}"))?;
    let _ = std::fs::remove_file(sidecar);
    Ok(())
}

/// Shows the modal layer, if one is open.
pub fn overlay(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    let modal_state = app.ext.files.modal.as_ref()?;
    let t = app.theme;
    let me = cx.entity();
    let close_me = me.clone();
    let on_close = move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
        close_me.update(cx, |app, cx| {
            app.ext.files.modal = None;
            cx.notify();
        });
    };
    let el = match modal_state {
        Modal::Trash { entries, offset } => {
            let body = trash_body(&t, &me, entries, *offset);
            let mut footer = Vec::new();
            if !entries.is_empty() {
                let count = entries.len();
                let empty_me = me.clone();
                footer.push(
                    button(
                        &t,
                        "清空回收站",
                        ButtonKind::Danger,
                        None,
                        move |_, _, cx| {
                            empty_me.update(cx, |app, cx| {
                                app.ext.files.modal = Some(Modal::Confirm(Confirm::Empty(count)));
                                cx.notify();
                            });
                        },
                    )
                    .into_any_element(),
                );
            }
            let close_btn = me.clone();
            footer.push(
                button(
                    &t,
                    "关闭",
                    ButtonKind::Secondary,
                    None,
                    move |_, _, cx| {
                        close_btn.update(cx, |app, cx| {
                            app.ext.files.modal = None;
                            cx.notify();
                        });
                    },
                )
                .into_any_element(),
            );
            modal(&t, "回收站", body, footer, 580.0, on_close)
        }
        Modal::Move {
            items,
            filter,
            query,
            ..
        } => {
            let candidates = move_candidates(app, items, query);
            let mut list = div()
                .id("move-folder-list")
                .flex()
                .flex_col()
                .max_h(px(320.0))
                .overflow_y_scroll()
                .gap(px(2.0));
            if candidates.is_empty() {
                list = list.child(
                    div()
                        .py(px(14.0))
                        .text_color(t.tx3)
                        .child("没有匹配的文件夹"),
                );
            }
            for dest in candidates {
                let label = if dest.is_empty() {
                    "根目录".to_string()
                } else {
                    dest.clone()
                };
                let row_me = me.clone();
                let on_pick = dest.clone();
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(9.0))
                        .h(px(32.0))
                        .px(px(10.0))
                        .rounded(px(6.0))
                        .text_color(t.tx2)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover).text_color(t.tx))
                        .child(icon(
                            if dest.is_empty() {
                                "folder-open"
                            } else {
                                "folder"
                            },
                            15.0,
                            t.tx3,
                        ))
                        .child(div().min_w_0().truncate().child(label))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            let dest = on_pick.clone();
                            row_me.update(cx, |app, cx| move_from_picker(app, dest, cx));
                        }),
                );
            }
            let name = match items.as_slice() {
                [(one, _)] => pure::name_of(one).to_string(),
                many => format!("{} 项", many.len()),
            };
            let body = div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(div().text_color(t.tx2).child(format!("移动「{name}」到：")))
                .child(filter.clone())
                .child(list);
            let cancel_me = me.clone();
            let footer = vec![
                button(
                    &t,
                    "取消",
                    ButtonKind::Secondary,
                    None,
                    move |_, _, cx| {
                        cancel_me.update(cx, |app, cx| {
                            app.ext.files.modal = None;
                            cx.notify();
                        });
                    },
                )
                .into_any_element(),
            ];
            modal(&t, "移动到", body, footer, 420.0, on_close)
        }
        Modal::Diff { path, rows } => {
            let name = pure::name_of(path).to_string();
            let body = diff_body(&t, rows.as_deref());
            let reload_me = me.clone();
            let reload_path = path.clone();
            let keep_me = me.clone();
            let keep_path = path.clone();
            let footer = vec![
                button(
                    &t,
                    "重新加载（丢弃我的修改）",
                    ButtonKind::Ghost,
                    None,
                    move |_, _, cx| {
                        let path = reload_path.clone();
                        reload_me.update(cx, |app, cx| {
                            app.ext.files.modal = None;
                            watch::discard_and_reload(app, &path, cx);
                        });
                    },
                )
                .into_any_element(),
                button(
                    &t,
                    "保留我的修改",
                    ButtonKind::Secondary,
                    None,
                    move |_, _, cx| {
                        let path = keep_path.clone();
                        keep_me.update(cx, |app, cx| {
                            app.ext.files.modal = None;
                            watch::keep_mine(app, &path, cx);
                        });
                    },
                )
                .into_any_element(),
            ];
            modal(&t, &format!("差异：{name}"), body, footer, 760.0, on_close)
        }
        Modal::Recovery { items, diff } => {
            let (title, body, footer) = recovery::view(app, items, diff.as_deref(), cx);
            modal(&t, &title, body, footer, 640.0, on_close)
        }
        Modal::Confirm(confirm) => {
            let key = confirm.clone_key();
            let (title, message, label, danger) = confirm_texts(&key);
            let confirm_me = me.clone();
            let cancel_me = me.clone();
            let on_confirm = move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                let key = key.clone();
                confirm_me.update(cx, |app, cx| run_confirm(app, key, cx));
            };
            let on_cancel = move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                cancel_me.update(cx, |app, cx| {
                    app.ext.files.modal = None;
                    cx.notify();
                });
            };
            crate::ui::widgets::overlay::confirm_dialog(
                &t, &title, &message, &label, danger, on_confirm, on_cancel,
            )
        }
    };
    Some(el)
}

/// Title, message, button label and danger flag of a confirmation.
fn confirm_texts(confirm: &ConfirmKey) -> (String, String, String, bool) {
    match confirm {
        ConfirmKey::Delete { path, .. } => (
            "移到回收站".to_string(),
            format!("将「{}」移到回收站，之后仍可以恢复。", pure::name_of(path)),
            "移到回收站".to_string(),
            true,
        ),
        ConfirmKey::DeleteMany(items) => (
            "移到回收站".to_string(),
            format!("将选中的 {} 项移到回收站，之后仍可以恢复。", items.len()),
            "移到回收站".to_string(),
            true,
        ),
        ConfirmKey::Purge { name, .. } => (
            "永久删除".to_string(),
            format!("「{name}」将被永久删除，无法恢复。"),
            "永久删除".to_string(),
            true,
        ),
        ConfirmKey::Empty(count) => (
            "清空回收站".to_string(),
            format!("回收站中的 {count} 个项目将被永久删除，无法恢复。"),
            "清空".to_string(),
            true,
        ),
        ConfirmKey::DiscardRecovery(paths) => (
            "全部丢弃未保存的内容".to_string(),
            format!(
                "{} 个笔记的未保存内容将被删除。磁盘上的文件不受影响。",
                paths.len()
            ),
            "全部丢弃".to_string(),
            true,
        ),
        ConfirmKey::CloseDespiteFailures(count) => (
            "仍然关闭？".to_string(),
            format!("{count} 个笔记保存失败。它们的未保存内容已写入恢复文件，下次启动时可以恢复。"),
            "仍然关闭".to_string(),
            true,
        ),
    }
}

/// The confirmation's data without the trash entry's borrow, so the buttons can own it.
#[derive(Clone)]
enum ConfirmKey {
    Delete { path: String, is_dir: bool },
    DeleteMany(Vec<(String, bool)>),
    Purge { entry: TrashEntry, name: String },
    Empty(usize),
    DiscardRecovery(Vec<String>),
    CloseDespiteFailures(usize),
}

trait CloneKey {
    fn clone_key(&self) -> ConfirmKey;
}

impl CloneKey for Confirm {
    fn clone_key(&self) -> ConfirmKey {
        match self {
            Confirm::Delete { path, is_dir } => ConfirmKey::Delete {
                path: path.clone(),
                is_dir: *is_dir,
            },
            Confirm::DeleteMany(items) => ConfirmKey::DeleteMany(items.clone()),
            Confirm::Purge(entry) => ConfirmKey::Purge {
                entry: entry.clone(),
                name: entry.original.clone(),
            },
            Confirm::Empty(count) => ConfirmKey::Empty(*count),
            Confirm::DiscardRecovery(paths) => ConfirmKey::DiscardRecovery(paths.clone()),
            Confirm::CloseDespiteFailures(count) => ConfirmKey::CloseDespiteFailures(*count),
        }
    }
}

/// Enter on an open confirmation runs its red action, as a click on the button does. False when
/// no confirmation is open.
pub fn confirm_enter(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) -> bool {
    let key = match app.ext.files.modal.as_ref() {
        Some(Modal::Confirm(confirm)) => confirm.clone_key(),
        _ => return false,
    };
    run_confirm(app, key, cx);
    true
}

/// Runs a confirmed action and closes the modal.
fn run_confirm(app: &mut RustdianApp, confirm: ConfirmKey, cx: &mut Context<RustdianApp>) {
    app.ext.files.modal = None;
    match confirm {
        ConfirmKey::Delete { path, is_dir } => {
            if let Err(message) = actions::delete_item(app, &path, is_dir, cx) {
                app.toast(ToastKind::Error, message);
            }
        }
        ConfirmKey::DeleteMany(items) => {
            app.ext.files.selection.clear();
            actions::delete_items(app, &items, cx);
        }
        ConfirmKey::Purge { entry, .. } => {
            match purge_entry(app, &entry) {
                Ok(()) => app.toast(ToastKind::Success, "已永久删除"),
                Err(message) => app.toast(ToastKind::Error, message),
            }
            open_trash(app);
        }
        ConfirmKey::DiscardRecovery(paths) => {
            for path in &paths {
                app.journals.remove(path);
            }
            app.toast(
                ToastKind::Info,
                format!("已丢弃 {} 个笔记的未保存内容", paths.len()),
            );
        }
        ConfirmKey::CloseDespiteFailures(_) => {
            // The user accepts losing the unsaved text of the failed notes from the open window;
            // their recovery copies stay on disk. The close path sees the flag and lets the window go.
            app.force_close = true;
            cx.quit();
        }
        ConfirmKey::Empty(_) => {
            match fsops::empty_trash(&app.vault.root_dir) {
                Ok(count) => app.toast(ToastKind::Success, format!("已清空回收站（{count} 项）")),
                Err(err) => app.toast(ToastKind::Error, pure::fs_error_zh(&err)),
            }
            open_trash(app);
        }
    }
    cx.notify();
}

fn trash_body(
    t: &crate::theme::ActiveTheme,
    me: &Entity<RustdianApp>,
    entries: &[TrashEntry],
    offset: i64,
) -> AnyElement {
    if entries.is_empty() {
        return empty_state(
            t,
            "trash",
            "回收站是空的",
            "删除的笔记和文件夹会出现在这里。",
        )
        .into_any_element();
    }
    let mut list = div()
        .id("trash-list")
        .flex()
        .flex_col()
        .max_h(px(380.0))
        .overflow_y_scroll();
    for entry in entries {
        let (original, time) = pure::trash_row_texts(entry, offset);
        let restore_me = me.clone();
        let restore_entry = entry.clone();
        let purge_me = me.clone();
        let purge_entry_c = entry.clone();
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .py(px(8.0))
                .px(px(6.0))
                .border_b_1()
                .border_color(t.line_soft)
                .child(icon(
                    if entry.is_dir { "folder" } else { "file-text" },
                    16.0,
                    t.tx3,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(original),
                        )
                        .child(div().text_size(px(11.5)).text_color(t.tx3).child(time)),
                )
                .child(button(
                    t,
                    "恢复",
                    ButtonKind::Secondary,
                    None,
                    move |_, _, cx| {
                        let entry = restore_entry.clone();
                        restore_me.update(cx, |app, cx| restore_from_trash(app, &entry, cx));
                    },
                ))
                .child(icon_button(t, "trash", move |_, _, cx| {
                    let entry = purge_entry_c.clone();
                    purge_me.update(cx, |app, cx| {
                        app.ext.files.modal = Some(Modal::Confirm(Confirm::Purge(entry)));
                        cx.notify();
                    });
                })),
        );
    }
    list.into_any_element()
}

pub(super) fn diff_body(t: &crate::theme::ActiveTheme, rows: Option<&[DiffRow]>) -> AnyElement {
    let Some(rows) = rows else {
        return empty_state(
            t,
            "info-circle",
            "文件太大，无法显示差异",
            "请使用「重新加载」或「保留我的修改」。",
        )
        .into_any_element();
    };
    let mut column = div()
        .flex()
        .flex_col()
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(12.5));
    column = column.child(
        div()
            .flex()
            .gap(px(8.0))
            .pb(px(6.0))
            .text_color(t.tx3)
            .child(div().flex_1().child("磁盘上的文件"))
            .child(div().flex_1().child("编辑器中的内容")),
    );
    for row in rows.iter().take(DIFF_ROW_LIMIT) {
        let (left_bg, right_bg) = match row.kind {
            DiffKind::Same => (None, None),
            DiffKind::Removed => (Some(t.hover), None),
            DiffKind::Added => (None, Some(t.hover)),
        };
        let mut left = div().flex_1().min_w_0().px(px(6.0)).child(row.left.clone());
        if let Some(bg) = left_bg {
            left = left.bg(bg);
        }
        let mut right = div()
            .flex_1()
            .min_w_0()
            .px(px(6.0))
            .child(row.right.clone());
        if let Some(bg) = right_bg {
            right = right.bg(bg);
        }
        column = column.child(div().flex().gap(px(8.0)).child(left).child(right));
    }
    if rows.len() > DIFF_ROW_LIMIT {
        column = column.child(div().pt(px(8.0)).text_color(t.tx3).child(format!(
            "仅显示前 {DIFF_ROW_LIMIT} 行，共 {} 行。",
            rows.len()
        )));
    }
    div()
        .id("diff-list")
        .max_h(px(440.0))
        .overflow_y_scroll()
        .child(column)
        .into_any_element()
}

/// Offset of local time from UTC in seconds (east is positive). Zero where it cannot be read.
pub fn local_offset_secs() -> i64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[allow(non_snake_case)]
        struct WinTime {
            wYear: u16,
            wMonth: u16,
            wDayOfWeek: u16,
            wDay: u16,
            wHour: u16,
            wMinute: u16,
            wSecond: u16,
            wMilliseconds: u16,
        }
        unsafe extern "system" {
            fn GetLocalTime(lp_system_time: *mut WinTime);
        }
        let mut st = WinTime {
            wYear: 1970,
            wMonth: 1,
            wDayOfWeek: 0,
            wDay: 1,
            wHour: 0,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        };
        // SAFETY: GetLocalTime writes one SYSTEMTIME, whose layout matches `WinTime`, through the
        // pointer. `st` is a live local that is exclusively borrowed for the call.
        unsafe {
            GetLocalTime(&mut st as *mut WinTime);
        }
        let days = Date::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32)
            .map(|d| d.to_days())
            .unwrap_or(0);
        let local =
            days * 86_400 + st.wHour as i64 * 3_600 + st.wMinute as i64 * 60 + st.wSecond as i64;
        let utc = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        local - utc
    }
    #[cfg(not(windows))]
    {
        0
    }
}
