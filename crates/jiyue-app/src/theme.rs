#![allow(dead_code)]
use gpui::Rgba;

#[path = "theme_tokens.rs"]
pub mod theme_tokens;
pub use theme_tokens::*;

/// Editor typography & behavior defaults.
pub const BASE_FONT_SIZE: f32 = 16.0;
pub const FONT_UI_SMALL: f32 = FONT_BASE;
pub const FONT_UI_MEDIUM: f32 = 15.0;
pub const LINE_HEIGHT_NORMAL: f32 = 1.5;

/// Gap between an icon and its label in rows, menus and buttons.
pub const ICON_GAP: f32 = 8.0;
/// Gap between an icon and its label in compact chips, breadcrumbs and tab-like controls.
pub const ICON_GAP_TIGHT: f32 = 6.0;

pub const TAB_SIZE: usize = 4;
pub const READABLE_LINE_LENGTH: bool = true;
pub const SHOW_LINE_NUMBER: bool = true;
pub const FOLD_HEADING: bool = true;
pub const AUTO_PAIR_MARKDOWN: bool = true;
pub const AUTO_PAIR_BRACKETS: bool = true;
pub const VIM_MODE: bool = false;
pub const LIVE_PREVIEW: bool = true;

/// Packs a colour as 0xRRGGBBAA (same layout as `gpui::rgba`), usable in const context.
pub const fn argb(hex_rrggbbaa: u32) -> Rgba {
    Rgba {
        r: ((hex_rrggbbaa >> 24) & 0xff) as f32 / 255.0,
        g: ((hex_rrggbbaa >> 16) & 0xff) as f32 / 255.0,
        b: ((hex_rrggbbaa >> 8) & 0xff) as f32 / 255.0,
        a: (hex_rrggbbaa & 0xff) as f32 / 255.0,
    }
}

/// The five approved themes, in the order shown in the design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Paper,
    Ink,
    Moss,
    Graphite,
    Fog,
}

impl ThemeKind {
    pub fn all() -> [ThemeKind; 5] {
        [
            ThemeKind::Paper,
            ThemeKind::Ink,
            ThemeKind::Moss,
            ThemeKind::Graphite,
            ThemeKind::Fog,
        ]
    }

    pub fn name_zh(self) -> &'static str {
        match self {
            ThemeKind::Paper => "纸",
            ThemeKind::Ink => "墨",
            ThemeKind::Moss => "苔",
            ThemeKind::Graphite => "石墨",
            ThemeKind::Fog => "雾",
        }
    }

    pub fn localized_name(self) -> String {
        match self {
            ThemeKind::Paper => crate::t!("theme-paper"),
            ThemeKind::Ink => crate::t!("theme-ink"),
            ThemeKind::Moss => crate::t!("theme-moss"),
            ThemeKind::Graphite => crate::t!("theme-graphite"),
            ThemeKind::Fog => crate::t!("theme-fog"),
        }
    }

    /// Parses a command line theme name (`ink`, `fog`, `paper`, `moss`, `graphite`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "paper" => Some(ThemeKind::Paper),
            "ink" => Some(ThemeKind::Ink),
            "moss" => Some(ThemeKind::Moss),
            "graphite" => Some(ThemeKind::Graphite),
            "fog" => Some(ThemeKind::Fog),
            _ => None,
        }
    }

    /// The next theme in `ThemeKind::all()` order (wraps around).
    pub fn next(self) -> Self {
        let all = Self::all();
        let pos = all.iter().position(|&k| k == self).unwrap_or(0);
        all[(pos + 1) % all.len()]
    }

    pub fn is_dark(self) -> bool {
        matches!(self, ThemeKind::Ink | ThemeKind::Moss | ThemeKind::Graphite)
    }

    fn tokens(self) -> &'static SsotTokens {
        match self {
            ThemeKind::Paper => &PAPER,
            ThemeKind::Ink => &INK,
            ThemeKind::Moss => &MOSS,
            ThemeKind::Graphite => &GRAPHITE,
            ThemeKind::Fog => &FOG,
        }
    }
}

static CURRENT_THEME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);

/// Records the active window theme globally so newly created controls stay in sync.
pub fn set_current_theme(kind: ThemeKind) {
    let code = match kind {
        ThemeKind::Paper => 0,
        ThemeKind::Ink => 1,
        ThemeKind::Moss => 2,
        ThemeKind::Graphite => 3,
        ThemeKind::Fog => 4,
    };
    CURRENT_THEME.store(code, std::sync::atomic::Ordering::Relaxed);
}

