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

use jiyue_core::buffer::{Note, ZedBuffer, wikilink_targets};
use jiyue_core::daily::Date;
use jiyue_core::vault::Vault;
use jiyue_sync::store::{OpendalStore, StoreConfig};
use jiyue_sync::{SyncOptions, run_sync, state};

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

/// Connection test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTestResult {
    pub success: bool,
    pub reachable: bool,
    pub can_list: bool,
    pub can_write: bool,
    pub message: String,
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
                path: note.path.clone(),
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
        let note = vault
            .notes
            .iter()
            .find(|n| n.path.replace('\\', "/") == norm)?;

        let abs_path = vault.root_dir.join(&note.path);
        let modified_ms = std::fs::metadata(&abs_path)
            .and_then(|m| m.modified())
            .and_then(|t| {
                t.duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .map_err(|e| std::io::Error::other(e.to_string()))
            })
            .unwrap_or(0);

        let content = note.content().to_string();
        let wikilinks = wikilink_targets(&content);

        Some(NoteDetail {
            id: note.id.clone(),
            title: note.title.clone(),
            path: note.path.clone(),
            content,
            tags: note.tags.clone(),
            wikilinks,
            modified_ms,
        })
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

        vault.notes[idx].buffer = ZedBuffer::new(new_content);
        vault
            .save_note_to_disk(idx)
            .map_err(|e| format!("Failed to save note: {e}"))?;
        vault.rebuild_graph();
        Ok(())
    }

    /// Create a new note
    pub fn create_note(&self, title: &str, content: &str) -> Result<NoteSummary, String> {
        let mut vault = self.vault.lock().unwrap();
        let safe_title = jiyue_core::vault::safe_note_title(title)
            .unwrap_or_else(|| format!("未命名 {}", vault.notes.len() + 1));

        let filename = format!("{}.md", safe_title);
        let mut note = Note::new(&safe_title, &filename, content);
        note.buffer = ZedBuffer::new(content);
        vault.notes.push(note);
        let new_idx = vault.notes.len() - 1;
        vault
            .save_note_to_disk(new_idx)
            .map_err(|e| format!("Failed to persist new note: {e}"))?;
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
        vault.rebuild_graph();
        Ok(())
    }

    /// Rename note
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

        let old_abs = vault.root_dir.join(&vault.notes[idx].path);
        let new_rel = format!("{}.md", safe_title);
        let new_abs = vault.root_dir.join(&new_rel);

        std::fs::rename(&old_abs, &new_abs)
            .map_err(|e| format!("Failed to rename file on disk: {e}"))?;

        vault.notes[idx].title = safe_title;
        vault.notes[idx].path = new_rel.clone();
        vault.rebuild_graph();
        Ok(new_rel)
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
                    path: note.path.clone(),
                    title: note.title.clone(),
                    snippet: format!("标题匹配: {}", note.title),
                    match_type: "title".to_string(),
                });
                continue;
            }

            if note.tags.iter().any(|t| t.to_lowercase().contains(&q)) {
                results.push(SearchResultItem {
                    path: note.path.clone(),
                    title: note.title.clone(),
                    snippet: format!("标签匹配: {}", note.tags.join(" ")),
                    match_type: "tag".to_string(),
                });
                continue;
            }

            let content = note.content();
            if let Some(pos) = content.to_lowercase().find(&q) {
                let start = pos.saturating_sub(30);
                let end = (pos + q.len() + 30).min(content.len());
                let snippet = format!("...{}...", content[start..end].replace('\n', " "));
                results.push(SearchResultItem {
                    path: note.path.clone(),
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

        let note = &vault.notes[idx];
        let content = note.content().to_string();
        let wikilinks = wikilink_targets(&content);

        Ok(NoteDetail {
            id: note.id.clone(),
            title: note.title.clone(),
            path: note.path.clone(),
            content,
            tags: note.tags.clone(),
            wikilinks,
            modified_ms: std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        })
    }

    /// Append a quick log entry to today's daily note
    pub fn append_to_daily(&self, entry: &str) -> Result<(), String> {
        let mut vault = self.vault.lock().unwrap();
        let today = Date::today();
        let idx = vault
            .open_or_create_daily(today)
            .map_err(|e| format!("Failed to open daily note: {e}"))?;

        let old_content = vault.notes[idx].content().to_string();
        let now_hhmm = jiyue_core::daily::local_hhmm();
        let new_line = format!("\n- [{}] {}\n", now_hhmm, entry.trim());
        let updated = format!("{}{}", old_content.trim_end(), new_line);

        vault.notes[idx].buffer = ZedBuffer::new(&updated);
        vault
            .save_note_to_disk(idx)
            .map_err(|e| format!("Failed to save daily note: {e}"))?;
        vault.rebuild_graph();
        Ok(())
    }

    /// Run S3 or WebDAV synchronization
    pub fn execute_sync(&self, config_json: &str, device: &str) -> SyncResultReport {
        let config: StoreConfig = match serde_json::from_str(config_json) {
            Ok(c) => c,
            Err(e) => {
                return SyncResultReport {
                    success: false,
                    uploaded: 0,
                    downloaded: 0,
                    deleted_remote: 0,
                    deleted_local: 0,
                    conflicts: 0,
                    message: format!("Invalid sync configuration JSON: {e}"),
                };
            }
        };

        let store = match OpendalStore::from_config(&config) {
            Ok(s) => s,
            Err(e) => {
                return SyncResultReport {
                    success: false,
                    uploaded: 0,
                    downloaded: 0,
                    deleted_remote: 0,
                    deleted_local: 0,
                    conflicts: 0,
                    message: format!("Failed to build remote store: {e}"),
                };
            }
        };

        let root_dir = {
            let vault = self.vault.lock().unwrap();
            vault.root_dir.clone()
        };

        let mut sync_state = match state::load_state(&root_dir) {
            Ok(s) => s,
            Err(e) => {
                return SyncResultReport {
                    success: false,
                    uploaded: 0,
                    downloaded: 0,
                    deleted_remote: 0,
                    deleted_local: 0,
                    conflicts: 0,
                    message: format!("Failed to load local sync state: {e}"),
                };
            }
        };

        let mut opts = SyncOptions::new(if device.is_empty() { "Mobile" } else { device });
        let report = run_sync(&root_dir, &store, &mut sync_state, &mut opts);

        // Reload vault from disk to reflect downloaded or merged notes
        {
            let mut vault = self.vault.lock().unwrap();
            vault.load_from_disk();
        }

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
        return to_json_c_string(&SyncResultReport {
            success: false,
            uploaded: 0,
            downloaded: 0,
            deleted_remote: 0,
            deleted_local: 0,
            conflicts: 0,
            message: "Vault handle is null".to_string(),
        });
    }
    let bridge = unsafe { &*bridge };
    let cfg = c_str_to_str(config_json);
    let dev = c_str_to_str(device_name);
    let result = bridge.execute_sync(cfg, dev);
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
}
