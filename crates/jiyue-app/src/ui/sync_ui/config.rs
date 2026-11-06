//! Sync settings stored in the `[sync]` section of `<vault>/.jiyue/config.toml`, without any secret, and the pure rules that
//! check and normalise them. Nothing here touches the UI or the network.

use std::io;
use std::path::Path;

use jiyue_sync::IgnoreRules;
use jiyue_sync::store::{S3Config, StoreConfig, WebDavConfig};
use serde::{Deserialize, Serialize};

/// The `[sync]` section of the vault's `.jiyue/config.toml` (see `load`).
pub const SECTION: &str = "sync";

/// Remote storage kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    S3,
    WebDav,
}

impl Backend {
    /// Segment labels in the settings form.
    pub const LABELS: [&'static str; 2] = ["S3", "WebDAV"];

    pub fn index(self) -> usize {
        match self {
            Backend::S3 => 0,
            Backend::WebDav => 1,
        }
    }

    pub fn from_index(i: usize) -> Self {
        if i == 1 { Backend::WebDav } else { Backend::S3 }
    }

    /// Short key of the backend (`s3` or `webdav`).
    pub fn key(self) -> &'static str {
        match self {
            Backend::S3 => "s3",
            Backend::WebDav => "webdav",
        }
    }
}

/// Automatic sync interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interval {
    S30,
    M1,
    #[default]
    M5,
    M15,
}

impl Interval {
    pub const LABELS: [&'static str; 4] = ["30 秒", "1 分钟", "5 分钟", "15 分钟"];

    pub fn secs(self) -> u64 {
        match self {
            Interval::S30 => 30,
            Interval::M1 => 60,
            Interval::M5 => 300,
            Interval::M15 => 900,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Interval::S30 => 0,
            Interval::M1 => 1,
            Interval::M5 => 2,
            Interval::M15 => 3,
        }
    }

    pub fn from_index(i: usize) -> Self {
        match i {
            0 => Interval::S30,
            1 => Interval::M1,
            3 => Interval::M15,
            _ => Interval::M5,
        }
    }
}

/// Choices for the save debounce, in seconds.
pub const DEBOUNCE_CHOICES: [u64; 3] = [10, 30, 60];
pub const DEBOUNCE_LABELS: [&str; 3] = ["10 秒", "30 秒", "1 分钟"];

/// Index of a debounce value in [`DEBOUNCE_CHOICES`]; unknown values map to the default (10 s).
pub fn debounce_index(secs: u64) -> usize {
    DEBOUNCE_CHOICES
        .iter()
        .position(|&c| c == secs)
        .unwrap_or(0)
}

/// The sync settings. Secrets (S3 secret key, WebDAV password) are not here: see `secrets.rs`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Random id of this vault's settings; the key of its entry in `credentials.toml`.
    pub vault_id: String,
    pub backend: Backend,
    pub s3_endpoint: String,
    pub s3_region: String,
    pub s3_bucket: String,
    /// Prefix inside the bucket that holds this vault, e.g. `jiyue/my-notes/`.
    pub s3_root_prefix: String,
    /// Kept in `credentials.toml` (see `secrets`), never in the vault's `[sync]` section.
    #[serde(skip)]
    pub s3_access_key_id: String,
    /// Path-style addressing (`endpoint/bucket/key`), needed by RustFS, MinIO and most self-hosted servers.
    pub s3_path_style: bool,
    pub webdav_endpoint: String,
    /// Kept in `credentials.toml` (see `secrets`), never in the vault's `[sync]` section.
    #[serde(skip)]
    pub webdav_username: String,
    pub webdav_root: String,
    /// Master switch. When off, nothing is synced (manual runs are refused too).
    pub enabled: bool,
    pub auto_sync: bool,
    pub interval: Interval,
    pub sync_on_save: bool,
    pub save_debounce_secs: u64,
    pub sync_on_start: bool,
    /// Run one sync when the window closes (waits at most 8 s; a second close click cancels).
    pub sync_on_exit: bool,
    pub device_name: String,
    /// One pattern per entry; merged with the built-in defaults of `IgnoreRules`.
    pub ignore_patterns: Vec<String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            vault_id: String::new(),
            backend: Backend::S3,
            s3_endpoint: String::new(),
            s3_region: "us-east-1".into(),
            s3_bucket: String::new(),
            s3_root_prefix: String::new(),
            s3_access_key_id: String::new(),
            s3_path_style: true,
            webdav_endpoint: String::new(),
            webdav_username: String::new(),
            webdav_root: String::new(),
            enabled: false,
            auto_sync: false,
            interval: Interval::M5,
            sync_on_save: false,
            save_debounce_secs: 10,
            sync_on_start: true,
            sync_on_exit: true,
            device_name: default_device_name(),
            ignore_patterns: Vec::new(),
        }
    }
}

