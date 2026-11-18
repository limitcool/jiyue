//! Lightweight multi-language syntax highlighter for fenced code blocks (zero external deps).

use std::ops::Range;

use gpui::{FontWeight, Hsla, SharedString, StyledText, TextRun, font};

use crate::theme::ActiveTheme;
use crate::ui::fonts::{self, Face};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Plain,
    Kw,
    Str,
    Func,
    Comment,
    Ty,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LangFamily {
    Rust,
    Python,
    JsTs,
    Go,
    CFamily,
    Shell,
    Config,
    Sql,
    Plain,
}

fn classify_lang(lang: &str) -> LangFamily {
    match lang.trim().to_ascii_lowercase().as_str() {
        "" | "rs" | "rust" => LangFamily::Rust,
        "py" | "python" | "py3" => LangFamily::Python,
        "js" | "jsx" | "ts" | "tsx" | "javascript" | "typescript" | "mjs" | "cjs" => {
            LangFamily::JsTs
        }
        "go" | "golang" => LangFamily::Go,
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "cs" | "csharp" | "java" | "kt" | "kotlin"
        | "swift" | "dart" => LangFamily::CFamily,
        "sh" | "bash" | "zsh" | "shell" | "fish" | "ps1" | "pwsh" | "powershell" => {
            LangFamily::Shell
        }
        "toml" | "yaml" | "yml" | "json" | "jsonc" | "ini" => LangFamily::Config,
        "sql" | "sqlite" | "postgres" | "postgresql" | "mysql" => LangFamily::Sql,
        "text" | "txt" | "plain" | "plaintext" | "md" | "markdown" => LangFamily::Plain,
        _ => LangFamily::CFamily,
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while",
];

const RUST_PRIMITIVES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64", "bool", "char", "str", "String",
];

const PYTHON_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def", "del",
    "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in",
    "is", "lambda", "match", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True",
    "try", "while", "with", "yield",
];

const PYTHON_TYPES: &[&str] = &[
    "bool", "bytes", "cls", "dict", "float", "int", "list", "object", "self", "set", "str", "tuple",
];

const JSTS_KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];

const JSTS_TYPES: &[&str] = &[
    "any", "Array", "bigint", "boolean", "Map", "never", "number", "object", "Promise", "Record",
    "Set", "string", "symbol", "unknown",
];

const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "false",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "iota",
    "map",
    "nil",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "true",
    "type",
    "var",
];

const GO_TYPES: &[&str] = &[
    "any",
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
];

const C_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "defer",
    "do",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "for",
    "fun",
    "func",
    "guard",
    "if",
    "implements",
    "import",
    "in",
    "interface",
    "internal",
    "is",
    "late",
    "let",
    "namespace",
    "new",
    "null",
    "nullptr",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "required",
    "return",
    "sealed",
    "static",
    "struct",
    "super",
    "switch",
    "this",
    "throw",
    "throws",
    "true",
    "try",
    "typedef",
    "using",
    "val",
    "var",
    "virtual",
    "while",
    "with",
    "yield",
];

const C_TYPES: &[&str] = &[
    "bool", "boolean", "byte", "char", "double", "dynamic", "float", "int", "int32_t", "int64_t",
    "long", "short", "signed", "size_t", "String", "uint8_t", "uint16_t", "uint32_t", "uint64_t",
    "unsigned", "void",
];

const SHELL_KEYWORDS: &[&str] = &[
    "begin", "break", "case", "catch", "continue", "do", "done", "elif", "else", "end", "esac",
    "exit", "export", "false", "fi", "finally", "for", "function", "if", "in", "local", "param",
    "process", "readonly", "return", "then", "throw", "true", "try", "unset", "until", "while",
];

const CONFIG_KEYWORDS: &[&str] = &["false", "null", "off", "on", "true", "yes", "no"];

const SQL_KEYWORDS: &[&str] = &[
    "ADD", "ALL", "ALTER", "AND", "AS", "ASC", "AVG", "BETWEEN", "BY", "CASE", "COUNT", "CREATE",
    "DEFAULT", "DELETE", "DESC", "DISTINCT", "DROP", "ELSE", "END", "EXISTS", "FALSE", "FROM",
    "GROUP", "HAVING", "IN", "INDEX", "INNER", "INSERT", "INTO", "IS", "JOIN", "KEY", "LEFT",
    "LIKE", "LIMIT", "MAX", "MIN", "NOT", "NULL", "OFFSET", "ON", "OR", "ORDER", "OUTER",
    "PRIMARY", "RIGHT", "SELECT", "SET", "SUM", "TABLE", "THEN", "TRUE", "UNION", "UPDATE",
    "VALUES", "WHEN", "WHERE",
];

