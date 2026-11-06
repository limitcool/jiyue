//! The app's view of the configuration files.
//!
//! The typed values are read from disk where they are used (the files are small, and each section
//! has its own loader in its module). This store keeps what must outlive a single read: which files
//! are broken (shown as a banner and an error toast), and the modification stamps of every file, so
//! an external edit is noticed (the app polls `poll` on focus and every few seconds) and reloaded
//! with a toast. A broken file is never overwritten: the save functions refuse while it is broken.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use jiyue_core::config::{self, ConfigIssue, Loaded, UserConfig, UserState};

use crate::ui::files_ui::autoname::EditorSettings;
use jiyue_ai::AiConfig;
use jiyue_core::daily::DailySettings;
use jiyue_export::templates::TemplateRules;
use jiyue_md::attach::AttachSettings;

use crate::ui::sync_ui::config::SyncConfig;

/// The whole vault configuration file, as one typed value. Used to check the file as a whole, so a
/// problem anywhere in it is reported with its line and column.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct VaultConfigDoc {
    pub editor: EditorSettings,
    pub daily: DailySettings,
    pub templates: TemplateRules,
    pub attachments: AttachSettings,
    pub sync: SyncConfig,
    pub ai: AiConfig,
}

/// Which typed shape a file is checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    UserConfig,
    UserState,
    Credentials,
    VaultConfig,
    VaultState,
    Bookmarks,
}

impl Kind {
    /// Keys that are maps (their entries are free-form) for this file.
    fn free_paths(self) -> &'static [&'static str] {
        match self {
            Kind::UserConfig => &["keys"],
            Kind::VaultConfig => &["templates.folder_templates"],
            _ => &[],
        }
    }
}

/// Modification stamp of a file. `None` when the file does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

/// One watched file: its last stamp and its problem, if any.
#[derive(Debug, Clone)]
pub struct Watched {
    pub path: PathBuf,
    pub kind: Kind,
    stamp: Option<Stamp>,
    pub issue: Option<ConfigIssue>,
}

/// Live state of the configuration files of the app and of the open vault.
#[derive(Debug, Default)]
pub struct ConfigStore {
    files: Vec<Watched>,
    /// The user-level folder, as the files were opened.
    user_dir: Option<PathBuf>,
    /// The vault that is open, when there is one.
    vault_root: Option<PathBuf>,
    /// Set once the user config file was found broken at start-up, so the banner can say so.
    pub startup_notice: Option<String>,
}

impl ConfigStore {
    /// Watches the user-level files of `user_dir`, and the vault files of `vault_root` when a vault
    /// is open. Call at start-up and whenever the vault changes.
    pub fn open(&mut self, user_dir: &Path, vault_root: Option<&Path>) {
        self.user_dir = Some(user_dir.to_path_buf());
        self.vault_root = vault_root.map(Path::to_path_buf);
        let mut files = vec![
            (user_dir.join(config::USER_CONFIG_FILE), Kind::UserConfig),
            (user_dir.join(config::USER_STATE_FILE), Kind::UserState),
            (user_dir.join(config::CREDENTIALS_FILE), Kind::Credentials),
        ];
        if let Some(root) = vault_root {
            files.push((
                config::vault_file(root, config::VAULT_CONFIG_FILE),
                Kind::VaultConfig,
            ));
            files.push((
                config::vault_file(root, config::VAULT_STATE_FILE),
                Kind::VaultState,
            ));
            files.push((
                config::vault_file(root, config::BOOKMARKS_FILE),
                Kind::Bookmarks,
            ));
        }
        self.files = files
            .into_iter()
            .map(|(path, kind)| {
                let stamp = stamp_of(&path);
                let issue = check(&path, kind);
                Watched {
                    path,
                    kind,
                    stamp,
                    issue,
                }
            })
            .collect();
        self.startup_notice = self
            .files
            .iter()
            .find(|f| f.kind == Kind::UserConfig)
            .and_then(|f| f.issue.as_ref())
            .map(|issue| format!("快捷键和全局设置文件有错误：{issue}"));
        // Credentials must never live in a vault: refuse such a user folder and say why.
        if let Some(root) = vault_root
            && let Err(message) = config::check_user_dir_outside_vaults(
                user_dir,
                std::slice::from_ref(&root.to_path_buf()),
            )
        {
            self.startup_notice = Some(message);
        }
    }

