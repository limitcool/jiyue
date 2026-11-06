//! Index-level tests: writes, queries, ranking, highlights, persistence and recovery.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{MAX_BODY_BYTES, NoteDoc, SCHEMA_VERSION, SearchIndex, version_marker};

const DAY_MS: i64 = 86_400_000;

/// Temporary directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "jiyue-search-test-{tag}-{}-{nanos}-{n}",
            process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn note(path: &str, title: &str, body: &str) -> NoteDoc {
    NoteDoc {
        path: path.to_string(),
        title: title.to_string(),
        body: body.to_string(),
        tags: Vec::new(),
        mtime_ms: 0,
    }
}

fn tagged(path: &str, title: &str, body: &str, tags: &[&str]) -> NoteDoc {
    NoteDoc {
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..note(path, title, body)
    }
}

fn ram_with(docs: Vec<NoteDoc>) -> SearchIndex {
    let mut index = SearchIndex::open_in_ram().expect("ram index");
    index.rebuild(docs.into_iter()).expect("rebuild");
    index
}

fn paths(index: &SearchIndex, query: &str) -> Vec<String> {
    index
        .search(query, 100)
        .into_iter()
        .map(|h| h.path)
        .collect()
}

fn highlighted_texts(hit: &crate::Hit) -> Vec<String> {
    hit.highlights
        .iter()
        .map(|r| hit.snippet[r.clone()].to_string())
        .collect()
}

fn assert_hit_ranges_valid(hit: &crate::Hit) {
    for r in &hit.highlights {
        assert!(hit.snippet.is_char_boundary(r.start));
        assert!(hit.snippet.is_char_boundary(r.end));
        assert!(r.start < r.end && r.end <= hit.snippet.len());
    }
    let mut prev_end = 0;
    for r in &hit.highlights {
        assert!(r.start >= prev_end, "ranges overlap or are unsorted");
        prev_end = r.end;
    }
}

#[test]
fn search_index_is_send_and_clone() {
    fn assert_send<T: Send>() {}
    assert_send::<SearchIndex>();
    let index = SearchIndex::open_in_ram().expect("ram");
    let _clone = index.clone();
}

#[test]
fn fresh_ram_index_is_empty_and_needs_no_rebuild() {
    let index = SearchIndex::open_in_ram().expect("ram");
    assert_eq!(index.count(), 0);
    assert!(index.search("anything", 10).is_empty());
    assert!(!index.needs_rebuild());
    assert!(index.dir().as_os_str().is_empty());
}

#[test]
fn upsert_is_invisible_until_commit() {
    let mut index = SearchIndex::open_in_ram().expect("ram");
    index
        .upsert(&note("a.md", "Rust", "hello world"))
        .expect("upsert");
    assert!(index.search("hello", 10).is_empty());
    assert_eq!(index.count(), 0);
    index.commit().expect("commit");
    assert_eq!(paths(&index, "hello"), vec!["a.md"]);
    assert_eq!(index.count(), 1);
}

#[test]
fn english_term_is_found_case_insensitively() {
    let index = ram_with(vec![note("a.md", "Title", "The Rust language")]);
    assert_eq!(paths(&index, "rust"), vec!["a.md"]);
    assert_eq!(paths(&index, "RUST"), vec!["a.md"]);
    assert!(paths(&index, "python").is_empty());
}

#[test]
fn hit_carries_path_title_and_score() {
    let index = ram_with(vec![note("dir/a.md", "My Title", "some body text")]);
    let hits = index.search("body", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "dir/a.md");
    assert_eq!(hits[0].title, "My Title");
    assert!(hits[0].score > 0.0);
}

#[test]
fn chinese_two_char_query_matches_adjacent_pair() {
    let index = ram_with(vec![note("a.md", "笔记", "我们在学习异步编程的基础知识")]);
    assert_eq!(paths(&index, "异步编程"), vec!["a.md"]);
    assert_eq!(paths(&index, "异步"), vec!["a.md"]);
    assert_eq!(paths(&index, "编程"), vec!["a.md"]);
}

#[test]
fn chinese_single_char_query_works() {
    let index = ram_with(vec![
        note("a.md", "笔记", "异步编程"),
        note("b.md", "x", "同步"),
    ]);
    let mut found = paths(&index, "步");
    found.sort();
    assert_eq!(found, vec!["a.md", "b.md"]);
    assert_eq!(paths(&index, "编"), vec!["a.md"]);
}