fn classify_ident_lang(family: LangFamily, word: &str, followed_by_paren: bool) -> Tok {
    let is_kw = match family {
        LangFamily::Rust => RUST_KEYWORDS.contains(&word),
        LangFamily::Python => PYTHON_KEYWORDS.contains(&word),
        LangFamily::JsTs => JSTS_KEYWORDS.contains(&word),
        LangFamily::Go => GO_KEYWORDS.contains(&word),
        LangFamily::CFamily => C_KEYWORDS.contains(&word),
        LangFamily::Shell => SHELL_KEYWORDS.contains(&word),
        LangFamily::Config => CONFIG_KEYWORDS.contains(&word),
        LangFamily::Sql => {
            let upper = word.to_ascii_uppercase();
            SQL_KEYWORDS.contains(&upper.as_str())
        }
        LangFamily::Plain => return Tok::Plain,
    };
    if is_kw {
        return Tok::Kw;
    }
    if followed_by_paren && family != LangFamily::Config {
        return Tok::Func;
    }
    let is_builtin_ty = match family {
        LangFamily::Rust => RUST_PRIMITIVES.contains(&word),
        LangFamily::Python => PYTHON_TYPES.contains(&word),
        LangFamily::JsTs => JSTS_TYPES.contains(&word),
        LangFamily::Go => GO_TYPES.contains(&word),
        LangFamily::CFamily => C_TYPES.contains(&word),
        _ => false,
    };
    if is_builtin_ty
        || (!matches!(
            family,
            LangFamily::Shell | LangFamily::Config | LangFamily::Sql
        ) && word.chars().next().is_some_and(|c| c.is_uppercase()))
    {
        Tok::Ty
    } else {
        Tok::Plain
    }
}

fn push_tok(out: &mut Vec<(Range<usize>, Tok)>, r: Range<usize>, tok: Tok) {
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
}

/// Scans a quoted string starting at `start` (UTF-8 char boundary), advancing by full chars so
/// multi-byte UTF-8 sequences are never split.
fn scan_quoted(line: &str, start: usize, quote: char) -> usize {
    let mut i = start + quote.len_utf8();
    while let Some(ch) = line.get(i..).and_then(|s| s.chars().next()) {
        i += ch.len_utf8();
        if ch == '\\' {
            if let Some(esc) = line.get(i..).and_then(|s| s.chars().next()) {
                i += esc.len_utf8();
            }
            continue;
        }
        if ch == quote {
            break;
        }
    }
    i.min(line.len())
}

fn try_config_prefix(line: &str, out: &mut Vec<(Range<usize>, Tok)>) -> usize {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut indent = 0usize;
    while indent < len && matches!(bytes[indent], b' ' | b'\t') {
        indent += 1;
    }
    if indent == len {
        return 0;
    }
    // Section headers like `[package]` or `[[bin]]`.
    if bytes[indent] == b'['
        && let Some(close) = line.get(indent..).and_then(|s| s.find(']'))
    {
        let end = indent
            + close
            + if bytes.get(indent + close + 1) == Some(&b']') {
                2
            } else {
                1
            };
        push_tok(out, 0..indent, Tok::Plain);
        push_tok(out, indent..end, Tok::Kw);
        return end;
    }
    // Quoted JSON key: `"key":`.
    if bytes[indent] == b'"' {
        let end_q = scan_quoted(line, indent, '"');
        let mut k = end_q;
        while k < len && matches!(bytes[k], b' ' | b'\t') {
            k += 1;
        }
        if bytes.get(k) == Some(&b':') {
            push_tok(out, 0..indent, Tok::Plain);
            push_tok(out, indent..end_q, Tok::Func);
            push_tok(out, end_q..k + 1, Tok::Plain);
            return k + 1;
        }
    }
    // Bare YAML / TOML / INI key: `key:` or `key =`.
    let mut key_start = indent;
    if bytes.get(indent) == Some(&b'-') && matches!(bytes.get(indent + 1), Some(b' ' | b'\t')) {
        key_start += 2;
        while key_start < len && matches!(bytes[key_start], b' ' | b'\t') {
            key_start += 1;
        }
    }
    let mut j = key_start;
    while j < len && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'_' | b'-' | b'.')) {
        j += 1;
    }
    if j > key_start {
        let mut k = j;
        while k < len && matches!(bytes[k], b' ' | b'\t') {
            k += 1;
        }
        let is_eq = bytes.get(k) == Some(&b'=');
        let is_colon = bytes.get(k) == Some(&b':')
            && (k + 1 == len || matches!(bytes.get(k + 1), Some(b' ' | b'\t')));
        if is_eq || is_colon {
            push_tok(out, 0..key_start, Tok::Plain);
            push_tok(out, key_start..j, Tok::Func);
            push_tok(out, j..k + 1, Tok::Plain);
            return k + 1;
        }
    }
    0
}

