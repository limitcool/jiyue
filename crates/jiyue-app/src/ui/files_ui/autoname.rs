//! Automatic file name of a new note. A note created in this session as `未命名 N` takes the text of
//! its first `# Heading` as its file name once typing pauses (`AUTO_NAME_DELAY`), through the same
//! rename as the file tree (links, history, bookmarks and the undo toast follow it). The name
//! follows the heading while the user leaves the file name alone: after an automatic rename the
//! note stays in the session with the name that was produced; a later heading change renames it
//! again, and a name the user gave it drops the note for good. Only notes created in this session
//! qualify: the record is not persisted, so after a restart nothing is renamed.
//! The switch `新建笔记时用首个标题作为文件名` lives in the `[editor]` section of `.jiyue/config.toml`.
//!
//! The rules are in `autoname/pure.rs`.

pub mod pure;

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::time::Duration;

use gpui::{Context, Task, Window};
use serde::{Deserialize, Serialize};

use super::actions;
use super::with_window;
use crate::RustdianApp;
use crate::session::Debounce;
use crate::ui::ext::ToastKind;
use pure::{Action, Check};

/// Typing must pause this long before the note is renamed.
pub const AUTO_NAME_DELAY: Duration = Duration::from_millis(2000);
/// The vault configuration file, relative to the vault root (the `[editor]` section holds the switch).
pub const SETTINGS_FILE: &str = ".jiyue/config.toml";
/// Section of the vault configuration file that holds these settings.
pub const SECTION: &str = "editor";

/// State of the automatic rename. Lives in `files_ui::State::autoname`; not persisted.
#[derive(Default)]
pub struct Session {
    /// The notes created in this session, by path. The value is the path the automatic rename
    /// produced (`None` until the note was renamed automatically).
    created: HashMap<String, Option<String>>,
    /// The note and heading of the last try (a failed try is not repeated for the same heading).
    last_tried: Option<(String, String)>,
    debounce: Debounce,
    task: Option<Task<()>>,
    /// The automatic rename is running (renames made now keep the note in `created`).
    renaming: bool,
}

/// The vault's editor settings (the `[editor]` section of `.jiyue/config.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// `新建笔记时用首个标题作为文件名`: on by default.
    pub auto_name_from_heading: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            auto_name_from_heading: true,
        }
    }
}

/// Reads the editor settings of the vault at `root`. A missing or broken file gives the defaults.
pub fn load(root: &Path) -> EditorSettings {
    jiyue_core::config::load_section::<EditorSettings>(&root.join(SETTINGS_FILE), SECTION).value
}

/// Writes the editor settings to the `[editor]` section, keeping the rest of the file.
pub fn save(root: &Path, settings: &EditorSettings) -> io::Result<()> {
    jiyue_core::config::save_section(
        &root.join(SETTINGS_FILE),
        SECTION,
        settings,
        jiyue_core::config::templates::VAULT_CONFIG,
        &[],
    )
    .map_err(io::Error::other)
}

/// Flips the switch and saves it to the vault.
pub fn toggle(app: &mut RustdianApp) {
    let root = app.vault.root_dir.clone();
    let mut settings = load(&root);
    settings.auto_name_from_heading = !settings.auto_name_from_heading;
    if let Err(err) = save(&root, &settings) {
        app.toast(ToastKind::Error, format!("无法保存设置：{err}"));
    }
}

/// A note was created in this session (new note, or a new note in the file tree).
pub fn track_new(app: &mut RustdianApp, rel: &str) {
    app.ext.files.autoname.created.insert(rel.to_string(), None);
}

/// Renames of notes (`(old, new)` paths) move the session's record with them. The automatic
/// rename keeps the note with its produced name; a rename the user made drops the note.
pub fn after_rename(app: &mut RustdianApp, renames: &[(String, String)]) {
    let session = &mut app.ext.files.autoname;
    for (old, new) in renames {
        if session.created.remove(old).is_some() && session.renaming {
            session.created.insert(new.clone(), Some(new.clone()));
        }
    }
    if let Some((path, heading)) = session.last_tried.clone()
        && let Some((_, new)) = renames.iter().find(|(old, _)| *old == path)
    {
        session.last_tried = Some((new.clone(), heading));
    }
}

