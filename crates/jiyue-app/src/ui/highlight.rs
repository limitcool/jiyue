//! Small syntax highlighter for fenced code blocks (Rust-oriented, good enough for the design).

use std::ops::Range;

use gpui::{FontWeight, Hsla, SharedString, StyledText, TextRun, font};

use super::MONO_FONT;
use crate::theme::ActiveTheme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Kw,
    Str,
    Func,
    Comment,
    Ty,
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while",
];

const PRIMITIVES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64", "bool", "char", "str", "String",
];

fn classify_ident(word: &str, followed_by_paren: bool) -> Tok {
    if KEYWORDS.contains(&word) {
        Tok::Kw
    } else if followed_by_paren {
        Tok::Func
    } else if PRIMITIVES.contains(&word) || word.chars().next().is_some_and(|c| c.is_uppercase()) {
        Tok::Ty
    } else {
        Tok::Plain
    }
}

/// Splits one line of code into coloured byte ranges. Ranges cover the whole line.
pub fn tokenize(line: &str) -> Vec<(Range<usize>, Tok)> {
    let bytes = line.as_bytes();
    let mut out: Vec<(Range<usize>, Tok)> = Vec::new();
    let mut i = 0;

    let push = |out: &mut Vec<(Range<usize>, Tok)>, r: Range<usize>, tok: Tok| {
        if r.is_empty() {
            return;
        }
        if let Some(last) = out.last_mut()
            && last.1 == tok
            && last.0.end == r.start
        {
            last.0.end = r.end;
            return;
        }
        out.push((r, tok));
    };

    while i < bytes.len() {
        let c = bytes[i];
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            push(&mut out, i..bytes.len(), Tok::Comment);
            break;
        }
        if c == b'"' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(bytes.len());
            push(&mut out, i..end, Tok::Str);
            i = end;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let word = &line[i..j];
            let mut k = j;
            while k < bytes.len() && bytes[k] == b' ' {
                k += 1;
            }
            let followed = bytes.get(k) == Some(&b'(');
            let tok = classify_ident(word, followed);
            push(&mut out, i..j, tok);
            i = j;
            continue;
        }
        // Any other byte (including UTF-8 continuation bytes) is plain text.
        let width = line[i..].chars().next().map_or(1, |ch| ch.len_utf8());
        push(&mut out, i..i + width, Tok::Plain);
        i += width;
    }
    out
}

/// Builds a StyledText for one line of code using the theme's syntax colours.
pub fn code_line(line: &str, t: &ActiveTheme) -> StyledText {
    let text = if line.is_empty() { " " } else { line };
    let mut runs = Vec::new();
    if line.is_empty() {
        runs.push(run(1, t.tx, Tok::Plain, t));
    } else {
        for (range, tok) in tokenize(line) {
            runs.push(run(range.len(), t.tx, tok, t));
        }
    }
    StyledText::new(SharedString::from(text.to_string())).with_runs(runs)
}

/// Text runs for one line of code (they cover the line exactly). Empty for an empty line.
pub fn line_runs(line: &str, t: &ActiveTheme) -> Vec<TextRun> {
    tokenize(line)
        .into_iter()
        .map(|(range, tok)| run(range.len(), t.tx, tok, t))
        .collect()
}

fn run(len: usize, base: gpui::Rgba, tok: Tok, t: &ActiveTheme) -> TextRun {
    let color = match tok {
        Tok::Plain => base,
        Tok::Kw => t.kw,
        Tok::Str => t.st,
        Tok::Func => t.func,
        Tok::Comment => t.cm,
        Tok::Ty => t.ty,
    };
    let mut f = font(MONO_FONT.to_string());
    f.weight = FontWeight::NORMAL;
    if tok == Tok::Comment {
        f.style = gpui::FontStyle::Italic;
    }
    let color: Hsla = color.into();
    TextRun {
        len,
        font: f,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}
