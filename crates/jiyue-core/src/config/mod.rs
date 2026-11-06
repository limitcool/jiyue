//! The TOML configuration of Jiyue.
//!
//! User level (per OS config folder, or `JIYUE_CONFIG_DIR`):
//! - `config.toml`: global settings, keyboard shortcut overrides in `[keys]`.
//! - `state.toml`: machine state, the recent vaults and the last opened vault.
//! - `credentials.toml`: secrets, plain text, owner-only. Never inside a vault.
//!
//! Vault level (`<vault>/.jiyue/`):
//! - `config.toml`: vault settings in sections (`[editor]`, `[daily]`, `[templates]`,
//!   `[attachments]`, `[sync]`, `[ai]`).
//! - `state.toml`: vault UI state (tabs, panes, search history, collapsed sections).
//! - `bookmarks.toml`: bookmarks.
//!
//! Every file has a top-level `version = 1`. The typed structs use `#[serde(default)]`, so a key
//! that is missing takes its default. The generic loader and saver live in [`doc`].

pub mod credentials;
pub mod doc;
pub mod secret;
pub mod templates;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use doc::{ConfigIssue, Loaded, load_file, load_file_with, save_file};

/// Environment variable that moves the user-level folder (used by tests and dev drivers).
pub const CONFIG_DIR_ENV: &str = "JIYUE_CONFIG_DIR";
/// File names at user level.
pub const USER_CONFIG_FILE: &str = "config.toml";
pub const USER_STATE_FILE: &str = "state.toml";
pub const CREDENTIALS_FILE: &str = "credentials.toml";
/// File names at vault level, inside `.jiyue`.
pub const VAULT_DIR: &str = ".jiyue";
pub const VAULT_CONFIG_FILE: &str = "config.toml";
pub const VAULT_STATE_FILE: &str = "state.toml";
pub const BOOKMARKS_FILE: &str = "bookmarks.toml";

/// The user-level folder for a given override (`JIYUE_CONFIG_DIR`) and default. Pure, so the rule
/// can be tested without touching the environment.
pub fn user_dir_from(override_dir: Option<PathBuf>, default_dir: PathBuf) -> PathBuf {
    match override_dir {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => default_dir,
    }
}

/// The user-level folder of this process.
pub fn user_dir() -> PathBuf {
    let override_dir = std::env::var_os(CONFIG_DIR_ENV).map(PathBuf::from);
    user_dir_from(override_dir, crate::platform::config_dir())
}

/// Path of a user-level file.
pub fn user_file(name: &str) -> PathBuf {
    user_dir().join(name)
}

/// Path of a vault-level file (`<vault>/.jiyue/<name>`).
pub fn vault_file(vault_root: &Path, name: &str) -> PathBuf {
    vault_root.join(VAULT_DIR).join(name)
}

/// Refuses a user-level folder that sits inside an opened vault: credentials must never be
/// stored in a folder that may be synced or committed.
pub fn check_user_dir_outside_vaults(
    user_dir: &Path,
    vault_roots: &[PathBuf],
) -> Result<(), String> {
    let dir = absolute(user_dir);
    for root in vault_roots {
        let root = absolute(root);
        if dir.starts_with(&root) {
            return Err(format!(
                "配置文件夹 {} 位于笔记库 {} 之内；凭据不能放在笔记库里",
                dir.display(),
                root.display()
            ));
        }
    }
    Ok(())
}

fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|d| d.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

/// Writes `template` to `path` when the file does not exist: a new default file with its comments.
/// An existing file is left alone.
pub fn ensure_template(path: &Path, template: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::fsops::write_atomic(path, template.as_bytes()).map_err(|e| e.to_string())
}

/// Puts a default file in place of a broken one: the broken file is renamed to
/// `<name>.bad-<unix seconds>` first, so nothing is lost. Returns the renamed file, if there was one.
pub fn reset_to_default(path: &Path, template: &str) -> Result<Option<PathBuf>, String> {
    let moved = if path.exists() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config.toml");
        let bad = path.with_file_name(format!("{name}.bad-{stamp}"));
        std::fs::rename(path, &bad).map_err(|e| e.to_string())?;
        Some(bad)
    } else {
        None
    };
    crate::fsops::write_atomic(path, template.as_bytes()).map_err(|e| e.to_string())?;
    Ok(moved)
}

