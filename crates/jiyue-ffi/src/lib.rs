#![allow(clippy::not_unsafe_ptr_arg_deref)]

//! jiyue-ffi: Cross-platform C-ABI and Rust bridge for Jiyue Mobile (iOS & Android).
//!
//! Provides thread-safe access to Vault management, Markdown notes, Wikilinks,
//! Daily notes, and S3 / WebDAV synchronization powered by `jiyue-sync`.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use jiyue_core::buffer::{Note, wikilink_targets};
use jiyue_core::daily::{self, Date};
use jiyue_core::inbox::{self, CaptureMode, InboxSettings, TodoGroup};
use jiyue_core::links::{
    LinkKind, LinkOccurrence, NoteRef, extract_links, resolve_link, resolve_occurrence,
};
use jiyue_core::vault::Vault;
use jiyue_md::attach::{AttachSettings, Timestamp, plan_import, write_import};
use jiyue_sync::store::{OpendalStore, StoreConfig};
use jiyue_sync::{SyncOptions, run_sync, state};

/// Backlink item pointing to a note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkItem {
    pub path: String,
    pub title: String,
    pub snippet: String,
}

/// High-level note summary for list views
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSummary {
    pub id: String,
    pub title: String,
    pub path: String,
    pub preview: String,
    pub tags: Vec<String>,
    pub modified_ms: i64,
}

/// Detailed note model for reading and editing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDetail {
    pub id: String,
    pub title: String,
    pub path: String,
    pub content: String,
    pub tags: Vec<String>,
    pub wikilinks: Vec<String>,
    #[serde(default)]
    pub backlinks: Vec<BacklinkItem>,
    pub modified_ms: i64,
}

/// Search match item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub match_type: String, // "title", "tag", "content"
}

/// Report of S3 / WebDAV synchronization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResultReport {
    pub success: bool,
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted_remote: usize,
    pub deleted_local: usize,
    pub conflicts: usize,
    pub message: String,
}

impl SyncResultReport {
    fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            uploaded: 0,
            downloaded: 0,
            deleted_remote: 0,
            deleted_local: 0,
            conflicts: 0,
            message: message.into(),
        }
    }
}

fn run_sync_at_root(root_dir: &Path, config_json: &str, device: &str) -> SyncResultReport {
    let config: StoreConfig = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => {
            return SyncResultReport::failure(format!("Invalid sync configuration JSON: {e}"));
        }
    };

    let store = match OpendalStore::from_config(&config) {
        Ok(s) => s,
        Err(e) => return SyncResultReport::failure(format!("Failed to build remote store: {e}")),
    };

    let mut sync_state = match state::load_state(root_dir) {
        Ok(s) => s,
        Err(e) => {
            return SyncResultReport::failure(format!("Failed to load local sync state: {e}"));
        }
    };

    let mut opts = SyncOptions::new(if device.is_empty() { "Mobile" } else { device });
    let report = run_sync(root_dir, &store, &mut sync_state, &mut opts);

    let is_ok = report.errors.is_empty() && report.fatal.is_none();
    SyncResultReport {
        success: is_ok,
        uploaded: report.uploaded.len(),
        downloaded: report.downloaded.len(),
        deleted_remote: report.deleted_remote.len(),
        deleted_local: report.deleted_local.len(),
        conflicts: report.conflicts.len(),
        message: if is_ok {
            format!(
                "同步成功: 上传 {} 篇，下载 {} 篇，合并冲突 {} 处",
                report.uploaded.len(),
                report.downloaded.len(),
                report.conflicts.len()
            )
        } else {
            format!(
                "同步失败: fatal={:?}, errors={:?}",
                report.fatal, report.errors
            )
        },
    }
}

/// Connection test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTestResult {
    pub success: bool,
    pub reachable: bool,
    pub can_list: bool,
    pub can_write: bool,
    pub message: String,
}

/// Fleeting capture entry for mobile inbox / stream views
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxCaptureDto {
    pub note_path: String,
    pub note_title: String,
    pub line_index: Option<usize>,
    pub date_label: String,
    pub time_label: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
}

/// Markdown task item (`- [ ]` / `- [x]`) aggregated from vault notes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxTodoDto {
    pub note_path: String,
    pub note_title: String,
    pub group: String,
    pub line_index: usize,
    pub text: String,
    pub done: bool,
    pub tags: Vec<String>,
}

/// Aggregated inbox captures, vault todos, and active target paths
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxSummaryDto {
    pub captures: Vec<InboxCaptureDto>,
    pub todos: Vec<InboxTodoDto>,
    pub stream_path: String,
    pub daily_path: String,
}

/// Result of importing a shared file into vault attachments + inbox capture
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportFileResultDto {
    pub success: bool,
    pub dest_rel: String,
    pub link_text: String,
    pub note_path: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct TemplatesFolderConfig {
    templates_folder: String,
}

impl Default for TemplatesFolderConfig {
    fn default() -> Self {
        Self {
            templates_folder: "Templates".to_string(),
        }
    }
}

fn load_templates_folder(root: &Path) -> String {
    let cfg_path = root.join(jiyue_core::config::VAULT_CONFIG_FILE);
    jiyue_core::config::load_section::<TemplatesFolderConfig>(&cfg_path, "templates")
        .value
        .templates_folder
}

fn load_attach_settings(root: &Path) -> AttachSettings {
    let cfg_path = root.join(jiyue_core::config::VAULT_CONFIG_FILE);
    jiyue_core::config::load_section::<AttachSettings>(&cfg_path, "attachments").value
}

fn today_daily_rel(vault: &Vault, today: Date) -> String {
    let ds = vault.daily_settings();
    let title = daily::format_date(today, &ds.date_format);
    let folder = ds.folder.trim().replace('\\', "/");
    let folder = folder.trim_matches('/');
    if folder.is_empty() {
        format!("{title}.md")
    } else {
        format!("{folder}/{title}.md")
    }
}

fn current_timestamp(today: Date, hhmm: &str) -> Timestamp {
    let mut parts = hhmm.split(':');
    let hh = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let mm = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let ss = (std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() % 60)
        .unwrap_or(0)) as u32;
    Timestamp {
        y: today.year.max(1970) as u32,
        m: today.month,
        d: today.day,
        hh,
        mm,
        ss,
    }
}

