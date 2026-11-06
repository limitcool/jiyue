//! Resolved font settings: the families and sizes that the notes and the interface draw with.
//!
//! The user's `[appearance]` section (see `jiyue_core::config::appearance`) is turned into a
//! [`FontSettings`] by the pure function [`resolve`]: unknown font names fall back to the built-in
//! family and are reported. [`metrics`] derives every editor size from the body size and the line
//! height, so headings and widgets scale together. The current settings live in one process-wide
//! snapshot ([`current`]) with a generation number; the editor's shaped-line cache includes the
//! generation, so a change re-shapes the text.
//!
//! Reads are cheap and need no `Context`: the editor paints from [`current`] inside its prepaint,
//! and the settings page calls [`set_appearance`] for every change.

use std::sync::{Arc, OnceLock, RwLock};

use gpui::SharedString;
use jiyue_core::config::appearance::{Appearance, CODE_SIZE_FOLLOW};

use crate::theme::ThemeKind;
use crate::ui::{MONO_FONT, SERIF_FONT, UI_FONT};

/// Ratio of the code size to the body size when the code size follows the editor.
pub const CODE_FOLLOW_RATIO: f32 = 0.9;
/// Heading sizes relative to the body size (H1 to H6): 28, 22, 18, 16, 15, 15 at a 15 px body.
const HEADING_SIZE_RATIO: [f32; 6] = [28.0 / 15.0, 22.0 / 15.0, 18.0 / 15.0, 16.0 / 15.0, 1.0, 1.0];
/// Heading row heights relative to the body row height: 44, 36, 30, 28, 26, 26 at a 26 px row.
const HEADING_LH_RATIO: [f32; 6] = [44.0 / 26.0, 36.0 / 26.0, 30.0 / 26.0, 28.0 / 26.0, 1.0, 1.0];
/// The body size the design was drawn at. Other sizes scale with `body / BASE_BODY`.
pub const BASE_BODY: f32 = 15.0;

/// Fonts that the picker lists first when they are installed: Chinese text faces, then monospace
/// faces for code. Matched case-insensitively.
pub const PINNED_FONTS: &[&str] = &[
    "Microsoft YaHei UI",
    "微软雅黑",
    "等线",
    "宋体",
    "黑体",
    "楷体",
    "霞鹜文楷",
    "LXGW WenKai",
    "Noto Sans SC",
    "Noto Serif SC",
    "思源黑体",
    "思源宋体",
    "PingFang SC",
    "Hiragino Sans GB",
    "JetBrains Mono",
    "JetBrainsMono Nerd Font",
    "Cascadia Code",
    "Cascadia Mono",
    "Consolas",
    "Fira Code",
    "Source Code Pro",
    "SF Mono",
    "Menlo",
    "Monaco",
];

/// Where a font is used. The editor's body text, the interface, headings and code have their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// Body text of the editor and of read mode.
    Editor,
    /// Labels, menus and other interface text.
    Ui,
    /// Headings: serif in the paper and moss themes, else the editor family.
    Heading,
    /// Code blocks, inline code and the source of formulas.
    Code,
}

/// Which setting a font name belongs to (for the warnings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Ui,
    Editor,
    Code,
}

impl Slot {
    /// The Chinese label of the setting, as shown in the settings page.
    pub fn label(self) -> &'static str {
        match self {
            Slot::Ui => "界面字体",
            Slot::Editor => "正文字体",
            Slot::Code => "代码字体",
        }
    }
}

/// The resolved settings. Families are always names that the app asks the system for; a missing
/// configured font was replaced by the built-in one (see `missing`).
#[derive(Clone, Debug, PartialEq)]
pub struct FontSettings {
    /// Bumped on every change; part of the editor's shaped-line cache key.
    pub generation: u64,
    pub ui_family: String,
    pub editor_family: String,
    pub code_family: String,
    /// Body size in px.
    pub editor_size: f32,
    /// Code size in px (already resolved: "follow" is applied).
    pub code_size: f32,
    /// Line height multiplier (inside its range).
    pub line_height: f32,
    /// The clamped settings the snapshot was built from (what the settings page edits).
    pub config: Appearance,
    /// One message per configured font that is not installed.
    pub missing: Vec<String>,
    /// One message per number that was out of range in the file.
    pub clamp_warnings: Vec<String>,
}

