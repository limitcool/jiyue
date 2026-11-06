//! `credentials.toml`: API keys and sync secrets, in plain text, in the user's config folder.
//!
//! The file is created owner-only: on Unix with mode 0600, on Windows with an `icacls` call that
//! removes inherited access and grants only the current user. Both are best effort: when they
//! fail the caller shows a warning. A file that is readable by other users is reported at load.
//!
//! Writes go through `toml_edit` so that the comments in the file survive. A field is removed when
//! its value is cleared, and the table that holds it is removed when it becomes empty.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::doc::{Loaded, edit_base, load_file};
use super::templates;
use super::{CREDENTIALS_FILE, user_file};

/// Table of AI provider keys, keyed by provider id.
pub const AI_TABLE: &str = "ai";
/// Table of sync secrets, keyed by the vault id.
pub const SYNC_TABLE: &str = "sync";

/// One AI provider's secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AiCredential {
    pub api_key: String,
}

/// One vault's sync secrets. Only the fields the vault's backend uses are filled.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SyncCredential {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub password: String,
    pub username: String,
}

// Debug never prints a stored value: a credential in a log line would leak it.
impl fmt::Debug for AiCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiCredential")
            .field("api_key", &"***")
            .finish()
    }
}

impl fmt::Debug for SyncCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncCredential")
            .field("access_key_id", &"***")
            .field("secret_access_key", &"***")
            .field("password", &"***")
            .field("username", &"***")
            .finish()
    }
}

/// The whole credentials file.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CredentialsFile {
    pub ai: BTreeMap<String, AiCredential>,
    pub sync: BTreeMap<String, SyncCredential>,
}

impl fmt::Debug for CredentialsFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialsFile")
            .field("ai", &self.ai.keys().collect::<Vec<_>>())
            .field("sync", &self.sync.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Path of the credentials file in the user folder.
pub fn path() -> PathBuf {
    user_file(CREDENTIALS_FILE)
}

/// Loads the credentials file. A missing file is empty; a broken one is an error the caller
/// reports (and then never overwrites).
pub fn load() -> Loaded<CredentialsFile> {
    load_from(&path())
}

/// Loads a credentials file from `path` (see [`load`]).
pub fn load_from(path: &Path) -> Loaded<CredentialsFile> {
    let mut loaded: Loaded<CredentialsFile> = load_file(path);
    if !loaded.missing && is_open_to_others(path) {
        loaded.warnings.push(format!(
            "凭据文件 {} 对其他用户可读，请只保留当前用户的访问权限",
            path.display()
        ));
    }
    loaded
}

/// Writes one field of a credential. `value` of `None` or an empty string removes the field and
/// the tables that become empty. Refuses when the file is broken.
pub fn write_field(
    path: &Path,
    table: &str,
    id: &str,
    field: &str,
    value: Option<&str>,
) -> Result<WriteOutcome, String> {
    let check: Loaded<CredentialsFile> = load_from(path);
    if let Some(issue) = check.error {
        return Err(format!("凭据文件有错误，未保存：{issue}"));
    }
    if !is_safe_id(id) || !is_safe_id(field) {
        return Err("凭据编号不合法".to_string());
    }
    let mut doc = edit_base(path, templates::CREDENTIALS)?;
    let value = value.map(str::trim).filter(|v| !v.is_empty());
    match value {
        Some(v) => {
            let entry = ensure_table(doc.as_table_mut(), table);
            let fields = ensure_table(entry, id);
            fields.insert(field, toml_edit::value(v));
        }
        None => {
            if let Some(entry) = doc.get_mut(table).and_then(|t| t.as_table_mut()) {
                if let Some(fields) = entry.get_mut(id).and_then(|t| t.as_table_mut()) {
                    fields.remove(field);
                    if fields.is_empty() {
                        entry.remove(id);
                    }
                }
                if entry.is_empty() {
                    doc.remove(table);
                }
            }
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::fsops::write_atomic(path, doc.to_string().as_bytes()).map_err(|e| e.to_string())?;
    // The atomic write replaces the file, so the owner-only mode is applied again every time.
    let permissions_ok = restrict_to_owner(path).is_ok();
    Ok(WriteOutcome { permissions_ok })
}

/// The table `key` under `parent`, created as an implicit table (no header of its own) when missing.
fn ensure_table<'a>(parent: &'a mut toml_edit::Table, key: &str) -> &'a mut toml_edit::Table {
    if !parent.get(key).is_some_and(|item| item.is_table()) {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        parent.insert(key, toml_edit::Item::Table(table));
    }
    parent
        .get_mut(key)
        .and_then(|item| item.as_table_mut())
        .expect("table was just inserted")
}

/// Result of a credential write: whether the owner-only permissions are in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOutcome {
    pub permissions_ok: bool,
}

