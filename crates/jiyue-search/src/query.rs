//! Query language: bare terms (AND), `"phrases"`, `-exclusion`, `OR`, and the scoped operators
//! `tag:`, `path:`, `file:`, `title:`. Anything unrecognised degrades to plain terms.
//! Parsing never fails.

use std::collections::HashSet;

use tantivy::query::{BooleanQuery, BoostQuery, Occur, PhraseQuery, Query, RegexQuery, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Term};

use crate::Fields;
use crate::analysis::{AnalyzedToken, index_tokens, query_tokens};

/// Boost applied to matches in the title.
pub(crate) const TITLE_BOOST: f32 = 3.0;
/// Boost applied to matches in the file name.
pub(crate) const FILE_BOOST: f32 = 2.0;

/// Which field a term applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Title, file name and body together.
    Text,
    Title,
    File,
    Tag,
    /// Substring of the note path.
    Path,
}

/// One term or phrase. `text` is the raw value, analysed later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    pub scope: Scope,
    pub text: String,
}

/// A parsed query. `groups` are ANDed; the items inside a group are ORed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub groups: Vec<Vec<Item>>,
    pub excluded: Vec<Item>,
}

#[derive(Debug)]
enum Clause {
    Term { negated: bool, item: Item },
    Or,
}

/// Parses a query string. Never fails.
pub(crate) fn parse(input: &str) -> Parsed {
    let mut groups: Vec<Vec<Item>> = Vec::new();
    let mut excluded = Vec::new();
    let mut join_next = false;
    for clause in lex(input) {
        match clause {
            Clause::Or => join_next = !groups.is_empty(),
            Clause::Term {
                negated: true,
                item,
            } => {
                excluded.push(item);
                join_next = false;
            }
            Clause::Term {
                negated: false,
                item,
            } => {
                match groups.last_mut() {
                    Some(last) if join_next => last.push(item),
                    _ => groups.push(vec![item]),
                }
                join_next = false;
            }
        }
    }
    Parsed { groups, excluded }
}

fn lex(input: &str) -> Vec<Clause> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    loop {
        while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut negated = false;
        if chars.peek() == Some(&'-') {
            negated = true;
            chars.next();
        }
        let starts_quoted = chars.peek() == Some(&'"');

        // Read one word. Quotes group whitespace and are removed from the word.
        let mut word = String::new();
        let mut in_quote = false;
        let mut saw_quote = false;
        while let Some(&c) = chars.peek() {
            if c == '"' {
                in_quote = !in_quote;
                saw_quote = true;
                chars.next();
                continue;
            }
            if c.is_whitespace() && !in_quote {
                break;
            }
            word.push(c);
            chars.next();
        }

        if word.trim().is_empty() {
            continue;
        }
        if !negated && !saw_quote && word == "OR" {
            out.push(Clause::Or);
            continue;
        }
        out.push(Clause::Term {
            negated,
            item: scoped_item(word, starts_quoted),
        });
    }
    out
}

fn scoped_item(word: String, starts_quoted: bool) -> Item {
    if !starts_quoted && let Some((field, value)) = word.split_once(':') {
        let scope = match field.to_ascii_lowercase().as_str() {
            "tag" => Some(Scope::Tag),
            "path" => Some(Scope::Path),
            "file" => Some(Scope::File),
            "title" => Some(Scope::Title),
            _ => None,
        };
        let value = value.trim();
        if let (Some(scope), false) = (scope, value.is_empty()) {
            return Item {
                scope,
                text: value.to_string(),
            };
        }
    }
    Item {
        scope: Scope::Text,
        text: word,
    }
}

/// Normalises a tag: trims, drops leading `#`s, lowercases.
pub(crate) fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('#').to_lowercase()
}

/// Terms that should be highlighted in snippets: analysed text of the positive text-like items.
pub(crate) fn highlight_terms(parsed: &Parsed) -> HashSet<String> {
    parsed
        .groups
        .iter()
        .flatten()
        .filter(|item| matches!(item.scope, Scope::Text | Scope::Title | Scope::File))
        .flat_map(|item| query_tokens(&item.text).into_iter().map(|t| t.text))
        .collect()
}

