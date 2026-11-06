//! The vault boundary. Every path a client supplies is checked here before it touches the disk:
//! syntax (`jiyue_core::fsops::safe_rel_path`), ignored folders, symbolic links and junctions
//! on the way, and the final canonical location. Only `.md` and `.txt` files count as notes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiyue_core::fsops::{is_ignored, safe_rel_path};

/// Largest note that is read in full, analysed or written.
pub const MAX_NOTE_BYTES: u64 = 5 * 1024 * 1024;
/// File extensions that count as notes.
pub const NOTE_EXTENSIONS: [&str; 2] = ["md", "txt"];

/// True when the file name of `rel` has a note extension (any case) and a non-empty stem.
pub fn is_note_path(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty() && NOTE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
        }
        None => false,
    }
}

/// MIME type for a note path.
pub fn mime_for(rel: &str) -> &'static str {
    if rel.to_ascii_lowercase().ends_with(".txt") {
        "text/plain"
    } else {
        "text/markdown"
    }
}

/// The canonical vault root. Every checked path lives below it.
#[derive(Debug, Clone)]
pub struct VaultRoot {
    canon: PathBuf,
}

impl VaultRoot {
    /// Opens an existing directory as the vault root.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let canon = fs::canonicalize(dir)
            .map_err(|err| format!("cannot open vault {}: {err}", dir.display()))?;
        if !canon.is_dir() {
            return Err(format!("vault is not a directory: {}", dir.display()));
        }
        Ok(Self { canon })
    }

    /// Absolute, canonical root directory.
    pub fn path(&self) -> &Path {
        &self.canon
    }

    /// Checks a vault-relative path and returns its absolute location. The path need not exist.
    ///
    /// Refused: invalid syntax (`..`, absolute, backslashes, reserved names), anything inside an
    /// ignored or hidden folder, any existing component that is a symbolic link, and anything
    /// whose real location lies outside the root (junctions).
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, String> {
        let parsed = safe_rel_path(rel).map_err(|err| format!("invalid path {rel:?}: {err}"))?;
        if is_ignored(rel) {
            return Err(format!(
                "path {rel:?} is in an ignored folder or is a hidden file"
            ));
        }
        let mut current = self.canon.clone();
        let mut missing = false;
        for part in parsed.components() {
            current.push(part);
            if missing {
                // Below a folder that does not exist yet, nothing can be a link.
                continue;
            }
            match fs::symlink_metadata(&current) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return Err(format!("path {rel:?} goes through a symbolic link"));
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => missing = true,
                Err(err) => return Err(format!("cannot inspect {rel:?}: {err}")),
            }
        }
        if let Ok(real) = fs::canonicalize(&current)
            && !real.starts_with(&self.canon)
        {
            return Err(format!("path {rel:?} leaves the vault"));
        }
        Ok(current)
    }

    /// Like [`VaultRoot::resolve`], and additionally requires a note extension.
    pub fn resolve_note(&self, rel: &str) -> Result<PathBuf, String> {
        if !is_note_path(rel) {
            return Err(format!(
                "{rel:?} is not a note: only .md and .txt files are supported"
            ));
        }
        self.resolve(rel)
    }
}

/// True when something (file or folder) exists at `abs`. Does not follow links.
pub fn exists(abs: &Path) -> bool {
    fs::symlink_metadata(abs).is_ok()
}

/// Reads a note's raw bytes after checking that it is a regular file within the size limit.
pub fn read_note_bytes(abs: &Path, rel: &str) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(abs).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => format!("note not found: {rel}"),
        _ => format!("cannot read {rel}: {err}"),
    })?;
    if !meta.is_file() {
        return Err(format!("{rel} is not a file"));
    }
    if meta.len() > MAX_NOTE_BYTES {
        return Err(format!(
            "{rel} is {} bytes; the limit is {} bytes",
            meta.len(),
            MAX_NOTE_BYTES
        ));
    }
    fs::read(abs).map_err(|err| format!("cannot read {rel}: {err}"))
}

