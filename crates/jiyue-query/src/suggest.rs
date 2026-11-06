//! Autocomplete for the query language.
//!
//! Completes keywords and functions, built-in and frontmatter property names, and, inside a
//! string, the values of `folder("...")`, `tag("...")`, `link_to("...")` and `prop("...")`.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::ast::BUILTINS;
use crate::index::PropertyIndex;

/// What a completion refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    Keyword,
    Function,
    Builtin,
    Property,
    Tag,
    Folder,
    Note,
}

/// One suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Completion {
    pub label: String,
    /// Text to insert in place of the prefix typed so far.
    pub insert: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
}

const MAX_RESULTS: usize = 50;

const CLAUSES: [&str; 7] = [
    "from", "where", "select", "sort", "group by", "limit", "view",
];
const KEYWORDS: [&str; 13] = [
    "and", "or", "not", "in", "contains", "has", "as", "asc", "desc", "true", "false", "null",
    "all",
];
/// (name, signature shown as detail, text to insert)
const FUNCTIONS: [(&str, &str, &str); 17] = [
    ("today", "today()", "today()"),
    ("now", "now()", "now()"),
    ("date", "date(\"...\")", "date(\"\")"),
    ("len", "len(x)", "len()"),
    ("lower", "lower(x)", "lower()"),
    ("upper", "upper(x)", "upper()"),
    ("exists", "exists(prop)", "exists()"),
    ("default", "default(prop, x)", "default()"),
    ("prop", "prop(\"name\")", "prop(\"\")"),
    ("count", "count(x) or count()", "count()"),
    ("sum", "sum(x) (select only)", "sum()"),
    ("avg", "avg(x) (select only)", "avg()"),
    ("min", "min(x) (select only)", "min()"),
    ("max", "max(x) (select only)", "max()"),
    ("folder", "folder(\"name\") (from only)", "folder(\"\")"),
    ("tag", "tag(\"name\") (from only)", "tag(\"\")"),
    ("link_to", "link_to(\"note\") (from only)", "link_to(\"\")"),
];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Text needs backticks when it is not a plain name or it is a keyword.
fn needs_backticks(name: &str) -> bool {
    let mut chars = name.chars();
    let plain_start = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_');
    !(plain_start && name.chars().all(is_word_char))
        || CLAUSES.contains(&name)
        || KEYWORDS.contains(&name.to_ascii_lowercase().as_str())
}

fn property_insert(name: &str) -> String {
    if needs_backticks(name) {
        format!("`{name}`")
    } else {
        name.to_string()
    }
}

enum Open {
    Closed,
    /// Inside a quoted string that started at this byte index with this quote char.
    Str(usize, char),
    /// Inside backticks that started at this byte index.
    Tick(usize),
}

fn scan_open(before: &str) -> Open {
    let mut state = Open::Closed;
    let mut escaped = false;
    for (i, c) in before.char_indices() {
        state = match state {
            Open::Closed => match c {
                '"' | '\'' => Open::Str(i, c),
                '`' => Open::Tick(i),
                _ => Open::Closed,
            },
            Open::Str(start, q) => {
                if escaped {
                    escaped = false;
                    Open::Str(start, q)
                } else if c == '\\' {
                    escaped = true;
                    Open::Str(start, q)
                } else if c == q {
                    Open::Closed
                } else {
                    Open::Str(start, q)
                }
            }
            Open::Tick(start) => {
                if c == '`' {
                    Open::Closed
                } else {
                    Open::Tick(start)
                }
            }
        };
    }
    state
}

/// The identifier characters immediately before the end of `before`.
fn trailing_word(before: &str) -> &str {
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(before.len());
    before.get(start..).unwrap_or("")
}

fn starts_with_ci(label: &str, prefix: &str) -> bool {
    prefix.is_empty() || label.to_lowercase().starts_with(&prefix.to_lowercase())
}

/// Suggests completions for `query` with the cursor at byte offset `cursor`. A cursor that is
/// past the end or inside a character is moved to the nearest valid position.
pub fn suggest(index: &PropertyIndex, query: &str, cursor: usize) -> Vec<Completion> {
    let mut at = cursor.min(query.len());
    while at > 0 && !query.is_char_boundary(at) {
        at -= 1;
    }
    let before = query.get(..at).unwrap_or("");
    let mut out = match scan_open(before) {
        Open::Str(start, _) => string_completions(index, before, start),
        Open::Tick(start) => {
            let prefix = before.get(start + 1..).unwrap_or("");
            property_completions(index, prefix, true)
        }
        Open::Closed => {
            let prefix = trailing_word(before);
            expression_completions(index, prefix)
        }
    };
    out.truncate(MAX_RESULTS);
    out
}

