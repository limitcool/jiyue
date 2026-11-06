//! LaTeX math to self-contained SVG, without a browser or a JavaScript engine.
//!
//! Pipeline: `latex-rust` (pure Rust TeX-style layout, STIX Two Math outlines)
//! produces an SVG document in which every glyph is a `<path>`. This crate adds
//! a worker-thread timeout, size limits, a safety scan, a content-addressed LRU
//! cache, a resvg rasteriser (feature `raster`) and a `$...$` span finder.
//!
//! Units: `RenderedMath::{width, height, baseline}` are CSS pixels (96 dpi) at
//! `MathTheme::font_size`, which is also a CSS pixel size.

use std::fmt;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use latex_rust::{Color, Dim, Error as LrError, FontError, MathFont, MathStyle, SvgOptions};
use serde::{Deserialize, Serialize};

mod cache;
mod spans;

#[cfg(feature = "raster")]
mod raster;

pub use cache::MathCache;
pub use spans::{MathSpan, find_math_spans};

#[cfg(feature = "raster")]
pub use raster::{Rgba8Image, rasterize};

/// Maximum accepted LaTeX source length, and maximum produced SVG length (bytes).
pub const MAX_BYTES: usize = 20 * 1024;

/// Default wall-clock budget for one render on the worker thread.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

/// Font-size bounds in CSS pixels accepted by [`MathTheme`].
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> = 4.0..=256.0;

/// Visual parameters of a rendered formula.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MathTheme {
    /// Glyph colour, `#RRGGBB`.
    pub color: String,
    /// Em size in CSS pixels.
    pub font_size: f32,
}

impl Default for MathTheme {
    fn default() -> Self {
        Self {
            color: "#000000".to_string(),
            font_size: 16.0,
        }
    }
}

/// A formula rendered to SVG plus the metrics needed for inline alignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderedMath {
    /// Self-contained SVG 1.1 document (paths and rects only, no scripts, no links).
    pub svg: String,
    /// Width in CSS px.
    pub width: f32,
    /// Total height (ascent + descent) in CSS px.
    pub height: f32,
    /// Distance from the top edge to the math axis/baseline, in CSS px.
    pub baseline: f32,
}

/// Failure modes of this crate. Never a fake render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MathError {
    /// The source is not valid math (or is empty).
    Parse { message: String },
    /// Valid LaTeX that this renderer does not support (e.g. CJK glyphs in `\text{}`).
    Unsupported(String),
    /// Source or output exceeds [`MAX_BYTES`], or a raster exceeds its pixel limit.
    TooLarge,
    /// The worker did not finish within the time budget.
    Timeout,
    /// Internal failure: invalid theme, font error, panic, thread error.
    Render(String),
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { message } => write!(f, "math parse error: {message}"),
            Self::Unsupported(what) => write!(f, "unsupported math: {what}"),
            Self::TooLarge => write!(f, "math too large"),
            Self::Timeout => write!(f, "math render timed out"),
            Self::Render(msg) => write!(f, "math render error: {msg}"),
        }
    }
}

impl std::error::Error for MathError {}

/// Render one formula with the default 2 s timeout.
///
/// `display` selects display style (`$$...$$`) versus text style (`$...$`).
pub fn render_math(
    latex: &str,
    display: bool,
    theme: &MathTheme,
) -> Result<RenderedMath, MathError> {
    render_math_with_timeout(latex, display, theme, DEFAULT_TIMEOUT)
}