/// The note at `path` changed its text. A session note starts (or restarts) the pause timer.
pub fn on_edit(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    if app.ext.files.autoname.created.contains_key(path) {
        schedule(app, path, cx);
    }
}

/// Starts the pause timer for `path`. Replacing the stored task cancels the previous timer.
fn schedule(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    let session = &mut app.ext.files.autoname;
    let token = session.debounce.restart();
    let key = path.to_string();
    session.task = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(AUTO_NAME_DELAY).await;
        let _ = this.update(cx, |app, cx| {
            if app.ext.files.autoname.debounce.is_current(token) {
                let key = key.clone();
                // The rename needs a window: it runs with the active one, as the file tree does.
                with_window(cx, move |app, window, cx| run(app, window, &key, token, cx));
            }
        });
    }));
}

/// The timer fired: renames the note, drops it, or leaves it, as `pure::decide` says.
fn run(
    app: &mut RustdianApp,
    window: &mut Window,
    path: &str,
    token: u64,
    cx: &mut Context<RustdianApp>,
) {
    let session = &app.ext.files.autoname;
    if !session.debounce.is_current(token) || !session.created.contains_key(path) {
        return;
    }
    let Some(idx) = app.vault.notes.iter().position(|n| n.path == path) else {
        return;
    };
    let composing = app
        .editor_for(path)
        .is_some_and(|editor| editor.read(cx).is_composing());
    if composing {
        // Never rename in the middle of an IME composition: look again after the next pause.
        schedule(app, path, cx);
        return;
    }
    let enabled = load(&app.vault.root_dir).auto_name_from_heading;
    let heading = pure::heading_text(app.vault.notes[idx].content());
    let (dir, stem) = pure::split_note_path(path);
    let (dir, stem) = (dir.to_string(), stem.to_string());
    let auto_stem = session
        .created
        .get(path)
        .cloned()
        .flatten()
        .map(|produced| pure::split_note_path(&produced).1.to_string());
    let last_tried = session
        .last_tried
        .as_ref()
        .filter(|(p, _)| p == path)
        .map(|(_, h)| h.clone());
    let check = Check {
        enabled,
        created_here: true,
        stem: &stem,
        auto_stem: auto_stem.as_deref(),
        heading: heading.as_deref(),
        last_tried: last_tried.as_deref(),
        composing: false,
    };
    let root = app.vault.root_dir.clone();
    let taken = |name: &str| {
        let rel = format!("{dir}{name}.md");
        app.vault
            .notes
            .iter()
            .any(|n| n.path.eq_ignore_ascii_case(&rel))
            || root.join(&rel).exists()
    };
    match pure::decide(&check, taken) {
        Action::Keep => {}
        Action::Drop => {
            app.ext.files.autoname.created.remove(path);
        }
        Action::Rename(new_stem) => {
            let new_rel = format!("{dir}{new_stem}.md");
            let heading = heading.unwrap_or_default();
            app.ext.files.autoname.last_tried = Some((path.to_string(), heading));
            app.ext.files.autoname.renaming = true;
            let result = actions::rename(app, window, path, &new_rel, false, cx);
            app.ext.files.autoname.renaming = false;
            if let Err(message) = result {
                app.toast(ToastKind::Error, message);
            }
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn missing_or_broken_file_gives_the_default_on() {
        let dir = std::env::temp_dir().join("jiyue-editor-settings-missing-xyz");
        assert!(load(&dir).auto_name_from_heading);
        let parsed: EditorSettings = serde_json::from_str("{not json").unwrap_or_default();
        assert!(parsed.auto_name_from_heading);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!(
            "jiyue-editor-settings-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let off = EditorSettings {
            auto_name_from_heading: false,
        };
        save(&dir, &off).expect("save");
        assert!(dir.join(".jiyue").join("config.toml").is_file());
        assert_eq!(load(&dir), off);
        let _ = fs::remove_dir_all(&dir);
    }
}