/// Splits one line of code into coloured byte ranges using the language hint `lang`.
/// Guaranteed to cover `0..line.len()` contiguously on valid UTF-8 boundaries.
pub fn tokenize_lang(line: &str, lang: &str) -> Vec<(Range<usize>, Tok)> {
    let family = classify_lang(lang);
    if line.is_empty() {
        return Vec::new();
    }
    if family == LangFamily::Plain {
        return vec![(0..line.len(), Tok::Plain)];
    }

    let bytes = line.as_bytes();
    let mut out: Vec<(Range<usize>, Tok)> = Vec::new();
    let mut i = if family == LangFamily::Config {
        try_config_prefix(line, &mut out)
    } else {
        0
    };

    while i < bytes.len() {
        let c = bytes[i];

        // Line comments
        let is_slash_comment = matches!(
            family,
            LangFamily::Rust
                | LangFamily::JsTs
                | LangFamily::Go
                | LangFamily::CFamily
                | LangFamily::Config
        ) && c == b'/'
            && bytes.get(i + 1) == Some(&b'/');
        let is_hash_comment = matches!(
            family,
            LangFamily::Python | LangFamily::Shell | LangFamily::Config
        ) && c == b'#'
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'$');
        let is_sql_comment =
            family == LangFamily::Sql && c == b'-' && bytes.get(i + 1) == Some(&b'-');
        if is_slash_comment || is_hash_comment || is_sql_comment {
            push_tok(&mut out, i..bytes.len(), Tok::Comment);
            break;
        }

        // Single-line block comments `/* ... */`
        if matches!(
            family,
            LangFamily::Rust
                | LangFamily::JsTs
                | LangFamily::Go
                | LangFamily::CFamily
                | LangFamily::Sql
        ) && c == b'/'
            && bytes.get(i + 1) == Some(&b'*')
        {
            let end = line
                .get(i + 2..)
                .and_then(|s| s.find("*/"))
                .map_or(bytes.len(), |rel| i + 2 + rel + 2);
            push_tok(&mut out, i..end, Tok::Comment);
            i = end;
            continue;
        }

        // Strings
        if c == b'"' {
            let end = scan_quoted(line, i, '"');
            push_tok(&mut out, i..end, Tok::Str);
            i = end;
            continue;
        }
        if c == b'\'' && family != LangFamily::Rust {
            let end = scan_quoted(line, i, '\'');
            push_tok(&mut out, i..end, Tok::Str);
            i = end;
            continue;
        }
        if c == b'`' && matches!(family, LangFamily::JsTs | LangFamily::Go) {
            let end = scan_quoted(line, i, '`');
            push_tok(&mut out, i..end, Tok::Str);
            i = end;
            continue;
        }

        // Python decorator `@name`
        if family == LangFamily::Python && c == b'@' {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'_' | b'.'))
            {
                j += 1;
            }
            if j > i + 1 {
                push_tok(&mut out, i..j, Tok::Func);
                i = j;
                continue;
            }
        }

        // Shell variable `$VAR` or `${VAR}`
        if family == LangFamily::Shell && c == b'$' {
            if bytes.get(i + 1) == Some(&b'{')
                && let Some(rel) = line.get(i + 2..).and_then(|s| s.find('}'))
            {
                let end = i + 2 + rel + 1;
                push_tok(&mut out, i..end, Tok::Ty);
                i = end;
                continue;
            }
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j > i + 1 {
                push_tok(&mut out, i..j, Tok::Ty);
                i = j;
                continue;
            }
        }

        // Number literals
        if c.is_ascii_digit()
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        {
            let mut j = i;
            while j < bytes.len() {
                let b = bytes[j];
                if b.is_ascii_alphanumeric()
                    || b == b'_'
                    || (b == b'.' && bytes.get(j + 1).is_some_and(u8::is_ascii_digit))
                {
                    j += 1;
                } else {
                    break;
                }
            }
            push_tok(&mut out, i..j, Tok::Ty);
            i = j;
            continue;
        }

        // Identifiers & keywords
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
            let tok = classify_ident_lang(family, word, followed);
            push_tok(&mut out, i..j, tok);
            i = j;
            continue;
        }

        // Any other byte (including multi-byte UTF-8 characters).
        let width = line[i..].chars().next().map_or(1, char::len_utf8);
        push_tok(&mut out, i..i + width, Tok::Plain);
        i += width;
    }
    out
}

/// Splits one line of Rust code into coloured byte ranges. Ranges cover the whole line.
pub fn tokenize(line: &str) -> Vec<(Range<usize>, Tok)> {
    tokenize_lang(line, "rust")
}

