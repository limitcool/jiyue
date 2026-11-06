//! Incremental index tests: upsert, remove, rename, counts, link resolution and backlinks.

mod common;

use common::{FIXTURE, vault};
use jiyue_query::{Cell, PropertyIndex, normalize_path, stem_of};

fn one(text: &str) -> PropertyIndex {
    let mut idx = PropertyIndex::new();
    idx.upsert("n.md", text, 10, 5);
    idx
}

#[test]
fn fixture_indexes_every_note() {
    let idx = vault();
    assert_eq!(idx.len(), FIXTURE.len());
    assert!(!idx.is_empty());
}

#[test]
fn record_fields_are_filled_from_text_and_path() {
    let idx = vault();
    let r = idx.get("项目/rust-async.md");
    let Some(r) = r else { panic!("missing record") };
    assert_eq!(r.name, "rust-async");
    assert_eq!(r.folder, "项目");
    assert_eq!(r.tags, vec!["rust", "async", "rust/tips"]);
    assert_eq!(r.outgoing_links, vec!["Tokio"]);
    assert_eq!(r.props.get("priority"), Some(&Cell::Number(2.0)));
    assert_eq!(
        r.props.get("due"),
        Some(&Cell::Date(
            jiyue_query::date::parse_date("2026-11-08").unwrap_or(0)
        ))
    );
    assert!(r.word_count >= 4);
    assert!(r.size > 0);
}

#[test]
fn upsert_reports_whether_anything_changed() {
    let mut idx = PropertyIndex::new();
    assert!(idx.upsert("a.md", "one", 1, 1));
    assert!(
        !idx.upsert("a.md", "one", 1, 1),
        "same text and times is a no-op"
    );
    assert!(idx.upsert("a.md", "two", 1, 1), "new text");
    assert!(idx.upsert("a.md", "two", 2, 1), "new mtime");
    assert!(idx.upsert("a.md", "two", 2, 9), "new ctime");
    assert_eq!(idx.len(), 1);
}

#[test]
fn upsert_replaces_properties_and_tags() {
    let mut idx = one("---\nstatus: a\n---\n#old\n");
    assert!(idx.property_counts().contains_key("status"));
    assert!(idx.tag_counts().contains_key("old"));
    idx.upsert("n.md", "plain text", 11, 5);
    assert!(idx.property_counts().is_empty());
    assert!(idx.tag_counts().is_empty());
}

#[test]
fn counts_track_each_upsert_exactly() {
    let mut idx = PropertyIndex::new();
    idx.upsert("a.md", "---\nk: 1\n---\n#t\n", 1, 1);
    idx.upsert("b.md", "---\nk: 2\n---\n#t\n", 1, 1);
    assert_eq!(idx.property_counts().get("k"), Some(&2));
    assert_eq!(idx.tag_counts().get("t"), Some(&2));
    idx.upsert("a.md", "no props", 2, 1);
    assert_eq!(idx.property_counts().get("k"), Some(&1));
    assert_eq!(idx.tag_counts().get("t"), Some(&1));
}

#[test]
fn remove_drops_record_and_counts() {
    let mut idx = vault();
    assert!(idx.remove("inbox.md"));
    assert!(!idx.remove("inbox.md"));
    assert_eq!(idx.len(), FIXTURE.len() - 1);
    assert!(idx.get("inbox.md").is_none());
    assert_eq!(idx.resolve_link("inbox"), None);
}

#[test]
fn removing_everything_empties_all_counts() {
    let mut idx = vault();
    let paths: Vec<String> = idx.records().map(|r| r.path.clone()).collect();
    for p in &paths {
        assert!(idx.remove(p));
    }
    assert!(idx.is_empty());
    assert!(idx.property_counts().is_empty());
    assert!(idx.tag_counts().is_empty());
    assert_eq!(idx.folders().count(), 0);
    assert_eq!(idx.resolve_link("gpui"), None);
}

