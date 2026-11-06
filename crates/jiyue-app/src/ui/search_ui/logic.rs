//! Pure logic of the search panel and the find bar: query building per scope, query terms,
//! match finding (Unicode aware), replace edits, highlight ranges, tags, recent searches and the
//! first hit position. No UI types here, so everything is unit tested.

use std::ops::Range;
use std::path::{Path, PathBuf};

use jiyue_md::frontmatter;

/// Most results the panel shows.
pub const RESULT_LIMIT: usize = 50;
/// Most recent searches kept in the `[search]` section of `.jiyue/state.toml`.
pub const HISTORY_MAX: usize = 10;
/// Pause after the last keystroke before the panel searches (ms).
pub const SEARCH_DEBOUNCE_MS: u64 = 120;
/// Pause after the last edit before an edited note is written to the index (ms).
pub const CHANGE_DEBOUNCE_MS: u64 = 1500;

/// Which notes the panel searches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    /// Notes in the folder of the active note.
    Folder,
    /// The typed words are tags.
    Tag,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::All, Scope::Folder, Scope::Tag];

    pub fn index(self) -> usize {
        match self {
            Scope::All => 0,
            Scope::Folder => 1,
            Scope::Tag => 2,
        }
    }

    pub fn from_index(i: usize) -> Scope {
        Self::ALL.get(i).copied().unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::All => "全部",
            Scope::Folder => "当前文件夹",
            Scope::Tag => "标签",
        }
    }
}

/// The query text sent to the index for the typed `input` in `scope`. `folder` is the folder key
/// of the active note (`项目/子目录`, empty for the vault root). An empty input gives an empty
/// query. A folder scope in the vault root searches everything.
pub fn build_query(scope: Scope, input: &str, folder: &str) -> String {
    let typed = input.trim();
    if typed.is_empty() {
        return String::new();
    }
    match scope {
        Scope::All => typed.to_string(),
        Scope::Folder => {
            let folder = folder.trim().trim_matches('/');
            if folder.is_empty() || folder.contains('"') {
                typed.to_string()
            } else {
                format!("path:\"{folder}/\" {typed}")
            }
        }
        Scope::Tag => typed
            .split_whitespace()
            .filter_map(tag_term)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// `rust` -> `tag:rust`, `-draft` -> `-tag:draft`, `#rust` -> `tag:rust`. Tokens that cannot be a
/// tag (empty, quoted, with a colon) are dropped.
fn tag_term(token: &str) -> Option<String> {
    let (negated, body) = match token.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    let tag = body.trim_start_matches('#');
    if tag.is_empty() || tag.contains(['"', ':']) {
        return None;
    }
    Some(format!("{}tag:{tag}", if negated { "-" } else { "" }))
}

/// Folder key of a vault-relative path: `想法/读书/a.md` -> `想法/读书`, `a.md` -> ``.
/// Accepts `/` and `\` separators; the result always uses `/`.
pub fn folder_of(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    match normalized.rfind('/') {
        Some(i) => normalized[..i].to_string(),
        None => String::new(),
    }
}

/// Plain words and phrases of a query that are worth highlighting: positive terms only, without
/// the operators (`-x`, `tag:`, `path:`, `file:`, `title:`, `OR`) and without the quotes.
pub fn query_terms(input: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut rest = input;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let negated = rest.starts_with('-');
        if negated {
            rest = &rest[1..];
        }
        if let Some(stripped) = rest.strip_prefix('"') {
            let end = stripped.find('"').unwrap_or(stripped.len());
            let phrase = stripped[..end].trim();
            if !negated && !phrase.is_empty() {
                terms.push(phrase.to_string());
            }
            rest = &stripped[(end + 1).min(stripped.len())..];
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let word = &rest[..end];
            rest = &rest[end..];
            if !negated && !word.is_empty() && !is_operator(word) {
                terms.push(word.to_string());
            }
        }
    }
    terms
}

fn is_operator(word: &str) -> bool {
    if word == "OR" {
        return true;
    }
    ["tag:", "path:", "file:", "title:"].iter().any(|p| {
        word.len() > p.len()
            && word.is_char_boundary(p.len())
            && word[..p.len()].eq_ignore_ascii_case(p)
    })
}

/// Options of a text match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MatchOpts {
    pub case_sensitive: bool,
    /// The match must not be next to a letter, digit or `_`. CJK characters count as letters.
    pub whole_word: bool,
}

