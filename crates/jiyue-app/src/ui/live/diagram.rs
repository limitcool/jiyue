//! Mermaid diagrams of the live preview (pure logic, no gpui).
//!
//! A fenced block whose info string is `mermaid` is a block widget (`map::BlockKind::Mermaid`).
//! This module reads the block source, decides whether the diagram kind can be drawn at all
//! ([`support`]), turns a render error into a short message a person can read ([`error_message`]),
//! and sizes the picture for the column ([`fit`], [`raster_scale`]). The render itself runs on the
//! background executor (see `context::ensure_diagrams`), and the cache key is [`DiagramKey`].

use jiyue_diagram::{DiagramError, DiagramKind, detect_kind};

use super::blocks;

/// Height of the reserved box while a diagram renders (the picture replaces it once).
pub const LOADING_H: f32 = 120.0;
/// Tallest a diagram may be drawn, in CSS px. A taller one is scaled down to it.
pub const MAX_CSS_H: f32 = 640.0;
/// Pixels per CSS px of the raster (crisp on a 2x display).
pub const RASTER_DPI: f32 = 2.0;
/// Most diagrams kept in the cache; the oldest finished ones are dropped beyond this.
pub const CACHE_MAX: usize = 48;

/// Cache key of one rendered diagram: the source text, the theme (a hash of its colours) and the
/// column width it was rasterised for (in 8 px steps, see [`width_bucket`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiagramKey {
    pub source: String,
    pub theme: u64,
    pub width: u32,
}

/// Whether a fence info string names Mermaid (`mermaid`, any case, first word only).
pub fn is_mermaid_lang(lang: &str) -> bool {
    lang.trim().eq_ignore_ascii_case("mermaid")
}

/// Whether a whole fenced block (fences included) is a Mermaid block.
pub fn is_mermaid_block(block: &str) -> bool {
    is_mermaid_lang(&blocks::split_fenced(block).0)
}

/// The Mermaid source of a fenced block: the lines between its fences, without the fences.
pub fn source_of(block: &str) -> String {
    let (_, body) = blocks::split_fenced(block);
    block.get(body).unwrap_or("").to_string()
}

/// Whether the diagram can be drawn. Some kinds render badly in this renderer; they get a card
/// that says so and offers the source, instead of a broken picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Render,
    /// The diagram kind's label (Chinese), for the card title.
    Unsupported(&'static str),
}

/// Support of a diagram source. `dark` is the theme kind: Sankey is only poor in the dark theme.
pub fn support(source: &str, dark: bool) -> Support {
    match detect_kind(source) {
        Some(DiagramKind::Requirement) => Support::Unsupported("需求图"),
        Some(DiagramKind::Block) => Support::Unsupported("块图"),
        Some(DiagramKind::Kanban) => Support::Unsupported("看板图"),
        Some(DiagramKind::Radar) => Support::Unsupported("雷达图"),
        Some(DiagramKind::Sankey) if dark => Support::Unsupported("桑基图"),
        _ => Support::Render,
    }
}

/// A short message for a render error. Never the Debug form of the error.
pub fn error_message(err: &DiagramError) -> String {
    match err {
        DiagramError::Parse { line: Some(n), .. } => format!("第 {n} 行有语法错误"),
        DiagramError::Parse { line: None, .. } => "图表语法错误，请检查图表类型和写法".to_string(),
        DiagramError::Unsupported(_) => "此图表类型暂不支持预览".to_string(),
        DiagramError::TooLarge => "图表太大，无法显示".to_string(),
        DiagramError::Timeout => "渲染超时".to_string(),
        DiagramError::Render(_) => "渲染失败".to_string(),
    }
}

/// The first `n` source lines, each cut to `max_chars` characters (with an ellipsis mark).
pub fn first_lines(source: &str, n: usize, max_chars: usize) -> Vec<String> {
    source
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(n)
        .map(|l| {
            if l.chars().count() > max_chars {
                let cut: String = l.chars().take(max_chars).collect();
                format!("{cut}…")
            } else {
                l.to_string()
            }
        })
        .collect()
}

/// The column width a diagram is rasterised for: the column rounded to 8 px, kept in a sane range.
pub fn width_bucket(column: f32) -> u32 {
    let steps = (column / 8.0).round().max(1.0) as u32;
    (steps * 8).clamp(64, 4096)
}

/// CSS size of a diagram of natural size `natural` drawn in a column of `column` width: never
/// enlarged, scaled down to the column and to [`MAX_CSS_H`] (the aspect ratio is kept). `None`
/// for a zero or invalid natural size.
pub fn fit(natural: (f32, f32), column: f32) -> Option<(f32, f32)> {
    let (w, h) = natural;
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    // `min` ignores NaN, so the value clamped here is never NaN.
    let s = (column / w).min(MAX_CSS_H / h).clamp(0.0, 1.0);
    Some((w * s, h * s))
}