/// Message shown when a configured font is not installed.
pub fn missing_message(slot: Slot, name: &str) -> String {
    format!("{}：未找到字体 {name}，已使用默认字体", slot.label())
}

/// Turns the configured settings into the resolved ones. `installed` is the list of installed
/// families; an empty list means "not known yet", and then every configured name is accepted.
pub fn resolve(config: &Appearance, installed: &[String]) -> FontSettings {
    let (clean, clamp_warnings) = config.clamped();
    let mut missing = Vec::new();
    let mut pick = |slot: Slot, name: &str, default: &str| -> String {
        if name.is_empty() {
            return default.to_string();
        }
        if installed.is_empty() || is_installed(installed, name) {
            return name.to_string();
        }
        missing.push(missing_message(slot, name));
        default.to_string()
    };
    let ui_family = pick(Slot::Ui, &clean.ui_font, UI_FONT);
    let editor_family = if clean.editor_font.is_empty() {
        ui_family.clone()
    } else {
        pick(Slot::Editor, &clean.editor_font, UI_FONT)
    };
    let code_family = pick(Slot::Code, &clean.code_font, MONO_FONT);
    let editor_size = clean.editor_font_size;
    let code_size = if clean.code_font_size == CODE_SIZE_FOLLOW {
        editor_size * CODE_FOLLOW_RATIO
    } else {
        clean.code_font_size
    };
    FontSettings {
        generation: 0,
        ui_family,
        editor_family,
        code_family,
        editor_size,
        code_size,
        line_height: clean.line_height,
        config: clean,
        missing,
        clamp_warnings,
    }
}

/// True when `name` is one of `installed` (case-insensitive, as the system matches families).
pub fn is_installed(installed: &[String], name: &str) -> bool {
    let wanted = name.trim();
    installed.iter().any(|f| f.eq_ignore_ascii_case(wanted))
}

/// Every size the editor draws with, for the current settings. Sizes are px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Body text size and row height (rows of plain text and of the source view).
    pub font: f32,
    pub line: f32,
    /// Heading sizes and row heights, H1 to H6.
    pub heading: [f32; 6],
    pub heading_lh: [f32; 6],
    /// Code block text size (source view) and the size of code in the live view.
    pub code: f32,
    pub code_live: f32,
    /// Row height of a code line in the live view.
    pub code_row: f32,
    /// Scale of the widgets (tables, callouts, embeds) relative to the design: `font / BASE_BODY`.
    pub k: f32,
}

/// Metrics of the current settings.
pub fn current_metrics() -> Metrics {
    metrics(&current())
}

/// Metrics for resolved settings.
pub fn metrics(s: &FontSettings) -> Metrics {
    let font = s.editor_size.max(1.0);
    let line = (font * s.line_height).round().max(1.0);
    let code = s.code_size.max(1.0);
    let k = font / BASE_BODY;
    let mut heading = [0.0; 6];
    let mut heading_lh = [0.0; 6];
    for i in 0..6 {
        heading[i] = font * HEADING_SIZE_RATIO[i];
        heading_lh[i] = (line * HEADING_LH_RATIO[i]).round();
    }
    Metrics {
        font,
        line,
        heading,
        heading_lh,
        code,
        // The live code panel was drawn at 13 px where the source view used 13.5.
        code_live: code * 13.0 / 13.5,
        code_row: 20.0 * k,
        k,
    }
}

/// The key part that a shaped-line cache adds for the font settings: the same theme hash with a
/// different generation gives another key, so lines are shaped again after a font change.
pub fn shape_salt(theme_salt: u64, generation: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    theme_salt.hash(&mut h);
    generation.hash(&mut h);
    h.finish()
}

// ----- picker list -------------------------------------------------------------------------

/// Sorts and de-duplicates a list of family names for the picker. Names that start with `.` (system
/// placeholders) or `@` (vertical variants of CJK faces) are dropped. Pinned faces come first, in
/// the order of `PINNED_FONTS`; the rest is sorted case-insensitively.
pub fn normalize_font_list(names: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for raw in names {
        let name = raw.trim();
        if name.is_empty() || name.starts_with('.') || name.starts_with('@') {
            continue;
        }
        if seen.insert(name.to_lowercase()) {
            out.push(name.to_string());
        }
    }
    out.sort_by(|a, b| {
        pin_rank(a)
            .cmp(&pin_rank(b))
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
    });
    out
}

