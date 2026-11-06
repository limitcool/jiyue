//! Execution tests: sources, filters, sorting, limits, grouping, aggregates and warnings.

mod common;

use common::index_from;
use common::{column, ctx, names, paths, run, run_with, vault};
use jiyue_query::{Cell, Kind, QueryContext, execute_with_resolver, parse};

fn n(v: f64) -> Cell {
    Cell::Number(v)
}

#[test]
fn unsorted_results_are_in_path_order() {
    let idx = vault();
    let r = run(&idx, "select name");
    assert_eq!(
        paths(&r),
        vec![
            "archive/old.md",
            "inbox.md",
            "日记/2026-11-04.md",
            "项目/gpui.md",
            "项目/rust-async.md",
            "项目/tokio.md",
        ]
    );
    assert_eq!(r.total, 6);
    assert_eq!(r.shown, 6);
}

#[test]
fn filter_by_property_equality() {
    let idx = vault();
    let r = run(&idx, "where status = \"进行中\" select name");
    assert_eq!(names(&idx, &r), vec!["inbox", "gpui", "rust-async"]);
}

#[test]
fn filter_with_relative_date_window() {
    let idx = vault();
    let r = run(&idx, "where due < today() + 7d select name");
    assert_eq!(names(&idx, &r), vec!["old", "inbox", "rust-async", "tokio"]);
}

#[test]
fn notes_without_the_property_never_match_comparisons() {
    let idx = vault();
    let r = run(&idx, "where due > date(\"2000-01-01\") select name");
    assert_eq!(r.total, 5, "the diary note has no due date");
}

#[test]
fn sort_puts_missing_values_last_and_keeps_path_order_for_ties() {
    let idx = vault();
    let r = run(&idx, "sort due asc select name");
    assert_eq!(
        names(&idx, &r),
        vec!["old", "tokio", "inbox", "rust-async", "gpui", "2026-11-04"]
    );
}

#[test]
fn sort_descending_still_puts_missing_last() {
    let idx = vault();
    let r = run(&idx, "sort due desc select name");
    assert_eq!(
        names(&idx, &r),
        vec!["gpui", "rust-async", "inbox", "tokio", "old", "2026-11-04"]
    );
}

#[test]
fn stable_sort_keeps_path_order_for_equal_keys() {
    let idx = vault();
    // status is the same for rust-async and gpui, so their path order decides.
    let r = run(&idx, "where status = \"进行中\" sort status select name");
    assert_eq!(names(&idx, &r), vec!["inbox", "gpui", "rust-async"]);
}

#[test]
fn multiple_sort_keys_apply_in_order() {
    let idx = vault();
    let r = run(
        &idx,
        "where status = \"进行中\" sort status asc, priority desc select name",
    );
    assert_eq!(names(&idx, &r), vec!["gpui", "rust-async", "inbox"]);
}

#[test]
fn limit_applies_after_sort_and_total_counts_all_matches() {
    let idx = vault();
    let r = run(&idx, "sort due asc limit 2 select name");
    assert_eq!(names(&idx, &r), vec!["old", "tokio"]);
    assert_eq!(r.total, 6);
    assert_eq!(r.shown, 2);
}

#[test]
fn limit_zero_shows_nothing() {
    let idx = vault();
    let r = run(&idx, "limit 0");
    assert_eq!(r.shown, 0);
    assert_eq!(r.total, 6);
}

#[test]
fn folder_source_is_recursive() {
    let idx = vault();
    let r = run(&idx, "from folder(\"项目\") select name");
    assert_eq!(names(&idx, &r), vec!["gpui", "rust-async", "tokio"]);
}

#[test]
fn folder_source_does_not_match_prefix_of_another_folder() {
    let idx = index_from(&[("项目/a.md", "x"), ("项目2/b.md", "y")]);
    let r = run(&idx, "from folder(\"项目\") select name");
    assert_eq!(names(&idx, &r), vec!["a"]);
}

#[test]
fn tag_source_matches_nested_tags_case_insensitively() {
    let idx = vault();
    let r = run(&idx, "from tag(\"RUST\") select name");
    assert_eq!(names(&idx, &r), vec!["rust-async", "tokio"]);
    let nested = run(&idx, "from tag(\"rust/tips\") select name");
    assert_eq!(names(&idx, &nested), vec!["rust-async"]);
}

#[test]
fn tags_inside_code_blocks_are_ignored() {
    let idx = vault();
    let r = run(&idx, "from tag(\"notatag\") select name");
    assert_eq!(r.total, 0);
}

