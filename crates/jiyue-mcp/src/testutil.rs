//! Helpers for unit tests: temporary vault folders and app construction.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::app::App;
use crate::config::{Config, DEFAULT_MAX_READ_BYTES};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh folder under the system temp directory, removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("jiyue-mcp-test-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Writes `content` to `rel` under `root`, creating folders as needed.
pub fn write_file(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

/// A test config for `dir`.
pub fn config(dir: &Path, read_only: bool, allow_delete: bool) -> Config {
    Config {
        vault: dir.to_path_buf(),
        read_only,
        allow_delete,
        max_read_bytes: DEFAULT_MAX_READ_BYTES,
    }
}

/// Opens an app on `dir` with the given flags.
pub fn app_for(dir: &Path, read_only: bool, allow_delete: bool) -> App {
    App::open(config(dir, read_only, allow_delete)).expect("open app")
}

/// A vault folder with the given files, and an app on it (writes enabled).
pub fn vault_with(label: &str, files: &[(&str, &str)]) -> (TempDir, App) {
    let dir = TempDir::new(label);
    for (rel, content) in files {
        write_file(dir.path(), rel, content);
    }
    let app = app_for(dir.path(), false, false);
    (dir, app)
}