/// Raster scale for a diagram drawn at `drawn_w` CSS px wide from a natural width `natural_w`:
/// [`RASTER_DPI`] times the drawn size, clamped to what the rasteriser accepts.
pub fn raster_scale(natural_w: f32, drawn_w: f32) -> f32 {
    if natural_w <= 0.0 {
        return RASTER_DPI;
    }
    (RASTER_DPI * drawn_w / natural_w).clamp(0.05, 8.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_mermaid_fence() {
        assert!(is_mermaid_block("```mermaid\ngraph TD\n A-->B\n```"));
        assert!(is_mermaid_block("```Mermaid title=x\nflowchart LR\n```"));
        assert!(!is_mermaid_block("```rust\nfn main() {}\n```"));
        assert!(is_mermaid_lang(" mermaid "));
    }

    #[test]
    fn source_is_the_body_between_the_fences() {
        let block = "```mermaid\nflowchart LR\n  A-->B\n```";
        assert_eq!(source_of(block), "flowchart LR\n  A-->B");
        assert_eq!(source_of("```mermaid\n```"), "");
    }

    #[test]
    fn support_flags_the_poor_kinds() {
        assert_eq!(
            support("requirementDiagram\n requirement r {}", true),
            Support::Unsupported("需求图")
        );
        assert_eq!(
            support("block-beta\n columns 2", false),
            Support::Unsupported("块图")
        );
        assert_eq!(
            support("kanban\n Todo", false),
            Support::Unsupported("看板图")
        );
        assert_eq!(
            support("radar-beta\n axis a", false),
            Support::Unsupported("雷达图")
        );
        assert_eq!(
            support("sankey-beta\n a,b,1", true),
            Support::Unsupported("桑基图")
        );
        assert_eq!(support("sankey-beta\n a,b,1", false), Support::Render);
        assert_eq!(support("flowchart TD\n A-->B", true), Support::Render);
        assert_eq!(
            support("sequenceDiagram\n A->>B: hi", true),
            Support::Render
        );
    }

    #[test]
    fn error_messages_are_human_and_never_debug() {
        let parse = DiagramError::Parse {
            message: "unexpected 'x'".into(),
            line: Some(4),
        };
        assert_eq!(error_message(&parse), "第 4 行有语法错误");
        let no_line = DiagramError::Parse {
            message: "bad".into(),
            line: None,
        };
        assert!(!error_message(&no_line).contains("Parse"));
        for e in [
            DiagramError::TooLarge,
            DiagramError::Timeout,
            DiagramError::Render("x".into()),
            DiagramError::Unsupported("C4".into()),
        ] {
            let m = error_message(&e);
            assert!(!m.is_empty());
            assert!(!m.contains("Render(") && !m.contains("Unsupported("));
        }
    }

    #[test]
    fn first_lines_skip_blanks_and_cut_long_lines() {
        let src = "flowchart TD\n\n  A --> B\nC\nD";
        assert_eq!(first_lines(src, 2, 80), vec!["flowchart TD", "  A --> B"]);
        let long = "x".repeat(10);
        assert_eq!(first_lines(&long, 1, 4), vec!["xxxx…"]);
    }

    #[test]
    fn width_buckets_round_to_eight() {
        assert_eq!(width_bucket(624.0), 624);
        assert_eq!(width_bucket(613.0), 616);
        assert_eq!(width_bucket(1.0), 64);
        assert_eq!(width_bucket(100_000.0), 4096);
    }

    #[test]
    fn fit_never_enlarges_and_respects_both_limits() {
        assert_eq!(fit((300.0, 200.0), 620.0), Some((300.0, 200.0)));
        assert_eq!(fit((1240.0, 400.0), 620.0), Some((620.0, 200.0)));
        let (w, h) = fit((400.0, 4000.0), 620.0).unwrap();
        assert!((h - MAX_CSS_H).abs() < 0.01);
        assert!((w - 64.0).abs() < 0.01);
        assert_eq!(fit((0.0, 10.0), 620.0), None);
        assert_eq!(fit((f32::NAN, 10.0), 620.0), None);
    }

    #[test]
    fn raster_scale_is_two_times_the_drawn_size() {
        assert!((raster_scale(1000.0, 500.0) - 1.0).abs() < 1e-6);
        assert!((raster_scale(300.0, 300.0) - 2.0).abs() < 1e-6);
        assert_eq!(raster_scale(0.0, 10.0), RASTER_DPI);
    }
}