#[test]
fn link_to_finds_linking_notes_and_resolves_names() {
    let idx = vault();
    let r = run(&idx, "from link_to(\"gpui\") select name");
    assert_eq!(names(&idx, &r), vec!["inbox"]);
    let r2 = run(&idx, "from link_to(\"rust-async\") select name");
    assert_eq!(names(&idx, &r2), vec!["gpui"]);
}

#[test]
fn link_to_unresolved_note_matches_nothing() {
    let idx = vault();
    let r = run(&idx, "from link_to(\"NoSuchNote\") select name");
    assert_eq!(r.total, 0);
}

#[test]
fn combined_sources_with_and_or_not() {
    let idx = vault();
    let r = run(
        &idx,
        "from folder(\"项目\") and not tag(\"rust\") select name",
    );
    assert_eq!(names(&idx, &r), vec!["gpui"]);
    let r2 = run(&idx, "from tag(\"daily\") or link_to(\"gpui\") select name");
    assert_eq!(names(&idx, &r2), vec!["inbox", "2026-11-04"]);
}

#[test]
fn backlinks_are_counted_from_resolved_links() {
    let idx = vault();
    let r = run(&idx, "where name = \"tokio\" select count(backlinks) as n");
    assert_eq!(column(&r, 0), vec!["1"]);
    let r2 = run(
        &idx,
        "where name = \"gpui\" select count(backlinks) as n, backlinks as who",
    );
    assert_eq!(column(&r2, 0), vec!["1"]);
    assert_eq!(column(&r2, 1), vec!["inbox"]);
}

#[test]
fn backlink_filter_uses_counts() {
    let idx = vault();
    let r = run(&idx, "where count(backlinks) > 0 select name");
    assert_eq!(names(&idx, &r), vec!["gpui", "rust-async", "tokio"]);
}

#[test]
fn outgoing_link_count() {
    let idx = vault();
    let r = run(&idx, "where count(links) > 0 select name");
    assert_eq!(names(&idx, &r), vec!["inbox", "gpui", "rust-async"]);
}

#[test]
fn custom_resolver_is_used_for_links() {
    let idx = index_from(&[("a.md", "[[zzz]]"), ("b.md", "x")]);
    let q = parse("from link_to(\"zzz\") select name").unwrap_or_default();
    let resolve = |t: &str| {
        if t == "zzz" {
            Some("b.md".to_string())
        } else {
            None
        }
    };
    let r = execute_with_resolver(&idx, &q, &ctx(), &resolve);
    assert_eq!(names(&idx, &r), vec!["a"]);
}

#[test]
fn group_by_orders_groups_by_first_appearance_with_null_last() {
    let idx = vault();
    let r = run(&idx, "group by status select name");
    let keys: Vec<String> = r.groups.iter().map(|g| g.key.display()).collect();
    assert_eq!(keys, vec!["完成", "进行中", ""]);
    assert_eq!(r.groups.last().map(|g| g.key.is_null()), Some(true));
}

#[test]
fn group_rows_keep_sort_order() {
    let idx = vault();
    let r = run(&idx, "group by status sort due desc select name");
    let first_group = r.groups.first();
    let got: Vec<String> = first_group
        .map(|g| g.rows.iter().map(|row| row.path.clone()).collect())
        .unwrap_or_default();
    assert_eq!(got, vec!["项目/gpui.md", "项目/rust-async.md", "inbox.md"]);
}

#[test]
fn group_aggregates_count_sum_avg_min_max() {
    let idx = vault();
    let r = run(
        &idx,
        "group by status select name, count() as n, sum(priority) as s, avg(priority) as a, min(due) as lo, max(due) as hi",
    );
    let inprog = r.groups.iter().find(|g| g.key.display() == "进行中");
    let Some(g) = inprog else {
        panic!("no 进行中 group");
    };
    assert_eq!(g.aggregates[1], n(3.0));
    assert_eq!(g.aggregates[2], n(5.0));
    assert_eq!(g.aggregates[3], n(2.5));
    assert_eq!(
        g.aggregates[4],
        Cell::Date(jiyue_query::date::parse_date("2026-11-05").unwrap_or(0))
    );
    assert_eq!(
        g.aggregates[5],
        Cell::Date(jiyue_query::date::parse_date("2026-12-01").unwrap_or(0))
    );
    assert_eq!(
        g.aggregates[0],
        Cell::Null,
        "plain columns have no aggregate"
    );
}

