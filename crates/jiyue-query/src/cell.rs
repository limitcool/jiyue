//! Typed property values ("cells") with SQL-like comparison rules and a total sort order.
//!
//! Comparison rules used by the query language:
//! - A missing value (`Null`) never compares equal or ordered to anything, so a filter on a
//!   missing property is false. Use `exists(x)` or `x = null` to test for it.
//! - Numbers compare numerically; text compares by code point.
//! - Text that parses as a number or a date is coerced when compared with a number or a date.
//! - Dates and date-times compare by their epoch milliseconds.
//! - Values of incompatible types are not equal and not ordered.
//!
//! Sorting uses [`sort_order`], which is total: values are grouped by type rank and `Null`
//! always sorts last.

use std::cmp::Ordering;

use jiyue_md::frontmatter::Value;
use serde::Serialize;

use crate::date::{self, DAY_MS};

/// The type of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Null,
    Bool,
    Number,
    Text,
    Date,
    DateTime,
    List,
}

/// A decoded property value or computed result.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Cell {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    /// Days since 1970-01-01.
    Date(i64),
    /// Milliseconds since 1970-01-01T00:00 (naive wall-clock time).
    DateTime(i64),
    List(Vec<Cell>),
}

impl Cell {
    pub fn kind(&self) -> Kind {
        match self {
            Cell::Null => Kind::Null,
            Cell::Bool(_) => Kind::Bool,
            Cell::Number(_) => Kind::Number,
            Cell::Text(_) => Kind::Text,
            Cell::Date(_) => Kind::Date,
            Cell::DateTime(_) => Kind::DateTime,
            Cell::List(_) => Kind::List,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Cell::Null)
    }

    /// Converts a decoded frontmatter value. Dates that do not validate stay text.
    pub fn from_value(v: &Value) -> Cell {
        match v {
            Value::Text(s) => Cell::Text(s.clone()),
            Value::Number(n) => Cell::Number(*n),
            Value::Bool(b) => Cell::Bool(*b),
            Value::Date(s) => match date::parse_date(s) {
                Some(d) => Cell::Date(d),
                None => Cell::Text(s.clone()),
            },
            Value::DateTime(s) => match date::parse_datetime_parts(s) {
                Some((ms, true)) => Cell::DateTime(ms),
                _ => Cell::Text(s.clone()),
            },
            Value::List(items) => Cell::List(items.iter().map(|s| Cell::Text(s.clone())).collect()),
            Value::Null => Cell::Null,
        }
    }

    /// Epoch milliseconds for dates and date-times.
    pub(crate) fn as_ms(&self) -> Option<i64> {
        match self {
            Cell::Date(d) => Some(date::days_to_ms(*d)),
            Cell::DateTime(ms) => Some(*ms),
            _ => None,
        }
    }

