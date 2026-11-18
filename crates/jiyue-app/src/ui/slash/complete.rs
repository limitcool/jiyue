//! Inline editor completions triggered by `[[`, `![[`, `[[Note#`, `[[#`, and `#tag` (pure logic).

use std::cmp::Reverse;
use std::collections::HashMap;

use super::picker;

/// What kind of inline completion is active at the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    WikiLink,
    Embed,
    Heading { embed: bool, note_target: String },
    Tag,
}

/// Active completion context detected around the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionCtx {
    pub kind: TriggerKind,
    pub anchor_start: usize,
    pub replace_end: usize,
    pub query: String,
}

/// One completion candidate shown in the inline popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub detail: String,
    pub icon: &'static str,
    pub insert: String,
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

fn strip_md_ext(path: &str) -> &str {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[bytes.len() - 3..].eq_ignore_ascii_case(b".md") {
        &path[..path.len() - 3]
    } else {
        path
    }
}

fn match_score(candidate: &str, query_lower: &str) -> u32 {
    if query_lower.is_empty() {
        return 1;
    }
    let cand = candidate.to_lowercase();
    if cand == query_lower {
        100
    } else if cand.starts_with(query_lower) {
        80
    } else if cand.contains(query_lower) {
        60
    } else if is_subsequence(query_lower, &cand) {
        30
    } else {
        0
    }
}

fn is_subsequence(q: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    q.chars().all(|c| it.any(|k| k == c))
}

/// Detects an active inline completion (`[[`, `![[`, `[[Note#`, `[[#`, or `#tag`) at `caret`.
pub fn detect(text: &str, caret: usize) -> Option<CompletionCtx> {
    if caret > text.len() || !text.is_char_boundary(caret) {
        return None;
    }
    let ls = super::trigger::line_start(text, caret);
    if super::trigger::in_block(text, ls) {
        return None;
    }
    let prefix = text.get(ls..caret)?;
    let trimmed = prefix.trim_start();
    if trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || trimmed.starts_with("$$")
        || (ls == 0 && prefix.trim_end() == "---")
    {
        return None;
    }
    if prefix.matches('`').count() % 2 == 1 || prefix.matches('$').count() % 2 == 1 {
        return None;
    }

    let line_tail = text
        .get(caret..)
        .unwrap_or("")
        .split('\n')
        .next()
        .unwrap_or("");

    // Branch A: `[[` or `![[` (must handle unclosed `[[` exclusively so `#` inside it won't fall
    // through into tag detection).
    if let Some(rel_open) = prefix.rfind("[[") {
        let after_open = prefix.get(rel_open + 2..)?;
        if !after_open.contains("]]") && !after_open.contains('|') {
            let is_embed = rel_open > 0 && prefix.as_bytes().get(rel_open - 1) == Some(&b'!');
            let anchor_start = ls + if is_embed { rel_open - 1 } else { rel_open };
            let replace_end = if line_tail.starts_with("]]") {
                caret + 2
            } else if line_tail.starts_with(']') {
                caret + 1
            } else {
                caret
            };
            if let Some((note_part, heading_part)) = after_open.split_once('#') {
                if heading_part.starts_with('^') {
                    return None;
                }
                return Some(CompletionCtx {
                    kind: TriggerKind::Heading {
                        embed: is_embed,
                        note_target: note_part.trim().to_string(),
                    },
                    anchor_start,
                    replace_end,
                    query: heading_part.to_string(),
                });
            }
            return Some(CompletionCtx {
                kind: if is_embed {
                    TriggerKind::Embed
                } else {
                    TriggerKind::WikiLink
                },
                anchor_start,
                replace_end,
                query: after_open.to_string(),
            });
        }
    }

    // Branch B: `#tag`
    let mut tag_start_rel = prefix.len();
    for (idx, ch) in prefix.char_indices().rev() {
        if is_tag_char(ch) {
            tag_start_rel = idx;
        } else {
            break;
        }
    }
    let tag_query = prefix.get(tag_start_rel..)?;
    if tag_query.is_empty() || !tag_query.chars().any(|c| !c.is_ascii_digit()) {
        return None;
    }
    if tag_start_rel == 0 || prefix.as_bytes().get(tag_start_rel - 1) != Some(&b'#') {
        return None;
    }
    let hash_rel = tag_start_rel - 1;
    let boundary_ok = hash_rel == 0
        || prefix
            .get(..hash_rel)
            .and_then(|s| s.chars().next_back())
            .is_some_and(char::is_whitespace);
    if !boundary_ok {
        return None;
    }
    let tail_ext: usize = line_tail
        .chars()
        .take_while(|&c| is_tag_char(c))
        .map(char::len_utf8)
        .sum();
    Some(CompletionCtx {
        kind: TriggerKind::Tag,
        anchor_start: ls + hash_rel,
        replace_end: caret + tail_ext,
        query: tag_query.to_string(),
    })
}

