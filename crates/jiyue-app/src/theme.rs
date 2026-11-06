#![allow(dead_code)]
use gpui::Rgba;

/// Theme and mechanics constants directly extracted from obsidian.asar
pub const BASE_FONT_SIZE: f32 = 16.0;
pub const FONT_UI_SMALL: f32 = 13.0;
pub const FONT_UI_MEDIUM: f32 = 15.0;
pub const LINE_HEIGHT_NORMAL: f32 = 1.5;
pub const HEADER_HEIGHT: f32 = 40.0;
pub const SIDEBAR_WIDTH: f32 = 240.0;
/// Corner radii, one set for the whole app. Small: rows, chips, key caps, menu items, tooltips.
/// Medium: buttons, inputs, selects, segmented controls. Large: cards, dialogs, menus, popups.
/// Pill: fully rounded chips and switches. Every rounded element uses one of these four values;
/// a container that paints a background and a child that paints a background must use the same
/// radius on the corners the child reaches (gpui clips children to the rectangle only).
pub const RADIUS_S: f32 = 6.0;
pub const RADIUS_M: f32 = 8.0;
pub const RADIUS_L: f32 = 12.0;
pub const RADIUS_PILL: f32 = 999.0;

/// Icon size used in rows, buttons and menus.
pub const ICON_SIZE: f32 = 16.0;
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

/// Raw design tokens for one theme, as 0xRRGGBBAA values.
/// Field names mirror the CSS variables in the original HTML mock-up.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub chrome: u32,
    pub sheet: u32,
    pub raised: u32,
    pub hover: u32,
    pub active: u32,
    pub line: u32,
    pub line_soft: u32,
    pub tx: u32,
    pub tx2: u32,
    pub tx3: u32,
    pub ac: u32,
    pub ac_soft: u32,
    pub ac_on: u32,
    pub ac_line: u32,
    pub sel: u32,
    pub ok: u32,
    pub ok_soft: u32,
    pub kw: u32,
    pub st: u32,
    pub func: u32,
    pub cm: u32,
    pub ty: u32,
}

/// Paper palette (values converted from the original HTML mock-up).
pub const PAPER: Tokens = Tokens {
    chrome: 0xEFEAE1FF,    // #efeae1
    sheet: 0xFDFBF7FF,     // #fdfbf7
    raised: 0xFFFFFFFF,    // #ffffff
    hover: 0x2B28230E,     // rgba(43,40,35,.055)
    active: 0x2B282317,    // rgba(43,40,35,.09)
    line: 0x2B28231A,      // rgba(43,40,35,.10)
    line_soft: 0x2B28230F, // rgba(43,40,35,.06)
    tx: 0x2B2823FF,        // #2b2823
    tx2: 0x5F5A50FF,       // #5f5a50
    tx3: 0x736D63FF,       // #736d63
    ac: 0x1B7566FF,        // #1b7566
    ac_soft: 0x1B75661A,   // rgba(27,117,102,.10)
    ac_on: 0xFFFFFFFF,     // #ffffff
    ac_line: 0x1B756647,   // rgba(27,117,102,.28)
    sel: 0x1B756633,       // rgba(27,117,102,.20)
    ok: 0x2F7D4FFF,        // #2f7d4f
    ok_soft: 0x2F7D4F1F,   // rgba(47,125,79,.12)
    kw: 0x1B7566FF,        // #1b7566
    st: 0xA8581AFF,        // #a8581a
    func: 0x2F5F9EFF,      // #2f5f9e
    cm: 0x8B857AFF,        // #8b857a
    ty: 0x8A5A2CFF,        // #8a5a2c
};

/// Ink palette (values converted from the original HTML mock-up).
pub const INK: Tokens = Tokens {
    chrome: 0x070A10FF,    // #070a10
    sheet: 0x0E131BFF,     // #0e131b
    raised: 0x151B25FF,    // #151b25
    hover: 0xAAC8FF0F,     // rgba(170,200,255,.06)
    active: 0xAAC8FF1A,    // rgba(170,200,255,.10)
    line: 0xA0BEFF1A,      // rgba(160,190,255,.10)
    line_soft: 0xA0BEFF0F, // rgba(160,190,255,.06)
    tx: 0xE2E8F1FF,        // #e2e8f1
    tx2: 0x9BA8BBFF,       // #9ba8bb
    tx3: 0x7A889DFF,       // #7a889d
    ac: 0x6BB2FFFF,        // #6bb2ff
    ac_soft: 0x6BB2FF24,   // rgba(107,178,255,.14)
    ac_on: 0x05162BFF,     // #05162b
    ac_line: 0x6BB2FF5C,   // rgba(107,178,255,.36)
    sel: 0x6BB2FF47,       // rgba(107,178,255,.28)
    ok: 0x5FCF8FFF,        // #5fcf8f
    ok_soft: 0x5FCF8F21,   // rgba(95,207,143,.13)
    kw: 0x6BB2FFFF,        // #6bb2ff
    st: 0x8FD19EFF,        // #8fd19e
    func: 0xE6BD70FF,      // #e6bd70
    cm: 0x6C7A8FFF,        // #6c7a8f
    ty: 0x7FD6D0FF,        // #7fd6d0
};

