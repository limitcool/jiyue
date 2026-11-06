//! Evaluator tests: every operator and function, nulls, mixed types, dates around month ends.

mod common;

use common::index_from;
use common::{run, run_with, value, value_on, vault};
use jiyue_query::{Cell, QueryContext};

fn b(v: bool) -> Cell {
    Cell::Bool(v)
}

fn n(v: f64) -> Cell {
    Cell::Number(v)
}

fn t(s: &str) -> Cell {
    Cell::Text(s.to_string())
}

fn d(s: &str) -> Cell {
    Cell::Date(jiyue_query::date::parse_date(s).unwrap_or(0))
}

#[test]
fn arithmetic_follows_precedence() {
    assert_eq!(value("1 + 2 * 3"), n(7.0));
    assert_eq!(value("(1 + 2) * 3"), n(9.0));
    assert_eq!(value("10 / 4"), n(2.5));
    assert_eq!(value("-3 + 5"), n(2.0));
    assert_eq!(value("2 - 5"), n(-3.0));
}

#[test]
fn division_by_zero_gives_null() {
    assert_eq!(value("1 / 0"), Cell::Null);
}

#[test]
fn numeric_comparisons() {
    assert_eq!(value("2 < 3"), b(true));
    assert_eq!(value("3 <= 3"), b(true));
    assert_eq!(value("4 > 3"), b(true));
    assert_eq!(value("3 >= 4"), b(false));
    assert_eq!(value("2 != 3"), b(true));
}

#[test]
fn text_comparisons_are_codepoint_ordered() {
    assert_eq!(value("\"b\" > \"a\""), b(true));
    assert_eq!(value("\"进行中\" = \"进行中\""), b(true));
    assert_eq!(
        value("\"Rust\" = \"rust\""),
        b(false),
        "equality is case-sensitive"
    );
}

#[test]
fn number_and_numeric_text_compare_equal() {
    assert_eq!(value("\"10\" = 10"), b(true));
    assert_eq!(value("\"10\" < 9"), b(false));
    assert_eq!(value("\"abc\" < 3"), b(false));
}

#[test]
fn incompatible_types_are_not_equal() {
    assert_eq!(value("\"abc\" = 1"), b(false));
    assert_eq!(value("\"abc\" != 1"), b(true));
    assert_eq!(value("true = 1"), b(false));
}

#[test]
fn contains_on_text_is_substring() {
    assert_eq!(value("\"hello world\" contains \"o w\""), b(true));
    assert_eq!(value("\"hello\" contains \"xyz\""), b(false));
}

#[test]
fn contains_and_has_on_lists() {
    let idx = vault();
    assert_eq!(
        value_on(&idx, "rust-async", "tags contains \"async\""),
        b(true)
    );
    assert_eq!(value_on(&idx, "rust-async", "tags has \"rust\""), b(true));
    assert_eq!(value_on(&idx, "rust-async", "tags has \"go\""), b(false));
    assert_eq!(
        value_on(&idx, "rust-async", "tags contains \"rust/tips\""),
        b(true)
    );
}

#[test]
fn has_on_text_matches_whole_words() {
    assert_eq!(value("\"a, b c\" has \"b\""), b(true));
    assert_eq!(value("\"a,b\" has \"ab\""), b(false));
}

#[test]
fn in_checks_a_list_literal() {
    assert_eq!(value("\"完成\" in [\"完成\", \"x\"]"), b(true));
    assert_eq!(value("\"y\" in [\"完成\", \"x\"]"), b(false));
    assert_eq!(value("1 in 1"), b(false), "in needs a list on the right");
}

#[test]
fn boolean_operators_and_truthiness() {
    assert_eq!(value("not false"), b(true));
    assert_eq!(value("true and null"), b(false));
    assert_eq!(value("null or true"), b(true));
    assert_eq!(value("not (1 = 1)"), b(false));
    assert_eq!(value("(1 = 2) or (3 = 3) and false"), b(false));
    assert_eq!(value("0 or \"\""), b(false));
}

