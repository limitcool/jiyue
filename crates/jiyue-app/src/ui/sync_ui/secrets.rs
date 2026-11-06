//! Sync credentials, kept in `credentials.toml` in the user's config folder, under
//! `[sync.<vault-id>]`: the secret (S3 secret key or WebDAV password) and the identity (S3 access
//! key id or WebDAV user name). Plain text, owner-only, never inside the vault, never synced.
//! A stored secret may be a literal, `env:NAME` or `cmd:COMMAND` (see `jiyue_core::config::secret`).
//!
//! The identity fields are not part of the vault's `[sync]` section: `SyncConfig` skips them on
//! save and `fill_identity` fills them from here on load.

use std::path::Path;

use jiyue_core::config::credentials::{self, SYNC_TABLE, SyncCredential};
use jiyue_core::config::secret::{SecretSource, resolve_secret, source_of};

use super::config::{Backend, SyncConfig};
use crate::ui::config_ui::{SecretDisplay, display_of};

/// Text shown in the secret field when a literal secret is already stored.
pub const STORED_PLACEHOLDER: &str = "已保存在 credentials.toml 中";

/// The credentials field that holds this backend's secret.
fn field(cfg: &SyncConfig) -> &'static str {
    match cfg.backend {
        Backend::S3 => "secret_access_key",
        Backend::WebDav => "password",
    }
}

fn entry(path: &Path, vault_id: &str) -> Option<SyncCredential> {
    let loaded = credentials::load_from(path);
    loaded.value.sync.get(vault_id).cloned()
}

/// The stored secret as written in the file (may be `env:` or `cmd:`). `None` when nothing is stored.
pub fn stored_at(path: &Path, cfg: &SyncConfig) -> Option<String> {
    let entry = entry(path, &cfg.vault_id)?;
    let value = match cfg.backend {
        Backend::S3 => entry.secret_access_key,
        Backend::WebDav => entry.password,
    };
    (!value.trim().is_empty()).then_some(value)
}

/// The stored secret as written in the file, for the user's credentials file.
pub fn stored(cfg: &SyncConfig) -> Option<String> {
    stored_at(&credentials::path(), cfg)
}

/// True when a secret is stored for this vault. Does not return the value.
pub fn exists(cfg: &SyncConfig) -> bool {
    stored(cfg).is_some()
}

/// How the stored secret is given: a literal (masked), an environment variable or a command.
pub fn source(cfg: &SyncConfig) -> Option<SecretSource> {
    stored(cfg).map(|raw| source_of(&raw))
}

/// What the settings page shows for the stored secret (never the value).
pub fn display(cfg: &SyncConfig) -> SecretDisplay {
    display_of(stored(cfg).as_deref())
}

/// The secret to use now. `env:` and `cmd:` forms are resolved here, at use time. Errors name the
/// problem, never the value.
pub fn load(cfg: &SyncConfig) -> Result<Option<String>, String> {
    match stored(cfg) {
        None => Ok(None),
        Some(raw) => resolve_secret(&raw)
            .map(|s| Some(s.expose().to_string()))
            .map_err(|e| format!("同步密钥无法读取：{e}")),
    }
}

/// Fills the identity fields (S3 access key id, WebDAV user name) from `credentials.toml`.
pub fn fill_identity_at(path: &Path, cfg: &mut SyncConfig) {
    if let Some(entry) = entry(path, &cfg.vault_id) {
        cfg.s3_access_key_id = entry.access_key_id;
        cfg.webdav_username = entry.username;
    }
}

/// Fills the identity fields from the user's credentials file.
pub fn fill_identity(cfg: &mut SyncConfig) {
    fill_identity_at(&credentials::path(), cfg);
}

/// Stores the secret (a literal, `env:NAME` or `cmd:...`) for this vault. Returns whether the
/// owner-only permissions could be applied (see `credentials::WriteOutcome`).
pub fn save(cfg: &SyncConfig, secret: &str) -> Result<bool, String> {
    write_at(&credentials::path(), cfg, Some(secret))
}

/// Removes this vault's sync secret.
pub fn delete(cfg: &SyncConfig) -> Result<bool, String> {
    write_at(&credentials::path(), cfg, None)
}

/// Writes the identity fields of `cfg` (S3 access key id and WebDAV user name) to `credentials.toml`.
/// Empty values remove the entry.
pub fn save_identity_at(path: &Path, cfg: &SyncConfig) -> Result<bool, String> {
    if cfg.vault_id.is_empty() {
        return Err("笔记库编号为空，无法保存同步凭据".to_string());
    }
    let access = credentials::write_field(
        path,
        SYNC_TABLE,
        &cfg.vault_id,
        "access_key_id",
        non_empty(&cfg.s3_access_key_id),
    )?;
    let user = credentials::write_field(
        path,
        SYNC_TABLE,
        &cfg.vault_id,
        "username",
        non_empty(&cfg.webdav_username),
    )?;
    Ok(access.permissions_ok && user.permissions_ok)
}

/// `save_identity_at` for the user's credentials file.
pub fn save_identity(cfg: &SyncConfig) -> Result<bool, String> {
    save_identity_at(&credentials::path(), cfg)
}

fn non_empty(value: &str) -> Option<&str> {
    let v = value.trim();
    (!v.is_empty()).then_some(v)
}

fn write_at(path: &Path, cfg: &SyncConfig, value: Option<&str>) -> Result<bool, String> {
    if cfg.vault_id.is_empty() {
        return Err("笔记库编号为空，无法保存同步密钥".to_string());
    }
    credentials::write_field(path, SYNC_TABLE, &cfg.vault_id, field(cfg), value)
        .map(|outcome| outcome.permissions_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jiyue-sync-cred-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir.join("credentials.toml")
    }

    fn cfg(backend: Backend) -> SyncConfig {
        SyncConfig {
            vault_id: "v1".into(),
            backend,
            s3_access_key_id: "AKIDEXAMPLE".into(),
            webdav_username: "alice".into(),
            ..SyncConfig::default()
        }
    }

    #[test]
    fn the_secret_field_follows_the_backend() {
        assert_eq!(field(&cfg(Backend::S3)), "secret_access_key");
        assert_eq!(field(&cfg(Backend::WebDav)), "password");
    }

    #[test]
    fn saving_without_a_vault_id_is_refused() {
        let c = SyncConfig::default();
        assert!(save(&c, "x").is_err());
        assert!(save_identity(&c).is_err());
    }

    #[test]
    fn identity_is_written_to_credentials_and_read_back() {
        let path = scratch("identity");
        let c = cfg(Backend::S3);
        save_identity_at(&path, &c).expect("write identity");
        let mut back = SyncConfig {
            vault_id: "v1".into(),
            ..SyncConfig::default()
        };
        fill_identity_at(&path, &mut back);
        assert_eq!(back.s3_access_key_id, "AKIDEXAMPLE");
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn secret_is_stored_as_given_and_reported_by_its_form() {
        let path = scratch("form");
        let c = cfg(Backend::S3);
        write_at(&path, &c, Some("env:AWS_SECRET")).expect("write env form");
        assert_eq!(stored_at(&path, &c).as_deref(), Some("env:AWS_SECRET"));
        assert_eq!(
            display_of(stored_at(&path, &c).as_deref()),
            SecretDisplay::Env("AWS_SECRET".into())
        );
        write_at(&path, &c, None).expect("clear");
        assert_eq!(stored_at(&path, &c), None);
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }
}
