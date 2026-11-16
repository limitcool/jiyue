//! Local-first capture stream (随手记), todo aggregation (待办), and unified standard templates.
//!
//! All data lives in plain Markdown files inside the vault so daily notes, inbox captures, and
//! todo lists work without any external service. Settings are stored in `<vault>/.jiyue/config.toml`
//! under `[inbox]`.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::daily::{self, Date};

/// Default folder for inbox / fleeting captures (relative to the vault root).
pub const DEFAULT_INBOX_FOLDER: &str = "Inbox";
/// Default folder for dedicated todo checklists (relative to the vault root).
pub const DEFAULT_TODO_FOLDER: &str = "Todo";
/// Default stream file stem inside the inbox folder.
pub const DEFAULT_STREAM_FILE: &str = "随手记";

/// Built-in standard template for daily notes (uses unified variables such as `{{cursor}}`).
pub const STANDARD_DAILY_TEMPLATE: &str =
    "# {{date:YYYY年M月D日}} {{weekday}}\n\n## 今日待办\n- [ ] {{cursor}}\n\n## 记录\n\n## 回顾\n";

/// Built-in standard template for dedicated todo / project checklists.
pub const DEFAULT_TODO_TEMPLATE: &str = "# {{title}}\n\n> 创建于 {{date}} {{weekday}}\n\n## 核心目标\n- \n\n## 待办事项\n- [ ] {{cursor}}\n\n## 备忘\n";

/// Built-in standard template for standalone inbox cards.
pub const DEFAULT_INBOX_CARD_TEMPLATE: &str =
    "# {{title}}\n\n> {{datetime}} · #随手记\n\n{{cursor}}\n";

/// File names generated in the templates folder by [`ensure_standard_templates`].
pub const STANDARD_TEMPLATE_FILES: [(&str, &str); 3] = [
    ("日记模板.md", STANDARD_DAILY_TEMPLATE),
    ("待办清单.md", DEFAULT_TODO_TEMPLATE),
    ("随手记卡片.md", DEFAULT_INBOX_CARD_TEMPLATE),
];

/// Where quick captures from the sidebar are written in the local vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// Append to a configurable stream file in the inbox folder, grouped by `## YYYY-MM-DD` sections.
    #[default]
    StreamFile,
    /// Append into today's daily note (`## 今日待办` for tasks, `## 记录` for notes).
    DailyNote,
    /// Create a standalone Markdown card note inside the inbox folder for each capture.
    CardNote,
}

/// Which notes the sidebar todo dashboard aggregates `- [ ]` / `- [x]` items from by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TodoScope {
    /// Only scan core workflow folders (daily folder + inbox folder + todo folder).
    #[default]
    CoreFolders,
    /// Scan every Markdown note in the vault (excluding the templates folder).
    AllNotes,
}

/// Settings for local captures, todo aggregation, and core folders (`[inbox]` in `.jiyue/config.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InboxSettings {
    /// Vault-relative folder for fleeting captures / inbox notes.
    pub folder: String,
    /// Vault-relative folder for dedicated todo checklists.
    pub todo_folder: String,
    /// Where quick captures are persisted locally.
    pub capture_mode: CaptureMode,
    /// File name pattern for `CaptureMode::StreamFile`: literal stem like `"随手记"` or a date pattern like `"YYYY-MM"` / `"YYYY"`.
    pub stream_file: String,
    /// Default scan scope for the local todo list.
    pub todo_scope: TodoScope,
    /// Custom template file (vault-relative) for inbox cards; empty uses built-in standard template.
    pub inbox_template_path: String,
    /// Custom template file (vault-relative) for todo checklists; empty uses built-in standard template.
    pub todo_template_path: String,
}

impl Default for InboxSettings {
    fn default() -> Self {
        Self {
            folder: DEFAULT_INBOX_FOLDER.to_string(),
            todo_folder: DEFAULT_TODO_FOLDER.to_string(),
            capture_mode: CaptureMode::StreamFile,
            stream_file: DEFAULT_STREAM_FILE.to_string(),
            todo_scope: TodoScope::CoreFolders,
            inbox_template_path: String::new(),
            todo_template_path: String::new(),
        }
    }
}

impl InboxSettings {
    /// Path of `<vault>/.jiyue/config.toml`.
    pub fn settings_path(vault_root: &Path) -> PathBuf {
        config::vault_file(vault_root, config::VAULT_CONFIG_FILE)
    }

    /// Loads `[inbox]` from `<vault>/.jiyue/config.toml`; missing or invalid files fall back to defaults.
    pub fn load(vault_root: &Path) -> Self {
        config::load_section::<Self>(&Self::settings_path(vault_root), "inbox").value
    }

