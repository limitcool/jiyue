//! Pure logic of the file operations: name rules and messages, path mapping, menu rows, drop
//! targets, trash times and the line diff. No GPUI state here, so everything is unit tested.

use jiyue_core::daily::Date;
use jiyue_core::fsops::{self, FsError, TrashEntry};

use crate::ui::widgets::overlay::MenuItem;

// ---------------------------------------------------------------------------
// Names and messages
// ---------------------------------------------------------------------------

/// Chinese text for a file-operation error, shown in toasts.
pub fn fs_error_zh(err: &FsError) -> String {
    match err {
        FsError::NotFound => "找不到这个项目，可能已被移动或删除".to_string(),
        FsError::Exists => "同名项目已存在".to_string(),
        FsError::Invalid(reason) => format!("名称不可用：{}", invalid_reason_zh(reason)),
        FsError::Io(message) => format!("文件操作失败：{message}"),
    }
}

/// Chinese text for one reason from `fsops::safe_rel_path` and friends.
pub fn invalid_reason_zh(reason: &str) -> &'static str {
    match reason {
        "empty path" | "empty path segment" | "empty title" => "名称不能为空",
        "backslash separator" | "illegal character in name" => "名称不能包含 \\ / : * ? \" < > |",
        "absolute path" => "名称不能以 / 开头",
        "relative segment" => "名称不能是 . 或 ..",
        "name longer than 200 bytes" => "名称太长（最多 200 字节）",
        "name ends with a dot or a space" => "名称不能以点或空格结尾",
        "control character in name" => "名称不能包含控制字符",
        "reserved Windows device name" => "Windows 保留的设备名（如 CON、NUL）不能使用",
        "title starts with a dot" => "名称不能以点开头",
        "cannot move a folder into itself" => "不能移动到它自己的子文件夹中",
        "destination is not a folder" => "目标不是文件夹",
        "item is already in the trash" => "项目已经在回收站中",
        _ => "名称无效",
    }
}

/// Checks a name typed for one item (a note title without `.md`, or a folder name). Returns the
/// trimmed name or the Chinese reason it is not allowed.
pub fn validate_item_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name.starts_with('.') {
        return Err(invalid_reason_zh("title starts with a dot").to_string());
    }
    if name.contains(['/', '\\']) {
        return Err(invalid_reason_zh("illegal character in name").to_string());
    }
    fsops::safe_rel_path(name)
        .map(|_| name.to_string())
        .map_err(|err| match err {
            FsError::Invalid(reason) => invalid_reason_zh(&reason).to_string(),
            other => fs_error_zh(&other),
        })
}

