//! Note-level AI actions: suggest a title, suggest tags, summarise the note. Results appear in the
//! side panel; nothing is written into the note until the user presses the matching button.
//!
//! The text sent is the note as it is open in the editor. Excluded notes are refused.

use gpui::{App, Context};
use jiyue_ai::prompts;

use super::chat::{self, Kind, Role};
use super::context::cut_chars;
use super::selection::Target;
use super::{Modal, editing, effective_config};
use crate::RustdianApp;
use crate::ui::ext::ToastKind;

/// Longest note text sent for a note-level action.
pub const MAX_NOTE_CHARS: usize = 12_000;

/// The active note, when AI may use it. Shows a message and returns `None` otherwise.
fn ready_note(app: &mut RustdianApp, cx: &App) -> Option<(String, String)> {
    if !app.ext.ai.config.enabled {
        app.toast(ToastKind::Warn, "AI 功能未开启，请先在 设置 → AI 中开启");
        return None;
    }
    let cfg = effective_config(&app.ext.ai);
    if cfg.chat_provider().is_none() {
        app.toast(ToastKind::Warn, "请先在 设置 → AI 中选择对话服务");
        return None;
    }
    let (path, tags, fallback) = {
        let note = app.vault.active_note()?;
        let tags = super::exclusion_tags(note.content(), &note.tags);
        (note.path.clone(), tags, note.content().to_string())
    };
    if cfg.is_note_excluded(&path, &tags) {
        app.toast(ToastKind::Warn, "该笔记已排除，不能使用 AI 操作");
        return None;
    }
    let text = editing::editor_text(app, &path, cx).unwrap_or(fallback);
    Some((path, text))
}

/// Palette 总结当前笔记: asks for confirmation first, because the whole note is sent.
pub fn ask_summarize(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some((path, text)) = ready_note(app, cx) else {
        return;
    };
    let len = text.len();
    app.ext.ai.target = Some(Target {
        path,
        start: len,
        end: len,
        text,
        selected: false,
    });
    app.ext.ai.modal = Modal::Summary;
    cx.notify();
}

/// Menu 总结 without a selection: the same confirmation, with the target given.
pub fn ask_summarize_target(app: &mut RustdianApp, target: Target) {
    app.ext.ai.target = Some(target);
    app.ext.ai.modal = Modal::Summary;
}

/// The 发送 button of the summary dialog.
pub fn confirm_summary(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.ext.ai.modal = Modal::None;
    let Some(target) = app.ext.ai.target.take() else {
        return;
    };
    let (text, _) = cut_chars(&target.text, MAX_NOTE_CHARS);
    let prompt = prompts::summarize(&text);
    let path = target.path.clone();
    chat::oneshot(
        app,
        prompt.messages,
        cx,
        move |app, result, _cx| match result {
            Ok(answer) => {
                let id = chat::push(
                    app,
                    Role::Assistant,
                    Kind::Summary,
                    prompts::clean_output(&answer),
                );
                if let Some(m) = chat::message_mut(app, id) {
                    m.path = Some(path);
                }
                app.ext.ai.panel_open = true;
            }
            Err(e) => app.toast(ToastKind::Error, super::errors::describe(&e)),
        },
    );
    cx.notify();
}

/// Palette 建议标题: a candidate in the panel, with 复制 and 插入为标题.
pub fn suggest_title(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some((path, text)) = ready_note(app, cx) else {
        return;
    };
    let (text, _) = cut_chars(&text, MAX_NOTE_CHARS);
    let prompt = prompts::suggest_title(&text);
    app.ext.ai.panel_open = true;
    chat::oneshot(
        app,
        prompt.messages,
        cx,
        move |app, result, _cx| match result {
            Ok(answer) => {
                let title = prompts::parse_title(&prompts::clean_output(&answer));
                if title.is_empty() {
                    app.toast(ToastKind::Warn, "没有得到可用的标题");
                    return;
                }
                let id = chat::push(app, Role::Assistant, Kind::Title, title);
                if let Some(m) = chat::message_mut(app, id) {
                    m.path = Some(path);
                }
            }
            Err(e) => app.toast(ToastKind::Error, super::errors::describe(&e)),
        },
    );
    cx.notify();
}

/// Palette 建议标签: chips in the panel. A chip adds the tag to the frontmatter when pressed.
pub fn suggest_tags(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some((path, text)) = ready_note(app, cx) else {
        return;
    };
    let (text, _) = cut_chars(&text, MAX_NOTE_CHARS);
    let existing = app.vault.all_tags();
    let prompt = prompts::suggest_tags(&text, &existing);
    app.ext.ai.panel_open = true;
    chat::oneshot(
        app,
        prompt.messages,
        cx,
        move |app, result, _cx| match result {
            Ok(answer) => match prompts::parse_tags(&answer) {
                Ok(tags) if !tags.is_empty() => {
                    let id = chat::push(app, Role::Assistant, Kind::Tags(tags), String::new());
                    if let Some(m) = chat::message_mut(app, id) {
                        m.path = Some(path);
                    }
                }
                Ok(_) => app.toast(ToastKind::Info, "没有建议的标签"),
                Err(e) => app.toast(ToastKind::Error, super::errors::describe(&e)),
            },
            Err(e) => app.toast(ToastKind::Error, super::errors::describe(&e)),
        },
    );
    cx.notify();
}