/// A negated multi-token phrase. It is checked on candidate documents instead of in the
/// tantivy query, because tantivy's exclusion of phrase scorers trips a debug assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Exclusion {
    pub scope: Scope,
    pub tokens: Vec<AnalyzedToken>,
}

/// A built query: the tantivy part plus the phrase exclusions to check on candidates.
pub(crate) struct Built {
    pub query: Box<dyn Query>,
    pub exclusions: Vec<Exclusion>,
}

/// Builds the query. Returns `None` when there is no positive clause, so that an empty or
/// purely negative query matches nothing.
pub(crate) fn build(parsed: &Parsed, fields: Fields) -> Option<Built> {
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    for group in &parsed.groups {
        let mut alts: Vec<(Occur, Box<dyn Query>)> = group
            .iter()
            .filter_map(|item| item_query(item, fields))
            .map(|q| (Occur::Should, q))
            .collect();
        match alts.len() {
            0 => {}
            1 => {
                if let Some((_, q)) = alts.pop() {
                    clauses.push((Occur::Must, q));
                }
            }
            _ => {
                let q: Box<dyn Query> = Box::new(BooleanQuery::new(alts));
                clauses.push((Occur::Must, q));
            }
        }
    }
    if clauses.is_empty() {
        return None;
    }
    let mut exclusions = Vec::new();
    for item in &parsed.excluded {
        let tokens = query_tokens(&item.text);
        let phrase_like =
            matches!(item.scope, Scope::Text | Scope::Title | Scope::File) && tokens.len() >= 2;
        if phrase_like {
            exclusions.push(Exclusion {
                scope: item.scope,
                tokens,
            });
        } else if let Some(q) = item_query(item, fields) {
            clauses.push((Occur::MustNot, q));
        }
    }
    let query: Box<dyn Query> = Box::new(BooleanQuery::new(clauses));
    Some(Built { query, exclusions })
}

/// True when any exclusion phrase occurs in the title, file name or body (per its scope).
pub(crate) fn is_excluded(exclusions: &[Exclusion], title: &str, file: &str, body: &str) -> bool {
    exclusions.iter().any(|ex| {
        let texts: &[&str] = match ex.scope {
            Scope::Title => &[title],
            Scope::File => &[file],
            _ => &[title, file, body],
        };
        texts.iter().any(|text| contains_phrase(text, &ex.tokens))
    })
}

/// True when `phrase` (slots relative to its first token) occurs in `text` with the same
/// adjacency that the index gives it.
fn contains_phrase(text: &str, phrase: &[AnalyzedToken]) -> bool {
    let Some(first) = phrase.first() else {
        return false;
    };
    let wanted: HashSet<&str> = phrase.iter().map(|t| t.text.as_str()).collect();
    let doc: Vec<AnalyzedToken> = index_tokens(text)
        .filter(|t| wanted.contains(t.text.as_str()))
        .collect();
    let present: HashSet<(&str, usize)> = doc.iter().map(|t| (t.text.as_str(), t.slot)).collect();
    doc.iter().filter(|t| t.text == first.text).any(|t| {
        phrase.iter().all(|p| {
            let slot = t.slot + p.slot.saturating_sub(first.slot);
            present.contains(&(p.text.as_str(), slot))
        })
    })
}

