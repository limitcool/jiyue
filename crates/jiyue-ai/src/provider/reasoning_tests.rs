//! Mock-server tests for reasoning output, the model that answered, streams answered without
//! `stream: true`, and the output limit sent by default.

use std::time::Duration;

use crate::testutil::{MockServer, Reply};
use crate::{
    AiError, ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatRole, ProviderConfig,
    ProviderKind,
};

use super::{HttpSettings, Provider, RetryPolicy};

const KEY: &str = "sk-REASONING-TEST-KEY";

fn settings(stream: bool) -> HttpSettings {
    HttpSettings {
        connect_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_secs(5),
        retry: RetryPolicy {
            max_retries: 0,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(50),
        },
        stream,
    }
}

fn openai(base: &str, stream: bool) -> Provider {
    Provider::with_settings(
        ProviderConfig {
            base_url: base.into(),
            model: "configured-model".into(),
            kind: ProviderKind::OpenAiCompatible,
        },
        Some(KEY.into()),
        settings(stream),
    )
    .expect("provider")
}

fn anthropic(base: &str) -> Provider {
    Provider::with_settings(
        ProviderConfig {
            base_url: base.into(),
            model: "claude-configured".into(),
            kind: ProviderKind::Anthropic,
        },
        Some(KEY.into()),
        settings(true),
    )
    .expect("provider")
}

fn user_req() -> ChatRequest {
    ChatRequest::new(
        "",
        vec![ChatMessage {
            role: ChatRole::User,
            content: "question".into(),
        }],
    )
}

fn ask(p: &Provider, req: &ChatRequest) -> (Result<ChatResponse, AiError>, String) {
    let mut seen = String::new();
    let res = p.chat_stream(req, &mut |d| seen.push_str(d));
    (res, seen)
}

// ---------- OpenAI-compatible: reasoning_content ----------