/// Adds `tag` to the frontmatter `tags` list of the note at `path`, as one undo step.
pub fn add_tag(app: &mut RustdianApp, path: &str, tag: &str, cx: &mut Context<RustdianApp>) {
    let Some(text) = editing::editor_text(app, path, cx) else {
        app.toast(ToastKind::Warn, "该笔记没有打开，无法添加标签");
        return;
    };
    match tags_after_add(&text, tag) {
        None => app.toast(ToastKind::Info, format!("标签 #{tag} 已存在")),
        Some(new_text) => {
            // Only the changed span is replaced, so the caret and the scroll position stay put.
            let (range, insert) = minimal_edit(&text, &new_text);
            match editing::replace_checked(app, path, range.start, range.end, None, &insert, cx) {
                Ok(()) => app.toast(ToastKind::Success, format!("已添加标签 #{tag}")),
                Err(m) => app.toast(ToastKind::Error, m),
            }
        }
    }
    cx.notify();
}

/// The smallest single edit that turns `old` into `new`: the byte range of `old` to replace, and
/// the text to put there. Both ends are char boundaries.
pub fn minimal_edit(old: &str, new: &str) -> (std::ops::Range<usize>, String) {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let end_old = old.len() - suffix;
    let end_new = new.len() - suffix;
    (prefix..end_old, new[prefix..end_new].to_string())
}

/// New note text with `tag` in its frontmatter, or `None` when the tag is already there.
pub fn tags_after_add(text: &str, tag: &str) -> Option<String> {
    let tag = tag.trim().trim_start_matches('#');
    let existing = jiyue_md::frontmatter::tags(text);
    if existing
        .iter()
        .any(|t| t.trim_start_matches('#').eq_ignore_ascii_case(tag))
    {
        return None;
    }
    Some(jiyue_md::frontmatter::add_to_list(text, "tags", tag))
}

/// 插入为标题: a level-one heading after the frontmatter.
pub fn insert_title(app: &mut RustdianApp, path: &str, title: &str, cx: &mut Context<RustdianApp>) {
    match editing::insert_at_body_start(app, path, &heading_line(title), cx) {
        Ok(()) => app.toast(ToastKind::Success, "已插入标题"),
        Err(m) => app.toast(ToastKind::Error, m),
    }
    cx.notify();
}

/// 插入摘要: a callout after the frontmatter.
pub fn insert_summary(
    app: &mut RustdianApp,
    path: &str,
    summary: &str,
    cx: &mut Context<RustdianApp>,
) {
    match editing::insert_at_body_start(app, path, &callout(summary), cx) {
        Ok(()) => app.toast(ToastKind::Success, "已插入摘要"),
        Err(m) => app.toast(ToastKind::Error, m),
    }
    cx.notify();
}

/// `# title` followed by a blank line.
pub fn heading_line(title: &str) -> String {
    format!("# {}\n\n", title.trim())
}

/// An Obsidian-style note callout holding the summary, followed by a blank line.
pub fn callout(summary: &str) -> String {
    let mut out = String::from("> [!note] 摘要\n");
    for line in summary.trim().lines() {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callout_quotes_every_line() {
        let text = callout("第一点\n\n第二点");
        assert_eq!(text, "> [!note] 摘要\n> 第一点\n>\n> 第二点\n\n");
    }

    #[test]
    fn heading_is_level_one() {
        assert_eq!(heading_line("  新标题 "), "# 新标题\n\n");
    }

    #[test]
    fn tag_is_added_to_the_frontmatter_once() {
        let note = "---\ntitle: 测试\n---\n正文";
        let added = tags_after_add(note, "读书").expect("new tag");
        assert!(added.contains("读书"), "{added}");
        assert!(added.ends_with("正文"));
        assert!(
            jiyue_md::frontmatter::tags(&added)
                .iter()
                .any(|t| t == "读书")
        );
        assert!(tags_after_add(&added, "读书").is_none(), "already present");
        assert!(
            tags_after_add(&added, "#读书").is_none(),
            "hash prefix ignored"
        );
    }

    #[test]
    fn minimal_edit_replaces_only_the_changed_span() {
        let old = "abc";
        let (range, insert) = minimal_edit(old, "axc");
        assert_eq!(range, 1..2);
        assert_eq!(insert, "x");
        // Applying the edit reproduces the new text, also with multi-byte characters.
        for (a, b) in [
            (
                "---\ntags: [a]\n---\n正文",
                "---\ntags:\n  - a\n  - 读书\n---\n正文",
            ),
            ("aa", "aaa"),
            ("同样", "同样"),
            ("", "新"),
            ("删", ""),
        ] {
            let (r, ins) = minimal_edit(a, b);
            let mut out = String::from(&a[..r.start]);
            out.push_str(&ins);
            out.push_str(&a[r.end..]);
            assert_eq!(out, b, "{a:?} -> {b:?}");
        }
    }

    #[test]
    fn tag_is_added_when_the_note_has_no_frontmatter() {
        let added = tags_after_add("只有正文", "新").expect("new tag");
        assert!(added.contains("新"));
        assert!(added.contains("只有正文"));
    }
}