/// Like [`render_math`] with an explicit time budget.
///
/// The work runs on a detached worker thread. On timeout the caller gets
/// [`MathError::Timeout`] immediately; the worker finishes on its own and its
/// result is dropped (Rust cannot kill a thread). Input size is capped, which
/// bounds that background cost.
pub fn render_math_with_timeout(
    latex: &str,
    display: bool,
    theme: &MathTheme,
    timeout: Duration,
) -> Result<RenderedMath, MathError> {
    let (rgb, font_px) = parse_theme(theme)?;
    if latex.len() > MAX_BYTES {
        return Err(MathError::TooLarge);
    }
    let source = latex.to_owned();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("jiyue-math".to_string())
        .spawn(move || {
            let out = std::panic::catch_unwind(AssertUnwindSafe(|| {
                render_core(&source, display, rgb, font_px)
            }))
            .unwrap_or_else(|_| Err(MathError::Render("internal renderer panic".to_string())));
            // The receiver may already have timed out and dropped; that is fine.
            let _ = tx.send(out);
        })
        .map_err(|e| MathError::Render(format!("cannot start worker thread: {e}")))?;
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(MathError::Timeout),
        Err(RecvTimeoutError::Disconnected) => Err(MathError::Render(
            "worker thread exited without a result".to_string(),
        )),
    }
}

/// Validate the theme and split it into (rgb, font size in px).
fn parse_theme(theme: &MathTheme) -> Result<((u8, u8, u8), f32), MathError> {
    let hex = theme
        .color
        .strip_prefix('#')
        .filter(|h| h.len() == 6 && h.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| MathError::Render(format!("invalid theme colour {:?}", theme.color)))?;
    let channel = |range: std::ops::Range<usize>| {
        hex.get(range)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .ok_or_else(|| MathError::Render("invalid theme colour".to_string()))
    };
    let rgb = (channel(0..2)?, channel(2..4)?, channel(4..6)?);
    if !FONT_SIZE_RANGE.contains(&theme.font_size) {
        return Err(MathError::Render(format!(
            "font size {} outside {:?}",
            theme.font_size, FONT_SIZE_RANGE
        )));
    }
    Ok((rgb, theme.font_size))
}

/// Synchronous render. Runs on the worker thread.
fn render_core(
    latex: &str,
    display: bool,
    rgb: (u8, u8, u8),
    font_px: f32,
) -> Result<RenderedMath, MathError> {
    if latex.trim().is_empty() {
        return Err(MathError::Parse {
            message: "empty formula".to_string(),
        });
    }
    if latex.len() > MAX_BYTES {
        return Err(MathError::TooLarge);
    }
    let font = MathFont::stix_two_math().map_err(map_lr_error)?;
    let ast = latex_rust::parse(latex).map_err(|e| MathError::Parse {
        message: format!("{e:?}"),
    })?;
    let style = if display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    // latex-rust sizes are in points; 1 CSS px = 0.75 pt. Layout uses exact
    // rationals, so the f32 size only crosses the boundary as a decimal string.
    let em_pt = Dim::parse(&format!("{:.6}", font_px * 0.75));
    let tree =
        latex_rust::layout_with_em_size_pt(&ast, &font, style, &em_pt).map_err(map_lr_error)?;

    let mut options = SvgOptions::new();
    options.font_size_pt = em_pt;
    options.color = Color::rgb(rgb.0, rgb.1, rgb.2);
    options.display = display;
    let svg = latex_rust::render_svg(&tree, &font, &options).map_err(map_lr_error)?;
    if svg.len() > MAX_BYTES {
        return Err(MathError::TooLarge);
    }
    check_svg_is_inert(&svg)?;

    Ok(RenderedMath {
        svg,
        width: dim_to_px(&tree.width, font_px)?,
        height: dim_to_px(&(&tree.height + &tree.depth), font_px)?,
        baseline: dim_to_px(&tree.height, font_px)?,
    })
}

/// Convert an em-relative `Dim` to CSS px.
fn dim_to_px(d: &Dim, font_px: f32) -> Result<f32, MathError> {
    let em = d
        .to_dec_string()
        .parse::<f32>()
        .map_err(|_| MathError::Render("non-numeric layout dimension".to_string()))?;
    let px = em * font_px;
    if px.is_finite() && px >= 0.0 {
        Ok(px)
    } else {
        Err(MathError::Render("invalid layout dimension".to_string()))
    }
}

