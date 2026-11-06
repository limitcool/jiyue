//! Syntax tree of a parsed query.

use serde::Serialize;

use crate::cell::Cell;
use crate::date::Duration;

/// How the result is meant to be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Table,
    List,
    Cards,
}

/// Row-level built-in functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Today,
    Now,
    Date,
    Len,
    Lower,
    Upper,
    Exists,
    Default,
    /// `count(x)`: size of a list, 0 for null, 1 for any other value.
    Count,
    /// `prop("name")`: a frontmatter property, ignoring built-in names of the same name.
    Prop,
}

impl Func {
    pub fn name(self) -> &'static str {
        match self {
            Func::Today => "today",
            Func::Now => "now",
            Func::Date => "date",
            Func::Len => "len",
            Func::Lower => "lower",
            Func::Upper => "upper",
            Func::Exists => "exists",
            Func::Default => "default",
            Func::Count => "count",
            Func::Prop => "prop",
        }
    }
}

/// Aggregates. Only allowed as a whole `select` column; they are computed per group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggFn {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl AggFn {
    pub fn name(self) -> &'static str {
        match self {
            AggFn::Count => "count",
            AggFn::Sum => "sum",
            AggFn::Avg => "avg",
            AggFn::Min => "min",
            AggFn::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    Has,
    In,
    Add,
    Sub,
    Mul,
    Div,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Literal null, bool, number or text.
    Lit(Cell),
    Dur(Duration),
    List(Vec<Expr>),
    /// A built-in (`name`, `tags`, ...) or a frontmatter property.
    Ident(String),
    Call(Func, Vec<Expr>),
    Agg(AggFn, Option<Box<Expr>>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    /// `x = null` (false = is null, true = is not null).
    IsNull(Box<Expr>, bool),
    Bin(BinOp, Box<Expr>, Box<Expr>),
}

/// Which notes a query reads.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    All,
    /// Recursive. Stored normalized: forward slashes, no leading or trailing slash.
    Folder(String),
    /// Matches the tag and its nested children. Stored without `#`.
    Tag(String),
    /// Notes that link to the named note.
    LinkTo(String),
    And(Box<Source>, Box<Source>),
    Or(Box<Source>, Box<Source>),
    Not(Box<Source>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectItem {
    pub expr: Expr,
    /// Alias, or the expression as written.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SortItem {
    pub expr: Expr,
    pub desc: bool,
}

/// A parsed query.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub source: Source,
    pub filter: Option<Expr>,
    pub select: Vec<SelectItem>,
    pub sort: Vec<SortItem>,
    pub group_by: Option<Expr>,
    pub limit: Option<usize>,
    pub view: ViewKind,
    /// True when the query needs backlink data (`backlinks`, `count(backlinks)`, `link_to`).
    pub uses_backlinks: bool,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            source: Source::All,
            filter: None,
            select: Vec::new(),
            sort: Vec::new(),
            group_by: None,
            limit: None,
            view: ViewKind::Table,
            uses_backlinks: false,
        }
    }
}

/// Calls `f` on `e` and on every sub-expression, parents first.
pub fn visit_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match e {
        Expr::List(items) | Expr::Call(_, items) => {
            for x in items {
                visit_expr(x, f);
            }
        }
        Expr::Agg(_, Some(a)) | Expr::Not(a) | Expr::Neg(a) | Expr::IsNull(a, _) => {
            visit_expr(a, f);
        }
        Expr::Bin(_, a, b) => {
            visit_expr(a, f);
            visit_expr(b, f);
        }
        Expr::Lit(_) | Expr::Dur(_) | Expr::Ident(_) | Expr::Agg(_, None) => {}
    }
}

/// True when the expression contains an aggregate anywhere.
pub fn contains_agg(e: &Expr) -> bool {
    let mut found = false;
    visit_expr(e, &mut |x| {
        if matches!(x, Expr::Agg(..)) {
            found = true;
        }
    });
    found
}

/// Names of the built-in note fields that are not frontmatter properties.
pub const BUILTINS: [&str; 10] = [
    "name",
    "path",
    "folder",
    "tags",
    "ctime",
    "mtime",
    "size",
    "words",
    "backlinks",
    "links",
];

pub fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

/// Every source name, for error messages.
pub fn source_names() -> &'static [&'static str] {
    &["folder", "tag", "link_to", "all"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visitor_reaches_nested_nodes() {
        let e = Expr::Bin(
            BinOp::And,
            Box::new(Expr::Not(Box::new(Expr::Ident("a".into())))),
            Box::new(Expr::Call(Func::Count, vec![Expr::Ident("b".into())])),
        );
        let mut names = Vec::new();
        visit_expr(&e, &mut |x| {
            if let Expr::Ident(n) = x {
                names.push(n.clone());
            }
        });
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn aggregate_detection() {
        assert!(contains_agg(&Expr::Bin(
            BinOp::Add,
            Box::new(Expr::Agg(
                AggFn::Sum,
                Some(Box::new(Expr::Ident("x".into())))
            )),
            Box::new(Expr::Lit(Cell::Number(1.0))),
        )));
        assert!(!contains_agg(&Expr::Ident("x".into())));
    }

    #[test]
    fn builtins_are_known() {
        assert!(is_builtin("backlinks"));
        assert!(!is_builtin("status"));
    }
}