fn item_query(item: &Item, fields: Fields) -> Option<Box<dyn Query>> {
    match item.scope {
        Scope::Text => {
            let mut subs: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            if let Some(q) = text_query(fields.title, &item.text) {
                let boosted: Box<dyn Query> = Box::new(BoostQuery::new(q, TITLE_BOOST));
                subs.push((Occur::Should, boosted));
            }
            if let Some(q) = text_query(fields.file, &item.text) {
                let boosted: Box<dyn Query> = Box::new(BoostQuery::new(q, FILE_BOOST));
                subs.push((Occur::Should, boosted));
            }
            if let Some(q) = text_query(fields.body, &item.text) {
                subs.push((Occur::Should, q));
            }
            if subs.is_empty() {
                return None;
            }
            let q: Box<dyn Query> = Box::new(BooleanQuery::new(subs));
            Some(q)
        }
        Scope::Title => text_query(fields.title, &item.text),
        Scope::File => text_query(fields.file, &item.text),
        Scope::Tag => {
            let value = normalize_tag(&item.text);
            if value.is_empty() {
                return None;
            }
            let term = Term::from_field_text(fields.tags, &value);
            let q: Box<dyn Query> = Box::new(TermQuery::new(term, IndexRecordOption::Basic));
            Some(q)
        }
        Scope::Path => {
            let value = item.text.trim();
            if value.is_empty() {
                return None;
            }
            let pattern = format!(".*{}.*", escape_regex(value));
            let q: Box<dyn Query> = Box::new(RegexQuery::from_pattern(&pattern, fields.path).ok()?);
            Some(q)
        }
    }
}

/// Analysed text query on one field: a term for one token, a phrase for several.
fn text_query(field: Field, text: &str) -> Option<Box<dyn Query>> {
    let tokens = query_tokens(text);
    match tokens.len() {
        0 => None,
        1 => {
            let term = Term::from_field_text(field, &tokens[0].text);
            let q: Box<dyn Query> = Box::new(TermQuery::new(
                term,
                IndexRecordOption::WithFreqsAndPositions,
            ));
            Some(q)
        }
        _ => {
            let terms: Vec<(usize, Term)> = tokens
                .iter()
                .map(|t| (t.slot, Term::from_field_text(field, &t.text)))
                .collect();
            let q: Box<dyn Query> = Box::new(PhraseQuery::new_with_offset(terms));
            Some(q)
        }
    }
}