#[test]
fn null_never_equals_anything() {
    assert_eq!(value("missing = 1"), b(false));
    assert_eq!(value("missing != 1"), b(false));
    assert_eq!(value("missing < 1"), b(false));
    assert_eq!(value("missing = null"), b(true));
    assert_eq!(value("missing != null"), b(false));
    assert_eq!(value("status = null"), b(false));
    assert_eq!(value("status != null"), b(true));
}

#[test]
fn exists_and_default() {
    assert_eq!(value("exists(missing)"), b(false));
    assert_eq!(value("exists(status)"), b(true));
    assert_eq!(value("default(missing, \"x\")"), t("x"));
    assert_eq!(value("default(status, \"x\")"), t("完成"));
}

#[test]
fn length_functions() {
    assert_eq!(value("len(\"项目\")"), n(2.0));
    assert_eq!(value("len(missing)"), Cell::Null);
    let idx = vault();
    assert_eq!(value_on(&idx, "rust-async", "len(tags)"), n(3.0));
}

#[test]
fn count_of_list_null_and_scalar() {
    assert_eq!(value("count(missing)"), n(0.0));
    assert_eq!(value("count(\"x\")"), n(1.0));
    let idx = vault();
    assert_eq!(value_on(&idx, "rust-async", "count(tags)"), n(3.0));
    assert_eq!(
        value_on(&idx, "inbox", "count(links)"),
        n(2.0),
        "links are de-duplicated"
    );
}

#[test]
fn case_functions() {
    assert_eq!(value("upper(\"abc\")"), t("ABC"));
    assert_eq!(value("lower(\"ABC\")"), t("abc"));
    assert_eq!(
        value("lower(priority)"),
        Cell::Null,
        "missing values stay null"
    );
}

#[test]
fn text_concatenation() {
    assert_eq!(value("\"a\" + \"b\""), t("ab"));
}

#[test]
fn date_literal_and_today() {
    assert_eq!(value("date(\"2026-11-08\") > today()"), b(true));
    assert_eq!(value("today() = date(\"2026-11-04\")"), b(true));
    assert_eq!(
        value("date(lower(\"BAD\"))"),
        Cell::Null,
        "runtime text that is not a date gives null"
    );
}

#[test]
fn text_coerces_to_dates_in_comparisons() {
    assert_eq!(value("\"2026-03-01\" = date(\"2026-03-01\")"), b(true));
    assert_eq!(value("\"2026-10-01\" < today()"), b(true));
    assert_eq!(value("\"someday\" < today()"), b(false));
}

#[test]
fn month_end_arithmetic() {
    assert_eq!(value("date(\"2026-01-31\") + 1mo"), d("2026-02-28"));
    assert_eq!(value("date(\"2024-01-31\") + 1mo"), d("2024-02-29"));
    assert_eq!(value("date(\"2026-03-31\") - 1mo"), d("2026-02-28"));
    assert_eq!(value("date(\"2026-08-31\") + 1mo"), d("2026-09-30"));
    assert_eq!(value("date(\"2026-12-31\") + 1d"), d("2027-01-01"));
    assert_eq!(value("date(\"2026-02-28\") + 1d"), d("2026-03-01"));
    assert_eq!(value("date(\"2028-02-28\") + 1d"), d("2028-02-29"));
    assert_eq!(value("date(\"2100-02-28\") + 1d"), d("2100-03-01"));
}

#[test]
fn year_offsets_and_weeks() {
    assert_eq!(value("date(\"2024-02-29\") + 1y"), d("2025-02-28"));
    assert_eq!(value("date(\"2026-11-04\") + 2w"), d("2026-11-18"));
    assert_eq!(value("date(\"2026-11-04\") - 3d"), d("2026-11-01"));
    assert_eq!(value("date(\"2026-11-01\") + 2 * 7d"), d("2026-11-15"));
}

#[test]
fn date_minus_date_is_days() {
    assert_eq!(value("date(\"2026-11-04\") - date(\"2026-11-01\")"), n(3.0));
    assert_eq!(value("date(\"2026-03-01\") - date(\"2026-02-28\")"), n(1.0));
}

#[test]
fn datetime_arithmetic_keeps_time_of_day() {
    assert_eq!(value("now() - 2h > date(\"2026-11-04\")"), b(true));
    assert_eq!(
        value("now() - 12h = date(\"2026-11-04\")"),
        b(true),
        "midnight is a date"
    );
    let c = value("date(\"2026-11-04\") + 90min");
    assert_eq!(
        c,
        Cell::DateTime(jiyue_query::date::parse_datetime("2026-11-04 01:30").unwrap_or(0))
    );
}