/// Moss palette (values converted from the original HTML mock-up).
pub const MOSS: Tokens = Tokens {
    chrome: 0x0C110FFF,    // #0c110f
    sheet: 0x131A17FF,     // #131a17
    raised: 0x1A2420FF,    // #1a2420
    hover: 0xFFFFFF0D,     // rgba(255,255,255,.05)
    active: 0xFFFFFF16,    // rgba(255,255,255,.085)
    line: 0xFFFFFF16,      // rgba(255,255,255,.085)
    line_soft: 0xFFFFFF0D, // rgba(255,255,255,.05)
    tx: 0xDFE7E1FF,        // #dfe7e1
    tx2: 0x9FB0A5FF,       // #9fb0a5
    tx3: 0x7C8C81FF,       // #7c8c81
    ac: 0x7FD1A8FF,        // #7fd1a8
    ac_soft: 0x7FD1A821,   // rgba(127,209,168,.13)
    ac_on: 0x07170FFF,     // #07170f
    ac_line: 0x7FD1A857,   // rgba(127,209,168,.34)
    sel: 0x7FD1A842,       // rgba(127,209,168,.26)
    ok: 0x7FD1A8FF,        // #7fd1a8
    ok_soft: 0x7FD1A821,   // rgba(127,209,168,.13)
    kw: 0x7FD1A8FF,        // #7fd1a8
    st: 0xE0C27AFF,        // #e0c27a
    func: 0x8FC4E8FF,      // #8fc4e8
    cm: 0x6D7D72FF,        // #6d7d72
    ty: 0xE39B7BFF,        // #e39b7b
};

/// Graphite palette (values converted from the original HTML mock-up).
pub const GRAPHITE: Tokens = Tokens {
    chrome: 0x0D0D0DFF,    // #0d0d0d
    sheet: 0x171717FF,     // #171717
    raised: 0x202020FF,    // #202020
    hover: 0xFFFFFF0E,     // rgba(255,255,255,.055)
    active: 0xFFFFFF17,    // rgba(255,255,255,.09)
    line: 0xFFFFFF17,      // rgba(255,255,255,.09)
    line_soft: 0xFFFFFF0D, // rgba(255,255,255,.05)
    tx: 0xECECECFF,        // #ececec
    tx2: 0xA3A3A3FF,       // #a3a3a3
    tx3: 0x838383FF,       // #838383
    ac: 0xFF7A59FF,        // #ff7a59
    ac_soft: 0xFF7A5924,   // rgba(255,122,89,.14)
    ac_on: 0x1D0A04FF,     // #1d0a04
    ac_line: 0xFF7A595C,   // rgba(255,122,89,.36)
    sel: 0xFF7A5947,       // rgba(255,122,89,.28)
    ok: 0x6FD08CFF,        // #6fd08c
    ok_soft: 0x6FD08C21,   // rgba(111,208,140,.13)
    kw: 0xFF7A59FF,        // #ff7a59
    st: 0xE5C07BFF,        // #e5c07b
    func: 0x7CB7F0FF,      // #7cb7f0
    cm: 0x767676FF,        // #767676
    ty: 0x6FD0C0FF,        // #6fd0c0
};

