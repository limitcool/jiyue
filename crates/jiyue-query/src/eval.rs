//! Expression evaluation against one note. Never panics: type mismatches give `null`.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::ast::{BinOp, Expr, Func};
use crate::cell::{self, Cell};
use crate::date::{self, DAY_MS, Duration};
use crate::index::{NoteRecord, stem_of};

/// Per-query data shared by all rows.
pub(crate) struct Env<'a> {
    /// Current local time, as naive epoch milliseconds.
    pub now_local: i64,
    /// Current local day number.
    pub today: i64,
    /// Offset added to UTC file times so they compare with local `now()`.
    pub offset_ms: i64,
    /// Target path -> sorted source paths.
    pub backlinks: &'a HashMap<String, Vec<String>>,
}

/// A value during evaluation: a cell, or a duration literal (which only combines with dates
/// and other durations).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    C(Cell),
    D(Duration),
}

impl Val {
    fn null() -> Val {
        Val::C(Cell::Null)
    }
}

pub(crate) fn to_cell(v: Val) -> Cell {
    match v {
        Val::C(c) => c,
        Val::D(d) => Cell::Text(d.display()),
    }
}

pub(crate) fn truthy(v: &Val) -> bool {
    match v {
        Val::C(Cell::Null) => false,
        Val::C(Cell::Bool(b)) => *b,
        Val::C(Cell::Number(n)) => *n != 0.0 && !n.is_nan(),
        Val::C(Cell::Text(s)) => !s.is_empty(),
        Val::C(Cell::List(items)) => !items.is_empty(),
        Val::C(Cell::Date(_) | Cell::DateTime(_)) => true,
        Val::D(_) => true,
    }
}

pub(crate) fn eval(e: &Expr, rec: &NoteRecord, env: &Env<'_>) -> Val {
    match e {
        Expr::Lit(c) => Val::C(c.clone()),
        Expr::Dur(d) => Val::D(*d),
        Expr::List(items) => Val::C(Cell::List(
            items.iter().map(|x| to_cell(eval(x, rec, env))).collect(),
        )),
        Expr::Ident(name) => Val::C(ident(name, rec, env)),
        Expr::Call(f, args) => call(*f, args, rec, env),
        // Aggregates are computed by the executor, never per row.
        Expr::Agg(..) => Val::null(),
        Expr::Not(x) => Val::C(Cell::Bool(!truthy(&eval(x, rec, env)))),
        Expr::Neg(x) => match eval(x, rec, env) {
            Val::C(Cell::Number(n)) => Val::C(Cell::Number(-n)),
            Val::D(d) => Val::D(Duration {
                months: d.months.saturating_neg(),
                ms: d.ms.saturating_neg(),
            }),
            _ => Val::null(),
        },
        Expr::IsNull(x, negated) => {
            let is_null = matches!(eval(x, rec, env), Val::C(Cell::Null));
            Val::C(Cell::Bool(is_null != *negated))
        }
        Expr::Bin(op, a, b) => binary(*op, a, b, rec, env),
    }
}

/// Reads a built-in or a frontmatter property.
fn ident(name: &str, rec: &NoteRecord, env: &Env<'_>) -> Cell {
    match name {
        "name" => Cell::Text(rec.name.clone()),
        "path" => Cell::Text(rec.path.clone()),
        "folder" => Cell::Text(rec.folder.clone()),
        "tags" => Cell::List(rec.tags.iter().map(|t| Cell::Text(t.clone())).collect()),
        "ctime" => Cell::DateTime(rec.ctime_ms.saturating_add(env.offset_ms)),
        "mtime" => Cell::DateTime(rec.mtime_ms.saturating_add(env.offset_ms)),
        "size" => Cell::Number(rec.size as f64),
        "words" => Cell::Number(rec.word_count as f64),
        "links" => Cell::List(
            rec.outgoing_links
                .iter()
                .map(|l| Cell::Text(l.clone()))
                .collect(),
        ),
        "backlinks" => Cell::List(
            env.backlinks
                .get(&rec.path)
                .map(|srcs| srcs.iter().map(|p| Cell::Text(stem_of(p))).collect())
                .unwrap_or_default(),
        ),
        _ => rec.props.get(name).cloned().unwrap_or(Cell::Null),
    }
}