    /// Re-checks every watched file. Returns the paths whose stamp changed since the last call;
    /// their issues are refreshed.
    pub fn poll(&mut self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        for file in &mut self.files {
            let now = stamp_of(&file.path);
            if now != file.stamp {
                file.stamp = now;
                file.issue = check(&file.path, file.kind);
                changed.push(file.path.clone());
            }
        }
        changed
    }

    /// Every file that cannot be read, with the reason and the position.
    pub fn broken(&self) -> Vec<&ConfigIssue> {
        self.files.iter().filter_map(|f| f.issue.as_ref()).collect()
    }

    /// True when the file at `path` cannot be read. Writers check this before overwriting.
    pub fn is_broken(&self, path: &Path) -> bool {
        self.files
            .iter()
            .any(|f| f.path == path && f.issue.is_some())
    }

    /// The user-level folder being watched.
    pub fn user_dir(&self) -> Option<&Path> {
        self.user_dir.as_deref()
    }

    /// The vault being watched.
    pub fn vault_root(&self) -> Option<&Path> {
        self.vault_root.as_deref()
    }
}

/// Checks one file against its typed shape. `None` when it is fine, or when it does not exist.
pub fn check(path: &Path, kind: Kind) -> Option<ConfigIssue> {
    match kind {
        Kind::UserConfig => check_typed::<UserConfig>(path, kind),
        Kind::UserState => check_typed::<UserState>(path, kind),
        Kind::Credentials => check_typed::<config::credentials::CredentialsFile>(path, kind),
        Kind::VaultConfig => check_typed::<VaultConfigDoc>(path, kind),
        Kind::VaultState => check_typed::<toml::Table>(path, kind),
        Kind::Bookmarks => {
            check_typed::<crate::ui::ext::bookmarks::logic::BookmarksDoc>(path, kind)
        }
    }
}