    /// Writes `[inbox]` atomically into `<vault>/.jiyue/config.toml`, keeping all other sections and comments.
    pub fn save(&self, vault_root: &Path) -> io::Result<()> {
        config::save_section(
            &Self::settings_path(vault_root),
            "inbox",
            self,
            config::templates::VAULT_CONFIG,
            &[],
        )
        .map_err(io::Error::other)
    }

    /// Vault-relative path of the capture stream file for `date`, using `/` separators.
    pub fn stream_rel_path(&self, date: Date) -> String {
        let raw_stem = self.stream_file.trim();
        let stem_base = raw_stem
            .strip_suffix(".md")
            .or_else(|| raw_stem.strip_suffix(".MD"))
            .unwrap_or(raw_stem)
            .trim();
        let stem = if stem_base.is_empty() {
            DEFAULT_STREAM_FILE.to_string()
        } else if stem_base.contains("YYYY") || stem_base.contains("YY") {
            let rendered = daily::format_date(date, stem_base);
            if rendered.trim().is_empty() {
                DEFAULT_STREAM_FILE.to_string()
            } else {
                rendered
            }
        } else {
            stem_base.to_string()
        };
        let folder = normalize_folder(&self.folder);
        if folder.is_empty() {
            format!("{stem}.md")
        } else {
            format!("{folder}/{stem}.md")
        }
    }

    /// Display stem (without folder or `.md`) for the stream file on `date`.
    pub fn stream_title(&self, date: Date) -> String {
        let rel = self.stream_rel_path(date);
        Path::new(&rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(DEFAULT_STREAM_FILE)
            .to_string()
    }
}

/// Priority group for aggregated local todo items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TodoGroup {
    TodayDaily = 0,
    CoreFolder = 1,
    OtherNote = 2,
}

/// A Markdown task item (`- [ ]` or `- [x]`) extracted from a local note in the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTodo {
    pub note_path: String,
    pub note_title: String,
    pub group: TodoGroup,
    pub line_index: usize,
    pub text: String,
    pub done: bool,
    pub tags: Vec<String>,
}

/// A local fleeting capture entry extracted from an inbox stream, inbox card, or today's daily log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCaptureItem {
    pub note_path: String,
    pub note_title: String,
    /// `Some(line_idx)` for list items in stream/daily notes; `None` for standalone card notes.
    pub line_index: Option<usize>,
    pub date_label: String,
    pub time_label: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
}

fn normalize_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}

fn normalize_folder(folder: &str) -> String {
    normalize_path(folder).trim_matches('/').to_string()
}

fn is_safe_rel_folder(folder: &str) -> bool {
    !folder.is_empty()
        && !folder.starts_with('/')
        && !folder.contains('\\')
        && Path::new(folder)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn path_in_folder(note_path: &str, folder: &str) -> bool {
    let clean_folder = normalize_folder(folder);
    if clean_folder.is_empty() {
        return false;
    }
    let clean_path = normalize_path(note_path);
    let prefix = format!("{clean_folder}/");
    clean_path.len() >= prefix.len()
        && clean_path
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(&prefix))
}

/// Creates any missing standard template files (`日记模板.md`, `待办清单.md`, `随手记卡片.md`)
/// inside `templates_folder` without overwriting existing files. Returns the newly created
/// vault-relative paths.
pub fn ensure_standard_templates(
    vault_root: &Path,
    templates_folder: &str,
) -> io::Result<Vec<String>> {
    let folder = normalize_folder(templates_folder);
    let folder = if folder.is_empty() {
        "Templates".to_string()
    } else {
        folder
    };
    if !is_safe_rel_folder(&folder) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("模板目录路径无效：{folder}"),
        ));
    }
    let dir_abs = vault_root.join(&folder);
    fs::create_dir_all(&dir_abs)?;

    let mut created = Vec::new();
    for (file_name, body) in STANDARD_TEMPLATE_FILES {
        let target = dir_abs.join(file_name);
        if target.exists() {
            continue;
        }
        crate::fsops::write_atomic(&target, body.as_bytes())
            .map_err(|err| io::Error::other(err.to_string()))?;
        created.push(format!("{folder}/{file_name}"));
    }
    Ok(created)
}

/// Lists top-level user directories inside `vault_root`, skipping dot-prefixed internal folders
/// such as `.jiyue`, `.git`, `.obsidian`, and `.trash`. Sorted case-insensitively and capped at 12.
pub fn list_vault_top_folders(vault_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(vault_root) else {
        return Vec::new();
    };
    let mut dirs: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let name = entry.file_name().into_string().ok()?;
            if name.starts_with('.') {
                return None;
            }
            Some(name)
        })
        .collect();
    dirs.sort_by_key(|a| a.to_lowercase());
    dirs.truncate(12);
    dirs
}

