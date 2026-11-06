//! File operations: create note and folder, rename and move (with link updates), delete to the
//! trash and restore, copy path, reveal in Explorer, and the undo of the last operation.
//!
//! Every operation keeps the app state in step with the disk: dirty editors are saved first, the
//! open editors and tabs follow their notes by path, and the note list is re-read from disk.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, ClipboardItem, Context, EntityInputHandler, SharedString, Window, div, prelude::*,
    px,
};
use jiyue_core::fsops::{self, TrashEntry};
use jiyue_core::links::{self, NoteRef};

use super::autoname;
use super::pure::{self, MoveCheck};
use crate::RustdianApp;
use crate::ui::ext::ToastKind;
use crate::ui::history_ui;
use crate::ui::widgets::controls::{ButtonKind, button};

/// What an undo does.
pub enum UndoOp {
    /// Put a trash entry back.
    Restore(TrashEntry),
    /// Put back the entries of a batch delete.
    RestoreMany(Vec<TrashEntry>),
    /// Rename `new` back to `old`, and put the texts of the link-updated notes back.
    Rename(RenameStep),
    /// Undo a batch move as one step: every move is undone, the last one first.
    RenameMany(Vec<RenameStep>),
}

/// One rename of an undo. `texts` holds `(path after the rename, text before it)` for the notes
/// whose links changed.
pub struct RenameStep {
    pub old: String,
    pub new: String,
    pub is_dir: bool,
    pub texts: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Creates the note `title` in `dir` and opens it in Source mode with the caret in the body.
pub fn create_note(
    app: &mut RustdianApp,
    dir: &str,
    title: &str,
    cx: &mut Context<RustdianApp>,
) -> Result<String, String> {
    let root = app.vault.root_dir.clone();
    let rel = fsops::create_note(&root, dir, title).map_err(|err| pure::fs_error_zh(&err))?;
    autoname::track_new(app, &rel);
    super::expand_folder(app, dir);
    reload(app, &[], cx);
    if let Some(idx) = app.note_index(&rel) {
        app.open_note_by_index(idx);
        app.pending_new_note = true;
    }
    super::reveal_path(app, &rel);
    cx.notify();
    Ok(rel)
}

/// Creates the folder `rel` (missing parents too).
pub fn create_folder(
    app: &mut RustdianApp,
    rel: &str,
    cx: &mut Context<RustdianApp>,
) -> Result<(), String> {
    let root = app.vault.root_dir.clone();
    fsops::create_folder(&root, rel).map_err(|err| pure::fs_error_zh(&err))?;
    reload(app, &[], cx);
    super::expand_folder(app, rel);
    app.ext.files.selected = Some(rel.to_string());
    cx.notify();
    Ok(())
}

// ---------------------------------------------------------------------------
// Rename and move
// ---------------------------------------------------------------------------

/// Renames a note or folder and updates the `[[links]]` and relative Markdown links that pointed
/// at it, without an undo toast. Returns the undo step and how many notes changed, or `None` when
/// the name does not change.
fn rename_step(
    app: &mut RustdianApp,
    window: &mut Window,
    old: &str,
    new: &str,
    is_dir: bool,
    cx: &mut Context<RustdianApp>,
) -> Result<Option<(RenameStep, usize)>, String> {
    if old == new {
        return Ok(None);
    }
    fsops::safe_rel_path(new).map_err(|err| pure::fs_error_zh(&err))?;
    let root = app.vault.root_dir.clone();
    // Dirty editors are written first, so the plan and the disk agree.
    flush_dirty(app, cx);
    let refs = note_refs(app, &root);
    let texts: Vec<(String, String)> = app
        .vault
        .notes
        .iter()
        .map(|n| (n.path.clone(), n.content().to_string()))
        .collect();
    let edits = links::plan_rename_updates(texts.into_iter(), old, new, &refs);

    fsops::rename(&root, old, new).map_err(|err| pure::fs_error_zh(&err))?;

    let paths: Vec<String> = app.vault.notes.iter().map(|n| n.path.clone()).collect();
    let renames = pure::note_renames(&paths, old, new);
    // The history follows the notes to their new paths (the text is still the pre-rename text).
    history_ui::before_rename(app, &renames, cx);
    remap_state(app, &renames, Some((old, new)), cx);
    crate::ui::ext::bookmarks::on_rename(app, &renames, Some((old, new)));
    if is_dir {
        app.expanded_folders = pure::remap_keys(&app.expanded_folders, old, new);
    }
    reload(app, &renames, cx);

    let apply: Vec<(String, String)> = edits
        .iter()
        .map(|e| (e.path.clone(), e.new_text.clone()))
        .collect();
    let linked: Vec<String> = apply.iter().map(|(p, _)| p.clone()).collect();
    history_ui::before_change(app, &linked, cx);
    let before = apply_texts(app, window, &apply, cx);
    let step = RenameStep {
        old: old.to_string(),
        new: new.to_string(),
        is_dir,
        texts: before,
    };
    Ok(Some((step, edits.len())))
}

/// Renames a note or folder and updates the links that pointed at it (see `rename_step`). The
/// toast offers an undo. Returns how many notes changed.
pub fn rename(
    app: &mut RustdianApp,
    window: &mut Window,
    old: &str,
    new: &str,
    is_dir: bool,
    cx: &mut Context<RustdianApp>,
) -> Result<usize, String> {
    let Some((step, count)) = rename_step(app, window, old, new, is_dir, cx)? else {
        return Ok(0);
    };
    let text = rename_toast_text(old, new, count);
    super::set_undo(app, text, UndoOp::Rename(step));
    app.ext.files.selected = Some(new.to_string());
    cx.notify();
    Ok(count)
}

/// Moves a note or folder into `dest_dir` (`""` for the root), keeping its name.
pub fn move_item(
    app: &mut RustdianApp,
    window: &mut Window,
    src: &str,
    is_dir: bool,
    dest_dir: &str,
    cx: &mut Context<RustdianApp>,
) -> Result<(), String> {
    match pure::move_target(src, is_dir, dest_dir) {
        MoveCheck::NoOp => Ok(()),
        MoveCheck::Invalid(message) => Err(message),
        MoveCheck::Move(new) => {
            super::expand_folder(app, dest_dir);
            rename(app, window, src, &new, is_dir, cx).map(|_| ())
        }
    }
}

// ---------------------------------------------------------------------------
// Delete, restore
// ---------------------------------------------------------------------------

/// Moves a note or folder to `.trash`. The toast offers an undo.
pub fn delete_item(
    app: &mut RustdianApp,
    path: &str,
    is_dir: bool,
    cx: &mut Context<RustdianApp>,
) -> Result<(), String> {
    let root = app.vault.root_dir.clone();
    flush_dirty(app, cx);
    let entry = fsops::delete_to_trash(&root, path).map_err(|err| pure::fs_error_zh(&err))?;
    // Every note inside the item (the item itself, or the notes of a folder) keeps its last text.
    let gone: Vec<String> = app
        .vault
        .notes
        .iter()
        .filter(|n| pure::is_under(&n.path, path))
        .map(|n| n.path.clone())
        .collect();
    history_ui::before_delete(app, &gone, cx);
    crate::ui::nav_history::forget(app, path);
    if is_dir {
        app.expanded_folders.retain(|k| !pure::is_under(k, path));
    }
    if app
        .ext
        .files
        .selected
        .as_deref()
        .is_some_and(|s| pure::is_under(s, path))
    {
        app.ext.files.selected = None;
    }
    reload(app, &[], cx);
    super::set_undo(
        app,
        format!("已移到回收站：{}", pure::name_of(path)),
        UndoOp::Restore(entry),
    );
    cx.notify();
    Ok(())
}

/// Moves several rows (`(path, is_dir)`, effective roots) to the trash. A row that fails stays where
/// it is and is counted; the others still go. One undo toast restores all of them.
pub fn delete_items(
    app: &mut RustdianApp,
    items: &[(String, bool)],
    cx: &mut Context<RustdianApp>,
) {
    let root = app.vault.root_dir.clone();
    flush_dirty(app, cx);
    let mut entries = Vec::new();
    let mut done: Vec<String> = Vec::new();
    let mut failed = 0usize;
    for (path, _) in items {
        match fsops::delete_to_trash(&root, path) {
            Ok(entry) => {
                entries.push(entry);
                done.push(path.clone());
            }
            Err(_) => failed += 1,
        }
    }
    if done.is_empty() {
        app.toast(ToastKind::Error, format!("删除失败：{failed} 项"));
        cx.notify();
        return;
    }
    let gone: Vec<String> = app
        .vault
        .notes
        .iter()
        .filter(|n| done.iter().any(|d| pure::is_under(&n.path, d)))
        .map(|n| n.path.clone())
        .collect();
    history_ui::before_delete(app, &gone, cx);
    for path in &done {
        app.expanded_folders.retain(|k| !pure::is_under(k, path));
        crate::ui::nav_history::forget(app, path);
    }
    if app
        .ext
        .files
        .selected
        .as_deref()
        .is_some_and(|s| done.iter().any(|d| pure::is_under(s, d)))
    {
        app.ext.files.selected = None;
    }
    reload(app, &[], cx);
    let count = entries.len();
    super::set_undo(
        app,
        format!("已移到回收站：{count} 项"),
        UndoOp::RestoreMany(entries),
    );
    if failed > 0 {
        app.toast(
            ToastKind::Error,
            format!("已移到回收站 {count} 项，{failed} 项失败"),
        );
    }
    cx.notify();
}

/// Puts a trash entry back where it came from. Returns the new vault-relative path.
pub fn restore_entry(
    app: &mut RustdianApp,
    entry: &TrashEntry,
    cx: &mut Context<RustdianApp>,
) -> Result<String, String> {
    let root = app.vault.root_dir.clone();
    let rel = fsops::restore_from_trash(&root, entry).map_err(|err| pure::fs_error_zh(&err))?;
    super::expand_folder(app, pure::parent_of(&rel));
    reload(app, &[], cx);
    super::reveal_path(app, &rel);
    cx.notify();
    Ok(rel)
}

// ---------------------------------------------------------------------------
// Copy path, reveal, undo
// ---------------------------------------------------------------------------

/// Copies the absolute path of `rel` to the clipboard.
pub fn copy_path(app: &mut RustdianApp, rel: &str, cx: &mut Context<RustdianApp>) {
    let abs = abs_path(&app.vault.root_dir, rel);
    cx.write_to_clipboard(ClipboardItem::new_string(abs.display().to_string()));
    app.toast(ToastKind::Success, "已复制路径");
    cx.notify();
}

/// Copies the absolute paths of several rows, one per line.
pub fn copy_paths(app: &mut RustdianApp, rels: &[String], cx: &mut Context<RustdianApp>) {
    let text = super::batch::paths_text(&app.vault.root_dir, rels);
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    app.toast(ToastKind::Success, format!("已复制 {} 个路径", rels.len()));
    cx.notify();
}

/// Moves several rows (the effective roots of a selection) into `dest_dir`. Each move goes through
/// `rename`, so links are updated as for a rename. A row that fails is counted and the rest still
/// move; a summary toast reports the counts.
pub fn move_items(
    app: &mut RustdianApp,
    window: &mut Window,
    items: &[(String, bool)],
    dest_dir: &str,
    cx: &mut Context<RustdianApp>,
) {
    let plan = {
        let vault = &app.vault;
        let taken = |p: &str| {
            vault.notes.iter().any(|n| n.path == p) || vault.folders.iter().any(|f| f.as_str() == p)
        };
        super::batch::plan_moves(items, dest_dir, &taken)
    };
    super::expand_folder(app, dest_dir);
    let mut moved = 0usize;
    let mut failed = plan.invalid.len();
    let mut steps = Vec::new();
    for (src, dst, is_dir) in &plan.moves {
        match rename_step(app, window, src, dst, *is_dir, cx) {
            Ok(step) => {
                moved += 1;
                if let Some((step, _)) = step {
                    steps.push(step);
                }
                app.ext.files.selected = Some(dst.clone());
            }
            Err(_) => failed += 1,
        }
    }
    // The whole batch is one undo step, so undo puts back every item.
    if !steps.is_empty() {
        let text = format!("已移动 {} 项", steps.len());
        super::set_undo(app, text, UndoOp::RenameMany(steps));
    }
    if moved == 0 && failed == 0 {
        cx.notify();
        return;
    }
    let (kind, text) = if failed == 0 {
        (ToastKind::Success, format!("已移动 {moved} 项"))
    } else {
        (
            ToastKind::Error,
            format!("已移动 {moved} 项，{failed} 项失败"),
        )
    };
    app.toast(kind, text);
    cx.notify();
}

/// Shows `rel` in the system file manager (Explorer, Finder or the Linux file manager).
pub fn reveal(app: &mut RustdianApp, rel: &str) {
    let abs = abs_path(&app.vault.root_dir, rel);
    crate::ui::ext::reveal_in_explorer(&abs);
}

/// Reverts the operation of the undo toast.
pub fn run_undo(app: &mut RustdianApp, window: &mut Window, cx: &mut Context<RustdianApp>) {
    let Some(toast) = app.ext.files.undo.take() else {
        return;
    };
    match toast.op {
        UndoOp::Restore(entry) => match restore_entry(app, &entry, cx) {
            Ok(rel) => app.toast(ToastKind::Success, format!("已恢复：{rel}")),
            Err(message) => app.toast(ToastKind::Error, message),
        },
        UndoOp::RestoreMany(entries) => {
            let mut failed = 0usize;
            for entry in &entries {
                if restore_entry(app, entry, cx).is_err() {
                    failed += 1;
                }
            }
            let restored = entries.len() - failed;
            if failed == 0 {
                app.toast(ToastKind::Success, format!("已恢复 {restored} 项"));
            } else {
                app.toast(
                    ToastKind::Error,
                    format!("已恢复 {restored} 项，{failed} 项失败"),
                );
            }
        }
        UndoOp::Rename(step) => match undo_rename(app, window, step, cx) {
            Ok(()) => app.toast(ToastKind::Success, "已撤销重命名"),
            Err(message) => app.toast(ToastKind::Error, message),
        },
        UndoOp::RenameMany(steps) => {
            let total = steps.len();
            let mut failed = 0usize;
            // Last move first: each step expects the state its own move left behind.
            for step in steps.into_iter().rev() {
                if undo_rename(app, window, step, cx).is_err() {
                    failed += 1;
                }
            }
            let restored = total - failed;
            if failed == 0 {
                app.toast(ToastKind::Success, format!("已撤销移动 {restored} 项"));
            } else {
                app.toast(
                    ToastKind::Error,
                    format!("已撤销移动 {restored} 项，{failed} 项失败"),
                );
            }
        }
    }
    cx.notify();
}

fn undo_rename(
    app: &mut RustdianApp,
    window: &mut Window,
    step: RenameStep,
    cx: &mut Context<RustdianApp>,
) -> Result<(), String> {
    let RenameStep {
        old,
        new,
        is_dir,
        texts,
    } = step;
    let (old, new) = (old.as_str(), new.as_str());
    let root = app.vault.root_dir.clone();
    fsops::rename(&root, new, old).map_err(|err| pure::fs_error_zh(&err))?;
    let paths: Vec<String> = app.vault.notes.iter().map(|n| n.path.clone()).collect();
    let renames = pure::note_renames(&paths, new, old);
    history_ui::before_rename(app, &renames, cx);
    remap_state(app, &renames, Some((new, old)), cx);
    crate::ui::ext::bookmarks::on_rename(app, &renames, Some((new, old)));
    if is_dir {
        app.expanded_folders = pure::remap_keys(&app.expanded_folders, new, old);
    }
    reload(app, &renames, cx);
    let back: Vec<(String, String)> = texts
        .into_iter()
        .map(|(path, text)| (pure::map_under(&path, new, old).unwrap_or(path), text))
        .collect();
    let linked: Vec<String> = back.iter().map(|(p, _)| p.clone()).collect();
    history_ui::before_change(app, &linked, cx);
    apply_texts(app, window, &back, cx);
    app.ext.files.selected = Some(old.to_string());
    Ok(())
}

/// The undo toast, bottom left of the editor area.
pub fn undo_overlay(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    let toast = app.ext.files.undo.as_ref()?;
    let t = app.theme;
    let me = cx.entity();
    let left = if app.left_sidebar_open { 264.0 } else { 16.0 };
    Some(
        div()
            .absolute()
            .left(px(left))
            // Above the view-mode pill at the foot of the editor.
            .bottom(px(92.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(14.0))
            .py(px(8.0))
            .rounded(px(10.0))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .shadow_md()
            .font_family(crate::ui::fonts::ui_name())
            .text_size(px(13.0))
            .text_color(t.tx)
            .child(SharedString::from(toast.text.clone()))
            .child(button(
                &t,
                "撤销",
                ButtonKind::Ghost,
                None,
                move |_, window, cx| {
                    me.update(cx, |app, cx| run_undo(app, window, cx));
                },
            ))
            .into_any_element(),
    )
}

// ---------------------------------------------------------------------------
// State helpers shared with the watcher and the modals
// ---------------------------------------------------------------------------

/// Saves every editor with unsaved changes (before a file operation).
pub fn flush_dirty(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let dirty: Vec<String> = app
        .editors
        .keys()
        .filter(|p| app.is_dirty(p, cx))
        .cloned()
        .collect();
    for path in dirty {
        app.save_path(&path, cx);
    }
}

/// Re-reads the note list from disk. Notes with unsaved editor text keep it.
pub fn reload(app: &mut RustdianApp, renames: &[(String, String)], cx: &mut Context<RustdianApp>) {
    let keep: Vec<String> = app
        .editors
        .keys()
        .filter(|p| app.is_dirty(p, cx))
        .cloned()
        .collect();
    app.vault.reload_from_disk(&keep, renames);
}

/// Moves the state that refers to notes that were renamed: their editors (with their text and
/// dirty flag), the active note, the watcher's record of our writes, the selection and the
/// conflict list. `moved` is the item (file or folder) that was moved, for entries that are not
/// notes (the selection).
pub fn remap_state(
    app: &mut RustdianApp,
    renames: &[(String, String)],
    moved: Option<(&str, &str)>,
    cx: &mut Context<RustdianApp>,
) {
    let map = |p: &str| -> String {
        if let Some((_, new)) = renames.iter().find(|(old, _)| old == p) {
            return new.clone();
        }
        if let Some((from, to)) = moved
            && let Some(mapped) = pure::map_under(p, from, to)
        {
            return mapped;
        }
        p.to_string()
    };
    for (old, new) in renames {
        rekey_editor(app, old, new, cx);
    }
    autoname::after_rename(app, renames);
    // The pane without the focus keeps its tabs as paths: they follow the rename too.
    if let Some(split) = app.layout.split.as_mut() {
        crate::ui::split_model::remap_pane(&mut split.other, map);
    }
    if let Some(active) = app.last_active_path.clone() {
        app.last_active_path = Some(map(&active));
    }
    crate::ui::nav_history::remap(app, map);
    let written: Vec<(String, String)> = app.ext.files.last_written.drain().collect();
    for (path, text) in written {
        app.ext.files.last_written.insert(map(&path), text);
    }
    if let Some(selected) = app.ext.files.selected.clone() {
        app.ext.files.selected = Some(map(&selected));
    }
    for path in app.ext.files.conflicts.iter_mut() {
        *path = map(path);
    }
}

/// Moves a note's editor from `old` to `new`. The editor entity (text, caret, undo, dirty flag)
/// stays; its subscriptions are made again, because they name the note by path.
fn rekey_editor(app: &mut RustdianApp, old: &str, new: &str, cx: &mut Context<RustdianApp>) {
    let Some(entry) = app.editors.remove(old) else {
        return;
    };
    let editor = entry.editor.clone();
    let key = new.to_string();
    let observe = cx.observe(&editor, move |this, _, cx| this.on_editor_changed(&key, cx));
    let links = cx.subscribe(
        &editor,
        |this, _, ev: &crate::ui::text_input::OpenWikilink, cx| {
            crate::ui::attach_ui::navigate_link(this, &ev.0, cx);
        },
    );
    let dirty = editor.read(cx).is_dirty() || entry.restored;
    let mut moved = crate::NoteEditor::new(editor, observe, links);
    moved.dirty_seen = entry.dirty_seen;
    moved.restored = entry.restored;
    moved.base_mtime_ms = entry.base_mtime_ms;
    app.editors.insert(new.to_string(), moved);
    if dirty {
        // The recovery copy follows the note to its new path.
        app.journals.remove(old);
        app.schedule_autosave(new, cx);
        app.schedule_journal(new, cx);
    }
}

/// Writes `texts` (path, new text) into the notes. An open editor takes the text as an undoable
/// change, so it becomes dirty and autosave writes it. A closed note is written to its file at
/// once. Returns `(path, text before)` for the undo.
fn apply_texts(
    app: &mut RustdianApp,
    window: &mut Window,
    texts: &[(String, String)],
    cx: &mut Context<RustdianApp>,
) -> Vec<(String, String)> {
    let root = app.vault.root_dir.clone();
    let mut before = Vec::new();
    for (path, text) in texts {
        let Some(idx) = app.note_index(path) else {
            continue;
        };
        let old_text = app.vault.notes[idx].content().to_string();
        if old_text == *text {
            continue;
        }
        before.push((path.clone(), old_text));
        match app.editor_for(path) {
            Some(editor) => {
                let units = editor.read(cx).text().encode_utf16().count();
                editor.update(cx, |ed, cx| {
                    ed.replace_text_in_range(Some(0..units), text, window, cx);
                });
            }
            None => {
                if fsops::write_atomic(&root.join(path), text.as_bytes()).is_ok() {
                    app.ext
                        .files
                        .last_written
                        .insert(path.clone(), text.clone());
                }
            }
        }
        app.vault.notes[idx].set_content(text);
    }
    before
}

/// Every file of the vault (notes and attachments) as a link target. The resolver needs them all.
fn note_refs(app: &RustdianApp, root: &Path) -> Vec<NoteRef> {
    let mut paths: BTreeSet<String> = app.vault.notes.iter().map(|n| n.path.clone()).collect();
    walk_files(root, root, &mut paths);
    paths.into_iter().map(NoteRef::new).collect()
}

fn walk_files(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if fsops::is_ignored(&rel) {
            continue;
        }
        if path.is_dir() {
            walk_files(root, &path, out);
        } else {
            out.insert(rel);
        }
    }
}

/// The absolute path of a vault-relative path (made absolute when the vault folder is relative).
pub fn abs_path(root: &Path, rel: &str) -> PathBuf {
    let path = root.join(rel);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(&path))
            .unwrap_or(path)
    }
}

