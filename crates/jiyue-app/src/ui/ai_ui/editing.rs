//! Applying AI results to the open note. Every change goes through `TextEditor::replace_text_range`,
//! so it is one undo step, and the autosave of the note picks it up like typing.

use gpui::{App, Context};

use super::context::selection_slice;
use super::selection::Target;
use crate::RustdianApp;
use crate::ui::history_ui;
use crate::ui::text_input::TextEditor;

/// The non-empty selection of the active note's editor, if any.
pub fn selection(app: &RustdianApp, cx: &App) -> Option<Target> {
    let (path, editor) = super::active_editor(app)?;
    let ed = editor.read(cx);
    let text = ed.text();
    let (anchor, head) = ed.selection_offsets();
    let (range, body) = selection_slice(&text, anchor, head)?;
    Some(Target {
        path,
        start: range.start,
        end: range.end,
        text: body,
        selected: true,
    })
}

/// Full text of an open note's editor.
pub fn editor_text(app: &RustdianApp, path: &str, cx: &App) -> Option<String> {
    app.editor_for(path).map(|e| e.read(cx).text())
}

/// Replaces `start..end` of the note at `path` with `text`, as one undo step.
///
/// Refuses when the note is not open, when the text under the range is no longer `expected`
/// (the note changed meanwhile), or during an IME composition.
pub fn replace_checked(
    app: &RustdianApp,
    path: &str,
    start: usize,
    end: usize,
    expected: Option<&str>,
    text: &str,
    cx: &mut App,
) -> Result<(), String> {
    let editor = app
        .editor_for(path)
        .ok_or_else(|| "该笔记没有打开，无法写入".to_string())?;
    let current = editor.read(cx).text();
    if start > end
        || end > current.len()
        || !current.is_char_boundary(start)
        || !current.is_char_boundary(end)
    {
        return Err("笔记已变化，未写入".into());
    }
    if let Some(expected) = expected
        && &current[start..end] != expected
    {
        return Err("选中的文字已变化，未替换".into());
    }
    let applied = editor.update(cx, |ed: &mut TextEditor, cx: &mut Context<TextEditor>| {
        ed.replace_text_range(start..end, text, cx)
    });
    if applied {
        // The text this apply replaces stays in the version history.
        history_ui::keep_before_edit(app, path, &current);
        Ok(())
    } else {
        Err("正在输入法组合输入中，请确认输入后再试".into())
    }
}

/// Appends `text` at the end of the note, on its own paragraph.
pub fn append_to_note(
    app: &RustdianApp,
    path: &str,
    text: &str,
    cx: &mut App,
) -> Result<(), String> {
    let len = editor_text(app, path, cx)
        .map(|t| t.len())
        .ok_or_else(|| "该笔记没有打开，无法写入".to_string())?;
    let body = format!("\n\n{}", text.trim_end());
    replace_checked(app, path, len, len, None, &body, cx)
}

/// Byte offset where the body starts (after the frontmatter block, if any).
pub fn body_start(text: &str) -> usize {
    jiyue_md::frontmatter::split(text).1
}

/// Inserts `text` before the body of the note, after its frontmatter.
pub fn insert_at_body_start(
    app: &RustdianApp,
    path: &str,
    text: &str,
    cx: &mut App,
) -> Result<(), String> {
    let current =
        editor_text(app, path, cx).ok_or_else(|| "该笔记没有打开，无法写入".to_string())?;
    let at = body_start(&current);
    replace_checked(app, path, at, at, None, text, cx)
}

/// Inserts the text of a selection's replacement into the note, or appends it when the target
/// is a position at the end. Used by the result card.
pub fn apply_target(
    app: &RustdianApp,
    target: &Target,
    expected: Option<&str>,
    text: &str,
    cx: &mut App,
) -> Result<(), String> {
    replace_checked(
        app,
        &target.path,
        target.start,
        target.end,
        expected,
        text,
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_start_skips_frontmatter() {
        let with = "---\ntags: [a]\n---\n正文";
        let at = body_start(with);
        assert_eq!(&with[at..], "正文");
        assert_eq!(body_start("没有属性"), 0);
    }
}
