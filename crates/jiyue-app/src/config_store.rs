//! The app's view of the configuration files.
//!
//! The typed values are read from disk where they are used (the files are small, and each section
//! has its own loader in its module). This store keeps what must outlive a single read: which files
//! are broken (shown as a banner and an error toast), and the modification stamps of every file, so
//! an external edit is noticed (the app polls `poll` on focus and every few seconds) and reloaded
//! with a toast. A broken file is never overwritten: the save functions refuse while it is broken.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::{App, Bounds, Pixels, Window, WindowBounds, point, px, size};
use jiyue_core::config::appearance::Appearance;
use jiyue_core::config::prefs::{self, Rect, UiPrefs, WindowState};
use jiyue_core::config::{self, ConfigIssue, Loaded, UserConfig, UserState};

use crate::ui::ext::ToastKind;
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
    pub vault: jiyue_core::config::vault_meta::VaultMeta,
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
    /// The interface preferences as the app had them at the last check (see `sync_ui_prefs`).
    /// `None` until the first check, which becomes the baseline and writes nothing.
    ui_baseline: Option<UiPrefs>,
    /// The vault's display name from its `[vault] name` ("" = the folder name).
    vault_display: String,
    /// The window title last set, so it is only changed when the name changes.
    window_title: Option<String>,
}