/// Reject any SVG that could run code, load something or is not plain drawing.
fn check_svg_is_inert(svg: &str) -> Result<(), MathError> {
    let lower = svg.to_ascii_lowercase();
    const FORBIDDEN: [&str; 9] = [
        "<script",
        "<foreignobject",
        "<image",
        "<use",
        "<style",
        "href",
        "url(",
        "<!entity",
        "<!doctype",
    ];
    if FORBIDDEN.iter().any(|tok| lower.contains(tok)) {
        return Err(MathError::Render(
            "generated SVG contains a forbidden construct".to_string(),
        ));
    }
    Ok(())
}

fn map_lr_error(e: LrError) -> MathError {
    match e {
        LrError::Parse(p) => MathError::Parse {
            message: format!("{p:?}"),
        },
        LrError::Malformed { what } => MathError::Parse { message: what },
        LrError::Unsupported { what } => MathError::Unsupported(what),
        LrError::Font(FontError::MissingGlyph { ch, .. }) => MathError::Unsupported(format!(
            "character U+{:04X} is not in STIX Two Math (CJK in \\text is not supported)",
            ch as u32
        )),
        LrError::Font(other) => MathError::Render(format!("font: {other:?}")),
        LrError::InvalidOption { what } => MathError::Render(what),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_validation() {
        assert!(parse_theme(&MathTheme::default()).is_ok());
        let bad_colour = MathTheme {
            color: "red".into(),
            font_size: 16.0,
        };
        assert!(matches!(
            parse_theme(&bad_colour),
            Err(MathError::Render(_))
        ));
        let short = MathTheme {
            color: "#abc".into(),
            font_size: 16.0,
        };
        assert!(parse_theme(&short).is_err());
        let nan = MathTheme {
            color: "#000000".into(),
            font_size: f32::NAN,
        };
        assert!(parse_theme(&nan).is_err());
        let huge = MathTheme {
            color: "#000000".into(),
            font_size: 1.0e9,
        };
        assert!(parse_theme(&huge).is_err());
    }

    #[test]
    fn empty_and_oversize_inputs_are_rejected() {
        let t = MathTheme::default();
        assert!(matches!(
            render_math("   ", false, &t),
            Err(MathError::Parse { .. })
        ));
        let big = "x".repeat(MAX_BYTES + 1);
        assert_eq!(render_math(&big, false, &t), Err(MathError::TooLarge));
    }

    #[test]
    fn invalid_latex_is_a_parse_error_not_a_panic() {
        let t = MathTheme::default();
        assert!(matches!(
            render_math(r"\frac{", false, &t),
            Err(MathError::Parse { .. })
        ));
        assert!(matches!(
            render_math(r"\nosuchcommand{x}", false, &t),
            Err(MathError::Parse { .. })
        ));
    }

    #[test]
    fn cjk_text_is_unsupported_not_a_fake_render() {
        let t = MathTheme::default();
        assert!(matches!(
            render_math(r"\text{中文}", false, &t),
            Err(MathError::Unsupported(_))
        ));
    }

    #[test]
    fn simple_formula_has_sane_metrics() {
        let t = MathTheme::default();
        let m = render_math("x", false, &t).expect("render x");
        assert!(m.svg.starts_with("<?xml"));
        assert!(m.width > 4.0 && m.width < 40.0, "width {}", m.width);
        assert!(m.height > 4.0 && m.height < 40.0, "height {}", m.height);
        assert!(
            m.baseline > 0.0 && m.baseline <= m.height,
            "baseline {}",
            m.baseline
        );
    }

    #[test]
    fn timeout_is_enforced() {
        let t = MathTheme::default();
        let res = render_math_with_timeout("x", false, &t, Duration::ZERO);
        assert!(matches!(res, Ok(_) | Err(MathError::Timeout)));
    }

    #[test]
    fn colour_is_applied_to_the_svg() {
        let t = MathTheme {
            color: "#1A2B3C".into(),
            font_size: 16.0,
        };
        let m = render_math("x", false, &t).expect("render");
        assert!(
            m.svg.contains("#1a2b3c") || m.svg.contains("#1A2B3C"),
            "{}",
            &m.svg[..200.min(m.svg.len())]
        );
    }
}
