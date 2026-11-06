//! Scans the vault folder for files and reads the pixel size of its images from their headers.
//! Hidden folders (`.jiyue`, `.trash`, `.git`) and symbolic links are skipped, so the index only
//! names real files inside the vault.

use std::collections::HashMap;
use std::path::Path;

use jiyue_md::embeds::{EmbedKind, embed_kind};

use super::resolve::VaultFiles;

/// Deepest folder level the scan descends into.
const MAX_DEPTH: usize = 32;

/// Scans `root`. Blocking file system work: call it from a background task.
pub fn scan(root: &Path) -> VaultFiles {
    let mut files = Vec::new();
    walk(root, "", &mut files, 0);
    files.sort();
    let mut dims = HashMap::new();
    for rel in &files {
        if embed_kind(rel) == EmbedKind::Image
            && let Some(size) = image_size(&root.join(rel))
        {
            dims.insert(rel.clone(), size);
        }
    }
    VaultFiles { files, dims }
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let child = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk(&entry.path(), &child, out, depth + 1);
        } else if kind.is_file() {
            out.push(child);
        }
    }
}

/// Width and height of an image, read from its header only. `None` when the file is not an
/// image this build can read.
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1x1 transparent PNG.
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x62, 0x60,
        0x01, 0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0xa5, 0xf6, 0x45, 0x40, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("jiyue-scan-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn scan_lists_files_and_reads_image_sizes() {
        let root = temp_dir("list");
        std::fs::create_dir_all(root.join("附件")).unwrap();
        std::fs::create_dir_all(root.join(".jiyue")).unwrap();
        std::fs::write(root.join("附件").join("pic.png"), PNG_1X1).unwrap();
        std::fs::write(root.join("notes.md"), "# hi").unwrap();
        std::fs::write(root.join("broken.png"), b"not really a png").unwrap();
        std::fs::write(
            root.join(".jiyue").join("config.toml"),
            "version = 1
",
        )
        .unwrap();

        let v = scan(&root);
        assert_eq!(
            v.files,
            vec![
                "broken.png".to_string(),
                "notes.md".to_string(),
                "附件/pic.png".to_string()
            ]
        );
        assert_eq!(v.image_size("附件/pic.png"), Some((1, 1)));
        assert_eq!(v.image_size("broken.png"), None);
        assert!(v.contains("notes.md"));
        assert!(!v.contains(".jiyue/config.toml"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_root_scans_empty() {
        let v = scan(Path::new("Z:/definitely/not/here/jiyue"));
        assert!(v.files.is_empty());
        assert!(v.dims.is_empty());
    }
}