/// Formats one capture line (and optional indented continuation lines) as a Markdown list item.
pub fn format_capture_entry(hhmm: &str, raw_text: &str, as_todo: bool) -> String {
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut lines = trimmed.lines();
    let first = lines.next().unwrap_or("").trim();
    let trimmed_todo = first
        .strip_prefix("- [ ] ")
        .or_else(|| first.strip_prefix("- [x] "))
        .or_else(|| first.strip_prefix("- [X] "));

    let first_line = if trimmed_todo.is_some() {
        first.to_string()
    } else if as_todo {
        format!("- [ ] {first}")
    } else {
        let rest = first.strip_prefix("- ").unwrap_or(first);
        if hhmm.trim().is_empty() {
            format!("- {rest}")
        } else {
            format!("- {} {rest}", hhmm.trim())
        }
    };

    let mut out = vec![first_line];
    for extra in lines {
        let extra = extra.trim_end();
        if extra.is_empty() {
            continue;
        }
        if extra.starts_with("  ") {
            out.push(extra.to_string());
        } else {
            out.push(format!("  {}", extra.trim_start()));
        }
    }
    out.join("\n")
}

/// Extracts hashtags (`#tag`, `#work/sub`) from Markdown text while ignoring `# heading` markers.
pub fn extract_tags(content: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for word in content.split_whitespace() {
        let Some(rest) = word.strip_prefix('#') else {
            continue;
        };
        if rest.is_empty() || rest.starts_with('#') {
            continue;
        }
        let clean =
            rest.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '_' && c != '-');
        if !clean.is_empty() && seen.insert(clean.to_string()) {
            out.push(clean.to_string());
        }
    }
    out
}

/// Parses a Markdown checkbox line (`- [ ] ...` or `- [x] ...`).
pub fn parse_todo_line(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
        return Some((false, rest.trim()));
    }
    if let Some(rest) = trimmed
        .strip_prefix("- [x] ")
        .or_else(|| trimmed.strip_prefix("- [X] "))
    {
        return Some((true, rest.trim()));
    }
    None
}

/// Toggles `- [ ]` <-> `- [x]` on zero-based `line_index`, preserving leading indentation and
/// trailing newline behavior. Returns `None` if `line_index` is not a checkbox line.
pub fn toggle_todo_line(content: &str, line_index: usize) -> Option<String> {
    let lines: Vec<&str> = content.split('\n').collect();
    if line_index >= lines.len() {
        return None;
    }
    let line = lines[line_index];
    let has_cr = line.ends_with('\r');
    let raw = line.strip_suffix('\r').unwrap_or(line);
    let indent_len = raw.len() - raw.trim_start().len();
    let indent = &raw[..indent_len];
    let trimmed = &raw[indent_len..];

    let toggled = if let Some(rest) = trimmed.strip_prefix("- [ ]") {
        format!("{indent}- [x]{rest}")
    } else {
        let rest = trimmed
            .strip_prefix("- [x]")
            .or_else(|| trimmed.strip_prefix("- [X]"))?;
        format!("{indent}- [ ]{rest}")
    };

    let mut rebuilt = Vec::with_capacity(lines.len());
    for (idx, l) in lines.iter().enumerate() {
        if idx == line_index {
            if has_cr {
                rebuilt.push(format!("{toggled}\r"));
            } else {
                rebuilt.push(toggled.clone());
            }
        } else {
            rebuilt.push((*l).to_string());
        }
    }
    Some(rebuilt.join("\n"))
}

/// Deletes the list entry at `line_index` along with any immediately following indented continuation
/// lines (at least 2 leading spaces and not starting a new `- ` list item).
pub fn delete_list_entry(content: &str, line_index: usize) -> Option<String> {
    let mut lines: Vec<&str> = content.split('\n').collect();
    // Preserve trailing newline sentinel as the last empty element when `content.ends_with('\n')`.
    let ends_nl = content.ends_with('\n');
    if ends_nl && lines.last() == Some(&"") {
        lines.pop();
    }
    if line_index >= lines.len() {
        return None;
    }
    let target = lines[line_index].trim_start();
    if !target.starts_with("- ") && !target.starts_with("* ") {
        return None;
    }

    let mut end = line_index + 1;
    while end < lines.len() {
        let next = lines[end];
        let trimmed = next.trim_start();
        let is_indented = next.starts_with("  ") || next.starts_with('\t');
        if is_indented
            && !trimmed.is_empty()
            && !trimmed.starts_with("- ")
            && !trimmed.starts_with("* ")
        {
            end += 1;
        } else {
            break;
        }
    }

    let mut remaining: Vec<&str> = Vec::with_capacity(lines.len() - (end - line_index));
    remaining.extend_from_slice(&lines[..line_index]);
    remaining.extend_from_slice(&lines[end..]);

    let mut out = remaining.join("\n");
    if ends_nl && !out.is_empty() {
        out.push('\n');
    }
    Some(out)
}