#[test]
fn chinese_query_does_not_match_unrelated_text() {
    let index = ram_with(vec![note("a.md", "x", "同步编程")]);
    assert!(paths(&index, "异步").is_empty());
    assert_eq!(paths(&index, "步编"), vec!["a.md"]);
}

#[test]
fn chinese_quoted_phrase_needs_adjacency() {
    let index = ram_with(vec![
        note("ok.md", "x", "我们学习异步编程"),
        note("reversed.md", "x", "我们学习编程和异步"),
    ]);
    assert_eq!(paths(&index, "\"异步编程\""), vec!["ok.md"]);
    assert!(paths(&index, "\"编程异步\"").is_empty());
}

#[test]
fn mixed_chinese_and_english_query() {
    let index = ram_with(vec![
        note("a.md", "x", "Rust 异步编程 async/await"),
        note("b.md", "x", "Go 异步编程"),
    ]);
    assert_eq!(paths(&index, "rust 异步"), vec!["a.md"]);
    assert_eq!(paths(&index, "rust异步"), vec!["a.md"]);
    assert_eq!(paths(&index, "async await"), vec!["a.md"]);
}

#[test]
fn fullwidth_query_matches_halfwidth_text() {
    let index = ram_with(vec![note("a.md", "x", "rust 2024")]);
    assert_eq!(paths(&index, "ＲＵＳＴ"), vec!["a.md"]);
    assert_eq!(paths(&index, "２０２４"), vec!["a.md"]);
}

#[test]
fn emoji_in_query_is_ignored() {
    let index = ram_with(vec![note("a.md", "x", "rust notes")]);
    assert_eq!(paths(&index, "🚀 rust"), vec!["a.md"]);
    assert!(paths(&index, "🚀🎉").is_empty());
}

#[test]
fn phrase_requires_adjacent_words() {
    let index = ram_with(vec![
        note("ok.md", "x", "the quick brown fox"),
        note("swapped.md", "x", "the brown quick fox"),
    ]);
    assert_eq!(paths(&index, "\"quick brown\""), vec!["ok.md"]);
    assert_eq!(paths(&index, "quick brown"), vec!["ok.md", "swapped.md"]);
}

#[test]
fn and_is_the_default_operator() {
    let index = ram_with(vec![
        note("both.md", "x", "alpha beta"),
        note("one.md", "x", "alpha only"),
    ]);
    assert_eq!(paths(&index, "alpha beta"), vec!["both.md"]);
}

#[test]
fn negated_term_excludes_documents() {
    let index = ram_with(vec![
        note("keep.md", "x", "rust notes"),
        note("drop.md", "x", "rust and go"),
    ]);
    assert_eq!(paths(&index, "rust -go"), vec!["keep.md"]);
}

