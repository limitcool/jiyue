//! Side-panel chat flow: prepare a question (preview), confirm the send, stream the answer, stop,
//! retry and regenerate. Also the one-shot requests the note actions use.
//!
//! Every send goes through a preview first: the prompt that was previewed is the one that is
//! sent, and nothing is sent before the user confirms it.

use gpui::Context;
use jiyue_ai::prompts::Prompt;
use jiyue_ai::{AiError, CancelToken, ChatMessage, ChatResponse};

use super::{editing, effective_config, errors, host_of, input_clear, input_text, runner, worker};
use crate::RustdianApp;
use crate::ui::ext::ToastKind;

/// Who wrote a panel message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// What an assistant message is: a chat answer, or the result of a note action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Chat,
    /// A suggested title (in `text`), inserted as a heading on request.
    Title,
    /// Suggested tags, each one a button.
    Tags(Vec<String>),
    /// A summary (in `text`), inserted as a callout on request.
    Summary,
}

/// One message in the side panel.
#[derive(Debug, Clone)]
pub struct UiMessage {
    pub id: u64,
    pub role: Role,
    pub kind: Kind,
    pub text: String,
    pub streaming: bool,
    /// The user pressed 停止 before the end.
    pub stopped: bool,
    pub error: Option<AiError>,
    /// The messages that produced this answer, kept to retry or regenerate it.
    pub request: Vec<ChatMessage>,
    /// Note paths the answer was built from (shown as sources).
    pub sources: Vec<String>,
    /// Note the action applies to (title, tags, summary).
    pub path: Option<String>,
    /// The model's reasoning for this answer, shown under 思考过程 (collapsed by default).
    pub reasoning: Option<String>,
    /// The 思考过程 block is expanded.
    pub reasoning_open: bool,
    /// The model name the server reported for this answer, when it is known.
    pub answered_by: Option<String>,
}

/// A prepared question. Shown in the preview card until the user confirms or cancels.
#[derive(Debug, Clone)]
pub struct Preview {
    pub question: String,
    pub prompt: Prompt,
    pub sources: Vec<String>,
    pub provider: String,
    pub host: String,
    pub vector_used: bool,
    pub warning: Option<String>,
}

/// The chat stream in progress.
pub struct Busy {
    pub id: u64,
    pub cancel: CancelToken,
}

/// Adds a message and returns its id.
pub fn push(app: &mut RustdianApp, role: Role, kind: Kind, text: String) -> u64 {
    let id = app.ext.ai.take_id();
    app.ext.ai.messages.push(UiMessage {
        id,
        role,
        kind,
        text,
        streaming: false,
        stopped: false,
        error: None,
        request: Vec::new(),
        sources: Vec::new(),
        path: None,
        reasoning: None,
        reasoning_open: false,
        answered_by: None,
    });
    id
}

pub fn message_mut(app: &mut RustdianApp, id: u64) -> Option<&mut UiMessage> {
    app.ext.ai.messages.iter_mut().find(|m| m.id == id)
}