/// Appends `raw_text` to an inbox stream Markdown document grouped by `## YYYY-MM-DD 星期X`.
/// If today's section exists, the entry is appended at the end of today's section; otherwise today's
/// section is inserted right after the top `# Title` so newest dates stay at the top.
pub fn append_to_stream_markdown(
    existing: &str,
    stream_title: &str,
    date: Date,
    hhmm: &str,
    raw_text: &str,
    as_todo: bool,
) -> String {
    let entry = format_capture_entry(hhmm, raw_text, as_todo);
    if entry.is_empty() {
        return existing.to_string();
    }
    let title = if stream_title.trim().is_empty() {
        DEFAULT_STREAM_FILE
    } else {
        stream_title.trim()
    };
    let date_prefix = format!("## {}", daily::format_date(date, "YYYY-MM-DD"));
    let date_heading = format!("{date_prefix} {}", daily::weekday_name_zh(date));

    if existing.trim().is_empty() {
        return format!("# {title}\n\n{date_heading}\n{entry}\n");
    }

    let normalized = existing.replace("\r\n", "\n");
    let mut lines: Vec<String> = normalized.split('\n').map(str::to_string).collect();
    if normalized.ends_with('\n') && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    // Case 1: today's `## YYYY-MM-DD` section already exists -> append before next `## ` heading.
    if let Some(sec_start) = lines
        .iter()
        .position(|l| l.trim_start().starts_with(&date_prefix))
    {
        let next_sec = (sec_start + 1..lines.len())
            .find(|&i| lines[i].trim_start().starts_with("## "))
            .unwrap_or(lines.len());
        let mut insert_at = next_sec;
        while insert_at > sec_start + 1 && lines[insert_at - 1].trim().is_empty() {
            insert_at -= 1;
        }
        let entry_lines: Vec<String> = entry.lines().map(str::to_string).collect();
        let entry_count = entry_lines.len();
        lines.splice(insert_at..insert_at, entry_lines);
        // Ensure a single blank separator before the following `## ` section if any.
        let after = insert_at + entry_count;
        if after < lines.len() && !lines[after].trim().is_empty() {
            lines.insert(after, String::new());
        }
        return format!("{}\n", lines.join("\n"));
    }

    // Case 2: insert today's new section after the first `# ` heading (or at the top).
    let insert_at = if let Some(h1) = lines
        .iter()
        .position(|l| l.starts_with("# ") && !l.starts_with("##"))
    {
        let mut idx = h1 + 1;
        while idx < lines.len() && lines[idx].trim().is_empty() {
            idx += 1;
        }
        idx
    } else {
        0
    };

    let mut block: Vec<String> = Vec::new();
    if insert_at > 0 && !lines[insert_at - 1].trim().is_empty() {
        block.push(String::new());
    }
    block.push(date_heading);
    for l in entry.lines() {
        block.push(l.to_string());
    }
    if insert_at < lines.len() {
        block.push(String::new());
    }
    lines.splice(insert_at..insert_at, block);
    format!("{}\n", lines.join("\n"))
}

/// Appends a todo item (to `## 今日待办` / `## 待办`) or a timestamped note (to `## 记录` / `## 随手记`)
/// inside a daily note. If `## 今日待办` only holds an empty `- [ ]` placeholder, replaces it.
pub fn append_to_daily_markdown(
    existing: &str,
    hhmm: &str,
    raw_text: &str,
    as_todo: bool,
) -> String {
    let entry = format_capture_entry(hhmm, raw_text, as_todo);
    if entry.is_empty() {
        return existing.to_string();
    }
    let normalized = existing.replace("\r\n", "\n");
    let mut lines: Vec<String> = normalized.split('\n').map(str::to_string).collect();
    if normalized.ends_with('\n') && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    let targets: &[&str] = if as_todo {
        &["## 今日待办", "## 待办"]
    } else {
        &["## 记录", "## 随手记"]
    };

    if let Some(sec_start) = lines
        .iter()
        .position(|l| targets.iter().any(|h| l.trim() == *h))
    {
        let next_sec = (sec_start + 1..lines.len())
            .find(|&i| lines[i].trim_start().starts_with("## "))
            .unwrap_or(lines.len());

        // Replace empty `- [ ]` template placeholder when adding a todo.
        if as_todo
            && let Some(empty_idx) = (sec_start + 1..next_sec).find(|&i| lines[i].trim() == "- [ ]")
        {
            let entry_lines: Vec<String> = entry.lines().map(str::to_string).collect();
            lines.splice(empty_idx..=empty_idx, entry_lines);
            return format!("{}\n", lines.join("\n"));
        }

        let mut insert_at = next_sec;
        while insert_at > sec_start + 1 && lines[insert_at - 1].trim().is_empty() {
            insert_at -= 1;
        }
        let entry_lines: Vec<String> = entry.lines().map(str::to_string).collect();
        let entry_count = entry_lines.len();
        lines.splice(insert_at..insert_at, entry_lines);
        let after = insert_at + entry_count;
        if after < lines.len() && !lines[after].trim().is_empty() {
            lines.insert(after, String::new());
        }
        return format!("{}\n", lines.join("\n"));
    }

    // Target heading missing -> append section cleanly at the end of the note.
    let heading = targets[0];
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(heading.to_string());
    for l in entry.lines() {
        lines.push(l.to_string());
    }
    format!("{}\n", lines.join("\n"))
}

