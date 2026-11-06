//! State shared by all handlers: the configuration, the vault root and the catalog.

use crate::catalog::Catalog;
use crate::config::Config;
use crate::vault::VaultRoot;

/// One running server: its settings, the vault boundary and the in-memory catalog.
pub struct App {
    pub config: Config,
    pub root: VaultRoot,
    pub catalog: Catalog,
}

impl App {
    /// Opens the vault and builds the initial catalog.
    pub fn open(config: Config) -> Result<Self, String> {
        let root = VaultRoot::open(&config.vault)?;
        let mut catalog = Catalog::new(root.clone())?;
        catalog.refresh()?;
        Ok(Self {
            config,
            root,
            catalog,
        })
    }
}
