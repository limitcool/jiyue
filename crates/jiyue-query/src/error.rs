//! Query errors with a position and an optional suggestion.

use std::fmt;
use std::ops::Range;

/// A query error. `line` and `column` are 1-based; `column` counts characters, not bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    /// Byte range of the offending text in the query source.
    pub start: usize,
    pub end: usize,
    /// A hint such as "did you mean `sort`?".
    pub suggestion: Option<String>,
}

impl ParseError {
    pub(crate) fn at(
        src: &str,
        span: Range<usize>,
        message: impl Into<String>,
        suggestion: Option<String>,
    ) -> Self {
        let start = floor_boundary(src, span.start);
        let end = floor_boundary(src, span.end).max(start);
        let (line, column) = position(src, start);
        ParseError {
            message: message.into(),
            line,
            column,
            start,
            end,
            suggestion,
        }
    }
}

/// Largest char boundary that is `<= idx` (and `<= src.len()`).
pub(crate) fn floor_boundary(src: &str, idx: usize) -> usize {
    if idx >= src.len() {
        return src.len();
    }
    let mut i = idx;
    while i > 0 && !src.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 1-based (line, column) of a byte offset.
pub(crate) fn position(src: &str, byte: usize) -> (usize, usize) {
    let upto = src.get(..byte).unwrap_or(src);
    let line = upto.matches('\n').count() + 1;
    let line_start = upto.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let column = upto
        .get(line_start..)
        .map(|s| s.chars().count())
        .unwrap_or(0)
        + 1;
    (line, column)
}

/// The candidate closest to `name` (edit distance within a third of its length).
pub(crate) fn nearest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(usize, &'a str)> = None;
    for &c in candidates {
        let d = levenshtein(name, c);
        let limit = (c.chars().count() / 3).max(1);
        if d <= limit && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.get(b.len()).copied().unwrap_or(usize::MAX)
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )?;
        if let Some(s) = &self.suggestion {
            write!(f, " (hint: {s})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_counts_lines_and_chars() {
        let src = "from all\nwhere 项目 = 1";
        let at = src.find('=').unwrap_or(0);
        assert_eq!(position(src, at), (2, 10));
        assert_eq!(position(src, 0), (1, 1));
    }

    #[test]
    fn floor_boundary_never_splits_chars() {
        let s = "项目";
        assert_eq!(floor_boundary(s, 1), 0);
        assert_eq!(floor_boundary(s, 3), 3);
        assert_eq!(floor_boundary(s, 99), s.len());
    }

    #[test]
    fn display_includes_hint() {
        let e = ParseError::at("sor x", 0..3, "unknown clause `sor`", Some("sort".into()));
        assert_eq!(
            e.to_string(),
            "line 1, column 1: unknown clause `sor` (hint: sort)"
        );
    }
}