/// Aggregates Markdown todo items from `(note_path, note_title, content)` pairs according to
/// `inbox.todo_scope` while always skipping files inside `templates_folder` and blank `- [ ]`
/// template placeholders.
pub fn collect_local_todos<'a>(
    notes: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    today_rel_path: &str,
    daily_folder: &str,
    templates_folder: &str,
    inbox: &InboxSettings,
) -> Vec<LocalTodo> {
    let today_norm = normalize_path(today_rel_path);
    let mut todos = Vec::new();

    for (raw_path, title, content) in notes {
        let note_path = normalize_path(raw_path);
        if path_in_folder(&note_path, templates_folder) {
            continue;
        }
        let is_today = !today_norm.is_empty() && note_path.eq_ignore_ascii_case(&today_norm);
        let in_core = is_today
            || path_in_folder(&note_path, daily_folder)
            || path_in_folder(&note_path, &inbox.folder)
            || path_in_folder(&note_path, &inbox.todo_folder);

        if inbox.todo_scope == TodoScope::CoreFolders && !in_core {
            continue;
        }

        let group = if is_today {
            TodoGroup::TodayDaily
        } else if in_core {
            TodoGroup::CoreFolder
        } else {
            TodoGroup::OtherNote
        };

        for (line_index, raw_line) in content.lines().enumerate() {
            let Some((done, task_text)) = parse_todo_line(raw_line) else {
                continue;
            };
            if task_text.is_empty() {
                continue;
            }
            let tags = extract_tags(task_text);
            todos.push(LocalTodo {
                note_path: note_path.clone(),
                note_title: title.to_string(),
                group,
                line_index,
                text: task_text.to_string(),
                done,
                tags,
            });
        }
    }

    todos.sort_by(|a, b| {
        a.done
            .cmp(&b.done)
            .then_with(|| a.group.cmp(&b.group))
            .then_with(|| b.note_path.cmp(&a.note_path))
            .then_with(|| a.line_index.cmp(&b.line_index))
    });
    todos
}

/// Collects local fleeting capture entries for the `随手记` timeline tab:
/// - Date-sectioned list items from the inbox stream file & inbox folder stream-style notes
/// - Standalone card notes under `inbox.folder`
/// - Timestamped `- HH:MM ...` entries inside today's `## 记录` / `## 随手记` daily section
pub fn collect_local_captures<'a>(
    notes: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    today_rel_path: &str,
    stream_rel_path: &str,
    templates_folder: &str,
    inbox: &InboxSettings,
) -> Vec<LocalCaptureItem> {
    let today_norm = normalize_path(today_rel_path);
    let stream_norm = normalize_path(stream_rel_path);
    let mut items = Vec::new();

    for (raw_path, title, content) in notes {
        let note_path = normalize_path(raw_path);
        if path_in_folder(&note_path, templates_folder) {
            continue;
        }
        let is_stream = !stream_norm.is_empty() && note_path.eq_ignore_ascii_case(&stream_norm);
        let is_inbox_file = path_in_folder(&note_path, &inbox.folder);
        let is_today = !today_norm.is_empty() && note_path.eq_ignore_ascii_case(&today_norm);

        if is_stream || is_inbox_file {
            if is_date_stream_document(content) {
                items.extend(parse_stream_entries(&note_path, title, content));
            } else if !is_stream {
                if let Some(card) = parse_card_capture(&note_path, title, content) {
                    items.push(card);
                }
            } else {
                // Stream file without `## YYYY-MM-DD` yet: still extract any top-level list entries.
                items.extend(parse_stream_entries(&note_path, title, content));
            }
        } else if is_today {
            items.extend(parse_daily_log_entries(&note_path, title, content));
        }
    }

    items
}

