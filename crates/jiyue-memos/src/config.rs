//! Configuration definitions for Memos integration.

use serde::{Deserialize, Serialize};

/// Memos integration configuration stored in vault's `.jiyue/config.toml` under `[memos]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemosConfig {
    /// Whether Memos integration is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// The base URL of the Memos instance (e.g. `https://memos.example.com`).
    #[serde(default)]
    pub base_url: String,
    /// Background auto-refresh interval in seconds (0 = manual refresh only).
    #[serde(default = "default_interval")]
    pub sync_interval_secs: u64,
    /// Optional tag filter (e.g. `todo`, `#todo`, `inbox`).
    #[serde(default)]
    pub filter_tag: Option<String>,
}

fn default_interval() -> u64 {
    300
}

impl Default for MemosConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            sync_interval_secs: default_interval(),
            filter_tag: None,
        }
    }
}