fn arg(args: &[Expr], i: usize, rec: &NoteRecord, env: &Env<'_>) -> Val {
    match args.get(i) {
        Some(e) => eval(e, rec, env),
        None => Val::null(),
    }
}

fn call(f: Func, args: &[Expr], rec: &NoteRecord, env: &Env<'_>) -> Val {
    match f {
        Func::Today => Val::C(Cell::Date(env.today)),
        Func::Now => Val::C(Cell::DateTime(env.now_local)),
        Func::Date => match arg(args, 0, rec, env) {
            Val::C(Cell::Text(s)) => Val::C(cell::parse_any(&s).unwrap_or(Cell::Null)),
            _ => Val::null(),
        },
        Func::Len => match arg(args, 0, rec, env) {
            Val::C(Cell::Text(s)) => Val::C(Cell::Number(s.chars().count() as f64)),
            Val::C(Cell::List(items)) => Val::C(Cell::Number(items.len() as f64)),
            _ => Val::null(),
        },
        Func::Lower => match arg(args, 0, rec, env) {
            Val::C(Cell::Text(s)) => Val::C(Cell::Text(s.to_lowercase())),
            other => other,
        },
        Func::Upper => match arg(args, 0, rec, env) {
            Val::C(Cell::Text(s)) => Val::C(Cell::Text(s.to_uppercase())),
            other => other,
        },
        Func::Exists => Val::C(Cell::Bool(!matches!(
            arg(args, 0, rec, env),
            Val::C(Cell::Null)
        ))),
        Func::Default => match arg(args, 0, rec, env) {
            Val::C(Cell::Null) => arg(args, 1, rec, env),
            other => other,
        },
        Func::Count => match arg(args, 0, rec, env) {
            Val::C(Cell::List(items)) => Val::C(Cell::Number(items.len() as f64)),
            Val::C(Cell::Null) => Val::C(Cell::Number(0.0)),
            _ => Val::C(Cell::Number(1.0)),
        },
        Func::Prop => match args.first() {
            Some(Expr::Lit(Cell::Text(name))) => {
                Val::C(rec.props.get(name).cloned().unwrap_or(Cell::Null))
            }
            _ => Val::null(),
        },
    }
}

fn binary(op: BinOp, a: &Expr, b: &Expr, rec: &NoteRecord, env: &Env<'_>) -> Val {
    match op {
        BinOp::And => {
            if !truthy(&eval(a, rec, env)) {
                return Val::C(Cell::Bool(false));
            }
            Val::C(Cell::Bool(truthy(&eval(b, rec, env))))
        }
        BinOp::Or => {
            if truthy(&eval(a, rec, env)) {
                return Val::C(Cell::Bool(true));
            }
            Val::C(Cell::Bool(truthy(&eval(b, rec, env))))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
            let x = eval(a, rec, env);
            let y = eval(b, rec, env);
            arith(op, &x, &y)
        }
        _ => {
            let x = eval(a, rec, env);
            let y = eval(b, rec, env);
            Val::C(Cell::Bool(compare_op(op, &x, &y)))
        }
    }
}

fn apply_ordering(op: BinOp, ord: Option<Ordering>) -> bool {
    match op {
        BinOp::Eq => ord == Some(Ordering::Equal),
        BinOp::Ne => ord.is_some_and(|o| o != Ordering::Equal),
        BinOp::Lt => ord == Some(Ordering::Less),
        BinOp::Le => matches!(ord, Some(Ordering::Less | Ordering::Equal)),
        BinOp::Gt => ord == Some(Ordering::Greater),
        BinOp::Ge => matches!(ord, Some(Ordering::Greater | Ordering::Equal)),
        _ => false,
    }
}