/// The Enter key and the send button. Prepares the preview; nothing is sent here.
pub fn send_draft(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let question = input_text(&app.ext.ai.inputs.draft, cx).trim().to_string();
    if question.is_empty() {
        return;
    }
    if app.ext.ai.busy.is_some() {
        app.toast(ToastKind::Warn, "正在生成回答，请先停止或等待完成");
        return;
    }
    let cfg = effective_config(&app.ext.ai);
    let Some(entry) = cfg.chat_provider().cloned() else {
        let msg = if app.ext.ai.config.enabled {
            "请先在 设置 → AI 中选择对话服务"
        } else {
            "AI 功能未开启，请先在 设置 → AI 中开启"
        };
        app.toast(ToastKind::Warn, msg);
        return;
    };
    let chips = app.ext.ai.chips;

    // The active note, unless the user turned it off or it is excluded.
    let mut current: Option<(String, String)> = None;
    let mut excluded_note = false;
    if let Some(note) = app.vault.active_note() {
        let tags = super::exclusion_tags(note.content(), &note.tags);
        if cfg.is_note_excluded(&note.path, &tags) {
            excluded_note = chips.note;
        } else if chips.note {
            current = Some((note.path.clone(), note.content().to_string()));
        }
    }
    if excluded_note {
        app.toast(ToastKind::Warn, "该笔记已排除，不会发送");
    }

    let mut selection_text: Option<String> = None;
    if chips.selection {
        selection_text = editing::selection(app, cx).map(|t| t.text);
        if selection_text.is_none() {
            app.toast(ToastKind::Info, "没有选中文字，本次不包含选区");
        }
    }

    let current_path = current.as_ref().map(|(p, _)| p.clone());
    let job = worker::PrepareJob {
        question: question.clone(),
        notes: super::snapshot_notes(app, &cfg),
        current,
        selection: selection_text,
        vault_search: chips.vault,
        index_dir: app.vault.root_dir.join(".jiyue").join("index"),
        embed: cfg
            .embedding_provider()
            .cloned()
            .map(|e| (e, app.ext.ai.test.clone())),
        test: app.ext.ai.test.clone(),
    };
    app.ext.ai.preparing = true;
    let provider = entry.name.clone();
    let host = host_of(&entry.base_url);
    runner::once(
        cx,
        move || worker::prepare(&job),
        move |app, result, _cx| {
            app.ext.ai.preparing = false;
            match result {
                Ok(prepared) => {
                    let mut sources: Vec<String> = Vec::new();
                    if let Some(path) = current_path {
                        sources.push(path);
                    }
                    for c in &prepared.contexts {
                        if !sources.contains(&c.path) {
                            sources.push(c.path.clone());
                        }
                    }
                    app.ext.ai.preview = Some(Preview {
                        question,
                        prompt: prepared.prompt,
                        sources,
                        provider,
                        host,
                        vector_used: prepared.vector_used,
                        warning: prepared.warning,
                    });
                }
                Err(e) => app.toast(ToastKind::Error, errors::describe(&e)),
            }
        },
    );
    cx.notify();
}

/// The confirm button of the preview card: sends exactly the previewed prompt.
pub fn confirm_send(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(preview) = app.ext.ai.preview.take() else {
        return;
    };
    input_clear(&app.ext.ai.inputs.draft, cx);
    push(app, Role::User, Kind::Chat, preview.question.clone());
    let id = push(app, Role::Assistant, Kind::Chat, String::new());
    if let Some(m) = message_mut(app, id) {
        m.request = preview.prompt.messages.clone();
        m.sources = preview.sources.clone();
    }
    start_stream(app, id, preview.prompt.messages, cx);
    cx.notify();
}

/// Drops the preview without sending anything.
pub fn cancel_preview(app: &mut RustdianApp) {
    app.ext.ai.preview = None;
}

/// Streams a reply into the message `id`.
pub fn start_stream(
    app: &mut RustdianApp,
    id: u64,
    messages: Vec<ChatMessage>,
    cx: &mut Context<RustdianApp>,
) {
    let cfg = effective_config(&app.ext.ai);
    let Some(entry) = cfg.chat_provider().cloned() else {
        if let Some(m) = message_mut(app, id) {
            m.error = Some(AiError::Other("没有配置对话服务".into()));
        }
        return;
    };
    let cancel = CancelToken::new();
    app.ext.ai.busy = Some(Busy {
        id,
        cancel: cancel.clone(),
    });
    if let Some(m) = message_mut(app, id) {
        m.streaming = true;
        m.stopped = false;
        m.error = None;
    }
    let rx = worker::spawn_chat(entry, app.ext.ai.test.clone(), messages, cancel);
    runner::pump(cx, rx, move |app, msg, _cx| match msg {
        worker::ChatMsg::Delta(d) => {
            if let Some(m) = message_mut(app, id) {
                m.text.push_str(&d);
            }
            app.ext.ai.scroll.scroll_to_bottom();
            true
        }
        worker::ChatMsg::Done(result) => {
            finish(app, id, result);
            false
        }
    });
    app.ext.ai.scroll.scroll_to_bottom();
    cx.notify();
}