fn score_notes(
    notes: &[(String, String, Vec<String>)],
    embed: bool,
    query: &str,
) -> Vec<(u32, usize, Item)> {
    let q = query.trim().to_lowercase();
    let mut title_counts: HashMap<String, usize> = HashMap::new();
    for (title, _, _) in notes {
        *title_counts.entry(title.to_lowercase()).or_insert(0) += 1;
    }
    let mut scored = Vec::new();
    for (idx, (title, path, aliases)) in notes.iter().enumerate() {
        let unique = title_counts
            .get(&title.to_lowercase())
            .copied()
            .unwrap_or(0)
            <= 1;
        let inner = if unique {
            title.clone()
        } else {
            strip_md_ext(path).to_string()
        };
        let insert = if embed {
            format!("![[{inner}]]")
        } else {
            format!("[[{inner}]]")
        };

        let (score, matched_alias) = if q.is_empty() {
            (1, None)
        } else {
            let title_score = match_score(title, &q);
            let path_score = match_score(strip_md_ext(path), &q);
            let mut best_alias: Option<(u32, &str)> = None;
            for alias in aliases {
                let s = match_score(alias, &q);
                if s > 0 && best_alias.is_none_or(|(prev, _)| s > prev) {
                    best_alias = Some((s, alias.as_str()));
                }
            }
            let alias_score = best_alias.map_or(0, |(s, _)| s);
            let max_score = title_score.max(alias_score).max(path_score);
            if max_score == 0 {
                continue;
            }
            let shown_alias = if alias_score > title_score {
                best_alias.map(|(_, a)| a)
            } else {
                None
            };
            (max_score, shown_alias)
        };

        let detail = match matched_alias {
            Some(alias) => format!("别名: {alias} · {path}"),
            None => path.clone(),
        };
        scored.push((
            score,
            idx,
            Item {
                label: title.clone(),
                detail,
                icon: "file-text",
                insert,
            },
        ));
    }
    scored
}

/// Builds note completion candidates (`[[Title]]` or disambiguated path) sorted by relevance.
pub fn note_items(
    notes: &[(String, String, Vec<String>)],
    embed: bool,
    query: &str,
    limit: usize,
) -> Vec<Item> {
    let mut scored = score_notes(notes, embed, query);
    scored.sort_by_key(|(s, idx, _)| (Reverse(*s), *idx));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, item)| item)
        .collect()
}

/// Builds combined embed completion candidates (notes and image attachments).
pub fn embed_items(
    notes: &[(String, String, Vec<String>)],
    files: &[String],
    query: &str,
    limit: usize,
) -> Vec<Item> {
    let q = query.trim().to_lowercase();
    let mut combined: Vec<(u32, u8, usize, Item)> = score_notes(notes, true, query)
        .into_iter()
        .map(|(s, idx, item)| (s, 0u8, idx, item))
        .collect();

    for (idx, img) in picker::image_candidates(files).into_iter().enumerate() {
        let score = if q.is_empty() {
            1
        } else {
            let s = match_score(&img.label, &q).max(match_score(&img.detail, &q));
            if s == 0 {
                continue;
            }
            s
        };
        combined.push((
            score,
            1u8,
            idx,
            Item {
                label: img.label,
                detail: img.detail,
                icon: "photo",
                insert: img.link,
            },
        ));
    }

    combined.sort_by_key(|(s, kind_ord, idx, _)| (Reverse(*s), *kind_ord, *idx));
    combined
        .into_iter()
        .take(limit)
        .map(|(_, _, _, item)| item)
        .collect()
}

/// Builds heading anchor candidates (`[[Note#Heading]]` or `[[#Heading]]`).
pub fn heading_items(
    note_target: &str,
    headings: &[(usize, String)],
    embed: bool,
    query: &str,
    limit: usize,
) -> Vec<Item> {
    let q = query.trim().to_lowercase();
    let prefix = if embed { "!" } else { "" };
    let mut scored = Vec::new();
    for (idx, (level, text)) in headings.iter().enumerate() {
        let score = if q.is_empty() {
            1
        } else {
            let s = match_score(text, &q);
            if s == 0 {
                continue;
            }
            s
        };
        let insert = if note_target.is_empty() {
            format!("{prefix}[[#{text}]]")
        } else {
            format!("{prefix}[[{note_target}#{text}]]")
        };
        scored.push((
            score,
            idx,
            Item {
                label: text.clone(),
                detail: format!("H{level}"),
                icon: "hash",
                insert,
            },
        ));
    }
    scored.sort_by_key(|(s, idx, _)| (Reverse(*s), *idx));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, item)| item)
        .collect()
}

