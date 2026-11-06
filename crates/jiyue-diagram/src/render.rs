use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use mermaid_rs_renderer::{LayoutConfig, ParseError, RenderOptions};
use serde::{Deserialize, Serialize};

use crate::error::DiagramError;
use crate::kind::{detect_kind, header_word_with_line};
use crate::sanitize::sanitize_svg;
use crate::theme::{DiagramTheme, Resolved};

/// Largest accepted source, in bytes.
pub const MAX_SOURCE_BYTES: usize = 100 * 1024;
/// Largest accepted number of source lines.
pub const MAX_LINES: usize = 2_000;
/// Largest accepted number of distinct identifier-like tokens (a cheap node-count proxy).
pub const MAX_NODES: usize = 800;
/// Default wall-clock budget for one render.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

const WORKER_STACK_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIMENSION: f32 = 20_000.0;

/// A self-contained SVG and its natural size in px.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderedDiagram {
    /// Sanitised SVG document (no scripts, foreign objects or external references).
    pub svg: String,
    /// Width in px, from the root `width` (or `viewBox`).
    pub width: f32,
    /// Height in px, from the root `height` (or `viewBox`).
    pub height: f32,
}

/// Renders Mermaid `source` with the default 3 s timeout.
///
/// Never panics. The work runs on a worker thread; see [`render_mermaid_with_timeout`].
pub fn render_mermaid(source: &str, theme: &DiagramTheme) -> Result<RenderedDiagram, DiagramError> {
    render_mermaid_with_timeout(source, theme, DEFAULT_TIMEOUT)
}

/// Renders Mermaid `source`, giving up after `timeout`.
///
/// Size and node limits are checked before any work starts. After a timeout the
/// worker thread is detached and finishes in the background; its result is dropped.
pub fn render_mermaid_with_timeout(
    source: &str,
    theme: &DiagramTheme,
    timeout: Duration,
) -> Result<RenderedDiagram, DiagramError> {
    precheck(source)?;
    let resolved = theme.resolve();
    let owned = source.to_owned();
    let (tx, rx) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("jiyue-mermaid".into())
        .stack_size(WORKER_STACK_BYTES)
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| render_inner(&owned, &resolved)))
                .unwrap_or_else(|_| {
                    Err(DiagramError::Render(
                        "the renderer hit an internal error".into(),
                    ))
                });
            // The receiver may already be gone after a timeout; that is fine.
            let _ = tx.send(result);
        });
    if let Err(e) = spawned {
        return Err(DiagramError::Render(format!(
            "could not start render thread: {e}"
        )));
    }
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(DiagramError::Timeout),
        Err(RecvTimeoutError::Disconnected) => Err(DiagramError::Render(
            "render thread stopped unexpectedly".into(),
        )),
    }
}

/// Cheap checks that run on the caller's thread before any rendering.
pub(crate) fn precheck(source: &str) -> Result<(), DiagramError> {
    if source.trim().is_empty() {
        return Err(DiagramError::Parse {
            message: "the diagram is empty".into(),
            line: None,
        });
    }
    if source.len() > MAX_SOURCE_BYTES || source.lines().count() > MAX_LINES {
        return Err(DiagramError::TooLarge);
    }
    if distinct_tokens(source) > MAX_NODES {
        return Err(DiagramError::TooLarge);
    }
    let Some(kind) = detect_kind(source) else {
        let line = header_word_with_line(source).map(|(_, line)| line);
        return Err(DiagramError::Parse {
            message: "no known diagram type; start with a header such as `flowchart TD`, `sequenceDiagram` or `pie`".into(),
            line,
        });
    };
    if !kind.is_supported() {
        return Err(DiagramError::Unsupported(format!(
            "{} diagrams",
            kind.name()
        )));
    }
    Ok(())
}