/// The title a note name is built from: trimmed, and a trailing `.md` (any case) removed.
pub fn note_title_from_input(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.len() > 3 && trimmed.to_ascii_lowercase().ends_with(".md") {
        trimmed[..trimmed.len() - 3].trim_end().to_string()
    } else {
        trimmed.to_string()
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// The folder part of a vault-relative path (`""` for the root).
pub fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

/// The last component of a vault-relative path.
pub fn name_of(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// `dir/name`, or `name` for the root.
pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// True when `path` is `base` or lies below it. An empty `base` (the root) contains everything.
pub fn is_under(path: &str, base: &str) -> bool {
    base.is_empty() || path == base || path.starts_with(&format!("{base}/"))
}

/// Maps `path` through a move of `from` to `to` (an item, or a folder and everything below it).
/// `None` when the path is not affected.
pub fn map_under(path: &str, from: &str, to: &str) -> Option<String> {
    if from.is_empty() {
        return None;
    }
    if path == from {
        return Some(to.to_string());
    }
    path.strip_prefix(&format!("{from}/"))
        .map(|rest| join(to, rest))
}

/// Pairs `(old, new)` for every note path that a move of `from` to `to` changes.
pub fn note_renames(note_paths: &[String], from: &str, to: &str) -> Vec<(String, String)> {
    note_paths
        .iter()
        .filter_map(|p| map_under(p, from, to).map(|new| (p.clone(), new)))
        .collect()
}

/// Folder keys (expanded folders) after a move of `from` to `to`.
pub fn remap_keys(keys: &[String], from: &str, to: &str) -> Vec<String> {
    keys.iter()
        .map(|k| map_under(k, from, to).unwrap_or_else(|| k.clone()))
        .collect()
}

/// Every folder key of a path, shallowest first: `a/b/c.md` gives `a` and `a/b`.
pub fn ancestor_keys(path: &str) -> Vec<String> {
    let dir = parent_of(path);
    if dir.is_empty() {
        return Vec::new();
    }
    let parts: Vec<&str> = dir.split('/').collect();
    (1..=parts.len()).map(|n| parts[..n].join("/")).collect()
}

/// What a drag of `src` onto the folder `dest_dir` means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveCheck {
    /// The item is already in that folder: nothing to do.
    NoOp,
    /// Move to this new vault-relative path.
    Move(String),
    /// Not allowed; the text says why.
    Invalid(String),
}

/// Resolves a move of `src` (a note or folder, `is_dir`) into the folder `dest_dir` (`""` for
/// the root). A folder cannot go into itself or into one of its own subfolders.
pub fn move_target(src: &str, is_dir: bool, dest_dir: &str) -> MoveCheck {
    if is_dir && is_under(dest_dir, src) {
        return MoveCheck::Invalid(invalid_reason_zh("cannot move a folder into itself").into());
    }
    if parent_of(src) == dest_dir {
        return MoveCheck::NoOp;
    }
    MoveCheck::Move(join(dest_dir, name_of(src)))
}

// ---------------------------------------------------------------------------
// Context menus
// ---------------------------------------------------------------------------

/// What a context menu was opened on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A note (`is_dir` false) or a folder row of the file tree.
    Tree { path: String, is_dir: bool },
    /// Empty space of the file tree (new items go to the root).
    Empty,
    /// A tab, by its position in the tab strip.
    Tab(usize),
}

/// Command behind one menu row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    NewNote(String),
    NewFolder(String),
    Rename {
        path: String,
        is_dir: bool,
    },
    Move {
        path: String,
        is_dir: bool,
    },
    CopyPath(String),
    Reveal(String),
    /// 加入/取消书签 for the note or folder (`ui/bookmarks`).
    Bookmark(String),
    Delete {
        path: String,
        is_dir: bool,
    },
    CloseTab(usize),
    CloseOthers(usize),
    CloseRight(usize),
    CopyTabPath(usize),
    RevealTab(usize),
    /// The rows of a multi-selection (`menu.rs` reads the selection when the row runs).
    BatchMove,
    BatchBookmark,
    BatchCopyPath,
    BatchReveal,
    BatchDelete,
}

/// Rows of the context menu for `target`, each with its command (`None` for separators).
/// Rows of the menu of a multi-selection.
pub fn batch_menu_rows() -> Vec<(MenuItem, Option<Act>)> {
    let mut rows = vec![
        row("移动到…", "folder", None, Act::BatchMove),
        row("加入/取消书签", "bookmark", None, Act::BatchBookmark),
        row("复制路径", "copy", None, Act::BatchCopyPath),
        row("在资源管理器中显示", "folder-open", None, Act::BatchReveal),
        sep(),
    ];
    let (delete, act) = row("删除到回收站", "trash", Some("Delete"), Act::BatchDelete);
    rows.push((delete.danger(), act));
    rows
}