impl ConfigStore {
    /// Watches the user-level files of `user_dir`, and the vault files of `vault_root` when a vault
    /// is open. Call at start-up and whenever the vault changes.
    pub fn open(&mut self, user_dir: &Path, vault_root: Option<&Path>) {
        self.user_dir = Some(user_dir.to_path_buf());
        self.vault_root = vault_root.map(Path::to_path_buf);
        self.vault_display = vault_root
            .map(jiyue_core::config::vault_meta::read_name)
            .unwrap_or_default();
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
                if file.kind == Kind::UserConfig {
                    // An external edit of the fonts takes effect at once (see `ui::fonts`).
                    crate::ui::fonts::reload_from_disk();
                }
                if file.kind == Kind::VaultConfig
                    && let Some(root) = self.vault_root.as_deref()
                {
                    self.vault_display = jiyue_core::config::vault_meta::read_name(root);
                }
                changed.push(file.path.clone());
            }
        }
        changed
    }

    /// Writes `[appearance]` to the user's `config.toml` (see `write_appearance_at`) and marks the
    /// file as written by the app, so the write is not reported as an external change.
    pub fn write_appearance(&mut self, value: &Appearance) -> Result<(), String> {
        let Some(dir) = self.user_dir.clone() else {
            return Ok(());
        };
        let path = dir.join(config::USER_CONFIG_FILE);
        if self.is_broken(&path) {
            return Err("配置文件有错误，字体设置未能保存".to_string());
        }
        let result = write_appearance_at(&dir, value);
        self.note_written(&path);
        result
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

    /// Re-reads one watched file after the app wrote it itself, so the write is not reported as
    /// an external change (which would show the "配置已重新载入" toast).
    pub fn note_written(&mut self, path: &Path) {
        for file in self.files.iter_mut().filter(|f| f.path == path) {
            file.stamp = stamp_of(&file.path);
            file.issue = check(&file.path, file.kind);
        }
    }

    /// The name of the open vault as shown in the sidebar and the window title: its display name,
    /// or the folder name.
    pub fn vault_title(&self) -> String {
        let folder = self
            .vault_root
            .as_deref()
            .and_then(|r| r.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let folder = if folder.is_empty() {
            "我的笔记库"
        } else {
            folder
        };
        jiyue_core::config::vault_meta::shown_name(&self.vault_display, folder)
    }

    /// Records the display name just saved for the open vault.
    pub fn set_vault_display(&mut self, name: &str) {
        self.vault_display = name.trim().to_string();
        if let Some(root) = self.vault_root.clone() {
            let path = config::vault_file(&root, config::VAULT_CONFIG_FILE);
            self.note_written(&path);
        }
    }

    /// The `[ui]` preferences as they are on disk (defaults when the file is missing or broken).
    pub fn ui_prefs_on_disk(&self) -> UiPrefs {
        match &self.user_dir {
            Some(dir) => read_ui_prefs_at(dir),
            None => UiPrefs::default(),
        }
    }

    /// Writes the interface preferences that changed since the last check to `[ui]` of the user's
    /// `config.toml`. Only the changed keys are written, on top of what the file holds, so a value
    /// that was set for this run only (for example by `--theme`) is not saved unless it changes.
    /// The first call sets the baseline and writes nothing. A broken file is never overwritten; the
    /// change is then dropped and reported once.
    pub fn sync_ui_prefs(&mut self, now: &UiPrefs) -> Result<(), String> {
        let Some(base) = self.ui_baseline.replace(now.clone()) else {
            return Ok(());
        };
        if &base == now {
            return Ok(());
        }
        let Some(dir) = self.user_dir.clone() else {
            return Ok(());
        };
        let path = dir.join(config::USER_CONFIG_FILE);
        if self.is_broken(&path) {
            return Err("配置文件有错误，界面设置未能保存".to_string());
        }
        let mut disk: UiPrefs = config::load_section(&path, UI_SECTION).value;
        if base.theme != now.theme {
            disk.theme = now.theme.clone();
        }
        if base.locale != now.locale {
            disk.locale = now.locale.clone();
        }
        if base.live_preview != now.live_preview {
            disk.live_preview = now.live_preview;
        }
        if base.left_sidebar != now.left_sidebar {
            disk.left_sidebar = now.left_sidebar;
        }
        if base.right_sidebar != now.right_sidebar {
            disk.right_sidebar = now.right_sidebar;
        }
        let result = config::save_section(
            &path,
            UI_SECTION,
            &disk,
            config::templates::USER_CONFIG,
            &[],
        );
        self.note_written(&path);
        result
    }
}

/// Section names in the user files.
const UI_SECTION: &str = "ui";
const WINDOW_SECTION: &str = "window";
const APPEARANCE_SECTION: &str = "appearance";

/// The `[ui]` preferences of the user folder `user_dir`. Missing or broken: the defaults.
pub fn read_ui_prefs_at(user_dir: &Path) -> UiPrefs {
    config::load_section(&user_dir.join(config::USER_CONFIG_FILE), UI_SECTION).value
}

/// The `[appearance]` section of the user folder `user_dir`, clamped to its ranges, with one
/// warning per out-of-range value. Missing or broken: the defaults.
pub fn read_appearance_at(user_dir: &Path) -> (Appearance, Vec<String>) {
    let path = user_dir.join(config::USER_CONFIG_FILE);
    let raw: Appearance = config::load_section(&path, APPEARANCE_SECTION).value;
    raw.clamped()
}

/// Writes the `[appearance]` section into the user's `config.toml`, keeping the rest of the file
/// and its comments. A broken config file is not overwritten.
pub fn write_appearance_at(user_dir: &Path, value: &Appearance) -> Result<(), String> {
    let path = user_dir.join(config::USER_CONFIG_FILE);
    config::save_section(
        &path,
        APPEARANCE_SECTION,
        value,
        config::templates::USER_CONFIG,
        &[],
    )
}

/// The `[window]` record of the user folder `user_dir`. Missing or broken: nothing saved.
pub fn read_window_at(user_dir: &Path) -> WindowState {
    config::load_section(&user_dir.join(config::USER_STATE_FILE), WINDOW_SECTION).value
}

/// Writes the `[window]` record into `state.toml`, keeping the rest of the file. A broken state
/// file is not overwritten.
pub fn write_window_at(user_dir: &Path, state: &WindowState) -> Result<(), String> {
    config::save_section(
        &user_dir.join(config::USER_STATE_FILE),
        WINDOW_SECTION,
        state,
        config::templates::USER_STATE,
        &[],
    )
}

/// The theme saved in the user's `config.toml`, before the app starts (the `--theme` flag is
/// resolved against it in `main.rs`).
pub fn saved_theme() -> String {
    read_ui_prefs_at(&config::user_dir()).theme
}

/// The interface locale saved in `config.toml`.
pub fn saved_locale() -> String {
    read_ui_prefs_at(&config::user_dir()).locale
}

/// Writes the interface preferences of the running app, at most once per change (called at the
/// start of every render). A failure is reported once as a toast.
pub fn sync_ui(app: &mut crate::RustdianApp) {
    let now = UiPrefs {
        theme: crate::vaults::theme_arg(app.theme.kind).to_string(),
        locale: crate::i18n::configured_locale(),
        live_preview: app.live_preview,
        left_sidebar: app.left_sidebar_open,
        right_sidebar: app.right_sidebar_open,
    };
    if let Err(message) = app.configs.sync_ui_prefs(&now) {
        app.toast(ToastKind::Error, message);
    }
}

/// Keeps the window title as `<vault name> - 霁月 Jiyue`. Called every frame; the title is only
/// set when the name changed.
pub fn sync_window_title(app: &mut crate::RustdianApp, window: &mut Window) {
    let title = format!("{} - 霁月 Jiyue", app.configs.vault_title());
    if app.configs.window_title.as_deref() != Some(title.as_str()) {
        window.set_window_title(&title);
        app.configs.window_title = Some(title);
    }
}

/// The window bounds for the first window: the saved place and size when they fit a display,
/// otherwise centred at `default_size`.
pub fn startup_window(cx: &App, default_size: (f32, f32)) -> WindowBounds {
    let saved = read_window_at(&config::user_dir());
    let displays: Vec<Rect> = cx
        .displays()
        .iter()
        .map(|d| {
            let b = d.bounds();
            (
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
                f32::from(b.size.height),
            )
        })
        .collect();
    if prefs::window_fits(&saved, &displays) {
        let bounds = Bounds::new(
            point(px(saved.x), px(saved.y)),
            size(px(saved.width), px(saved.height)),
        );
        return match saved.mode.as_str() {
            "maximized" => WindowBounds::Maximized(bounds),
            "fullscreen" => WindowBounds::Fullscreen(bounds),
            _ => WindowBounds::Windowed(bounds),
        };
    }
    WindowBounds::Windowed(Bounds::centered(
        None,
        size(px(default_size.0), px(default_size.1)),
        cx,
    ))
}

/// Saves the window's place and size (called when the window is asked to close). Minimised
/// windows report no usable size and are skipped.
pub fn save_window(window: &Window) {
    let (mode, bounds): (&str, Bounds<Pixels>) = match window.window_bounds() {
        WindowBounds::Windowed(b) => ("windowed", b),
        WindowBounds::Maximized(b) => ("maximized", b),
        WindowBounds::Fullscreen(b) => ("fullscreen", b),
    };
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let state = WindowState {
        mode: mode.to_string(),
        x: f32::from(bounds.origin.x),
        y: f32::from(bounds.origin.y),
        width,
        height,
    };
    let _ = write_window_at(&config::user_dir(), &state);
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

    /// A fresh user folder under the temp dir (stands in for `JIYUE_CONFIG_DIR`).
    fn user_folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jiyue-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    /// Opens a store the way the app does at start-up, with the given user folder.
    fn open_store(user: &Path) -> ConfigStore {
        let mut store = ConfigStore::default();
        store.open(user, None);
        store
    }

    #[test]
    fn ui_changes_are_saved_and_restored_by_a_new_store() {
        let user = user_folder("restore");
        // Start-up: the app runs with the explicit `--theme paper` flag and the defaults.
        let mut store = open_store(&user);
        let applied = UiPrefs {
            theme: "paper".into(),
            ..read_ui_prefs_at(&user)
        };
        // The first check is the baseline: the flag is applied but not written.
        store.sync_ui_prefs(&applied).expect("baseline");
        assert!(!user.join(config::USER_CONFIG_FILE).exists());

        // The user hides the left sidebar and switches the theme to fog.
        let changed = UiPrefs {
            theme: "fog".into(),
            left_sidebar: false,
            ..applied.clone()
        };
        store.sync_ui_prefs(&changed).expect("save");

        // A new store (a new start) reads what was saved.
        let back = open_store(&user).ui_prefs_on_disk();
        assert_eq!(back.theme, "fog");
        assert!(!back.left_sidebar);
        assert!(back.right_sidebar);
        assert!(back.live_preview);
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn a_flag_only_value_is_not_written_when_something_else_changes() {
        let user = user_folder("flag");
        std::fs::write(
            user.join(config::USER_CONFIG_FILE),
            "version = 1\n[ui]\ntheme = \"moss\"\n",
        )
        .expect("write");
        let mut store = open_store(&user);
        let applied = UiPrefs {
            theme: "graphite".into(),
            ..read_ui_prefs_at(&user)
        };
        store.sync_ui_prefs(&applied).expect("baseline");
        let changed = UiPrefs {
            right_sidebar: false,
            ..applied
        };
        store.sync_ui_prefs(&changed).expect("save");
        let back = read_ui_prefs_at(&user);
        assert_eq!(
            back.theme, "moss",
            "the flag did not overwrite the saved theme"
        );
        assert!(!back.right_sidebar);
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn saving_ui_keeps_keys_comments_and_unknown_keys() {
        let user = user_folder("keep");
        let path = user.join(config::USER_CONFIG_FILE);
        std::fs::write(
            &path,
            "version = 1\n# 我的备注\n[keys]\n\"file.save\" = [\"ctrl-shift-s\"]\n\n[ui]\n# 侧栏\nleft_sidebar = true\nmystery = 3\n",
        )
        .expect("write");
        let mut store = open_store(&user);
        let applied = read_ui_prefs_at(&user);
        store.sync_ui_prefs(&applied).expect("baseline");
        store
            .sync_ui_prefs(&UiPrefs {
                left_sidebar: false,
                ..applied
            })
            .expect("save");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("# 我的备注"), "{text}");
        assert!(text.contains("# 侧栏"), "{text}");
        assert!(text.contains("ctrl-shift-s"), "{text}");
        assert!(text.contains("mystery = 3"), "{text}");
        assert!(text.contains("left_sidebar = false"), "{text}");
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn a_broken_config_is_never_overwritten_and_the_change_is_reported_once() {
        let user = user_folder("broken");
        let path = user.join(config::USER_CONFIG_FILE);
        let broken = "version = 1\n[ui\nleft_sidebar = true\n";
        std::fs::write(&path, broken).expect("write");
        let mut store = open_store(&user);
        assert!(store.is_broken(&path));
        let base = UiPrefs::default();
        store.sync_ui_prefs(&base).expect("baseline");
        let changed = UiPrefs {
            left_sidebar: false,
            ..base.clone()
        };
        assert!(store.sync_ui_prefs(&changed).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), broken);
        // The same state again is not reported a second time.
        assert!(store.sync_ui_prefs(&changed).is_ok());
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn a_broken_config_reads_as_the_defaults() {
        let user = user_folder("readbroken");
        std::fs::write(user.join(config::USER_CONFIG_FILE), "[ui\n").expect("write");
        assert_eq!(read_ui_prefs_at(&user), UiPrefs::default());
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn a_missing_config_is_created_with_the_changed_values_only() {
        let user = user_folder("missing");
        let mut store = open_store(&user);
        let base = UiPrefs::default();
        store.sync_ui_prefs(&base).expect("baseline");
        store
            .sync_ui_prefs(&UiPrefs {
                live_preview: false,
                ..base
            })
            .expect("save");
        let text = std::fs::read_to_string(user.join(config::USER_CONFIG_FILE)).expect("read");
        assert!(text.contains("live_preview = false"), "{text}");
        assert!(read_ui_prefs_at(&user).theme == "ink");
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn window_record_is_kept_next_to_the_recent_vaults() {
        let user = user_folder("window");
        let state = user.join(config::USER_STATE_FILE);
        std::fs::write(
            &state,
            "version = 1\nrecent_vaults = [\"D:\\\\notes\"]\nlast_vault = \"D:\\\\notes\"\n",
        )
        .expect("write");
        let saved = WindowState {
            mode: "maximized".into(),
            x: 20.0,
            y: 30.0,
            width: 1440.0,
            height: 900.0,
        };
        write_window_at(&user, &saved).expect("save window");
        assert_eq!(read_window_at(&user), saved);
        let text = std::fs::read_to_string(&state).expect("read");
        assert!(text.contains("recent_vaults"), "{text}");
        assert!(text.contains("last_vault"), "{text}");
        // A broken state file is not overwritten by a window save.
        std::fs::write(&state, "recent_vaults = [").expect("break");
        assert!(write_window_at(&user, &WindowState::default()).is_err());
        assert_eq!(
            std::fs::read_to_string(&state).expect("read"),
            "recent_vaults = ["
        );
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

    #[test]
    fn appearance_is_optional_and_saving_keeps_other_sections() {
        let dir = scratch("appearance");
        let path = dir.join(config::USER_CONFIG_FILE);
        std::fs::write(
            &path,
            "version = 1\n# 我的注释\n[ui]\ntheme = \"fog\"\n[keys]\n\"file.save\" = [\"ctrl-shift-y\"]\n",
        )
        .expect("write");
        let (value, warnings) = read_appearance_at(&dir);
        assert_eq!(
            value,
            Appearance::default(),
            "a file without [appearance] gives the defaults"
        );
        assert!(warnings.is_empty());
        let changed = Appearance {
            ui_font: "Consolas".into(),
            editor_font_size: 18.0,
            ..Appearance::default()
        };
        write_appearance_at(&dir, &changed).expect("save");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("# 我的注释"), "{text}");
        assert!(text.contains("theme = \"fog\""), "{text}");
        assert!(text.contains("file.save"), "{text}");
        assert!(text.contains("[appearance]"), "{text}");
        let (back, _) = read_appearance_at(&dir);
        assert_eq!(back.editor_font_size, 18.0);
        assert_eq!(back.ui_font, "Consolas");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn out_of_range_appearance_is_clamped_with_a_warning() {
        let dir = scratch("appearance-range");
        std::fs::write(
            dir.join(config::USER_CONFIG_FILE),
            "version = 1\n[appearance]\neditor_font_size = 99.0\n",
        )
        .expect("write");
        let (value, warnings) = read_appearance_at(&dir);
        assert_eq!(value.editor_font_size, 36.0);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