#[test]
fn negated_phrase_excludes_documents() {
    let index = ram_with(vec![
        note("keep.md", "x", "rust tips"),
        note("drop.md", "x", "rust old api here"),
    ]);
    assert_eq!(paths(&index, r#"rust -"old api""#), vec!["keep.md"]);
}

#[test]
fn negated_chinese_term_excludes_documents() {
    let index = ram_with(vec![
        note("keep.md", "x", "异步编程入门"),
        note("drop.md", "x", "异步编程 与 草稿"),
    ]);
    assert_eq!(paths(&index, "异步编程 -草稿"), vec!["keep.md"]);
}

#[test]
fn or_operator_matches_either_term() {
    let index = ram_with(vec![
        note("a.md", "x", "alpha"),
        note("b.md", "x", "beta"),
        note("c.md", "x", "gamma"),
    ]);
    assert_eq!(paths(&index, "alpha OR beta"), vec!["a.md", "b.md"]);
    // (alpha OR beta) AND gamma: no note has gamma together with alpha or beta.
    assert!(paths(&index, "alpha OR beta gamma").is_empty());
}

#[test]
fn or_group_is_anded_with_following_term() {
    let index = ram_with(vec![
        note("ab.md", "x", "alpha common"),
        note("bg.md", "x", "beta gamma"),
        note("c.md", "x", "gamma common"),
    ]);
    // (alpha OR beta) AND gamma: only bg.md has gamma and beta.
    assert_eq!(paths(&index, "alpha OR beta gamma"), vec!["bg.md"]);
}

#[test]
fn tag_filter_matches_normalised_tags() {
    let index = ram_with(vec![
        tagged("a.md", "x", "body one", &["Rust"]),
        tagged("b.md", "x", "body two", &["go"]),
    ]);
    assert_eq!(paths(&index, "tag:rust"), vec!["a.md"]);
    assert_eq!(paths(&index, "tag:#RUST"), vec!["a.md"]);
    assert_eq!(paths(&index, "tag:go"), vec!["b.md"]);
    assert!(paths(&index, "tag:python").is_empty());
}

#[test]
fn multi_valued_tags_all_match() {
    let index = ram_with(vec![tagged("a.md", "x", "body", &["rust", "async"])]);
    assert_eq!(paths(&index, "tag:async"), vec!["a.md"]);
    assert_eq!(paths(&index, "tag:rust"), vec!["a.md"]);
}

#[test]
fn negated_tag_excludes() {
    let index = ram_with(vec![
        tagged("draft.md", "x", "rust", &["draft"]),
        tagged("final.md", "x", "rust", &["final"]),
    ]);
    assert_eq!(paths(&index, "rust -tag:draft"), vec!["final.md"]);
}

#[test]
fn path_filter_is_a_substring_match() {
    let index = ram_with(vec![
        note("想法/a.md", "x", "body"),
        note("工作/想法/b.md", "x", "body"),
        note("读书/c.md", "x", "body"),
    ]);
    let mut found = paths(&index, "path:想法/");
    found.sort();
    assert_eq!(found, vec!["工作/想法/b.md", "想法/a.md"]);
    assert_eq!(paths(&index, "path:读书"), vec!["读书/c.md"]);
}

#[test]
fn path_filter_treats_regex_characters_literally() {
    let index = ram_with(vec![
        note("notes (v2).md", "x", "body"),
        note("notesXv2.md", "x", "body"),
    ]);
    assert_eq!(paths(&index, "path:(v2)"), vec!["notes (v2).md"]);
    assert_eq!(
        paths(&index, "path:.md"),
        vec!["notes (v2).md", "notesXv2.md"]
    );
}

#[test]
fn file_filter_matches_file_name_words() {
    let index = ram_with(vec![
        note("读书/读书笔记.md", "x", "body"),
        note("工作/周报.md", "x", "body"),
    ]);
    assert_eq!(paths(&index, "file:读书"), vec!["读书/读书笔记.md"]);
    assert_eq!(paths(&index, "file:笔记"), vec!["读书/读书笔记.md"]);
    assert!(paths(&index, "file:工作").is_empty());
}

#[test]
fn file_filter_accepts_windows_separators() {
    let index = ram_with(vec![note("dir\\roadmap.md", "x", "body")]);
    assert_eq!(paths(&index, "file:roadmap"), vec!["dir\\roadmap.md"]);
}

#[test]
fn title_filter_only_looks_at_titles() {
    let index = ram_with(vec![
        note("t.md", "异步编程指南", "unrelated"),
        note("b.md", "other", "异步编程 mentioned in body"),
    ]);
    assert_eq!(paths(&index, "title:异步"), vec!["t.md"]);
    assert!(paths(&index, "title:mentioned").is_empty());
}

#[test]
fn title_match_outranks_body_match() {
    let index = ram_with(vec![
        note("body.md", "notes", "rust"),
        note("title.md", "rust guide", "intro text"),
    ]);
    let hits = index.search("rust", 10);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].path, "title.md");
    assert!(hits[0].score > hits[1].score);
}

#[test]
fn recency_boost_prefers_newer_notes_when_scores_tie() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0);
    let old = NoteDoc {
        mtime_ms: now - 900 * DAY_MS,
        ..note("old.md", "x", "shared words here")
    };
    let new = NoteDoc {
        mtime_ms: now,
        ..note("new.md", "x", "shared words here")
    };
    let index = ram_with(vec![old, new]);
    let hits = index.search("shared", 10);
    assert_eq!(hits[0].path, "new.md");
    assert_eq!(hits[1].path, "old.md");
}

#[test]
fn ties_are_ordered_by_path() {
    let index = ram_with(vec![
        note("c.md", "x", "same text"),
        note("a.md", "x", "same text"),
        note("b.md", "x", "same text"),
    ]);
    assert_eq!(paths(&index, "same"), vec!["a.md", "b.md", "c.md"]);
}