/// Non-overlapping occurrences of `needle` in `text`, left to right, as byte ranges of `text`.
/// Case folding (Unicode lower case) applies only when `case_sensitive` is off. An empty needle
/// finds nothing.
pub fn find_matches(text: &str, needle: &str, opts: MatchOpts) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    // Every folded char with the byte range of the original char it came from.
    let mut folded: Vec<(char, usize, usize)> = Vec::with_capacity(text.len());
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        if opts.case_sensitive {
            folded.push((ch, start, end));
        } else {
            folded.extend(ch.to_lowercase().map(|lc| (lc, start, end)));
        }
    }
    let pattern: Vec<char> = if opts.case_sensitive {
        needle.chars().collect()
    } else {
        needle.chars().flat_map(|c| c.to_lowercase()).collect()
    };
    let m = pattern.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i + m <= folded.len() {
        let hit = folded[i..i + m]
            .iter()
            .map(|f| f.0)
            .eq(pattern.iter().copied());
        if hit {
            let start = folded[i].1;
            let end = folded[i + m - 1].2;
            if !opts.whole_word || is_word_boundary(text, start, end) {
                out.push(start..end);
                i += m;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let before = text.get(..start).and_then(|s| s.chars().next_back());
    let after = text.get(end..).and_then(|s| s.chars().next());
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

/// The one edit that replaces every match with `replacement`: `(start, end, new text)` covering
/// the first match up to the last one. Applied as one replacement it is one undo step. `matches`
/// must be ordered and must not overlap (what `find_matches` returns). `None` without matches.
pub fn replace_all_edit(
    text: &str,
    matches: &[Range<usize>],
    replacement: &str,
) -> Option<(usize, usize, String)> {
    let first = matches.first()?.start;
    let last = matches.last()?.end;
    let mut out = String::new();
    let mut pos = first;
    for m in matches {
        out.push_str(text.get(pos..m.start).unwrap_or(""));
        out.push_str(replacement);
        pos = m.end.max(pos);
    }
    Some((first, last, out))
}

/// Sorts, merges and clamps highlight ranges to the char boundaries of `text`. Empty ranges are
/// dropped.
pub fn normalize_ranges(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut sorted: Vec<Range<usize>> = ranges
        .iter()
        .filter_map(|r| {
            let start = floor_boundary(text, r.start);
            let end = ceil_boundary(text, r.end);
            (start < end).then_some(start..end)
        })
        .collect();
    sorted.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in sorted {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

fn floor_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// `(row, col)`, both zero based (col in chars), of the first occurrence of any term in `body`.
pub fn first_hit(body: &str, terms: &[String]) -> Option<(usize, usize)> {
    let opts = MatchOpts::default();
    let offset = terms
        .iter()
        .filter_map(|t| find_matches(body, t, opts).first().map(|r| r.start))
        .min()?;
    let before = &body[..offset];
    let row = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = body[line_start..offset].chars().count();
    Some((row, col))
}

/// Tags written inline as `#tag` (not headings `## x`), without the `#`, in order, no duplicates.
pub fn inline_tags(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        if !word.starts_with('#') || word.starts_with("##") {
            continue;
        }
        let tag = word
            .trim_start_matches('#')
            .trim_matches(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '/'));
        if tag.is_empty() || out.iter().any(|t| t == tag) {
            continue;
        }
        out.push(tag.to_string());
    }
    out
}

/// Frontmatter tags (`tags:` / `tag:`) followed by inline `#tags`, without duplicates.
pub fn note_tags(text: &str) -> Vec<String> {
    let mut out = frontmatter::tags(text);
    for tag in inline_tags(text) {
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Pushes `query` to the front of the recent searches (no duplicates, at most `HISTORY_MAX`).
/// Blank queries are ignored.
pub fn push_history(list: &mut Vec<String>, query: &str) {
    let q = query.trim();
    if q.is_empty() {
        return;
    }
    list.retain(|h| h != q);
    list.insert(0, q.to_string());
    list.truncate(HISTORY_MAX);
}

/// The vault's state file, which holds the search history in its `[search]` section.
pub fn history_path(vault_root: &Path) -> PathBuf {
    jiyue_core::config::vault_file(vault_root, jiyue_core::config::VAULT_STATE_FILE)
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct SearchSection {
    history: Vec<String>,
}

/// The index directory: `<vault>/.jiyue/index/search`.
pub fn index_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(".jiyue").join("index").join("search")
}

/// Reads the recent searches of a vault (empty when the file is missing or broken).
pub fn load_history(vault_root: &Path) -> Vec<String> {
    let section: SearchSection =
        jiyue_core::config::load_section(&history_path(vault_root), "search").value;
    let mut list = section.history;
    list.retain(|s| !s.trim().is_empty());
    list.truncate(HISTORY_MAX);
    list
}

/// Writes the recent searches of a vault, keeping the rest of the state file.
pub fn save_history(vault_root: &Path, list: &[String]) -> std::io::Result<()> {
    let section = SearchSection {
        history: list.to_vec(),
    };
    jiyue_core::config::save_section(
        &history_path(vault_root),
        "search",
        &section,
        jiyue_core::config::templates::VAULT_STATE,
        &[],
    )
    .map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn opts(case_sensitive: bool, whole_word: bool) -> MatchOpts {
        MatchOpts {
            case_sensitive,
            whole_word,
        }
    }

    #[test]
    fn all_scope_passes_the_text_through() {
        assert_eq!(build_query(Scope::All, "  沉积  ", "x"), "沉积");
        assert_eq!(build_query(Scope::All, "   ", "x"), "");
    }

    #[test]
    fn folder_scope_limits_to_the_active_folder() {
        assert_eq!(
            build_query(Scope::Folder, "沉积", "想法/读书"),
            "path:\"想法/读书/\" 沉积"
        );
        // In the vault root the scope is the whole vault.
        assert_eq!(build_query(Scope::Folder, "沉积", ""), "沉积");
        assert_eq!(build_query(Scope::Folder, "", "想法"), "");
    }

    #[test]
    fn tag_scope_turns_words_into_tag_terms() {
        assert_eq!(build_query(Scope::Tag, "rust", ""), "tag:rust");
        assert_eq!(
            build_query(Scope::Tag, "#rust -draft", ""),
            "tag:rust -tag:draft"
        );
        assert_eq!(build_query(Scope::Tag, "a:b \"x\" #", ""), "");
    }

    #[test]
    fn scope_indexes_round_trip() {
        for s in Scope::ALL {
            assert_eq!(Scope::from_index(s.index()), s);
        }
        assert_eq!(Scope::from_index(9), Scope::All);
    }

    #[test]
    fn folder_of_handles_both_separators() {
        assert_eq!(folder_of("想法/读书/a.md"), "想法/读书");
        assert_eq!(folder_of("想法\\a.md"), "想法");
        assert_eq!(folder_of("a.md"), "");
    }

    #[test]
    fn query_terms_skip_operators_and_negations() {
        assert_eq!(
            query_terms("沉积 \"地质 年代\" -排除 tag:rust path:想法 OR 异步"),
            vec!["沉积", "地质 年代", "异步"]
        );
        assert_eq!(query_terms("  "), Vec::<String>::new());
        assert_eq!(query_terms("\""), Vec::<String>::new());
        assert_eq!(query_terms("-"), Vec::<String>::new());
    }

    #[test]
    fn matches_are_case_folded_unless_case_sensitive() {
        assert_eq!(
            find_matches("Rust rust RUST", "rust", opts(false, false)).len(),
            3
        );
        assert_eq!(
            find_matches("Rust rust RUST", "rust", opts(true, false)),
            vec![5..9]
        );
    }

    #[test]
    fn cjk_matches_use_byte_ranges() {
        let text = "沉积岩与沉积作用";
        let m = find_matches(text, "沉积", opts(false, false));
        assert_eq!(m.len(), 2);
        assert_eq!(&text[m[0].clone()], "沉积");
        assert_eq!(&text[m[1].clone()], "沉积");
        assert_eq!(m[1].start, "沉积岩与".len());
    }

    #[test]
    fn matches_do_not_overlap() {
        let m = find_matches("aaaa", "aa", opts(false, false));
        assert_eq!(m, vec![0..2, 2..4]);
        assert!(find_matches("abc", "", opts(false, false)).is_empty());
    }

    #[test]
    fn whole_word_rejects_matches_inside_words() {
        let m = find_matches("cat catalog cat_x the cat", "cat", opts(false, true));
        assert_eq!(m, vec![0..3, 22..25]);
    }

    #[test]
    fn whole_word_treats_cjk_letters_as_word_characters() {
        // 沉积 inside 沉积物 is not a whole word; on its own it is.
        let m = find_matches("沉积物 沉积", "沉积", opts(false, true));
        assert_eq!(m, vec![10..16]);
    }

    #[test]
    fn replace_all_is_one_edit_with_correct_offsets() {
        let text = "banana bandana";
        let m = find_matches(text, "an", opts(true, false));
        let (start, end, new) = replace_all_edit(text, &m, "AN").expect("edit");
        let mut applied = String::new();
        applied.push_str(&text[..start]);
        applied.push_str(&new);
        applied.push_str(&text[end..]);
        assert_eq!(applied, "bANANa bANdANa");
        assert_eq!(replace_all_edit(text, &[], "x"), None);
    }

    #[test]
    fn replace_all_handles_empty_replacement_and_cjk() {
        let text = "沉积 沉积 岩";
        let m = find_matches(text, "沉积", opts(false, false));
        let (s, e, new) = replace_all_edit(text, &m, "").expect("edit");
        let mut applied = String::new();
        applied.push_str(&text[..s]);
        applied.push_str(&new);
        applied.push_str(&text[e..]);
        assert_eq!(applied, "  岩");
    }

    #[test]
    fn highlight_ranges_are_merged_and_aligned() {
        let text = "沉积岩";
        // 1..4 cuts the first char; it is widened to whole chars.
        assert_eq!(
            normalize_ranges(text, std::slice::from_ref(&(1..4))),
            vec![0..6]
        );
        assert_eq!(
            normalize_ranges("abcdef", &[4..6, 0..2, 1..3]),
            vec![0..3, 4..6]
        );
        assert_eq!(
            normalize_ranges("abc", std::slice::from_ref(&(2..2))),
            Vec::<Range<usize>>::new()
        );
        assert_eq!(
            normalize_ranges("abc", std::slice::from_ref(&(1..99))),
            vec![1..3]
        );
    }

    #[test]
    fn first_hit_gives_row_and_column() {
        let body = "# 标题\n第一行\n沉积作用很重要";
        assert_eq!(first_hit(body, &["沉积".to_string()]), Some((2, 0)));
        assert_eq!(
            first_hit(body, &["作用".to_string(), "标题".to_string()]),
            Some((0, 2))
        );
        assert_eq!(first_hit(body, &["不存在".to_string()]), None);
    }

    #[test]
    fn inline_and_frontmatter_tags_are_collected() {
        let text = "---\ntags: rust, notes\n---\n# 标题\n正文 #地质 和 #rust。\n## 小标题\n#";
        assert_eq!(note_tags(text), vec!["rust", "notes", "地质"]);
        assert_eq!(inline_tags("#a, #b. ##h #c/d"), vec!["a", "b", "c/d"]);
    }

    #[test]
    fn history_keeps_ten_unique_recent_queries() {
        let mut list = Vec::new();
        for i in 0..12 {
            push_history(&mut list, &format!("q{i}"));
        }
        assert_eq!(list.len(), HISTORY_MAX);
        assert_eq!(list[0], "q11");
        push_history(&mut list, " q5 ");
        assert_eq!(list[0], "q5");
        assert_eq!(list.iter().filter(|s| *s == "q5").count(), 1);
        push_history(&mut list, "   ");
        assert_eq!(list.len(), HISTORY_MAX);
    }

    #[test]
    fn history_round_trips_in_the_state_file_and_tolerates_garbage() {
        let root = std::env::temp_dir().join(format!("jiyue-search-hist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".jiyue")).expect("mkdir");
        let list = vec!["沉积".to_string(), "tag:rust".to_string()];
        save_history(&root, &list).expect("save");
        assert_eq!(load_history(&root), list);
        std::fs::write(
            history_path(&root),
            "version = 1
[search
history = ",
        )
        .expect("write");
        assert!(load_history(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn history_and_index_paths_live_in_jiyue_dir() {
        let root = Path::new("v");
        assert!(history_path(root).ends_with("state.toml"));
        assert!(history_path(root).starts_with(root.join(".jiyue")));
        assert!(index_dir(root).ends_with("search"));
    }

    #[test]
    fn history_file_saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("jiyue-hist-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(load_history(&dir).is_empty());
        save_history(&dir, &["沉积".to_string()]).expect("save");
        assert_eq!(load_history(&dir), vec!["沉积".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }
}