/// Returns the active theme recorded by the window.
pub fn current_theme() -> ActiveTheme {
    let kind = match CURRENT_THEME.load(std::sync::atomic::Ordering::Relaxed) {
        0 => ThemeKind::Paper,
        2 => ThemeKind::Moss,
        3 => ThemeKind::Graphite,
        4 => ThemeKind::Fog,
        _ => ThemeKind::Ink,
    };
    ActiveTheme::from_kind(kind)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

/// The active theme. Exposes both the unified SSOT semantic tokens and backward-compatible
/// short field names (`chrome`/`sheet`/`raised`/`line`/`tx`/`ac`) for existing call sites.
#[derive(Debug, Clone, Copy)]
pub struct ActiveTheme {
    pub kind: ThemeKind,
    pub mode: ThemeMode,

    // Legacy / short aliases
    pub chrome: Rgba,
    pub sheet: Rgba,
    pub raised: Rgba,
    pub hover: Rgba,
    pub active: Rgba,
    pub line: Rgba,
    pub line_soft: Rgba,
    pub tx: Rgba,
    pub tx2: Rgba,
    pub tx3: Rgba,
    pub ac: Rgba,
    pub ac_hover: Rgba,
    pub ac_soft: Rgba,
    pub ac_on: Rgba,
    pub ac_line: Rgba,
    pub sel: Rgba,
    pub ok: Rgba,
    pub ok_soft: Rgba,
    pub warn: Rgba,
    pub warn_soft: Rgba,
    pub danger: Rgba,
    pub danger_soft: Rgba,
    pub kw: Rgba,
    pub st: Rgba,
    pub func: Rgba,
    pub cm: Rgba,
    pub ty: Rgba,

    // SSOT semantic slots
    pub canvas: Rgba,
    pub panel: Rgba,
    pub surface: Rgba,
    pub elevated: Rgba,
    pub selected: Rgba,
    pub scrim: Rgba,
    pub border_subtle: Rgba,
    pub border_default: Rgba,
    pub border_strong: Rgba,
    pub border_focus: Rgba,
    pub fg_primary: Rgba,
    pub fg_secondary: Rgba,
    pub fg_muted: Rgba,
    pub fg_disabled: Rgba,

    pub base_00: u32,
    pub base_05: u32,
    pub base_10: u32,
    pub base_20: u32,
    pub base_25: u32,
    pub base_30: u32,
    pub base_35: u32,
    pub base_50: u32,
    pub base_70: u32,
    pub base_100: u32,
    pub accent: u32,
}

/// Composites a (possibly translucent) foreground over an opaque background.
fn over(fg: Rgba, bg: Rgba) -> Rgba {
    Rgba {
        r: fg.r * fg.a + bg.r * (1.0 - fg.a),
        g: fg.g * fg.a + bg.g * (1.0 - fg.a),
        b: fg.b * fg.a + bg.b * (1.0 - fg.a),
        a: 1.0,
    }
}

/// Flattens an opaque-equivalent colour to 0xRRGGBB.
fn to_rgb_u32(c: Rgba) -> u32 {
    let ch = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u32;
    (ch(c.r) << 16) | (ch(c.g) << 8) | ch(c.b)
}

impl ActiveTheme {
    pub fn from_kind(kind: ThemeKind) -> Self {
        let p = kind.tokens();
        let c = argb;
        let canvas = c(p.canvas);
        let panel = c(p.panel);
        let surface = c(p.surface);
        let elevated = c(p.elevated);
        let hover = c(p.hover);
        let active = c(p.active);
        let selected = c(p.selected);
        let scrim = c(p.scrim);
        let border_subtle = c(p.border_subtle);
        let border_default = c(p.border_default);
        let border_strong = c(p.border_strong);
        let border_focus = c(p.border_focus);
        let fg_primary = c(p.fg_primary);
        let fg_secondary = c(p.fg_secondary);
        let fg_muted = c(p.fg_muted);
        let fg_disabled = c(p.fg_disabled);
        let ac = c(p.accent_default);
        let ac_hover = c(p.accent_hover);
        let ac_soft = c(p.accent_soft);
        let ac_on = c(p.accent_on_accent);
        let ac_line = Rgba {
            a: 0x47 as f32 / 255.0,
            ..ac
        };

        Self {
            kind,
            mode: if kind.is_dark() {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            chrome: canvas,
            sheet: surface,
            raised: elevated,
            hover,
            active,
            line: border_default,
            line_soft: border_subtle,
            tx: fg_primary,
            tx2: fg_secondary,
            tx3: fg_muted,
            ac,
            ac_hover,
            ac_soft,
            ac_on,
            ac_line,
            sel: selected,
            ok: c(p.status_ok),
            ok_soft: c(p.status_ok_soft),
            warn: c(p.status_warn),
            warn_soft: c(p.status_warn_soft),
            danger: c(p.status_danger),
            danger_soft: c(p.status_danger_soft),
            kw: c(p.syntax_keyword),
            st: c(p.syntax_string),
            func: c(p.syntax_function),
            cm: c(p.syntax_comment),
            ty: c(p.syntax_type),
            canvas,
            panel,
            surface,
            elevated,
            selected,
            scrim,
            border_subtle,
            border_default,
            border_strong,
            border_focus,
            fg_primary,
            fg_secondary,
            fg_muted,
            fg_disabled,
            base_00: to_rgb_u32(surface),
            base_05: to_rgb_u32(elevated),
            base_10: to_rgb_u32(over(active, surface)),
            base_20: to_rgb_u32(panel),
            base_25: to_rgb_u32(over(hover, panel)),
            base_30: to_rgb_u32(over(border_default, surface)),
            base_35: to_rgb_u32(over(hover, surface)),
            base_50: to_rgb_u32(fg_muted),
            base_70: to_rgb_u32(fg_secondary),
            base_100: to_rgb_u32(fg_primary),
            accent: to_rgb_u32(ac),
        }
    }

    /// Dark default (Ink).
    pub fn dark() -> Self {
        Self::from_kind(ThemeKind::Ink)
    }

    /// Light default (Fog).
    pub fn light() -> Self {
        Self::from_kind(ThemeKind::Fog)
    }

    #[inline]
    pub fn bg_editor(&self) -> Rgba {
        self.surface
    }
    #[inline]
    pub fn bg_sidebar(&self) -> Rgba {
        self.panel
    }
    #[inline]
    pub fn bg_card(&self) -> Rgba {
        self.elevated
    }
    #[inline]
    pub fn bg_hover(&self) -> Rgba {
        self.hover
    }
    #[inline]
    pub fn border(&self) -> Rgba {
        self.border_default
    }
    #[inline]
    pub fn text(&self) -> Rgba {
        self.fg_primary
    }
    #[inline]
    pub fn text_muted(&self) -> Rgba {
        self.fg_secondary
    }
    #[inline]
    pub fn text_faint(&self) -> Rgba {
        self.fg_muted
    }
    #[inline]
    pub fn accent_color(&self) -> Rgba {
        self.ac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of an opaque colour.
    fn luminance(c: Rgba) -> f32 {
        let lin = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    /// WCAG contrast ratio of `fg` over the theme's sheet colour (translucent fg is blended first).
    fn contrast_on_sheet(fg: Rgba, sheet: Rgba) -> f32 {
        let a = luminance(over(fg, sheet));
        let b = luminance(sheet);
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn radius_tokens_are_ordered_and_icon_gaps_are_shared() {
        const { assert!(RADIUS_S < RADIUS_M && RADIUS_M < RADIUS_L && RADIUS_L < RADIUS_PILL) };
        assert_eq!(
            (RADIUS_S, RADIUS_M, RADIUS_L, RADIUS_XL),
            (4.0, 6.0, 10.0, 12.0)
        );
        assert_eq!(
            (
                ACTIVITY_BAR_WIDTH,
                SIDEBAR_WIDTH,
                RIGHT_PANEL_WIDTH,
                HEADER_HEIGHT,
                STATUSBAR_HEIGHT,
                ROW_HEIGHT
            ),
            (42.0, 236.0, 264.0, 38.0, 24.0, 28.0)
        );
        const { assert!(ICON_GAP_TIGHT < ICON_GAP) };
        assert_eq!(ICON_SIZE, 16.0);
    }

    #[test]
    fn every_theme_builds() {
        for kind in ThemeKind::all() {
            let t = ActiveTheme::from_kind(kind);
            assert_eq!(t.kind, kind);
            assert!(!kind.name_zh().is_empty());
            let expected = if kind.is_dark() {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            };
            assert_eq!(t.mode, expected, "mode mismatch for {:?}", kind);
            assert!(t.tx.a > 0.99, "text token must be opaque for {:?}", kind);
            assert!(t.warn.a > 0.99, "warn token must be opaque for {:?}", kind);
            assert!(
                t.danger.a > 0.99,
                "danger token must be opaque for {:?}",
                kind
            );
            assert!(
                t.fg_disabled.a > 0.99,
                "disabled token must be opaque for {:?}",
                kind
            );
            assert!(
                t.scrim.a > 0.2,
                "scrim token must be non-empty for {:?}",
                kind
            );
        }
        assert_eq!(ActiveTheme::dark().kind, ThemeKind::Ink);
        assert_eq!(ActiveTheme::light().kind, ThemeKind::Fog);
    }

    #[test]
    fn text_contrast_on_sheet() {
        for kind in ThemeKind::all() {
            let t = ActiveTheme::from_kind(kind);
            let tx = contrast_on_sheet(t.tx, t.sheet);
            let tx2 = contrast_on_sheet(t.tx2, t.sheet);
            let tx3 = contrast_on_sheet(t.tx3, t.sheet);
            let ac = contrast_on_sheet(t.ac, t.sheet);
            println!(
                "{} ({:?}): tx {:.2}  tx2 {:.2}  tx3 {:.2}  ac {:.2}",
                kind.name_zh(),
                kind,
                tx,
                tx2,
                tx3,
                ac
            );
            assert!(tx >= 7.0, "tx on sheet {:.2} < 7.0 for {:?}", tx, kind);
            assert!(tx2 >= 4.5, "tx2 on sheet {:.2} < 4.5 for {:?}", tx2, kind);
            assert!(ac >= 3.0, "ac on sheet {:.2} < 3.0 for {:?}", ac, kind);
        }
    }
}
