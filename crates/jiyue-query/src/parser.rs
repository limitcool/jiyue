//! Recursive-descent parser for the query language.
//!
//! ```text
//! query   := clause*                       (each clause at most once, any order)
//! clause  := "from" source | "where" expr | "select" item ("," item)*
//!          | "sort" key ("," key)* | "group" "by" expr | "limit" INT
//!          | "view" ("table" | "list" | "cards")
//! source  := sgroup ("or" sgroup)* ; sgroup := snot ("and" snot)*
//! snot    := "not" snot | satom
//! satom   := "all" | "(" source ")" | ("folder"|"tag"|"link_to") "(" STRING ")"
//! item    := expr ["as" (NAME | STRING)]
//! key     := expr ["asc" | "desc"]
//! expr    := or ; or := and ("or" and)* ; and := not ("and" not)*
//! not     := "not" not | cmp
//! cmp     := add [("=" | "!=" | "<" | "<=" | ">" | ">=" | "contains" | "has" | "in") add]
//! add     := mul (("+" | "-") mul)* ; mul := unary (("*" | "/") unary)*
//! unary   := "-" unary | primary
//! primary := NUMBER | STRING | DURATION | true | false | null | NAME | NAME "(" args ")"
//!          | "(" expr ")" | "[" expr ("," expr)* "]"
//! ```

use std::ops::Range;

use crate::ast::{
    AggFn, BinOp, Expr, Func, Query, SelectItem, SortItem, Source, ViewKind, contains_agg,
};
use crate::cell::{Cell, format_number};
use crate::date;
use crate::error::{ParseError, nearest};
use crate::lexer::{Kw, Op, Tok, Token, lex};

const CLAUSES: [&str; 7] = ["from", "where", "select", "sort", "group", "limit", "view"];
const FUNC_NAMES: [&str; 14] = [
    "today", "now", "date", "len", "lower", "upper", "exists", "default", "prop", "count", "sum",
    "avg", "min", "max",
];
static EOF: Tok = Tok::Eof;
/// Nesting limit for parentheses, groups and `not`, which keeps the parser off the stack limit.
const MAX_DEPTH: usize = 200;

/// Parses a query. Errors carry the line and column of the problem and a hint when one is
/// obvious.
pub fn parse(src: &str) -> Result<Query, ParseError> {
    let toks = lex(src)?;
    let mut p = Parser {
        src,
        toks,
        pos: 0,
        allow_agg: false,
        last_end: 0,
        depth: 0,
    };
    let mut q = p.query()?;
    if q.select.is_empty() {
        q.select = default_select();
    }
    q.uses_backlinks = uses_backlinks(&q);
    Ok(q)
}

/// Columns shown when a query has no `select` clause.
pub fn default_select() -> Vec<SelectItem> {
    ["name", "tags", "mtime"]
        .iter()
        .map(|n| SelectItem {
            expr: Expr::Ident((*n).to_string()),
            label: (*n).to_string(),
        })
        .collect()
}

/// Normalizes a folder name from a `folder("...")` source.
pub(crate) fn normalize_folder(s: &str) -> String {
    s.trim().replace('\\', "/").trim_matches('/').to_string()
}

/// Normalizes a tag from a `tag("...")` source: no `#`, lowercase, no surrounding slashes.
pub(crate) fn normalize_tag(s: &str) -> String {
    s.trim()
        .trim_start_matches('#')
        .trim_matches('/')
        .to_ascii_lowercase()
}

