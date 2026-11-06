//! Context assembly: which notes and selections may leave the machine, how the prompt is built
//! from them, and how the preview totals are kept consistent. Pure functions, unit-tested.

use std::ops::Range;

use jiyue_ai::prompts::{self, Prompt};
use jiyue_ai::{AiConfig, ChatRole, Context as RagContext};
use jiyue_search::NoteDoc;

/// Longest selection sent with a question or an action.
pub const MAX_SELECTION_CHARS: usize = 4000;

/// A note as the AI features see it: its text and whether it is excluded from every model call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSnap {
    pub path: String,
    pub title: String,
    pub text: String,
    pub tags: Vec<String>,
    pub excluded: bool,
}

impl NoteSnap {
    pub fn new(cfg: &AiConfig, path: &str, title: &str, text: &str, tags: &[String]) -> Self {
        Self {
            path: path.to_string(),
            title: title.to_string(),
            text: text.to_string(),
            tags: tags.to_vec(),
            excluded: cfg.is_note_excluded(path, tags),
        }
    }
}

/// Notes that may be used as sources. Excluded notes never appear here.
pub fn allowed(snaps: &[NoteSnap]) -> impl Iterator<Item = &NoteSnap> {
    snaps.iter().filter(|s| !s.excluded)
}

/// Documents for the in-memory lexical index (excluded notes are left out).
pub fn lexical_docs(snaps: &[NoteSnap]) -> Vec<NoteDoc> {
    allowed(snaps)
        .map(|s| NoteDoc {
            path: s.path.clone(),
            title: s.title.clone(),
            body: s.text.clone(),
            tags: s.tags.clone(),
            mtime_ms: 0,
        })
        .collect()
}

/// Byte range and text of a selection given as (anchor, head) byte offsets. `None` when the
/// selection is empty or does not lie on character boundaries of `text`.
pub fn selection_slice(text: &str, anchor: usize, head: usize) -> Option<(Range<usize>, String)> {
    let lo = anchor.min(head);
    let hi = anchor.max(head);
    if lo == hi || hi > text.len() || !text.is_char_boundary(lo) || !text.is_char_boundary(hi) {
        return None;
    }
    let body = &text[lo..hi];
    if body.trim().is_empty() {
        return None;
    }
    Some((lo..hi, body.to_string()))
}

/// Limits a text to `max` characters. Returns the text and whether it was cut.
pub fn cut_chars(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        (text.to_string(), false)
    } else {
        (text.chars().take(max).collect(), true)
    }
}

/// Adds a labelled part to the user message of `prompt` and keeps the preview totals in step.
pub fn append_part(prompt: &mut Prompt, label: &str, body: &str) {
    if let Some(user) = prompt
        .messages
        .iter_mut()
        .find(|m| m.role == ChatRole::User)
    {
        user.content.push_str("\n\n");
        user.content.push_str(body);
    }
    prompt
        .preview
        .items
        .push((label.to_string(), body.chars().count()));
    recount(prompt);
}

/// Recomputes the total of the preview from its items.
pub fn recount(prompt: &mut Prompt) {
    prompt.preview.total_chars = prompt.preview.items.iter().map(|(_, n)| *n).sum();
}

/// Labelled block for a selection, with a shortened marker when it was cut.
pub fn selection_part(selected: &str) -> String {
    let (body, cut) = cut_chars(selected, MAX_SELECTION_CHARS);
    let mut block = format!("Selected text:\n<<<\n{body}\n>>>");
    if cut {
        block.push_str("\n(excerpt shortened)");
    }
    block
}

/// The question prompt: notes as sources, the current note and, when given, the selection.
/// Without vault search the "no matching notes" part is left out, because nothing was searched.
pub fn question_prompt(
    question: &str,
    contexts: &[RagContext],
    current: Option<(&str, &str)>,
    selection: Option<&str>,
    vault_search: bool,
) -> Prompt {
    let mut prompt = prompts::ask_with_context(question, contexts, current);
    if !vault_search && contexts.is_empty() {
        remove_part(&mut prompt, "Notes", "No matching notes were found.");
    }
    if let Some(sel) = selection {
        append_part(&mut prompt, "选中文字", &selection_part(sel));
    }
    prompt
}

