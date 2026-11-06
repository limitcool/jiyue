//! Tokenizer for the query language. Every token carries its byte span for error messages.

use std::ops::Range;

use crate::error::ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kw {
    From,
    Where,
    Select,
    Sort,
    Group,
    By,
    Limit,
    And,
    Or,
    Not,
    In,
    Contains,
    Has,
    As,
    Asc,
    Desc,
    True,
    False,
    Null,
    All,
}

impl Kw {
    pub fn text(self) -> &'static str {
        match self {
            Kw::From => "from",
            Kw::Where => "where",
            Kw::Select => "select",
            Kw::Sort => "sort",
            Kw::Group => "group",
            Kw::By => "by",
            Kw::Limit => "limit",
            Kw::And => "and",
            Kw::Or => "or",
            Kw::Not => "not",
            Kw::In => "in",
            Kw::Contains => "contains",
            Kw::Has => "has",
            Kw::As => "as",
            Kw::Asc => "asc",
            Kw::Desc => "desc",
            Kw::True => "true",
            Kw::False => "false",
            Kw::Null => "null",
            Kw::All => "all",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A bare name. It may contain CJK characters.
    Ident(String),
    /// A name in backticks, for names with spaces or symbols. Never a keyword.
    Quoted(String),
    Str(String),
    Num(f64),
    /// A number with a unit suffix, such as `7d`.
    Dur(f64, String),
    Kw(Kw),
    Op(Op),
    LParen,
    RParen,
    LBrack,
    RBrack,
    Comma,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Range<usize>,
}

fn keyword(s: &str) -> Option<Kw> {
    Some(match s.to_ascii_lowercase().as_str() {
        "from" => Kw::From,
        "where" => Kw::Where,
        "select" => Kw::Select,
        "sort" => Kw::Sort,
        "group" => Kw::Group,
        "by" => Kw::By,
        "limit" => Kw::Limit,
        "and" => Kw::And,
        "or" => Kw::Or,
        "not" => Kw::Not,
        "in" => Kw::In,
        "contains" => Kw::Contains,
        "has" => Kw::Has,
        "as" => Kw::As,
        "asc" => Kw::Asc,
        "desc" => Kw::Desc,
        "true" => Kw::True,
        "false" => Kw::False,
        "null" => Kw::Null,
        "all" => Kw::All,
        _ => return None,
    })
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Splits a query into tokens. The last token is always `Tok::Eof`.
pub fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let n = chars.len();
    let byte_at = |i: usize| chars.get(i).map(|(b, _)| *b).unwrap_or(src.len());
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let (start, c) = chars[i];
        let next = chars.get(i + 1).map(|(_, c)| *c);
        let single = move |tok: Tok, len: usize| (tok, byte_at(i)..byte_at(i + len));
        let (tok, span, consumed) = match c {
            _ if c.is_whitespace() => {
                i += 1;
                continue;
            }
            '(' => {
                let (t, s) = single(Tok::LParen, 1);
                (t, s, 1)
            }
            ')' => {
                let (t, s) = single(Tok::RParen, 1);
                (t, s, 1)
            }
            '[' => {
                let (t, s) = single(Tok::LBrack, 1);
                (t, s, 1)
            }
            ']' => {
                let (t, s) = single(Tok::RBrack, 1);
                (t, s, 1)
            }
            ',' => {
                let (t, s) = single(Tok::Comma, 1);
                (t, s, 1)
            }
            '+' => {
                let (t, s) = single(Tok::Op(Op::Add), 1);
                (t, s, 1)
            }
            '-' => {
                let (t, s) = single(Tok::Op(Op::Sub), 1);
                (t, s, 1)
            }
            '*' => {
                let (t, s) = single(Tok::Op(Op::Mul), 1);
                (t, s, 1)
            }
            '/' => {
                let (t, s) = single(Tok::Op(Op::Div), 1);
                (t, s, 1)
            }
            '=' => {
                let len = if next == Some('=') { 2 } else { 1 };
                let (t, s) = single(Tok::Op(Op::Eq), len);
                (t, s, len)
            }
            '!' => {
                if next == Some('=') {
                    let (t, s) = single(Tok::Op(Op::Ne), 2);
                    (t, s, 2)
                } else {
                    return Err(ParseError::at(
                        src,
                        start..byte_at(i + 1),
                        "unexpected `!`",
                        Some("write `not` to negate a condition, or `!=` to compare".into()),
                    ));
                }
            }
            '<' => {
                let len = if next == Some('=') { 2 } else { 1 };
                let op = if len == 2 { Op::Le } else { Op::Lt };
                let (t, s) = single(Tok::Op(op), len);
                (t, s, len)
            }
            '>' => {
                let len = if next == Some('=') { 2 } else { 1 };
                let op = if len == 2 { Op::Ge } else { Op::Gt };
                let (t, s) = single(Tok::Op(op), len);
                (t, s, len)
            }
            '"' | '\'' => {
                let (s, len) = lex_string(src, &chars, i)?;
                let span = start..byte_at(i + len);
                (Tok::Str(s), span, len)
            }
            '`' => {
                let mut j = i + 1;
                let mut name = String::new();
                loop {
                    match chars.get(j) {
                        Some((_, '`')) => break,
                        Some((_, ch)) => name.push(*ch),
                        None => {
                            return Err(ParseError::at(
                                src,
                                start..src.len(),
                                "unterminated name in backticks",
                                Some("add a closing backtick, for example `due date`".into()),
                            ));
                        }
                    }
                    j += 1;
                }
                if name.is_empty() {
                    return Err(ParseError::at(
                        src,
                        start..byte_at(j + 1),
                        "empty name in backticks",
                        None,
                    ));
                }
                let span = start..byte_at(j + 1);
                (Tok::Quoted(name), span, j + 1 - i)
            }
            d if d.is_ascii_digit() => {
                let mut j = i;
                let mut text = String::new();
                while let Some((_, ch)) = chars.get(j) {
                    if ch.is_ascii_digit() {
                        text.push(*ch);
                        j += 1;
                    } else {
                        break;
                    }
                }
                if chars.get(j).map(|(_, c)| *c) == Some('.')
                    && chars.get(j + 1).is_some_and(|(_, c)| c.is_ascii_digit())
                {
                    text.push('.');
                    j += 1;
                    while let Some((_, ch)) = chars.get(j) {
                        if ch.is_ascii_digit() {
                            text.push(*ch);
                            j += 1;
                        } else {
                            break;
                        }
                    }
                }
                let value: f64 = text.parse().unwrap_or(f64::NAN);
                let mut unit = String::new();
                while let Some((_, ch)) = chars.get(j) {
                    if ch.is_ascii_alphabetic() {
                        unit.push(*ch);
                        j += 1;
                    } else {
                        break;
                    }
                }
                let span = start..byte_at(j);
                let tok = if unit.is_empty() {
                    Tok::Num(value)
                } else {
                    Tok::Dur(value, unit)
                };
                (tok, span, j - i)
            }
            c if is_ident_start(c) => {
                let mut j = i;
                let mut name = String::new();
                while let Some((_, ch)) = chars.get(j) {
                    if is_ident_char(*ch) {
                        name.push(*ch);
                        j += 1;
                    } else {
                        break;
                    }
                }
                let span = start..byte_at(j);
                let tok = match keyword(&name) {
                    Some(k) => Tok::Kw(k),
                    None => Tok::Ident(name),
                };
                (tok, span, j - i)
            }
            '#' => {
                return Err(ParseError::at(
                    src,
                    start..byte_at(i + 1),
                    "unexpected `#`",
                    Some("select tags with tag(\"name\") instead of #name".into()),
                ));
            }
            other => {
                return Err(ParseError::at(
                    src,
                    start..byte_at(i + 1),
                    format!("unexpected character `{other}`"),
                    None,
                ));
            }
        };
        out.push(Token { tok, span });
        i += consumed.max(1);
    }
    out.push(Token {
        tok: Tok::Eof,
        span: src.len()..src.len(),
    });
    Ok(out)
}