fn uses_backlinks(q: &Query) -> bool {
    fn source_uses(s: &Source) -> bool {
        match s {
            Source::LinkTo(_) => true,
            Source::And(a, b) | Source::Or(a, b) => source_uses(a) || source_uses(b),
            Source::Not(a) => source_uses(a),
            Source::All | Source::Folder(_) | Source::Tag(_) => false,
        }
    }
    fn expr_uses(e: &Expr) -> bool {
        let mut found = false;
        crate::ast::visit_expr(e, &mut |x| {
            if matches!(x, Expr::Ident(n) if n == "backlinks") {
                found = true;
            }
        });
        found
    }
    source_uses(&q.source)
        || q.filter.as_ref().is_some_and(expr_uses)
        || q.select.iter().any(|s| expr_uses(&s.expr))
        || q.sort.iter().any(|s| expr_uses(&s.expr))
        || q.group_by.as_ref().is_some_and(expr_uses)
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(n) | Tok::Quoted(n) => format!("`{n}`"),
        Tok::Str(_) => "text".to_string(),
        Tok::Num(n) => format!("number {}", format_number(*n)),
        Tok::Dur(n, u) => format!("`{}{u}`", format_number(*n)),
        Tok::Kw(k) => format!("`{}`", k.text()),
        Tok::Op(_) => "operator".to_string(),
        Tok::LParen => "`(`".to_string(),
        Tok::RParen => "`)`".to_string(),
        Tok::LBrack => "`[`".to_string(),
        Tok::RBrack => "`]`".to_string(),
        Tok::Comma => "`,`".to_string(),
        Tok::Eof => "end of query".to_string(),
    }
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    /// True only while parsing a `select` item, where aggregates are allowed.
    allow_agg: bool,
    /// End byte of the last consumed token.
    last_end: usize,
    /// Current nesting depth of recursive rules, bounded by `MAX_DEPTH`.
    depth: usize,
}

