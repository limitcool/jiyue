use std::fmt;

/// Why a diagram could not be rendered. Every variant carries a message that is
/// safe to show to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagramError {
    /// The source is not valid Mermaid. `line` is 1-based when known.
    Parse {
        message: String,
        line: Option<usize>,
    },
    /// Valid-looking Mermaid of a kind this renderer does not support.
    Unsupported(String),
    /// Source exceeds the byte, line or node limits.
    TooLarge,
    /// Rendering did not finish within the timeout.
    Timeout,
    /// Internal renderer failure (panic, invalid SVG output, ...).
    Render(String),
}

impl fmt::Display for DiagramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse {
                message,
                line: Some(line),
            } => write!(f, "Mermaid syntax error on line {line}: {message}"),
            Self::Parse {
                message,
                line: None,
            } => write!(f, "Mermaid syntax error: {message}"),
            Self::Unsupported(what) => write!(f, "Unsupported diagram: {what}"),
            Self::TooLarge => write!(f, "Diagram is too large to render"),
            Self::Timeout => write!(f, "Diagram rendering timed out"),
            Self::Render(message) => write!(f, "Diagram rendering failed: {message}"),
        }
    }
}

impl std::error::Error for DiagramError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_line_when_known() {
        let e = DiagramError::Parse {
            message: "bad arrow".into(),
            line: Some(3),
        };
        assert_eq!(e.to_string(), "Mermaid syntax error on line 3: bad arrow");
    }

    #[test]
    fn display_without_line() {
        let e = DiagramError::Parse {
            message: "empty".into(),
            line: None,
        };
        assert_eq!(e.to_string(), "Mermaid syntax error: empty");
    }

    #[test]
    fn display_other_variants() {
        assert!(DiagramError::TooLarge.to_string().contains("too large"));
        assert!(DiagramError::Timeout.to_string().contains("timed out"));
        assert!(
            DiagramError::Unsupported("C4".into())
                .to_string()
                .contains("C4")
        );
        assert!(
            DiagramError::Render("x".into())
                .to_string()
                .contains("failed")
        );
    }

    #[test]
    fn is_std_error() {
        fn takes_error<E: std::error::Error>(_: &E) {}
        takes_error(&DiagramError::TooLarge);
    }
}
