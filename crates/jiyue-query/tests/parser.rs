//! Parser tests: valid and invalid queries, CJK names, precedence, durations, errors.

use jiyue_query::ast::{BinOp, Expr, Func, Source, ViewKind};
use jiyue_query::{AggFn, Cell, Duration, ParseError, Query, parse};

fn ok(src: &str) -> Query {
    match parse(src) {
        Ok(q) => q,
        Err(e) => panic!("expected `{src}` to parse, got: {e}"),
    }
}

fn err(src: &str) -> ParseError {
    match parse(src) {
        Ok(_) => panic!("expected `{src}` to fail"),
        Err(e) => e,
    }
}

fn where_expr(src: &str) -> Expr {
    match ok(src).filter {
        Some(e) => e,
        None => panic!("no filter in {src}"),
    }
}

#[test]
fn parses_the_full_example_from_the_spec() {
    let q = ok(
        "from folder(\"项目\") and tag(\"rust\")\nwhere status = \"进行中\" and due < today() + 7d\nselect name, status, due, tags\nsort due asc, mtime desc\ngroup by status\nlimit 50\nview table",
    );
    assert!(matches!(q.source, Source::And(..)));
    assert!(q.filter.is_some());
    assert_eq!(q.select.len(), 4);
    assert_eq!(q.sort.len(), 2);
    assert!(!q.sort[0].desc);
    assert!(q.sort[1].desc);
    assert!(q.group_by.is_some());
    assert_eq!(q.limit, Some(50));
    assert_eq!(q.view, ViewKind::Table);
}

#[test]
fn empty_query_means_all_notes() {
    let q = ok("   \n  ");
    assert_eq!(q.source, Source::All);
    assert!(q.filter.is_none());
    assert_eq!(q.select.len(), 3, "default columns");
}

#[test]
fn clauses_may_come_in_any_order() {
    let a = ok("limit 3 sort name where x = 1 from all");
    assert_eq!(a.limit, Some(3));
    assert_eq!(a.sort.len(), 1);
    assert!(a.filter.is_some());
}

#[test]
fn duplicate_clause_is_an_error_with_hint() {
    let e = err("where a = 1 where b = 2");
    assert!(e.message.contains("duplicate `where`"), "{e}");
    assert!(e.suggestion.is_some());
}

#[test]
fn unknown_clause_suggests_the_closest_name() {
    let e = err("sor due");
    assert!(e.message.contains("unknown clause `sor`"), "{e}");
    assert_eq!(e.suggestion.as_deref(), Some("did you mean `sort`?"));
}

#[test]
fn error_position_is_line_and_column() {
    let e = err("from all\n  where status = ");
    assert_eq!(e.line, 2);
    assert!(e.column >= 1);
    assert!(e.message.contains("expected a value"), "{e}");
}

#[test]
fn error_column_counts_characters_not_bytes() {
    let e = err("where 状态 = = 1");
    assert_eq!(e.line, 1);
    assert_eq!(e.column, 12, "{e}");
}

