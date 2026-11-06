use crate::buffer::Note;
use crate::fsops;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod title_tests {
    use super::*;

    #[test]
    fn plain_and_cjk_titles_are_accepted() {
        assert_eq!(safe_note_title("  全新笔记 "), Some("全新笔记".to_string()));
        assert_eq!(
            safe_note_title("v1.2 notes"),
            Some("v1.2 notes".to_string())
        );
        assert_eq!(safe_note_title("com10"), Some("com10".to_string()));
    }

    #[test]
    fn unsafe_titles_are_rejected() {
        for bad in [
            "",
            "   ",
            "a/b",
            "a\\b",
            "..",
            "../x",
            "x..y",
            ".hidden",
            "trailing.",
            "a:b",
            "a*b",
            "a?b",
            "a|b",
            "a#b",
            "a[b]",
            "tab\tname",
            "CON",
            "nul",
            "Com1",
            "LPT9.txt",
        ] {
            assert_eq!(safe_note_title(bad), None, "{:?} should be rejected", bad);
        }
        let long = "a".repeat(121);
        assert_eq!(safe_note_title(&long), None);
    }

    #[test]
    fn new_blank_note_has_an_empty_body() {
        let dir = std::env::temp_dir().join(format!("jiyue-blank-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut vault = Vault::open_or_create(&dir);
        vault.create_new_note("未命名 1");
        let idx = vault
            .notes
            .iter()
            .position(|n| n.path == "未命名 1.md")
            .expect("new note");
        assert_eq!(vault.notes[idx].title, "未命名 1");
        assert_eq!(vault.notes[idx].content(), "");
        assert_eq!(
            fs::read_to_string(dir.join("未命名 1.md")).unwrap_or_default(),
            ""
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_link_note_writes_file_and_reuses_existing_title() {
        let dir = std::env::temp_dir().join(format!("jiyue-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut vault = Vault::open_or_create(&dir);
        let before = vault.notes.len();

        let idx = vault.create_link_note("全新笔记").expect("create");
        assert_eq!(vault.notes.len(), before + 1);
        assert_eq!(vault.notes[idx].title, "全新笔记");
        assert!(dir.join("全新笔记.md").exists());
        let on_disk = fs::read_to_string(dir.join("全新笔记.md")).unwrap_or_default();
        assert_eq!(on_disk, "# 全新笔记\n\n");

        // Same title (any case) opens the existing note instead of creating another one.
        assert_eq!(vault.create_link_note("全新笔记"), Ok(idx));
        assert!(vault.create_link_note("../escape").is_err());
        assert_eq!(vault.notes.len(), before + 1);

        let _ = fs::remove_dir_all(&dir);
    }
}

/// A note title that is safe to use as the file name `<title>.md` in the vault root, or `None`.
/// The title is trimmed. Empty or over-long titles, path separators, `..`, control characters,
/// characters Windows forbids in file names, `#`, `|`, `[`, `]` and the reserved Windows device
/// names (CON, PRN, AUX, NUL, COM1-9, LPT1-9) are rejected.
pub fn safe_note_title(raw: &str) -> Option<String> {
    let title = raw.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return None;
    }
    if title
        .chars()
        .any(|c| c.is_control() || "/\\:*?\"<>|#[]".contains(c))
    {
        return None;
    }
    if title.contains("..") || title.starts_with('.') || title.ends_with('.') {
        return None;
    }
    let base = title
        .split('.')
        .next()
        .unwrap_or(title)
        .trim_end()
        .to_ascii_uppercase();
    if is_reserved_device_name(&base) {
        return None;
    }
    Some(title.to_string())
}

#[cfg(test)]
mod reload_tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn tabs_follow_their_notes_by_path() {
        // Note `b` moved to index 0 and note `c` was dropped.
        let paths = s(&["b.md", "a.md"]);
        let (tabs, tab, note) =
            remap_tab_state(&s(&["a.md", "c.md", "b.md"]), Some("b.md"), 2, &paths);
        assert_eq!(tabs, vec![1, 0]);
        assert_eq!(tab, 1);
        assert_eq!(note, 0);
    }

    #[test]
    fn active_note_removed_falls_back_to_same_tab_position() {
        let paths = s(&["a.md", "c.md"]);
        let (tabs, tab, note) =
            remap_tab_state(&s(&["a.md", "b.md", "c.md"]), Some("b.md"), 1, &paths);
        assert_eq!(tabs, vec![0, 1]);
        assert_eq!(tab, 1);
        assert_eq!(note, 1);
    }

    #[test]
    fn all_tabs_gone_opens_first_note_and_empty_vault_has_none() {
        let paths = s(&["x.md"]);
        assert_eq!(
            remap_tab_state(&s(&["gone.md"]), Some("gone.md"), 0, &paths),
            (vec![0], 0, 0)
        );
        assert_eq!(
            remap_tab_state(&s(&["gone.md"]), None, 3, &[]),
            (vec![], 0, 0)
        );
    }

    #[test]
    fn reload_keeps_tabs_and_content_and_skips_trash() {
        let dir = std::env::temp_dir().join(format!("jiyue-reload-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).expect("mkdir");
        fs::create_dir_all(dir.join(".trash")).expect("mkdir");
        fs::write(dir.join("a.md"), "# A\n").expect("write");
        fs::write(dir.join("sub").join("b.md"), "# B\n").expect("write");
        fs::write(dir.join(".trash").join("hidden.md"), "# H\n").expect("write");
        fs::create_dir_all(dir.join("empty")).expect("mkdir");

        let mut vault = Vault::open_or_create(&dir);
        assert_eq!(vault.notes.len(), 2);
        assert!(vault.notes.iter().any(|n| n.path == "sub/b.md"));
        assert_eq!(vault.folders, vec!["empty".to_string(), "sub".to_string()]);

        let a = vault.note_index_by_title("A").expect("a");
        vault.open_tabs = vec![a];
        vault.active_tab_index = 0;
        vault.active_note_index = a;
        vault.notes[a].set_content("# A unsaved\n");

        // Rename b.md into a.md's folder-less name, and keep a's unsaved text.
        fs::rename(dir.join("sub").join("b.md"), dir.join("c.md")).expect("rename");
        vault.reload_from_disk(
            &["a.md".to_string()],
            &[("sub/b.md".to_string(), "c.md".to_string())],
        );
        assert_eq!(vault.notes.len(), 2);
        assert_eq!(vault.active_note().map(|n| n.path.as_str()), Some("a.md"));
        assert_eq!(
            vault.active_note().map(|n| n.content()),
            Some("# A unsaved\n")
        );
        assert_eq!(vault.open_tabs.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }
}

/// Works out the tab state after the note list changed. `open_paths` are the paths of the open
/// tabs in tab order, `active_path` the active note (if any), `active_tab` the active tab
/// position before the change, and `paths` the note list after it.
///
/// Returns `(open_tabs, active_tab_index, active_note_index)` as indices into `paths`. Tabs whose
/// note is gone are dropped. When the active note is gone, the tab at the old active position (or
/// the last one) becomes active. When every tab is gone but notes remain, the first note is opened,
/// like a fresh vault. An empty list gives no tabs and index 0.
pub fn remap_tab_state(
    open_paths: &[String],
    active_path: Option<&str>,
    active_tab: usize,
    paths: &[String],
) -> (Vec<usize>, usize, usize) {
    let mut tabs: Vec<usize> = Vec::new();
    for path in open_paths {
        if let Some(idx) = paths.iter().position(|p| p == path)
            && !tabs.contains(&idx)
        {
            tabs.push(idx);
        }
    }
    if tabs.is_empty() {
        if paths.is_empty() {
            return (Vec::new(), 0, 0);
        }
        tabs.push(0);
    }
    let active_idx = active_path.and_then(|a| paths.iter().position(|p| p == a));
    let pos = match active_idx.and_then(|idx| tabs.iter().position(|&t| t == idx)) {
        Some(pos) => pos,
        None => active_tab.min(tabs.len() - 1),
    };
    let note = tabs[pos];
    (tabs, pos, note)
}

/// True for CON, PRN, AUX, NUL and COM1-COM9, LPT1-LPT9 (upper case, without extension).
fn is_reserved_device_name(base: &str) -> bool {
    if matches!(base, "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = base.as_bytes();
    bytes.len() == 4
        && (base.starts_with("COM") || base.starts_with("LPT"))
        && (b'1'..=b'9').contains(&bytes[3])
}

/// 2D Point on graph canvas
#[derive(Debug, Clone, Copy)]
pub struct GraphPoint {
    pub x: f32,
    pub y: f32,
}

/// What the graph needs of one note. The text is copied so the graph can be built off the UI
/// thread (see `build_graph`).
#[derive(Clone, Debug)]
pub struct GraphSource {
    pub id: String,
    pub title: String,
    pub content: String,
}

impl GraphSource {
    /// The graph input of a note.
    pub fn of(note: &Note) -> Self {
        Self {
            id: note.id.clone(),
            title: note.title.clone(),
            content: note.content().to_string(),
        }
    }
}

/// Builds the graph from the notes: an edge for each wikilink to another note's title (case
/// insensitive; the first note with that title is the target), and a circular initial layout.
/// Pure (no vault access), so it can run on a background thread.
pub fn build_graph(sources: &[GraphSource]) -> KnowledgeGraph {
    let count = sources.len();
    if count == 0 {
        return KnowledgeGraph::default();
    }
    // Title (lower case) to the first note with it: the same match as a case-insensitive scan.
    let mut by_title: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, s) in sources.iter().enumerate() {
        by_title.entry(s.title.to_ascii_lowercase()).or_insert(i);
    }
    let mut edges = Vec::new();
    let mut link_counts = vec![0usize; count];
    for (from_idx, s) in sources.iter().enumerate() {
        for target_title in crate::buffer::wikilink_targets(&s.content) {
            if let Some(&to_idx) = by_title.get(&target_title.to_ascii_lowercase())
                && from_idx != to_idx
            {
                edges.push(GraphEdge {
                    from_index: from_idx,
                    to_index: to_idx,
                });
                link_counts[from_idx] += 1;
                link_counts[to_idx] += 1;
            }
        }
    }

    // Circular force-directed initial distribution
    let center_x = 360.0f32;
    let center_y = 280.0f32;
    let radius_base = 180.0f32;

    let mut nodes = Vec::new();
    for (i, s) in sources.iter().enumerate() {
        let angle = (i as f32 / count as f32) * std::f32::consts::TAU;
        let dist = radius_base + ((i * 37) % 50) as f32 - 25.0;
        let x = center_x + angle.cos() * dist;
        let y = center_y + angle.sin() * dist;
        let links = link_counts[i];
        let radius = (8.0 + (links as f32 * 3.0)).min(24.0);

        nodes.push(GraphNode {
            id: s.id.clone(),
            title: s.title.clone(),
            note_index: i,
            x,
            y,
            radius,
            link_count: links,
        });
    }
    KnowledgeGraph { nodes, edges }
}

/// A node in the Obsidian knowledge graph
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub note_index: usize,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub link_count: usize,
}

/// An edge connecting two notes in the knowledge graph
#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub from_index: usize,
    pub to_index: usize,
}

/// Full knowledge graph model computed from Vault wikilinks
#[derive(Debug, Clone, Default)]
pub struct KnowledgeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Vault tree item: either a folder or a note
#[derive(Debug, Clone)]
pub enum VaultNode {
    Folder {
        name: String,
        children: Vec<VaultNode>,
        expanded: bool,
    },
    File {
        name: String,
        path: String,
        note_index: usize,
    },
}

/// The document vault managing all markdown files, disk persistence, and linkages
pub struct Vault {
    pub name: String,
    pub root_dir: PathBuf,
    pub notes: Vec<Note>,
    pub active_note_index: usize,
    pub open_tabs: Vec<usize>, // indices into notes
    pub active_tab_index: usize,
    pub graph: KnowledgeGraph,
    /// Folders under the root (vault-relative, forward slashes), empty ones included, sorted.
    /// Filled by every disk scan; internal folders (`.trash`, hidden ones) are left out.
    pub folders: Vec<String>,
}

impl Vault {
    /// Open a vault from local directory, creating sample files if empty
    pub fn open_or_create(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("我的笔记库")
            .to_string();

        let mut vault = Self {
            name,
            root_dir: dir.to_path_buf(),
            notes: Vec::new(),
            active_note_index: 0,
            open_tabs: Vec::new(),
            active_tab_index: 0,
            graph: KnowledgeGraph::default(),
            folders: Vec::new(),
        };

        // Scan directory for .md files
        vault.load_from_disk();

        // If no notes exist on disk, create standard sample vault notes
        if vault.notes.is_empty() {
            vault.populate_initial_samples();
            vault.save_all();
        }

        if !vault.notes.is_empty() {
            vault.open_tabs = vec![0];
            if vault.notes.len() > 1 {
                vault.open_tabs.push(1);
            }
        }

        vault.rebuild_graph();
        vault
    }

    /// Recursively load .md files from root_dir
    pub fn load_from_disk(&mut self) {
        let mut loaded = Vec::new();
        let mut folders = Vec::new();
        Self::scan_dir_recursive(&self.root_dir, &self.root_dir, &mut loaded, &mut folders);
        self.notes = loaded;
        folders.sort();
        self.folders = folders;
    }

    /// Walks `current` and collects the `.md` notes and the folders below `root`. Paths are
    /// stored with forward slashes (the same form the watcher and `fsops` use), and internal or
    /// hidden entries (`.trash`, dot-files, `*.tmp`) are skipped.
    fn scan_dir_recursive(
        root: &Path,
        current: &Path,
        list: &mut Vec<Note>,
        folders: &mut Vec<String>,
    ) {
        if let Ok(entries) = fs::read_dir(current) {
            for entry in entries.flatten() {
                let path = entry.path();
                let rel_path = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if fsops::is_ignored(&rel_path) {
                    continue;
                }
                if path.is_dir() {
                    folders.push(rel_path);
                    Self::scan_dir_recursive(root, &path, list, folders);
                } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    let title = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Untitled")
                        .to_string();
                    if let Ok(content) = fs::read_to_string(&path) {
                        list.push(Note::new(&title, &rel_path, &content));
                    }
                }
            }
        }
    }

    /// Re-reads the note list and the folders from disk, after files changed or were renamed,
    /// moved or deleted outside this state (or by the file operations themselves).
    ///
    /// - `renames` maps the old note path to the new one for every note that moved. Open tabs and
    ///   the active note follow their note through it; a note that vanished without an entry in
    ///   `renames` drops out of the tabs.
    /// - `keep_text` lists (post-change) note paths whose in-memory text must win over the disk
    ///   copy. Their content is kept as it is; everything else is read from disk.
    ///
    /// Caller should `rebuild_graph` afterwards (done here).
    pub fn reload_from_disk(&mut self, keep_text: &[String], renames: &[(String, String)]) {
        let renamed = |path: &str| -> String {
            renames
                .iter()
                .find(|(old, _)| old == path)
                .map(|(_, new)| new.clone())
                .unwrap_or_else(|| path.to_string())
        };
        let open_paths: Vec<String> = self
            .open_tabs
            .iter()
            .filter_map(|&i| self.notes.get(i))
            .map(|n| renamed(&n.path))
            .collect();
        let active_path = self.active_note().map(|n| renamed(&n.path));
        let active_tab = self.active_tab_index;

        let mut loaded = Vec::new();
        let mut folders = Vec::new();
        Self::scan_dir_recursive(&self.root_dir, &self.root_dir, &mut loaded, &mut folders);
        for note in &mut loaded {
            if !keep_text.contains(&note.path) {
                continue;
            }
            if let Some(old) = self.notes.iter().find(|n| renamed(&n.path) == note.path) {
                let content = old.content().to_string();
                note.set_content(&content);
            }
        }
        let paths: Vec<String> = loaded.iter().map(|n| n.path.clone()).collect();
        self.notes = loaded;
        folders.sort();
        self.folders = folders;

        let (tabs, tab, note) =
            remap_tab_state(&open_paths, active_path.as_deref(), active_tab, &paths);
        self.open_tabs = tabs;
        self.active_tab_index = tab;
        self.active_note_index = note;
        self.rebuild_graph();
    }

    /// Write all notes back to disk
    pub fn save_all(&self) {
        for (i, _) in self.notes.iter().enumerate() {
            let _ = self.save_note_to_disk(i);
        }
    }

    /// Save a specific note to disk
    pub fn save_note_to_disk(&self, index: usize) -> std::io::Result<()> {
        if let Some(note) = self.notes.get(index) {
            let file_path = self.root_dir.join(&note.path);
            // Temp file, fsync, then replace: a failed or interrupted save leaves the old file intact.
            crate::fsops::write_atomic(&file_path, note.content().as_bytes())
                .map_err(|err| std::io::Error::other(err.to_string()))?;
        }
        Ok(())
    }

    /// Save active note
    pub fn save_active_note(&self) -> std::io::Result<()> {
        self.save_note_to_disk(self.active_note_index)
    }

    /// Create a new note on disk and open it. The body starts empty: the title is the file name,
    /// so a heading is not written here (the first `# Heading` names the file later, see
    /// `files_ui::autoname`).
    pub fn create_new_note(&mut self, title: &str) {
        let filename = format!("{}.md", title);
        let note = Note::new(title, &filename, "");
        self.notes.push(note);
        let new_idx = self.notes.len() - 1;
        let _ = self.save_note_to_disk(new_idx);

        self.open_tabs.push(new_idx);
        self.active_tab_index = self.open_tabs.len() - 1;
        self.active_note_index = new_idx;
        self.rebuild_graph();
    }

    /// Delete currently active note
    pub fn delete_active_note(&mut self) -> bool {
        if self.notes.len() <= 1 {
            return false;
        }
        let del_idx = self.active_note_index;
        let file_path = self.root_dir.join(&self.notes[del_idx].path);
        let _ = fs::remove_file(file_path);

        self.notes.remove(del_idx);

        // Adjust open tabs
        self.open_tabs.retain(|&idx| idx != del_idx);
        for tab in &mut self.open_tabs {
            if *tab > del_idx {
                *tab -= 1;
            }
        }
        if self.open_tabs.is_empty() {
            self.open_tabs.push(0);
        }
        self.active_tab_index = 0;
        self.active_note_index = self.open_tabs[0];
        self.rebuild_graph();
        true
    }

    /// Recompute Graph View layout positions and topological links
    pub fn rebuild_graph(&mut self) {
        self.graph = build_graph(&self.graph_sources());
    }

    /// The inputs of the graph: id, title and text of every note, in note order.
    pub fn graph_sources(&self) -> Vec<GraphSource> {
        self.notes.iter().map(GraphSource::of).collect()
    }

    /// Populate the Chinese sample notes shown on first run.
    fn populate_initial_samples(&mut self) {
        self.notes.push(Note::new(
            "欢迎使用 Jiyue",
            "欢迎使用 Jiyue.md",
            r#"# 欢迎使用 Jiyue

Jiyue 是一款基于 Rust 与 GPUI 构建的本地笔记应用，支持双向链接、知识图谱和每日笔记。

## 核心功能
| 功能 | 说明 |
| --- | --- |
| 双向链接 | 使用 [[知识图谱]] 连接不同笔记 |
| 知识图谱 | 在图谱视图中查看笔记之间的关系 |
| 每日笔记 | 通过日历或「今日日记」打开当天的笔记 |
| 分屏编辑 | 左右并排阅读与编辑 |

## 格式示例
- **粗体**、*斜体* 与 ~~删除线~~。
- 行内代码：`cargo run`。
- 标签示例：#入门 #笔记

> 先记录，再整理。
"#,
        ));

        self.notes.push(Note::new(
            "GPUI 与 Zed 集成",
            "GPUI 与 Zed 集成.md",
            r#"# GPUI 与 Zed 集成

Jiyue 的文本缓冲借鉴了 **Zed** 编辑器的设计，界面由 **GPUI** 框架以 GPU 渲染。

## 设计要点
1. **SumTree 缓冲区**：编辑与选区更新保持对数复杂度。
2. **GPU 渲染**：界面由 GPUI 的布局原语驱动。
3. **分屏**：左侧编辑时，可在右侧打开 [[知识图谱]]。

相关阅读：[[欢迎使用 Jiyue]]、[[Jiyue 引擎]]。
"#,
        ));

        self.notes.push(Note::new(
            "知识图谱",
            "知识图谱.md",
            r#"# 知识图谱

在 Jiyue 中，笔记不是孤立的文件，而是相互连接的网络。

## 相关笔记
- [[欢迎使用 Jiyue]]：入门指南。
- [[GPUI 与 Zed 集成]]：引擎细节。
- [[2026-11-03]]：当天的日记。
- [[Jiyue 引擎]]：工程规格。

#知识 #图谱 #笔记
"#,
        ));

        self.notes.push(Note::new(
            "2026-11-03",
            "Daily Notes/2026-11-03.md",
            r#"# 2026-11-03 日记

## 今日安排
- [x] 梳理 Jiyue 的界面结构
- [x] 实现知识图谱视图
- [x] 增加分屏与命令面板

## 记录
- 界面全部使用 GPUI 绘制。
- 将 [[GPUI 与 Zed 集成]] 用于高性能的文本缓冲。
"#,
        ));

        self.notes.push(Note::new(
            "Jiyue 引擎",
            "项目/Jiyue 引擎.md",
            r#"# 项目 / Jiyue 引擎

用 Rust 并借鉴 Zed 的设计思路重新实现笔记引擎的记录。

| 规格 | 数值 | 来源 |
| --- | --- | --- |
| 基础字号 | 16px | 设计稿 |
| 行高 | 1.5 | 设计稿 |
| 顶栏高度 | 40px | 设计稿 |
| 侧栏宽度 | 248px | 设计稿 |

相关：[[欢迎使用 Jiyue]]、[[知识图谱]]。
"#,
        ));
    }

    /// Get currently active note
    pub fn active_note(&self) -> Option<&Note> {
        self.notes.get(self.active_note_index)
    }

    /// Get mutable active note
    pub fn active_note_mut(&mut self) -> Option<&mut Note> {
        self.notes.get_mut(self.active_note_index)
    }

    /// Collect all unique tags across the vault
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags = Vec::new();
        for note in &self.notes {
            for tag in &note.tags {
                if !tags.contains(tag) {
                    tags.push(tag.clone());
                }
            }
        }
        tags
    }

    /// Build backlinks map: Target Note Title -> List of (Source Note Title, preview)
    pub fn get_backlinks(&self, target_title: &str) -> Vec<(String, String)> {
        let mut backlinks = Vec::new();
        let query = format!("[[{}]]", target_title);
        let query_short = target_title.to_lowercase();

        for note in &self.notes {
            if note.title == target_title {
                continue;
            }
            if note.content().contains(&query)
                || note
                    .content()
                    .to_lowercase()
                    .contains(&format!("[[{}]]", query_short))
                || note
                    .extract_wikilinks()
                    .iter()
                    .any(|l| l.eq_ignore_ascii_case(target_title))
            {
                let snippet = note
                    .content()
                    .lines()
                    .find(|line| {
                        line.contains(target_title)
                            || line.contains(&format!("[[{}]]", target_title))
                    })
                    .unwrap_or("Linked reference")
                    .trim()
                    .to_string();
                backlinks.push((note.title.clone(), snippet));
            }
        }
        backlinks
    }

    /// Select or open a note by title (e.g. from wikilink click)
    pub fn open_note_by_title(&mut self, title: &str) -> bool {
        let clean_title = title.trim_matches(|c| c == '[' || c == ']');
        if let Some((idx, _)) = self.notes.iter().enumerate().find(|(_, n)| {
            n.title.eq_ignore_ascii_case(clean_title)
                || n.path.eq_ignore_ascii_case(clean_title)
                || n.path
                    .trim_end_matches(".md")
                    .eq_ignore_ascii_case(clean_title)
        }) {
            self.active_note_index = idx;
            if let Some(tab_pos) = self.open_tabs.iter().position(|&t| t == idx) {
                self.active_tab_index = tab_pos;
            } else {
                self.open_tabs.push(idx);
                self.active_tab_index = self.open_tabs.len() - 1;
            }
            true
        } else {
            false
        }
    }

    /// Index of the note titled `title` (case-insensitive).
    pub fn note_index_by_title(&self, title: &str) -> Option<usize> {
        self.notes
            .iter()
            .position(|n| n.title.eq_ignore_ascii_case(title))
    }

    /// Creates the note `title` in the vault root (an empty note with a `# title` heading) and
    /// saves it. Used when a wikilink points at a missing note. Returns the note's index; the
    /// caller opens it. An existing note with the same title is returned unchanged.
    pub fn create_link_note(&mut self, title: &str) -> Result<usize, String> {
        let title = safe_note_title(title).ok_or_else(|| "invalid note title".to_string())?;
        if let Some(idx) = self.note_index_by_title(&title) {
            return Ok(idx);
        }
        let filename = format!("{}.md", title);
        if self.root_dir.join(&filename).exists() {
            return Err(format!("{} already exists", filename));
        }
        let content = format!("# {}\n\n", title);
        self.notes.push(Note::new(&title, &filename, &content));
        let idx = self.notes.len() - 1;
        if let Err(err) = self.save_note_to_disk(idx) {
            self.notes.pop();
            return Err(err.to_string());
        }
        self.rebuild_graph();
        Ok(idx)
    }

    /// Close a tab
    pub fn close_tab(&mut self, tab_pos: usize) {
        if tab_pos < self.open_tabs.len() {
            self.open_tabs.remove(tab_pos);
            if self.open_tabs.is_empty() {
                self.open_tabs.push(0);
                self.active_tab_index = 0;
                self.active_note_index = 0;
            } else if self.active_tab_index >= self.open_tabs.len() {
                self.active_tab_index = self.open_tabs.len() - 1;
                self.active_note_index = self.open_tabs[self.active_tab_index];
            } else {
                self.active_note_index = self.open_tabs[self.active_tab_index];
            }
        }
    }
}
