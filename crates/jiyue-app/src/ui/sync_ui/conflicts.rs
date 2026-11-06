//! Conflict list: the conflicts of the last runs, persisted in `<vault>/.jiyue/sync/conflicts.json`,
//! and the three ways to resolve one. Resolving never deletes data: superseded copies move to
//! `.trash/sync/`. The resolver for conflict markers is pure and tested.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiyue_sync::merge::{MARKER_LOCAL, MARKER_REMOTE, MARKER_SPLIT};
use jiyue_sync::state::{now_ms, sync_dir, write_atomic};
use jiyue_sync::{ConflictKind, ConflictRecord};
use serde::{Deserialize, Serialize};

/// How the conflict looks on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Text merged with markers in the file; the remote version is in the copy.
    Markers,
    /// Binary on both sides; the local version stays at the path, the remote one is in the copy.
    BothKept,
}

/// One unresolved conflict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// The file that conflicted (vault-relative, forward slashes).
    pub path: String,
    /// The conflict copy that holds the other version.
    pub copy_path: String,
    pub kind: Kind,
    /// When the conflict was first recorded, Unix milliseconds.
    pub detected_ms: i64,
}

impl From<ConflictKind> for Kind {
    fn from(k: ConflictKind) -> Self {
        match k {
            ConflictKind::Markers => Kind::Markers,
            ConflictKind::BothKept => Kind::BothKept,
        }
    }
}

/// Which side of a conflict to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Local,
    Remote,
}

/// `<root>/.jiyue/sync/conflicts.json`.
pub fn list_path(root: &Path) -> PathBuf {
    sync_dir(root).join("conflicts.json")
}

