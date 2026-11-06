//! AI settings: the `[ai]` section of `<vault>/.jiyue/config.toml` and the types it holds.
//!
//! The file never contains secrets. API keys live in `credentials.toml`, keyed by provider id.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ProviderConfig, ProviderKind};

/// One configured provider. Its API key is stored separately under `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEntry {
    /// Stable identifier, also the key of the provider's entry in `credentials.toml`. ASCII letters, digits, `-`
    /// and `_` only.
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub chat_model: String,
    /// Embedding model; `None` disables embeddings for this provider.
    #[serde(default)]
    pub embedding_model: Option<String>,
}

impl ProviderEntry {
    /// Connection settings for chat requests.
    pub fn chat_config(&self) -> ProviderConfig {
        ProviderConfig {
            base_url: self.base_url.clone(),
            model: self.chat_model.clone(),
            kind: self.kind,
        }
    }

    /// Connection settings for embedding requests, or `None` when no embedding model is set.
    pub fn embedding_config(&self) -> Option<ProviderConfig> {
        let model = self.embedding_model.as_ref()?.trim();
        if model.is_empty() {
            return None;
        }
        Some(ProviderConfig {
            base_url: self.base_url.clone(),
            model: model.to_string(),
            kind: self.kind,
        })
    }
}

/// Whole AI configuration. Defaults to disabled with no providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub providers: Vec<ProviderEntry>,
    #[serde(default)]
    pub active_chat: Option<String>,
    #[serde(default)]
    pub active_embedding: Option<String>,
    /// Vault-relative folders whose notes are never sent to a model. Matched as a path prefix on
    /// whole segments, so `private` covers `private/x.md` but not `privately.md`.
    #[serde(default)]
    pub excluded_folders: Vec<String>,
    /// Tags (without `#`) whose notes are never sent to a model. Case-insensitive.
    #[serde(default)]
    pub excluded_tags: Vec<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Invalid(m) => write!(f, "invalid AI settings: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl AiConfig {
    /// Checks ids, base URLs and references to active providers.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (i, p) in self.providers.iter().enumerate() {
            if !valid_id(&p.id) {
                return Err(ConfigError::Invalid(format!(
                    "provider id {:?} must use letters, digits, '-' or '_'",
                    p.id
                )));
            }
            if self.providers[..i].iter().any(|q| q.id == p.id) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate provider id {:?}",
                    p.id
                )));
            }
            let url = p.base_url.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(ConfigError::Invalid(format!(
                    "base URL of {:?} must start with https:// or http://",
                    p.id
                )));
            }
            if p.chat_model.trim().is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "chat model of {:?} is empty",
                    p.id
                )));
            }
        }
        for (label, active) in [
            ("active chat provider", &self.active_chat),
            ("active embedding provider", &self.active_embedding),
        ] {
            if let Some(id) = active
                && !self.providers.iter().any(|p| &p.id == id)
            {
                return Err(ConfigError::Invalid(format!(
                    "{label} {id:?} does not exist"
                )));
            }
        }
        Ok(())
    }

    /// The provider used for chat, if chat is enabled and one is selected.
    pub fn chat_provider(&self) -> Option<&ProviderEntry> {
        if !self.enabled {
            return None;
        }
        let id = self.active_chat.as_ref()?;
        self.providers.iter().find(|p| &p.id == id)
    }

    /// The provider used for embeddings, if one is selected and has an embedding model.
    pub fn embedding_provider(&self) -> Option<&ProviderEntry> {
        if !self.enabled {
            return None;
        }
        let id = self.active_embedding.as_ref()?;
        self.providers
            .iter()
            .find(|p| &p.id == id && p.embedding_config().is_some())
    }

    /// True when a note must never be sent to a model: its path is inside an excluded folder or
    /// it carries an excluded tag. `path` is vault-relative.
    pub fn is_note_excluded(&self, path: &str, tags: &[String]) -> bool {
        let norm_path = normalize_path(path);
        let in_folder = self.excluded_folders.iter().any(|folder| {
            let f = normalize_path(folder);
            !f.is_empty() && (norm_path == f || norm_path.starts_with(&format!("{f}/")))
        });
        if in_folder {
            return true;
        }
        tags.iter().any(|t| {
            let t = t.trim().trim_start_matches('#').to_lowercase();
            !t.is_empty()
                && self
                    .excluded_tags
                    .iter()
                    .any(|x| x.trim().trim_start_matches('#').to_lowercase() == t)
        })
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn normalize_path(p: &str) -> String {
    p.trim().replace('\\', "/").trim_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> ProviderEntry {
        ProviderEntry {
            id: id.into(),
            name: format!("Provider {id}"),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "https://api.example.test/v1".into(),
            chat_model: "chat-1".into(),
            embedding_model: Some("embed-1".into()),
        }
    }

    #[test]
    fn default_is_disabled_with_no_providers() {
        let cfg = AiConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.chat_provider().is_none());
    }

    #[test]
    fn section_round_trip_through_toml() {
        let cfg = AiConfig {
            enabled: true,
            providers: vec![entry("openai")],
            active_chat: Some("openai".into()),
            ..AiConfig::default()
        };
        let text = toml::to_string(&cfg).expect("serialise");
        let back: AiConfig = toml::from_str(&text).expect("parse");
        assert_eq!(back, cfg);
    }

    #[test]
    fn validation_rejects_bad_ids_urls_and_dangling_references() {
        let mut cfg = AiConfig {
            providers: vec![entry("a b")],
            ..AiConfig::default()
        };
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));

        cfg.providers = vec![entry("ok"), entry("ok")];
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(m)) if m.contains("duplicate")));

        cfg.providers = vec![ProviderEntry {
            base_url: "file:///etc".into(),
            ..entry("x")
        }];
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));

        cfg.providers = vec![entry("x")];
        cfg.active_chat = Some("missing".into());
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(m)) if m.contains("missing")));
    }

    #[test]
    fn active_provider_requires_enabled_flag() {
        let mut cfg = AiConfig {
            enabled: false,
            providers: vec![entry("x")],
            active_chat: Some("x".into()),
            active_embedding: Some("x".into()),
            ..AiConfig::default()
        };
        assert!(cfg.chat_provider().is_none());
        assert!(cfg.embedding_provider().is_none());
        cfg.enabled = true;
        assert_eq!(cfg.chat_provider().map(|p| p.id.as_str()), Some("x"));
        assert_eq!(cfg.embedding_provider().map(|p| p.id.as_str()), Some("x"));
        let chat = cfg.chat_provider().expect("chat").chat_config();
        assert_eq!(chat.model, "chat-1");
        let emb = cfg
            .embedding_provider()
            .expect("emb")
            .embedding_config()
            .expect("model");
        assert_eq!(emb.model, "embed-1");
    }

    #[test]
    fn embedding_provider_needs_a_model() {
        let mut e = entry("x");
        e.embedding_model = None;
        assert!(e.embedding_config().is_none());
        e.embedding_model = Some("  ".into());
        assert!(e.embedding_config().is_none());
        let cfg = AiConfig {
            enabled: true,
            providers: vec![e],
            active_embedding: Some("x".into()),
            ..AiConfig::default()
        };
        assert!(cfg.embedding_provider().is_none());
    }

    #[test]
    fn folder_exclusion_matches_whole_segments() {
        let cfg = AiConfig {
            excluded_folders: vec!["Private/".into(), "a\\b".into()],
            ..AiConfig::default()
        };
        assert!(cfg.is_note_excluded("Private/x.md", &[]));
        assert!(cfg.is_note_excluded("Private", &[]));
        assert!(cfg.is_note_excluded("a/b/c.md", &[]));
        assert!(!cfg.is_note_excluded("privately.md", &[]));
        assert!(!cfg.is_note_excluded("Public/Private/x.md", &[]));
        assert!(!cfg.is_note_excluded("a/bc.md", &[]));
    }

    #[test]
    fn tag_exclusion_is_case_insensitive_and_ignores_hash() {
        let cfg = AiConfig {
            excluded_tags: vec!["#Secret".into()],
            ..AiConfig::default()
        };
        assert!(cfg.is_note_excluded("n.md", &["secret".into()]));
        assert!(cfg.is_note_excluded("n.md", &["#SECRET".into()]));
        assert!(!cfg.is_note_excluded("n.md", &["public".into()]));
        assert!(!cfg.is_note_excluded("n.md", &[]));
    }

    #[test]
    fn empty_excluded_entries_do_not_match_everything() {
        let cfg = AiConfig {
            excluded_folders: vec!["  ".into(), "/".into()],
            excluded_tags: vec!["".into(), "#".into()],
            ..AiConfig::default()
        };
        assert!(!cfg.is_note_excluded("x/y.md", &["tag".into()]));
        assert!(!cfg.is_note_excluded("x/y.md", &[]));
    }
}