pub fn menu_rows(target: &Target) -> Vec<(MenuItem, Option<Act>)> {
    match target {
        Target::Tree { path, is_dir } => {
            let dir = if *is_dir {
                path.clone()
            } else {
                parent_of(path).to_string()
            };
            let mut rows = vec![
                row("新建笔记", "file-plus", None, Act::NewNote(dir.clone())),
                row("新建文件夹", "folder-plus", None, Act::NewFolder(dir)),
                sep(),
                row(
                    &crate::t!("menu-rename"),
                    "pencil",
                    Some("F2"),
                    Act::Rename {
                        path: path.clone(),
                        is_dir: *is_dir,
                    },
                ),
                row(
                    &crate::t!("menu-move-to"),
                    "folder",
                    None,
                    Act::Move {
                        path: path.clone(),
                        is_dir: *is_dir,
                    },
                ),
                row(
                    &crate::t!("menu-copy-path"),
                    "copy",
                    None,
                    Act::CopyPath(path.clone()),
                ),
                row(
                    &crate::t!("menu-bookmark-toggle"),
                    "bookmark",
                    None,
                    Act::Bookmark(path.clone()),
                ),
                row(
                    &crate::t!("menu-reveal-file"),
                    "folder-open",
                    None,
                    Act::Reveal(path.clone()),
                ),
                sep(),
            ];
            let (delete, act) = row(
                &crate::t!("menu-delete"),
                "trash",
                Some("Delete"),
                Act::Delete {
                    path: path.clone(),
                    is_dir: *is_dir,
                },
            );
            rows.push((delete.danger(), act));
            rows
        }
        Target::Empty => vec![
            row(
                &crate::t!("menu-new-note"),
                "file-plus",
                None,
                Act::NewNote(String::new()),
            ),
            row(
                &crate::t!("menu-new-folder"),
                "folder-plus",
                None,
                Act::NewFolder(String::new()),
            ),
        ],
        Target::Tab(pos) => vec![
            row(&crate::t!("menu-close-tab"), "x", None, Act::CloseTab(*pos)),
            row(
                &crate::t!("menu-close-other-tabs"),
                "x",
                None,
                Act::CloseOthers(*pos),
            ),
            row(
                &crate::t!("menu-close-right-tabs"),
                "x",
                None,
                Act::CloseRight(*pos),
            ),
            sep(),
            row(
                &crate::t!("menu-copy-path"),
                "copy",
                None,
                Act::CopyTabPath(*pos),
            ),
            row(
                &crate::t!("menu-reveal-file"),
                "folder-open",
                None,
                Act::RevealTab(*pos),
            ),
        ],
    }
}

fn row(
    label: &str,
    icon: &'static str,
    shortcut: Option<&str>,
    act: Act,
) -> (MenuItem, Option<Act>) {
    let mut item = MenuItem::new(label).icon(icon);
    if let Some(text) = shortcut {
        item = item.shortcut(text);
    }
    (item, Some(act))
}

fn sep() -> (MenuItem, Option<Act>) {
    (MenuItem::separator(), None)
}

// ---------------------------------------------------------------------------
// Trash and times
// ---------------------------------------------------------------------------

/// Formats Unix milliseconds as local `YYYY-MM-DD HH:MM`, given the local offset in seconds.
pub fn format_local(ms: i64, offset_secs: i64) -> String {
    let secs = ms.div_euclid(1000) + offset_secs;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let date = Date::from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        date.year,
        date.month,
        date.day,
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// The two texts of a trash row: the original path, and when it was deleted.
pub fn trash_row_texts(entry: &TrashEntry, offset_secs: i64) -> (String, String) {
    (
        entry.original.clone(),
        format!("删除于 {}", format_local(entry.deleted_ms, offset_secs)),
    )
}

// ---------------------------------------------------------------------------
// Line diff (for 查看差异)
// ---------------------------------------------------------------------------

/// One aligned row of the two-column diff. `left` is the file on disk, `right` the editor text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    pub kind: DiffKind,
    pub left: String,
    pub right: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Same,
    /// Only on disk.
    Removed,
    /// Only in the editor.
    Added,
}

/// Largest `lines(left) * lines(right)` the diff table is built for.
pub const DIFF_CELL_LIMIT: usize = 2_000_000;