fn string_completions(index: &PropertyIndex, before: &str, quote_at: usize) -> Vec<Completion> {
    let prefix = before.get(quote_at + 1..).unwrap_or("");
    let head = before.get(..quote_at).unwrap_or("").trim_end();
    let func = head
        .strip_suffix('(')
        .map(|h| trailing_word(h.trim_end()).to_ascii_lowercase())
        .unwrap_or_default();
    match func.as_str() {
        "folder" => index
            .folders()
            .filter(|f| starts_with_ci(f, prefix))
            .map(|f| Completion {
                label: f.to_string(),
                insert: f.to_string(),
                kind: CompletionKind::Folder,
                detail: Some("folder".to_string()),
            })
            .collect(),
        "tag" => index
            .tag_counts()
            .iter()
            .filter(|(t, _)| starts_with_ci(t, prefix))
            .map(|(t, n)| Completion {
                label: t.clone(),
                insert: t.clone(),
                kind: CompletionKind::Tag,
                detail: Some(format!("{n} notes")),
            })
            .collect(),
        "link_to" => {
            let names: BTreeSet<String> = index.records().map(|r| r.name.clone()).collect();
            names
                .into_iter()
                .filter(|n| starts_with_ci(n, prefix))
                .map(|n| Completion {
                    label: n.clone(),
                    insert: n,
                    kind: CompletionKind::Note,
                    detail: None,
                })
                .collect()
        }
        "prop" => property_completions(index, prefix, true),
        _ => Vec::new(),
    }
}

fn expression_completions(index: &PropertyIndex, prefix: &str) -> Vec<Completion> {
    let mut out = Vec::new();
    for clause in CLAUSES {
        if starts_with_ci(clause, prefix) {
            out.push(Completion {
                label: clause.to_string(),
                insert: clause.to_string(),
                kind: CompletionKind::Keyword,
                detail: Some("clause".to_string()),
            });
        }
    }
    for kw in KEYWORDS {
        if starts_with_ci(kw, prefix) {
            out.push(Completion {
                label: kw.to_string(),
                insert: kw.to_string(),
                kind: CompletionKind::Keyword,
                detail: None,
            });
        }
    }
    for (name, detail, insert) in FUNCTIONS {
        if starts_with_ci(name, prefix) {
            out.push(Completion {
                label: name.to_string(),
                insert: insert.to_string(),
                kind: CompletionKind::Function,
                detail: Some(detail.to_string()),
            });
        }
    }
    for b in BUILTINS {
        if starts_with_ci(b, prefix) {
            out.push(Completion {
                label: b.to_string(),
                insert: b.to_string(),
                kind: CompletionKind::Builtin,
                detail: Some("built-in".to_string()),
            });
        }
    }
    out.extend(property_completions(index, prefix, false));
    out
}

/// Property names. `raw` inserts the bare name, for use inside backticks or a string.
fn property_completions(index: &PropertyIndex, prefix: &str, raw: bool) -> Vec<Completion> {
    index
        .property_counts()
        .iter()
        .filter(|(name, _)| starts_with_ci(name, prefix))
        .map(|(name, n)| Completion {
            label: name.clone(),
            insert: if raw {
                name.clone()
            } else {
                property_insert(name)
            },
            kind: CompletionKind::Property,
            detail: Some(format!("in {n} notes")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backtick_quoting_rules() {
        assert_eq!(property_insert("status"), "status");
        assert_eq!(property_insert("due date"), "`due date`");
        assert_eq!(property_insert("from"), "`from`");
        assert_eq!(property_insert("状态"), "状态");
        assert_eq!(property_insert("1st"), "`1st`");
    }

    #[test]
    fn trailing_word_handles_cjk() {
        assert_eq!(trailing_word("where 状态"), "状态");
        assert_eq!(trailing_word("where "), "");
    }

    #[test]
    fn scan_detects_open_strings() {
        assert!(matches!(scan_open("folder(\"a"), Open::Str(7, '"')));
        assert!(matches!(scan_open("folder(\"a\")"), Open::Closed));
        assert!(matches!(scan_open("x = `due"), Open::Tick(4)));
        assert!(matches!(scan_open("x = \"a\\\""), Open::Str(4, '"')));
    }
}