#[test]
fn rename_moves_the_record_and_updates_name_and_folder() {
    let mut idx = vault();
    assert!(idx.rename("项目/gpui.md", "归档/gpui-old.md"));
    assert!(idx.get("项目/gpui.md").is_none());
    let r = idx.get("归档/gpui-old.md");
    let Some(r) = r else {
        panic!("renamed record missing")
    };
    assert_eq!(r.name, "gpui-old");
    assert_eq!(r.folder, "归档");
    assert_eq!(
        idx.resolve_link("gpui-old"),
        Some("归档/gpui-old.md".to_string())
    );
    assert_eq!(idx.resolve_link("gpui"), None);
    assert!(idx.folders().any(|f| f == "归档"));
}

#[test]
fn rename_keeps_property_counts_unchanged() {
    let mut idx = vault();
    let before = idx.property_counts().clone();
    idx.rename("inbox.md", "other.md");
    assert_eq!(idx.property_counts(), &before);
}

#[test]
fn rename_missing_source_is_false() {
    let mut idx = vault();
    assert!(!idx.rename("nope.md", "x.md"));
    assert_eq!(idx.len(), FIXTURE.len());
}

#[test]
fn rename_onto_existing_note_replaces_it() {
    let mut idx = vault();
    assert!(idx.rename("inbox.md", "archive/old.md"));
    assert_eq!(idx.len(), FIXTURE.len() - 1);
    let r = idx.get("archive/old.md");
    assert!(r.is_some_and(|r| r.props.contains_key("status") && r.outgoing_links.len() == 2));
}

#[test]
fn folders_include_every_ancestor() {
    let idx = index_with(&[("a/b/c/x.md", "x"), ("a/y.md", "y"), ("root.md", "z")]);
    let folders: Vec<&str> = idx.folders().collect();
    assert_eq!(folders, vec!["a", "a/b", "a/b/c"]);
}

fn index_with(notes: &[(&str, &str)]) -> PropertyIndex {
    common::index_from(notes)
}

#[test]
fn resolve_link_prefers_exact_paths() {
    let idx = index_with(&[("x/Note.md", "a"), ("y/Note.md", "b")]);
    assert_eq!(idx.resolve_link("y/Note"), Some("y/Note.md".into()));
    assert_eq!(idx.resolve_link("y/Note.md"), Some("y/Note.md".into()));
    assert_eq!(
        idx.resolve_link("note"),
        Some("x/Note.md".into()),
        "first by path"
    );
}

#[test]
fn resolve_link_is_case_insensitive_and_strips_extension() {
    let idx = index_with(&[("Projects/Rust Notes.md", "a")]);
    assert_eq!(
        idx.resolve_link("rust notes"),
        Some("Projects/Rust Notes.md".into())
    );
    assert_eq!(
        idx.resolve_link("RUST NOTES.MD"),
        Some("Projects/Rust Notes.md".into())
    );
    assert_eq!(idx.resolve_link("   "), None);
    assert_eq!(idx.resolve_link("missing"), None);
}

#[test]
fn backlink_counts_use_the_supplied_resolver() {
    let idx = index_with(&[
        ("a.md", "[[alias]]"),
        ("b.md", "[[alias]] [[a]]"),
        ("c.md", "[[b]]"),
    ]);
    let resolve = |t: &str| match t {
        "alias" => Some("a.md".to_string()),
        "a" => Some("a.md".to_string()),
        "b" => Some("b.md".to_string()),
        _ => None,
    };
    let counts = idx.backlink_counts(&resolve);
    assert_eq!(
        counts.get("a.md"),
        Some(&1),
        "b counts once even with two links"
    );
    assert_eq!(counts.get("b.md"), Some(&1));
    assert!(!counts.contains_key("c.md"));
}

#[test]
fn self_links_do_not_count_as_backlinks() {
    let idx = index_with(&[("a.md", "[[a]]")]);
    assert!(
        idx.backlink_counts(&|t: &str| idx.resolve_link(t))
            .is_empty()
    );
}

#[test]
fn backlink_sources_are_sorted_and_unique() {
    let idx = index_with(&[("z.md", "[[t]]"), ("m.md", "[[t]]"), ("t.md", "x")]);
    let map = idx.backlink_sources(&|x: &str| idx.resolve_link(x));
    assert_eq!(
        map.get("t.md"),
        Some(&vec!["m.md".to_string(), "z.md".to_string()])
    );
}

