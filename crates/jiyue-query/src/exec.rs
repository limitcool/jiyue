//! Runs a parsed query against a [`PropertyIndex`].
//!
//! Pipeline: source and filter, then stable sort (ties keep path order), then `limit`, then
//! grouping (groups keep the order of their first row; the null group goes last), then
//! aggregates per group.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::ast::{
    AggFn, Expr, Func, Query, SortItem, Source, contains_agg, is_builtin, visit_expr,
};
use crate::cell::{self, Cell, Kind};
use crate::date::DAY_MS;
use crate::error::ParseError;
use crate::eval::{Env, eval, to_cell, truthy};
use crate::index::{NoteRecord, PropertyIndex};
use crate::parser::parse;
use crate::result::{Column, Group, QueryResult, Row};

/// The clock and time zone a query runs with. Passing them in keeps queries deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryContext {
    /// Current time in UTC epoch milliseconds.
    pub now_utc_ms: i64,
    /// Local offset from UTC in minutes (for example 480 for UTC+8). Used by `today()`,
    /// `now()` and the file time built-ins.
    pub utc_offset_min: i32,
}

impl QueryContext {
    pub fn new(now_utc_ms: i64, utc_offset_min: i32) -> Self {
        QueryContext {
            now_utc_ms,
            utc_offset_min,
        }
    }

    /// The current time in UTC.
    pub fn system() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .unwrap_or(0);
        QueryContext::new(now, 0)
    }
}

/// Parses and runs a query. Parse errors are returned; evaluation never fails.
pub fn run_text(
    index: &PropertyIndex,
    src: &str,
    ctx: &QueryContext,
) -> Result<QueryResult, ParseError> {
    let q = parse(src)?;
    Ok(execute(index, &q, ctx))
}

/// Runs a query, resolving links with [`PropertyIndex::resolve_link`].
pub fn execute(index: &PropertyIndex, q: &Query, ctx: &QueryContext) -> QueryResult {
    let resolve = |t: &str| index.resolve_link(t);
    execute_with_resolver(index, q, ctx, &resolve)
}

/// Runs a query with a caller-supplied resolver that maps link text to a note path.
pub fn execute_with_resolver(
    index: &PropertyIndex,
    q: &Query,
    ctx: &QueryContext,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> QueryResult {
    let started = Instant::now();
    let offset_ms = i64::from(ctx.utc_offset_min).saturating_mul(60_000);
    let now_local = ctx.now_utc_ms.saturating_add(offset_ms);
    let today = now_local.div_euclid(DAY_MS);
    let backlinks: HashMap<String, Vec<String>> = if q.uses_backlinks {
        index.backlink_sources(resolve)
    } else {
        HashMap::new()
    };
    let source = resolve_source(&q.source, resolve);
    let env = Env {
        now_local,
        today,
        offset_ms,
        backlinks: &backlinks,
    };

    let mut warnings = Vec::new();
    check_properties(index, q, &mut warnings);

    // Source and filter.
    let mut rows: Vec<&NoteRecord> = index
        .records()
        .filter(|rec| source_matches(&source, rec, &backlinks))
        .filter(|rec| {
            q.filter
                .as_ref()
                .is_none_or(|f| truthy(&eval(f, rec, &env)))
        })
        .collect();
    let total = rows.len();

    // Stable sort: ties keep path order.
    if !q.sort.is_empty() {
        let mut keyed: Vec<(Vec<Cell>, &NoteRecord)> = rows
            .iter()
            .map(|rec| {
                let key = q
                    .sort
                    .iter()
                    .map(|s| to_cell(eval(&s.expr, rec, &env)))
                    .collect();
                (key, *rec)
            })
            .collect();
        keyed.sort_by(|p, r| compare_keys(&p.0, &r.0, &q.sort));
        rows = keyed.into_iter().map(|(_, rec)| rec).collect();
    }

    if let Some(n) = q.limit {
        rows.truncate(n);
    }

    // Grouping.
    let mut groups: Vec<(Cell, Vec<&NoteRecord>)> = Vec::new();
    match &q.group_by {
        None => groups.push((Cell::Null, rows)),
        Some(gexpr) => {
            let mut slot: HashMap<String, usize> = HashMap::new();
            for rec in rows {
                let key = to_cell(eval(gexpr, rec, &env));
                let mut k = String::new();
                key.key_into(&mut k);
                let idx = match slot.get(&k) {
                    Some(&i) => i,
                    None => {
                        groups.push((key, Vec::new()));
                        let i = groups.len() - 1;
                        slot.insert(k, i);
                        i
                    }
                };
                if let Some(g) = groups.get_mut(idx) {
                    g.1.push(rec);
                }
            }
            // Stable: the null group moves last, other groups keep their order.
            groups.sort_by_key(|(k, _)| k.is_null());
        }
    }

    // Rows and aggregates per group.
    let ncols = q.select.len();
    let mut out_groups: Vec<Group> = Vec::with_capacity(groups.len());
    for (key, members) in groups {
        let member_count = members.len();
        let mut agg_inputs: Vec<Vec<Cell>> = vec![Vec::new(); ncols];
        let mut rows_out = Vec::with_capacity(member_count);
        for rec in &members {
            let mut cells = Vec::with_capacity(ncols);
            for (ci, item) in q.select.iter().enumerate() {
                match &item.expr {
                    Expr::Agg(_, arg) => {
                        cells.push(Cell::Null);
                        if let (Some(a), Some(bucket)) = (arg, agg_inputs.get_mut(ci)) {
                            bucket.push(to_cell(eval(a, rec, &env)));
                        }
                    }
                    e => cells.push(to_cell(eval(e, rec, &env))),
                }
            }
            rows_out.push(Row {
                path: rec.path.clone(),
                cells,
            });
        }
        let aggregates: Vec<Cell> = q
            .select
            .iter()
            .enumerate()
            .map(|(ci, item)| match &item.expr {
                Expr::Agg(f, _) => {
                    let values: &[Cell] = agg_inputs.get(ci).map(Vec::as_slice).unwrap_or(&[]);
                    aggregate(*f, values, member_count)
                }
                _ => Cell::Null,
            })
            .collect();
        out_groups.push(Group {
            key,
            rows: rows_out,
            aggregates,
        });
    }

    let shown = out_groups.iter().map(|g| g.rows.len()).sum();
    let columns: Vec<Column> = q
        .select
        .iter()
        .enumerate()
        .map(|(ci, item)| {
            let is_agg = matches!(item.expr, Expr::Agg(..));
            let kind = if is_agg {
                dominant_kind(out_groups.iter().filter_map(|g| g.aggregates.get(ci)))
            } else {
                dominant_kind(
                    out_groups
                        .iter()
                        .flat_map(|g| g.rows.iter())
                        .filter_map(|r| r.cells.get(ci)),
                )
            };
            Column {
                name: item.label.clone(),
                kind,
                aggregate: is_agg,
            }
        })
        .collect();

    QueryResult {
        columns,
        groups: out_groups,
        view: q.view,
        total,
        shown,
        elapsed_us: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        warnings,
    }
}

/// The most common non-null kind. `Null` when every value is null (or there are none).
fn dominant_kind<'a>(cells: impl Iterator<Item = &'a Cell>) -> Kind {
    let mut counts: BTreeMap<Kind, usize> = BTreeMap::new();
    for c in cells {
        if !c.is_null() {
            *counts.entry(c.kind()).or_insert(0) += 1;
        }
    }
    let mut best = (0usize, Kind::Null);
    for (k, n) in counts {
        if n > best.0 {
            best = (n, k);
        }
    }
    best.1
}