/// The toast after a rename or a move. The same parent folder means a rename; another parent
/// means a move, which names the folder it went to (`根目录` for the root).
fn rename_toast_text(old: &str, new: &str, links: usize) -> String {
    let dest = pure::parent_of(new);
    if pure::parent_of(old) == dest {
        if links == 0 {
            format!("已重命名为 {}", pure::name_of(new))
        } else {
            format!("已重命名，并更新了 {links} 个笔记中的链接")
        }
    } else if links == 0 {
        let folder = if dest.is_empty() {
            "根目录"
        } else {
            pure::name_of(dest)
        };
        format!("已移动到 {folder}")
    } else {
        format!("已移动，并更新了 {links} 个笔记中的链接")
    }
}

#[cfg(test)]
mod tests {
    use super::rename_toast_text;

    #[test]
    fn rename_in_the_same_folder_keeps_the_rename_wording() {
        assert_eq!(rename_toast_text("a/x.md", "a/y.md", 0), "已重命名为 y.md");
        assert_eq!(
            rename_toast_text("x.md", "y.md", 2),
            "已重命名，并更新了 2 个笔记中的链接"
        );
    }

    #[test]
    fn move_names_the_destination_folder() {
        assert_eq!(rename_toast_text("x.md", "a/b/x.md", 0), "已移动到 b");
        assert_eq!(rename_toast_text("a/b/x.md", "x.md", 0), "已移动到 根目录");
        assert_eq!(rename_toast_text("a/x.md", "a/b/x.md", 0), "已移动到 b");
    }

    #[test]
    fn move_with_updated_links_reports_both() {
        assert_eq!(
            rename_toast_text("a/x.md", "b/x.md", 3),
            "已移动，并更新了 3 个笔记中的链接"
        );
        assert_eq!(
            rename_toast_text("a/x.md", "x.md", 1),
            "已移动，并更新了 1 个笔记中的链接"
        );
    }
}
