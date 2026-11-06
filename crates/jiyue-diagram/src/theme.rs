use serde::{Deserialize, Serialize};

use crate::color::{self, Rgb};

/// Colours and typography applied to rendered diagrams.
///
/// Colours are `#RRGGBB`, or `"transparent"` for the background. Invalid colour
/// strings are replaced by a default for their role, so a bad value can never
/// inject markup into the SVG. `font_family` keeps only letters, digits, spaces
/// and `,-'"._` characters for the same reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagramTheme {
    /// Page colour behind the diagram, or `"transparent"`.
    pub background: String,
    /// Main text colour.
    pub foreground: String,
    /// Secondary text and de-emphasised lines.
    pub muted: String,
    /// Accent colour; drives pie slices, git lanes and activation bars.
    pub accent: String,
    /// Node / box fill.
    pub node_fill: String,
    /// Node / box outline.
    pub node_stroke: String,
    /// Edges and arrows.
    pub line: String,
    /// CSS font-family list. Put CJK faces (e.g. `"Microsoft YaHei"`) after the Latin face.
    pub font_family: String,
    /// Base font size in px (clamped to 6..=48).
    pub font_size: f32,
    /// True when `background` is dark. Selects the base Mermaid theme.
    pub dark: bool,
}

impl DiagramTheme {
    /// Light theme suitable for a white page.
    pub fn light() -> Self {
        Self {
            background: "#FFFFFF".into(),
            foreground: "#1F2937".into(),
            muted: "#6B7280".into(),
            accent: "#2563EB".into(),
            node_fill: "#F8FAFC".into(),
            node_stroke: "#94A3B8".into(),
            line: "#64748B".into(),
            font_family: default_font_family(),
            font_size: 14.0,
            dark: false,
        }
    }

    /// Dark theme suitable for a near-black page.
    pub fn dark() -> Self {
        Self {
            background: "#1E1E1E".into(),
            foreground: "#E5E7EB".into(),
            muted: "#9CA3AF".into(),
            accent: "#60A5FA".into(),
            node_fill: "#2A2D34".into(),
            node_stroke: "#6B7280".into(),
            line: "#9CA3AF".into(),
            font_family: default_font_family(),
            font_size: 14.0,
            dark: true,
        }
    }

    /// Validated copy with every field replaced by a safe value when invalid.
    pub(crate) fn resolve(&self) -> Resolved {
        let dark = self.dark;
        let default_bg = if dark { "#1E1E1E" } else { "#FFFFFF" };
        let default_fg = if dark { "#E5E7EB" } else { "#1F2937" };
        let background = colour_or(&self.background, default_bg);
        Resolved {
            background,
            foreground: colour_or(&self.foreground, default_fg),
            muted: colour_or(&self.muted, if dark { "#9CA3AF" } else { "#6B7280" }),
            accent: colour_or(&self.accent, "#2563EB"),
            node_fill: colour_or(&self.node_fill, if dark { "#2A2D34" } else { "#F8FAFC" }),
            node_stroke: colour_or(&self.node_stroke, "#94A3B8"),
            line: colour_or(&self.line, "#64748B"),
            font_family: sanitize_font_family(&self.font_family),
            font_size: if self.font_size.is_finite() {
                self.font_size.clamp(6.0, 48.0)
            } else {
                14.0
            },
            dark,
        }
    }

    /// Stable text key covering every rendering-relevant field (post-validation).
    pub(crate) fn fingerprint(&self) -> String {
        self.resolve().key()
    }
}

impl Default for DiagramTheme {
    fn default() -> Self {
        Self::light()
    }
}

fn default_font_family() -> String {
    "'Segoe UI', 'Microsoft YaHei', 'Noto Sans SC', Arial, sans-serif".into()
}

/// Theme with validated values. Crate-private; produced by [`DiagramTheme::resolve`].
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub background: String,
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    pub node_fill: String,
    pub node_stroke: String,
    pub line: String,
    pub font_family: String,
    pub font_size: f32,
    pub dark: bool,
}

impl Resolved {
    pub(crate) fn is_transparent_background(&self) -> bool {
        self.background == "transparent"
    }