#[test]
fn cjk_property_names_need_no_quotes() {
    let q = ok("where 状态 = \"进行中\" select 状态 as 状态名");
    match q.filter {
        Some(Expr::Bin(BinOp::Eq, l, _)) => assert_eq!(*l, Expr::Ident("状态".into())),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(q.select[0].label, "状态名");
}

#[test]
fn backticks_allow_spaces_and_keywords() {
    let q = ok("where `due date` > date(\"2026-01-01\") and `from` = 1");
    let f = q.filter.map(|e| format!("{e:?}")).unwrap_or_default();
    assert!(f.contains("due date"), "{f}");
    assert!(f.contains("\"from\""), "{f}");
}

#[test]
fn keyword_used_as_value_suggests_backticks() {
    let e = err("where from = 1");
    assert!(e.message.contains("`from`"), "{e}");
    assert!(e.suggestion.is_some_and(|s| s.contains("backticks")));
}

#[test]
fn and_binds_tighter_than_or() {
    match where_expr("where a = 1 or b = 2 and c = 3") {
        Expr::Bin(BinOp::Or, _, rhs) => assert!(matches!(*rhs, Expr::Bin(BinOp::And, ..))),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn not_binds_tighter_than_and() {
    match where_expr("where not a = 1 and b = 2") {
        Expr::Bin(BinOp::And, lhs, _) => assert!(matches!(*lhs, Expr::Not(_))),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn parentheses_override_precedence() {
    match where_expr("where (a = 1 or b = 2) and c = 3") {
        Expr::Bin(BinOp::And, lhs, _) => assert!(matches!(*lhs, Expr::Bin(BinOp::Or, ..))),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    match where_expr("where 1 + 2 * 3 = 7") {
        Expr::Bin(BinOp::Eq, lhs, _) => match *lhs {
            Expr::Bin(BinOp::Add, _, rhs) => assert!(matches!(*rhs, Expr::Bin(BinOp::Mul, ..))),
            other => panic!("unexpected lhs {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn comparisons_cannot_be_chained() {
    let e = err("where 1 < a < 3");
    assert!(e.message.contains("chained"), "{e}");
    assert!(e.suggestion.is_some());
}

#[test]
fn comparing_to_null_becomes_null_test() {
    assert_eq!(
        where_expr("where x = null"),
        Expr::IsNull(Box::new(Expr::Ident("x".into())), false)
    );
    assert_eq!(
        where_expr("where x != null"),
        Expr::IsNull(Box::new(Expr::Ident("x".into())), true)
    );
    assert_eq!(
        where_expr("where null = x"),
        Expr::IsNull(Box::new(Expr::Ident("x".into())), false)
    );
}

#[test]
fn durations_parse_with_units() {
    match where_expr("where due < today() + 7d") {
        Expr::Bin(BinOp::Lt, _, rhs) => match *rhs {
            Expr::Bin(BinOp::Add, _, d) => {
                assert_eq!(
                    *d,
                    Expr::Dur(Duration {
                        months: 0,
                        ms: 7 * 86_400_000
                    })
                );
            }
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        where_expr("where x = 2w"),
        Expr::Bin(
            BinOp::Eq,
            Box::new(Expr::Ident("x".into())),
            Box::new(Expr::Dur(Duration {
                months: 0,
                ms: 14 * 86_400_000
            }))
        )
    );
    assert!(
        matches!(where_expr("where x = 1mo"), Expr::Bin(BinOp::Eq, _, d) if matches!(*d, Expr::Dur(Duration { months: 1, .. })))
    );
}

#[test]
fn ambiguous_m_unit_is_rejected_with_hint() {
    let e = err("where x = 3m");
    assert!(e.message.contains("ambiguous"), "{e}");
}

#[test]
fn unknown_duration_unit_is_rejected() {
    let e = err("where x = 3days");
    assert!(e.message.contains("unknown duration unit"), "{e}");
}

#[test]
fn fractional_months_are_rejected() {
    assert!(parse("where x = 1.5mo").is_err());
}

#[test]
fn date_literals_are_validated_at_parse_time() {
    assert!(parse("where x = date(\"2026-02-28\")").is_ok());
    let e = err("where x = date(\"2026-02-30\")");
    assert!(e.message.contains("not a date"), "{e}");
}

#[test]
fn unknown_function_suggests_closest() {
    let e = err("where toda() = 1");
    assert!(e.message.contains("unknown function `toda`"), "{e}");
    assert_eq!(e.suggestion.as_deref(), Some("did you mean `today()`?"));
}

#[test]
fn function_arity_is_checked() {
    assert!(err("where len() = 1").message.contains("takes 1"));
    assert!(err("where default(a) = 1").message.contains("takes 2"));
    assert!(err("where today(1) = 1").message.contains("takes 0"));
}

#[test]
fn aggregates_only_work_as_whole_select_columns() {
    let e = err("where sum(size) > 1");
    assert!(e.message.contains("select"), "{e}");
    let e = err("select sum(size) + 1");
    assert!(e.message.contains("whole select column"), "{e}");
    let e = err("select count() + 1");
    assert!(
        e.message.contains("whole select column") || e.message.contains("select"),
        "{e}"
    );
}

#[test]
fn aggregates_cannot_nest() {
    let e = err("select sum(sum(size))");
    assert!(e.message.contains("nested"), "{e}");
}

#[test]
fn row_level_count_may_sit_inside_an_aggregate() {
    let q = ok("select sum(count(links)) as total");
    assert!(matches!(&q.select[0].expr, Expr::Agg(AggFn::Sum, Some(_))));
}

#[test]
fn whole_column_aggregates_parse() {
    let q = ok("select name, count() as n, sum(size), avg(words), min(due), max(due)");
    let aggs: Vec<AggFn> = q
        .select
        .iter()
        .filter_map(|s| match &s.expr {
            Expr::Agg(f, _) => Some(*f),
            _ => None,
        })
        .collect();
    assert_eq!(
        aggs,
        vec![AggFn::Count, AggFn::Sum, AggFn::Avg, AggFn::Min, AggFn::Max]
    );
}

#[test]
fn count_with_argument_is_row_level() {
    let q = ok("select count(backlinks) as n");
    assert!(matches!(&q.select[0].expr, Expr::Call(Func::Count, _)));
    assert!(q.uses_backlinks);
}

#[test]
fn folder_source_is_normalized() {
    assert_eq!(
        ok("from folder(\"/项目/\")").source,
        Source::Folder("项目".into())
    );
    assert_eq!(
        ok("from folder(\"a\\\\b\")").source,
        Source::Folder("a/b".into())
    );
}

#[test]
fn tag_source_is_normalized() {
    assert_eq!(ok("from tag(\"#Rust\")").source, Source::Tag("rust".into()));
    assert_eq!(ok("from tag(\"a/b\")").source, Source::Tag("a/b".into()));
}

#[test]
fn source_operators_nest() {
    let q = ok("from (folder(\"a\") or tag(\"x\")) and not link_to(\"Home\")");
    match q.source {
        Source::And(l, r) => {
            assert!(matches!(*l, Source::Or(..)));
            assert!(matches!(*r, Source::Not(_)));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn source_must_be_known() {
    let e = err("from notes");
    assert!(e.message.contains("unknown source"), "{e}");
    assert!(
        e.suggestion.is_none()
            || e.suggestion
                .as_deref()
                .is_some_and(|s| s.contains("folder") || s.contains("tag"))
    );
}

#[test]
fn source_needs_quoted_name() {
    let e = err("from folder(projects)");
    assert!(e.message.contains("quoted name"), "{e}");
}

#[test]
fn folder_function_outside_from_is_rejected() {
    let e = err("where folder(\"x\") = 1");
    assert!(e.message.contains("only works after `from`"), "{e}");
}

#[test]
fn limit_must_be_a_whole_number() {
    assert!(err("limit 2.5").message.contains("whole number"));
    assert!(err("limit -1").message.contains("whole number") || parse("limit -1").is_err());
    assert_eq!(ok("limit 0").limit, Some(0));
}

#[test]
fn view_names_are_case_insensitive_and_checked() {
    assert_eq!(ok("view CARDS").view, ViewKind::Cards);
    assert_eq!(ok("view list").view, ViewKind::List);
    let e = err("view tabl");
    assert_eq!(e.suggestion.as_deref(), Some("did you mean `table`?"));
}

#[test]
fn aliases_accept_names_and_strings() {
    let q = ok("select status as 状态, due as \"due date\"");
    assert_eq!(q.select[0].label, "状态");
    assert_eq!(q.select[1].label, "due date");
}

#[test]
fn select_label_is_the_source_text() {
    let q = ok("select priority*2");
    assert_eq!(q.select[0].label, "priority*2");
}

#[test]
fn default_select_applies_when_absent() {
    let labels: Vec<String> = ok("where a = 1")
        .select
        .into_iter()
        .map(|s| s.label)
        .collect();
    assert_eq!(labels, vec!["name", "tags", "mtime"]);
}

#[test]
fn group_requires_by() {
    let e = err("group status");
    assert!(e.message.contains("`by`"), "{e}");
}

#[test]
fn sort_direction_defaults_to_ascending() {
    let q = ok("sort a, b desc, c asc");
    let dirs: Vec<bool> = q.sort.iter().map(|s| s.desc).collect();
    assert_eq!(dirs, vec![false, true, false]);
}

#[test]
fn in_takes_a_list_literal() {
    match where_expr("where status in [\"a\", \"b\"]") {
        Expr::Bin(BinOp::In, _, rhs) => match *rhs {
            Expr::List(items) => assert_eq!(items.len(), 2),
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn string_escapes_are_decoded() {
    assert_eq!(
        where_expr("where a = \"x\\\"y\\n\""),
        Expr::Bin(
            BinOp::Eq,
            Box::new(Expr::Ident("a".into())),
            Box::new(Expr::Lit(Cell::Text("x\"y\n".into())))
        )
    );
}

#[test]
fn unterminated_string_points_at_the_quote() {
    let e = err("where a = \"abc");
    assert!(e.message.contains("unterminated string"), "{e}");
    assert_eq!(e.column, 11);
}

#[test]
fn hash_gets_a_tag_hint() {
    let e = err("where #rust");
    assert!(e.suggestion.is_some_and(|s| s.contains("tag(")));
}

#[test]
fn unexpected_characters_are_reported() {
    assert!(err("where a ! b").message.contains("`!`"));
    assert!(
        err("where a = 1 ?")
            .message
            .contains("unexpected character")
    );
}

#[test]
fn uses_backlinks_flag() {
    assert!(!ok("where a = 1").uses_backlinks);
    assert!(ok("where backlinks > 0").uses_backlinks);
    assert!(ok("from link_to(\"x\")").uses_backlinks);
}

#[test]
fn nesting_limit_returns_error_instead_of_overflowing() {
    let deep = format!("where {}1{}", "(".repeat(100_000), ")".repeat(100_000));
    let e = err(&deep);
    assert!(e.message.contains("nested too deeply"), "{e}");
}

#[test]
fn reasonable_nesting_still_parses() {
    let src = format!("where {}1{}", "(".repeat(50), ")".repeat(50));
    assert!(parse(&src).is_ok());
}

#[test]
fn parse_is_case_insensitive_for_keywords_and_functions() {
    let q = ok("WHERE NOT Status = \"x\" AND TODAY() > date(\"2026-01-01\") SORT name DESC");
    assert!(q.filter.is_some());
    assert!(q.sort[0].desc);
}

#[test]
fn negative_numbers_and_unary_minus() {
    assert_eq!(
        where_expr("where x = -3"),
        Expr::Bin(
            BinOp::Eq,
            Box::new(Expr::Ident("x".into())),
            Box::new(Expr::Neg(Box::new(Expr::Lit(Cell::Number(3.0)))))
        )
    );
}

#[test]
fn list_literal_can_be_empty() {
    assert_eq!(
        where_expr("where x has []"),
        Expr::Bin(
            BinOp::Has,
            Box::new(Expr::Ident("x".into())),
            Box::new(Expr::List(vec![]))
        )
    );
}

#[test]
fn prop_function_requires_a_string() {
    assert!(parse("where prop(\"due date\") = 1").is_ok());
    assert!(err("where prop(due) = 1").message.contains("quoted"));
}

#[test]
fn view_without_name_is_an_error() {
    assert!(parse("view").is_err());
}

#[test]
fn trailing_comma_is_an_error() {
    assert!(parse("select name,").is_err());
    assert!(parse("sort name,").is_err());
}

#[test]
fn stray_clause_word_after_expression_is_reported() {
    let e = err("select name status");
    assert!(e.message.contains("unknown clause `status`"), "{e}");
}
