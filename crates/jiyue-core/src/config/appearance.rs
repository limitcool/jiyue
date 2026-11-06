//! Font and size settings of the notes and the interface (`[appearance]` in the user's
//! `config.toml`). Pure data and pure rules, no gpui: the app turns these values into the
//! resolved font settings (see `ui/fonts.rs`).

use serde::{Deserialize, Serialize};

/// Editor (body text) font size in px: default, and the allowed range.
pub const EDITOR_SIZE_DEFAULT: f32 = 15.0;
pub const EDITOR_SIZE_RANGE: (f32, f32) = (10.0, 36.0);
/// Code font size in px. `0` means "follow the editor" (90 % of the editor size).
pub const CODE_SIZE_FOLLOW: f32 = 0.0;
pub const CODE_SIZE_RANGE: (f32, f32) = (9.0, 32.0);
/// Line height as a multiplier of the font size. The default reproduces the historic 26 px rows
/// at a 15 px font.
pub const LINE_HEIGHT_DEFAULT: f32 = 1.73;
pub const LINE_HEIGHT_RANGE: (f32, f32) = (1.0, 2.5);

/// `[appearance]` section. Every key has a default; an empty font name means "the built-in font".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Family of the interface (menus, sidebar, labels). Empty = the built-in font.
    pub ui_font: String,
    /// Family of the editor and of read mode. Empty = the same as `ui_font`.
    pub editor_font: String,
    /// Family of code blocks and inline code. Empty = the built-in monospace font.
    pub code_font: String,
    /// Body text size in px (10 to 36).
    pub editor_font_size: f32,
    /// Code text size in px (9 to 32). `0` follows the editor size.
    pub code_font_size: f32,
    /// Line height multiplier (1.0 to 2.5).
    pub line_height: f32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            ui_font: String::new(),
            editor_font: String::new(),
            code_font: String::new(),
            editor_font_size: EDITOR_SIZE_DEFAULT,
            code_font_size: CODE_SIZE_FOLLOW,
            line_height: LINE_HEIGHT_DEFAULT,
        }
    }
}

impl Appearance {
    /// The same settings with every number inside its range and every font name trimmed. Returns
    /// the clamped value and one warning per number that was out of range (a warning, not an
    /// error: the file stays usable). Non-finite numbers fall back to the default.
    pub fn clamped(&self) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut out = self.clone();
        out.ui_font = self.ui_font.trim().to_string();
        out.editor_font = self.editor_font.trim().to_string();
        out.code_font = self.code_font.trim().to_string();
        out.editor_font_size = clamp_f32(
            self.editor_font_size,
            EDITOR_SIZE_RANGE,
            EDITOR_SIZE_DEFAULT,
            "editor_font_size",
            &mut warnings,
        );
        out.code_font_size = if self.code_font_size == CODE_SIZE_FOLLOW {
            CODE_SIZE_FOLLOW
        } else {
            clamp_f32(
                self.code_font_size,
                CODE_SIZE_RANGE,
                CODE_SIZE_FOLLOW,
                "code_font_size",
                &mut warnings,
            )
        };
        out.line_height = clamp_f32(
            self.line_height,
            LINE_HEIGHT_RANGE,
            LINE_HEIGHT_DEFAULT,
            "line_height",
            &mut warnings,
        );
        (out, warnings)
    }
}

/// Clamps one float into `range`. NaN and infinities give `default`. A value outside the range
/// is clamped and reported.
fn clamp_f32(
    value: f32,
    range: (f32, f32),
    default: f32,
    key: &str,
    warnings: &mut Vec<String>,
) -> f32 {
    if !value.is_finite() {
        warnings.push(format!("appearance.{key} 不是有效的数字，已使用默认值"));
        return default;
    }
    let (lo, hi) = range;
    if value < lo || value > hi {
        warnings.push(format!(
            "appearance.{key} = {value} 超出范围 {lo}..={hi}，已调整"
        ));
        return value.clamp(lo, hi);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_in_range_and_clean() {
        let (clean, warnings) = Appearance::default().clamped();
        assert_eq!(clean, Appearance::default());
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn out_of_range_numbers_are_clamped_with_warnings() {
        let wild = Appearance {
            editor_font_size: 99.0,
            code_font_size: 4.0,
            line_height: 0.2,
            ..Appearance::default()
        };
        let (clean, warnings) = wild.clamped();
        assert_eq!(clean.editor_font_size, 36.0);
        assert_eq!(clean.code_font_size, 9.0);
        assert_eq!(clean.line_height, 1.0);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn code_size_zero_means_follow_and_is_not_clamped() {
        let (clean, warnings) = Appearance::default().clamped();
        assert_eq!(clean.code_font_size, CODE_SIZE_FOLLOW);
        assert!(warnings.is_empty());
    }

    #[test]
    fn non_finite_numbers_fall_back_to_defaults() {
        let bad = Appearance {
            editor_font_size: f32::NAN,
            line_height: f32::INFINITY,
            ..Appearance::default()
        };
        let (clean, warnings) = bad.clamped();
        assert_eq!(clean.editor_font_size, EDITOR_SIZE_DEFAULT);
        assert_eq!(clean.line_height, LINE_HEIGHT_DEFAULT);
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn font_names_are_trimmed() {
        let named = Appearance {
            ui_font: "  Noto Sans SC ".into(),
            ..Appearance::default()
        };
        assert_eq!(named.clamped().0.ui_font, "Noto Sans SC");
    }

    #[test]
    fn missing_keys_keep_their_defaults() {
        let partial: Appearance = toml::from_str("editor_font_size = 18.0").expect("partial");
        assert_eq!(partial.editor_font_size, 18.0);
        assert_eq!(partial.line_height, LINE_HEIGHT_DEFAULT);
        assert!(partial.ui_font.is_empty());
    }

    #[test]
    fn whole_numbers_are_accepted_for_float_keys() {
        let whole: Appearance =
            toml::from_str("editor_font_size = 18\nline_height = 2").expect("integers for floats");
        assert_eq!(whole.editor_font_size, 18.0);
        assert_eq!(whole.line_height, 2.0);
    }

    #[test]
    fn legacy_ui_scale_key_loads_and_is_reported_as_unknown() {
        use crate::config::UserConfig;
        let text = "version = 1\n[appearance]\neditor_font_size = 18.0\nui_scale = 120\n";
        let loaded: crate::config::Loaded<UserConfig> =
            crate::config::doc::load_text(std::path::Path::new("config.toml"), text);
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.value.appearance.editor_font_size, 18.0);
        assert!(
            loaded
                .warnings
                .iter()
                .any(|w| w.contains("appearance.ui_scale")),
            "{:?}",
            loaded.warnings
        );
    }

    #[test]
    fn round_trips_through_toml() {
        let value = Appearance {
            ui_font: "霞鹜文楷".into(),
            editor_font: "Noto Serif SC".into(),
            code_font: "Cascadia Mono".into(),
            editor_font_size: 17.5,
            code_font_size: 14.0,
            line_height: 1.9,
        };
        let text = toml::to_string(&value).expect("serialise");
        let back: Appearance = toml::from_str(&text).expect("parse");
        assert_eq!(back, value);
    }
}
