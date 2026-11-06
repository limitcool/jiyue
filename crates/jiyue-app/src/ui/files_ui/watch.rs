//! Vault watching: drains the watcher channel on a timer, reloads the note list after files were
//! created, removed or renamed, and reconciles notes changed on disk. A note whose editor is clean
//! is reloaded silently; a note with unsaved edits gets the conflict bar in the breadcrumb.

use std::fs;
use std::sync::mpsc::Receiver;

use gpui::{AnyElement, Context, Task, div, prelude::*, px};
use jiyue_core::fsops;
use jiyue_core::watch::{VaultEvent, VaultWatcher};

use super::{POLL_INTERVAL, RECONCILE_DELAY, actions, modals, pure};
use crate::RustdianApp;
use crate::ui::UI_FONT;
use crate::ui::ext::{ExtEvent, ToastKind};
use crate::ui::icons::icon;
use crate::ui::widgets::controls::{ButtonKind, button};

/// The watcher, its event channel and the polling task. Dropping it stops the watch.
pub struct WatchService {
    rx: Receiver<VaultEvent>,
    _watcher: VaultWatcher,
    _poll: Task<()>,
}

/// Starts watching the vault. Called once, on `ExtEvent::Started`.
pub fn start(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let watcher = match VaultWatcher::start(&app.vault.root_dir, tx) {
        Ok(watcher) => watcher,
        Err(err) => {
            app.toast(ToastKind::Warn, format!("无法监听笔记库的文件变化：{err}"));
            return;
        }
    };
    let poll = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            if this.update(cx, drain).is_err() {
                break;
            }
        }
    });
    app.ext.files.watch = Some(WatchService {
        rx,
        _watcher: watcher,
        _poll: poll,
    });
}

/// Handles everything the watcher reported since the last tick.
fn drain(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let events: Vec<VaultEvent> = match app.ext.files.watch.as_ref() {
        Some(service) => service.rx.try_iter().collect(),
        None => Vec::new(),
    };
    if super::purge_undo(app) {
        cx.notify();
    }
    if !events.is_empty() {
        handle_events(app, events, cx);
        cx.notify();
    }
}

/// Applies a batch of watcher events. Structural changes reload the note list once; modified
/// notes are reconciled one by one. Other modules hear about the paths through `VaultFilesChanged`.
pub fn handle_events(
    app: &mut RustdianApp,
    events: Vec<VaultEvent>,
    cx: &mut Context<RustdianApp>,
) {
    let mut structural = false;
    let mut changed: Vec<String> = Vec::new();
    let mut modified: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let mut renames: Vec<(String, String)> = Vec::new();
    for event in events {
        match event {
            VaultEvent::Created(path) => {
                structural = true;
                changed.push(path);
            }
            VaultEvent::Removed(path) => {
                structural = true;
                changed.push(path.clone());
                removed.push(path);
            }
            VaultEvent::Renamed { from, to } => {
                structural = true;
                changed.push(from.clone());
                changed.push(to.clone());
                renames.push((from, to));
            }
            VaultEvent::Modified(path) => {
                if path.ends_with(".md") {
                    modified.push(path.clone());
                }
                changed.push(path);
            }
        }
    }
    changed.sort();
    changed.dedup();
    if changed.is_empty() {
        return;
    }

    // Renames of notes (a file, or a folder and every note below it): editors, tabs and the
    // remembered state follow the notes to their new paths before the list is reloaded.
    let before: Vec<String> = app.vault.notes.iter().map(|n| n.path.clone()).collect();
    let mut note_renames: Vec<(String, String)> = Vec::new();
    for (from, to) in &renames {
        note_renames.extend(pure::note_renames(&before, from, to));
    }
    actions::remap_state(app, &note_renames, None, cx);
    for (from, to) in renames.iter().filter(|(from, _)| !from.ends_with(".md")) {
        app.expanded_folders = pure::remap_keys(&app.expanded_folders, from, to);
    }

    // A dirty note whose file vanished (and that was not renamed) would lose its text when its
    // editor is dropped: write it back to its path.
    let dirty_paths: Vec<String> = app
        .editors
        .keys()
        .filter(|p| app.is_dirty(p, cx))
        .cloned()
        .collect();
    for path in dirty_paths {
        let gone = !app.vault.root_dir.join(&path).exists();
        let renamed = note_renames.iter().any(|(old, _)| *old == path);
        if gone && !renamed && removed.iter().any(|r| pure::is_under(&path, r)) {
            let text = app.editor_for(&path).map(|e| e.read(cx).text());
            if let Some(text) = text
                && fsops::write_atomic(&app.vault.root_dir.join(&path), text.as_bytes()).is_ok()
            {
                app.toast(
                    ToastKind::Warn,
                    format!("文件已在外部删除，未保存的内容已写回：{path}"),
                );
            }
        }
    }

    if structural {
        // Editors were re-keyed above, so their keys are the post-change paths.
        let keep: Vec<String> = app
            .editors
            .keys()
            .filter(|p| app.is_dirty(p, cx))
            .cloned()
            .collect();
        app.vault.reload_from_disk(&keep, &note_renames);
        let paths: Vec<String> = app.vault.notes.iter().map(|n| n.path.clone()).collect();
        app.ext.files.conflicts.retain(|p| paths.contains(p));
    }

    for path in modified {
        handle_modified(app, &path, cx);
    }
    app.vault.rebuild_graph();
    crate::ui::ext::on_event(app, &ExtEvent::VaultFilesChanged(changed), cx);
}

