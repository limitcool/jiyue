//! A compact tokenizer for code blocks in a fixed set of languages.
//!
//! Recognises comments, strings, numbers, keywords and function calls. It is not a parser:
//! it is good enough for reading code in a note and avoids embedding a full grammar set.
//! Output is escaped HTML with `tok-*` classes.

use super::esc;

struct Spec {
    keywords: &'static [&'static str],
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static [u8],
    case_insensitive: bool,
}

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

const JS_KW: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
    "interface",
    "type",
    "enum",
    "implements",
    "readonly",
    "private",
    "public",
    "protected",
    "as",
    "from",
];

const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];

const GO_KW: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    "nil",
    "true",
    "false",
];

const C_KW: &[&str] = &[
    "auto",
    "bool",
    "break",
    "case",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "false",
    "float",
    "for",
    "goto",
    "if",
    "int",
    "long",
    "namespace",
    "new",
    "nullptr",
    "private",
    "protected",
    "public",
    "register",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "this",
    "true",
    "typedef",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

const JAVA_KW: &[&str] = &[
    "abstract",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "throws",
    "true",
    "try",
    "void",
    "volatile",
    "while",
];

const SQL_KW: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create",
    "table", "drop", "alter", "join", "left", "right", "inner", "outer", "on", "group", "by",
    "order", "having", "limit", "and", "or", "not", "null", "as", "primary", "key", "distinct",
    "union", "index", "view", "case", "when", "then", "else", "end",
];

const BASH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "while", "do", "done", "in",
    "function", "return", "local", "export", "exit",
];

const JSON_KW: &[&str] = &["true", "false", "null"];
const YAML_KW: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];
const TOML_KW: &[&str] = &["true", "false"];
const NO_KW: &[&str] = &[];

const SLASH: &[&str] = &["//"];
const HASH: &[&str] = &["#"];
const DASHES: &[&str] = &["--"];
const NO_LINE: &[&str] = &[];

fn spec(lang: &str) -> Option<Spec> {
    let s = match lang.to_ascii_lowercase().as_str() {
        "rust" | "rs" => Spec {
            keywords: RUST_KW,
            line_comments: SLASH,
            block_comment: Some(("/*", "*/")),
            quotes: b"\"",
            case_insensitive: false,
        },
        "js" | "javascript" | "jsx" | "mjs" | "cjs" | "ts" | "typescript" | "tsx" => Spec {
            keywords: JS_KW,
            line_comments: SLASH,
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'`",
            case_insensitive: false,
        },
        "python" | "py" => Spec {
            keywords: PY_KW,
            line_comments: HASH,
            block_comment: None,
            quotes: b"\"'",
            case_insensitive: false,
        },
        "json" => Spec {
            keywords: JSON_KW,
            line_comments: NO_LINE,
            block_comment: None,
            quotes: b"\"",
            case_insensitive: false,
        },
        "yaml" | "yml" => Spec {
            keywords: YAML_KW,
            line_comments: HASH,
            block_comment: None,
            quotes: b"\"'",
            case_insensitive: false,
        },
        "toml" => Spec {
            keywords: TOML_KW,
            line_comments: HASH,
            block_comment: None,
            quotes: b"\"'",
            case_insensitive: false,
        },
        "bash" | "sh" | "shell" | "zsh" => Spec {
            keywords: BASH_KW,
            line_comments: HASH,
            block_comment: None,
            quotes: b"\"'`",
            case_insensitive: false,
        },
        "sql" => Spec {
            keywords: SQL_KW,
            line_comments: DASHES,
            block_comment: Some(("/*", "*/")),
            quotes: b"'\"",
            case_insensitive: true,
        },
        "html" => Spec {
            keywords: NO_KW,
            line_comments: NO_LINE,
            block_comment: Some(("<!--", "-->")),
            quotes: b"\"'",
            case_insensitive: false,
        },
        "css" => Spec {
            keywords: NO_KW,
            line_comments: NO_LINE,
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'",
            case_insensitive: false,
        },
        "go" | "golang" => Spec {
            keywords: GO_KW,
            line_comments: SLASH,
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'`",
            case_insensitive: false,
        },
        "c" | "cpp" | "c++" | "cc" | "cxx" => Spec {
            keywords: C_KW,
            line_comments: SLASH,
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'",
            case_insensitive: false,
        },
        "java" => Spec {
            keywords: JAVA_KW,
            line_comments: SLASH,
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'",
            case_insensitive: false,
        },
        _ => return None,
    };
    Some(s)
}