/// Reads the persisted list. A missing or unreadable file is an empty list (conflicts are a
/// convenience view; the copies on disk stay where they are).
pub fn load(root: &Path) -> Vec<Item> {
    fs::read(list_path(root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes the list atomically.
pub fn save(root: &Path, items: &[Item]) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(items).map_err(|e| io::Error::other(e.to_string()))?;
    if let Some(dir) = list_path(root).parent() {
        fs::create_dir_all(dir)?;
    }
    write_atomic(&list_path(root), &bytes)
}

/// Adds the conflicts of one run. Entries are keyed by copy path, so a conflict seen twice is kept
/// once. Returns true when the list changed.
pub fn add_records(items: &mut Vec<Item>, records: &[ConflictRecord]) -> bool {
    let mut changed = false;
    for rec in records {
        if items.iter().any(|i| i.copy_path == rec.copy_path) {
            continue;
        }
        items.push(Item {
            path: rec.path.clone(),
            copy_path: rec.copy_path.clone(),
            kind: rec.kind.into(),
            detected_ms: now_ms(),
        });
        changed = true;
    }
    changed
}

/// Drops entries whose conflict copy no longer exists (resolved outside the app, or synced away).
/// Returns true when something was dropped.
pub fn prune(items: &mut Vec<Item>, copy_exists: impl Fn(&str) -> bool) -> bool {
    let before = items.len();
    items.retain(|i| copy_exists(&i.copy_path));
    items.len() != before
}

/// Resolves conflict markers, keeping one side of every hunk and the text around them.
/// Lines that are not inside markers are kept unchanged.
pub fn resolve_markers(text: &str, keep: Side) -> String {
    #[derive(PartialEq)]
    enum State {
        Normal,
        Local,
        Remote,
    }
    let mut out = String::with_capacity(text.len());
    let mut state = State::Normal;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']);
        match state {
            State::Normal if bare == MARKER_LOCAL => {
                state = State::Local;
                continue;
            }
            State::Local if bare == MARKER_SPLIT => {
                state = State::Remote;
                continue;
            }
            State::Remote if bare == MARKER_REMOTE => {
                state = State::Normal;
                continue;
            }
            _ => {}
        }
        let keep_line = match state {
            State::Normal => true,
            State::Local => keep == Side::Local,
            State::Remote => keep == Side::Remote,
        };
        if keep_line {
            out.push_str(line);
        }
    }
    out
}

/// Moves `rel` into `.trash/sync/` (kept, not deleted). Returns the new location.
fn move_to_trash(root: &Path, rel: &str) -> io::Result<PathBuf> {
    let source = root.join(rel);
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let dir = root.join(".trash").join("sync");
    fs::create_dir_all(&dir)?;
    let mut target = dir.join(format!("{}-{name}", now_ms()));
    let mut n = 1u32;
    while target.exists() {
        n += 1;
        target = dir.join(format!("{}-{n}-{name}", now_ms()));
    }
    fs::rename(&source, &target)?;
    Ok(target)
}

/// Keeps the local version: markers are resolved to the local side, the copy moves to the trash.
pub fn keep_local(root: &Path, item: &Item) -> Result<(), String> {
    let path = root.join(&item.path);
    if item.kind == Kind::Markers {
        let text = fs::read_to_string(&path).map_err(|e| format!("无法读取笔记：{e}"))?;
        let resolved = resolve_markers(&text, Side::Local);
        write_atomic(&path, resolved.as_bytes()).map_err(|e| format!("无法写入笔记：{e}"))?;
    }
    move_to_trash(root, &item.copy_path)
        .map(|_| ())
        .map_err(|e| format!("无法移动冲突副本：{e}"))
}

/// Keeps the remote version: the copy's content replaces the file, then the copy moves to the trash.
pub fn keep_remote(root: &Path, item: &Item) -> Result<(), String> {
    let copy =
        fs::read(root.join(&item.copy_path)).map_err(|e| format!("无法读取冲突副本：{e}"))?;
    write_atomic(&root.join(&item.path), &copy).map_err(|e| format!("无法写入笔记：{e}"))?;
    move_to_trash(root, &item.copy_path)
        .map(|_| ())
        .map_err(|e| format!("无法移动冲突副本：{e}"))
}

/// Marks the conflict resolved by hand: only the copy moves to the trash.
pub fn mark_resolved(root: &Path, item: &Item) -> Result<(), String> {
    move_to_trash(root, &item.copy_path)
        .map(|_| ())
        .map_err(|e| format!("无法移动冲突副本：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jiyue-conflicts-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn item(path: &str, copy: &str, kind: Kind) -> Item {
        Item {
            path: path.into(),
            copy_path: copy.into(),
            kind,
            detected_ms: 1,
        }
    }

    #[test]
    fn markers_keep_either_side() {
        let text = format!(
            "intro\n{MARKER_LOCAL}\nlocal A\n{MARKER_SPLIT}\nremote A\n{MARKER_REMOTE}\nmiddle\n{MARKER_LOCAL}\nlocal B\n{MARKER_SPLIT}\n{MARKER_REMOTE}\nend\n"
        );
        assert_eq!(
            resolve_markers(&text, Side::Local),
            "intro\nlocal A\nmiddle\nlocal B\nend\n"
        );
        assert_eq!(
            resolve_markers(&text, Side::Remote),
            "intro\nremote A\nmiddle\nend\n"
        );
        assert_eq!(resolve_markers("plain\n", Side::Local), "plain\n");
    }

    #[test]
    fn records_are_added_once_and_pruned_when_copy_is_gone() {
        let mut items = Vec::new();
        let rec = ConflictRecord {
            path: "a.md".into(),
            copy_path: "a (冲突副本 PC 1).md".into(),
            kind: ConflictKind::Markers,
        };
        assert!(add_records(&mut items, std::slice::from_ref(&rec)));
        assert!(!add_records(&mut items, std::slice::from_ref(&rec)));
        assert_eq!(items.len(), 1);
        assert!(!prune(&mut items, |_| true));
        assert!(prune(&mut items, |_| false));
        assert!(items.is_empty());
    }

    #[test]
    fn list_round_trips_on_disk() {
        let root = temp_root("list");
        let items = vec![item("a.md", "a (冲突副本 PC 1).md", Kind::Markers)];
        save(&root, &items).expect("save");
        assert_eq!(load(&root), items);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn keep_remote_replaces_file_and_keeps_copy_in_trash() {
        let root = temp_root("remote");
        fs::write(
            root.join("a.md"),
            "<<<<<<< 本机\nx\n=======\ny\n>>>>>>> 远端\n",
        )
        .unwrap();
        fs::write(root.join("a (冲突副本 PC 1).md"), "remote text\n").unwrap();
        let it = item("a.md", "a (冲突副本 PC 1).md", Kind::Markers);
        keep_remote(&root, &it).expect("keep remote");
        assert_eq!(
            fs::read_to_string(root.join("a.md")).unwrap(),
            "remote text\n"
        );
        assert!(!root.join("a (冲突副本 PC 1).md").exists());
        let trashed: Vec<_> = fs::read_dir(root.join(".trash/sync")).unwrap().collect();
        assert_eq!(trashed.len(), 1);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn keep_local_resolves_markers_to_local_side() {
        let root = temp_root("local");
        let text = format!("top\n{MARKER_LOCAL}\nmine\n{MARKER_SPLIT}\ntheirs\n{MARKER_REMOTE}\n");
        fs::write(root.join("b.md"), text).unwrap();
        fs::write(root.join("b (冲突副本 PC 1).md"), "theirs\n").unwrap();
        let it = item("b.md", "b (冲突副本 PC 1).md", Kind::Markers);
        keep_local(&root, &it).expect("keep local");
        assert_eq!(
            fs::read_to_string(root.join("b.md")).unwrap(),
            "top\nmine\n"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mark_resolved_only_moves_the_copy() {
        let root = temp_root("resolved");
        fs::write(root.join("c.png"), [1u8, 2, 3]).unwrap();
        fs::write(root.join("c (冲突副本 PC 1).png"), [9u8]).unwrap();
        let it = item("c.png", "c (冲突副本 PC 1).png", Kind::BothKept);
        mark_resolved(&root, &it).expect("resolve");
        assert_eq!(fs::read(root.join("c.png")).unwrap(), vec![1u8, 2, 3]);
        assert!(!root.join("c (冲突副本 PC 1).png").exists());
        fs::remove_dir_all(&root).ok();
    }
}
