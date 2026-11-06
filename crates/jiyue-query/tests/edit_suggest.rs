//! Cell-edit round trips (comments and quoting preserved), property types, and autocomplete.

mod common;

use common::{FIXTURE, vault};
use jiyue_query::{
    Cell, CompletionKind, Kind, PropertyIndex, parse_input, plan_cell_edit, property_types, suggest,
};

fn reindex(text: &str) -> PropertyIndex {
    let mut idx = PropertyIndex::new();
    idx.upsert("n.md", text, 1, 1);
    idx
}

fn prop(idx: &PropertyIndex, key: &str) -> Option<Cell> {
    idx.get("n.md").and_then(|r| r.props.get(key).cloned())
}

const NOTE: &str = "---\n# keep this comment\ntitle: 'Quoted title'\nstatus: 进行中   # trailing\ntags: [a, b]\npriority: 2\n---\nBody line\n\nMore body\n";

#[test]
fn edit_rewrites_only_the_edited_line() {
    let out = plan_cell_edit(NOTE, "status", &Cell::Text("完成".into()));
    let before: Vec<&str> = NOTE.lines().collect();
    let after: Vec<&str> = out.lines().collect();
    assert_eq!(before.len(), after.len(), "no line added or removed");
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        if b.starts_with("status:") {
            assert!(a.starts_with("status:"), "line {i}: {a}");
            assert!(a.contains("完成"));
        } else {
            assert_eq!(a, b, "line {i} must be untouched");
        }
    }
    assert!(out.ends_with("Body line\n\nMore body\n"), "body untouched");
}

#[test]
fn edit_keeps_quoting_style_of_other_lines() {
    let out = plan_cell_edit(NOTE, "priority", &Cell::Number(5.0));
    assert!(out.contains("title: 'Quoted title'\n"));
    assert!(out.contains("# keep this comment\n"));
    assert!(out.contains("priority: 5"));
}

#[test]
fn edit_preserves_crlf_line_endings() {
    let text = "---\r\nstatus: a\r\nkeep: 'x'\r\n---\r\nbody\r\n";
    let out = plan_cell_edit(text, "status", &Cell::Text("b".into()));
    assert!(out.contains("\r\n"));
    assert!(out.contains("keep: 'x'\r\n"));
    assert!(out.ends_with("body\r\n"));
    assert_eq!(prop(&reindex(&out), "status"), Some(Cell::Text("b".into())));
}

#[test]
fn edit_adds_a_missing_property_before_the_closing_line() {
    let out = plan_cell_edit(
        NOTE,
        "due",
        &Cell::Date(jiyue_query::date::parse_date("2026-05-01").unwrap_or(0)),
    );
    assert!(out.contains("due: 2026-05-01\n---\n"));
    assert!(out.contains("title: 'Quoted title'"));
}

#[test]
fn edit_creates_frontmatter_when_absent() {
    let out = plan_cell_edit("just body\n", "status", &Cell::Text("new".into()));
    assert!(out.starts_with("---\n"));
    assert!(out.ends_with("just body\n"));
    assert_eq!(
        prop(&reindex(&out), "status"),
        Some(Cell::Text("new".into()))
    );
}

#[test]
fn invalid_frontmatter_is_returned_unchanged() {
    let text = "---\n: : :\n  - [\n---\nbody\n";
    assert_eq!(
        plan_cell_edit(text, "status", &Cell::Text("x".into())),
        text
    );
}

fn round_trip(cell: Cell) -> Cell {
    let out = plan_cell_edit("---\nkey: old\n---\nbody\n", "key", &cell);
    prop(&reindex(&out), "key").unwrap_or(Cell::Text("<missing>".into()))
}

#[test]
fn text_values_round_trip_exactly() {
    let samples = [
        "plain",
        "进行中",
        "a: b",
        "#tag",
        "yes",
        "no",
        "null",
        "123",
        "2026-03-01",
        "2026-03-01T10:00",
        "[x]",
        "{y}",
        "  padded  ",
        "quote \" inside",
        "it's",
        "back\\slash",
        "- dash",
        "ends with colon:",
        "tab\there",
        "emoji 🎉",
    ];
    for s in samples {
        assert_eq!(
            round_trip(Cell::Text(s.to_string())),
            Cell::Text(s.to_string()),
            "sample {s:?}"
        );
    }
}

