//! Helpers for editing properties from a table: write a cell back into the note, parse what
//! the user typed, and infer the type of each property.
//!
//! Writes go through `jiyue_md::frontmatter::set_property`, which replaces only the line of
//! the edited key. Comments, other keys, quoting and line endings stay byte for byte.

use std::collections::BTreeMap;

use jiyue_md::frontmatter::{self, Value};

use crate::cell::{Cell, Kind};
use crate::date::{self, format_date, format_iso};
use crate::index::PropertyIndex;

/// Converts a cell to the frontmatter value written for it.
fn to_value(c: &Cell) -> Value {
    match c {
        Cell::Null => Value::Null,
        Cell::Bool(b) => Value::Bool(*b),
        Cell::Number(n) => Value::Number(*n),
        Cell::Text(s) => Value::Text(s.clone()),
        Cell::Date(d) => Value::Date(format_date(*d)),
        Cell::DateTime(ms) => Value::DateTime(format_iso(*ms)),
        Cell::List(items) => Value::List(items.iter().map(Cell::display).collect()),
    }
}

/// Returns the note text with `property` set to `new_value`. Only that property's line is
/// rewritten. Text that has no valid frontmatter, or an edit that would not re-parse, is
/// returned unchanged. Clearing a cell writes `key: null`.
pub fn plan_cell_edit(note_text: &str, property: &str, new_value: &Cell) -> String {
    frontmatter::set_property(note_text, property, &to_value(new_value))
}

/// Turns typed text into a cell. With a hint, the text is read as that type and falls back to
/// text when it does not fit. Without a hint, numbers, booleans and dates are detected.
/// Empty input is `Null`.
pub fn parse_input(raw: &str, hint: Option<Kind>) -> Cell {
    let t = raw.trim();
    if t.is_empty() {
        return Cell::Null;
    }
    let text = || Cell::Text(t.to_string());
    let number = || {
        t.parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(Cell::Number)
    };
    let boolean = || match t.to_ascii_lowercase().as_str() {
        "true" | "yes" => Some(Cell::Bool(true)),
        "false" | "no" => Some(Cell::Bool(false)),
        _ => None,
    };
    let date_cell = || date::parse_date(t).map(Cell::Date);
    let datetime_cell = || match date::parse_datetime_parts(t) {
        Some((ms, true)) => Some(Cell::DateTime(ms)),
        _ => None,
    };
    match hint {
        Some(Kind::Text) => text(),
        Some(Kind::Number) => number().unwrap_or_else(text),
        Some(Kind::Bool) => boolean().unwrap_or_else(text),
        Some(Kind::Date) => date_cell().unwrap_or_else(text),
        Some(Kind::DateTime) => datetime_cell().unwrap_or_else(text),
        Some(Kind::List) => {
            let items: Vec<Cell> = t
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Cell::Text(s.to_string()))
                .collect();
            Cell::List(items)
        }
        Some(Kind::Null) => Cell::Null,
        None => boolean()
            .or_else(number)
            .or_else(datetime_cell)
            .or_else(date_cell)
            .unwrap_or_else(text),
    }
}

/// The dominant type of each property across the vault. Ties go to the earlier kind in
/// [`Kind`] order. Properties whose values are all empty are left out.
pub fn property_types(index: &PropertyIndex) -> BTreeMap<String, Kind> {
    let mut counts: BTreeMap<String, BTreeMap<Kind, usize>> = BTreeMap::new();
    for rec in index.records() {
        for (key, value) in &rec.props {
            if value.is_null() {
                continue;
            }
            *counts
                .entry(key.clone())
                .or_default()
                .entry(value.kind())
                .or_insert(0) += 1;
        }
    }
    let mut out = BTreeMap::new();
    for (key, by_kind) in counts {
        let mut best: Option<(usize, Kind)> = None;
        for (kind, n) in by_kind {
            if best.is_none_or(|(bn, _)| n > bn) {
                best = Some((n, kind));
            }
        }
        if let Some((_, kind)) = best {
            out.insert(key, kind);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_with_hints() {
        assert_eq!(parse_input("  ", None), Cell::Null);
        assert_eq!(parse_input("3.5", Some(Kind::Number)), Cell::Number(3.5));
        assert_eq!(
            parse_input("abc", Some(Kind::Number)),
            Cell::Text("abc".into())
        );
        assert_eq!(parse_input("yes", Some(Kind::Bool)), Cell::Bool(true));
        assert!(matches!(
            parse_input("2026-03-01", Some(Kind::Date)),
            Cell::Date(_)
        ));
        assert!(matches!(
            parse_input("2026-03-01 09:00", Some(Kind::DateTime)),
            Cell::DateTime(_)
        ));
        assert_eq!(
            parse_input("a, b ,,c", Some(Kind::List)),
            Cell::List(vec![
                Cell::Text("a".into()),
                Cell::Text("b".into()),
                Cell::Text("c".into())
            ])
        );
    }

    #[test]
    fn parse_input_without_hint_detects_types() {
        assert_eq!(parse_input("true", None), Cell::Bool(true));
        assert_eq!(parse_input("42", None), Cell::Number(42.0));
        assert!(matches!(parse_input("2026-01-01", None), Cell::Date(_)));
        assert_eq!(parse_input("hello", None), Cell::Text("hello".into()));
    }

    #[test]
    fn values_map_to_frontmatter() {
        assert_eq!(to_value(&Cell::Date(0)), Value::Date("1970-01-01".into()));
        assert_eq!(
            to_value(&Cell::DateTime(3_600_000)),
            Value::DateTime("1970-01-01T01:00:00".into())
        );
    }
}