    /// Human-readable text, as shown in a table cell.
    pub fn display(&self) -> String {
        match self {
            Cell::Null => String::new(),
            Cell::Bool(b) => b.to_string(),
            Cell::Number(n) => format_number(*n),
            Cell::Text(s) => s.clone(),
            Cell::Date(d) => date::format_date(*d),
            Cell::DateTime(ms) => date::format_datetime(*ms),
            Cell::List(items) => items
                .iter()
                .map(Cell::display)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    /// Writes a canonical key for grouping. Equal keys mean equal values of the same type.
    pub(crate) fn key_into(&self, out: &mut String) {
        use std::fmt::Write;
        match self {
            Cell::Null => out.push('N'),
            Cell::Bool(b) => out.push_str(if *b { "b1" } else { "b0" }),
            Cell::Number(n) => {
                let _ = write!(out, "n{}", n.to_bits());
            }
            Cell::Text(s) => {
                let _ = write!(out, "t{}:{s}", s.len());
            }
            Cell::Date(d) => {
                let _ = write!(out, "d{d}");
            }
            Cell::DateTime(ms) => {
                let _ = write!(out, "m{ms}");
            }
            Cell::List(items) => {
                let _ = write!(out, "l{}[", items.len());
                for it in items {
                    it.key_into(out);
                    out.push(';');
                }
                out.push(']');
            }
        }
    }
}

/// Formats a number without a trailing `.0` for whole values.
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_string()
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Parses text that is a full date or date-time.
pub fn parse_any(s: &str) -> Option<Cell> {
    match date::parse_datetime_parts(s)? {
        (ms, false) => Some(Cell::Date(ms.div_euclid(DAY_MS))),
        (ms, true) => Some(Cell::DateTime(ms)),
    }
}

fn parse_number(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Ordering used by `<`, `<=`, `>`, `>=`. `None` for missing or incompatible values.
pub fn compare(a: &Cell, b: &Cell) -> Option<Ordering> {
    match (a, b) {
        (Cell::Null, _) | (_, Cell::Null) | (Cell::List(_), _) | (_, Cell::List(_)) => None,
        (Cell::Number(x), Cell::Number(y)) => x.partial_cmp(y),
        (Cell::Text(x), Cell::Text(y)) => Some(x.cmp(y)),
        (Cell::Bool(x), Cell::Bool(y)) => Some(x.cmp(y)),
        (Cell::Number(x), Cell::Text(s)) => parse_number(s).and_then(|y| x.partial_cmp(&y)),
        (Cell::Text(s), Cell::Number(y)) => parse_number(s).and_then(|x| x.partial_cmp(y)),
        (Cell::Date(_) | Cell::DateTime(_), Cell::Date(_) | Cell::DateTime(_)) => {
            Some(a.as_ms()?.cmp(&b.as_ms()?))
        }
        (Cell::Text(s), Cell::Date(_) | Cell::DateTime(_)) => {
            let other = parse_any(s)?.as_ms()?;
            Some(other.cmp(&b.as_ms()?))
        }
        (Cell::Date(_) | Cell::DateTime(_), Cell::Text(s)) => {
            let other = parse_any(s)?.as_ms()?;
            Some(a.as_ms()?.cmp(&other))
        }
        _ => None,
    }
}

/// Equality used by `=`, `!=` and `in`. Missing values are never equal.
pub fn equals(a: &Cell, b: &Cell) -> bool {
    match (a, b) {
        (Cell::Null, _) | (_, Cell::Null) => false,
        (Cell::List(x), Cell::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equals(p, q))
        }
        _ => compare(a, b) == Some(Ordering::Equal),
    }
}

fn rank(c: &Cell) -> u8 {
    match c {
        Cell::Bool(_) => 0,
        Cell::Number(_) => 1,
        Cell::Text(_) => 2,
        Cell::Date(_) | Cell::DateTime(_) => 3,
        Cell::List(_) => 4,
        Cell::Null => 5,
    }
}

/// Total order for sorting. Values of different types are ordered by type; `Null` is last.
pub fn sort_order(a: &Cell, b: &Cell) -> Ordering {
    let (ra, rb) = (rank(a), rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (Cell::Bool(x), Cell::Bool(y)) => x.cmp(y),
        (Cell::Number(x), Cell::Number(y)) => x.total_cmp(y),
        (Cell::Text(x), Cell::Text(y)) => x.cmp(y),
        (Cell::List(x), Cell::List(y)) => {
            for (p, q) in x.iter().zip(y) {
                let o = sort_order(p, q);
                if o != Ordering::Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
        (Cell::Date(_) | Cell::DateTime(_), Cell::Date(_) | Cell::DateTime(_)) => {
            let (x, y) = (a.as_ms().unwrap_or(0), b.as_ms().unwrap_or(0));
            x.cmp(&y)
        }
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Cell {
        Cell::Date(date::parse_date(s).unwrap_or(0))
    }

    #[test]
    fn numbers_and_text_coerce_when_possible() {
        assert_eq!(
            compare(&Cell::Number(3.0), &Cell::Text("3".into())),
            Some(Ordering::Equal)
        );
        assert_eq!(compare(&Cell::Number(3.0), &Cell::Text("x".into())), None);
        assert!(equals(&Cell::Text(" 2.5 ".into()), &Cell::Number(2.5)));
    }

    #[test]
    fn text_coerces_to_dates() {
        let t = Cell::Text("2026-03-01".into());
        assert_eq!(compare(&t, &d("2026-03-02")), Some(Ordering::Less));
        assert_eq!(compare(&Cell::Text("soon".into()), &d("2026-03-02")), None);
    }

    #[test]
    fn date_and_datetime_compare_by_milliseconds() {
        let dt = Cell::DateTime(date::parse_datetime("2026-03-01 12:00").unwrap_or(0));
        assert_eq!(compare(&d("2026-03-01"), &dt), Some(Ordering::Less));
        assert_eq!(compare(&dt, &d("2026-03-01")), Some(Ordering::Greater));
    }

    #[test]
    fn nulls_are_never_equal_and_sort_last() {
        assert!(!equals(&Cell::Null, &Cell::Null));
        assert_eq!(compare(&Cell::Null, &Cell::Number(1.0)), None);
        assert_eq!(
            sort_order(&Cell::Null, &Cell::Text("a".into())),
            Ordering::Greater
        );
        assert_eq!(sort_order(&Cell::Number(1.0), &Cell::Null), Ordering::Less);
    }

    #[test]
    fn sort_order_groups_types() {
        assert_eq!(
            sort_order(&Cell::Number(9.0), &Cell::Text("a".into())),
            Ordering::Less
        );
        assert_eq!(
            sort_order(&Cell::Bool(true), &Cell::Number(0.0)),
            Ordering::Less
        );
        assert_eq!(
            sort_order(&Cell::Number(f64::NAN), &Cell::Number(1.0)),
            Ordering::Greater
        );
    }

    #[test]
    fn display_formats() {
        assert_eq!(Cell::Number(2.0).display(), "2");
        assert_eq!(Cell::Number(2.5).display(), "2.5");
        assert_eq!(
            Cell::List(vec![Cell::Text("a".into()), Cell::Text("b".into())]).display(),
            "a, b"
        );
        assert_eq!(Cell::Date(0).display(), "1970-01-01");
        assert_eq!(Cell::Null.display(), "");
    }

    #[test]
    fn from_value_maps_all_variants() {
        assert_eq!(Cell::from_value(&Value::Number(1.5)), Cell::Number(1.5));
        assert_eq!(Cell::from_value(&Value::Bool(true)), Cell::Bool(true));
        assert_eq!(Cell::from_value(&Value::Null), Cell::Null);
        assert_eq!(
            Cell::from_value(&Value::Date("2026-02-30".into())),
            Cell::Text("2026-02-30".into())
        );
        assert_eq!(
            Cell::from_value(&Value::List(vec!["a".into()])),
            Cell::List(vec![Cell::Text("a".into())])
        );
        assert!(matches!(
            Cell::from_value(&Value::Date("2026-02-03".into())),
            Cell::Date(_)
        ));
        assert!(matches!(
            Cell::from_value(&Value::DateTime("2026-02-03T10:00".into())),
            Cell::DateTime(_)
        ));
    }

    #[test]
    fn group_keys_distinguish_types() {
        let mut a = String::new();
        let mut b = String::new();
        Cell::Text("1".into()).key_into(&mut a);
        Cell::Number(1.0).key_into(&mut b);
        assert_ne!(a, b);
    }

    #[test]
    fn list_equality_is_elementwise() {
        let x = Cell::List(vec![Cell::Number(1.0), Cell::Number(2.0)]);
        let y = Cell::List(vec![Cell::Text("1".into()), Cell::Text("2".into())]);
        assert!(equals(&x, &y));
        assert!(!equals(&x, &Cell::List(vec![Cell::Number(1.0)])));
    }

    #[test]
    fn parse_any_picks_kind() {
        assert_eq!(
            parse_any("2026-01-02"),
            Some(Cell::Date(date::parse_date("2026-01-02").unwrap_or(0)))
        );
        assert!(matches!(
            parse_any("2026-01-02 10:00"),
            Some(Cell::DateTime(_))
        ));
        assert_eq!(parse_any("nope"), None);
    }
}