/// Removes a labelled part (its item in the preview and its text in the user message).
pub fn remove_part(prompt: &mut Prompt, label: &str, body: &str) {
    prompt.preview.items.retain(|(l, _)| l != label);
    if let Some(user) = prompt
        .messages
        .iter_mut()
        .find(|m| m.role == ChatRole::User)
    {
        user.content = user
            .content
            .replace(&format!("\n\n{body}"), "")
            .replace(body, "");
    }
    recount(prompt);
}

/// Human label of a note in the preview list: the path as stored.
pub fn preview_label(path: &str) -> String {
    format!("笔记：{path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiyue_ai::{ChatRole, ProviderEntry, ProviderKind};

    fn cfg_with_rules() -> AiConfig {
        AiConfig {
            enabled: true,
            providers: vec![ProviderEntry {
                id: "p1".into(),
                name: "x".into(),
                kind: ProviderKind::OpenAiCompatible,
                base_url: "http://127.0.0.1:1/v1".into(),
                chat_model: "m".into(),
                embedding_model: None,
            }],
            active_chat: Some("p1".into()),
            active_embedding: None,
            excluded_folders: vec!["私人".into()],
            excluded_tags: vec!["secret".into()],
        }
    }

    #[test]
    fn excluded_notes_never_become_sources() {
        let cfg = cfg_with_rules();
        let snaps = vec![
            NoteSnap::new(&cfg, "公开/a.md", "a", "甲", &[]),
            NoteSnap::new(&cfg, "私人/b.md", "b", "乙", &[]),
            NoteSnap::new(&cfg, "c.md", "c", "丙", &["Secret".to_string()]),
        ];
        let allowed_paths: Vec<&str> = allowed(&snaps).map(|s| s.path.as_str()).collect();
        assert_eq!(allowed_paths, vec!["公开/a.md"]);
        let docs = lexical_docs(&snaps);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].path, "公开/a.md");
    }

    #[test]
    fn selection_must_be_non_empty_and_on_boundaries() {
        let text = "前缀 你好 后缀";
        let start = text.find("你").expect("present");
        let end = text.find("后").expect("present");
        let (range, body) = selection_slice(text, end, start).expect("reversed ok");
        assert_eq!(range, start..end);
        assert_eq!(body, "你好 ");
        assert!(selection_slice(text, 3, 3).is_none());
        assert!(selection_slice(text, 1, 2).is_none(), "inside a character");
        assert!(selection_slice(text, 0, 999).is_none());
        assert!(selection_slice("   ", 0, 3).is_none(), "whitespace only");
    }

    #[test]
    fn cut_marks_long_selection() {
        let long = "字".repeat(MAX_SELECTION_CHARS + 5);
        let part = selection_part(&long);
        assert!(part.contains("(excerpt shortened)"));
        assert!(part.starts_with("Selected text:"));
        assert!(!selection_part("短").contains("shortened"));
    }

    #[test]
    fn notes_part_is_dropped_when_vault_search_is_off() {
        let off = question_prompt("问", &[], None, None, false);
        assert!(!off.preview.items.iter().any(|(l, _)| l == "Notes"));
        assert!(!off.messages[1].content.contains("No matching notes"));
        let on = question_prompt("问", &[], None, None, true);
        assert!(on.preview.items.iter().any(|(l, _)| l == "Notes"));
        let sum: usize = off.preview.items.iter().map(|(_, n)| *n).sum();
        assert_eq!(off.preview.total_chars, sum);
    }

    #[test]
    fn preview_totals_match_the_parts() {
        let mut prompt = question_prompt(
            "问题是什么",
            &[],
            Some(("a.md", "正文内容")),
            Some("选区"),
            true,
        );
        let sum: usize = prompt.preview.items.iter().map(|(_, n)| *n).sum();
        assert_eq!(prompt.preview.total_chars, sum);
        assert!(prompt.preview.items.iter().any(|(l, _)| l == "选中文字"));
        // The selection text really is part of the user message that will be sent.
        let user = prompt
            .messages
            .iter()
            .find(|m| m.role == ChatRole::User)
            .expect("user message");
        assert!(user.content.contains("选区"));
        // Appending keeps the invariant.
        append_part(&mut prompt, "extra", "额外");
        let sum: usize = prompt.preview.items.iter().map(|(_, n)| *n).sum();
        assert_eq!(prompt.preview.total_chars, sum);
    }
}