fn is_date_stream_document(content: &str) -> bool {
    content.lines().any(|l| is_date_h2(l).is_some())
}

fn is_date_h2(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("## ")?;
    let mut bytes = rest.as_bytes().iter().copied();
    let is_yyyy_mm_dd = bytes.len() >= 10
        && rest.as_bytes()[0..4].iter().all(u8::is_ascii_digit)
        && rest.as_bytes()[4] == b'-'
        && rest.as_bytes()[5..7].iter().all(u8::is_ascii_digit)
        && rest.as_bytes()[7] == b'-'
        && rest.as_bytes()[8..10].iter().all(u8::is_ascii_digit);
    let _ = bytes.next();
    if is_yyyy_mm_dd {
        Some(rest.trim())
    } else {
        None
    }
}

fn split_time_prefix(item_body: &str) -> (Option<String>, &str) {
    let b = item_body.as_bytes();
    if b.len() >= 6
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2] == b':'
        && b[3].is_ascii_digit()
        && b[4].is_ascii_digit()
        && b[5] == b' '
    {
        return (
            Some(item_body[..5].to_string()),
            item_body[6..].trim_start(),
        );
    }
    (None, item_body)
}

fn parse_stream_entries(note_path: &str, note_title: &str, content: &str) -> Vec<LocalCaptureItem> {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = Vec::new();
    let mut current_date = note_title.to_string();
    let mut current_bucket: Vec<LocalCaptureItem> = Vec::new();
    let mut i = 0;

    let flush_bucket = |out: &mut Vec<LocalCaptureItem>, bucket: &mut Vec<LocalCaptureItem>| {
        if !bucket.is_empty() {
            bucket.reverse();
            out.append(bucket);
        }
    };

    while i < lines.len() {
        let line = lines[i];
        if let Some(date_hdr) = is_date_h2(line) {
            flush_bucket(&mut out, &mut current_bucket);
            current_date = date_hdr.to_string();
            i += 1;
            continue;
        }
        let trimmed = line.trim_start();
        let is_top_bullet = !line.starts_with("  ")
            && !line.starts_with('\t')
            && (trimmed.starts_with("- ") || trimmed.starts_with("* "));
        if is_top_bullet {
            let body_first = &trimmed[2..];
            let (time_label, first_clean) = if parse_todo_line(line).is_some() {
                (None, trimmed)
            } else {
                split_time_prefix(body_first)
            };
            let mut body_parts = vec![first_clean.to_string()];
            let entry_line = i;
            i += 1;
            while i < lines.len() {
                let next = lines[i];
                let nt = next.trim_start();
                let is_cont = (next.starts_with("  ") || next.starts_with('\t'))
                    && !nt.is_empty()
                    && !nt.starts_with("- ")
                    && !nt.starts_with("* ");
                if is_cont {
                    body_parts.push(nt.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            let full_content = body_parts.join("\n").trim().to_string();
            if !full_content.is_empty() {
                let tags = extract_tags(&full_content);
                current_bucket.push(LocalCaptureItem {
                    note_path: note_path.to_string(),
                    note_title: note_title.to_string(),
                    line_index: Some(entry_line),
                    date_label: current_date.clone(),
                    time_label,
                    content: full_content,
                    tags,
                });
            }
            continue;
        }
        i += 1;
    }

    flush_bucket(&mut out, &mut current_bucket);
    out
}

fn parse_daily_log_entries(
    note_path: &str,
    note_title: &str,
    content: &str,
) -> Vec<LocalCaptureItem> {
    let lines: Vec<&str> = content.lines().collect();
    let Some(sec_start) = lines
        .iter()
        .position(|l| matches!(l.trim(), "## 记录" | "## 随手记"))
    else {
        return Vec::new();
    };
    let next_sec = (sec_start + 1..lines.len())
        .find(|&idx| lines[idx].trim_start().starts_with("## "))
        .unwrap_or(lines.len());

    let mut out = Vec::new();
    let mut i = sec_start + 1;
    while i < next_sec {
        let line = lines[i];
        let trimmed = line.trim_start();
        if !line.starts_with("  ")
            && !line.starts_with('\t')
            && (trimmed.starts_with("- ") || trimmed.starts_with("* "))
        {
            let body_first = &trimmed[2..];
            let (time_label, first_clean) = split_time_prefix(body_first);
            let mut body_parts = vec![first_clean.to_string()];
            let entry_line = i;
            i += 1;
            while i < next_sec {
                let next = lines[i];
                let nt = next.trim_start();
                if (next.starts_with("  ") || next.starts_with('\t'))
                    && !nt.is_empty()
                    && !nt.starts_with("- ")
                    && !nt.starts_with("* ")
                {
                    body_parts.push(nt.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            let full_content = body_parts.join("\n").trim().to_string();
            if !full_content.is_empty() {
                let tags = extract_tags(&full_content);
                out.push(LocalCaptureItem {
                    note_path: note_path.to_string(),
                    note_title: note_title.to_string(),
                    line_index: Some(entry_line),
                    date_label: format!("今日日记 · {note_title}"),
                    time_label,
                    content: full_content,
                    tags,
                });
            }
            continue;
        }
        i += 1;
    }
    out.reverse();
    out
}

fn parse_card_capture(
    note_path: &str,
    note_title: &str,
    content: &str,
) -> Option<LocalCaptureItem> {
    let tags = extract_tags(content);
    let mut body_lines = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if body_lines.is_empty() && (t.starts_with("# ") || t.is_empty()) {
            continue;
        }
        body_lines.push(line);
    }
    let body = body_lines.join("\n").trim().to_string();
    let display = if body.is_empty() {
        note_title.to_string()
    } else {
        body
    };
    Some(LocalCaptureItem {
        note_path: note_path.to_string(),
        note_title: note_title.to_string(),
        line_index: None,
        date_label: "闪念卡片".to_string(),
        time_label: None,
        content: display,
        tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> Date {
        Date::new(y, m, day).expect("valid date")
    }

    fn temp_vault(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jiyue-inbox-test-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn settings_round_trip_and_stream_rel_path() {
        let dir = temp_vault("settings");
        assert_eq!(InboxSettings::load(&dir), InboxSettings::default());

        let custom = InboxSettings {
            folder: "📥 Inbox 闪念收集箱".to_string(),
            todo_folder: "待办".to_string(),
            capture_mode: CaptureMode::DailyNote,
            stream_file: "YYYY-MM".to_string(),
            todo_scope: TodoScope::AllNotes,
            inbox_template_path: "_templates/随手记卡片.md".to_string(),
            todo_template_path: "_templates/待办清单.md".to_string(),
        };
        custom.save(&dir).unwrap();
        assert_eq!(InboxSettings::load(&dir), custom);
        assert_eq!(
            custom.stream_rel_path(d(2026, 11, 16)),
            "📥 Inbox 闪念收集箱/2026-11.md"
        );
        assert_eq!(
            InboxSettings::default().stream_rel_path(d(2026, 11, 16)),
            "Inbox/随手记.md"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_to_stream_creates_and_groups_dates_newest_first() {
        let s1 = append_to_stream_markdown(
            "",
            "随手记",
            d(2026, 11, 15),
            "09:10",
            "第一条灵感 #工作",
            false,
        );
        assert_eq!(
            s1,
            "# 随手记\n\n## 2026-11-15 星期日\n- 09:10 第一条灵感 #工作\n"
        );

        let s2 = append_to_stream_markdown(&s1, "随手记", d(2026, 11, 15), "10:00", "买牛奶", true);
        assert_eq!(
            s2,
            "# 随手记\n\n## 2026-11-15 星期日\n- 09:10 第一条灵感 #工作\n- [ ] 买牛奶\n"
        );

        let s3 =
            append_to_stream_markdown(&s2, "随手记", d(2026, 11, 16), "08:30", "新的一天", false);
        assert_eq!(
            s3,
            "# 随手记\n\n## 2026-11-16 星期一\n- 08:30 新的一天\n\n## 2026-11-15 星期日\n- 09:10 第一条灵感 #工作\n- [ ] 买牛奶\n"
        );
    }

    #[test]
    fn append_to_daily_replaces_empty_todo_placeholder_and_logs() {
        let initial = "# 2026年11月16日 星期一\n\n## 今日待办\n- [ ] \n\n## 记录\n\n## 回顾\n";
        let with_todo = append_to_daily_markdown(initial, "09:00", "写完周报", true);
        assert_eq!(
            with_todo,
            "# 2026年11月16日 星期一\n\n## 今日待办\n- [ ] 写完周报\n\n## 记录\n\n## 回顾\n"
        );

        let with_log = append_to_daily_markdown(&with_todo, "14:20", "临时想到一个点子", false);
        assert_eq!(
            with_log,
            "# 2026年11月16日 星期一\n\n## 今日待办\n- [ ] 写完周报\n\n## 记录\n- 14:20 临时想到一个点子\n\n## 回顾\n"
        );
    }

    #[test]
    fn toggle_and_delete_list_entry_preserve_structure() {
        let doc = "# 随手记\n\n## 2026-11-16 星期一\n- [ ] 任务A\n  补充说明\n- 10:00 普通笔记\n";
        let toggled = toggle_todo_line(doc, 3).expect("toggle");
        assert!(toggled.contains("- [x] 任务A"));

        let deleted = delete_list_entry(&toggled, 3).expect("delete");
        assert_eq!(
            deleted,
            "# 随手记\n\n## 2026-11-16 星期一\n- 10:00 普通笔记\n"
        );
    }

    #[test]
    fn collect_todos_respects_scope_and_skips_templates() {
        let mut settings = InboxSettings::default();
        let notes = [
            (
                "Daily/2026-11-16.md",
                "2026-11-16",
                "## 今日待办\n- [ ] 今日任务 #紧急\n- [ ] \n",
            ),
            (
                "Inbox/随手记.md",
                "随手记",
                "- [x] 收集箱已完成\n- [ ] 收集箱待办",
            ),
            ("Todo/项目A.md", "项目A", "- [ ] 项目待办"),
            ("Archive/旧笔记.md", "旧笔记", "- [ ] 归档里的待办"),
            ("Templates/日记模板.md", "日记模板", "- [ ] {{cursor}}"),
        ];

        let core_only = collect_local_todos(
            notes,
            "Daily/2026-11-16.md",
            "Daily",
            "Templates",
            &settings,
        );
        assert_eq!(core_only.len(), 4);
        assert_eq!(core_only[0].note_path, "Daily/2026-11-16.md");
        assert_eq!(core_only[0].group, TodoGroup::TodayDaily);
        assert_eq!(core_only[0].tags, vec!["紧急"]);
        assert!(!core_only.iter().any(|t| t.note_path.contains("Archive")));
        assert!(!core_only.iter().any(|t| t.note_path.contains("Templates")));

        settings.todo_scope = TodoScope::AllNotes;
        let all = collect_local_todos(
            notes,
            "Daily/2026-11-16.md",
            "Daily",
            "Templates",
            &settings,
        );
        assert_eq!(all.len(), 5);
        assert!(all.iter().any(|t| t.note_path == "Archive/旧笔记.md"));
        assert!(!all.iter().any(|t| t.note_path.contains("Templates")));
    }

    #[test]
    fn collect_local_captures_parses_stream_cards_and_daily_logs() {
        let settings = InboxSettings::default();
        let notes = [
            (
                "Inbox/随手记.md",
                "随手记",
                "# 随手记\n\n## 2026-11-16 星期一\n- 09:30 晨间想法 #灵感\n  第二行说明\n- 10:15 第二条想法\n",
            ),
            (
                "Inbox/独立卡片.md",
                "独立卡片",
                "# 独立卡片\n\n卡片正文 #卡片",
            ),
            (
                "Daily/2026-11-16.md",
                "2026-11-16",
                "# 2026年11月16日\n\n## 记录\n- 15:00 今日日记随手记\n\n## 回顾\n",
            ),
            (
                "Templates/随手记卡片.md",
                "随手记卡片",
                "# {{title}}\n{{cursor}}",
            ),
        ];
        let caps = collect_local_captures(
            notes,
            "Daily/2026-11-16.md",
            "Inbox/随手记.md",
            "Templates",
            &settings,
        );
        assert_eq!(caps.len(), 4);
        assert_eq!(caps[0].time_label.as_deref(), Some("10:15"));
        assert_eq!(caps[1].time_label.as_deref(), Some("09:30"));
        assert_eq!(caps[1].line_index, Some(3));
        assert!(caps[1].content.contains("第二行说明"));
        assert_eq!(caps[2].line_index, None);
        assert!(caps[2].content.contains("卡片正文"));
        assert_eq!(caps[3].time_label.as_deref(), Some("15:00"));
        assert_eq!(caps[3].note_path, "Daily/2026-11-16.md");
    }

    #[test]
    fn ensure_standard_templates_creates_missing_without_overwriting() {
        let dir = temp_vault("std-tpls");
        fs::create_dir_all(dir.join("_templates")).unwrap();
        fs::write(dir.join("_templates/日记模板.md"), "MY CUSTOM DAILY").unwrap();

        let created = ensure_standard_templates(&dir, "_templates").unwrap();
        assert_eq!(
            created,
            vec!["_templates/待办清单.md", "_templates/随手记卡片.md"]
        );
        assert_eq!(
            fs::read_to_string(dir.join("_templates/日记模板.md")).unwrap(),
            "MY CUSTOM DAILY"
        );
        assert!(dir.join("_templates/待办清单.md").is_file());
        assert!(dir.join("_templates/随手记卡片.md").is_file());

        let second = ensure_standard_templates(&dir, "_templates").unwrap();
        assert!(second.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