#[test]
fn snippet_highlights_english_term() {
    let index = ram_with(vec![note("a.md", "x", "Intro. Learn Rust with examples.")]);
    let hits = index.search("rust", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(highlighted_texts(&hits[0]), vec!["Rust"]);
    assert_hit_ranges_valid(&hits[0]);
}

#[test]
fn snippet_highlights_chinese_phrase_as_one_range() {
    let index = ram_with(vec![note(
        "a.md",
        "x",
        "前面的内容。我们学习异步编程。后面的内容。",
    )]);
    let hits = index.search("异步编程", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(highlighted_texts(&hits[0]), vec!["异步编程"]);
    assert_hit_ranges_valid(&hits[0]);
}

#[test]
fn snippet_highlights_single_cjk_char() {
    let index = ram_with(vec![note("a.md", "x", "异步编程")]);
    let hits = index.search("步", 10);
    assert_eq!(highlighted_texts(&hits[0]), vec!["步"]);
    assert_hit_ranges_valid(&hits[0]);
}

#[test]
fn snippet_highlights_every_matched_term() {
    let index = ram_with(vec![note("a.md", "x", "rust and go and rust")]);
    let hits = index.search("rust go", 10);
    assert_eq!(highlighted_texts(&hits[0]), vec!["rust", "go", "rust"]);
}

#[test]
fn snippet_is_centred_on_first_match() {
    let filler = "lorem ipsum dolor sit amet consectetur ".repeat(20);
    let body = format!("{filler}NEEDLE {filler}");
    let index = ram_with(vec![note("a.md", "x", &body)]);
    let hits = index.search("needle", 10);
    assert!(hits[0].snippet.contains("NEEDLE"));
    assert!(hits[0].snippet.chars().count() < 200);
    assert_hit_ranges_valid(&hits[0]);
}

#[test]
fn snippet_falls_back_to_body_start_when_only_title_matches() {
    let index = ram_with(vec![note(
        "a.md",
        "Special topic",
        "Opening sentence of the body.",
    )]);
    let hits = index.search("special", 10);
    assert!(hits[0].snippet.starts_with("Opening sentence"));
    assert!(hits[0].highlights.is_empty());
}

#[test]
fn snippet_ranges_are_on_char_boundaries_for_cjk_bodies() {
    let body = format!(
        "{}异步编程{}",
        "中文内容较长".repeat(30),
        "结尾文字".repeat(30)
    );
    let index = ram_with(vec![note("a.md", "x", &body)]);
    // The snippet centres on the first matching token ("内容" near the start).
    let hits = index.search("异步编程 内容", 10);
    assert_eq!(hits.len(), 1);
    assert_hit_ranges_valid(&hits[0]);
    assert!(highlighted_texts(&hits[0]).contains(&"内容".to_string()));
    // Searching only the phrase centres the window on it.
    let hits = index.search("异步编程", 10);
    assert_hit_ranges_valid(&hits[0]);
    assert_eq!(highlighted_texts(&hits[0]), vec!["异步编程"]);
}

#[test]
fn remove_drops_note_after_commit() {
    let mut index = ram_with(vec![
        note("a.md", "x", "keepme"),
        note("b.md", "x", "keepme"),
    ]);
    index.remove("a.md").expect("remove");
    assert_eq!(
        paths(&index, "keepme").len(),
        2,
        "not visible before commit"
    );
    index.commit().expect("commit");
    assert_eq!(paths(&index, "keepme"), vec!["b.md"]);
    assert_eq!(index.count(), 1);
}

#[test]
fn remove_unknown_path_is_harmless() {
    let mut index = ram_with(vec![note("a.md", "x", "text")]);
    index.remove("missing.md").expect("remove");
    index.commit().expect("commit");
    assert_eq!(index.count(), 1);
}

#[test]
fn upsert_replaces_existing_note() {
    let mut index = ram_with(vec![note("a.md", "x", "old wording")]);
    index
        .upsert(&note("a.md", "x", "new wording"))
        .expect("upsert");
    index.commit().expect("commit");
    assert!(paths(&index, "old").is_empty());
    assert_eq!(paths(&index, "new"), vec!["a.md"]);
    assert_eq!(index.count(), 1);
}

#[test]
fn duplicate_upserts_in_one_batch_keep_last_version() {
    let mut index = SearchIndex::open_in_ram().expect("ram");
    index.upsert(&note("a.md", "x", "first")).expect("upsert");
    index.upsert(&note("a.md", "x", "second")).expect("upsert");
    index.commit().expect("commit");
    assert_eq!(index.count(), 1);
    assert!(paths(&index, "first").is_empty());
    assert_eq!(paths(&index, "second"), vec!["a.md"]);
}

#[test]
fn upsert_after_remove_in_same_batch_restores_note() {
    let mut index = ram_with(vec![note("a.md", "x", "original")]);
    index.remove("a.md").expect("remove");
    index
        .upsert(&note("a.md", "x", "restored"))
        .expect("upsert");
    index.commit().expect("commit");
    assert_eq!(index.count(), 1);
    assert_eq!(paths(&index, "restored"), vec!["a.md"]);
}

#[test]
fn rebuild_replaces_everything() {
    let mut index = ram_with(vec![note("old.md", "x", "stale")]);
    index
        .rebuild(vec![note("new.md", "x", "fresh")].into_iter())
        .expect("rebuild");
    assert!(paths(&index, "stale").is_empty());
    assert_eq!(paths(&index, "fresh"), vec!["new.md"]);
    assert_eq!(index.count(), 1);
}

#[test]
fn rebuild_with_empty_iterator_clears_index() {
    let mut index = ram_with(vec![note("a.md", "x", "text")]);
    index.rebuild(std::iter::empty()).expect("rebuild");
    assert_eq!(index.count(), 0);
}

#[test]
fn unicode_paths_work_for_upsert_search_and_remove() {
    let path = "想法/测试 笔记 ✨.md";
    let mut index = SearchIndex::open_in_ram().expect("ram");
    index
        .upsert(&note(path, "测试", "内容很短"))
        .expect("upsert");
    index.commit().expect("commit");
    let hits = index.search("内容", 10);
    assert_eq!(hits[0].path, path);
    index.remove(path).expect("remove");
    index.commit().expect("commit");
    assert_eq!(index.count(), 0);
}

#[test]
fn empty_body_and_title_only_notes_are_searchable() {
    let index = ram_with(vec![note("a.md", "Only A Title", ""), note("b.md", "", "")]);
    assert_eq!(paths(&index, "title"), vec!["a.md"]);
    assert_eq!(index.count(), 2);
}

#[test]
fn oversized_body_is_truncated_for_indexing() {
    let filler = "filler ".repeat(MAX_BODY_BYTES / 7 * 2);
    let body = format!("alphaword {filler} omegaword");
    assert!(body.len() > MAX_BODY_BYTES * 2 - 100);
    let index = ram_with(vec![note("big.md", "x", &body)]);
    assert_eq!(paths(&index, "alphaword"), vec!["big.md"]);
    assert!(paths(&index, "omegaword").is_empty());
    let hit = &index.search("alphaword", 1)[0];
    assert!(hit.snippet.len() < 1000);
}

#[test]
fn oversized_multibyte_body_is_truncated_on_char_boundary() {
    let body = format!("起始{}", "中".repeat(MAX_BODY_BYTES));
    let index = ram_with(vec![note("big.md", "x", &body)]);
    assert_eq!(index.count(), 1);
    assert_eq!(paths(&index, "起始"), vec!["big.md"]);
}

#[test]
fn blank_and_negative_only_queries_return_nothing() {
    let index = ram_with(vec![note("a.md", "x", "anything at all")]);
    assert!(index.search("", 10).is_empty());
    assert!(index.search("   ", 10).is_empty());
    assert!(index.search("-anything", 10).is_empty());
    assert!(index.search("\"\"", 10).is_empty());
    assert!(index.search("OR", 10).is_empty());
}

#[test]
fn zero_limit_returns_nothing() {
    let index = ram_with(vec![note("a.md", "x", "anything")]);
    assert!(index.search("anything", 0).is_empty());
}

#[test]
fn limit_caps_the_number_of_hits() {
    let docs: Vec<NoteDoc> = (0..10)
        .map(|i| note(&format!("n{i:02}.md"), "x", "common word"))
        .collect();
    let index = ram_with(docs);
    assert_eq!(index.search("common", 3).len(), 3);
    assert_eq!(index.search("common", 100).len(), 10);
}

#[test]
fn unknown_operator_degrades_to_plain_terms() {
    let index = ram_with(vec![note("a.md", "x", "foo bar baz")]);
    assert_eq!(paths(&index, "foo:bar"), vec!["a.md"]);
    assert_eq!(paths(&index, "wat:???"), Vec::<String>::new());
}

#[test]
fn odd_syntax_never_errors() {
    let index = ram_with(vec![note("a.md", "x", "text here")]);
    for q in [
        "\"", "\"\"\"", "tag:", "path:", "-", "--x", "OR OR", "a:\"b", "**", "(((", "[a-z]",
    ] {
        assert!(index.try_search(q, 10).is_ok(), "query {q:?}");
    }
    assert_eq!(paths(&index, "text here"), vec!["a.md"]);
}

#[test]
fn count_reflects_committed_notes() {
    let mut index = SearchIndex::open_in_ram().expect("ram");
    assert_eq!(index.count(), 0);
    index.upsert(&note("a.md", "x", "a")).expect("upsert");
    index.upsert(&note("b.md", "x", "b")).expect("upsert");
    assert_eq!(index.count(), 0);
    index.commit().expect("commit");
    assert_eq!(index.count(), 2);
}

#[test]
fn persistence_across_reopen_after_rebuild() {
    let tmp = TempDir::new("persist");
    {
        let mut index = SearchIndex::open(tmp.path()).expect("open");
        assert!(index.needs_rebuild(), "fresh index needs a rebuild");
        index
            .rebuild(
                vec![tagged(
                    "想法/a.md",
                    "异步编程",
                    "Rust 异步编程 notes",
                    &["rust"],
                )]
                .into_iter(),
            )
            .expect("rebuild");
        index
            .upsert(&note("b.md", "later", "added later"))
            .expect("upsert");
        index.commit().expect("commit");
    }
    let index = SearchIndex::open(tmp.path()).expect("reopen");
    assert!(!index.needs_rebuild());
    assert_eq!(index.count(), 2);
    assert_eq!(paths(&index, "异步编程"), vec!["想法/a.md"]);
    assert_eq!(paths(&index, "tag:rust"), vec!["想法/a.md"]);
    assert_eq!(paths(&index, "added"), vec!["b.md"]);
    assert_eq!(paths(&index, "path:想法/"), vec!["想法/a.md"]);
}

#[test]
fn reopen_before_rebuild_still_needs_rebuild() {
    let tmp = TempDir::new("pending");
    {
        let _ = SearchIndex::open(tmp.path()).expect("open");
    }
    let index = SearchIndex::open(tmp.path()).expect("reopen");
    assert!(index.needs_rebuild());
    assert_eq!(index.count(), 0);
}

#[test]
fn version_mismatch_wipes_and_requests_rebuild() {
    let tmp = TempDir::new("version");
    {
        let mut index = SearchIndex::open(tmp.path()).expect("open");
        index
            .rebuild(vec![note("a.md", "x", "stored")].into_iter())
            .expect("rebuild");
    }
    fs::write(
        tmp.path().join("jiyue-search.version"),
        "jiyue-search schema 0",
    )
    .expect("write marker");
    let index = SearchIndex::open(tmp.path()).expect("reopen");
    assert!(index.needs_rebuild());
    assert_eq!(index.count(), 0);
    assert!(index.search("stored", 10).is_empty());
}

#[test]
fn missing_version_marker_requests_rebuild() {
    let tmp = TempDir::new("nomarker");
    {
        let mut index = SearchIndex::open(tmp.path()).expect("open");
        index
            .rebuild(vec![note("a.md", "x", "stored")].into_iter())
            .expect("rebuild");
    }
    fs::remove_file(tmp.path().join("jiyue-search.version")).expect("remove marker");
    let index = SearchIndex::open(tmp.path()).expect("reopen");
    assert!(index.needs_rebuild());
    assert_eq!(index.count(), 0);
}

#[test]
fn version_marker_matches_written_file_after_rebuild() {
    let tmp = TempDir::new("marker");
    let mut index = SearchIndex::open(tmp.path()).expect("open");
    index.rebuild(std::iter::empty()).expect("rebuild");
    let written = fs::read_to_string(tmp.path().join("jiyue-search.version")).expect("marker");
    assert_eq!(written.trim(), version_marker());
}

#[test]
fn corrupted_index_directory_is_recreated() {
    let tmp = TempDir::new("corrupt");
    {
        let mut index = SearchIndex::open(tmp.path()).expect("open");
        index
            .rebuild(vec![note("a.md", "x", "stored")].into_iter())
            .expect("rebuild");
    }
    fs::write(
        tmp.path().join("tantivy").join("meta.json"),
        b"{ not json at all",
    )
    .expect("corrupt");
    let mut index = SearchIndex::open(tmp.path()).expect("open corrupt index");
    assert!(index.needs_rebuild());
    assert_eq!(index.count(), 0);
    index
        .rebuild(vec![note("b.md", "x", "rebuilt")].into_iter())
        .expect("rebuild");
    assert_eq!(paths(&index, "rebuilt"), vec!["b.md"]);
    assert!(!index.needs_rebuild());
}

#[test]
fn index_directory_removed_entirely_is_recreated() {
    let tmp = TempDir::new("gone");
    {
        let mut index = SearchIndex::open(tmp.path()).expect("open");
        index
            .rebuild(vec![note("a.md", "x", "stored")].into_iter())
            .expect("rebuild");
    }
    fs::remove_dir_all(tmp.path().join("tantivy")).expect("remove index dir");
    let index = SearchIndex::open(tmp.path()).expect("open");
    assert!(index.needs_rebuild());
    assert_eq!(index.count(), 0);
}

#[test]
fn open_creates_missing_directories() {
    let tmp = TempDir::new("nested");
    let nested = tmp.path().join("a").join("b").join("search");
    let index = SearchIndex::open(&nested).expect("open nested");
    assert!(index.needs_rebuild());
    assert!(nested.join("tantivy").is_dir());
}

#[test]
fn search_with_hits_from_disk_index() {
    let tmp = TempDir::new("disk");
    let mut index = SearchIndex::open(tmp.path()).expect("open");
    index
        .rebuild(vec![note("disk.md", "Disk", "stored on disk 磁盘")].into_iter())
        .expect("rebuild");
    let hits = index.search("磁盘", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(highlighted_texts(&hits[0]), vec!["磁盘"]);
}

#[test]
fn clones_share_the_same_index() {
    let mut index = SearchIndex::open_in_ram().expect("ram");
    let mut other = index.clone();
    other.upsert(&note("a.md", "x", "shared")).expect("upsert");
    other.commit().expect("commit");
    assert_eq!(index.count(), 1);
    assert_eq!(paths(&index, "shared"), vec!["a.md"]);
    index.remove("a.md").expect("remove");
    index.commit().expect("commit");
    assert_eq!(other.count(), 0);
}

#[test]
fn debug_output_mentions_directory() {
    let index = SearchIndex::open_in_ram().expect("ram");
    let text = format!("{index:?}");
    assert!(text.contains("SearchIndex"));
}

#[test]
fn try_search_reports_no_error_for_normal_queries() {
    let index = ram_with(vec![note("a.md", "x", "text")]);
    assert_eq!(index.try_search("text", 5).expect("ok").len(), 1);
}

#[test]
fn negated_cjk_phrase_excludes_only_adjacent_occurrences() {
    let index = ram_with(vec![
        note("drop.md", "x", "rust 异步编程入门"),
        note("keep.md", "x", "rust 异步 与 编程"),
    ]);
    assert_eq!(paths(&index, "rust -异步编程"), vec!["keep.md"]);
}

#[test]
fn negated_phrase_does_not_match_non_adjacent_words() {
    let index = ram_with(vec![
        note("a.md", "x", "rust old and api"),
        note("b.md", "x", "rust old api"),
    ]);
    assert_eq!(paths(&index, r#"rust -"old api""#), vec!["a.md"]);
}

#[test]
fn negated_title_phrase_only_checks_titles() {
    let index = ram_with(vec![
        note("a.md", "old api", "rust"),
        note("b.md", "x", "old api rust"),
    ]);
    assert_eq!(paths(&index, r#"rust -title:"old api""#), vec!["b.md"]);
}

#[test]
fn negated_phrase_refills_results_beyond_first_window() {
    let mut docs = Vec::new();
    for i in 0..40 {
        docs.push(note(&format!("old{i:02}.md"), "x", "rust old api"));
    }
    for i in 0..10 {
        docs.push(note(&format!("new{i:02}.md"), "x", "rust fresh"));
    }
    let index = ram_with(docs);
    let hits = index.search("rust -\"old api\"", 5);
    assert_eq!(hits.len(), 5);
    assert!(hits.iter().all(|h| h.path.starts_with("new")), "{hits:?}");
    assert_eq!(index.search("rust -\"old api\"", 100).len(), 10);
}

#[test]
fn negated_phrase_with_no_survivors_returns_empty() {
    let index = ram_with(vec![note("a.md", "x", "rust old api")]);
    assert!(index.search("rust -\"old api\"", 10).is_empty());
}

#[test]
fn snake_case_identifier_is_found_by_its_parts_and_as_a_whole() {
    let index = ram_with(vec![note("a.md", "x", "pub fn render_visible() {}")]);
    assert_eq!(paths(&index, "render"), vec!["a.md"]);
    assert_eq!(paths(&index, "visible"), vec!["a.md"]);
    assert_eq!(paths(&index, "RENDER"), vec!["a.md"]);
    assert_eq!(paths(&index, "render_visible"), vec!["a.md"]);
    assert!(paths(&index, "render_hidden").is_empty());
}

#[test]
fn whole_identifier_query_skips_docs_with_only_the_parts() {
    let index = ram_with(vec![
        note("a.md", "x", "pub fn render_visible"),
        note("b.md", "x", "render and visible separately"),
    ]);
    assert_eq!(paths(&index, "render_visible"), vec!["a.md"]);
    let mut both = paths(&index, "render visible");
    both.sort();
    assert_eq!(both, vec!["a.md", "b.md"]);
}

#[test]
fn camel_case_identifier_parts_are_found() {
    let index = ram_with(vec![note("a.md", "x", "fn renderVisible() {}")]);
    assert_eq!(paths(&index, "render"), vec!["a.md"]);
    assert_eq!(paths(&index, "visible"), vec!["a.md"]);
    assert_eq!(paths(&index, "rendervisible"), vec!["a.md"]);
    assert_eq!(paths(&index, "renderVisible"), vec!["a.md"]);
}

#[test]
fn identifier_parts_in_file_names_are_found() {
    let index = ram_with(vec![note("notes/render_visible.md", "x", "unrelated")]);
    assert_eq!(
        paths(&index, "file:visible"),
        vec!["notes/render_visible.md"]
    );
    assert_eq!(paths(&index, "visible"), vec!["notes/render_visible.md"]);
}

#[test]
fn phrase_matches_identifier_parts_in_order() {
    let index = ram_with(vec![
        note("a.md", "x", "pub fn render_visible() now"),
        note("b.md", "x", "visible render"),
    ]);
    assert_eq!(paths(&index, "\"render visible\""), vec!["a.md"]);
    assert_eq!(paths(&index, "\"visible render\""), vec!["b.md"]);
    assert_eq!(paths(&index, "\"render_visible now\""), vec!["a.md"]);
    assert!(paths(&index, "\"render_visible visible\"").is_empty());
}

#[test]
fn identifier_parts_work_with_chinese_text_around_them() {
    let index = ram_with(vec![note("a.md", "x", "我们学习render_visible函数")]);
    assert_eq!(paths(&index, "render"), vec!["a.md"]);
    assert_eq!(paths(&index, "学习"), vec!["a.md"]);
    assert_eq!(paths(&index, "visible函数"), vec!["a.md"]);
    assert_eq!(paths(&index, "\"visible函数\""), vec!["a.md"]);
    assert_eq!(paths(&index, "\"render_visible函数\""), vec!["a.md"]);
    assert_eq!(paths(&index, "\"学习render\""), vec!["a.md"]);
    assert_eq!(paths(&index, "render -函数"), Vec::<String>::new());
}

#[test]
fn highlights_cover_the_matched_part_of_an_identifier() {
    let index = ram_with(vec![note("a.md", "x", "fn render_visible() {}")]);
    let hits = index.search("visible", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(highlighted_texts(&hits[0]), vec!["visible"]);
    assert_hit_ranges_valid(&hits[0]);

    let whole = index.search("render_visible", 10);
    assert_eq!(highlighted_texts(&whole[0]), vec!["render_visible"]);
    assert_hit_ranges_valid(&whole[0]);
}

#[test]
fn highlights_stay_valid_for_identifiers_next_to_cjk() {
    let body = "中文说明 renderVisible函数 与 render_visible 的区别";
    let index = ram_with(vec![note("a.md", "x", body)]);
    for query in ["render", "visible", "函数", "render visible"] {
        let hits = index.search(query, 10);
        assert_eq!(hits.len(), 1, "{query}");
        assert_hit_ranges_valid(&hits[0]);
        assert!(!highlighted_texts(&hits[0]).is_empty(), "{query}");
    }
}

#[test]
fn identifier_query_returns_only_docs_with_the_identifier() {
    let index = ram_with(vec![
        note("a.md", "x", "render_visible once"),
        note("b.md", "x", "render_visible render_visible twice"),
        note("c.md", "x", "render visible without underscore"),
    ]);
    let mut found = paths(&index, "render_visible");
    found.sort();
    assert_eq!(found, vec!["a.md", "b.md"]);
    assert!(
        index
            .search("render_visible", 10)
            .iter()
            .all(|h| h.score > 0.0)
    );
}

#[test]
fn previous_schema_version_forces_rebuild() {
    let tmp = TempDir::new("prevschema");
    {
        let mut index = SearchIndex::open(tmp.path()).expect("open");
        index
            .rebuild(vec![note("a.md", "x", "render_visible")].into_iter())
            .expect("rebuild");
    }
    fs::write(
        tmp.path().join("jiyue-search.version"),
        format!("jiyue-search schema {}", SCHEMA_VERSION - 1),
    )
    .expect("write marker");
    let index = SearchIndex::open(tmp.path()).expect("reopen");
    assert!(index.needs_rebuild());
    assert_eq!(index.count(), 0);
    assert!(index.search("visible", 10).is_empty());
}
