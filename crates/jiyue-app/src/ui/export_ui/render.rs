//! Builds the self-contained HTML page of one note: images are inlined as data URIs, formulas and
//! Mermaid diagrams are drawn to SVG, and the page is styled by `jiyue-export`.

use std::fs;
use std::path::{Path, PathBuf};

use jiyue_export::{ExportCtx, ExportOptions, FrontmatterView, export_note_html};
use jiyue_md::embeds::EmbedKind;

use super::pure::{self, Theme};

/// Largest image inlined into a page. Bigger ones are left as links.
pub const MAX_INLINE_BYTES: u64 = 8 * 1024 * 1024;
/// Most directory entries looked at when a bare file name is searched for.
const SEARCH_BUDGET: usize = 50_000;

/// The vault-relative path of the file a link or embed names, or `None` when it does not exist.
/// Notes (`.md`) are never returned: a single exported page cannot link to other pages. A bare
/// file name is searched for anywhere in the vault, as Obsidian does.
pub fn find_asset(root: &Path, target: &str) -> Option<String> {
    let rel = target.trim().replace('\\', "/");
    if rel.is_empty() || rel.starts_with('/') || rel.split('/').any(|p| p == "..") {
        return None;
    }
    if rel.to_ascii_lowercase().ends_with(".md") {
        return None;
    }
    if root.join(&rel).is_file() {
        return Some(rel);
    }
    if rel.contains('/') {
        return None;
    }
    let mut budget = SEARCH_BUDGET;
    find_by_name(root, "", &rel, &mut budget)
}

/// Depth-first search for a file called `name`, skipping hidden folders. `rel_dir` is relative to
/// the vault root (empty for the root itself).
fn find_by_name(root: &Path, rel_dir: &str, name: &str, budget: &mut usize) -> Option<String> {
    let dir = if rel_dir.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel_dir)
    };
    let mut subdirs = Vec::new();
    for entry in fs::read_dir(&dir).ok()?.flatten() {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_name.starts_with('.') {
            continue;
        }
        let rel = if rel_dir.is_empty() {
            file_name.clone()
        } else {
            format!("{rel_dir}/{file_name}")
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() && file_name == name {
            return Some(rel);
        }
        if kind.is_dir() {
            subdirs.push(rel);
        }
    }
    subdirs
        .iter()
        .find_map(|sub| find_by_name(root, sub, name, budget))
}

/// Reads an image for inlining. `None` when it is missing, too big or unreadable.
fn read_image(path: PathBuf) -> Option<Vec<u8>> {
    let meta = fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_INLINE_BYTES {
        return None;
    }
    fs::read(path).ok()
}

/// The page of a note. `text` is the whole note (frontmatter included).
pub fn note_page(root: &Path, title: &str, text: &str, theme: Theme) -> String {
    let resolve = |target: &str| find_asset(root, target);
    let diagram_theme = match theme {
        Theme::Paper => jiyue_diagram::DiagramTheme::light(),
        Theme::Ink => jiyue_diagram::DiagramTheme::dark(),
    };
    let mermaid = |source: &str| {
        jiyue_diagram::render_mermaid(source, &diagram_theme)
            .ok()
            .map(|r| r.svg)
    };
    let math_theme = jiyue_math::MathTheme {
        color: theme.math_color().to_string(),
        font_size: 16.0,
    };
    let math = |latex: &str, display: bool| {
        jiyue_math::render_math(latex, display, &math_theme)
            .ok()
            .map(|r| r.svg)
    };

    let mut ctx = ExportCtx::new(title, &resolve);
    ctx.options = ExportOptions {
        allow_raw_html: false,
        toc: true,
        frontmatter: FrontmatterView::Table,
    };
    ctx.render_mermaid = Some(&mermaid);
    ctx.render_math = Some(&math);
    let page = export_note_html(text, &ctx);

    let mut html = pure::with_theme(&page.html, theme);
    for asset in &page.assets {
        if asset.kind != EmbedKind::Image {
            continue;
        }
        let Some(mime) = pure::image_mime(&asset.href) else {
            continue;
        };
        let Some(bytes) = read_image(root.join(&asset.href)) else {
            continue;
        };
        html = pure::inline_src(&html, &asset.href, &pure::data_uri(mime, &bytes));
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("jiyue-export-ui-{tag}-{nanos}"));
        fs::create_dir_all(root.join("附件")).expect("dirs");
        fs::create_dir_all(root.join(".jiyue")).expect("hidden");
        fs::write(root.join("附件").join("图.png"), [0x89, b'P', b'N', b'G']).expect("png");
        fs::write(root.join(".jiyue").join("hidden.png"), b"x").expect("hidden png");
        root
    }

    #[test]
    fn finds_files_by_path_and_by_bare_name() {
        let root = temp_vault("find");
        assert_eq!(
            find_asset(&root, "附件/图.png").as_deref(),
            Some("附件/图.png")
        );
        assert_eq!(find_asset(&root, "图.png").as_deref(), Some("附件/图.png"));
        assert_eq!(
            find_asset(&root, "hidden.png"),
            None,
            "hidden folders are skipped"
        );
        assert_eq!(find_asset(&root, "../etc/passwd"), None);
        assert_eq!(find_asset(&root, "/abs.png"), None);
        assert_eq!(find_asset(&root, "笔记.md"), None, "notes are not assets");
        assert_eq!(find_asset(&root, "missing.png"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn image_embed_is_inlined_and_theme_set() {
        let root = temp_vault("page");
        let page = note_page(&root, "测试", "# 标题\n\n![[图.png]]\n", Theme::Ink);
        assert!(page.contains("data-theme=\"dark\""), "theme attribute");
        assert!(page.contains("data:image/png;base64,"), "image inlined");
        assert!(
            !page.contains("src=\"附件/图.png\""),
            "no link left to the file"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_image_stays_as_it_was() {
        let root = temp_vault("missing");
        let page = note_page(&root, "测试", "![[没有.png]]\n", Theme::Paper);
        assert!(page.contains("data-theme=\"light\""));
        assert!(!page.contains("data:image"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn formula_is_drawn_to_svg() {
        let root = temp_vault("math");
        let page = note_page(&root, "公式", "行内 $x^2$ 公式\n", Theme::Paper);
        assert!(page.contains("<svg"), "formula drawn as svg");
        let _ = fs::remove_dir_all(&root);
    }
}