fn pin_rank(name: &str) -> usize {
    PINNED_FONTS
        .iter()
        .position(|p| p.eq_ignore_ascii_case(name))
        .unwrap_or(PINNED_FONTS.len())
}

/// True for a family that is likely to draw Chinese text: a CJK character in its name, or a known
/// Chinese face. A hint for the picker, not a guarantee.
pub fn is_cjk_font(name: &str) -> bool {
    const KNOWN: &[&str] = &[
        "Microsoft YaHei",
        "微软雅黑",
        "等线",
        "宋体",
        "黑体",
        "楷体",
        "霞鹜",
        "LXGW",
        "Noto Sans SC",
        "Noto Serif SC",
        "思源",
        "PingFang",
        "Hiragino",
        "Sarasa",
        "Source Han",
        "STSong",
        "STHeiti",
        "Songti",
        "Kaiti",
        "Heiti",
    ];
    let lower = name.to_lowercase();
    name.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        || KNOWN.iter().any(|k| lower.contains(&k.to_lowercase()))
}

/// True for a family that is likely monospace (a hint from the name).
pub fn is_mono_font(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "mono", "code", "console", "consolas", "menlo", "monaco", "courier", "fira",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// The families shown for a query: all of them when the query is blank, else the ones whose name
/// contains the query (case-insensitive). The list order is kept.
pub fn filter_fonts(list: &[String], query: &str) -> Vec<String> {
    let q = query.trim().to_lowercase();
    list.iter()
        .filter(|name| q.is_empty() || name.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

// ----- stepper rules -----------------------------------------------------------------------

/// Body size presets of the segmented control, in px.
pub const EDITOR_PRESETS: [f32; 6] = [12.0, 14.0, 16.0, 18.0, 20.0, 24.0];
/// Stepper bounds used by the settings page (narrower than the file's range on purpose).
pub const STEP_EDITOR: (f32, f32) = (10.0, 36.0);
pub const STEP_CODE: (f32, f32) = (9.0, 32.0);
pub const STEP_LINE: (f32, f32) = (1.2, 2.2);

/// One step of a float stepper: adds `delta`, clamps to `bounds` and rounds to two decimals so
/// repeated steps do not drift.
pub fn step_float(current: f32, delta: f32, bounds: (f32, f32)) -> f32 {
    let next = if current.is_finite() {
        current + delta
    } else {
        delta
    };
    let rounded = (next * 100.0).round() / 100.0;
    rounded.clamp(bounds.0, bounds.1)
}

/// One step of the code size stepper. Value 0 ("follow") starts from 90 % of the body size.
pub fn step_code(current: f32, body: f32, delta: f32, bounds: (f32, f32)) -> f32 {
    let base = if current == CODE_SIZE_FOLLOW {
        (body * CODE_FOLLOW_RATIO).round()
    } else {
        current
    };
    step_float(base, delta, bounds)
}

/// The preset equal to `value` (within a hundredth), if any.
pub fn snap_preset(value: f32, presets: &[f32]) -> Option<f32> {
    presets.iter().copied().find(|p| (p - value).abs() < 0.01)
}

/// Parses a number typed into a field. Surrounding spaces and a trailing `px` or `%` are ignored.
pub fn parse_number(text: &str) -> Option<f32> {
    let t = text
        .trim()
        .trim_end_matches("px")
        .trim_end_matches('%')
        .trim();
    let v: f32 = t.parse().ok()?;
    v.is_finite().then_some(v)
}

// ----- the process-wide snapshot -----------------------------------------------------------

/// The state behind [`current`]: the file's settings, the installed families and the snapshot.
struct Store {
    raw: Appearance,
    installed: Vec<String>,
    current: Arc<FontSettings>,
    generation: u64,
}

impl Store {
    fn build(raw: Appearance, installed: Vec<String>, generation: u64) -> Self {
        let current = Arc::new(rebuilt(&raw, &installed, generation));
        Self {
            raw,
            installed,
            current,
            generation,
        }
    }
}

static STORE: OnceLock<RwLock<Store>> = OnceLock::new();

fn store() -> &'static RwLock<Store> {
    STORE.get_or_init(|| {
        // Tests never read the developer's own config: they start from the built-in settings.
        #[cfg(test)]
        let raw = Appearance::default();
        #[cfg(not(test))]
        let raw = crate::config_store::read_appearance_at(&jiyue_core::config::user_dir()).0;
        RwLock::new(Store::build(raw, Vec::new(), 1))
    })
}

/// The current settings snapshot. Cheap: clones an `Arc`.
pub fn current() -> Arc<FontSettings> {
    match store().read() {
        Ok(g) => g.current.clone(),
        Err(p) => p.into_inner().current.clone(),
    }
}

/// The generation of the current snapshot (see `shape_salt`).
pub fn generation() -> u64 {
    current().generation
}

/// The clamped settings as the file holds them (what the settings page edits).
pub fn appearance() -> Appearance {
    current().config.clone()
}

/// The installed families, sorted (see `normalize_font_list`). Empty until `ensure_installed`.
pub fn installed() -> Vec<String> {
    match store().read() {
        Ok(g) => g.installed.clone(),
        Err(p) => p.into_inner().installed.clone(),
    }
}

/// Stores the list of installed families the first time it is asked for. `load` runs only while
/// the list is unknown. Enumerating the system fonts is quick, but it runs on the UI thread once.
pub fn ensure_installed(load: impl FnOnce() -> Vec<String>) {
    if !installed().is_empty() {
        return;
    }
    let list = normalize_font_list(&load());
    if list.is_empty() {
        return;
    }
    let mut g = match store().write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if !g.installed.is_empty() {
        return;
    }
    g.installed = list;
    g.generation += 1;
    g.current = Arc::new(rebuilt(&g.raw, &g.installed, g.generation));
}

/// Replaces the settings (after a change in the settings page or a file change) and rebuilds the
/// snapshot with a new generation.
pub fn set_appearance(value: Appearance) {
    let mut g = match store().write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    g.raw = value;
    g.generation += 1;
    g.current = Arc::new(rebuilt(&g.raw, &g.installed, g.generation));
}

/// Re-reads `[appearance]` from the user's `config.toml` (used when the file changes on disk).
pub fn reload_from_disk() {
    let (raw, _) = crate::config_store::read_appearance_at(&jiyue_core::config::user_dir());
    set_appearance(raw);
}

fn rebuilt(raw: &Appearance, installed: &[String], generation: u64) -> FontSettings {
    let mut snapshot = resolve(raw, installed);
    snapshot.generation = generation;
    snapshot
}

/// The family name to hand to gpui for a face in a theme.
pub fn face_name(face: Face, kind: ThemeKind) -> SharedString {
    let s = current();
    let name: &str = match face {
        Face::Ui => s.ui_family.as_str(),
        Face::Editor => s.editor_family.as_str(),
        Face::Code => s.code_family.as_str(),
        Face::Heading => match kind {
            ThemeKind::Paper | ThemeKind::Moss => SERIF_FONT,
            _ => s.editor_family.as_str(),
        },
    };
    SharedString::from(name.to_string())
}

/// The interface family (a shorthand for `face_name(Face::Ui, _)`).
pub fn ui_name() -> SharedString {
    SharedString::from(current().ui_family.clone())
}

/// The code family (a shorthand for `face_name(Face::Code, _)`).
pub fn code_name() -> SharedString {
    SharedString::from(current().code_family.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_settings_give_the_built_in_families() {
        let s = resolve(&Appearance::default(), &[]);
        assert_eq!(s.ui_family, UI_FONT);
        assert_eq!(s.editor_family, UI_FONT);
        assert_eq!(s.code_family, MONO_FONT);
        assert!(s.missing.is_empty());
        assert_eq!(s.editor_size, 15.0);
        assert!((s.code_size - 13.5).abs() < 1e-4, "follow = 90 %");
    }

    #[test]
    fn unknown_font_falls_back_to_the_default_with_a_warning() {
        let installed = names(&["Arial", "Consolas"]);
        let cfg = Appearance {
            ui_font: "Ghost Font".into(),
            code_font: "Consolas".into(),
            ..Appearance::default()
        };
        let s = resolve(&cfg, &installed);
        assert_eq!(s.ui_family, UI_FONT);
        assert_eq!(s.code_family, "Consolas");
        assert_eq!(s.missing, vec![missing_message(Slot::Ui, "Ghost Font")]);
        assert!(s.missing[0].contains("未找到字体 Ghost Font"));
        assert!(s.missing[0].ends_with("已使用默认字体"));
    }

    #[test]
    fn installed_names_match_case_insensitively() {
        let installed = names(&["Consolas"]);
        let cfg = Appearance {
            code_font: "consolas".into(),
            ..Appearance::default()
        };
        assert_eq!(resolve(&cfg, &installed).code_family, "consolas");
        assert!(resolve(&cfg, &installed).missing.is_empty());
    }

    #[test]
    fn unknown_installed_list_accepts_every_name() {
        let cfg = Appearance {
            ui_font: "Anything".into(),
            ..Appearance::default()
        };
        let s = resolve(&cfg, &[]);
        assert_eq!(s.ui_family, "Anything");
        assert!(s.missing.is_empty());
    }

    #[test]
    fn editor_font_defaults_to_the_interface_font() {
        let cfg = Appearance {
            ui_font: "Arial".into(),
            ..Appearance::default()
        };
        let s = resolve(&cfg, &names(&["Arial"]));
        assert_eq!(s.editor_family, "Arial");
        let own = Appearance {
            ui_font: "Arial".into(),
            editor_font: "Georgia".into(),
            ..Appearance::default()
        };
        let s = resolve(&own, &names(&["Arial"]));
        assert_eq!(
            s.editor_family, UI_FONT,
            "missing body font uses the built-in one"
        );
        assert_eq!(s.missing.len(), 1);
    }

    #[test]
    fn follow_code_size_is_ninety_percent_of_the_body() {
        let cfg = Appearance {
            editor_font_size: 20.0,
            ..Appearance::default()
        };
        assert!((resolve(&cfg, &[]).code_size - 18.0).abs() < 1e-4);
        let fixed = Appearance {
            editor_font_size: 20.0,
            code_font_size: 12.0,
            ..Appearance::default()
        };
        assert_eq!(resolve(&fixed, &[]).code_size, 12.0);
    }

    #[test]
    fn default_metrics_reproduce_the_design_sizes() {
        let m = metrics(&resolve(&Appearance::default(), &[]));
        assert_eq!(m.font, 15.0);
        assert_eq!(m.line, 26.0);
        assert_eq!(m.heading, [28.0, 22.0, 18.0, 16.0, 15.0, 15.0]);
        assert_eq!(m.heading_lh, [44.0, 36.0, 30.0, 28.0, 26.0, 26.0]);
        assert!((m.code - 13.5).abs() < 1e-4);
        assert!((m.code_live - 13.0).abs() < 1e-4);
        assert_eq!(m.code_row, 20.0);
        assert_eq!(m.k, 1.0);
    }

    #[test]
    fn headings_scale_proportionally_with_the_body() {
        let cfg = Appearance {
            editor_font_size: 20.0,
            ..Appearance::default()
        };
        let m = metrics(&resolve(&cfg, &[]));
        assert!((m.heading[0] - 20.0 * 28.0 / 15.0).abs() < 1e-3);
        assert!((m.heading[5] - 20.0).abs() < 1e-3);
        assert!(m.heading[0] > m.heading[1] && m.heading[1] > m.heading[2]);
        assert!(m.heading_lh[0] > m.line);
        assert_eq!(m.line, 35.0, "20 * 1.73 rounds to 35");
    }

    #[test]
    fn line_height_is_clamped_and_drives_the_row() {
        let tall = Appearance {
            line_height: 9.0,
            ..Appearance::default()
        };
        let s = resolve(&tall, &[]);
        assert_eq!(s.line_height, 2.5);
        assert_eq!(metrics(&s).line, 38.0, "15 * 2.5 = 37.5 rounds up");
        let low = Appearance {
            line_height: 0.1,
            ..Appearance::default()
        };
        assert_eq!(resolve(&low, &[]).line_height, 1.0);
    }

    #[test]
    fn shape_salt_changes_with_the_generation() {
        let theme = 0xABCD_u64;
        assert_ne!(shape_salt(theme, 1), shape_salt(theme, 2));
        assert_eq!(shape_salt(theme, 7), shape_salt(theme, 7));
        assert_ne!(shape_salt(theme, 1), shape_salt(theme + 1, 1));
    }

    #[test]
    fn font_list_is_sorted_deduplicated_and_pinned() {
        let raw = names(&[
            "Zeta",
            "alpha",
            "Consolas",
            "@Microsoft YaHei",
            ".SystemUIFont",
            "",
            "ALPHA",
            "宋体",
            "Arial",
            "Microsoft YaHei UI",
        ]);
        let list = normalize_font_list(&raw);
        assert_eq!(list[0], "Microsoft YaHei UI", "pinned first");
        assert_eq!(list[1], "宋体", "pinned order");
        assert_eq!(
            list[2], "Consolas",
            "pinned mono face comes after the CJK ones"
        );
        let rest: Vec<&str> = list[3..].iter().map(String::as_str).collect();
        assert_eq!(rest, vec!["alpha", "Arial", "Zeta"]);
        assert_eq!(
            list.iter()
                .filter(|n| n.eq_ignore_ascii_case("alpha"))
                .count(),
            1
        );
    }

    #[test]
    fn font_filter_is_a_case_insensitive_substring() {
        let list = names(&["Noto Sans SC", "Noto Serif SC", "Arial"]);
        assert_eq!(filter_fonts(&list, "").len(), 3);
        assert_eq!(filter_fonts(&list, "noto serif"), vec!["Noto Serif SC"]);
        assert_eq!(filter_fonts(&list, "  ARIAL "), vec!["Arial"]);
        assert!(filter_fonts(&list, "zzz").is_empty());
    }

    #[test]
    fn cjk_and_mono_hints() {
        assert!(is_cjk_font("霞鹜文楷 Regular"));
        assert!(is_cjk_font("Noto Sans SC"));
        assert!(!is_cjk_font("Arial"));
        assert!(is_mono_font("JetBrainsMono Nerd Font"));
        assert!(is_mono_font("Consolas"));
        assert!(!is_mono_font("Arial"));
    }

    #[test]
    fn float_stepper_clamps_and_rounds() {
        assert_eq!(step_float(15.0, 1.0, STEP_EDITOR), 16.0);
        assert_eq!(step_float(36.0, 1.0, STEP_EDITOR), 36.0);
        assert_eq!(step_float(1.2, -0.1, STEP_LINE), 1.2);
        let mut lh = 1.7_f32;
        for _ in 0..3 {
            lh = step_float(lh, 0.1, STEP_LINE);
        }
        assert_eq!(lh, 2.0, "steps do not drift");
        assert_eq!(step_float(f32::NAN, 0.1, STEP_LINE), 1.2);
    }

    #[test]
    fn code_stepper_starts_from_follow() {
        assert_eq!(step_code(CODE_SIZE_FOLLOW, 15.0, 1.0, STEP_CODE), 15.0);
        assert_eq!(step_code(14.0, 15.0, -1.0, STEP_CODE), 13.0);
        assert_eq!(step_code(9.0, 15.0, -1.0, STEP_CODE), 9.0);
    }

    #[test]
    fn presets_snap_only_on_exact_matches() {
        assert_eq!(snap_preset(16.0, &EDITOR_PRESETS), Some(16.0));
        assert_eq!(snap_preset(16.004, &EDITOR_PRESETS), Some(16.0));
        assert_eq!(snap_preset(17.0, &EDITOR_PRESETS), None);
    }

    #[test]
    fn typed_numbers_accept_units_and_reject_garbage() {
        assert_eq!(parse_number(" 18 "), Some(18.0));
        assert_eq!(parse_number("18px"), Some(18.0));
        assert_eq!(parse_number("125%"), Some(125.0));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn missing_message_names_the_setting_and_the_font() {
        let text = missing_message(Slot::Code, "Nope");
        assert!(text.starts_with("代码字体"));
        assert!(text.contains("Nope"));
    }
}
