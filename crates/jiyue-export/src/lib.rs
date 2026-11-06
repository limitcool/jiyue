//! Templates for new notes, and HTML export of notes and whole vaults.
//!
//! - [`templates`]: variable expansion (`{{date}}`, `{{title}}`, `{{cursor}}` ...), template
//!   listing and per-folder default template rules.
//! - [`html`]: Markdown with Jiyue syntax (wikilinks, embeds, callouts, highlights) to a
//!   standalone HTML page, and a static site exporter for a whole vault.
//!
//! Nothing in this crate reads the clock or the environment on its own; callers pass the
//! current time, clipboard text and random seeds in.

pub mod html;
pub mod templates;

pub use html::{
    AssetRef, ExportCtx, ExportOptions, ExportReport, ExportedPage, FrontmatterView, Heading,
    SiteOptions, Unresolved, export_note_html, export_vault_site, wrap_document,
};
pub use templates::{
    Locale, Moment, RenderedTemplate, TemplateCtx, TemplateInfo, TemplateRules, list_templates,
    render_template,
};

#[cfg(test)]
pub(crate) mod test_util {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A fresh, empty directory under the system temp dir, unique per process and call.
    pub fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "jiyue-export-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// Best-effort removal of a temp directory.
    pub fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