/// Fog palette (values converted from the original HTML mock-up).
pub const FOG: Tokens = Tokens {
    chrome: 0xE7ECF6FF,    // #e7ecf6
    sheet: 0xFFFFFFFF,     // #ffffff
    raised: 0xFFFFFFFF,    // #ffffff
    hover: 0x1E326E0E,     // rgba(30,50,110,.055)
    active: 0x1E326E17,    // rgba(30,50,110,.09)
    line: 0x1E326E1C,      // rgba(30,50,110,.11)
    line_soft: 0x1E326E11, // rgba(30,50,110,.065)
    tx: 0x172033FF,        // #172033
    tx2: 0x4F5B73FF,       // #4f5b73
    tx3: 0x667389FF,       // #667389
    ac: 0x2563EBFF,        // #2563eb
    ac_soft: 0x2563EB17,   // rgba(37,99,235,.09)
    ac_on: 0xFFFFFFFF,     // #ffffff
    ac_line: 0x2563EB4D,   // rgba(37,99,235,.30)
    sel: 0x2563EB2E,       // rgba(37,99,235,.18)
    ok: 0x1F8A4CFF,        // #1f8a4c
    ok_soft: 0x1F8A4C1C,   // rgba(31,138,76,.11)
    kw: 0x2563EBFF,        // #2563eb
    st: 0xB4531AFF,        // #b4531a
    func: 0x7A3FB8FF,      // #7a3fb8
    cm: 0x8793A8FF,        // #8793a8
    ty: 0x0F7F8AFF,        // #0f7f8a
};

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

    fn tokens(self) -> &'static Tokens {
        match self {
            ThemeKind::Paper => &PAPER,
            ThemeKind::Ink => &INK,
            ThemeKind::Moss => &MOSS,
            ThemeKind::Graphite => &GRAPHITE,
            ThemeKind::Fog => &FOG,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

/// The active theme. Token fields are gpui colours (may be translucent);
/// the `base_*` / `accent` u32 fields are opaque 0xRRGGBB approximations
/// (translucent tokens flattened over the sheet colour) kept for older call sites.
#[derive(Debug, Clone, Copy)]
pub struct ActiveTheme {
    pub kind: ThemeKind,
    pub mode: ThemeMode,

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
    pub ac_soft: Rgba,
    pub ac_on: Rgba,
    pub ac_line: Rgba,
    pub sel: Rgba,
    pub ok: Rgba,
    pub ok_soft: Rgba,
    pub kw: Rgba,
    pub st: Rgba,
    pub func: Rgba,
    pub cm: Rgba,
    pub ty: Rgba,

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
        let sheet = c(p.sheet);
        let chrome = c(p.chrome);
        let raised = c(p.raised);
        let hover = c(p.hover);
        let active = c(p.active);
        let line = c(p.line);
        let tx = c(p.tx);
        let tx2 = c(p.tx2);
        let tx3 = c(p.tx3);
        let ac = c(p.ac);
        Self {
            kind,
            mode: if kind.is_dark() {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            chrome,
            sheet,
            raised,
            hover,
            active,
            line,
            line_soft: c(p.line_soft),
            tx,
            tx2,
            tx3,
            ac,
            ac_soft: c(p.ac_soft),
            ac_on: c(p.ac_on),
            ac_line: c(p.ac_line),
            sel: c(p.sel),
            ok: c(p.ok),
            ok_soft: c(p.ok_soft),
            kw: c(p.kw),
            st: c(p.st),
            func: c(p.func),
            cm: c(p.cm),
            ty: c(p.ty),
            base_00: to_rgb_u32(sheet),
            base_05: to_rgb_u32(raised),
            base_10: to_rgb_u32(over(active, sheet)),
            base_20: to_rgb_u32(chrome),
            base_25: to_rgb_u32(over(hover, chrome)),
            base_30: to_rgb_u32(over(line, sheet)),
            base_35: to_rgb_u32(over(hover, sheet)),
            base_50: to_rgb_u32(tx3),
            base_70: to_rgb_u32(tx2),
            base_100: to_rgb_u32(tx),
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
        self.sheet
    }
    #[inline]
    pub fn bg_sidebar(&self) -> Rgba {
        self.chrome
    }
    #[inline]
    pub fn bg_card(&self) -> Rgba {
        self.raised
    }
    #[inline]
    pub fn bg_hover(&self) -> Rgba {
        self.hover
    }
    #[inline]
    pub fn border(&self) -> Rgba {
        self.line
    }
    #[inline]
    pub fn text(&self) -> Rgba {
        self.tx
    }
    #[inline]
    pub fn text_muted(&self) -> Rgba {
        self.tx2
    }
    #[inline]
    pub fn text_faint(&self) -> Rgba {
        self.tx3
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
        assert_eq!((RADIUS_S, RADIUS_M, RADIUS_L), (6.0, 8.0, 12.0));
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
