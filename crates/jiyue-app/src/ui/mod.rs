//! Visual shell of Jiyue: sidebar, tab strip, breadcrumb, editor, right panel, status bar.
//!
//! Every dimension, colour token and font size follows the original HTML mock-up.

pub mod ai_ui;
pub mod assets;
pub mod attach_ui;
pub mod calendar;
pub mod config_ui;
pub mod crumb;
pub mod editor;
pub mod export_ui;
pub mod ext;
pub mod files_ui;
pub mod fonts;
pub mod fonts_ui;
pub mod graph;
pub mod highlight;
pub mod history_ui;
pub mod icons;
pub mod inline;
pub mod keymap;
pub mod keymap_ui;
pub mod md_style;
pub mod nav_history;
pub mod palette;
pub mod right_panel;
pub mod search_ui;
pub mod settings;
pub mod sidebar;
pub mod split;
pub mod split_model;
pub mod statusbar;
pub mod sync_ui;
pub mod tabbar;
pub mod template_ui;
pub mod text_input;
pub mod vaults_ui;
pub mod welcome;
pub mod widgets;

use crate::theme::{ActiveTheme, ThemeKind};

/// Width in px of the right panel (outline, backlinks); the title bar leaves room for it.
pub const RIGHT_PANEL_W: f32 = 272.0;
/// Built-in interface font (installed system font, matched by family name). The user's choice
/// (`[appearance] ui_font`) replaces it; see `fonts::ui_name`.
pub const UI_FONT: &str = "Noto Sans SC";
/// Used by the design when the primary font is missing (listed for reference only).
pub const UI_FONT_FALLBACK: &str = "Microsoft YaHei UI";
/// Monospace font for code blocks and the source view.
pub const MONO_FONT: &str = "JetBrainsMono Nerd Font";
/// Serif font for headings, only in the paper and moss themes.
pub const SERIF_FONT: &str = "Noto Serif SC";

/// Font family for document headings: serif in paper and moss, UI font elsewhere.
pub fn display_family(t: &ActiveTheme) -> &'static str {
    match t.kind {
        ThemeKind::Paper | ThemeKind::Moss => SERIF_FONT,
        _ => UI_FONT,
    }
}

/// Splits a vault-relative path (either separator) into its components.
pub fn path_parts(path: &str) -> Vec<&str> {
    path.split(['\\', '/']).filter(|s| !s.is_empty()).collect()
}

/// Folder key of a note path, for example `想法` or `想法/子目录`. Empty for root notes.
pub fn folder_key(path: &str) -> String {
    let parts = path_parts(path);
    if parts.len() <= 1 {
        String::new()
    } else {
        parts[..parts.len() - 1].join("/")
    }
}

/// Number of Chinese-or-Latin characters, ignoring whitespace (the "字" count in the properties row).
pub fn char_count_no_ws(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}