fn upsert_and_save_note(
    vault: &mut Vault,
    rel_path: &str,
    new_content: &str,
) -> Result<(), String> {
    let norm = rel_path.replace('\\', "/");
    let idx = if let Some(existing) = vault
        .notes
        .iter()
        .position(|n| n.path.replace('\\', "/").eq_ignore_ascii_case(&norm))
    {
        vault.notes[existing].set_content(new_content);
        existing
    } else {
        let stem = Path::new(&norm)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("随手记");
        let note = Note::new(stem, &norm, new_content);
        vault.notes.push(note);
        vault.notes.len() - 1
    };
    vault
        .save_note_to_disk(idx)
        .map_err(|e| format!("Failed to save note {norm}: {e}"))?;
    vault.bump_revision();
    vault.rebuild_graph();
    Ok(())
}

fn allocate_inbox_card_path(root: &Path, folder: &str, raw_text: &str) -> String {
    let clean_folder = folder.trim().replace('\\', "/");
    let clean_folder = clean_folder.trim_matches('/');
    let first_line = raw_text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("随手记");
    let stripped = first_line
        .strip_prefix("- [ ] ")
        .or_else(|| first_line.strip_prefix("- [x] "))
        .or_else(|| first_line.strip_prefix("- [X] "))
        .unwrap_or(first_line)
        .trim();
    let short: String = stripped.chars().take(24).collect();
    let base = jiyue_core::vault::safe_note_title(&short).unwrap_or_else(|| "随手记".to_string());
    let make_rel = |name: &str| -> String {
        if clean_folder.is_empty() {
            format!("{name}.md")
        } else {
            format!("{clean_folder}/{name}.md")
        }
    };
    let mut candidate = make_rel(&base);
    let mut seq = 2usize;
    while root.join(&candidate).exists() {
        candidate = make_rel(&format!("{base} {seq}"));
        seq += 1;
    }
    candidate
}

fn note_refs(vault: &Vault) -> Vec<NoteRef> {
    vault
        .notes
        .iter()
        .map(|n| {
            let norm = n.path.replace('\\', "/");
            let aliases = jiyue_md::frontmatter::aliases(n.content());
            if aliases.is_empty() {
                NoteRef::new(norm)
            } else {
                NoteRef {
                    path: norm,
                    aliases,
                }
            }
        })
        .collect()
}

fn snippet_at_offset(content: &str, byte_offset: usize) -> String {
    let mut cursor = 0usize;
    for line in content.split('\n') {
        let line_end = cursor + line.len();
        if byte_offset <= line_end {
            let clean = line.trim();
            let clipped: String = clean.chars().take(140).collect();
            return if clean.chars().count() > 140 {
                format!("{clipped}...")
            } else {
                clipped
            };
        }
        cursor = line_end + 1;
    }
    String::new()
}

fn collect_backlinks(vault: &Vault, refs: &[NoteRef], target_idx: usize) -> Vec<BacklinkItem> {
    let mut backlinks = Vec::new();
    for (src_idx, src) in vault.notes.iter().enumerate() {
        if src_idx == target_idx {
            continue;
        }
        let content = src.content();
        let src_path = &refs[src_idx].path;
        let matched = extract_links(content)
            .into_iter()
            .find(|occ| resolve_occurrence(refs, occ, src_path) == Some(target_idx));
        if let Some(occ) = matched {
            backlinks.push(BacklinkItem {
                path: src_path.clone(),
                title: src.title.clone(),
                snippet: snippet_at_offset(content, occ.range.start),
            });
        }
    }
    backlinks
}

fn strip_index_prefix(stem: &str) -> &str {
    let bytes = stem.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return stem;
    }
    while i < bytes.len() && matches!(bytes[i], b'-' | b'_' | b'.' | b' ') {
        i += 1;
    }
    let rest = stem[i..].trim();
    if rest.is_empty() { stem } else { rest }
}

fn resolve_folder_index_note(notes: &[Note], from_norm: &str, target: &str) -> Option<String> {
    let before_pipe = target.split('|').next().unwrap_or(target);
    let before_hash = before_pipe.split('#').next().unwrap_or(before_pipe);
    let normalized = before_hash.trim().replace('\\', "/");
    let clean = normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .trim_matches('/');
    if clean.is_empty() {
        return None;
    }
    let base = clean.rsplit('/').next().unwrap_or(clean).trim();

    let mut folders: Vec<String> = Vec::new();
    for n in notes {
        let p = n.path.replace('\\', "/");
        let mut slice = p.as_str();
        while let Some((parent, _)) = slice.rsplit_once('/') {
            if !parent.is_empty() && !folders.iter().any(|f| f == parent) {
                folders.push(parent.to_string());
            }
            slice = parent;
        }
    }

    let from_dir = from_norm.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut matched_folder: Option<String> = None;
    if !from_dir.is_empty() {
        let rel = format!("{from_dir}/{clean}");
        matched_folder = folders
            .iter()
            .find(|f| f.eq_ignore_ascii_case(&rel))
            .cloned();
    }
    if matched_folder.is_none() {
        matched_folder = folders
            .iter()
            .find(|f| f.eq_ignore_ascii_case(clean))
            .cloned();
    }
    if matched_folder.is_none() && !clean.contains('/') {
        let mut leaf_matches: Vec<&String> = folders
            .iter()
            .filter(|f| {
                f.rsplit('/')
                    .next()
                    .is_some_and(|seg| seg.eq_ignore_ascii_case(clean))
            })
            .collect();
        leaf_matches.sort_by_key(|f| (f.chars().count(), f.as_str()));
        if let Some(first) = leaf_matches.first() {
            matched_folder = Some((*first).clone());
        }
    }

    let find_ci = |candidate: &str| -> Option<String> {
        notes
            .iter()
            .map(|n| n.path.replace('\\', "/"))
            .find(|p| p.eq_ignore_ascii_case(candidate))
    };

    if let Some(folder) = matched_folder.as_deref() {
        let folder_base = folder.rsplit('/').next().unwrap_or(folder);
        for candidate in [
            format!("{folder}/{folder_base}.md"),
            format!("{folder}/README.md"),
            format!("{folder}/index.md"),
        ] {
            if let Some(hit) = find_ci(&candidate) {
                return Some(hit);
            }
        }
    }

    if !base.is_empty() {
        if let Some(hit) = find_ci(&format!("_indexes/{base}.md")) {
            return Some(hit);
        }
        let prefix = "_indexes/";
        let mut matches: Vec<String> = notes
            .iter()
            .filter_map(|n| {
                let p = n.path.replace('\\', "/");
                let file = p
                    .get(..prefix.len())
                    .filter(|s| s.eq_ignore_ascii_case(prefix))
                    .and_then(|_| p.get(prefix.len()..))?;
                if file.is_empty() || file.contains('/') {
                    return None;
                }
                let stem = if file.len() >= 3
                    && file.as_bytes()[file.len() - 3..].eq_ignore_ascii_case(b".md")
                {
                    &file[..file.len() - 3]
                } else {
                    file
                };
                strip_index_prefix(stem)
                    .eq_ignore_ascii_case(base)
                    .then_some(p)
            })
            .collect();
        if matches.len() == 1 {
            return Some(matches.remove(0));
        }
    }

    None
}