/// Days of a duration without calendar months, used when comparing with numbers.
fn dur_days(d: &Duration) -> Option<f64> {
    if d.months == 0 {
        Some(d.ms as f64 / DAY_MS as f64)
    } else {
        None
    }
}

fn compare_op(op: BinOp, x: &Val, y: &Val) -> bool {
    match (x, y) {
        (Val::C(a), Val::C(b)) => match op {
            BinOp::Eq => cell::equals(a, b),
            BinOp::Ne => !a.is_null() && !b.is_null() && !cell::equals(a, b),
            BinOp::Contains => contains(a, b),
            BinOp::Has => has(a, b),
            BinOp::In => in_list(a, b),
            _ => apply_ordering(op, cell::compare(a, b)),
        },
        (Val::D(a), Val::D(b)) => {
            let ord = if a.months == 0 && b.months == 0 {
                Some(a.ms.cmp(&b.ms))
            } else if a == b {
                Some(Ordering::Equal)
            } else {
                None
            };
            apply_ordering(op, ord)
        }
        (Val::D(d), Val::C(Cell::Number(n))) | (Val::C(Cell::Number(n)), Val::D(d)) => {
            let days = dur_days(d);
            let ord = match days {
                Some(v) => v.partial_cmp(n),
                None => None,
            };
            // Keep operand order: the duration may be on either side.
            let ord = if matches!(x, Val::D(_)) {
                ord
            } else {
                ord.map(Ordering::reverse)
            };
            apply_ordering(op, ord)
        }
        _ => {
            op == BinOp::Ne && !matches!(x, Val::C(Cell::Null)) && !matches!(y, Val::C(Cell::Null))
        }
    }
}

/// `list contains item`, or substring for text.
fn contains(a: &Cell, b: &Cell) -> bool {
    match a {
        Cell::List(items) => items.iter().any(|x| cell::equals(x, b)),
        Cell::Text(s) => match b {
            Cell::Null => false,
            Cell::Text(sub) => s.contains(sub.as_str()),
            other => s.contains(other.display().as_str()),
        },
        _ => false,
    }
}

/// `list has item`, or a whole word (split on commas and spaces) for text.
fn has(a: &Cell, b: &Cell) -> bool {
    if b.is_null() {
        return false;
    }
    match a {
        Cell::List(items) => items.iter().any(|x| cell::equals(x, b)),
        Cell::Text(s) => {
            let needle = b.display();
            s.split(|c: char| c == ',' || c.is_whitespace())
                .any(|t| t == needle)
        }
        _ => false,
    }
}

/// `value in [..]`: the right side is a list.
fn in_list(a: &Cell, b: &Cell) -> bool {
    match b {
        Cell::List(items) => items.iter().any(|x| cell::equals(a, x)),
        _ => false,
    }
}

fn arith(op: BinOp, a: &Val, b: &Val) -> Val {
    match (a, b) {
        (Val::C(Cell::Number(x)), Val::C(Cell::Number(y))) => num_op(op, *x, *y),
        (Val::C(Cell::Text(x)), Val::C(Cell::Text(y))) if op == BinOp::Add => {
            Val::C(Cell::Text(format!("{x}{y}")))
        }
        (Val::C(c @ (Cell::Date(_) | Cell::DateTime(_))), Val::D(d))
            if matches!(op, BinOp::Add | BinOp::Sub) =>
        {
            Val::C(add_dur(c, d, op == BinOp::Sub))
        }
        (Val::D(d), Val::C(c @ (Cell::Date(_) | Cell::DateTime(_)))) if op == BinOp::Add => {
            Val::C(add_dur(c, d, false))
        }
        (
            Val::C(x @ (Cell::Date(_) | Cell::DateTime(_))),
            Val::C(y @ (Cell::Date(_) | Cell::DateTime(_))),
        ) if op == BinOp::Sub => match (x.as_ms(), y.as_ms()) {
            (Some(p), Some(q)) => {
                let days = p.saturating_sub(q) as f64 / DAY_MS as f64;
                Val::C(Cell::Number(days))
            }
            _ => Val::null(),
        },
        (Val::D(x), Val::D(y)) if matches!(op, BinOp::Add | BinOp::Sub) => {
            let sign = if op == BinOp::Sub { -1 } else { 1 };
            Val::D(Duration {
                months: x.months.saturating_add(y.months.saturating_mul(sign)),
                ms: x.ms.saturating_add(y.ms.saturating_mul(sign)),
            })
        }
        (Val::C(Cell::Number(n)), Val::D(d)) | (Val::D(d), Val::C(Cell::Number(n)))
            if op == BinOp::Mul =>
        {
            scale(d, *n)
        }
        _ => Val::null(),
    }
}