/// Reads a note as UTF-8 text, subject to the same checks as [`read_note_bytes`].
pub fn read_note_text(abs: &Path, rel: &str) -> Result<String, String> {
    let bytes = read_note_bytes(abs, rel)?;
    String::from_utf8(bytes).map_err(|_| format!("{rel} is not valid UTF-8 text"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, write_file};

    fn root(dir: &TempDir) -> VaultRoot {
        VaultRoot::open(dir.path()).expect("open vault")
    }

    #[test]
    fn note_extensions_are_md_and_txt_in_any_case() {
        assert!(is_note_path("a.md"));
        assert!(is_note_path("dir/B.MD"));
        assert!(is_note_path("notes/c.txt"));
        assert!(!is_note_path("image.png"));
        assert!(!is_note_path("canvas.canvas"));
        assert!(!is_note_path(".md"));
        assert!(!is_note_path("README"));
        assert!(!is_note_path("dir.md/file"));
    }

    #[test]
    fn mime_types_follow_the_extension() {
        assert_eq!(mime_for("a.md"), "text/markdown");
        assert_eq!(mime_for("a.TXT"), "text/plain");
    }

    #[test]
    fn plain_relative_paths_resolve_inside_the_root() {
        let dir = TempDir::new("resolve-plain");
        let r = root(&dir);
        let abs = r.resolve("folder/note.md").expect("valid path");
        assert!(abs.starts_with(r.path()));
        assert!(abs.ends_with("note.md"));
    }

    #[test]
    fn traversal_absolute_and_backslash_paths_are_refused() {
        let dir = TempDir::new("resolve-bad");
        let r = root(&dir);
        for bad in [
            "../escape.md",
            "a/../../escape.md",
            "/etc/passwd",
            "C:/Windows/win.ini",
            "..\\escape.md",
            "a\\b.md",
            "",
            "./x.md",
        ] {
            assert!(r.resolve(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn ignored_and_hidden_folders_are_refused() {
        let dir = TempDir::new("resolve-ignored");
        let r = root(&dir);
        for bad in [
            ".jiyue/x.md",
            ".trash/x.md",
            ".obsidian/app.md",
            ".git/config.md",
            "notes/.secret.md",
            "tmp/draft.tmp",
        ] {
            assert!(r.resolve(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn resolve_note_requires_a_note_extension() {
        let dir = TempDir::new("resolve-ext");
        let r = root(&dir);
        assert!(r.resolve_note("pic.png").is_err());
        assert!(r.resolve_note("pic.md").is_ok());
    }

    #[test]
    fn missing_paths_resolve_for_creation() {
        let dir = TempDir::new("resolve-missing");
        let r = root(&dir);
        assert!(r.resolve_note("new/deep/folder/x.md").is_ok());
        assert!(!exists(&r.resolve_note("new/x.md").expect("path")));
    }

    #[test]
    fn symbolic_links_inside_the_vault_are_refused() {
        let dir = TempDir::new("resolve-symlink");
        let outside = TempDir::new("resolve-symlink-target");
        write_file(outside.path(), "secret.md", "secret");
        let r = root(&dir);
        let link = dir.path().join("link");
        if !make_dir_link(outside.path(), &link) {
            // Symbolic links need extra rights on some Windows setups. Nothing to test then.
            return;
        }
        assert!(r.resolve_note("link/secret.md").is_err());
        assert!(r.resolve_note("link").is_err());
    }

    #[cfg(windows)]
    fn make_dir_link(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }

    #[cfg(unix)]
    fn make_dir_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[test]
    fn read_helpers_check_existence_type_and_utf8() {
        let dir = TempDir::new("read-helpers");
        let r = root(&dir);
        write_file(dir.path(), "ok.md", "héllo");
        write_file(dir.path(), "bad.md", "");
        fs::write(dir.path().join("bin.md"), [0xff, 0xfe, 0x00]).expect("write");
        let ok = r.resolve_note("ok.md").expect("path");
        assert_eq!(read_note_text(&ok, "ok.md").expect("text"), "héllo");
        let missing = r.resolve_note("nope.md").expect("path");
        assert!(read_note_text(&missing, "nope.md").is_err());
        let dir_path = dir.path().to_path_buf();
        assert!(read_note_bytes(&dir_path, "dir").is_err());
        let bin = r.resolve_note("bin.md").expect("path");
        assert!(read_note_text(&bin, "bin.md").is_err());
    }
}