/// Escapes regex metacharacters so `value` matches literally.
fn escape_regex(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    for c in value.chars() {
        if matches!(
            c,
            '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(scope: Scope, text: &str) -> Item {
        Item {
            scope,
            text: text.to_string(),
        }
    }

    fn text(s: &str) -> Item {
        item(Scope::Text, s)
    }

    #[test]
    fn empty_and_blank_queries_have_no_groups() {
        assert_eq!(parse(""), Parsed::default());
        assert_eq!(parse("   \t\n "), Parsed::default());
    }

    #[test]
    fn bare_terms_are_and_groups() {
        let p = parse("rust async");
        assert_eq!(p.groups, vec![vec![text("rust")], vec![text("async")]]);
        assert!(p.excluded.is_empty());
    }

    #[test]
    fn quoted_phrase_is_one_item() {
        let p = parse(r#""async await" rust"#);
        assert_eq!(
            p.groups,
            vec![vec![text("async await")], vec![text("rust")]]
        );
    }

    #[test]
    fn phrase_can_be_negated() {
        let p = parse(r#"rust -"old api""#);
        assert_eq!(p.groups, vec![vec![text("rust")]]);
        assert_eq!(p.excluded, vec![text("old api")]);
    }

    #[test]
    fn minus_term_is_exclusion() {
        let p = parse("rust -go");
        assert_eq!(p.groups, vec![vec![text("rust")]]);
        assert_eq!(p.excluded, vec![text("go")]);
    }

    #[test]
    fn or_joins_neighbours_into_one_group() {
        let p = parse("alpha OR beta gamma");
        assert_eq!(
            p.groups,
            vec![vec![text("alpha"), text("beta")], vec![text("gamma")]]
        );
    }

    #[test]
    fn chained_or_joins_all() {
        let p = parse("a OR b OR c");
        assert_eq!(p.groups, vec![vec![text("a"), text("b"), text("c")]]);
    }

    #[test]
    fn leading_or_and_trailing_or_are_ignored() {
        assert_eq!(parse("OR a").groups, vec![vec![text("a")]]);
        assert_eq!(parse("a OR").groups, vec![vec![text("a")]]);
        assert_eq!(parse("OR").groups, Vec::<Vec<Item>>::new());
    }

    #[test]
    fn negated_or_is_literal_text() {
        let p = parse("a -OR");
        assert_eq!(p.groups, vec![vec![text("a")]]);
        assert_eq!(p.excluded, vec![text("OR")]);
    }

    #[test]
    fn tag_with_and_without_hash() {
        assert_eq!(
            parse("tag:rust").groups,
            vec![vec![item(Scope::Tag, "rust")]]
        );
        assert_eq!(
            parse("tag:#rust").groups,
            vec![vec![item(Scope::Tag, "#rust")]]
        );
        assert_eq!(normalize_tag("#Rust"), "rust");
        assert_eq!(normalize_tag("  ##Rust "), "rust");
    }

    #[test]
    fn negated_tag() {
        let p = parse("-tag:draft");
        assert!(p.groups.is_empty());
        assert_eq!(p.excluded, vec![item(Scope::Tag, "draft")]);
    }

    #[test]
    fn path_scope_with_cjk() {
        assert_eq!(
            parse("path:想法/").groups,
            vec![vec![item(Scope::Path, "想法/")]]
        );
    }

    #[test]
    fn path_scope_with_quoted_value_keeps_spaces() {
        assert_eq!(
            parse(r#"path:"my dir/""#).groups,
            vec![vec![item(Scope::Path, "my dir/")]]
        );
    }

    #[test]
    fn file_scope() {
        assert_eq!(
            parse("file:读书").groups,
            vec![vec![item(Scope::File, "读书")]]
        );
    }

    #[test]
    fn title_scope_and_case_insensitive_operator_names() {
        assert_eq!(
            parse("TITLE:xxx").groups,
            vec![vec![item(Scope::Title, "xxx")]]
        );
        assert_eq!(parse("Tag:x").groups, vec![vec![item(Scope::Tag, "x")]]);
    }

    #[test]
    fn unknown_operator_degrades_to_plain_text() {
        assert_eq!(parse("foo:bar").groups, vec![vec![text("foo:bar")]]);
        assert_eq!(parse("http://x.y").groups, vec![vec![text("http://x.y")]]);
    }

    #[test]
    fn empty_operator_value_degrades_to_plain_text() {
        assert_eq!(parse("tag:").groups, vec![vec![text("tag:")]]);
        assert_eq!(parse("path:  ").groups, vec![vec![text("path:")]]);
    }

    #[test]
    fn quoted_word_with_colon_is_not_an_operator() {
        assert_eq!(parse(r#""tag:rust""#).groups, vec![vec![text("tag:rust")]]);
    }

    #[test]
    fn unclosed_quote_takes_the_rest() {
        assert_eq!(
            parse(r#""unclosed phrase here"#).groups,
            vec![vec![text("unclosed phrase here")]]
        );
    }

    #[test]
    fn lone_dash_and_empty_quotes_are_dropped() {
        assert_eq!(parse("-").groups, Vec::<Vec<Item>>::new());
        assert_eq!(parse("- -").groups, Vec::<Vec<Item>>::new());
        assert_eq!(parse(r#""""#).groups, Vec::<Vec<Item>>::new());
        assert_eq!(
            parse("a - b").groups,
            vec![vec![text("a")], vec![text("b")]]
        );
    }

    #[test]
    fn highlight_terms_skip_negated_and_scoped_tags() {
        let p = parse("rust 异步 -go tag:python path:x");
        let terms = highlight_terms(&p);
        assert!(terms.contains("rust"));
        assert!(terms.contains("异步"));
        assert!(!terms.contains("go"));
        assert!(!terms.contains("python"));
        assert!(!terms.contains("x"));
    }

    #[test]
    fn highlight_terms_for_single_cjk_char() {
        let terms = highlight_terms(&parse("步"));
        assert_eq!(terms, HashSet::from(["步".to_string()]));
    }

    #[test]
    fn escape_regex_escapes_metacharacters() {
        assert_eq!(escape_regex("a.b*c"), r"a\.b\*c");
        assert_eq!(escape_regex("想法/x"), "想法/x");
        assert_eq!(escape_regex(r"(a|b)\"), r"\(a\|b\)\\");
    }
}