/// Highlighted HTML for `code`, or `None` when the language is not supported. Markdown is
/// deliberately left unhighlighted: it returns the escaped code.
pub(super) fn highlight(lang: &str, code: &str) -> Option<String> {
    if lang.eq_ignore_ascii_case("markdown") || lang.eq_ignore_ascii_case("md") {
        return Some(esc(code));
    }
    let spec = spec(lang)?;
    let b = code.as_bytes();
    let n = b.len();
    let mut out = String::with_capacity(code.len() + code.len() / 2);
    let mut plain = 0usize;
    let mut i = 0usize;
    while i < n {
        if let Some(end) = comment_end(code, i, &spec) {
            flush(&mut out, code, plain, i);
            emit(&mut out, "com", &code[i..end]);
            i = end;
            plain = end;
            continue;
        }
        let c = b[i];
        if spec.quotes.contains(&c) {
            let end = string_end(b, i, c);
            flush(&mut out, code, plain, i);
            emit(&mut out, "str", &code[i..end]);
            i = end;
            plain = end;
            continue;
        }
        if c.is_ascii_digit() && !(i > 0 && is_ident(b[i - 1])) {
            let mut end = i;
            while end < n && (is_ident(b[end]) || b[end] == b'.') {
                end += 1;
            }
            flush(&mut out, code, plain, i);
            emit(&mut out, "num", &code[i..end]);
            i = end;
            plain = end;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut end = i;
            while end < n && is_ident(b[end]) {
                end += 1;
            }
            let word = &code[i..end];
            let is_kw = spec.keywords.iter().any(|k| {
                if spec.case_insensitive {
                    k.eq_ignore_ascii_case(word)
                } else {
                    *k == word
                }
            });
            let next_is_call = code[end..].trim_start_matches([' ', '\t']).starts_with('(');
            if is_kw || next_is_call {
                flush(&mut out, code, plain, i);
                emit(&mut out, if is_kw { "kw" } else { "fn" }, word);
                plain = end;
            }
            i = end;
            continue;
        }
        i += 1;
    }
    flush(&mut out, code, plain, n);
    Some(out)
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn flush(out: &mut String, code: &str, from: usize, to: usize) {
    if from < to {
        out.push_str(&esc(&code[from..to]));
    }
}

fn emit(out: &mut String, class: &str, text: &str) {
    out.push_str("<span class=\"tok-");
    out.push_str(class);
    out.push_str("\">");
    out.push_str(&esc(text));
    out.push_str("</span>");
}

/// End of a comment starting at `i`, if one starts there.
fn comment_end(code: &str, i: usize, spec: &Spec) -> Option<usize> {
    let b = code.as_bytes();
    for marker in spec.line_comments {
        if b[i..].starts_with(marker.as_bytes()) {
            let end = code[i..].find('\n').map_or(code.len(), |p| i + p);
            return Some(end);
        }
    }
    let (open, close) = spec.block_comment?;
    if b[i..].starts_with(open.as_bytes()) {
        let from = i + open.len();
        let end = code[from..]
            .find(close)
            .map_or(code.len(), |p| from + p + close.len());
        return Some(end);
    }
    None
}

/// End of a string literal starting at `i` (exclusive). Unterminated single-line strings end
/// before the newline. Backtick strings may span lines.
fn string_end(b: &[u8], i: usize, quote: u8) -> usize {
    let n = b.len();
    let mut j = i + 1;
    while j < n {
        match b[j] {
            b'\\' => j += 2,
            c if c == quote => return j + 1,
            b'\n' if quote != b'`' => return j,
            _ => j += 1,
        }
    }
    n.min(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(html: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = html;
        while let Some(p) = rest.find("<span class=\"tok-") {
            let after = &rest[p + 17..];
            let end = after.find('"').unwrap_or(0);
            out.push(after[..end].to_string());
            rest = &after[end..];
        }
        out
    }

    #[test]
    fn rust_keywords_strings_comments_numbers() {
        let html =
            highlight("rust", "fn main() { let x = 42; // hi\n let s = \"a\"; }").expect("rust");
        let c = classes(&html);
        assert!(c.contains(&"kw".to_string()));
        assert!(c.contains(&"num".to_string()));
        assert!(c.contains(&"com".to_string()));
        assert!(c.contains(&"str".to_string()));
        assert!(c.contains(&"fn".to_string()));
    }

    #[test]
    fn identifiers_are_not_keywords_inside_words() {
        let html = highlight("rust", "letter format").expect("rust");
        assert!(!html.contains("tok-kw"));
    }

    #[test]
    fn unsupported_language_is_none() {
        assert!(highlight("cobol", "DISPLAY").is_none());
        assert!(highlight("", "x").is_none());
    }

    #[test]
    fn markdown_is_escaped_only() {
        assert_eq!(highlight("markdown", "<b>").as_deref(), Some("&lt;b&gt;"));
    }

    #[test]
    fn output_is_always_escaped() {
        let html = highlight("js", "let a = '<script>'; // </b>").expect("js");
        assert!(!html.contains("<script>"));
        assert!(!html.contains("</b>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn sql_is_case_insensitive_and_uses_dash_comments() {
        let html = highlight("sql", "SELECT * FROM t -- note\n").expect("sql");
        assert!(html.contains("<span class=\"tok-kw\">SELECT</span>"));
        assert!(html.contains("tok-com"));
    }

    #[test]
    fn python_hash_comment_and_strings() {
        let html = highlight("python", "def f(x):  # c\n    return 'y'\n").expect("py");
        assert!(html.contains("tok-com"));
        assert!(html.contains("tok-str"));
        assert!(html.contains("<span class=\"tok-kw\">def</span>"));
    }

    #[test]
    fn json_and_yaml_keywords() {
        assert!(
            highlight("json", "{\"a\": true, \"b\": null}")
                .expect("json")
                .contains("tok-kw")
        );
        assert!(
            highlight("yaml", "key: yes # c")
                .expect("yaml")
                .contains("tok-com")
        );
    }

    #[test]
    fn block_comments_html_and_css() {
        let html = highlight("html", "<!-- x <b> -->").expect("html");
        assert!(html.contains("tok-com"));
        assert!(html.contains("&lt;b&gt;"));
        let css = highlight("css", "/* c */ a { color: red; }").expect("css");
        assert!(css.contains("tok-com"));
    }

    #[test]
    fn unterminated_string_stops_at_newline() {
        let html = highlight("c", "char *s = \"abc\nint x = 1;").expect("c");
        assert!(html.contains("int"));
        assert!(html.contains("tok-num"));
    }

    #[test]
    fn unterminated_block_comment_runs_to_end() {
        let html = highlight("go", "/* never closed\nvar x").expect("go");
        assert!(!html.contains("tok-kw\">var"));
    }

    #[test]
    fn trailing_backslash_does_not_panic() {
        assert!(highlight("js", "\"abc\\").is_some());
        assert!(highlight("js", "'").is_some());
        assert!(highlight("js", "`x").is_some());
    }

    #[test]
    fn cjk_source_text_survives() {
        let html = highlight("js", "const 名字 = \"世界\"; // 注释").expect("js");
        assert!(html.contains("名字"));
        assert!(html.contains("世界"));
        assert!(html.contains("注释"));
    }

    #[test]
    fn function_call_detection_with_space() {
        let html = highlight("java", "System.out.println (x);").expect("java");
        assert!(html.contains("<span class=\"tok-fn\">println</span>"));
    }

    #[test]
    fn number_with_suffix_and_dot() {
        let html = highlight("rust", "let a = 1_000u32 + 3.14;").expect("rust");
        assert!(html.contains("tok-num\">1_000u32</span>"));
        assert!(html.contains("tok-num\">3.14</span>"));
    }
}