/// Builds a StyledText for one line of code using the theme's syntax colours and language hint.
pub fn code_line_lang(line: &str, lang: &str, t: &ActiveTheme) -> StyledText {
    let text = if line.is_empty() { " " } else { line };
    let mut runs = Vec::new();
    if line.is_empty() {
        runs.push(run(1, t.tx, Tok::Plain, t));
    } else {
        for (range, tok) in tokenize_lang(line, lang) {
            runs.push(run(range.len(), t.tx, tok, t));
        }
    }
    StyledText::new(SharedString::from(text.to_string())).with_runs(runs)
}

/// Builds a StyledText for one line of code using the theme's syntax colours.
pub fn code_line(line: &str, t: &ActiveTheme) -> StyledText {
    code_line_lang(line, "rust", t)
}

/// Text runs for one line of code with language hint `lang`. Empty for an empty line.
pub fn line_runs_lang(line: &str, lang: &str, t: &ActiveTheme) -> Vec<TextRun> {
    tokenize_lang(line, lang)
        .into_iter()
        .map(|(range, tok)| run(range.len(), t.tx, tok, t))
        .collect()
}

/// Text runs for one line of code (they cover the line exactly). Empty for an empty line.
pub fn line_runs(line: &str, t: &ActiveTheme) -> Vec<TextRun> {
    line_runs_lang(line, "rust", t)
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
    let mut f = font(fonts::face_name(Face::Code, t.kind));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ActiveTheme, ThemeKind};

    fn assert_byte_invariant(line: &str, lang: &str) {
        let toks = tokenize_lang(line, lang);
        let mut cursor = 0usize;
        for (r, _) in &toks {
            assert!(!r.is_empty(), "empty token in {lang:?} for {line:?}");
            assert_eq!(r.start, cursor, "gap or overlap in {lang:?} for {line:?}");
            assert!(line.is_char_boundary(r.start));
            assert!(line.is_char_boundary(r.end));
            cursor = r.end;
        }
        assert_eq!(cursor, line.len(), "total bytes mismatch in {lang:?}");

        let t = ActiveTheme::from_kind(ThemeKind::Ink);
        let runs = line_runs_lang(line, lang, &t);
        let total_run_len: usize = runs.iter().map(|r| r.len).sum();
        assert_eq!(total_run_len, line.len());
    }

    #[test]
    fn multi_language_tokens_and_utf8_invariants_hold() {
        let cases = [
            ("rust", "fn main() { let x = 0x2a; // 中文注释 🎉 }"),
            (
                "python",
                "@dataclass\ndef greet(name: str = '世界'): # 注释",
            ),
            ("ts", "const ok: boolean = true; /* 行内注释 */ `模板${x}`"),
            ("go", "func Run(cfg Config) error { return nil // 注释 }"),
            ("bash", "export PATH=\"$HOME/.cargo/bin:${EXTRA}\" # 路径"),
            ("toml", "version = \"0.3.8\" # 版本\n"),
            ("yaml", "  - status: true # 状态"),
            ("json", "  \"name\": \"霁月 🚀\", \"count\": 42"),
            (
                "sql",
                "select count(*) from notes where title = '测试' -- 统计",
            ),
            ("cpp", "int32_t sum = 10 + 20; // \"未闭合"),
            ("txt", "普通文本 中文与 Emoji 🚀 保持不变"),
        ];
        for (lang, sample) in cases {
            for line in sample.split('\n') {
                assert_byte_invariant(line, lang);
            }
        }
    }

    #[test]
    fn highlights_key_constructs_per_language() {
        let py = tokenize_lang("@route def index(): return 'ok' # 注释", "python");
        assert!(py.iter().any(|(r, t)| *t == Tok::Func
            && &"@route def index(): return 'ok' # 注释"[r.clone()] == "@route"));
        assert!(
            py.iter().any(|(r, t)| *t == Tok::Kw
                && &"@route def index(): return 'ok' # 注释"[r.clone()] == "def")
        );
        assert!(py.iter().any(|(r, t)| *t == Tok::Str
            && &"@route def index(): return 'ok' # 注释"[r.clone()] == "'ok'"));
        assert!(py.iter().any(|(_, t)| *t == Tok::Comment));

        let sql_line = "SELECT id FROM users WHERE active = true -- ok";
        let sql = tokenize_lang(sql_line, "sql");
        assert_eq!(sql[0].1, Tok::Kw);
        assert!(
            sql.iter()
                .any(|(r, t)| *t == Tok::Kw && &sql_line[r.clone()] == "FROM")
        );
        assert!(sql.iter().any(|(_, t)| *t == Tok::Comment));

        let json_line = "  \"port\": 8080";
        let json = tokenize_lang(json_line, "json");
        assert!(
            json.iter()
                .any(|(r, t)| *t == Tok::Func && &json_line[r.clone()] == "\"port\"")
        );
        assert!(
            json.iter()
                .any(|(r, t)| *t == Tok::Ty && &json_line[r.clone()] == "8080")
        );
    }
}