fn build_note_detail(vault: &Vault, idx: usize) -> NoteDetail {
    let note = &vault.notes[idx];
    let norm_path = note.path.replace('\\', "/");
    let abs_path = vault.root_dir.join(&note.path);
    let modified_ms = std::fs::metadata(&abs_path)
        .and_then(|m| m.modified())
        .and_then(|t| {
            t.duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .map_err(|e| std::io::Error::other(e.to_string()))
        })
        .unwrap_or(0);

    let refs = note_refs(vault);
    let backlinks = collect_backlinks(vault, &refs, idx);
    let content = note.content().to_string();
    let wikilinks = wikilink_targets(&content);

    NoteDetail {
        id: note.id.clone(),
        title: note.title.clone(),
        path: norm_path,
        content,
        tags: note.tags.clone(),
        wikilinks,
        backlinks,
        modified_ms,
    }
}

/// Thread-safe wrapper around a Jiyue Vault
pub struct VaultBridge {
    pub vault: Mutex<Vault>,
}

impl VaultBridge {
    pub fn open_or_create(root_dir: &Path) -> Self {
        let vault = Vault::open_or_create(root_dir);
        Self {
            vault: Mutex::new(vault),
        }
    }

    /// List all notes in the vault sorted by modification time (most recent first)
    pub fn list_notes(&self) -> Vec<NoteSummary> {
        let vault = self.vault.lock().unwrap();
        let mut summaries = Vec::with_capacity(vault.notes.len());

        for note in &vault.notes {
            let abs_path = vault.root_dir.join(&note.path);
            let modified_ms = std::fs::metadata(&abs_path)
                .and_then(|m| m.modified())
                .and_then(|t| {
                    t.duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .map_err(|e| std::io::Error::other(e.to_string()))
                })
                .unwrap_or(0);

            let content = note.content();
            let preview: String = content
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(120)
                .collect();

            summaries.push(NoteSummary {
                id: note.id.clone(),
                title: note.title.clone(),
                path: note.path.replace('\\', "/"),
                preview,
                tags: note.tags.clone(),
                modified_ms,
            });
        }

        // Sort by modified time desc
        summaries.sort_by_key(|a| std::cmp::Reverse(a.modified_ms));
        summaries
    }

    /// Get note detail by path
    pub fn get_note(&self, path: &str) -> Option<NoteDetail> {
        let vault = self.vault.lock().unwrap();
        let norm = path.replace('\\', "/");
        let idx = vault
            .notes
            .iter()
            .position(|n| n.path.replace('\\', "/") == norm)?;
        Some(build_note_detail(&vault, idx))
    }

    /// Save note content and persist atomically
    pub fn save_note(&self, path: &str, new_content: &str) -> Result<(), String> {
        let mut vault = self.vault.lock().unwrap();
        let norm = path.replace('\\', "/");
        let idx = vault
            .notes
            .iter()
            .position(|n| n.path.replace('\\', "/") == norm)
            .ok_or_else(|| format!("Note not found: {path}"))?;

        vault.notes[idx].set_content(new_content);
        vault
            .save_note_to_disk(idx)
            .map_err(|e| format!("Failed to save note: {e}"))?;
        vault.bump_revision();
        vault.rebuild_graph();
        Ok(())
    }

    /// Create a new note
    pub fn create_note(&self, title: &str, content: &str) -> Result<NoteSummary, String> {
        let mut vault = self.vault.lock().unwrap();
        let safe_title = jiyue_core::vault::safe_note_title(title)
            .unwrap_or_else(|| format!("未命名 {}", vault.notes.len() + 1));

        let filename = format!("{}.md", safe_title);
        let note = Note::new(&safe_title, &filename, content);
        vault.notes.push(note);
        let new_idx = vault.notes.len() - 1;
        vault
            .save_note_to_disk(new_idx)
            .map_err(|e| format!("Failed to persist new note: {e}"))?;
        vault.bump_revision();
        vault.rebuild_graph();

        let n = &vault.notes[new_idx];
        Ok(NoteSummary {
            id: n.id.clone(),
            title: n.title.clone(),
            path: n.path.clone(),
            preview: content.chars().take(120).collect(),
            tags: n.tags.clone(),
            modified_ms: std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        })
    }

    /// Delete note by path
    pub fn delete_note(&self, path: &str) -> Result<(), String> {
        let mut vault = self.vault.lock().unwrap();
        let norm = path.replace('\\', "/");
        let idx = vault
            .notes
            .iter()
            .position(|n| n.path.replace('\\', "/") == norm)
            .ok_or_else(|| format!("Note not found: {path}"))?;

        let abs_path = vault.root_dir.join(&vault.notes[idx].path);
        let _ = std::fs::remove_file(abs_path);
        vault.notes.remove(idx);
        vault.bump_revision();
        vault.rebuild_graph();
        Ok(())
    }

    /// Rename note (preserving its parent folder)
    pub fn rename_note(&self, path: &str, new_title: &str) -> Result<String, String> {
        let mut vault = self.vault.lock().unwrap();
        let norm = path.replace('\\', "/");
        let idx = vault
            .notes
            .iter()
            .position(|n| n.path.replace('\\', "/") == norm)
            .ok_or_else(|| format!("Note not found: {path}"))?;

        let safe_title = jiyue_core::vault::safe_note_title(new_title)
            .ok_or_else(|| "Invalid note title".to_string())?;

        let old_norm = vault.notes[idx].path.replace('\\', "/");
        let new_rel = match old_norm.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{safe_title}.md"),
            None => format!("{safe_title}.md"),
        };
        if new_rel == old_norm {
            return Ok(new_rel);
        }