/// Lexes a quoted string starting at `chars[i]`. Returns the value and the number of
/// characters consumed, including both quotes.
fn lex_string(src: &str, chars: &[(usize, char)], i: usize) -> Result<(String, usize), ParseError> {
    let quote = chars.get(i).map(|(_, c)| *c).unwrap_or('"');
    let start = chars.get(i).map(|(b, _)| *b).unwrap_or(0);
    let mut value = String::new();
    let mut j = i + 1;
    loop {
        let Some((pos, c)) = chars.get(j).copied() else {
            return Err(ParseError::at(
                src,
                start..src.len(),
                "unterminated string",
                Some(format!("add a closing {quote}")),
            ));
        };
        if c == quote {
            return Ok((value, j + 1 - i));
        }
        if c == '\\' {
            let esc = chars.get(j + 1).map(|(_, c)| *c);
            let mapped = match esc {
                Some('n') => '\n',
                Some('t') => '\t',
                Some('r') => '\r',
                Some('\\') => '\\',
                Some('"') => '"',
                Some('\'') => '\'',
                _ => {
                    let end = chars.get(j + 2).map(|(b, _)| *b).unwrap_or(src.len());
                    return Err(ParseError::at(
                        src,
                        pos..end,
                        "invalid escape sequence in string",
                        Some("valid escapes are \\n \\t \\r \\\\ \\\" and \\'".into()),
                    ));
                }
            };
            value.push(mapped);
            j += 2;
            continue;
        }
        value.push(c);
        j += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        lex(src)
            .map(|v| v.into_iter().map(|t| t.tok).collect())
            .unwrap_or_default()
    }

    #[test]
    fn keywords_are_case_insensitive() {
        assert_eq!(
            toks("WHERE and"),
            vec![Tok::Kw(Kw::Where), Tok::Kw(Kw::And), Tok::Eof]
        );
    }

    #[test]
    fn durations_and_numbers() {
        assert_eq!(toks("7d 2.5 3")[0], Tok::Dur(7.0, "d".into()));
        assert_eq!(toks("7d 2.5 3")[1], Tok::Num(2.5));
        assert_eq!(toks("7d 2.5 3")[2], Tok::Num(3.0));
    }

    #[test]
    fn cjk_identifiers_and_backticks() {
        assert_eq!(toks("状态")[0], Tok::Ident("状态".into()));
        assert_eq!(toks("`due date`")[0], Tok::Quoted("due date".into()));
    }

    #[test]
    fn string_escapes() {
        assert_eq!(toks(r#""a\"b\n""#)[0], Tok::Str("a\"b\n".into()));
        assert_eq!(toks("'single'")[0], Tok::Str("single".into()));
    }

    #[test]
    fn errors_have_spans() {
        let e = lex("from \"abc").err();
        assert_eq!(e.map(|e| e.column), Some(6));
        let e = lex("a ! b").err();
        assert!(e.is_some_and(|e| e.message.contains('!')));
    }

    #[test]
    fn hash_gets_a_hint() {
        let e = lex("#rust").err();
        assert!(e.is_some_and(|e| e.suggestion.is_some_and(|s| s.contains("tag("))));
    }
}