#[test]
fn embeds_and_markdown_links_count_but_images_do_not() {
    let idx = index_with(&[(
        "n.md",
        "![[pic.png]] [text](other.md) ![alt](img.png) [web](https://x.y)",
    )]);
    let Some(r) = idx.get("n.md") else {
        panic!("missing")
    };
    assert!(r.outgoing_links.contains(&"pic.png".to_string()));
    assert!(r.outgoing_links.contains(&"other.md".to_string()));
    assert!(!r.outgoing_links.contains(&"img.png".to_string()));
    assert!(!r.outgoing_links.iter().any(|l| l.contains("x.y")));
}

#[test]
fn invalid_paths_are_rejected() {
    let mut idx = PropertyIndex::new();
    assert!(!idx.upsert("   ", "x", 1, 1));
    assert!(!idx.upsert("/", "x", 1, 1));
    assert_eq!(normalize_path("./a\\b.md"), "a/b.md");
    assert_eq!(normalize_path("/x.md"), "x.md");
}

#[test]
fn stem_of_strips_extension() {
    assert_eq!(stem_of("a/b.md"), "b");
    assert_eq!(stem_of("b.MD"), "b");
    assert_eq!(stem_of("pic.png"), "pic.png");
}

#[test]
fn records_iterate_in_path_order() {
    let idx = vault();
    let paths: Vec<&str> = idx.records().map(|r| r.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}

#[test]
fn approximate_memory_grows_with_content() {
    let small = index_with(&[("a.md", "x")]);
    let big = index_with(&[(
        "a.md",
        "---\nk: a long text value here\ntags: [a, b, c]\n---\nbody #t [[l]]",
    )]);
    assert!(big.approx_memory_bytes() > small.approx_memory_bytes());
    assert!(small.approx_memory_bytes() > 0);
}

#[test]
fn date_frontmatter_becomes_typed_cells() {
    let idx = index_with(&[(
        "d.md",
        "---\nday: 2026-03-01\nwhen: 2026-03-01 09:30\nbad: 2026-02-30\n---\n",
    )]);
    let Some(r) = idx.get("d.md") else {
        panic!("missing")
    };
    assert!(matches!(r.props.get("day"), Some(Cell::Date(_))));
    assert!(matches!(r.props.get("when"), Some(Cell::DateTime(_))));
    assert_eq!(r.props.get("bad"), Some(&Cell::Text("2026-02-30".into())));
}

#[test]
fn notes_without_frontmatter_have_no_props() {
    let idx = index_with(&[("plain.md", "just text")]);
    let Some(r) = idx.get("plain.md") else {
        panic!("missing")
    };
    assert!(r.props.is_empty());
    assert!(r.tags.is_empty());
}

#[test]
fn invalid_frontmatter_is_treated_as_absent() {
    let idx = index_with(&[("bad.md", "---\n: : :\n  - [\n---\nbody #tag\n")]);
    let Some(r) = idx.get("bad.md") else {
        panic!("missing")
    };
    assert!(r.props.is_empty());
    assert_eq!(r.tags, vec!["tag"]);
}

#[test]
fn word_count_counts_cjk_characters() {
    let idx = index_with(&[("w.md", "项目 rust 笔记")]);
    let Some(r) = idx.get("w.md") else {
        panic!("missing")
    };
    assert_eq!(
        r.word_count, 5,
        "two CJK chars, one latin word, two CJK chars"
    );
}

#[test]
fn content_hash_changes_with_text() {
    let a = index_with(&[("a.md", "one")]);
    let b = index_with(&[("a.md", "two")]);
    let ha = a.get("a.md").map(|r| r.content_hash);
    let hb = b.get("a.md").map(|r| r.content_hash);
    assert_ne!(ha, hb);
}

#[test]
fn large_index_round_trip() {
    let mut idx = PropertyIndex::new();
    for i in 0..500 {
        let text = format!("---\nn: {i}\ntags: [t{}]\n---\n", i % 7);
        idx.upsert(&format!("f{}/n{i}.md", i % 5), &text, i, i);
    }
    assert_eq!(idx.len(), 500);
    assert_eq!(idx.folders().count(), 5);
    for i in 0..500 {
        assert!(idx.remove(&format!("f{}/n{i}.md", i % 5)));
    }
    assert!(idx.is_empty());
    assert!(idx.tag_counts().is_empty());
}
