//! Pure rules for attachments and embeds: vault-relative resolution, display sizes, caption text
//! and embed depth and cycle checks. No gpui and no filesystem access in this file.

use std::collections::HashMap;

/// Most nested note embeds rendered inside one another (`![[a]]` inside `![[b]]` inside `![[c]]`).
pub const EMBED_MAX_DEPTH: u8 = 3;

/// Widest an image may be drawn in the 620 px document column.
pub const COLUMN_WIDTH: f32 = 620.0;

/// The vault's files and the pixel size of its images. Paths are vault-relative with `/`
/// separators; `files` is sorted so lookups can use binary search.
#[derive(Debug, Default, Clone)]
pub struct VaultFiles {
    pub files: Vec<String>,
    pub dims: HashMap<String, (u32, u32)>,
}

/// True for a vault-relative path that stays inside the vault: no absolute or drive prefix,
/// no backslash, and no empty, `.` or `..` segment.
pub fn is_safe_rel(rel: &str) -> bool {
    if rel.is_empty() || rel.contains('\\') || rel.contains(':') || rel.starts_with('/') {
        return false;
    }
    rel.split('/')
        .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// The folder part of a vault-relative path (empty for files in the root).
pub fn dir_of(note_path: &str) -> &str {
    note_path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The last path segment.
pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `dir/target` as a vault-relative path, or `None` when the result would leave the vault.
pub fn join_rel(dir: &str, target: &str) -> Option<String> {
    let target = target.trim_start_matches("./");
    let joined = if dir.is_empty() {
        target.to_string()
    } else {
        format!("{dir}/{target}")
    };
    is_safe_rel(&joined).then_some(joined)
}

impl VaultFiles {
    /// True when `rel` is a file of the vault.
    pub fn contains(&self, rel: &str) -> bool {
        self.files.binary_search_by(|f| f.as_str().cmp(rel)).is_ok()
    }

    /// Finds the file a link target names, as seen from the note `note_path`.
    ///
    /// Order: relative to the note's folder, then vault-relative, then by file name anywhere in
    /// the vault (a copy inside `attach_folder` wins, then the shallowest path). Targets that
    /// would leave the vault resolve to `None`. The result is always a file of the index.
    pub fn resolve(
        &self,
        note_path: &str,
        target: &str,
        attach_folder: Option<&str>,
    ) -> Option<String> {
        let target = target.trim().trim_start_matches("./");
        if !is_safe_rel(target) {
            return None;
        }
        if let Some(rel) = join_rel(dir_of(note_path), target)
            && self.contains(&rel)
        {
            return Some(rel);
        }
        if self.contains(target) {
            return Some(target.to_string());
        }
        let name = basename(target);
        let hits: Vec<&String> = self.files.iter().filter(|f| basename(f) == name).collect();
        if hits.is_empty() {
            return None;
        }
        if let Some(folder) = attach_folder.filter(|f| !f.is_empty()) {
            let prefix = format!("{folder}/");
            if let Some(hit) = hits.iter().find(|f| f.starts_with(&prefix)) {
                return Some((*hit).clone());
            }
        }
        hits.iter()
            .min_by_key(|f| (f.matches('/').count(), f.len()))
            .map(|f| (*f).clone())
    }

    /// Pixel size of an image of the index, if it could be read.
    pub fn image_size(&self, rel: &str) -> Option<(u32, u32)> {
        self.dims.get(rel).copied()
    }
}

/// Size shown for an image: `natural` pixels, scaled down to fit inside the optional box from
/// `|300` (width) or `|300x200` (width and height), and inside `column`. Images are never
/// enlarged. Returns `None` for an image with a zero dimension.
pub fn display_size(
    natural: (u32, u32),
    size: Option<(u32, Option<u32>)>,
    column: f32,
) -> Option<(f32, f32)> {
    if natural.0 == 0 || natural.1 == 0 || column <= 0.0 {
        return None;
    }
    let (nw, nh) = (natural.0 as f32, natural.1 as f32);
    let mut scale = (column / nw).min(1.0);
    if let Some((w, h)) = size {
        if w > 0 {
            scale = scale.min(w as f32 / nw);
        }
        if let Some(h) = h.filter(|&h| h > 0) {
            scale = scale.min(h as f32 / nh);
        }
    }
    Some((nw * scale, nh * scale))
}

/// True for `300` or `300x200`, which are sizes and never captions.
fn is_size_text(s: &str) -> bool {
    !s.is_empty()
        && s.split('x')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// The caption shown under an image: its alt text or wiki alias, trimmed. Size-only text
/// (`300`, `300x200`) and blank text give no caption.
pub fn caption(alias: Option<&str>) -> Option<String> {
    let text = alias?.trim();
    if text.is_empty() || is_size_text(text) {
        None
    } else {
        Some(text.to_string())
    }
}

/// Outcome of asking whether a note may be embedded at the current nesting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedGuard {
    Ok,
    /// The note is already being shown further out (a cycle).
    Cycle,
    /// Embedding would go deeper than `EMBED_MAX_DEPTH`.
    TooDeep,
}

/// Decides whether note `note_path` may be embedded at `depth` (0 for the note shown in the
/// editor). `stack` holds the paths of the notes whose embeds are being expanded, outermost first.
pub fn embed_guard(depth: u8, stack: &[String], note_path: &str) -> EmbedGuard {
    if stack.iter().any(|p| p == note_path) {
        EmbedGuard::Cycle
    } else if depth >= EMBED_MAX_DEPTH {
        EmbedGuard::TooDeep
    } else {
        EmbedGuard::Ok
    }
}

/// Zero-based line index of byte `offset` in `text`.
pub fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset).map_or(0, |s| s.matches('\n').count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(files: &[&str]) -> VaultFiles {
        let mut v: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        v.sort();
        VaultFiles {
            files: v,
            dims: HashMap::new(),
        }
    }

    #[test]
    fn safe_paths_stay_inside_the_vault() {
        assert!(is_safe_rel("附件/a.png"));
        assert!(is_safe_rel("a.png"));
        assert!(!is_safe_rel(""));
        assert!(!is_safe_rel("../secret.txt"));
        assert!(!is_safe_rel("a/../../b"));
        assert!(!is_safe_rel("/etc/passwd"));
        assert!(!is_safe_rel("C:/Windows/win.ini"));
        assert!(!is_safe_rel("a\\b.png"));
        assert!(!is_safe_rel("a//b.png"));
        assert!(!is_safe_rel("./a.png"));
    }

    #[test]
    fn join_rejects_traversal() {
        assert_eq!(join_rel("想法", "a.png"), Some("想法/a.png".to_string()));
        assert_eq!(join_rel("", "a.png"), Some("a.png".to_string()));
        assert_eq!(join_rel("想法", "../../x.png"), None);
        assert_eq!(join_rel("想法", "./a.png"), Some("想法/a.png".to_string()));
    }

    #[test]
    fn resolves_relative_then_root_then_by_name() {
        let v = vault(&[
            "附件/a.png",
            "想法/b.png",
            "其他/b.png",
            "根.png",
            "深/x/c.png",
            "深/c.png",
        ]);
        // Same folder as the note wins over other folders.
        assert_eq!(
            v.resolve("想法/笔记.md", "b.png", None),
            Some("想法/b.png".to_string())
        );
        // Vault-relative path.
        assert_eq!(
            v.resolve("想法/笔记.md", "附件/a.png", None),
            Some("附件/a.png".to_string())
        );
        // Name only, found in the attachment folder.
        assert_eq!(
            v.resolve("想法/笔记.md", "a.png", Some("附件")),
            Some("附件/a.png".to_string())
        );
        // Name only from another folder, shallowest path first.
        assert_eq!(
            v.resolve("别处/笔记.md", "c.png", None),
            Some("深/c.png".to_string())
        );
        assert_eq!(
            v.resolve("别处/笔记.md", "根.png", None),
            Some("根.png".to_string())
        );
        // Missing and unsafe targets.
        assert_eq!(v.resolve("想法/笔记.md", "missing.png", None), None);
        assert_eq!(v.resolve("想法/笔记.md", "../../etc/passwd", None), None);
        assert_eq!(v.resolve("想法/笔记.md", "", None), None);
        assert_eq!(v.resolve("想法/笔记.md", "/abs.png", None), None);
    }

    #[test]
    fn display_size_fits_the_box_and_never_upscales() {
        // No size: full natural size.
        assert_eq!(display_size((400, 200), None, 620.0), Some((400.0, 200.0)));
        // Wider than the column: scaled to the column, aspect kept.
        assert_eq!(display_size((1240, 620), None, 620.0), Some((620.0, 310.0)));
        // Width only.
        assert_eq!(
            display_size((1000, 500), Some((300, None)), 620.0),
            Some((300.0, 150.0))
        );
        // Width and height box: the tighter side wins, aspect kept.
        assert_eq!(
            display_size((1000, 500), Some((300, Some(200))), 620.0),
            Some((300.0, 150.0))
        );
        assert_eq!(
            display_size((1000, 1000), Some((300, Some(200))), 620.0),
            Some((200.0, 200.0))
        );
        // A requested size bigger than the image does not upscale.
        assert_eq!(
            display_size((100, 50), Some((300, None)), 620.0),
            Some((100.0, 50.0))
        );
        // Zero dimensions cannot be shown.
        assert_eq!(display_size((0, 50), None, 620.0), None);
    }

    #[test]
    fn captions_skip_size_text() {
        assert_eq!(caption(Some(" 图注 ")), Some("图注".to_string()));
        assert_eq!(caption(Some("300")), None);
        assert_eq!(caption(Some("300x200")), None);
        assert_eq!(caption(Some("   ")), None);
        assert_eq!(caption(None), None);
        // Text that merely contains digits is a caption.
        assert_eq!(caption(Some("图 1")), Some("图 1".to_string()));
    }

    #[test]
    fn embed_guard_detects_cycles_and_depth() {
        let stack = vec!["A.md".to_string(), "B.md".to_string()];
        assert_eq!(embed_guard(2, &stack, "A.md"), EmbedGuard::Cycle);
        assert_eq!(embed_guard(2, &stack, "C.md"), EmbedGuard::Ok);
        assert_eq!(
            embed_guard(EMBED_MAX_DEPTH, &stack, "C.md"),
            EmbedGuard::TooDeep
        );
        // A cycle is reported even when the depth is also exceeded.
        assert_eq!(
            embed_guard(EMBED_MAX_DEPTH, &stack, "A.md"),
            EmbedGuard::Cycle
        );
        assert_eq!(embed_guard(0, &[], "A.md"), EmbedGuard::Ok);
    }

    #[test]
    fn line_of_counts_newlines() {
        let text = "a\nb\nc";
        assert_eq!(line_of(text, 0), 0);
        assert_eq!(line_of(text, 2), 1);
        assert_eq!(line_of(text, 4), 2);
        assert_eq!(line_of(text, 99), 0);
    }
}
