//! Interface preferences that the user changes in the app and expects back at the next start:
//! the theme, the sidebars, the live preview switch (`[ui]` in the user's `config.toml`), and the
//! window geometry (`[window]` in the user's `state.toml`). Pure data and pure rules, no gpui.

use serde::{Deserialize, Serialize};

/// Theme names the app knows, in the order of the design (also the command line names).
pub const KNOWN_THEMES: [&str; 5] = ["ink", "fog", "paper", "moss", "graphite"];
/// The theme used when nothing else says otherwise.
pub const DEFAULT_THEME: &str = "ink";

/// `[ui]` section of the user's `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    /// Theme name: one of `KNOWN_THEMES`. Unknown names fall back to the default.
    pub theme: String,
    /// Interface language locale code (e.g., "zh-CN", "en-US", or empty/auto for system).
    pub locale: String,
    /// The rendered view is the default for notes (设置 > 编辑器 > 实时预览).
    pub live_preview: bool,
    /// The left sidebar (files, search, tags) is shown.
    pub left_sidebar: bool,
    /// The right panel (outline, backlinks) is shown.
    pub right_sidebar: bool,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME.to_string(),
            locale: String::new(),
            live_preview: true,
            left_sidebar: true,
            right_sidebar: true,
        }
    }
}

impl UiPrefs {
    /// The theme name, or the default when the stored name is not a known theme.
    pub fn theme_name(&self) -> &str {
        if is_known_theme(&self.theme) {
            &self.theme
        } else {
            DEFAULT_THEME
        }
    }
}

/// True for a name in `KNOWN_THEMES` (case-insensitive, as the command line accepts).
pub fn is_known_theme(name: &str) -> bool {
    KNOWN_THEMES
        .iter()
        .any(|known| known.eq_ignore_ascii_case(name))
}

/// The theme to start with. An explicit `--theme` value wins when it is known; otherwise the saved
/// value is used when it is known; otherwise the default. An unknown explicit value is ignored
/// (the caller reports it).
pub fn startup_theme(explicit: Option<&str>, saved: &str) -> String {
    if let Some(name) = explicit.filter(|n| is_known_theme(n)) {
        return name.to_ascii_lowercase();
    }
    if is_known_theme(saved) {
        return saved.to_ascii_lowercase();
    }
    DEFAULT_THEME.to_string()
}

/// `[window]` section of the user's `state.toml`: where the window was when the app last closed.
/// `width == 0` means nothing was saved yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    /// `windowed`, `maximized` or `fullscreen`.
    pub mode: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            mode: "windowed".to_string(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        }
    }
}

/// A rectangle in pixels: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

/// Smallest saved size that is still restored. Smaller values are treated as a broken record.
pub const MIN_SAVED_SIZE: (f32, f32) = (480.0, 320.0);
/// The saved window must overlap a display by at least this many pixels in both directions, so a
/// window left on a disconnected monitor is not restored off screen.
pub const MIN_VISIBLE_OVERLAP: f32 = 100.0;