/// Reconciles one note with its file. Our own writes (the text we last saved) are ignored, and so
/// is a file that already equals the editor. Otherwise a clean editor is reloaded and a dirty one
/// gets the conflict bar.
pub fn handle_modified(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    let Ok(disk) = fs::read_to_string(app.vault.root_dir.join(path)) else {
        return;
    };
    if app
        .ext
        .files
        .last_written
        .get(path)
        .is_some_and(|w| *w == disk)
    {
        return;
    }
    let Some(idx) = app.vault.notes.iter().position(|n| n.path == path) else {
        return;
    };
    let editor = app.editor_for(path);
    if let Some(ed) = editor.as_ref() {
        let (text, dirty, cursor) = {
            let view = ed.read(cx);
            let restored = app.editors.get(path).is_some_and(|e| e.restored);
            (
                view.text(),
                view.is_dirty() || restored,
                view.cursor_row_col(),
            )
        };
        if text == disk {
            return;
        }
        if dirty {
            if !app.ext.files.conflicts.iter().any(|p| p == path) {
                app.ext.files.conflicts.push(path.to_string());
            }
            cx.notify();
            return;
        }
        let (row, col) = (cursor.0.saturating_sub(1), cursor.1.saturating_sub(1));
        ed.update(cx, |ed, cx| {
            ed.set_text(&disk, cx);
            ed.set_caret_to_row_col(row, col, cx);
        });
        if let Some(entry) = app.editors.get_mut(path) {
            entry.dirty_seen = false;
        }
    }
    app.vault.notes[idx].set_content(&disk);
    if editor.is_some() {
        let title = app.vault.notes[idx].title.clone();
        app.toast(ToastKind::Info, format!("已从磁盘更新：{title}"));
    }
    cx.notify();
}

/// Records what this process just saved to `path`, and schedules a re-read a little later. The
/// watcher drops events for our own writes for two seconds, so an external edit made in that
/// window would otherwise go unnoticed.
pub fn note_saved(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    if let Some(note) = app.vault.notes.iter().find(|n| n.path == path) {
        let text = note.content().to_string();
        app.ext.files.last_written.insert(path.to_string(), text);
    }
    let key = path.to_string();
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(RECONCILE_DELAY).await;
        let _ = this.update(cx, |app, cx| handle_modified(app, &key, cx));
    })
    .detach();
}

/// The active note's disk file replaces its editor text, unsaved edits included.
pub fn discard_and_reload(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    let Ok(disk) = fs::read_to_string(app.vault.root_dir.join(path)) else {
        app.toast(ToastKind::Error, "无法读取磁盘上的文件");
        return;
    };
    app.ext.files.conflicts.retain(|p| p != path);
    if let Some(ed) = app.editor_for(path) {
        ed.update(cx, |ed, cx| ed.set_text(&disk, cx));
    }
    if let Some(entry) = app.editors.get_mut(path) {
        entry.dirty_seen = false;
    }
    if let Some(idx) = app.vault.notes.iter().position(|n| n.path == path) {
        app.vault.notes[idx].set_content(&disk);
    }
    // The unsaved text is gone: the note is clean and its recovery copy is no longer needed.
    if let Some(entry) = app.editors.get_mut(path) {
        entry.restored = false;
        entry.base_mtime_ms = None;
        entry.save_failing = false;
    }
    app.journals.remove(path);
    app.ext.files.last_written.insert(path.to_string(), disk);
    cx.notify();
}

/// Keeps the editor text. The conflict bar goes away; autosave writes the editor text.
pub fn keep_mine(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    app.ext.files.conflicts.retain(|p| p != path);
    cx.notify();
}

/// The conflict bar for the active note, when its file changed on disk under unsaved edits.
pub fn conflict_bar(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    let path = app.active_path()?.to_string();
    if !app.ext.files.conflicts.contains(&path) {
        return None;
    }
    let t = app.theme;
    let me = cx.entity();
    let (reload_me, keep_me, diff_me) = (me.clone(), me.clone(), me.clone());
    let (reload_path, keep_path) = (path.clone(), path);
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.0))
            .h(px(40.0))
            .px(px(14.0))
            .bg(t.ac_soft)
            .border_b_1()
            .border_color(t.line_soft)
            .font_family(UI_FONT)
            .text_size(px(12.5))
            .text_color(t.tx)
            .child(icon("info-circle", 15.0, t.ac))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child("磁盘上的文件已被外部修改"),
            )
            .child(button(
                &t,
                "重新加载（丢弃我的修改）",
                ButtonKind::Ghost,
                None,
                move |_, _, cx| {
                    let path = reload_path.clone();
                    reload_me.update(cx, |app, cx| discard_and_reload(app, &path, cx));
                },
            ))
            .child(button(
                &t,
                "保留我的修改",
                ButtonKind::Ghost,
                None,
                move |_, _, cx| {
                    let path = keep_path.clone();
                    keep_me.update(cx, |app, cx| keep_mine(app, &path, cx));
                },
            ))
            .child(button(
                &t,
                "查看差异",
                ButtonKind::Secondary,
                None,
                move |_, _, cx| {
                    diff_me.update(cx, modals::open_diff);
                },
            ))
            .into_any_element(),
    )
}