/// Reads one section of a file. Other sections are ignored (their own loader reports them).
pub fn load_section<T>(path: &Path, section: &str) -> Loaded<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    let whole: Loaded<toml::Table> = load_file(path);
    if whole.missing {
        return Loaded::missing();
    }
    if let Some(issue) = whole.error {
        return Loaded {
            value: T::default(),
            warnings: Vec::new(),
            error: Some(issue),
            missing: false,
        };
    }
    let Some(raw) = whole.value.get(section).cloned() else {
        return Loaded {
            value: T::default(),
            warnings: whole.warnings,
            error: None,
            missing: false,
        };
    };
    match serde_path_to_error::deserialize::<_, T>(raw) {
        Ok(value) => Loaded {
            value,
            warnings: whole.warnings,
            error: None,
            missing: false,
        },
        Err(e) => {
            let key = e.path().to_string();
            let key = if key == "." {
                section.to_string()
            } else {
                format!("{section}.{key}")
            };
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let header = format!("[{section}]");
            let line = text
                .lines()
                .position(|l| l.trim() == header)
                .map_or(1, |i| i + 1);
            Loaded {
                value: T::default(),
                warnings: Vec::new(),
                error: Some(ConfigIssue {
                    file: path.to_path_buf(),
                    line,
                    column: 1,
                    key: Some(key),
                    message: e
                        .into_inner()
                        .message()
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string(),
                }),
                missing: false,
            }
        }
    }
}

/// Writes one section of a file, merged into the existing text. Other sections stay as they are.
/// Refuses when the file is broken: a broken file is never overwritten.
pub fn save_section<T: Serialize>(
    path: &Path,
    section: &str,
    value: &T,
    template: &str,
    prune: &[&str],
) -> Result<(), String> {
    let whole: Loaded<toml::Table> = load_file(path);
    if let Some(issue) = whole.error {
        return Err(format!("配置文件有错误，未保存：{issue}"));
    }
    let mut wrapper = BTreeMap::new();
    wrapper.insert(section.to_string(), value);
    save_file(path, &wrapper, template, prune)
}

/// User-level settings in `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UserConfig {
    /// Keyboard shortcut overrides: command id to its chords, e.g. `save = ["ctrl-shift-y"]`.
    /// An empty list turns the command's shortcut off.
    pub keys: BTreeMap<String, Vec<String>>,
}

/// Machine state in `state.toml` (user level).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UserState {
    /// Vault folders, most recently used first.
    pub recent_vaults: Vec<String>,
    /// The vault that was open when the app last closed (empty = none).
    pub last_vault: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins_over_the_platform_folder() {
        let default = PathBuf::from("/default/jiyue");
        assert_eq!(
            user_dir_from(Some(PathBuf::from("/scratch")), default.clone()),
            PathBuf::from("/scratch")
        );
        assert_eq!(
            user_dir_from(Some(PathBuf::new()), default.clone()),
            default
        );
        assert_eq!(user_dir_from(None, default.clone()), default);
    }

    #[test]
    fn vault_files_live_under_dot_jiyue() {
        let root = PathBuf::from("/notes");
        assert_eq!(
            vault_file(&root, VAULT_CONFIG_FILE),
            PathBuf::from("/notes").join(".jiyue").join("config.toml")
        );
    }

    #[test]
    fn credentials_dir_inside_a_vault_is_refused() {
        let base = std::env::temp_dir().join(format!("jiyue-guard-{}", std::process::id()));
        let vault = base.join("vault");
        let inside = vault.join("hidden");
        std::fs::create_dir_all(&inside).expect("mkdir");
        let err = check_user_dir_outside_vaults(&inside, std::slice::from_ref(&vault))
            .expect_err("inside a vault");
        assert!(err.contains("凭据"));
        let outside = base.join("cfg");
        std::fs::create_dir_all(&outside).expect("mkdir");
        assert!(check_user_dir_outside_vaults(&outside, std::slice::from_ref(&vault)).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn section_round_trip_keeps_other_sections() {
        let dir = std::env::temp_dir().join(format!("jiyue-section-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            &path,
            "version = 1\n# 保留\n[editor]\nauto_name_from_heading = false\n",
        )
        .expect("write");
        #[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
        #[serde(default)]
        struct Daily {
            folder: String,
        }
        let daily = Daily {
            folder: "日记".into(),
        };
        save_section(&path, "daily", &daily, templates::VAULT_CONFIG, &[]).expect("save");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("# 保留"), "{text}");
        assert!(text.contains("auto_name_from_heading = false"), "{text}");
        assert!(text.contains("[daily]"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