    /// Mermaid theme with every colour role mapped from the diagram theme.
    pub(crate) fn to_mermaid(&self) -> mermaid_rs_renderer::Theme {
        let mut t = if self.dark {
            mermaid_rs_renderer::Theme::dark()
        } else {
            mermaid_rs_renderer::Theme::modern()
        };
        let transparent = self.is_transparent_background();
        // Colour used where a real RGB is needed for mixing. Transparent pages
        // mix against the default page colour of the chosen mode.
        let page_rgb: Rgb = color::parse_hex(&self.background).unwrap_or(if self.dark {
            (0x1E, 0x1E, 0x1E)
        } else {
            (0xFF, 0xFF, 0xFF)
        });
        let node_rgb = color::parse_hex(&self.node_fill).unwrap_or((0xF8, 0xFA, 0xFC));
        let accent_rgb = color::parse_hex(&self.accent).unwrap_or((0x25, 0x63, 0xEB));
        let muted_rgb = color::parse_hex(&self.muted).unwrap_or((0x6B, 0x72, 0x80));

        t.font_family = self.font_family.clone();
        t.font_size = self.font_size;
        t.background = self.background.clone();
        t.text_color = self.foreground.clone();
        t.primary_text_color = self.foreground.clone();
        t.primary_color = self.node_fill.clone();
        t.secondary_color = color::to_hex(color::mix(node_rgb, muted_rgb, 0.12));
        t.tertiary_color = color::to_hex(color::mix(node_rgb, page_rgb, 0.5));
        t.primary_border_color = self.node_stroke.clone();
        t.line_color = self.line.clone();
        t.edge_label_background = if transparent {
            "none".into()
        } else {
            self.background.clone()
        };
        t.cluster_background = color::to_hex(color::mix(node_rgb, page_rgb, 0.6));
        t.cluster_border = self.muted.clone();

        t.sequence_actor_fill = self.node_fill.clone();
        t.sequence_actor_border = self.node_stroke.clone();
        t.sequence_actor_line = self.muted.clone();
        t.sequence_note_fill = color::to_hex(color::mix(accent_rgb, page_rgb, 0.85));
        t.sequence_note_border = color::to_hex(color::mix(accent_rgb, muted_rgb, 0.3));
        t.sequence_activation_fill = color::to_hex(color::mix(accent_rgb, page_rgb, 0.7));
        t.sequence_activation_border = self.accent.clone();

        let lanes = color::palette(accent_rgb, 8, self.dark);
        t.git_inv_colors = lanes
            .clone()
            .try_into()
            .unwrap_or_else(|_| t.git_colors.clone());
        t.git_branch_label_colors = lanes
            .iter()
            .map(|c| {
                color::parse_hex(c)
                    .map(color::text_on)
                    .unwrap_or_else(|| "#FFFFFF".into())
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or_else(|_| t.git_branch_label_colors.clone());
        t.git_colors = lanes.try_into().unwrap_or_else(|_| t.git_colors.clone());
        t.git_commit_label_color = self.foreground.clone();
        t.git_commit_label_background = self.background.clone();
        t.git_tag_label_color = self.foreground.clone();
        t.git_tag_label_background = self.node_fill.clone();
        t.git_tag_label_border = self.node_stroke.clone();

        let pie = color::palette(accent_rgb, 12, self.dark);
        let pie_text = pie
            .first()
            .and_then(|c| color::parse_hex(c))
            .map(color::text_on)
            .unwrap_or_else(|| "#0B1020".into());
        t.pie_colors = pie.try_into().unwrap_or_else(|_| t.pie_colors.clone());
        t.pie_title_text_color = self.foreground.clone();
        t.pie_title_text_size = (self.font_size * 1.6).clamp(10.0, 72.0);
        t.pie_section_text_color = pie_text;
        t.pie_section_text_size = self.font_size * 1.1;
        t.pie_legend_text_color = self.foreground.clone();
        t.pie_legend_text_size = self.font_size * 1.1;
        t.pie_stroke_color = self.background.clone();
        t.pie_outer_stroke_color = self.node_stroke.clone();
        t.pie_opacity = 0.95;
        t
    }

    /// Replaces colour literals that the renderer hard-codes (timeline cards,
    /// quadrant labels) with tints and text colours derived from this theme.
    ///
    /// Only exact `fill="#RRGGBB"` attributes are rewritten, so user colours from
    /// `style` or `classDef` lines are left alone.
    pub(crate) fn remap_backend_literals(&self, svg: &str) -> String {
        let node = color::parse_hex(&self.node_fill).unwrap_or((0xF8, 0xFA, 0xFC));
        let accent = color::parse_hex(&self.accent).unwrap_or((0x25, 0x63, 0xEB));
        let tones = color::palette(accent, 6, self.dark);
        let tint = |hue: &str| -> String {
            let hue_rgb = color::parse_hex(hue).unwrap_or(accent);
            color::to_hex(color::mix(node, hue_rgb, 0.28))
        };
        let faint = |t: f32| color::to_hex(color::mix(node, accent, t));
        let mut out = svg.to_string();
        let pairs: [(&str, String); 9] = [
            ("ECECFF", tint(&tones[0])),
            ("FFE6CC", tint(&tones[1])),
            ("D5E8D4", tint(&tones[2])),
            ("F8CECC", tint(&tones[3])),
            ("FFF2CC", tint(&tones[4])),
            ("E1D5E7", tint(&tones[5])),
            ("F1F1FF", faint(0.08)),
            ("F6F6FF", faint(0.05)),
            ("FBFBFF", faint(0.03)),
        ];
        for (literal, to) in &pairs {
            out = replace_fill(&out, literal, to);
        }
        replace_fill(&out, "131300", &self.foreground)
    }

    /// Text key used by the cache; equals [`DiagramTheme::fingerprint`] for the same theme.
    pub(crate) fn key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.background,
            self.foreground,
            self.muted,
            self.accent,
            self.node_fill,
            self.node_stroke,
            self.line,
            self.font_family,
            self.font_size.to_bits(),
            self.dark
        )
    }
}