#[test]
fn typed_values_round_trip() {
    assert_eq!(round_trip(Cell::Number(3.5)), Cell::Number(3.5));
    assert_eq!(round_trip(Cell::Number(-2.0)), Cell::Number(-2.0));
    assert_eq!(round_trip(Cell::Bool(true)), Cell::Bool(true));
    assert_eq!(round_trip(Cell::Bool(false)), Cell::Bool(false));
    let d = Cell::Date(jiyue_query::date::parse_date("2026-02-28").unwrap_or(0));
    assert_eq!(round_trip(d.clone()), d);
    let dt = Cell::DateTime(jiyue_query::date::parse_datetime("2026-02-28 13:45").unwrap_or(0));
    assert_eq!(round_trip(dt.clone()), dt);
    assert_eq!(round_trip(Cell::Null), Cell::Null);
}

#[test]
fn list_values_round_trip() {
    let list = Cell::List(vec![
        Cell::Text("a".into()),
        Cell::Text("b c".into()),
        Cell::Text("d, e".into()),
    ]);
    assert_eq!(round_trip(list.clone()), list);
}

#[test]
fn clearing_a_cell_writes_null() {
    let out = plan_cell_edit("---\nkey: old\n---\n", "key", &Cell::Null);
    assert!(out.contains("key: null"));
    assert_eq!(prop(&reindex(&out), "key"), Some(Cell::Null));
}

#[test]
fn edits_survive_a_second_edit_of_another_key() {
    let once = plan_cell_edit(NOTE, "status", &Cell::Text("完成".into()));
    let twice = plan_cell_edit(&once, "priority", &Cell::Number(9.0));
    let idx = reindex(&twice);
    assert_eq!(prop(&idx, "status"), Some(Cell::Text("完成".into())));
    assert_eq!(prop(&idx, "priority"), Some(Cell::Number(9.0)));
    assert!(twice.contains("# keep this comment"));
}

#[test]
fn parse_input_turns_typed_text_into_cells() {
    assert_eq!(parse_input("", Some(Kind::Number)), Cell::Null);
    assert_eq!(parse_input("12", Some(Kind::Number)), Cell::Number(12.0));
    assert_eq!(
        parse_input("12x", Some(Kind::Number)),
        Cell::Text("12x".into())
    );
    assert_eq!(parse_input("2026-07-01", None).kind(), Kind::Date);
    assert_eq!(parse_input("no", Some(Kind::Bool)), Cell::Bool(false));
    assert_eq!(parse_input("x", Some(Kind::Null)), Cell::Null);
}

#[test]
fn property_types_infer_the_dominant_kind() {
    let idx = vault();
    let types = property_types(&idx);
    assert_eq!(types.get("status"), Some(&Kind::Text));
    assert_eq!(types.get("due"), Some(&Kind::Date));
    assert_eq!(types.get("priority"), Some(&Kind::Number));
    assert_eq!(types.get("tags"), Some(&Kind::List));
}

#[test]
fn property_types_pick_the_majority() {
    let mut idx = PropertyIndex::new();
    idx.upsert("a.md", "---\nx: 1\n---\n", 1, 1);
    idx.upsert("b.md", "---\nx: 2\n---\n", 1, 1);
    idx.upsert("c.md", "---\nx: hello\n---\n", 1, 1);
    assert_eq!(property_types(&idx).get("x"), Some(&Kind::Number));
}

#[test]
fn property_types_skip_all_empty_properties() {
    let idx = common::index_from(&[("a.md", "---\nempty:\nreal: 1\n---\n")]);
    let types = property_types(&idx);
    assert!(!types.contains_key("empty"));
    assert_eq!(types.get("real"), Some(&Kind::Number));
}

fn labels(v: &[jiyue_query::Completion]) -> Vec<String> {
    v.iter().map(|c| c.label.clone()).collect()
}

#[test]
fn keywords_complete_at_the_start() {
    let idx = vault();
    let got = suggest(&idx, "fr", 2);
    assert!(labels(&got).contains(&"from".to_string()));
    let kw = got.iter().find(|c| c.label == "from");
    assert_eq!(kw.map(|c| c.kind), Some(CompletionKind::Keyword));
}

#[test]
fn group_by_completes_as_one_clause() {
    let idx = vault();
    let got = suggest(&idx, "gro", 3);
    assert!(labels(&got).contains(&"group by".to_string()));
}

#[test]
fn empty_query_offers_clauses_and_properties() {
    let idx = vault();
    let got = suggest(&idx, "", 0);
    let l = labels(&got);
    assert!(l.contains(&"where".to_string()));
    assert!(l.contains(&"status".to_string()));
    assert!(l.contains(&"name".to_string()));
    assert!(got.len() <= 50);
}