fn check_typed<T>(path: &Path, kind: Kind) -> Option<ConfigIssue>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Default,
{
    let loaded: Loaded<T> = config::load_file_with(path, kind.free_paths());
    loaded.error
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jiyue-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".jiyue")).expect("mkdir");
        dir
    }

    #[test]
    fn a_saved_key_lands_only_in_credentials_and_comments_survive() {
        use crate::ui::sync_ui::config::{Backend, SyncConfig};
        use jiyue_core::config::credentials::{self, AI_TABLE, SYNC_TABLE};

        let base = scratch("secrets");
        let user = base.join("user");
        let vault = base.join("vault");
        std::fs::create_dir_all(&user).expect("mkdir");
        std::fs::create_dir_all(vault.join(".jiyue")).expect("mkdir");
        let cred_path = user.join(config::CREDENTIALS_FILE);
        let vault_cfg = config::vault_file(&vault, config::VAULT_CONFIG_FILE);

        // Hand-written comments in both files.
        std::fs::write(
            &vault_cfg,
            "version = 1\n# 我的笔记库备注\n[editor]\nauto_name_from_heading = true\n",
        )
        .expect("write");
        std::fs::write(&cred_path, "version = 1\n# 我的凭据备注\n").expect("write");

        // Sync settings with the identity and a secret, saved the way the settings page saves.
        let cfg = SyncConfig {
            vault_id: "vid1".into(),
            backend: Backend::S3,
            s3_endpoint: "https://s3.example.com".into(),
            s3_bucket: "notes".into(),
            s3_access_key_id: "AKIDEXAMPLE".into(),
            ..SyncConfig::default()
        };
        cfg.save(&vault).expect("save sync section");
        crate::ui::sync_ui::secrets::save_identity_at(&cred_path, &cfg).expect("identity");
        credentials::write_field(
            &cred_path,
            SYNC_TABLE,
            "vid1",
            "secret_access_key",
            Some("SYNC-SECRET-XYZ"),
        )
        .expect("sync secret");
        credentials::write_field(
            &cred_path,
            AI_TABLE,
            "openai",
            "api_key",
            Some("sk-AI-SECRET-123"),
        )
        .expect("ai key");

        let vault_text = std::fs::read_to_string(&vault_cfg).expect("read vault config");
        let cred_text = std::fs::read_to_string(&cred_path).expect("read credentials");
        // The secrets and the identity are only in credentials.toml.
        for secret in ["SYNC-SECRET-XYZ", "sk-AI-SECRET-123", "AKIDEXAMPLE"] {
            assert!(
                !vault_text.contains(secret),
                "{secret} leaked into the vault: {vault_text}"
            );
            assert!(
                cred_text.contains(secret),
                "{secret} missing from credentials: {cred_text}"
            );
        }
        assert!(!vault_text.contains("access_key_id"), "{vault_text}");
        // Comments written by hand survive in both files.
        assert!(vault_text.contains("# 我的笔记库备注"), "{vault_text}");
        assert!(cred_text.contains("# 我的凭据备注"), "{cred_text}");
        // The non-secret settings are in the vault section.
        assert!(
            vault_text.contains("s3_endpoint = \"https://s3.example.com\""),
            "{vault_text}"
        );

        // Reading back fills the identity from credentials.
        let mut back = SyncConfig::load(&vault).expect("load");
        assert_eq!(back.s3_access_key_id, "");
        crate::ui::sync_ui::secrets::fill_identity_at(&cred_path, &mut back);
        assert_eq!(back.s3_access_key_id, "AKIDEXAMPLE");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn default_vault_template_loads_to_the_defaults() {
        // The comments and the values of the default file must not drift from the structs.
        let loaded: Loaded<VaultConfigDoc> = config::doc::load_text_with(
            Path::new("template.toml"),
            config::templates::VAULT_CONFIG,
            &["templates.folder_templates"],
        );
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        // The default device name is this machine's host name, so it is taken from the file.
        let mut expected = VaultConfigDoc::default();
        expected.sync.device_name = loaded.value.sync.device_name.clone();
        assert_eq!(loaded.value, expected);
    }

    #[test]
    fn external_edit_is_detected_by_stamp_and_reported_as_changed() {
        let user = scratch("user");
        let vault = scratch("vault");
        let mut store = ConfigStore::default();
        store.open(&user, Some(&vault));
        assert!(store.poll().is_empty(), "nothing changed yet");

        let cfg = config::vault_file(&vault, config::VAULT_CONFIG_FILE);
        std::fs::write(
            &cfg,
            "version = 1\n[editor]\nauto_name_from_heading = false\n",
        )
        .expect("write");
        let changed = store.poll();
        assert_eq!(changed, vec![cfg.clone()]);
        assert!(store.broken().is_empty());
        assert!(store.poll().is_empty(), "a second poll sees no change");

        std::fs::write(&cfg, "version = 1\n[editor\n").expect("write broken");
        assert_eq!(store.poll(), vec![cfg.clone()]);
        assert_eq!(store.broken().len(), 1);
        assert!(store.is_broken(&cfg));
        let _ = std::fs::remove_dir_all(&user);
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn broken_user_config_sets_the_startup_notice_with_position() {
        let user = scratch("notice");
        std::fs::write(user.join(config::USER_CONFIG_FILE), "version = 1\n[keys\n").expect("write");
        let mut store = ConfigStore::default();
        store.open(&user, None);
        let notice = store.startup_notice.clone().expect("notice");
        assert!(notice.contains(":2:"), "{notice}");
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn a_wrong_type_in_the_vault_config_names_the_key() {
        let vault = scratch("typed");
        let cfg = config::vault_file(&vault, config::VAULT_CONFIG_FILE);
        std::fs::write(
            &cfg,
            "version = 1\n[editor]\nauto_name_from_heading = \"yes\"\n",
        )
        .expect("write");
        let issue = check(&cfg, Kind::VaultConfig).expect("issue");
        assert_eq!(issue.line, 3);
        assert!(
            issue
                .key
                .as_deref()
                .is_some_and(|k| k.contains("auto_name_from_heading")),
            "{issue:?}"
        );
        let _ = std::fs::remove_dir_all(&vault);
    }
}