/// True when the saved window can be restored on one of the `displays`: its size is sane, all
/// numbers are finite, and it overlaps a display enough to be reached with the mouse.
pub fn window_fits(saved: &WindowState, displays: &[Rect]) -> bool {
    let finite = [saved.x, saved.y, saved.width, saved.height]
        .iter()
        .all(|v| v.is_finite());
    if !finite || saved.width < MIN_SAVED_SIZE.0 || saved.height < MIN_SAVED_SIZE.1 {
        return false;
    }
    let (x, y, w, h) = (saved.x, saved.y, saved.width, saved.height);
    displays.iter().any(|&(dx, dy, dw, dh)| {
        let overlap_w = (x + w).min(dx + dw) - x.max(dx);
        let overlap_h = (y + h).min(dy + dh) - y.max(dy);
        overlap_w >= MIN_VISIBLE_OVERLAP && overlap_h >= MIN_VISIBLE_OVERLAP
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(x: f32, y: f32, w: f32, h: f32) -> WindowState {
        WindowState {
            mode: "windowed".into(),
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn explicit_flag_beats_saved_which_beats_default() {
        assert_eq!(startup_theme(Some("fog"), "paper"), "fog");
        assert_eq!(startup_theme(None, "paper"), "paper");
        assert_eq!(startup_theme(None, ""), "ink");
        assert_eq!(startup_theme(None, "neon"), "ink");
    }

    #[test]
    fn unknown_explicit_flag_falls_back_to_saved() {
        assert_eq!(startup_theme(Some("neon"), "moss"), "moss");
        assert_eq!(startup_theme(Some("neon"), "neon"), "ink");
    }

    #[test]
    fn theme_names_are_case_insensitive_and_lower_cased() {
        assert_eq!(startup_theme(Some("FOG"), "ink"), "fog");
        assert!(is_known_theme("Graphite"));
        assert!(!is_known_theme("dark"));
    }

    #[test]
    fn theme_name_of_ui_prefs_falls_back_for_unknown() {
        let prefs = UiPrefs {
            theme: "neon".into(),
            ..UiPrefs::default()
        };
        assert_eq!(prefs.theme_name(), "ink");
        assert_eq!(UiPrefs::default().theme_name(), "ink");
    }

    #[test]
    fn window_on_a_display_is_restored() {
        let displays = [(0.0, 0.0, 1920.0, 1080.0)];
        assert!(window_fits(&win(100.0, 80.0, 1280.0, 800.0), &displays));
    }

    #[test]
    fn window_on_a_missing_monitor_is_not_restored() {
        let displays = [(0.0, 0.0, 1920.0, 1080.0)];
        // Far to the right of the only display (a disconnected second monitor).
        assert!(!window_fits(&win(3000.0, 100.0, 1280.0, 800.0), &displays));
        // Mostly off the left edge, with only a sliver visible.
        assert!(!window_fits(&win(-1250.0, 100.0, 1280.0, 800.0), &displays));
    }

    #[test]
    fn broken_geometry_is_not_restored() {
        let displays = [(0.0, 0.0, 1920.0, 1080.0)];
        assert!(!window_fits(&WindowState::default(), &displays));
        assert!(!window_fits(&win(10.0, 10.0, 100.0, 100.0), &displays));
        assert!(!window_fits(&win(f32::NAN, 0.0, 1280.0, 800.0), &displays));
        assert!(!window_fits(
            &win(0.0, 0.0, f32::INFINITY, 800.0),
            &displays
        ));
        assert!(!window_fits(&win(10.0, 10.0, 1280.0, 800.0), &[]));
    }

    #[test]
    fn ui_prefs_round_trip_through_toml() {
        let prefs = UiPrefs {
            theme: "paper".into(),
            locale: "zh-CN".into(),
            live_preview: false,
            left_sidebar: false,
            right_sidebar: true,
        };
        let text = toml::to_string(&prefs).expect("serialise");
        let back: UiPrefs = toml::from_str(&text).expect("parse");
        assert_eq!(back, prefs);
        // A partial table keeps the defaults of the missing keys (not `false`).
        let partial: UiPrefs = toml::from_str("left_sidebar = false").expect("partial");
        assert!(!partial.left_sidebar);
        assert!(partial.right_sidebar);
        assert!(partial.live_preview);
        assert_eq!(partial.theme, "ink");
        assert!(partial.locale.is_empty());
    }

    #[test]
    fn window_state_round_trips_through_toml() {
        let state = WindowState {
            mode: "maximized".into(),
            x: 12.5,
            y: 40.0,
            width: 1400.0,
            height: 900.0,
        };
        let text = toml::to_string(&state).expect("serialise");
        assert_eq!(toml::from_str::<WindowState>(&text).expect("parse"), state);
    }
}