fn num_op(op: BinOp, x: f64, y: f64) -> Val {
    let r = match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div => {
            if y == 0.0 {
                return Val::null();
            }
            x / y
        }
        _ => return Val::null(),
    };
    if r.is_finite() {
        Val::C(Cell::Number(r))
    } else {
        Val::null()
    }
}

fn scale(d: &Duration, n: f64) -> Val {
    if n.fract() != 0.0 || n.abs() > 1.0e6 {
        return Val::null();
    }
    let k = n as i64;
    Val::D(Duration {
        months: d.months.saturating_mul(k),
        ms: d.ms.saturating_mul(k),
    })
}

/// Adds or subtracts a duration from a date or date-time. Calendar months are applied first
/// (clamping to the end of the month), then the exact part.
pub(crate) fn add_dur(base: &Cell, d: &Duration, subtract: bool) -> Cell {
    let sign: i64 = if subtract { -1 } else { 1 };
    let months = d.months.saturating_mul(sign);
    let ms = d.ms.saturating_mul(sign);
    match base {
        Cell::Date(days) => {
            let moved = date::add_months(*days, months);
            if ms % DAY_MS == 0 {
                Cell::Date(moved.saturating_add(ms / DAY_MS))
            } else {
                Cell::DateTime(date::days_to_ms(moved).saturating_add(ms))
            }
        }
        Cell::DateTime(t) => {
            let days = t.div_euclid(DAY_MS);
            let tod = t.rem_euclid(DAY_MS);
            let moved = date::add_months(days, months);
            Cell::DateTime(
                date::days_to_ms(moved)
                    .saturating_add(tod)
                    .saturating_add(ms),
            )
        }
        _ => Cell::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthiness() {
        assert!(!truthy(&Val::null()));
        assert!(truthy(&Val::C(Cell::Number(2.0))));
        assert!(!truthy(&Val::C(Cell::Number(f64::NAN))));
        assert!(!truthy(&Val::C(Cell::Text(String::new()))));
        assert!(truthy(&Val::D(Duration { months: 0, ms: 0 })));
    }

    #[test]
    fn division_by_zero_is_null() {
        assert_eq!(num_op(BinOp::Div, 1.0, 0.0), Val::null());
        assert_eq!(num_op(BinOp::Div, 1.0, 4.0), Val::C(Cell::Number(0.25)));
    }

    #[test]
    fn date_plus_days_stays_date() {
        let d = Cell::Date(date::days_from_civil(2026, 1, 30));
        let out = add_dur(
            &d,
            &Duration {
                months: 0,
                ms: 2 * DAY_MS,
            },
            false,
        );
        assert_eq!(out, Cell::Date(date::days_from_civil(2026, 2, 1)));
    }

    #[test]
    fn date_plus_hours_becomes_datetime() {
        let d = Cell::Date(0);
        let out = add_dur(
            &d,
            &Duration {
                months: 0,
                ms: 3_600_000,
            },
            false,
        );
        assert_eq!(out, Cell::DateTime(3_600_000));
    }
}