fn compare_keys(a: &[Cell], b: &[Cell], items: &[SortItem]) -> Ordering {
    for (i, item) in items.iter().enumerate() {
        let ord = match (a.get(i), b.get(i)) {
            (Some(x), Some(y)) => match (x.is_null(), y.is_null()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    let o = cell::sort_order(x, y);
                    if item.desc { o.reverse() } else { o }
                }
            },
            _ => Ordering::Equal,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn number_of(c: &Cell) -> Option<f64> {
    match c {
        Cell::Number(n) => Some(*n),
        Cell::Text(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    }
}

fn aggregate(f: AggFn, values: &[Cell], rows: usize) -> Cell {
    match f {
        AggFn::Count => Cell::Number(rows as f64),
        AggFn::Sum => Cell::Number(values.iter().filter_map(number_of).sum()),
        AggFn::Avg => {
            let nums: Vec<f64> = values.iter().filter_map(number_of).collect();
            if nums.is_empty() {
                Cell::Null
            } else {
                Cell::Number(nums.iter().sum::<f64>() / nums.len() as f64)
            }
        }
        AggFn::Min => values
            .iter()
            .filter(|c| !c.is_null())
            .min_by(|a, b| cell::sort_order(a, b))
            .cloned()
            .unwrap_or(Cell::Null),
        AggFn::Max => values
            .iter()
            .filter(|c| !c.is_null())
            .max_by(|a, b| cell::sort_order(a, b))
            .cloned()
            .unwrap_or(Cell::Null),
    }
}

/// A source with `link_to` already resolved to a note path.
enum RSource {
    All,
    Folder(String),
    Tag(String),
    LinkTo(Option<String>),
    And(Box<RSource>, Box<RSource>),
    Or(Box<RSource>, Box<RSource>),
    Not(Box<RSource>),
}

fn resolve_source(s: &Source, resolve: &dyn Fn(&str) -> Option<String>) -> RSource {
    match s {
        Source::All => RSource::All,
        Source::Folder(f) => RSource::Folder(f.clone()),
        Source::Tag(t) => RSource::Tag(t.clone()),
        Source::LinkTo(t) => RSource::LinkTo(resolve(t)),
        Source::And(a, b) => RSource::And(
            Box::new(resolve_source(a, resolve)),
            Box::new(resolve_source(b, resolve)),
        ),
        Source::Or(a, b) => RSource::Or(
            Box::new(resolve_source(a, resolve)),
            Box::new(resolve_source(b, resolve)),
        ),
        Source::Not(a) => RSource::Not(Box::new(resolve_source(a, resolve))),
    }
}

/// `folder`: the folder itself or any folder below it.
fn folder_matches(folder: &str, wanted: &str) -> bool {
    if wanted.is_empty() || folder == wanted {
        return true;
    }
    folder.starts_with(wanted) && folder.as_bytes().get(wanted.len()) == Some(&b'/')
}

/// `tag`: the tag itself or a nested tag (`a/b` matches `a`), ignoring ASCII case.
fn tag_matches(tag: &str, wanted: &str) -> bool {
    if tag.eq_ignore_ascii_case(wanted) {
        return true;
    }
    match tag.get(..wanted.len()) {
        Some(head) => {
            head.eq_ignore_ascii_case(wanted) && tag.as_bytes().get(wanted.len()) == Some(&b'/')
        }
        None => false,
    }
}

fn source_matches(s: &RSource, rec: &NoteRecord, backlinks: &HashMap<String, Vec<String>>) -> bool {
    match s {
        RSource::All => true,
        RSource::Folder(f) => folder_matches(&rec.folder, f),
        RSource::Tag(t) => rec.tags.iter().any(|tag| tag_matches(tag, t)),
        RSource::LinkTo(Some(target)) => backlinks
            .get(target)
            .is_some_and(|srcs| srcs.binary_search(&rec.path).is_ok()),
        RSource::LinkTo(None) => false,
        RSource::And(a, b) => {
            source_matches(a, rec, backlinks) && source_matches(b, rec, backlinks)
        }
        RSource::Or(a, b) => source_matches(a, rec, backlinks) || source_matches(b, rec, backlinks),
        RSource::Not(a) => !source_matches(a, rec, backlinks),
    }
}

/// Warns about property names that no note has. Only when the index is not empty.
fn check_properties(index: &PropertyIndex, q: &Query, warnings: &mut Vec<String>) {
    if index.is_empty() {
        return;
    }
    let mut exprs: Vec<&Expr> = Vec::new();
    if let Some(f) = &q.filter {
        exprs.push(f);
    }
    exprs.extend(q.select.iter().map(|s| &s.expr));
    exprs.extend(q.sort.iter().map(|s| &s.expr));
    if let Some(g) = &q.group_by {
        exprs.push(g);
    }
    let mut names: BTreeSet<String> = BTreeSet::new();
    for e in exprs {
        visit_expr(e, &mut |x| match x {
            Expr::Ident(n) if !is_builtin(n) => {
                names.insert(n.clone());
            }
            Expr::Call(Func::Prop, args) => {
                if let Some(Expr::Lit(Cell::Text(n))) = args.first() {
                    names.insert(n.clone());
                }
            }
            _ => {}
        });
    }
    let known = index.property_counts();
    for name in names {
        if !known.contains_key(&name) {
            warnings.push(format!("no note has a property named `{name}`"));
        }
    }
}

/// True when any select, sort, group or filter expression uses an aggregate. Used by callers
/// that want to know whether a footer row is shown.
pub fn has_aggregates(q: &Query) -> bool {
    q.select.iter().any(|s| contains_agg(&s.expr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_matching_is_nested_and_case_insensitive() {
        assert!(tag_matches("Rust", "rust"));
        assert!(tag_matches("rust/async", "rust"));
        assert!(!tag_matches("rustacean", "rust"));
        assert!(!tag_matches("ru", "rust"));
    }

    #[test]
    fn folder_matching_is_recursive() {
        assert!(folder_matches("项目/a/b", "项目"));
        assert!(folder_matches("项目", "项目"));
        assert!(!folder_matches("项目2", "项目"));
        assert!(folder_matches("anything", ""));
    }

    #[test]
    fn aggregates_over_empty_input() {
        assert_eq!(aggregate(AggFn::Sum, &[], 0), Cell::Number(0.0));
        assert_eq!(aggregate(AggFn::Avg, &[], 0), Cell::Null);
        assert_eq!(aggregate(AggFn::Min, &[], 0), Cell::Null);
        assert_eq!(aggregate(AggFn::Count, &[], 4), Cell::Number(4.0));
    }

    #[test]
    fn dominant_kind_ignores_nulls() {
        let cells = [
            Cell::Null,
            Cell::Number(1.0),
            Cell::Number(2.0),
            Cell::Text("x".into()),
        ];
        assert_eq!(dominant_kind(cells.iter()), Kind::Number);
        assert_eq!(dominant_kind([Cell::Null].iter()), Kind::Null);
    }
}