/// Number of distinct identifier-like tokens (letters, digits, `_`, CJK) in `source`.
fn distinct_tokens(source: &str) -> usize {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut start: Option<usize> = None;
    for (idx, ch) in source.char_indices() {
        let is_word = ch.is_alphanumeric() || ch == '_';
        match (is_word, start) {
            (true, None) => start = Some(idx),
            (false, Some(s)) => {
                seen.insert(&source[s..idx]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        seen.insert(&source[s..]);
    }
    seen.len()
}

fn render_inner(source: &str, resolved: &Resolved) -> Result<RenderedDiagram, DiagramError> {
    let options = RenderOptions {
        theme: resolved.to_mermaid(),
        layout: LayoutConfig::default(),
    };
    let raw = mermaid_rs_renderer::render_strict(source, options).map_err(map_parse_error)?;
    let svg = sanitize_svg(&resolved.remap_backend_literals(&raw))?;
    let (width, height) = read_dimensions(&svg)?;
    Ok(RenderedDiagram { svg, width, height })
}

pub(crate) fn map_parse_error(err: ParseError) -> DiagramError {
    match err {
        ParseError::UnknownParticipant { name, line, .. } => DiagramError::Parse {
            message: format!("'{name}' is used before it is declared"),
            line: Some(line as usize),
        },
        ParseError::UnclosedSubgraph { opened_at } => DiagramError::Parse {
            message: "a block is opened here but never closed with `end`".into(),
            line: Some(opened_at as usize),
        },
        ParseError::UnexpectedToken {
            line,
            col,
            found,
            expected,
        } => DiagramError::Parse {
            message: format!("unexpected '{found}', expected {expected}"),
            // The backend reports line 1 / column 1 when it cannot locate the error.
            line: if line == 1 && col == 1 {
                None
            } else {
                Some(line as usize)
            },
        },
        ParseError::InvalidDirective { line, reason, .. } => DiagramError::Parse {
            message: reason,
            line: Some(line as usize),
        },
        other => DiagramError::Parse {
            message: other.to_string(),
            line: None,
        },
    }
}

/// Reads the natural size from the root element; falls back to `viewBox`.
pub(crate) fn read_dimensions(svg: &str) -> Result<(f32, f32), DiagramError> {
    let doc = roxmltree::Document::parse(svg)
        .map_err(|e| DiagramError::Render(format!("invalid SVG: {e}")))?;
    let root = doc.root_element();
    let view_box: Option<(f32, f32)> = root.attribute("viewBox").and_then(|vb| {
        let nums: Vec<f32> = vb
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f32>().ok())
            .collect();
        match nums.as_slice() {
            [_, _, w, h] => Some((*w, *h)),
            _ => None,
        }
    });
    let attr = |name: &str| root.attribute(name).and_then(parse_length);
    let width = attr("width").or(view_box.map(|(w, _)| w));
    let height = attr("height").or(view_box.map(|(_, h)| h));
    match (width, height) {
        (Some(w), Some(h))
            if w.is_finite()
                && h.is_finite()
                && w > 0.0
                && h > 0.0
                && w <= MAX_DIMENSION
                && h <= MAX_DIMENSION =>
        {
            Ok((w, h))
        }
        _ => Err(DiagramError::Parse {
            message: "the diagram has no content".into(),
            line: None,
        }),
    }
}

fn parse_length(value: &str) -> Option<f32> {
    let trimmed = value.trim().trim_end_matches("px").trim();
    trimmed.parse::<f32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_is_a_parse_error() {
        let err = render_mermaid("   \n\t", &DiagramTheme::light());
        assert!(matches!(err, Err(DiagramError::Parse { line: None, .. })));
    }

    #[test]
    fn oversized_source_is_too_large() {
        let big = format!("flowchart LR\n{}", "A-->B\n".repeat(30_000));
        assert!(big.len() > MAX_SOURCE_BYTES);
        assert_eq!(
            render_mermaid(&big, &DiagramTheme::light()),
            Err(DiagramError::TooLarge)
        );
    }

    #[test]
    fn too_many_lines_is_too_large() {
        let many = format!("flowchart LR\n{}", "%% x\n".repeat(MAX_LINES + 10));
        assert!(many.len() < MAX_SOURCE_BYTES);
        assert_eq!(
            render_mermaid(&many, &DiagramTheme::light()),
            Err(DiagramError::TooLarge)
        );
    }

    #[test]
    fn node_guard_rejects_huge_graphs() {
        let mut src = String::from("flowchart LR\n");
        for i in 0..(MAX_NODES + 50) {
            src.push_str(&format!("n{i} --> n{}\n", i + 1));
        }
        assert!(src.len() < MAX_SOURCE_BYTES);
        assert_eq!(
            render_mermaid(&src, &DiagramTheme::light()),
            Err(DiagramError::TooLarge)
        );
    }

    #[test]
    fn garbage_is_a_friendly_parse_error_with_line() {
        let err = render_mermaid("%% comment\nhello there", &DiagramTheme::light());
        match err {
            Err(DiagramError::Parse { message, line }) => {
                assert!(message.contains("diagram type"), "{message}");
                assert_eq!(line, Some(2));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unsupported_kind_is_reported() {
        let err = render_mermaid("zenuml\n A->B: x", &DiagramTheme::light());
        assert!(matches!(err, Err(DiagramError::Unsupported(_))), "{err:?}");
    }

    #[test]
    fn distinct_token_counter_handles_cjk_and_ascii() {
        assert_eq!(distinct_tokens("A --> B\nA --> 中文"), 3);
        assert_eq!(distinct_tokens(""), 0);
        assert_eq!(distinct_tokens("a_b c"), 2);
    }

    #[test]
    fn parse_length_accepts_px_and_rejects_percent() {
        assert_eq!(parse_length("120px"), Some(120.0));
        assert_eq!(parse_length(" 42.5 "), Some(42.5));
        assert_eq!(parse_length("100%"), None);
    }

    #[test]
    fn dimensions_fall_back_to_viewbox() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 300 150\"></svg>";
        assert_eq!(read_dimensions(svg).ok(), Some((300.0, 150.0)));
        let bad = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100%\" height=\"0\"></svg>";
        assert!(read_dimensions(bad).is_err());
    }

    #[test]
    fn dimensions_prefer_explicit_attributes() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200px\" height=\"80\" viewBox=\"0 0 1 1\"/>";
        assert_eq!(read_dimensions(svg).ok(), Some((200.0, 80.0)));
    }

    #[test]
    fn parse_error_mapping_keeps_known_lines() {
        let mapped = map_parse_error(ParseError::UnclosedSubgraph { opened_at: 4 });
        assert_eq!(
            mapped,
            DiagramError::Parse {
                message: "a block is opened here but never closed with `end`".into(),
                line: Some(4),
            }
        );
        let generic = map_parse_error(ParseError::UnexpectedToken {
            line: 1,
            col: 1,
            found: "x".into(),
            expected: "y".into(),
        });
        assert!(matches!(generic, DiagramError::Parse { line: None, .. }));
    }
}