#[test]
fn openai_stream_keeps_reasoning_apart_from_the_answer() {
    let events = [
        r#"{"model":"gateway-alias-7","choices":[{"delta":{"reasoning_content":"先想"}}]}"#,
        r#"{"model":"gateway-alias-7","choices":[{"delta":{"reasoning_content":"一下"}}]}"#,
        r#"{"model":"gateway-alias-7","choices":[{"delta":{"content":"答"}}]}"#,
        r#"{"model":"gateway-alias-7","choices":[{"delta":{"content":"案"},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ];
    let srv = MockServer::start(move |_| Reply::sse(&events));
    let p = openai(&format!("{}/v1", srv.base_url), true);
    let (res, seen) = ask(&p, &user_req());
    let out = res.expect("ok");
    assert_eq!(out.content, "答案");
    assert_eq!(out.reasoning.as_deref(), Some("先想一下"));
    assert_eq!(out.model.as_deref(), Some("gateway-alias-7"));
    assert_eq!(seen, "答案", "reasoning is never passed as answer text");
    assert!(!out.is_reasoning_only());
}

#[test]
fn openai_reasoning_only_reply_is_flagged_not_a_plain_success() {
    let srv = MockServer::start(|_| {
        Reply::json(
            200,
            r#"{"model":"m-x","choices":[{"message":{"role":"assistant","content":"","reasoning_content":"推理中，未输出答案"},"finish_reason":"length"}]}"#,
        )
    });
    let p = openai(&srv.base_url, false);
    let (res, seen) = ask(&p, &user_req());
    let out = res.expect("the call itself succeeds");
    assert!(out.content.is_empty());
    assert_eq!(out.reasoning.as_deref(), Some("推理中，未输出答案"));
    assert!(out.is_reasoning_only(), "the caller must be able to tell");
    assert_eq!(out.finish_reason.as_deref(), Some("length"));
    assert!(seen.is_empty(), "reasoning is not delivered as answer text");
}

#[test]
fn openai_stream_reasoning_only_is_flagged() {
    let events = [
        r#"{"choices":[{"delta":{"reasoning_content":"只有推理"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
        "[DONE]",
    ];
    let srv = MockServer::start(move |_| Reply::sse(&events));
    let p = openai(&srv.base_url, true);
    let (res, _) = ask(&p, &user_req());
    let out = res.expect("ok");
    assert!(out.is_reasoning_only());
    assert_eq!(out.reasoning.as_deref(), Some("只有推理"));
}

#[test]
fn openai_non_stream_reads_reasoning_and_the_answering_model() {
    let srv = MockServer::start(|_| {
        Reply::json(
            200,
            r#"{"model":"deepseek-alias","choices":[{"message":{"content":"好的","reasoning":"gateway 字段"},"finish_reason":"stop"}]}"#,
        )
    });
    let p = openai(&srv.base_url, false);
    let (res, seen) = ask(&p, &user_req());
    let out = res.expect("ok");
    assert_eq!(out.content, "好的");
    assert_eq!(out.reasoning.as_deref(), Some("gateway 字段"));
    assert_eq!(out.model.as_deref(), Some("deepseek-alias"));
    assert_eq!(seen, "好的");
}

#[test]
fn openai_without_reasoning_fields_reports_none() {
    let srv =
        MockServer::start(|_| Reply::json(200, r#"{"choices":[{"message":{"content":"plain"}}]}"#));
    let p = openai(&srv.base_url, false);
    let out = ask(&p, &user_req()).0.expect("ok");
    assert_eq!(out.reasoning, None);
    assert_eq!(out.model, None);
    assert!(!out.is_reasoning_only());
}

// ---------- stream=false with an SSE answer ----------

#[test]
fn stream_false_still_reads_an_event_stream_answer() {
    let events = [
        r#"{"choices":[{"delta":{"content":"流"}}]}"#,
        r#"{"choices":[{"delta":{"content":"式"},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ];
    // The server ignores `stream: false` and answers with an event stream.
    let srv = MockServer::start(move |_| Reply::sse(&events));
    let p = openai(&srv.base_url, false);
    let (res, seen) = ask(&p, &user_req());
    let out = res.expect("parsed as a stream");
    assert_eq!(out.content, "流式");
    assert_eq!(seen, "流式", "deltas still arrive while streaming");
    let recs = srv.recorded();
    assert_eq!(
        recs[0].json()["stream"],
        false,
        "the request itself is unchanged"
    );
}

#[test]
fn event_stream_body_without_its_content_type_is_still_parsed() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"嗅\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"探\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let srv = MockServer::start(move |_| Reply::json(200, body));
    let p = openai(&srv.base_url, false);
    let (res, seen) = ask(&p, &user_req());
    assert_eq!(res.expect("sniffed").content, "嗅探");
    assert_eq!(seen, "嗅探");
}

// ---------- output limit ----------

#[test]
fn default_request_sends_no_output_limit() {
    let srv =
        MockServer::start(|_| Reply::json(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#));
    let p = openai(&srv.base_url, false);
    let _ = ask(&p, &user_req());
    let body = srv.recorded()[0].json();
    assert!(
        body.get("max_tokens").is_none(),
        "no max_tokens unless the caller sets one: {body}"
    );
}

#[test]
fn explicit_output_limit_is_sent_as_given() {
    let srv =
        MockServer::start(|_| Reply::json(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#));
    let p = openai(&srv.base_url, false);
    let mut req = user_req();
    req.max_tokens = Some(512);
    let _ = ask(&p, &req);
    assert_eq!(srv.recorded()[0].json()["max_tokens"], 512);
}

// ---------- Anthropic: thinking blocks ----------

#[test]
fn anthropic_thinking_block_is_reasoning_not_answer() {
    let srv = MockServer::start(|_| {
        Reply::json(
            200,
            r#"{"model":"claude-alias","content":[{"type":"thinking","thinking":"让我想想","signature":"s"},{"type":"text","text":"结论"}],"stop_reason":"end_turn"}"#,
        )
    });
    let p = anthropic(&srv.base_url);
    let (res, seen) = ask(&p, &user_req());
    let out: ChatResponse = res.expect("ok");
    assert_eq!(out.content, "结论");
    assert_eq!(out.reasoning.as_deref(), Some("让我想想"));
    assert_eq!(out.model.as_deref(), Some("claude-alias"));
    assert_eq!(seen, "结论");
}

#[test]
fn anthropic_thinking_deltas_stream_apart_from_text() {
    let events = [
        r#"{"type":"message_start","message":{"id":"m","model":"claude-stream-alias"}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"推理"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"x"}}"#,
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"回答"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        r#"{"type":"message_stop"}"#,
    ];
    let srv = MockServer::start(move |_| Reply::sse(&events));
    let p = anthropic(&srv.base_url);
    let (res, seen) = ask(&p, &user_req());
    let out = res.expect("ok");
    assert_eq!(out.content, "回答");
    assert_eq!(out.reasoning.as_deref(), Some("推理"));
    assert_eq!(out.model.as_deref(), Some("claude-stream-alias"));
    assert_eq!(seen, "回答");
}

#[test]
fn anthropic_thinking_only_reply_is_flagged() {
    let srv = MockServer::start(|_| {
        Reply::json(
            200,
            r#"{"content":[{"type":"thinking","thinking":"只想不说"}],"stop_reason":"max_tokens"}"#,
        )
    });
    let p = anthropic(&srv.base_url);
    let out = ask(&p, &user_req()).0.expect("ok");
    assert!(out.is_reasoning_only());
    assert_eq!(out.model, None);
}