/// The machine name, used as the device label in conflict-copy names.
pub fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "PC".into())
}

/// Default S3 prefix for a vault folder called `name`: `jiyue/<name>/`.
pub fn default_root_prefix(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' { '-' } else { c })
        .collect();
    format!("jiyue/{}/", clean.trim())
}

impl SyncConfig {
    /// Loads the `[sync]` section of the vault's configuration file. A missing section gives the
    /// defaults; a broken file is an error, so the user can fix it instead of losing settings.
    pub fn load(root: &Path) -> Result<Self, String> {
        let loaded: jiyue_core::config::Loaded<Self> = jiyue_core::config::load_section(
            &jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE),
            SECTION,
        );
        match loaded.error {
            Some(issue) => Err(format!("同步设置有错误：{issue}")),
            None => Ok(loaded.value),
        }
    }

    /// Writes the `[sync]` section, keeping the rest of the file. Never writes secrets.
    pub fn save(&self, root: &Path) -> io::Result<()> {
        jiyue_core::config::save_section(
            &jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE),
            SECTION,
            self,
            jiyue_core::config::templates::VAULT_CONFIG,
            &[],
        )
        .map_err(io::Error::other)
    }

    /// Fills in values that depend on the vault folder (prefix, vault id) when they are empty.
    pub fn with_defaults(
        mut self,
        vault_name: &str,
        new_vault_id: impl FnOnce() -> String,
    ) -> Self {
        if self.vault_id.is_empty() {
            self.vault_id = new_vault_id();
        }
        if self.s3_root_prefix.is_empty() {
            self.s3_root_prefix = default_root_prefix(vault_name);
        }
        self
    }

    /// Ignore rules from the textarea entries (defaults always apply).
    pub fn ignore_rules(&self) -> IgnoreRules {
        IgnoreRules::new(&self.ignore_patterns)
    }

    /// Checks the fields needed for `backend`. `has_secret` says whether a secret is stored or was
    /// typed. Messages are shown in the settings form and in toasts.
    pub fn validate(&self, has_secret: bool) -> Result<(), String> {
        match self.backend {
            Backend::S3 => {
                check_endpoint(&self.s3_endpoint)?;
                if self.s3_bucket.trim().is_empty() {
                    return Err("请填写存储桶名称".into());
                }
                if self.s3_access_key_id.trim().is_empty() {
                    return Err("请填写 Access Key ID".into());
                }
                if !has_secret {
                    return Err("请填写 Secret Access Key".into());
                }
                check_prefix(&self.s3_root_prefix)
            }
            Backend::WebDav => {
                check_endpoint(&self.webdav_endpoint)?;
                if self.webdav_username.trim().is_empty() {
                    return Err("请填写用户名".into());
                }
                if !has_secret {
                    return Err("请填写密码".into());
                }
                check_prefix(&self.webdav_root)
            }
        }
    }

    /// The store configuration for the active backend. `secret` is the S3 secret key or the WebDAV
    /// password. `test_s3` replaces the S3 fields (debug builds only, see [`test_override`]).
    pub fn store_config(&self, secret: &str, test_s3: Option<&TestS3Override>) -> StoreConfig {
        match (self.backend, test_s3) {
            (Backend::S3, Some(o)) => StoreConfig::S3(S3Config {
                endpoint: o.endpoint.clone(),
                region: o.region.clone(),
                bucket: o.bucket.clone(),
                access_key_id: o.access_key_id.clone(),
                secret_access_key: o.secret_access_key.clone(),
                session_token: None,
                root_prefix: normalize_prefix(&self.s3_root_prefix),
                force_path_style: self.s3_path_style,
            }),
            (Backend::S3, None) => StoreConfig::S3(S3Config {
                endpoint: normalize_endpoint(&self.s3_endpoint),
                region: self.s3_region.trim().to_owned(),
                bucket: self.s3_bucket.trim().to_owned(),
                access_key_id: self.s3_access_key_id.trim().to_owned(),
                secret_access_key: secret.to_owned(),
                session_token: None,
                root_prefix: normalize_prefix(&self.s3_root_prefix),
                force_path_style: self.s3_path_style,
            }),
            (Backend::WebDav, _) => StoreConfig::WebDav(WebDavConfig {
                endpoint: normalize_endpoint(&self.webdav_endpoint),
                username: self.webdav_username.trim().to_owned(),
                password: secret.to_owned(),
                root: normalize_prefix(&self.webdav_root),
            }),
        }
    }
}