        let old_abs = vault.root_dir.join(&vault.notes[idx].path);
        let new_abs = vault.root_dir.join(&new_rel);
        if !new_rel.eq_ignore_ascii_case(&old_norm) && new_abs.exists() {
            return Err(format!("Target note already exists: {new_rel}"));
        }

        std::fs::rename(&old_abs, &new_abs)
            .map_err(|e| format!("Failed to rename file on disk: {e}"))?;

        vault.notes[idx].title = safe_title;
        vault.notes[idx].path = new_rel.clone();
        vault.bump_revision();
        vault.rebuild_graph();
        Ok(new_rel)
    }

    /// Resolve a wikilink or Markdown relative link target from `from_path`
    pub fn resolve_link_target(&self, from_path: &str, target: &str) -> Option<String> {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return None;
        }
        let vault = self.vault.lock().unwrap();
        let refs = note_refs(&vault);
        let from_norm = from_path.replace('\\', "/");

        if let Some(idx) = resolve_link(&refs, trimmed, &from_norm) {
            return Some(vault.notes[idx].path.replace('\\', "/"));
        }

        let clean_md = trimmed.split('#').next().unwrap_or(trimmed).trim();
        if !clean_md.is_empty() {
            let occ = LinkOccurrence {
                range: 0..0,
                target_range: 0..0,
                kind: LinkKind::MdLink,
                target: clean_md.to_string(),
                anchor: None,
                alias: None,
            };
            if let Some(idx) = resolve_occurrence(&refs, &occ, &from_norm) {
                return Some(vault.notes[idx].path.replace('\\', "/"));
            }
        }

        resolve_folder_index_note(&vault.notes, &from_norm, trimmed)
    }

    /// Full-text search across titles, tags, and content
    pub fn search(&self, query: &str) -> Vec<SearchResultItem> {
        let vault = self.vault.lock().unwrap();
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        for note in &vault.notes {
            if note.title.to_lowercase().contains(&q) {
                results.push(SearchResultItem {
                    path: note.path.replace('\\', "/"),
                    title: note.title.clone(),
                    snippet: format!("标题匹配: {}", note.title),
                    match_type: "title".to_string(),
                });
                continue;
            }

            if note.tags.iter().any(|t| t.to_lowercase().contains(&q)) {
                results.push(SearchResultItem {
                    path: note.path.replace('\\', "/"),
                    title: note.title.clone(),
                    snippet: format!("标签匹配: {}", note.tags.join(" ")),
                    match_type: "tag".to_string(),
                });
                continue;
            }

            let content = note.content();
            if let Some(pos) = content.to_lowercase().find(&q) {
                let snippet = snippet_at_offset(content, pos.min(content.len()));
                results.push(SearchResultItem {
                    path: note.path.replace('\\', "/"),
                    title: note.title.clone(),
                    snippet,
                    match_type: "content".to_string(),
                });
            }
        }
        results
    }

    /// Open or create today's daily note
    pub fn get_or_create_daily(&self) -> Result<NoteDetail, String> {
        let mut vault = self.vault.lock().unwrap();
        let today = Date::today();
        let idx = vault
            .open_or_create_daily(today)
            .map_err(|e| format!("Failed to open or create daily note: {e}"))?;

        Ok(build_note_detail(&vault, idx))
    }

    /// Append a quick log entry to today's daily note using the standard `## 记录` section
    pub fn append_to_daily(&self, entry: &str) -> Result<(), String> {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let mut vault = self.vault.lock().unwrap();
        let today = Date::today();
        let idx = vault
            .open_or_create_daily(today)
            .map_err(|e| format!("Failed to open daily note: {e}"))?;

        let old_content = vault.notes[idx].content().to_string();
        let now_hhmm = daily::local_hhmm();
        let updated = inbox::append_to_daily_markdown(&old_content, &now_hhmm, trimmed, false);

        vault.notes[idx].set_content(&updated);
        vault
            .save_note_to_disk(idx)
            .map_err(|e| format!("Failed to save daily note: {e}"))?;
        vault.bump_revision();
        vault.rebuild_graph();
        Ok(())
    }

    /// Collect local fleeting captures and aggregated Markdown todos across the vault
    pub fn inbox_summary(&self) -> InboxSummaryDto {
        let vault = self.vault.lock().unwrap();
        let root = vault.root_dir.clone();
        let inbox_cfg = InboxSettings::load(&root);
        let templates_folder = load_templates_folder(&root);
        let daily_folder = vault.daily_settings().folder;
        let today = Date::today();
        let today_rel = today_daily_rel(&vault, today);
        let stream_rel = inbox_cfg.stream_rel_path(today);

        let pairs: Vec<(String, String, &str)> = vault
            .notes
            .iter()
            .map(|n| (n.path.replace('\\', "/"), n.title.clone(), n.content()))
            .collect();
        let pair_refs: Vec<(&str, &str, &str)> = pairs
            .iter()
            .map(|(p, t, c)| (p.as_str(), t.as_str(), *c))
            .collect();

        let captures = inbox::collect_local_captures(
            pair_refs.iter().copied(),
            &today_rel,
            &stream_rel,
            &templates_folder,
            &inbox_cfg,
        )
        .into_iter()
        .map(|c| InboxCaptureDto {
            note_path: c.note_path,
            note_title: c.note_title,
            line_index: c.line_index,
            date_label: c.date_label,
            time_label: c.time_label,
            content: c.content,
            tags: c.tags,
        })
        .collect();

        let todos = inbox::collect_local_todos(
            pair_refs,
            &today_rel,
            &daily_folder,
            &templates_folder,
            &inbox_cfg,
        )
        .into_iter()
        .map(|t| InboxTodoDto {
            note_path: t.note_path,
            note_title: t.note_title,
            group: match t.group {
                TodoGroup::TodayDaily => "today".to_string(),
                TodoGroup::CoreFolder => "core".to_string(),
                TodoGroup::OtherNote => "other".to_string(),
            },
            line_index: t.line_index,
            text: t.text,
            done: t.done,
            tags: t.tags,
        })
        .collect();

        InboxSummaryDto {
            captures,
            todos,
            stream_path: stream_rel,
            daily_path: today_rel,
        }
    }

    /// Append a quick capture or todo into the local vault according to `[inbox]` settings
    pub fn inbox_append(&self, raw_text: &str, as_todo: bool) -> Result<String, String> {
        let trimmed = raw_text.trim();
        if trimmed.is_empty() {
            return Err("Capture text cannot be empty".to_string());
        }
        let mut vault = self.vault.lock().unwrap();
        let root = vault.root_dir.clone();
        let inbox_cfg = InboxSettings::load(&root);
        let today = Date::today();
        let hhmm = daily::local_hhmm();
        let effective_todo = as_todo
            || trimmed.starts_with("- [ ] ")
            || trimmed.starts_with("- [x] ")
            || trimmed.starts_with("- [X] ");

        match inbox_cfg.capture_mode {
            CaptureMode::StreamFile => {
                let rel = inbox_cfg.stream_rel_path(today);
                let existing = vault
                    .notes
                    .iter()
                    .find(|n| n.path.replace('\\', "/").eq_ignore_ascii_case(&rel))
                    .map(|n| n.content().to_string())
                    .or_else(|| std::fs::read_to_string(root.join(&rel)).ok())
                    .unwrap_or_default();
                let title = inbox_cfg.stream_title(today);
                let updated = inbox::append_to_stream_markdown(
                    &existing,
                    &title,
                    today,
                    &hhmm,
                    trimmed,
                    effective_todo,
                );
                upsert_and_save_note(&mut vault, &rel, &updated)?;
                Ok(rel)
            }
            CaptureMode::DailyNote => {
                let idx = vault
                    .open_or_create_daily(today)
                    .map_err(|e| format!("Failed to open daily note: {e}"))?;
                let rel = vault.notes[idx].path.replace('\\', "/");
                let old_content = vault.notes[idx].content().to_string();
                let updated =
                    inbox::append_to_daily_markdown(&old_content, &hhmm, trimmed, effective_todo);
                upsert_and_save_note(&mut vault, &rel, &updated)?;
                Ok(rel)
            }
            CaptureMode::CardNote => {
                let rel = allocate_inbox_card_path(&root, &inbox_cfg.folder, trimmed);
                let stem = Path::new(&rel)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("随手记");
                let body = if effective_todo
                    && !trimmed.starts_with("- [ ] ")
                    && !trimmed.starts_with("- [x] ")
                {
                    format!("# {stem}\n\n> {hhmm} · #随手记\n\n- [ ] {trimmed}\n")
                } else {
                    format!("# {stem}\n\n> {hhmm} · #随手记\n\n{trimmed}\n")
                };
                upsert_and_save_note(&mut vault, &rel, &body)?;
                Ok(rel)
            }
        }
    }

    /// Toggle a Markdown todo item `- [ ]` / `- [x]` inside `note_path` at `line_index`
    pub fn inbox_toggle_todo(&self, note_path: &str, line_index: usize) -> Result<bool, String> {
        let mut vault = self.vault.lock().unwrap();
        let norm = note_path.replace('\\', "/");
        let idx = vault
            .notes
            .iter()
            .position(|n| n.path.replace('\\', "/").eq_ignore_ascii_case(&norm))
            .ok_or_else(|| format!("Note not found: {note_path}"))?;
        let current = vault.notes[idx].content().to_string();
        let updated = inbox::toggle_todo_line(&current, line_index)
            .ok_or_else(|| format!("Todo line not found at {norm}:{line_index}"))?;
        let now_done = updated
            .lines()
            .nth(line_index)
            .and_then(inbox::parse_todo_line)
            .map(|(done, _)| done)
            .unwrap_or(false);
        vault.notes[idx].set_content(&updated);
        vault
            .save_note_to_disk(idx)
            .map_err(|e| format!("Failed to save todo toggle: {e}"))?;
        vault.bump_revision();
        vault.rebuild_graph();
        Ok(now_done)
    }

    /// Delete a list entry from `note_path` or remove a standalone card note when `line_index` is None
    pub fn inbox_delete_entry(
        &self,
        note_path: &str,
        line_index: Option<usize>,
    ) -> Result<(), String> {
        let mut vault = self.vault.lock().unwrap();
        let norm = note_path.replace('\\', "/");
        let idx = vault
            .notes
            .iter()
            .position(|n| n.path.replace('\\', "/").eq_ignore_ascii_case(&norm))
            .ok_or_else(|| format!("Note not found: {note_path}"))?;

        match line_index {
            Some(line) => {
                let current = vault.notes[idx].content().to_string();
                let updated = inbox::delete_list_entry(&current, line)
                    .ok_or_else(|| format!("Entry not found at {norm}:{line}"))?;
                vault.notes[idx].set_content(&updated);
                vault
                    .save_note_to_disk(idx)
                    .map_err(|e| format!("Failed to save note after delete: {e}"))?;
            }
            None => {
                let abs_path = vault.root_dir.join(&vault.notes[idx].path);
                let _ = std::fs::remove_file(abs_path);
                vault.notes.remove(idx);
            }
        }
        vault.bump_revision();
        vault.rebuild_graph();
        Ok(())
    }

    /// Import a shared/local file into the vault attachments folder and record it in the inbox stream
    pub fn inbox_import_file(
        &self,
        src_path: &str,
        note_text: &str,
    ) -> Result<ImportFileResultDto, String> {
        let src = Path::new(src_path.trim());
        if !src.is_file() {
            return Err(format!("Source file does not exist: {src_path}"));
        }
        let bytes = std::fs::read(src).map_err(|e| format!("Failed to read source file: {e}"))?;
        let original_name = src.file_name().and_then(|s| s.to_str());

        let mut vault = self.vault.lock().unwrap();
        let root = vault.root_dir.clone();
        let inbox_cfg = InboxSettings::load(&root);
        let attach_cfg = load_attach_settings(&root);
        let today = Date::today();
        let hhmm = daily::local_hhmm();
        let target_note_rel = match inbox_cfg.capture_mode {
            CaptureMode::DailyNote => today_daily_rel(&vault, today),
            _ => inbox_cfg.stream_rel_path(today),
        };
        let now_ts = current_timestamp(today, &hhmm);

        let plan = plan_import(
            &attach_cfg,
            &target_note_rel,
            original_name,
            &bytes,
            now_ts,
            &|rel| std::fs::read(root.join(rel)).ok(),
        );
        if !plan.reuse_existing {
            write_import(&root, &plan, &bytes)
                .map_err(|e| format!("Failed to write attachment: {e}"))?;
        }

        let extra = note_text.trim();
        let capture_line = if extra.is_empty() {
            plan.link_text.clone()
        } else {
            format!("{extra} {}", plan.link_text)
        };

        let saved_note_path = match inbox_cfg.capture_mode {
            CaptureMode::DailyNote => {
                let idx = vault
                    .open_or_create_daily(today)
                    .map_err(|e| format!("Failed to open daily note: {e}"))?;
                let rel = vault.notes[idx].path.replace('\\', "/");
                let old_content = vault.notes[idx].content().to_string();
                let updated =
                    inbox::append_to_daily_markdown(&old_content, &hhmm, &capture_line, false);
                upsert_and_save_note(&mut vault, &rel, &updated)?;
                rel
            }
            _ => {
                let rel = inbox_cfg.stream_rel_path(today);
                let existing = vault
                    .notes
                    .iter()
                    .find(|n| n.path.replace('\\', "/").eq_ignore_ascii_case(&rel))
                    .map(|n| n.content().to_string())
                    .or_else(|| std::fs::read_to_string(root.join(&rel)).ok())
                    .unwrap_or_default();
                let title = inbox_cfg.stream_title(today);
                let updated = inbox::append_to_stream_markdown(
                    &existing,
                    &title,
                    today,
                    &hhmm,
                    &capture_line,
                    false,
                );
                upsert_and_save_note(&mut vault, &rel, &updated)?;
                rel
            }
        };

        Ok(ImportFileResultDto {
            success: true,
            dest_rel: plan.dest_rel,
            link_text: plan.link_text,
            note_path: saved_note_path,
            message: "已导入附件并记录到随手记".to_string(),
        })
    }

    /// Reload vault state from disk (e.g. after background path-based sync)
    pub fn reload(&self) {
        let mut vault = self.vault.lock().unwrap();
        vault.load_from_disk();
    }

    /// Run S3 or WebDAV synchronization
    pub fn execute_sync(&self, config_json: &str, device: &str) -> SyncResultReport {
        let root_dir = {
            let vault = self.vault.lock().unwrap();
            vault.root_dir.clone()
        };
        let result = run_sync_at_root(&root_dir, config_json, device);
        self.reload();
        result
    }
}