#[test]
fn aggregate_columns_are_flagged_and_rows_hold_null() {
    let idx = vault();
    let r = run(&idx, "group by status select name, count() as n");
    assert!(!r.columns[0].aggregate);
    assert!(r.columns[1].aggregate);
    assert_eq!(r.columns[1].kind, Kind::Number);
    let first = r.groups.first().and_then(|g| g.rows.first());
    assert_eq!(
        first.and_then(|row| row.cells.get(1).cloned()),
        Some(Cell::Null)
    );
}

#[test]
fn ungrouped_aggregate_is_a_footer_over_all_rows() {
    let idx = vault();
    let r = run(&idx, "select count() as total, sum(priority) as p");
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].aggregates[0], n(6.0));
    assert_eq!(r.groups[0].aggregates[1], n(6.0));
}

#[test]
fn empty_result_with_grouping_has_no_groups() {
    let idx = vault();
    let r = run(&idx, "where status = \"none\" group by status select name");
    assert!(r.groups.is_empty());
    assert_eq!(r.total, 0);
}

#[test]
fn empty_result_without_grouping_keeps_a_footer_group() {
    let idx = vault();
    let r = run(&idx, "where status = \"none\" select count() as n");
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].aggregates[0], n(0.0));
}

#[test]
fn columns_take_alias_or_source_text_and_dominant_kind() {
    let idx = vault();
    let r = run(&idx, "select name as 名字, due, priority * 2 as double");
    let labels: Vec<&str> = r.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(labels, vec!["名字", "due", "double"]);
    assert_eq!(r.columns[1].kind, Kind::Date);
    assert_eq!(r.columns[2].kind, Kind::Number);
}

#[test]
fn default_columns_when_no_select() {
    let idx = vault();
    let r = run(&idx, "where name = \"gpui\"");
    assert_eq!(
        r.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        vec!["name", "tags", "mtime"]
    );
}

#[test]
fn view_kind_is_carried_into_the_result() {
    let idx = vault();
    let r = run(&idx, "view cards");
    assert_eq!(r.view, jiyue_query::ViewKind::Cards);
}

#[test]
fn missing_property_produces_a_warning() {
    let idx = vault();
    let r = run(&idx, "where nothere = 1 select name, alsomissing");
    assert_eq!(r.warnings.len(), 2);
    assert!(r.warnings.iter().any(|w| w.contains("nothere")));
    assert!(r.warnings.iter().any(|w| w.contains("alsomissing")));
    assert_eq!(r.total, 0);
}

#[test]
fn builtin_names_do_not_warn() {
    let idx = vault();
    let r = run(
        &idx,
        "where name != \"x\" and words > 0 select name, folder, size",
    );
    assert!(r.warnings.is_empty());
}

#[test]
fn empty_index_gives_no_warnings_and_no_rows() {
    let idx = index_from(&[]);
    let r = run(&idx, "where anything = 1");
    assert!(r.warnings.is_empty());
    assert_eq!(r.total, 0);
}

#[test]
fn result_serializes_to_json() {
    let idx = vault();
    let r = run(&idx, "group by status select name, count() as n limit 2");
    let json = serde_json::to_string(&r).unwrap_or_default();
    assert!(json.contains("\"columns\""));
    assert!(json.contains("\"aggregate\":true"));
    assert!(json.contains("\"kind\":\"text\""));
}

#[test]
fn elapsed_time_is_reported() {
    let idx = vault();
    let r = run(&idx, "select name");
    assert!(r.elapsed_us < 60_000_000);
}

#[test]
fn query_context_drives_now_and_today() {
    let idx = vault();
    let c = QueryContext::new(
        jiyue_query::date::parse_datetime("2026-05-05 10:00").unwrap_or(0),
        0,
    );
    let r = run_with(&idx, "where due < today() select name", &c);
    assert_eq!(names(&idx, &r), vec!["old"]);
}

#[test]
fn group_by_expression_not_only_property() {
    let idx = vault();
    let r = run(&idx, "group by folder select name");
    let keys: Vec<String> = r.groups.iter().map(|g| g.key.display()).collect();
    assert_eq!(keys, vec!["archive", "", "日记", "项目"]);
}

#[test]
fn group_by_list_is_keyed_by_content() {
    let idx = vault();
    let r = run(&idx, "group by tags select name");
    assert!(r.groups.len() >= 3);
    assert!(
        r.groups
            .iter()
            .all(|g| g.rows.iter().all(|row| !row.path.is_empty()))
    );
}

#[test]
fn rows_expose_paths_for_every_group() {
    let idx = vault();
    let r = run(&idx, "group by status select name");
    assert_eq!(r.rows().count(), 6);
}