/// S3 fields supplied by the test-only environment override.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestS3Override {
    pub endpoint: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
}

/// Reads the `JIYUE_TEST_S3_*` override from a lookup function. All five values must be present
/// and non-empty; otherwise there is no override.
pub fn test_override_from(lookup: impl Fn(&str) -> Option<String>) -> Option<TestS3Override> {
    let get = |key: &str| {
        lookup(key)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    Some(TestS3Override {
        endpoint: get("JIYUE_TEST_S3_ENDPOINT")?,
        bucket: get("JIYUE_TEST_S3_BUCKET")?,
        access_key_id: get("JIYUE_TEST_S3_KEY")?,
        secret_access_key: get("JIYUE_TEST_S3_SECRET")?,
        region: get("JIYUE_TEST_S3_REGION")?,
    })
}

/// The test override from the process environment. Only debug builds read it; release builds
/// always return `None`.
pub fn test_override() -> Option<TestS3Override> {
    if !cfg!(debug_assertions) {
        return None;
    }
    test_override_from(|key| std::env::var(key).ok())
}

/// Endpoint without surrounding blanks and without trailing slashes.
pub fn normalize_endpoint(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_owned()
}

/// Checks an endpoint URL: `http://` or `https://` followed by a host.
pub fn check_endpoint(raw: &str) -> Result<(), String> {
    let endpoint = normalize_endpoint(raw);
    if endpoint.is_empty() {
        return Err("请填写服务器地址".into());
    }
    let rest = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .ok_or_else(|| "服务器地址需以 http:// 或 https:// 开头".to_owned())?;
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || host.chars().any(|c| c.is_whitespace()) {
        return Err("服务器地址格式不正确".into());
    }
    Ok(())
}

/// Prefix or folder inside the store: forward slashes, no leading slash, a trailing slash when not
/// empty. An empty prefix means the store root.
pub fn normalize_prefix(raw: &str) -> String {
    let unified = raw.trim().replace('\\', "/");
    let trimmed = unified.trim_matches('/').trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

/// Rejects prefixes that try to leave the bucket or vault folder.
pub fn check_prefix(raw: &str) -> Result<(), String> {
    let normalized = normalize_prefix(raw);
    if normalized.split('/').any(|seg| seg == "..") {
        return Err("远端路径不能包含 ..".into());
    }
    Ok(())
}

/// Parses the ignore textarea: one pattern per line, blanks dropped.
pub fn parse_ignore_text(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s3_config() -> SyncConfig {
        SyncConfig {
            vault_id: "abc".into(),
            s3_endpoint: "http://127.0.0.1:9100/".into(),
            s3_bucket: "notes".into(),
            s3_access_key_id: "AK".into(),
            ..SyncConfig::default()
        }
    }

    #[test]
    fn endpoint_must_be_http_or_https_with_host() {
        assert!(check_endpoint("http://192.0.2.10:9100").is_ok());
        assert!(check_endpoint("https://s3.example.com/").is_ok());
        assert_eq!(check_endpoint("   "), Err("请填写服务器地址".into()));
        assert!(check_endpoint("s3.example.com").is_err());
        assert!(check_endpoint("ftp://x").is_err());
        assert!(check_endpoint("http://").is_err());
        assert!(check_endpoint("http:///bucket").is_err());
        assert!(check_endpoint("http://a b").is_err());
    }

    #[test]
    fn endpoint_trailing_slashes_are_dropped() {
        assert_eq!(normalize_endpoint(" http://h:9/// "), "http://h:9");
    }

    #[test]
    fn prefix_is_normalised_and_checked() {
        assert_eq!(normalize_prefix("/jiyue/notes"), "jiyue/notes/");
        assert_eq!(normalize_prefix("jiyue\\notes/"), "jiyue/notes/");
        assert_eq!(normalize_prefix("  /  "), "");
        assert!(check_prefix("jiyue/../other").is_err());
        assert!(check_prefix("jiyue/notes").is_ok());
    }

    #[test]
    fn default_prefix_uses_vault_name() {
        assert_eq!(default_root_prefix("my notes"), "jiyue/my notes/");
        assert_eq!(default_root_prefix("a/b"), "jiyue/a-b/");
    }

    #[test]
    fn validation_messages_are_chinese_and_specific() {
        let mut c = s3_config();
        assert!(c.validate(true).is_ok());
        assert_eq!(c.validate(false), Err("请填写 Secret Access Key".into()));
        c.s3_bucket = "  ".into();
        assert_eq!(c.validate(true), Err("请填写存储桶名称".into()));
        c.s3_bucket = "notes".into();
        c.s3_access_key_id.clear();
        assert_eq!(c.validate(true), Err("请填写 Access Key ID".into()));
        c.s3_access_key_id = "AK".into();
        c.s3_endpoint.clear();
        assert_eq!(c.validate(true), Err("请填写服务器地址".into()));
    }

    #[test]
    fn webdav_validation_checks_user_and_password() {
        let c = SyncConfig {
            backend: Backend::WebDav,
            webdav_endpoint: "https://dav.example.com/remote.php/dav/files/me".into(),
            ..SyncConfig::default()
        };
        assert_eq!(c.validate(true), Err("请填写用户名".into()));
        let c = SyncConfig {
            webdav_username: "me".into(),
            ..c
        };
        assert_eq!(c.validate(false), Err("请填写密码".into()));
        assert!(c.validate(true).is_ok());
    }

    #[test]
    fn store_config_uses_normalised_values_and_secret() {
        match s3_config().store_config("SECRET", None) {
            StoreConfig::S3(cfg) => {
                assert_eq!(cfg.endpoint, "http://127.0.0.1:9100");
                assert_eq!(cfg.secret_access_key, "SECRET");
                assert!(cfg.force_path_style);
                assert_eq!(cfg.region, "us-east-1");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_override_needs_all_five_values() {
        let full = |key: &str| {
            Some(
                match key {
                    "JIYUE_TEST_S3_ENDPOINT" => "http://h:1",
                    "JIYUE_TEST_S3_BUCKET" => "b",
                    "JIYUE_TEST_S3_KEY" => "k",
                    "JIYUE_TEST_S3_SECRET" => "s",
                    "JIYUE_TEST_S3_REGION" => "r",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let o = test_override_from(full).expect("all five present");
        assert_eq!(o.bucket, "b");
        assert!(
            test_override_from(|k| if k == "JIYUE_TEST_S3_KEY" {
                None
            } else {
                full(k)
            })
            .is_none()
        );
        assert!(
            test_override_from(|k| Some(if k == "JIYUE_TEST_S3_BUCKET" {
                "  ".into()
            } else {
                full(k)?.to_string()
            }))
            .is_none()
        );
    }

    #[test]
    fn override_replaces_s3_fields_only() {
        let o = TestS3Override {
            endpoint: "http://192.0.2.10:9100".into(),
            bucket: "jiyue-test-1".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            region: "us-east-1".into(),
        };
        match s3_config().store_config("ignored", Some(&o)) {
            StoreConfig::S3(cfg) => {
                assert_eq!(cfg.bucket, "jiyue-test-1");
                assert_eq!(cfg.secret_access_key, "s");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn ignore_text_is_one_pattern_per_line() {
        assert_eq!(
            parse_ignore_text("  *.tmp \n\n# note\nimages/\r\n"),
            vec!["*.tmp", "# note", "images/"]
        );
        let c = SyncConfig {
            ignore_patterns: vec!["*.tmp".into()],
            ..SyncConfig::default()
        };
        assert!(c.ignore_rules().is_ignored("a/b.tmp", false));
        assert!(c.ignore_rules().is_ignored(".jiyue/x", false));
    }

    #[test]
    fn interval_and_debounce_choices_round_trip() {
        for i in 0..4 {
            assert_eq!(Interval::from_index(i).index(), i);
        }
        assert_eq!(Interval::M15.secs(), 900);
        assert_eq!(debounce_index(30), 1);
        assert_eq!(debounce_index(7), 0);
    }

    #[test]
    fn config_round_trips_without_secrets_or_identity() {
        let c = s3_config().with_defaults("my notes", || "unused".into());
        let text = toml::to_string(&c).expect("serialise");
        assert!(!text.contains("secret"));
        assert!(
            !text.contains("access_key_id"),
            "identity lives in credentials.toml"
        );
        let back: SyncConfig = toml::from_str(&text).expect("parse");
        // The identity fields are skipped by design; everything else round-trips.
        let mut expected = c.clone();
        expected.s3_access_key_id.clear();
        expected.webdav_username.clear();
        assert_eq!(back, expected);
        assert_eq!(back.s3_root_prefix, "".to_owned() + "jiyue/my notes/");
    }

    #[test]
    fn missing_fields_take_defaults() {
        let c: SyncConfig = serde_json::from_str(r#"{"enabled":true}"#).expect("parse");
        assert!(c.enabled);
        assert_eq!(c.interval, Interval::M5);
        assert_eq!(c.save_debounce_secs, 10);
        assert!(c.s3_path_style);
    }
}