// ---------------------------------------------------------------------------
// C-ABI Foreign Function Interface for Flutter / Dart FFI
// ---------------------------------------------------------------------------

fn c_str_to_str<'a>(ptr: *const c_char) -> &'a str {
    if ptr.is_null() {
        ""
    } else {
        unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("")
    }
}

fn to_c_string(s: impl AsRef<str>) -> *mut c_char {
    CString::new(s.as_ref()).unwrap_or_default().into_raw()
}

fn to_json_c_string<T: Serialize>(val: &T) -> *mut c_char {
    let json = serde_json::to_string(val).unwrap_or_else(|_| "{}".to_string());
    to_c_string(json)
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_open(root_path: *const c_char) -> *mut VaultBridge {
    let path_str = c_str_to_str(root_path);
    if path_str.is_empty() {
        return std::ptr::null_mut();
    }
    let bridge = Box::new(VaultBridge::open_or_create(Path::new(path_str)));
    Box::into_raw(bridge)
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_close(bridge: *mut VaultBridge) {
    if !bridge.is_null() {
        unsafe { drop(Box::from_raw(bridge)) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_list_notes(bridge: *mut VaultBridge) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("[]");
    }
    let bridge = unsafe { &*bridge };
    let summaries = bridge.list_notes();
    to_json_c_string(&summaries)
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_get_note(
    bridge: *mut VaultBridge,
    path: *const c_char,
) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("null");
    }
    let bridge = unsafe { &*bridge };
    let p = c_str_to_str(path);
    match bridge.get_note(p) {
        Some(detail) => to_json_c_string(&detail),
        None => to_c_string("null"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_save_note(
    bridge: *mut VaultBridge,
    path: *const c_char,
    content: *const c_char,
) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("Vault handle is null");
    }
    let bridge = unsafe { &*bridge };
    let p = c_str_to_str(path);
    let c = c_str_to_str(content);
    match bridge.save_note(p, c) {
        Ok(_) => std::ptr::null_mut(),
        Err(e) => to_c_string(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_create_note(
    bridge: *mut VaultBridge,
    title: *const c_char,
    content: *const c_char,
) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("null");
    }
    let bridge = unsafe { &*bridge };
    let t = c_str_to_str(title);
    let c = c_str_to_str(content);
    match bridge.create_note(t, c) {
        Ok(summary) => to_json_c_string(&summary),
        Err(_) => to_c_string("null"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_delete_note(bridge: *mut VaultBridge, path: *const c_char) -> bool {
    if bridge.is_null() {
        return false;
    }
    let bridge = unsafe { &*bridge };
    let p = c_str_to_str(path);
    bridge.delete_note(p).is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_rename_note(
    bridge: *mut VaultBridge,
    path: *const c_char,
    new_title: *const c_char,
) -> *mut c_char {
    if bridge.is_null() {
        return std::ptr::null_mut();
    }
    let bridge = unsafe { &*bridge };
    let p = c_str_to_str(path);
    let t = c_str_to_str(new_title);
    match bridge.rename_note(p, t) {
        Ok(new_path) => to_c_string(new_path),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_resolve_link(
    bridge: *mut VaultBridge,
    from_path: *const c_char,
    target: *const c_char,
) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("null");
    }
    let bridge = unsafe { &*bridge };
    let from = c_str_to_str(from_path);
    let tgt = c_str_to_str(target);
    match bridge.resolve_link_target(from, tgt) {
        Some(resolved) => to_json_c_string(&resolved),
        None => to_c_string("null"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_reload(bridge: *mut VaultBridge) {
    if !bridge.is_null() {
        let bridge = unsafe { &*bridge };
        bridge.reload();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_vault_search(
    bridge: *mut VaultBridge,
    query: *const c_char,
) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("[]");
    }
    let bridge = unsafe { &*bridge };
    let q = c_str_to_str(query);
    let results = bridge.search(q);
    to_json_c_string(&results)
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_daily_get_or_create(bridge: *mut VaultBridge) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("null");
    }
    let bridge = unsafe { &*bridge };
    match bridge.get_or_create_daily() {
        Ok(detail) => to_json_c_string(&detail),
        Err(_) => to_c_string("null"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_daily_append(bridge: *mut VaultBridge, text: *const c_char) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("Vault handle is null");
    }
    let bridge = unsafe { &*bridge };
    let t = c_str_to_str(text);
    match bridge.append_to_daily(t) {
        Ok(_) => std::ptr::null_mut(),
        Err(e) => to_c_string(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_inbox_summary(bridge: *mut VaultBridge) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string(
            r#"{"captures":[],"todos":[],"stream_path":"Inbox/随手记.md","daily_path":""}"#,
        );
    }
    let bridge = unsafe { &*bridge };
    let summary = bridge.inbox_summary();
    to_json_c_string(&summary)
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_inbox_append(
    bridge: *mut VaultBridge,
    text: *const c_char,
    as_todo: bool,
) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("Vault handle is null");
    }
    let bridge = unsafe { &*bridge };
    let t = c_str_to_str(text);
    match bridge.inbox_append(t, as_todo) {
        Ok(_) => std::ptr::null_mut(),
        Err(e) => to_c_string(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_inbox_toggle_todo(
    bridge: *mut VaultBridge,
    note_path: *const c_char,
    line_index: i32,
) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("Vault handle is null");
    }
    if line_index < 0 {
        return to_c_string("Invalid line index");
    }
    let bridge = unsafe { &*bridge };
    let path = c_str_to_str(note_path);
    match bridge.inbox_toggle_todo(path, line_index as usize) {
        Ok(_) => std::ptr::null_mut(),
        Err(e) => to_c_string(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_inbox_delete_entry(
    bridge: *mut VaultBridge,
    note_path: *const c_char,
    line_index: i32,
) -> *mut c_char {
    if bridge.is_null() {
        return to_c_string("Vault handle is null");
    }
    let bridge = unsafe { &*bridge };
    let path = c_str_to_str(note_path);
    let line = if line_index < 0 {
        None
    } else {
        Some(line_index as usize)
    };
    match bridge.inbox_delete_entry(path, line) {
        Ok(_) => std::ptr::null_mut(),
        Err(e) => to_c_string(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_inbox_import_file(
    bridge: *mut VaultBridge,
    src_path: *const c_char,
    note_text: *const c_char,
) -> *mut c_char {
    if bridge.is_null() {
        return to_json_c_string(&ImportFileResultDto {
            success: false,
            dest_rel: String::new(),
            link_text: String::new(),
            note_path: String::new(),
            message: "Vault handle is null".to_string(),
        });
    }
    let bridge = unsafe { &*bridge };
    let src = c_str_to_str(src_path);
    let extra = c_str_to_str(note_text);
    match bridge.inbox_import_file(src, extra) {
        Ok(res) => to_json_c_string(&res),
        Err(err) => to_json_c_string(&ImportFileResultDto {
            success: false,
            dest_rel: String::new(),
            link_text: String::new(),
            note_path: String::new(),
            message: err,
        }),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_sync_test(config_json: *const c_char) -> *mut c_char {
    let json_str = c_str_to_str(config_json);
    let config: StoreConfig = match serde_json::from_str(json_str) {
        Ok(c) => c,
        Err(e) => {
            return to_json_c_string(&ConnectionTestResult {
                success: false,
                reachable: false,
                can_list: false,
                can_write: false,
                message: format!("JSON parse error: {e}"),
            });
        }
    };

    let store = match OpendalStore::from_config(&config) {
        Ok(s) => s,
        Err(e) => {
            return to_json_c_string(&ConnectionTestResult {
                success: false,
                reachable: false,
                can_list: false,
                can_write: false,
                message: format!("Store init error: {e}"),
            });
        }
    };

    let report = store.check_connection();
    to_json_c_string(&ConnectionTestResult {
        success: report.reachable && report.can_list && report.can_write,
        reachable: report.reachable,
        can_list: report.can_list,
        can_write: report.can_write,
        message: report.message,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_sync_execute(
    bridge: *mut VaultBridge,
    config_json: *const c_char,
    device_name: *const c_char,
) -> *mut c_char {
    if bridge.is_null() {
        return to_json_c_string(&SyncResultReport::failure("Vault handle is null"));
    }
    let bridge = unsafe { &*bridge };
    let cfg = c_str_to_str(config_json);
    let dev = c_str_to_str(device_name);
    let result = bridge.execute_sync(cfg, dev);
    to_json_c_string(&result)
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_sync_run_path(
    vault_path: *const c_char,
    config_json: *const c_char,
    device_name: *const c_char,
) -> *mut c_char {
    let root = c_str_to_str(vault_path);
    if root.is_empty() {
        return to_json_c_string(&SyncResultReport::failure("Vault path is empty"));
    }
    let cfg = c_str_to_str(config_json);
    let dev = c_str_to_str(device_name);
    let result = run_sync_at_root(Path::new(root), cfg, dev);
    to_json_c_string(&result)
}

#[unsafe(no_mangle)]
pub extern "C" fn jiyue_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

// ---------------------------------------------------------------------------
// Unit tests verifying core bridge operations in pure Rust and C-ABI
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_vault_bridge_lifecycle_and_notes() {
        let dir = tempdir().unwrap();
        let bridge = VaultBridge::open_or_create(dir.path());

        // 1. Initial vault should start clean with no sample notes
        let initial_notes = bridge.list_notes();
        assert!(
            initial_notes.is_empty(),
            "Initial vault should have no sample notes"
        );

        // 2. Create a new note
        let created = bridge
            .create_note(
                "移动端设计手记",
                "# 移动端手记\n\n测试正文内容，包含 [[欢迎使用 Jiyue]] 双链。\n",
            )
            .expect("create note");
        assert_eq!(created.title, "移动端设计手记");

        // 3. Retrieve note detail and verify wikilink extraction
        let detail = bridge.get_note(&created.path).expect("get note detail");
        assert_eq!(detail.title, "移动端设计手记");
        assert!(detail.wikilinks.contains(&"欢迎使用 Jiyue".to_string()));

        // 4. Update note
        bridge
            .save_note(&created.path, "# 移动端手记 (已修改)\n\n更新后的内容。")
            .expect("save note");
        let updated = bridge.get_note(&created.path).expect("get updated note");
        assert!(updated.content.contains("已修改"));

        // 5. Search notes
        let search_res = bridge.search("手记");
        assert!(!search_res.is_empty(), "Should match note with '手记'");

        // 6. Daily note
        let daily = bridge.get_or_create_daily().expect("daily note");
        assert!(!daily.path.is_empty());
        bridge
            .append_to_daily("完成移动端 FFI 核心测试")
            .expect("append daily");
        let daily_after = bridge.get_note(&daily.path).expect("daily after append");
        assert!(daily_after.content.contains("完成移动端 FFI 核心测试"));

        // 7. Delete note
        bridge.delete_note(&created.path).expect("delete note");
        assert!(bridge.get_note(&created.path).is_none());
    }

    #[test]
    fn test_c_abi_functions() {
        let dir = tempdir().unwrap();
        let path_c = CString::new(dir.path().to_str().unwrap()).unwrap();

        let handle = jiyue_vault_open(path_c.as_ptr());
        assert!(!handle.is_null());

        // List notes
        let list_json_ptr = jiyue_vault_list_notes(handle);
        assert!(!list_json_ptr.is_null());
        let list_str = c_str_to_str(list_json_ptr);
        assert!(list_str.starts_with('['));
        jiyue_string_free(list_json_ptr);

        // Close vault
        jiyue_vault_close(handle);
    }

    #[test]
    fn test_backlinks_link_resolution_and_subdir_rename() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("backend")).unwrap();
        std::fs::create_dir_all(dir.path().join("_indexes")).unwrap();

        std::fs::write(
            dir.path().join("README.md"),
            "# Root\n\nSee [[backend]] and [[Target Note|alias]] and [MD](backend/Rust.md).\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("backend/README.md"),
            "# Backend Index\n\nIntro to backend.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("backend/Rust.md"),
            "# Rust\n\nBack to [[README]] and [[Target Note]].\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Target Note.md"),
            "# Target Note\n\nTarget body.\n",
        )
        .unwrap();

        let bridge = VaultBridge::open_or_create(dir.path());

        // Resolve folder index & relative links
        assert_eq!(
            bridge.resolve_link_target("README.md", "backend"),
            Some("backend/README.md".to_string())
        );
        assert_eq!(
            bridge.resolve_link_target("README.md", "Target Note|alias"),
            Some("Target Note.md".to_string())
        );
        assert_eq!(
            bridge.resolve_link_target("README.md", "backend/Rust.md"),
            Some("backend/Rust.md".to_string())
        );
        assert_eq!(
            bridge.resolve_link_target("backend/Rust.md", "../README.md"),
            Some("README.md".to_string())
        );

        // Verify backlinks extracted for Target Note.md and backend/Rust.md
        let target_detail = bridge.get_note("Target Note.md").expect("target detail");
        assert_eq!(target_detail.backlinks.len(), 2);
        let backlink_paths: Vec<&str> = target_detail
            .backlinks
            .iter()
            .map(|b| b.path.as_str())
            .collect();
        assert!(backlink_paths.contains(&"README.md"));
        assert!(backlink_paths.contains(&"backend/Rust.md"));

        let rust_detail = bridge.get_note("backend/Rust.md").expect("rust detail");
        assert!(
            rust_detail.backlinks.iter().any(|b| b.path == "README.md"),
            "Markdown link from README.md should count as backlink"
        );

        // Rename inside subdirectory preserves parent directory
        let renamed = bridge
            .rename_note("backend/Rust.md", "Rust Async")
            .expect("rename note");
        assert_eq!(renamed, "backend/Rust Async.md");
        assert!(dir.path().join("backend/Rust Async.md").exists());
        assert!(!dir.path().join("backend/Rust.md").exists());

        // C-ABI resolve_link check
        let path_c = CString::new(dir.path().to_str().unwrap()).unwrap();
        let handle = jiyue_vault_open(path_c.as_ptr());
        let from_c = CString::new("README.md").unwrap();
        let tgt_c = CString::new("backend").unwrap();
        let res_ptr = jiyue_vault_resolve_link(handle, from_c.as_ptr(), tgt_c.as_ptr());
        let res_str = c_str_to_str(res_ptr).to_string();
        jiyue_string_free(res_ptr);
        assert_eq!(res_str, "\"backend/README.md\"");
        jiyue_vault_close(handle);
    }

    #[test]
    fn test_inbox_captures_todos_and_file_import() {
        let dir = tempdir().unwrap();
        let bridge = VaultBridge::open_or_create(dir.path());

        // 1. Append fleeting capture and todo into Inbox stream
        let note_rel = bridge
            .inbox_append("移动端随手记想法 #灵感", false)
            .expect("append capture");
        assert_eq!(note_rel, "Inbox/随手记.md");

        bridge
            .inbox_append("整理随手记同步方案", true)
            .expect("append todo");

        let summary = bridge.inbox_summary();
        assert_eq!(summary.captures.len(), 2);
        assert_eq!(summary.todos.len(), 1);
        assert!(!summary.todos[0].done);
        assert_eq!(summary.todos[0].text, "整理随手记同步方案");

        // 2. Toggle todo line
        let line_idx = summary.todos[0].line_index;
        let now_done = bridge
            .inbox_toggle_todo(&summary.todos[0].note_path, line_idx)
            .expect("toggle todo");
        assert!(now_done);
        let after_toggle = bridge.inbox_summary();
        assert!(after_toggle.todos[0].done);

        // 3. Delete capture entry
        let cap = after_toggle
            .captures
            .iter()
            .find(|c| c.content.contains("移动端随手记想法"))
            .expect("capture found");
        bridge
            .inbox_delete_entry(&cap.note_path, cap.line_index)
            .expect("delete capture");
        let after_del = bridge.inbox_summary();
        assert!(
            !after_del
                .captures
                .iter()
                .any(|c| c.content.contains("移动端随手记想法"))
        );

        // 4. Import a shared file into attachments + inbox
        let src_dir = tempdir().unwrap();
        let src_file = src_dir.path().join("demo.png");
        std::fs::write(&src_file, b"\x89PNG\r\n\x1a\ndemo").unwrap();
        let import_res = bridge
            .inbox_import_file(src_file.to_str().unwrap(), "分享截图")
            .expect("import file");
        assert!(import_res.success);
        assert!(dir.path().join(&import_res.dest_rel).is_file());
        assert!(import_res.link_text.contains("demo"));

        // 5. C-ABI inbox summary & append
        let path_c = CString::new(dir.path().to_str().unwrap()).unwrap();
        let handle = jiyue_vault_open(path_c.as_ptr());
        let txt_c = CString::new("C ABI 随手记").unwrap();
        let err_ptr = jiyue_inbox_append(handle, txt_c.as_ptr(), false);
        assert!(err_ptr.is_null());

        let sum_ptr = jiyue_inbox_summary(handle);
        let sum_json = c_str_to_str(sum_ptr).to_string();
        jiyue_string_free(sum_ptr);
        assert!(sum_json.contains("C ABI 随手记"));
        jiyue_vault_close(handle);
    }
}