#[test]
fn property_names_complete_after_where() {
    let idx = vault();
    let got = suggest(&idx, "where sta", 9);
    let status = got.iter().find(|c| c.label == "status");
    assert_eq!(status.map(|c| c.insert.clone()), Some("status".to_string()));
    assert_eq!(status.map(|c| c.kind), Some(CompletionKind::Property));
    assert!(
        status
            .and_then(|c| c.detail.clone())
            .is_some_and(|d| d.contains("notes"))
    );
}

#[test]
fn builtins_are_offered_with_their_kind() {
    let idx = vault();
    let got = suggest(&idx, "where ba", 8);
    assert!(
        got.iter()
            .any(|c| c.label == "backlinks" && c.kind == CompletionKind::Builtin)
    );
}

#[test]
fn cjk_property_names_complete() {
    let idx = common::index_from(&[("a.md", "---\n状态: 进行中\n---\n")]);
    let got = suggest(&idx, "where 状", "where 状".len());
    assert_eq!(
        got.first().map(|c| c.insert.clone()),
        Some("状态".to_string())
    );
}

#[test]
fn names_with_spaces_are_inserted_in_backticks() {
    let idx = common::index_from(&[("a.md", "---\ndue date: 2026-01-01\n---\n")]);
    let got = suggest(&idx, "where due", 9);
    assert_eq!(
        got.first().map(|c| c.insert.clone()),
        Some("`due date`".to_string())
    );
}

#[test]
fn inside_backticks_the_raw_name_is_inserted() {
    let idx = common::index_from(&[("a.md", "---\ndue date: 2026-01-01\n---\n")]);
    let q = "where `due";
    let got = suggest(&idx, q, q.len());
    assert_eq!(
        got.first().map(|c| (c.label.clone(), c.insert.clone())),
        Some(("due date".into(), "due date".into()))
    );
}

#[test]
fn folder_names_complete_inside_folder_string() {
    let idx = vault();
    let q = "from folder(\"项";
    let got = suggest(&idx, q, q.len());
    assert_eq!(labels(&got), vec!["项目".to_string()]);
    assert_eq!(got.first().map(|c| c.kind), Some(CompletionKind::Folder));
}

#[test]
fn tag_names_complete_inside_tag_string() {
    let idx = vault();
    let q = "from tag(\"ru";
    let got = suggest(&idx, q, q.len());
    let l = labels(&got);
    assert!(l.contains(&"rust".to_string()));
    assert!(l.contains(&"rust/tips".to_string()));
    assert!(got.iter().all(|c| c.kind == CompletionKind::Tag));
}

#[test]
fn note_names_complete_inside_link_to() {
    let idx = vault();
    let q = "from link_to(\"gp";
    let got = suggest(&idx, q, q.len());
    assert_eq!(labels(&got), vec!["gpui".to_string()]);
}

#[test]
fn property_names_complete_inside_prop_function() {
    let idx = vault();
    let q = "where prop(\"sta";
    let got = suggest(&idx, q, q.len());
    assert_eq!(
        got.first().map(|c| c.insert.clone()),
        Some("status".to_string())
    );
}

#[test]
fn no_completions_inside_plain_strings() {
    let idx = vault();
    let q = "where status = \"fr";
    assert!(suggest(&idx, q, q.len()).is_empty());
}

#[test]
fn function_completions_insert_call_syntax() {
    let idx = vault();
    let got = suggest(&idx, "where tod", 9);
    let today = got.iter().find(|c| c.label == "today");
    assert_eq!(today.map(|c| c.insert.clone()), Some("today()".to_string()));
    assert_eq!(today.map(|c| c.kind), Some(CompletionKind::Function));
}

#[test]
fn cursor_positions_are_clamped_safely() {
    let idx = vault();
    let q = "where 状态";
    let _ = suggest(&idx, q, 999);
    let _ = suggest(&idx, q, 1);
    let _ = suggest(&idx, q, 0);
    let got = suggest(&idx, q, 4);
    assert!(got.len() <= 50);
}

#[test]
fn suggestions_are_capped() {
    let mut notes = Vec::new();
    let texts: Vec<String> = (0..80)
        .map(|i| format!("---\nprop{i:02}: 1\n---\n"))
        .collect();
    for (i, t) in texts.iter().enumerate() {
        notes.push((format!("n{i}.md"), t.clone()));
    }
    let refs: Vec<(&str, &str)> = notes
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let idx = common::index_from(&refs);
    let got = suggest(&idx, "where ", 6);
    assert_eq!(got.len(), 50);
}

#[test]
fn fixture_is_used_by_the_helper_module() {
    assert!(FIXTURE.len() >= 6);
}