impl Parser<'_> {
    fn tok(&self) -> &Tok {
        match self.toks.get(self.pos).or_else(|| self.toks.last()) {
            Some(t) => &t.tok,
            None => &EOF,
        }
    }

    fn peek_at(&self, k: usize) -> &Tok {
        match self.toks.get(self.pos + k).or_else(|| self.toks.last()) {
            Some(t) => &t.tok,
            None => &EOF,
        }
    }

    fn span(&self) -> Range<usize> {
        match self.toks.get(self.pos).or_else(|| self.toks.last()) {
            Some(t) => t.span.clone(),
            None => self.src.len()..self.src.len(),
        }
    }

    fn bump(&mut self) {
        if let Some(t) = self.toks.get(self.pos)
            && t.tok != Tok::Eof
        {
            self.last_end = t.span.end;
            self.pos += 1;
        }
    }

    fn at_eof(&self) -> bool {
        *self.tok() == Tok::Eof
    }

    fn err<T>(
        &self,
        span: Range<usize>,
        message: impl Into<String>,
        suggestion: Option<String>,
    ) -> Result<T, ParseError> {
        Err(ParseError::at(self.src, span, message, suggestion))
    }

    fn expect(&mut self, want: &Tok, message: &str) -> Result<(), ParseError> {
        if self.tok() == want {
            self.bump();
            Ok(())
        } else {
            let span = self.span();
            self.err(span, message, None)
        }
    }

    fn once(
        &self,
        seen: &mut Vec<&'static str>,
        name: &'static str,
        span: Range<usize>,
    ) -> Result<(), ParseError> {
        if seen.contains(&name) {
            return self.err(
                span,
                format!("duplicate `{name}` clause"),
                Some(format!(
                    "keep one `{name}` clause and combine conditions with `and`"
                )),
            );
        }
        seen.push(name);
        Ok(())
    }

    // ----- clauses ---------------------------------------------------------------------

    fn query(&mut self) -> Result<Query, ParseError> {
        let mut q = Query::default();
        let mut seen: Vec<&'static str> = Vec::new();
        while !self.at_eof() {
            let span = self.span();
            let tok = self.tok().clone();
            match tok {
                Tok::Kw(Kw::From) => {
                    self.once(&mut seen, "from", span)?;
                    self.bump();
                    q.source = self.source_or()?;
                }
                Tok::Kw(Kw::Where) => {
                    self.once(&mut seen, "where", span)?;
                    self.bump();
                    q.filter = Some(self.or_expr()?);
                }
                Tok::Kw(Kw::Select) => {
                    self.once(&mut seen, "select", span)?;
                    self.bump();
                    q.select = self.select_list()?;
                }
                Tok::Kw(Kw::Sort) => {
                    self.once(&mut seen, "sort", span)?;
                    self.bump();
                    q.sort = self.sort_list()?;
                }
                Tok::Kw(Kw::Group) => {
                    self.once(&mut seen, "group", span)?;
                    self.bump();
                    self.expect(&Tok::Kw(Kw::By), "expected `by` after `group`")?;
                    q.group_by = Some(self.or_expr()?);
                }
                Tok::Kw(Kw::Limit) => {
                    self.once(&mut seen, "limit", span)?;
                    self.bump();
                    q.limit = Some(self.limit_value()?);
                }
                Tok::Ident(name) if name.eq_ignore_ascii_case("view") => {
                    self.once(&mut seen, "view", span)?;
                    self.bump();
                    q.view = self.view_kind()?;
                }
                Tok::Ident(name) => {
                    let hint = nearest(&name, &CLAUSES).map(|s| format!("did you mean `{s}`?"));
                    return self.err(span, format!("unknown clause `{name}`"), hint);
                }
                other => {
                    return self.err(
                        span,
                        format!(
                            "unexpected {}, expected a clause such as `from`, `where` or `select`",
                            describe(&other)
                        ),
                        None,
                    );
                }
            }
        }
        Ok(q)
    }

    fn select_list(&mut self) -> Result<Vec<SelectItem>, ParseError> {
        let mut items = Vec::new();
        loop {
            let start = self.span().start;
            let span = self.span();
            self.allow_agg = true;
            let parsed = self.or_expr();
            self.allow_agg = false;
            let expr = parsed?;
            if contains_agg(&expr) && !matches!(expr, Expr::Agg(..)) {
                return self.err(
                    span,
                    "an aggregate must be the whole select column",
                    Some("write `sum(size)` as its own column, not inside an expression".into()),
                );
            }
            let mut label = self
                .src
                .get(start..self.last_end)
                .unwrap_or("")
                .trim()
                .to_string();
            if *self.tok() == Tok::Kw(Kw::As) {
                self.bump();
                let alias_span = self.span();
                let tok = self.tok().clone();
                label = match tok {
                    Tok::Ident(n) | Tok::Quoted(n) | Tok::Str(n) => {
                        self.bump();
                        n
                    }
                    _ => {
                        return self.err(alias_span, "expected a name after `as`", None);
                    }
                };
            }
            items.push(SelectItem { expr, label });
            if *self.tok() == Tok::Comma {
                self.bump();
            } else {
                break;
            }
        }
        Ok(items)
    }

    fn sort_list(&mut self) -> Result<Vec<SortItem>, ParseError> {
        let mut items = Vec::new();
        loop {
            let expr = self.or_expr()?;
            let mut desc = false;
            match self.tok().clone() {
                Tok::Kw(Kw::Asc) => self.bump(),
                Tok::Kw(Kw::Desc) => {
                    desc = true;
                    self.bump();
                }
                _ => {}
            }
            items.push(SortItem { expr, desc });
            if *self.tok() == Tok::Comma {
                self.bump();
            } else {
                break;
            }
        }
        Ok(items)
    }

    fn limit_value(&mut self) -> Result<usize, ParseError> {
        let span = self.span();
        match self.tok().clone() {
            Tok::Num(n) if n.fract() == 0.0 && (0.0..=1e9).contains(&n) => {
                self.bump();
                Ok(n as usize)
            }
            _ => self.err(
                span,
                "`limit` needs a whole number",
                Some("for example: limit 50".to_string()),
            ),
        }
    }

    fn view_kind(&mut self) -> Result<ViewKind, ParseError> {
        let span = self.span();
        let name = match self.tok().clone() {
            Tok::Ident(n) => n,
            _ => {
                return self.err(
                    span,
                    "expected `table`, `list` or `cards` after `view`",
                    None,
                );
            }
        };
        let lower = name.to_ascii_lowercase();
        let kind = match lower.as_str() {
            "table" => ViewKind::Table,
            "list" => ViewKind::List,
            "cards" => ViewKind::Cards,
            _ => {
                let hint = nearest(&lower, &["table", "list", "cards"])
                    .map(|s| format!("did you mean `{s}`?"));
                return self.err(span, format!("unknown view `{name}`"), hint);
            }
        };
        self.bump();
        Ok(kind)
    }

    /// Enters one level of recursion. Pair every successful call with `self.depth -= 1`.
    fn descend(&mut self) -> Result<(), ParseError> {
        if self.depth >= MAX_DEPTH {
            let span = self.span();
            return self.err(
                span,
                "expression is nested too deeply",
                Some("simplify the condition or split it into several queries".to_string()),
            );
        }
        self.depth += 1;
        Ok(())
    }

    // ----- sources ---------------------------------------------------------------------

    fn source_or(&mut self) -> Result<Source, ParseError> {
        self.descend()?;
        let r = self.source_or_chain();
        self.depth -= 1;
        r
    }

    fn source_or_chain(&mut self) -> Result<Source, ParseError> {
        let mut lhs = self.source_and()?;
        while *self.tok() == Tok::Kw(Kw::Or) {
            self.bump();
            let rhs = self.source_and()?;
            lhs = Source::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn source_and(&mut self) -> Result<Source, ParseError> {
        let mut lhs = self.source_not()?;
        while *self.tok() == Tok::Kw(Kw::And) {
            self.bump();
            let rhs = self.source_not()?;
            lhs = Source::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn source_not(&mut self) -> Result<Source, ParseError> {
        self.descend()?;
        let r = self.source_not_inner();
        self.depth -= 1;
        r
    }

    fn source_not_inner(&mut self) -> Result<Source, ParseError> {
        if *self.tok() == Tok::Kw(Kw::Not) {
            self.bump();
            return Ok(Source::Not(Box::new(self.source_not()?)));
        }
        self.source_atom()
    }

    fn source_atom(&mut self) -> Result<Source, ParseError> {
        let span = self.span();
        match self.tok().clone() {
            Tok::Kw(Kw::All) => {
                self.bump();
                Ok(Source::All)
            }
            Tok::LParen => {
                self.bump();
                let inner = self.source_or()?;
                self.expect(&Tok::RParen, "expected `)` to close the source group")?;
                Ok(inner)
            }
            Tok::Ident(name) => {
                let lower = name.to_ascii_lowercase();
                if !matches!(lower.as_str(), "folder" | "tag" | "link_to") {
                    let hint = nearest(&lower, &["folder", "tag", "link_to", "all"])
                        .map(|s| format!("did you mean `{s}`?"));
                    return self.err(span, format!("unknown source `{name}`"), hint);
                }
                self.bump();
                self.expect(&Tok::LParen, &format!("expected `(` after `{name}`"))?;
                let arg_span = self.span();
                let arg = match self.tok().clone() {
                    Tok::Str(s) => s,
                    _ => {
                        return self.err(
                            arg_span,
                            format!("`{lower}` needs a quoted name"),
                            Some(format!("for example {lower}(\"name\")")),
                        );
                    }
                };
                self.bump();
                self.expect(&Tok::RParen, "expected `)` after the name")?;
                Ok(match lower.as_str() {
                    "folder" => Source::Folder(normalize_folder(&arg)),
                    "tag" => Source::Tag(normalize_tag(&arg)),
                    _ => Source::LinkTo(arg.trim().to_string()),
                })
            }
            other => self.err(
                span,
                format!("expected a source after `from`, found {}", describe(&other)),
                Some("use folder(\"name\"), tag(\"name\"), link_to(\"note\") or all".to_string()),
            ),
        }
    }

    // ----- expressions -----------------------------------------------------------------

    fn or_expr(&mut self) -> Result<Expr, ParseError> {
        self.descend()?;
        let r = self.or_chain();
        self.depth -= 1;
        r
    }

    fn or_chain(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.and_expr()?;
        while *self.tok() == Tok::Kw(Kw::Or) {
            self.bump();
            let rhs = self.and_expr()?;
            lhs = Expr::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.not_expr()?;
        while *self.tok() == Tok::Kw(Kw::And) {
            self.bump();
            let rhs = self.not_expr()?;
            lhs = Expr::Bin(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn not_expr(&mut self) -> Result<Expr, ParseError> {
        self.descend()?;
        let r = self.not_inner();
        self.depth -= 1;
        r
    }

    fn not_inner(&mut self) -> Result<Expr, ParseError> {
        if *self.tok() == Tok::Kw(Kw::Not) {
            self.bump();
            return Ok(Expr::Not(Box::new(self.not_expr()?)));
        }
        self.cmp_expr()
    }

    fn cmp_op(&self) -> Option<BinOp> {
        match self.tok() {
            Tok::Op(Op::Eq) => Some(BinOp::Eq),
            Tok::Op(Op::Ne) => Some(BinOp::Ne),
            Tok::Op(Op::Lt) => Some(BinOp::Lt),
            Tok::Op(Op::Le) => Some(BinOp::Le),
            Tok::Op(Op::Gt) => Some(BinOp::Gt),
            Tok::Op(Op::Ge) => Some(BinOp::Ge),
            Tok::Kw(Kw::Contains) => Some(BinOp::Contains),
            Tok::Kw(Kw::Has) => Some(BinOp::Has),
            Tok::Kw(Kw::In) => Some(BinOp::In),
            _ => None,
        }
    }

    fn cmp_expr(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.add_expr()?;
        let Some(op) = self.cmp_op() else {
            return Ok(lhs);
        };
        self.bump();
        let rhs = self.add_expr()?;
        let expr = if matches!(op, BinOp::Eq | BinOp::Ne) {
            let negated = op == BinOp::Ne;
            if matches!(rhs, Expr::Lit(Cell::Null)) {
                Expr::IsNull(Box::new(lhs), negated)
            } else if matches!(lhs, Expr::Lit(Cell::Null)) {
                Expr::IsNull(Box::new(rhs), negated)
            } else {
                Expr::Bin(op, Box::new(lhs), Box::new(rhs))
            }
        } else {
            Expr::Bin(op, Box::new(lhs), Box::new(rhs))
        };
        if self.cmp_op().is_some() {
            let span = self.span();
            return self.err(
                span,
                "comparisons cannot be chained",
                Some("join them with `and`, for example `a < 1 and b < 2`".to_string()),
            );
        }
        Ok(expr)
    }

    fn add_expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.mul_expr()?;
        loop {
            let op = match self.tok() {
                Tok::Op(Op::Add) => BinOp::Add,
                Tok::Op(Op::Sub) => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.mul_expr()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn mul_expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.tok() {
                Tok::Op(Op::Mul) => BinOp::Mul,
                Tok::Op(Op::Div) => BinOp::Div,
                _ => break,
            };
            self.bump();
            let rhs = self.unary()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        self.descend()?;
        let r = self.unary_inner();
        self.depth -= 1;
        r
    }

    fn unary_inner(&mut self) -> Result<Expr, ParseError> {
        if *self.tok() == Tok::Op(Op::Sub) {
            self.bump();
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        let tok = self.tok().clone();
        match tok {
            Tok::Num(n) => {
                self.bump();
                Ok(Expr::Lit(Cell::Number(n)))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::Lit(Cell::Text(s)))
            }
            Tok::Dur(n, unit) => {
                self.bump();
                match date::duration_from_literal(n, &unit) {
                    Ok(d) => Ok(Expr::Dur(d)),
                    Err(msg) => self.err(
                        span,
                        msg,
                        Some("units are s, min, h, d, w, mo and y".to_string()),
                    ),
                }
            }
            Tok::Kw(Kw::True) => {
                self.bump();
                Ok(Expr::Lit(Cell::Bool(true)))
            }
            Tok::Kw(Kw::False) => {
                self.bump();
                Ok(Expr::Lit(Cell::Bool(false)))
            }
            Tok::Kw(Kw::Null) => {
                self.bump();
                Ok(Expr::Lit(Cell::Null))
            }
            Tok::LParen => {
                self.bump();
                let inner = self.or_expr()?;
                self.expect(&Tok::RParen, "expected `)` to close the parenthesis")?;
                Ok(inner)
            }
            Tok::LBrack => {
                self.bump();
                let mut items = Vec::new();
                if *self.tok() != Tok::RBrack {
                    loop {
                        items.push(self.or_expr()?);
                        if *self.tok() == Tok::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(&Tok::RBrack, "expected `]` to close the list")?;
                Ok(Expr::List(items))
            }
            Tok::Ident(name) => {
                if *self.peek_at(1) == Tok::LParen {
                    return self.call(name, span);
                }
                self.bump();
                Ok(Expr::Ident(name))
            }
            Tok::Quoted(name) => {
                self.bump();
                Ok(Expr::Ident(name))
            }
            Tok::Kw(k) => self.err(
                span,
                format!("`{}` cannot be used as a value here", k.text()),
                Some(format!(
                    "if you mean a property called `{}`, write it in backticks",
                    k.text()
                )),
            ),
            Tok::Eof => self.err(span, "expected a value, but the query ended", None),
            other => self.err(span, format!("unexpected {}", describe(&other)), None),
        }
    }

    fn call(&mut self, name: String, span: Range<usize>) -> Result<Expr, ParseError> {
        self.bump();
        self.bump();
        let mut args = Vec::new();
        if *self.tok() != Tok::RParen {
            loop {
                args.push(self.or_expr()?);
                if *self.tok() == Tok::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if *self.tok() != Tok::RParen {
            let at = self.span();
            return self.err(at, format!("expected `)` to close `{name}(`"), None);
        }
        self.bump();
        self.build_call(&name, args, span)
    }

    fn build_call(
        &self,
        name: &str,
        args: Vec<Expr>,
        span: Range<usize>,
    ) -> Result<Expr, ParseError> {
        let lower = name.to_ascii_lowercase();
        let agg = match lower.as_str() {
            "sum" => Some(AggFn::Sum),
            "avg" => Some(AggFn::Avg),
            "min" => Some(AggFn::Min),
            "max" => Some(AggFn::Max),
            _ => None,
        };
        if let Some(f) = agg {
            if !self.allow_agg {
                return self.err(
                    span,
                    format!(
                        "`{lower}()` is an aggregate and only works as a whole `select` column"
                    ),
                    None,
                );
            }
            return match <[Expr; 1]>::try_from(args) {
                Ok([arg]) if !contains_agg(&arg) => Ok(Expr::Agg(f, Some(Box::new(arg)))),
                Ok(_) => self.err(span, "aggregates cannot be nested", None),
                Err(_) => self.err(span, format!("`{lower}` takes exactly one argument"), None),
            };
        }
        if lower == "count" {
            return match args.len() {
                0 if self.allow_agg => Ok(Expr::Agg(AggFn::Count, None)),
                0 => self.err(
                    span,
                    "`count()` counts rows and only works as a whole `select` column",
                    Some("for a per-note count use count(links) or count(backlinks)".to_string()),
                ),
                1 if !args.iter().any(contains_agg) => Ok(Expr::Call(Func::Count, args)),
                _ => self.err(span, "aggregates cannot be nested", None),
            };
        }
        let (func, min, max) = match lower.as_str() {
            "today" => (Func::Today, 0, 0),
            "now" => (Func::Now, 0, 0),
            "date" => (Func::Date, 1, 1),
            "len" => (Func::Len, 1, 1),
            "lower" => (Func::Lower, 1, 1),
            "upper" => (Func::Upper, 1, 1),
            "exists" => (Func::Exists, 1, 1),
            "default" => (Func::Default, 2, 2),
            "prop" => (Func::Prop, 1, 1),
            "folder" | "tag" | "link_to" => {
                return self.err(
                    span,
                    format!("`{lower}()` is a source and only works after `from`"),
                    Some(format!("for example: from {lower}(\"name\")")),
                );
            }
            _ => {
                let hint = nearest(&lower, &FUNC_NAMES).map(|s| format!("did you mean `{s}()`?"));
                return self.err(span, format!("unknown function `{name}`"), hint);
            }
        };
        if args.len() < min || args.len() > max {
            let expected = if min == max {
                min.to_string()
            } else {
                format!("{min} to {max}")
            };
            return self.err(
                span,
                format!("`{lower}` takes {expected} argument(s), got {}", args.len()),
                None,
            );
        }
        if args.iter().any(contains_agg) {
            return self.err(span, "aggregates cannot be nested", None);
        }
        match func {
            Func::Date => {
                if let Some(Expr::Lit(Cell::Text(s))) = args.first()
                    && date::parse_datetime(s).is_none()
                {
                    return self.err(
                        span,
                        format!("`{s}` is not a date"),
                        Some("use YYYY-MM-DD or YYYY-MM-DD HH:MM".to_string()),
                    );
                }
            }
            Func::Prop if !matches!(args.first(), Some(Expr::Lit(Cell::Text(_)))) => {
                return self.err(span, "`prop` needs a quoted property name", None);
            }
            _ => {}
        }
        Ok(Expr::Call(func, args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_selects_everything() {
        let q = parse("").unwrap_or_default();
        assert_eq!(q.source, Source::All);
        assert_eq!(q.select.len(), 3);
    }

    #[test]
    fn default_select_is_name_tags_mtime() {
        let labels: Vec<String> = parse("where true")
            .map(|q| q.select.into_iter().map(|s| s.label).collect())
            .unwrap_or_default();
        assert_eq!(labels, vec!["name", "tags", "mtime"]);
    }
}