fn finish(app: &mut RustdianApp, id: u64, result: Result<ChatResponse, AiError>) {
    if app.ext.ai.busy.as_ref().is_some_and(|b| b.id == id) {
        app.ext.ai.busy = None;
    }
    if let Ok(resp) = &result {
        app.ext.ai.last_answered_by = resp.model.clone();
    }
    let Some(m) = message_mut(app, id) else {
        return;
    };
    m.streaming = false;
    match result {
        Ok(resp) => {
            // A reply with reasoning but no answer is not a success: say so, keep the reasoning.
            let reasoning_only = resp.is_reasoning_only();
            if !resp.content.trim().is_empty() {
                m.text = resp.content;
            }
            m.reasoning = resp.reasoning;
            m.answered_by = resp.model;
            if reasoning_only {
                m.error = Some(AiError::Other(
                    "模型只返回了思考过程，没有给出回答。可以重试，或换一个模型。".into(),
                ));
            }
        }
        Err(AiError::Cancelled) => m.stopped = true,
        Err(e) => m.error = Some(e),
    }
    app.ext.ai.scroll.scroll_to_bottom();
}

/// 停止: cancels the stream in progress. The partial answer stays.
pub fn stop(app: &mut RustdianApp) {
    if let Some(busy) = &app.ext.ai.busy {
        busy.cancel.cancel();
    }
}

/// 重试 on a failed answer: same messages, same message.
pub fn retry(app: &mut RustdianApp, id: u64, cx: &mut Context<RustdianApp>) {
    if app.ext.ai.busy.is_some() {
        return;
    }
    let request = app
        .ext
        .ai
        .messages
        .iter()
        .find(|m| m.id == id)
        .map(|m| m.request.clone())
        .unwrap_or_default();
    if request.is_empty() {
        return;
    }
    if let Some(m) = message_mut(app, id) {
        m.text.clear();
    }
    start_stream(app, id, request, cx);
}

/// 重新生成: a new answer for the same question.
pub fn regenerate(app: &mut RustdianApp, id: u64, cx: &mut Context<RustdianApp>) {
    if app.ext.ai.busy.is_some() {
        return;
    }
    let Some(source) = app.ext.ai.messages.iter().find(|m| m.id == id).cloned() else {
        return;
    };
    if source.request.is_empty() {
        return;
    }
    let new_id = push(app, Role::Assistant, Kind::Chat, String::new());
    if let Some(m) = message_mut(app, new_id) {
        m.request = source.request.clone();
        m.sources = source.sources.clone();
    }
    start_stream(app, new_id, source.request, cx);
}

/// 清空: forgets the conversation and stops any stream.
pub fn clear(app: &mut RustdianApp) {
    stop(app);
    app.ext.ai.messages.clear();
    app.ext.ai.preview = None;
}

/// One request whose answer is used as a whole (titles, tags, summaries). `on_done` gets the
/// full text or the error, on the UI thread.
pub fn oneshot(
    app: &mut RustdianApp,
    messages: Vec<ChatMessage>,
    cx: &mut Context<RustdianApp>,
    on_done: impl FnOnce(&mut RustdianApp, Result<String, AiError>, &mut Context<RustdianApp>) + 'static,
) {
    let cfg = effective_config(&app.ext.ai);
    let Some(entry) = cfg.chat_provider().cloned() else {
        app.toast(ToastKind::Warn, "请先在 设置 → AI 中开启 AI 并选择对话服务");
        return;
    };
    let cancel = CancelToken::new();
    let rx = worker::spawn_chat(entry, app.ext.ai.test.clone(), messages, cancel);
    let mut done = Some(on_done);
    runner::pump(cx, rx, move |app, msg, cx| match msg {
        worker::ChatMsg::Delta(_) => true,
        worker::ChatMsg::Done(result) => {
            if let Some(f) = done.take() {
                f(app, result.map(|r| r.content), cx);
            }
            false
        }
    });
    cx.notify();
}

#[cfg(test)]
mod tests {
    use crate::ui::ai_ui::State;

    #[test]
    fn message_ids_are_unique() {
        let mut state = State::default();
        let a = state.take_id();
        let b = state.take_id();
        assert_ne!(a, b);
    }
}