/// Rewrites `fill="#LITERAL"` (upper- or lower-case hex) to `fill="TO"`.
fn replace_fill(svg: &str, literal_hex: &str, to: &str) -> String {
    let upper = format!("fill=\"#{}\"", literal_hex.to_ascii_uppercase());
    let lower = format!("fill=\"#{}\"", literal_hex.to_ascii_lowercase());
    let replacement = format!("fill=\"{to}\"");
    svg.replace(&upper, &replacement)
        .replace(&lower, &replacement)
}

fn colour_or(value: &str, fallback: &str) -> String {
    if value.eq_ignore_ascii_case("transparent") {
        return "transparent".into();
    }
    match color::parse_hex(value) {
        Some(rgb) => color::to_hex(rgb),
        None => fallback.into(),
    }
}

fn sanitize_font_family(value: &str) -> String {
    let cleaned: String = value
        .chars()
        // Double quotes are dropped: the renderer writes this value into
        // double-quoted XML attributes, so a stray quote would break the document.
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | ',' | '-' | '\'' | '.' | '_'))
        .take(200)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Arial, sans-serif".into()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_colours_fall_back_per_role() {
        let t = DiagramTheme {
            background: "javascript:alert(1)".into(),
            foreground: "#12".into(),
            muted: "\"><script>".into(),
            accent: "#2563eb".into(),
            node_fill: "rgb(1,2,3)".into(),
            node_stroke: "transparent".into(),
            line: "#abc".into(),
            font_family: "Arial".into(),
            font_size: 14.0,
            dark: false,
        };
        let r = t.resolve();
        assert_eq!(r.background, "#FFFFFF");
        assert_eq!(r.foreground, "#1F2937");
        assert_eq!(r.muted, "#6B7280");
        assert_eq!(r.accent, "#2563EB");
        assert_eq!(r.node_fill, "#F8FAFC");
        assert_eq!(r.node_stroke, "transparent");
        assert_eq!(r.line, "#64748B");
    }

    #[test]
    fn font_family_strips_markup() {
        let t = DiagramTheme {
            font_family: "Arial\"><style>x</style>, Microsoft YaHei".into(),
            ..DiagramTheme::light()
        };
        let r = t.resolve();
        assert!(!r.font_family.contains('<'));
        assert!(!r.font_family.contains('>'));
        assert!(r.font_family.contains("Microsoft YaHei"));
        let empty = DiagramTheme {
            font_family: "<<<".into(),
            ..DiagramTheme::light()
        };
        assert_eq!(empty.resolve().font_family, "Arial, sans-serif");
    }

    #[test]
    fn font_size_is_clamped_and_finite() {
        let mut t = DiagramTheme::light();
        t.font_size = 1000.0;
        assert_eq!(t.resolve().font_size, 48.0);
        t.font_size = f32::NAN;
        assert_eq!(t.resolve().font_size, 14.0);
        t.font_size = 0.0;
        assert_eq!(t.resolve().font_size, 6.0);
    }

    #[test]
    fn transparent_background_is_kept() {
        let t = DiagramTheme {
            background: "Transparent".into(),
            ..DiagramTheme::dark()
        };
        let r = t.resolve();
        assert!(r.is_transparent_background());
        let m = r.to_mermaid();
        assert_eq!(m.background, "transparent");
        assert_eq!(m.edge_label_background, "none");
    }

    #[test]
    fn mermaid_theme_gets_user_colours() {
        let r = DiagramTheme::dark().resolve();
        let m = r.to_mermaid();
        assert_eq!(m.primary_color, r.node_fill);
        assert_eq!(m.text_color, r.foreground);
        assert_eq!(m.line_color, r.line);
        assert_eq!(m.background, r.background);
        assert_eq!(m.font_family, r.font_family);
        assert_eq!(m.pie_colors.len(), 12);
        assert_eq!(m.git_colors.len(), 8);
    }

    #[test]
    fn fingerprint_changes_with_theme_and_is_stable() {
        let a = DiagramTheme::light();
        let mut b = DiagramTheme::light();
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.accent = "#FF0000".into();
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint(), a.resolve().key());
    }

    #[test]
    fn theme_round_trips_through_serde() {
        let t = DiagramTheme::dark();
        let json = serde_json::to_string(&t).unwrap_or_default();
        assert!(json.contains("#1E1E1E"));
        let back: DiagramTheme =
            serde_json::from_str(&json).unwrap_or_else(|_| DiagramTheme::light());
        assert_eq!(back, t);
    }
}