/// Creates the file with its template if it does not exist, owner-only.
pub fn ensure_file(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::fsops::write_atomic(path, templates::CREDENTIALS.as_bytes())
        .map_err(|e| e.to_string())?;
    restrict_to_owner(path)
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Makes `path` readable by the current user only. Unix: mode 0600. Windows: `icacls`, which
/// removes inherited access and grants full control to `USERNAME`.
pub fn restrict_to_owner(path: &Path) -> Result<(), String> {
    restrict_impl(path)
}

#[cfg(unix)]
fn restrict_impl(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn restrict_impl(path: &Path) -> Result<(), String> {
    let user = std::env::var("USERNAME").map_err(|_| "USERNAME 未设置".to_string())?;
    let status = std::process::Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{user}:F"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| e.kind().to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("icacls 未能设置权限".to_string())
    }
}

#[cfg(not(any(unix, windows)))]
fn restrict_impl(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// True when other users may read the file. Only Unix can tell; elsewhere it is `false`.
#[cfg(unix)]
pub fn is_open_to_others(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o077 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn is_open_to_others(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jiyue-cred-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir.join(CREDENTIALS_FILE)
    }

    #[test]
    fn debug_output_redacts_every_secret() {
        let cred = AiCredential {
            api_key: "sk-super-secret".into(),
        };
        let sync = SyncCredential {
            secret_access_key: "hunter2".into(),
            ..SyncCredential::default()
        };
        let file = CredentialsFile {
            ai: [("openai".to_string(), cred.clone())].into_iter().collect(),
            sync: [("v1".to_string(), sync.clone())].into_iter().collect(),
        };
        for text in [
            format!("{cred:?}"),
            format!("{sync:?}"),
            format!("{file:?}"),
        ] {
            assert!(!text.contains("sk-super"), "{text}");
            assert!(!text.contains("hunter2"), "{text}");
        }
    }

    #[test]
    fn write_then_read_then_clear_removes_empty_tables() {
        let path = temp_file("write");
        write_field(&path, AI_TABLE, "openai", "api_key", Some("sk-one")).expect("write");
        let loaded = load_from(&path);
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.value.ai["openai"].api_key, "sk-one");

        write_field(&path, AI_TABLE, "openai", "api_key", None).expect("clear");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(!text.contains("api_key ="), "{text}");
        assert!(!text.contains("[ai.openai]"), "{text}");
        assert!(
            text.contains("凭据以明文保存"),
            "template comments stay: {text}"
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn writes_keep_comments_the_user_wrote() {
        let path = temp_file("comments");
        std::fs::write(
            &path,
            "version = 1\n# 我的备注：这是测试\n[ai.openai]\n# 旧密钥\napi_key = \"env:OPENAI_API_KEY\"\n",
        )
        .expect("write");
        write_field(&path, AI_TABLE, "openai", "api_key", Some("env:OTHER")).expect("update");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("# 我的备注：这是测试"), "{text}");
        assert!(text.contains("env:OTHER"), "{text}");
        write_field(&path, SYNC_TABLE, "v1", "password", Some("pw")).expect("second table");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("# 我的备注：这是测试"), "{text}");
        assert!(text.contains("[sync.v1]"), "{text}");
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn a_broken_file_is_never_overwritten() {
        let path = temp_file("broken");
        let original = "version = 1\n[ai.openai]\napi_key = \n";
        std::fs::write(&path, original).expect("write");
        let err =
            write_field(&path, AI_TABLE, "openai", "api_key", Some("sk")).expect_err("refused");
        assert!(err.contains("未保存"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), original);
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn invalid_ids_are_refused() {
        let path = temp_file("ids");
        assert!(write_field(&path, AI_TABLE, "a b", "api_key", Some("x")).is_err());
        assert!(write_field(&path, AI_TABLE, "", "api_key", Some("x")).is_err());
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[cfg(unix)]
    #[test]
    fn created_file_is_owner_only_and_open_files_are_reported() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_file("perm");
        write_field(&path, AI_TABLE, "openai", "api_key", Some("sk")).expect("write");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(is_open_to_others(&path));
        let loaded = load_from(&path);
        assert!(loaded.warnings.iter().any(|w| w.contains("其他用户")));
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }
}