#[test]
fn durations_compare_with_durations_and_numbers() {
    assert_eq!(value("7d > 3d"), b(true));
    assert_eq!(value("7d = 168h"), b(true));
    assert_eq!(value("date(\"2026-11-08\") - today() > 3"), b(true));
    assert_eq!(
        value("1mo = 30d"),
        b(false),
        "calendar months never equal days"
    );
}

#[test]
fn file_time_builtins_compare_with_now() {
    assert_eq!(value("mtime < now()"), b(true));
    assert_eq!(value("ctime < now()"), b(true));
}

#[test]
fn builtins_are_read_from_the_record() {
    let idx = vault();
    assert_eq!(value_on(&idx, "gpui", "folder"), t("项目"));
    assert_eq!(value_on(&idx, "gpui", "path"), t("项目/gpui.md"));
    assert_eq!(value_on(&idx, "gpui", "name"), t("gpui"));
    assert!(value_on(&idx, "gpui", "words") != n(0.0));
    assert_eq!(
        value_on(&idx, "gpui", "backlinks"),
        Cell::List(vec![t("inbox")])
    );
}

#[test]
fn prop_reads_frontmatter_even_when_name_is_builtin() {
    let idx = index_from(&[("a.md", "---\nname: 张三\n---\n")]);
    assert_eq!(value_on(&idx, "a", "name"), t("a"));
    assert_eq!(value_on(&idx, "a", "prop(\"name\")"), t("张三"));
}

#[test]
fn quoted_property_names_in_expressions() {
    let idx = index_from(&[("a.md", "---\ndue date: 2026-05-01\n---\n")]);
    assert_eq!(value_on(&idx, "a", "`due date`"), d("2026-05-01"));
}

#[test]
fn null_arithmetic_stays_null() {
    assert_eq!(value("missing + 1"), Cell::Null);
    assert_eq!(value("missing * 2"), Cell::Null);
    assert_eq!(value("date(\"2026-01-01\") + missing"), Cell::Null);
}

#[test]
fn list_literal_value() {
    assert_eq!(value("[1, 2]"), Cell::List(vec![n(1.0), n(2.0)]));
}

#[test]
fn today_respects_utc_offset() {
    let idx = index_from(&[("a.md", "x")]);
    // 2026-11-04 20:00 UTC is 2026-11-05 04:00 at UTC+8.
    let utc = jiyue_query::date::parse_datetime("2026-11-04 20:00").unwrap_or(0);
    let c = QueryContext::new(utc, 480);
    let r = run_with(&idx, "select today() as t", &c);
    let cell = r.rows().next().and_then(|row| row.cells.first().cloned());
    assert_eq!(cell, Some(d("2026-11-05")));
    let c0 = QueryContext::new(utc, 0);
    let r0 = run_with(&idx, "select today() as t", &c0);
    assert_eq!(
        r0.rows().next().and_then(|row| row.cells.first().cloned()),
        Some(d("2026-11-04"))
    );
}

#[test]
fn file_times_shift_with_offset() {
    let idx = index_from(&[("a.md", "x")]);
    let base = idx.get("a.md").map(|r| r.mtime_ms).unwrap_or(0);
    let c = QueryContext::new(base, 60);
    let r = run_with(&idx, "select mtime as m", &c);
    let cell = r.rows().next().and_then(|row| row.cells.first().cloned());
    assert_eq!(cell, Some(Cell::DateTime(base + 3_600_000)));
}

#[test]
fn nested_default_calls() {
    assert_eq!(value("default(missing, default(missing, 7))"), n(7.0));
}

#[test]
fn unknown_function_never_reaches_eval() {
    // Unknown functions are rejected by the parser; evaluation only sees known ones.
    assert!(jiyue_query::parse("where nope() = 1").is_err());
}

#[test]
fn values_on_a_missing_property_are_false_not_errors() {
    let idx = vault();
    let r = run(&idx, "where priority > 0 select name");
    assert_eq!(r.total, 3);
}