/// Aligns two texts line by line (longest common subsequence). Returns `None` when the texts are
/// too large for the table.
pub fn line_diff(left: &str, right: &str) -> Option<Vec<DiffRow>> {
    let a: Vec<&str> = left.lines().collect();
    let b: Vec<&str> = right.lines().collect();
    if a.len().saturating_mul(b.len()) > DIFF_CELL_LIMIT {
        return None;
    }
    let (n, m) = (a.len(), b.len());
    // lcs[i][j] = common length of a[i..] and b[j..].
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut rows = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            rows.push(DiffRow {
                kind: DiffKind::Same,
                left: a[i].to_string(),
                right: b[j].to_string(),
            });
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            rows.push(DiffRow {
                kind: DiffKind::Removed,
                left: a[i].to_string(),
                right: String::new(),
            });
            i += 1;
        } else {
            rows.push(DiffRow {
                kind: DiffKind::Added,
                left: String::new(),
                right: b[j].to_string(),
            });
            j += 1;
        }
    }
    for line in &a[i..] {
        rows.push(DiffRow {
            kind: DiffKind::Removed,
            left: line.to_string(),
            right: String::new(),
        });
    }
    for line in &b[j..] {
        rows.push(DiffRow {
            kind: DiffKind::Added,
            left: String::new(),
            right: line.to_string(),
        });
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(rows: &[(MenuItem, Option<Act>)]) -> Vec<String> {
        rows.iter()
            .map(|(item, _)| {
                if item.separator {
                    "-".to_string()
                } else {
                    item.label.clone()
                }
            })
            .collect()
    }

    #[test]
    fn invalid_names_get_chinese_hints() {
        assert_eq!(validate_item_name("  ").unwrap_err(), "名称不能为空");
        assert_eq!(
            validate_item_name("a/b").unwrap_err(),
            "名称不能包含 \\ / : * ? \" < > |"
        );
        assert_eq!(
            validate_item_name("a:b").unwrap_err(),
            "名称不能包含 \\ / : * ? \" < > |"
        );
        assert_eq!(
            validate_item_name("CON").unwrap_err(),
            "Windows 保留的设备名（如 CON、NUL）不能使用"
        );
        assert_eq!(
            validate_item_name(".hidden").unwrap_err(),
            "名称不能以点开头"
        );
        assert_eq!(
            validate_item_name("trailing.").unwrap_err(),
            "名称不能以点或空格结尾"
        );
        assert_eq!(validate_item_name("  全新笔记 ").unwrap(), "全新笔记");
    }

    #[test]
    fn md_suffix_is_stripped_from_note_titles() {
        assert_eq!(note_title_from_input("Plan.MD"), "Plan");
        assert_eq!(note_title_from_input(" 草稿 "), "草稿");
        assert_eq!(note_title_from_input(".md"), ".md");
    }

    #[test]
    fn fs_errors_have_chinese_text() {
        assert_eq!(fs_error_zh(&FsError::Exists), "同名项目已存在");
        assert!(fs_error_zh(&FsError::Invalid("empty path".into())).starts_with("名称不可用"));
    }

    #[test]
    fn folder_moves_map_every_path_below_them() {
        assert_eq!(
            map_under("想法/a.md", "想法", "归档/想法"),
            Some("归档/想法/a.md".to_string())
        );
        assert_eq!(map_under("想法", "想法", "归档"), Some("归档".to_string()));
        assert_eq!(map_under("想法2/a.md", "想法", "x"), None);
        assert_eq!(map_under("a.md", "a.md", "b.md"), Some("b.md".to_string()));
        let notes = vec![
            "想法/a.md".to_string(),
            "b.md".to_string(),
            "想法/子/c.md".to_string(),
        ];
        assert_eq!(
            note_renames(&notes, "想法", "归档"),
            vec![
                ("想法/a.md".to_string(), "归档/a.md".to_string()),
                ("想法/子/c.md".to_string(), "归档/子/c.md".to_string()),
            ]
        );
        assert_eq!(
            remap_keys(
                &[
                    "想法".to_string(),
                    "想法/子".to_string(),
                    "其他".to_string()
                ],
                "想法",
                "归档"
            ),
            vec![
                "归档".to_string(),
                "归档/子".to_string(),
                "其他".to_string()
            ]
        );
    }

    #[test]
    fn drop_targets_refuse_self_and_no_op_moves() {
        assert_eq!(move_target("a/x.md", false, "a"), MoveCheck::NoOp);
        assert_eq!(
            move_target("a/x.md", false, ""),
            MoveCheck::Move("x.md".into())
        );
        assert_eq!(
            move_target("a/x.md", false, "b"),
            MoveCheck::Move("b/x.md".into())
        );
        assert_eq!(
            move_target("a", true, "a/sub"),
            MoveCheck::Invalid("不能移动到它自己的子文件夹中".into())
        );
        assert_eq!(
            move_target("a", true, "a"),
            MoveCheck::Invalid("不能移动到它自己的子文件夹中".into())
        );
        assert_eq!(move_target("a", true, "b"), MoveCheck::Move("b/a".into()));
        // A file whose name starts like a folder is not inside it.
        assert_eq!(
            move_target("ab/x.md", false, "a"),
            MoveCheck::Move("a/x.md".into())
        );
    }

    #[test]
    fn tree_menu_rows_depend_on_the_target() {
        let file = menu_rows(&Target::Tree {
            path: "想法/a.md".into(),
            is_dir: false,
        });
        assert_eq!(
            labels(&file),
            vec![
                "新建笔记",
                "新建文件夹",
                "-",
                "重命名",
                "移动到…",
                "复制路径",
                "加入/取消书签",
                "在资源管理器中显示",
                "-",
                "删除"
            ]
        );
        // New items of a note go to its folder.
        assert_eq!(file[0].1, Some(Act::NewNote("想法".into())));
        let folder = menu_rows(&Target::Tree {
            path: "想法".into(),
            is_dir: true,
        });
        assert_eq!(folder[0].1, Some(Act::NewNote("想法".into())));
        assert!(folder.last().unwrap().0.danger);
        assert_eq!(
            labels(&menu_rows(&Target::Empty)),
            vec!["新建笔记", "新建文件夹"]
        );
        assert_eq!(menu_rows(&Target::Tab(2)).len(), 6);
    }

    #[test]
    fn trash_times_are_local_and_readable() {
        assert_eq!(format_local(0, 8 * 3600), "1970-01-01 08:00");
        // 2026-11-04 12:30 UTC, shown at UTC+8.
        let utc_ms = 1_793_795_400_000_i64;
        assert_eq!(format_local(utc_ms, 0), "2026-11-04 12:30");
        assert_eq!(format_local(utc_ms, 8 * 3600), "2026-11-04 20:30");
        let entry = TrashEntry {
            name: "1-a.md".into(),
            original: "想法/a.md".into(),
            deleted_ms: utc_ms,
            is_dir: false,
        };
        assert_eq!(
            trash_row_texts(&entry, 0),
            (
                "想法/a.md".to_string(),
                "删除于 2026-11-04 12:30".to_string()
            )
        );
    }

    #[test]
    fn line_diff_aligns_changes() {
        let rows = line_diff("a\nb\nc", "a\nB\nc\nd").unwrap();
        let kinds: Vec<DiffKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffKind::Same,
                DiffKind::Removed,
                DiffKind::Added,
                DiffKind::Same,
                DiffKind::Added
            ]
        );
        assert_eq!(rows[1].left, "b");
        assert_eq!(rows[2].right, "B");
        assert_eq!(rows[4].right, "d");
        assert_eq!(line_diff("same", "same").unwrap().len(), 1);
    }

    #[test]
    fn ancestors_of_a_path() {
        assert_eq!(
            ancestor_keys("a/b/c.md"),
            vec!["a".to_string(), "a/b".to_string()]
        );
        assert!(ancestor_keys("root.md").is_empty());
    }
}
