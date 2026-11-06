//! Filtering and ranking of the menu rows. Pure logic.
//!
//! A row matches a query when one of its keys matches: the Chinese label, the full pinyin, the
//! pinyin initials or an English alias. Spaces in the query and in the keys are ignored, and case
//! does not matter. The score of a key is the best of: equal (100), prefix (80), substring (60),
//! subsequence (30). A row's score is its best key. Rows are ordered by score, then menu order.

use std::cmp::Reverse;

use super::{ENTRIES, Entry};

/// Lowercase, without whitespace. The form every comparison uses.
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The first letter of each pinyin syllable (or English word): `biao ti 1` gives `bt1`.
pub fn initials(py: &str) -> String {
    py.split_whitespace()
        .filter_map(|t| t.chars().next())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Score of one key against a normalized query. 0 means no match.
fn key_score(key: &str, q: &str) -> u32 {
    if key.is_empty() || q.is_empty() {
        0
    } else if key == q {
        100
    } else if key.starts_with(q) {
        80
    } else if key.contains(q) {
        60
    } else if is_subsequence(q, key) {
        30
    } else {
        0
    }
}

/// True when every character of `q` appears in `key` in the same order.
fn is_subsequence(q: &str, key: &str) -> bool {
    let mut it = key.chars();
    q.chars().all(|c| it.any(|k| k == c))
}

/// The best score of a row for a normalized query.
pub fn score(entry: &Entry, q: &str) -> u32 {
    let mut keys: Vec<String> = Vec::with_capacity(4 + entry.aliases.len());
    keys.push(normalize(entry.zh));
    keys.push(normalize(entry.py));
    keys.push(initials(entry.py));
    keys.extend(entry.aliases.iter().map(|a| normalize(a)));
    keys.iter().map(|k| key_score(k, q)).max().unwrap_or(0)
}

/// Indexes into `ENTRIES` of the rows that match `query`, best first. An empty query lists every
/// row in menu order. The frontmatter row is left out unless `frontmatter_ok`.
pub fn matches(query: &str, frontmatter_ok: bool) -> Vec<usize> {
    let q = normalize(query);
    let mut scored: Vec<(u32, usize)> = Vec::new();
    for (i, entry) in ENTRIES.iter().enumerate() {
        if !frontmatter_ok && entry.kind == super::Kind::Frontmatter {
            continue;
        }
        if q.is_empty() {
            scored.push((0, i));
            continue;
        }
        let s = score(entry, &q);
        if s > 0 {
            scored.push((s, i));
        }
    }
    scored.sort_by_key(|&(s, i)| (Reverse(s), i));
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::super::{Kind, entry};
    use super::*;

    fn kinds(query: &str) -> Vec<Kind> {
        matches(query, true)
            .into_iter()
            .map(|i| ENTRIES[i].kind)
            .collect()
    }

    #[test]
    fn normalize_drops_spaces_and_lowercases() {
        assert_eq!(normalize(" Bi ao  T1 "), "biaot1");
        assert_eq!(normalize("标 题"), "标题");
    }

    #[test]
    fn initials_are_the_first_letter_of_each_syllable() {
        assert_eq!(initials("biao ti 1"), "bt1");
        assert_eq!(initials("mermaid liu cheng tu"), "mlct");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn empty_query_lists_everything_in_menu_order_without_frontmatter() {
        let all = matches("", false);
        assert_eq!(all.len(), ENTRIES.len() - 1);
        assert_eq!(ENTRIES[all[0]].kind, Kind::Heading1);
        assert!(!all.iter().any(|&i| ENTRIES[i].kind == Kind::Frontmatter));
        assert_eq!(matches("", true).len(), ENTRIES.len());
    }

    #[test]
    fn pinyin_initials_find_the_heading_for_bt() {
        let top = kinds("bt");
        assert_eq!(&top[..3], &[Kind::Heading1, Kind::Heading2, Kind::Heading3]);
    }

    #[test]
    fn full_pinyin_and_its_prefix_find_the_heading() {
        assert_eq!(kinds("biaoti")[0], Kind::Heading1);
        assert_eq!(kinds("biaoti 2")[0], Kind::Heading2);
    }

    #[test]
    fn english_alias_finds_the_heading() {
        let top = kinds("heading");
        assert_eq!(&top[..3], &[Kind::Heading1, Kind::Heading2, Kind::Heading3]);
        assert_eq!(kinds("h2")[0], Kind::Heading2, "exact alias ranks first");
    }

    #[test]
    fn chinese_label_matches_by_substring() {
        let top = kinds("标题");
        assert_eq!(&top[..3], &[Kind::Heading1, Kind::Heading2, Kind::Heading3]);
        assert_eq!(kinds("表格")[0], Kind::Table);
        assert_eq!(kinds("列表").len(), 3, "无序列表, 有序列表 and 任务列表");
    }

    #[test]
    fn table_is_found_by_its_pinyin_and_alias() {
        assert_eq!(kinds("bg")[0], Kind::Table);
        assert_eq!(kinds("biaoge")[0], Kind::Table);
        assert_eq!(kinds("table")[0], Kind::Table);
    }

    #[test]
    fn subsequence_matches_with_the_lowest_score() {
        // wxlb is the initials of 无序列表; `wl` is a subsequence of it and of nothing ranked higher.
        let hits = kinds("wl");
        assert!(hits.contains(&Kind::Bullet));
        assert!(score(entry(Kind::Bullet), "wl") < 80);
    }

    #[test]
    fn exact_beats_prefix_beats_substring() {
        // "h1" equals an alias (100); "1" is only a substring of several keys (60).
        let exact = score(entry(Kind::Heading1), "h1");
        let sub = score(entry(Kind::Heading1), "1");
        assert_eq!(exact, 100);
        assert!(sub <= 60 && sub > 0);
    }

    #[test]
    fn no_match_gives_an_empty_list() {
        assert!(matches("zzzq", true).is_empty());
        assert!(matches("标题x", true).is_empty());
    }

    #[test]
    fn frontmatter_row_needs_the_document_start() {
        assert!(
            !matches("shuxing", false)
                .iter()
                .any(|&i| ENTRIES[i].kind == Kind::Frontmatter)
        );
        assert_eq!(
            matches("shuxing", true).first().map(|&i| ENTRIES[i].kind),
            Some(Kind::Frontmatter)
        );
    }

    #[test]
    fn ties_keep_menu_order() {
        // 注释块, 提示块, 警告块, 危险块 all match "块" the same way; they keep their order.
        let hits: Vec<Kind> = matches("块", true)
            .into_iter()
            .map(|i| ENTRIES[i].kind)
            .collect();
        let pos = |k: Kind| hits.iter().position(|&x| x == k).unwrap();
        assert!(pos(Kind::CalloutNote) < pos(Kind::CalloutTip));
        assert!(pos(Kind::CalloutTip) < pos(Kind::CalloutWarning));
        assert!(pos(Kind::CalloutWarning) < pos(Kind::CalloutDanger));
    }
}