/// Builds tag completion candidates sorted by relevance and note count.
pub fn tag_items(tags: &[(String, usize)], query: &str, limit: usize) -> Vec<Item> {
    let q = query.trim().trim_start_matches('#').to_lowercase();
    let mut scored = Vec::new();
    for (tag, count) in tags {
        let score = if q.is_empty() {
            1
        } else {
            let s = match_score(tag, &q);
            if s == 0 {
                continue;
            }
            s
        };
        scored.push((
            score,
            *count,
            tag.to_lowercase(),
            Item {
                label: format!("#{tag}"),
                detail: count.to_string(),
                icon: "tag",
                insert: format!("#{tag}"),
            },
        ));
    }
    scored.sort_by(|a, b| {
        Reverse(a.0)
            .cmp(&Reverse(b.0))
            .then_with(|| Reverse(a.1).cmp(&Reverse(b.1)))
            .then_with(|| a.2.cmp(&b.2))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, _, item)| item)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_wikilink_with_and_without_autopaired_brackets() {
        let text = "参考 [[项目]] 后继续";
        let caret = "参考 [[项目".len();
        assert_eq!(
            detect(text, caret),
            Some(CompletionCtx {
                kind: TriggerKind::WikiLink,
                anchor_start: "参考 ".len(),
                replace_end: "参考 [[项目]]".len(),
                query: "项目".into(),
            })
        );

        let unclosed = "参考 [[项目";
        assert_eq!(
            detect(unclosed, unclosed.len()),
            Some(CompletionCtx {
                kind: TriggerKind::WikiLink,
                anchor_start: "参考 ".len(),
                replace_end: unclosed.len(),
                query: "项目".into(),
            })
        );
    }

    #[test]
    fn detects_embed_and_heading_anchors() {
        let embed_txt = "看图 ![[架构]]";
        assert_eq!(
            detect(embed_txt, "看图 ![[架构".len()),
            Some(CompletionCtx {
                kind: TriggerKind::Embed,
                anchor_start: "看图 ".len(),
                replace_end: embed_txt.len(),
                query: "架构".into(),
            })
        );

        let h_txt = "见 [[指南#安]]";
        assert_eq!(
            detect(h_txt, "见 [[指南#安".len()),
            Some(CompletionCtx {
                kind: TriggerKind::Heading {
                    embed: false,
                    note_target: "指南".into(),
                },
                anchor_start: "见 ".len(),
                replace_end: h_txt.len(),
                query: "安".into(),
            })
        );

        let local_h = "跳到 [[#总]]";
        assert_eq!(
            detect(local_h, "跳到 [[#总".len()),
            Some(CompletionCtx {
                kind: TriggerKind::Heading {
                    embed: false,
                    note_target: String::new(),
                },
                anchor_start: "跳到 ".len(),
                replace_end: local_h.len(),
                query: "总".into(),
            })
        );
    }

    #[test]
    fn suppresses_alias_pipe_block_ref_code_and_frontmatter() {
        assert_eq!(detect("[[笔记|别名", "[[笔记|别名".len()), None);
        assert_eq!(detect("[[笔记#^abc", "[[笔记#^abc".len()), None);
        assert_eq!(detect("`[[笔记`", "`[[笔记".len()), None);
        assert_eq!(detect("```\n[[笔记\n```", "```\n[[笔记".len()), None);
        assert_eq!(detect("---\ntag: [[x\n---", "---\ntag: [[x".len()), None);
    }

    #[test]
    fn detects_tags_without_false_positives_on_headings_or_numbers() {
        assert_eq!(detect("# 标题", "# 标题".len()), None);
        assert_eq!(detect("#123", "#123".len()), None);
        assert_eq!(detect("foo#bar", "foo#bar".len()), None);

        let t = "记录 #开发/rust 笔记";
        let caret = "记录 #开发".len();
        assert_eq!(
            detect(t, caret),
            Some(CompletionCtx {
                kind: TriggerKind::Tag,
                anchor_start: "记录 ".len(),
                replace_end: "记录 #开发/rust".len(),
                query: "开发".into(),
            })
        );
    }

    #[test]
    fn disambiguates_duplicate_titles_and_matches_aliases() {
        let notes = vec![
            (
                "周报".into(),
                "工作/周报.md".into(),
                vec!["Weekly".into(), "星期总结".into()],
            ),
            ("周报".into(), "生活/周报.md".into(), vec![]),
            (
                "Rust 笔记".into(),
                "Rust 笔记.md".into(),
                vec!["cargo".into()],
            ),
        ];
        let items = note_items(&notes, false, "week", 10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert, "[[工作/周报]]");
        assert!(items[0].detail.contains("Weekly"));

        let unique = note_items(&notes, false, "cargo", 10);
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].insert, "[[Rust 笔记]]");
    }

    #[test]
    fn embed_heading_and_tag_items_format_cleanly() {
        let notes = vec![("设计文档".into(), "设计文档.md".into(), vec![])];
        let files = vec!["assets/架构图.png".into(), "docs/readme.md".into()];
        let embeds = embed_items(&notes, &files, "架构", 10);
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0].icon, "photo");
        assert_eq!(embeds[0].insert, "![[架构图.png]]");

        let headings = heading_items("设计文档", &[(2, "总体架构".into())], false, "架构", 10);
        assert_eq!(headings[0].detail, "H2");
        assert_eq!(headings[0].insert, "[[设计文档#总体架构]]");

        let tags = tag_items(&[("rust".into(), 5), ("rust/gpui".into(), 2)], "ru", 10);
        assert_eq!(tags[0].insert, "#rust");
        assert_eq!(tags[0].detail, "5");
    }
}
