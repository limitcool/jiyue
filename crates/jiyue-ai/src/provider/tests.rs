//! HTTP-level tests against the scripted mock server in `testutil`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::testutil::{MockServer, Reply};
use crate::{
    AiError, CancelToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatRole,
    EmbeddingProvider, ProviderConfig, ProviderKind,
};

use super::{HttpSettings, Provider, RetryPolicy};

const KEY: &str = "sk-SECRET-KEY-12345";

fn fast_settings() -> HttpSettings {
    HttpSettings {
        connect_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_secs(5),
        retry: RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(50),
        },
        stream: true,
    }
}

fn openai_at(base: &str) -> Provider {
    Provider::with_settings(
        ProviderConfig {
            base_url: base.into(),
            model: "gpt-test".into(),
            kind: ProviderKind::OpenAiCompatible,
        },
        Some(KEY.into()),
        fast_settings(),
    )
    .expect("provider")
}

fn anthropic_at(base: &str) -> Provider {
    Provider::with_settings(
        ProviderConfig {
            base_url: base.into(),
            model: "claude-test".into(),
            kind: ProviderKind::Anthropic,
        },
        Some(KEY.into()),
        fast_settings(),
    )
    .expect("provider")
}

fn user_req(text: &str) -> ChatRequest {
    ChatRequest::new(
        "",
        vec![ChatMessage {
            role: ChatRole::User,
            content: text.into(),
        }],
    )
}

fn ask(p: &Provider, req: &ChatRequest) -> (Result<ChatResponse, AiError>, String) {
    let mut seen = String::new();
    let res = p.chat_stream(req, &mut |d| seen.push_str(d));
    (res, seen)
}

fn openai_events(parts: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = parts
        .iter()
        .map(|p| format!(r#"{{"choices":[{{"delta":{{"content":"{p}"}},"finish_reason":null}}]}}"#))
        .collect();
    v.push(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#.into());
    v.push("[DONE]".into());
    v
}

fn as_refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

// ---------- OpenAI-compatible chat ----------

#[test]
fn openai_stream_delivers_deltas_and_sends_auth_and_stream_flag() {
    let srv = MockServer::start(|_| Reply::sse(&as_refs(&openai_events(&["Hel", "lo"]))));
    let p = openai_at(&format!("{}/v1/", srv.base_url));
    let mut req = user_req("hi");
    req.system = Some("be brief".into());
    let (res, seen) = ask(&p, &req);
    let out = res.expect("ok");
    assert_eq!(out.content, "Hello");
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
    assert_eq!(seen, "Hello");

    let recs = srv.recorded();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].method, "POST");
    assert_eq!(recs[0].path, "/v1/chat/completions");
    assert_eq!(
        recs[0].header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    let body = recs[0].json();
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["model"], "gpt-test",
        "empty req.model falls back to config model"
    );
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["content"], "hi");
}

#[test]
fn openai_accepts_plain_json_when_server_ignores_stream() {
    let srv = MockServer::start(|_| {
        Reply::json(
            200,
            r#"{"choices":[{"message":{"role":"assistant","content":"whole answer"},"finish_reason":"length"}]}"#,
        )
    });
    let p = openai_at(&srv.base_url);
    let (res, seen) = ask(&p, &user_req("q"));
    let out = res.expect("ok");
    assert_eq!(out.content, "whole answer");
    assert_eq!(out.finish_reason.as_deref(), Some("length"));
    assert_eq!(
        seen, "whole answer",
        "non-streamed text is still delivered as one delta"
    );
}

#[test]
fn stream_false_setting_sends_stream_false_and_parses_json() {
    let srv = MockServer::start(|_| {
        Reply::json(
            200,
            r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
        )
    });
    let mut settings = fast_settings();
    settings.stream = false;
    let p = Provider::with_settings(
        ProviderConfig {
            base_url: srv.base_url.clone(),
            model: "m".into(),
            kind: ProviderKind::OpenAiCompatible,
        },
        None,
        settings,
    )
    .expect("provider");
    let (res, _) = ask(&p, &user_req("q"));
    assert_eq!(res.expect("ok").content, "ok");
    let recs = srv.recorded();
    assert_eq!(recs[0].json()["stream"], false);
    assert!(
        recs[0].header("authorization").is_none(),
        "no key means no auth header"
    );
}

#[test]
fn stream_without_done_but_with_finish_reason_is_complete() {
    let events = [
        r#"{"choices":[{"delta":{"content":"partial"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
    ];
    let srv = MockServer::start(move |_| Reply::sse(&events));
    let (res, _) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert_eq!(res.expect("ok").content, "partial");
}

#[test]
fn stream_cut_before_finish_is_a_network_error_with_partial_text_delivered() {
    let events = [r#"{"choices":[{"delta":{"content":"abc"}}]}"#];
    let srv = MockServer::start(move |_| Reply::sse(&events));
    let (res, seen) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert!(matches!(res, Err(AiError::Network(_))), "got {res:?}");
    assert_eq!(seen, "abc");
}

#[test]
fn connection_dropped_mid_stream_is_a_network_error() {
    let events = openai_events(&["one ", "two ", "three"]);
    let full = Reply::sse(&as_refs(&events));
    let cut = full.body.len() / 2;
    let srv = MockServer::start(move |_| full.clone().with_cut_after(cut));
    let (res, seen) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert!(matches!(res, Err(AiError::Network(_))), "got {res:?}");
    assert!(!seen.is_empty() && seen.len() < "one two three".len());
}

#[test]
fn malformed_sse_json_is_bad_response_and_keeps_earlier_text() {
    let srv = MockServer::start(|_| {
        Reply::sse(&[r#"{"choices":[{"delta":{"content":"ok"}}]}"#, "{not json"])
    });
    let (res, seen) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert!(matches!(res, Err(AiError::BadResponse(_))), "got {res:?}");
    assert_eq!(seen, "ok");
}

#[test]
fn cancel_mid_stream_returns_cancelled_after_partial_delta() {
    let events = openai_events(&["a", "b", "c", "d", "e"]);
    let srv = MockServer::start(move |_| {
        Reply::sse(&as_refs(&events)).with_write_delay(Duration::from_millis(80))
    });
    let p = openai_at(&srv.base_url);
    let req = user_req("q");
    let token: CancelToken = req.cancel.clone();
    let mut deltas = 0;
    let res = p.chat_stream(&req, &mut |_| {
        deltas += 1;
        token.cancel();
    });
    assert_eq!(res, Err(AiError::Cancelled));
    assert_eq!(deltas, 1, "no further deltas after cancel");
}

#[test]
fn cancel_before_send_makes_no_request() {
    let srv = MockServer::start(|_| Reply::json(200, "{}"));
    let req = user_req("q");
    req.cancel.cancel();
    let (res, _) = ask(&openai_at(&srv.base_url), &req);
    assert_eq!(res, Err(AiError::Cancelled));
    assert!(srv.recorded().is_empty());
}

#[test]
fn cancel_while_stalled_is_honoured_quickly() {
    let srv = MockServer::start(|_| Reply::json(200, "{}").with_stall(Duration::from_secs(3)));
    let p = openai_at(&srv.base_url);
    let req = user_req("q");
    let token = req.cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        token.cancel();
    });
    let started = std::time::Instant::now();
    let (res, _) = ask(&p, &req);
    canceller.join().expect("join");
    assert_eq!(res, Err(AiError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancel took {:?}",
        started.elapsed()
    );
}

#[test]
fn idle_timeout_between_reads_is_a_network_error() {
    let srv = MockServer::start(|_| Reply::json(200, "{}").with_stall(Duration::from_millis(2500)));
    let mut settings = fast_settings();
    settings.idle_timeout = Duration::from_millis(300);
    let p = Provider::with_settings(
        ProviderConfig {
            base_url: srv.base_url.clone(),
            model: "m".into(),
            kind: ProviderKind::OpenAiCompatible,
        },
        None,
        settings,
    )
    .expect("provider");
    let started = std::time::Instant::now();
    let (res, _) = ask(&p, &user_req("q"));
    assert!(matches!(res, Err(AiError::Network(_))), "got {res:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

// ---------- status codes, retries, error mapping ----------

#[test]
fn unauthorized_is_not_retried_and_key_is_not_in_the_error() {
    let srv = MockServer::start(|_| {
        Reply::json(
            401,
            &format!(r#"{{"error":{{"message":"bad key {KEY}"}}}}"#),
        )
    });
    let (res, _) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert_eq!(res, Err(AiError::Unauthorized));
    assert_eq!(srv.recorded().len(), 1);
}

#[test]
fn rate_limit_with_retry_after_zero_is_retried_then_succeeds() {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let srv = MockServer::start(move |_| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::json(429, r#"{"error":{"message":"slow down"}}"#).with_header("Retry-After", "0")
        } else {
            Reply::json(200, r#"{"choices":[{"message":{"content":"second try"}}]}"#)
        }
    });
    let (res, _) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert_eq!(res.expect("ok").content, "second try");
    assert_eq!(srv.recorded().len(), 2);
}

#[test]
fn rate_limit_with_long_retry_after_fails_fast_with_the_delay() {
    let srv = MockServer::start(|_| {
        Reply::json(429, r#"{"error":{"message":"quota"}}"#).with_header("Retry-After", "100")
    });
    let started = std::time::Instant::now();
    let (res, _) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert_eq!(
        res,
        Err(AiError::RateLimited {
            retry_after: Some(Duration::from_secs(100))
        })
    );
    assert_eq!(
        srv.recorded().len(),
        1,
        "no retry when Retry-After exceeds max_delay"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn server_error_is_retried_then_reported_with_status() {
    let srv = MockServer::start(|_| Reply::json(500, r#"{"error":"kaput"}"#));
    let (res, _) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert_eq!(res, Err(AiError::Server(500, "kaput".into())));
    assert_eq!(srv.recorded().len(), 3, "1 attempt + 2 retries");
}

#[test]
fn client_errors_map_to_bad_request_without_retry() {
    let srv = MockServer::start(|_| {
        Reply::json(400, r#"{"error":{"message":"model gpt-x does not exist"}}"#)
    });
    let (res, _) = ask(&openai_at(&srv.base_url), &user_req("q"));
    assert_eq!(
        res,
        Err(AiError::BadRequest("model gpt-x does not exist".into()))
    );
    assert_eq!(srv.recorded().len(), 1);
}

#[test]
fn error_body_that_echoes_the_key_is_redacted() {
    let srv =
        MockServer::start(|_| Reply::json(400, &format!("plain text containing {KEY} at the end")));
    let (res, _) = ask(&openai_at(&srv.base_url), &user_req("q"));
    let msg = res.expect_err("error").to_string();
    assert!(!msg.contains(KEY), "key leaked: {msg}");
    assert!(msg.contains("[redacted]"));
}

#[test]
fn connection_refused_is_retried_and_reported_as_network_error() {
    // Bind then drop to obtain a port nobody listens on.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    };
    let p = openai_at(&format!("http://127.0.0.1:{port}"));
    let (res, _) = ask(&p, &user_req("q"));
    assert!(matches!(res, Err(AiError::Network(_))), "got {res:?}");
}

#[test]
fn provider_debug_and_errors_never_show_the_key() {
    let p = openai_at("http://127.0.0.1:9");
    let dbg = format!("{p:?}");
    assert!(!dbg.contains(KEY));
    assert!(dbg.contains("has_api_key: true"));
}

#[test]
fn base_url_must_be_http_or_https() {
    let res = Provider::new(
        ProviderConfig {
            base_url: "ftp://x".into(),
            model: "m".into(),
            kind: ProviderKind::OpenAiCompatible,
        },
        None,
    );
    assert!(matches!(res, Err(AiError::Other(_))));
}

#[test]
fn empty_key_is_treated_as_no_key() {
    let p = Provider::new(
        ProviderConfig {
            base_url: "http://127.0.0.1:9".into(),
            model: "m".into(),
            kind: ProviderKind::OpenAiCompatible,
        },
        Some("   ".into()),
    )
    .expect("provider");
    assert!(!p.has_api_key());
}

// ---------- Anthropic ----------

#[test]
fn anthropic_stream_uses_headers_system_field_and_sse_events() {
    let events = [
        r#"{"type":"message_start","message":{"id":"m"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Bon"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"jour"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        r#"{"type":"message_stop"}"#,
    ];
    let srv = MockServer::start(move |_| Reply::sse(&events));
    let p = anthropic_at(&srv.base_url);
    let mut req = user_req("salut");
    req.system = Some("sys".into());
    let (res, seen) = ask(&p, &req);
    let out = res.expect("ok");
    assert_eq!(out.content, "Bonjour");
    assert_eq!(out.finish_reason.as_deref(), Some("end_turn"));
    assert_eq!(seen, "Bonjour");

    let recs = srv.recorded();
    assert_eq!(recs[0].path, "/v1/messages");
    assert_eq!(recs[0].header("x-api-key"), Some(KEY));
    assert_eq!(recs[0].header("anthropic-version"), Some("2023-06-01"));
    assert!(recs[0].header("authorization").is_none());
    let body = recs[0].json();
    assert_eq!(body["system"], "sys");
    assert_eq!(body["max_tokens"], 4096);
    assert_eq!(body["stream"], true);
}

#[test]
fn anthropic_base_url_already_ending_in_v1_is_not_doubled() {
    let srv = MockServer::start(|_| {
        Reply::json(
            200,
            r#"{"content":[{"type":"text","text":"yo"}],"stop_reason":"end_turn"}"#,
        )
    });
    let p = anthropic_at(&format!("{}/v1", srv.base_url));
    let (res, _) = ask(&p, &user_req("q"));
    assert_eq!(res.expect("ok").content, "yo");
    assert_eq!(srv.recorded()[0].path, "/v1/messages");
}

#[test]
fn anthropic_error_status_is_mapped_and_redacted() {
    let srv = MockServer::start(|_| {
        Reply::json(
            401,
            &format!(
                r#"{{"type":"error","error":{{"type":"authentication_error","message":"invalid {KEY}"}}}}"#
            ),
        )
    });
    let (res, _) = ask(&anthropic_at(&srv.base_url), &user_req("q"));
    assert_eq!(res, Err(AiError::Unauthorized));
}

#[test]
fn anthropic_error_event_in_stream_is_reported() {
    let srv = MockServer::start(|_| {
        Reply::sse(&[r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#])
    });
    let (res, _) = ask(&anthropic_at(&srv.base_url), &user_req("q"));
    assert!(matches!(res, Err(AiError::BadResponse(m)) if m.contains("busy")));
}

#[test]
fn anthropic_has_no_embeddings() {
    let p = anthropic_at("http://127.0.0.1:9");
    assert!(matches!(p.embed(&["x".into()]), Err(AiError::Other(_))));
}

// ---------- embeddings ----------

#[test]
fn embeddings_post_to_v1_embeddings_and_parse_vectors() {
    let srv = MockServer::start(|_| {
        Reply::json(
            200,
            r#"{"data":[{"index":0,"embedding":[0.5,0.25]},{"index":1,"embedding":[1,2]}]}"#,
        )
    });
    let p = openai_at(&format!("{}/v1", srv.base_url));
    let out = p
        .embed(&["alpha".to_string(), "beta".to_string()])
        .expect("embeddings");
    assert_eq!(out, vec![vec![0.5, 0.25], vec![1.0, 2.0]]);
    let recs = srv.recorded();
    assert_eq!(recs[0].path, "/v1/embeddings");
    assert_eq!(recs[0].json()["model"], "gpt-test");
    assert_eq!(recs[0].json()["input"][1], "beta");
}

#[test]
fn embeddings_empty_input_makes_no_request() {
    let srv = MockServer::start(|_| Reply::json(200, "{}"));
    let out = openai_at(&srv.base_url).embed(&[]).expect("ok");
    assert!(out.is_empty());
    assert!(srv.recorded().is_empty());
}

#[test]
fn embeddings_server_error_is_retried_and_mapped() {
    let srv = MockServer::start(|_| Reply::json(503, r#"{"error":{"message":"loading"}}"#));
    let res = openai_at(&srv.base_url).embed(&["x".into()]);
    assert_eq!(res, Err(AiError::Server(503, "loading".into())));
    assert_eq!(srv.recorded().len(), 3);
}

#[test]
fn embeddings_bad_payload_is_bad_response() {
    let srv = MockServer::start(|_| Reply::json(200, r#"{"data":[]}"#));
    let res = openai_at(&srv.base_url).embed(&["x".into()]);
    assert!(matches!(res, Err(AiError::BadResponse(_))));
}

/// Live check against a real OpenAI-compatible endpoint. Skipped unless the environment provides
/// `JIYUE_TEST_OPENAI_BASE`, `JIYUE_TEST_OPENAI_KEY` and `JIYUE_TEST_OPENAI_MODEL`. Optional:
/// `JIYUE_TEST_OPENAI_EMBED_MODEL` also exercises embeddings. Nothing is hard-coded and the key is
/// never printed.
/// Run with: `cargo test -p jiyue-ai -- --ignored live_openai`
#[test]
#[ignore]
fn live_openai_compatible_chat_and_embeddings() {
    let (Ok(base), Ok(key), Ok(model)) = (
        std::env::var("JIYUE_TEST_OPENAI_BASE"),
        std::env::var("JIYUE_TEST_OPENAI_KEY"),
        std::env::var("JIYUE_TEST_OPENAI_MODEL"),
    ) else {
        eprintln!("skipped: JIYUE_TEST_OPENAI_BASE / _KEY / _MODEL are not set");
        return;
    };
    let provider = Provider::new(
        ProviderConfig {
            base_url: base,
            model,
            kind: ProviderKind::OpenAiCompatible,
        },
        Some(key),
    )
    .expect("provider");
    let req = ChatRequest::new(
        "",
        vec![ChatMessage {
            role: ChatRole::User,
            content: "Reply with the single word: pong".into(),
        }],
    );
    let (res, seen) = ask(&provider, &req);
    let out = res.expect("live chat succeeds");
    // Model name and reasoning presence only; the key and the answer text are not printed.
    eprintln!(
        "live: answered by model {:?} (configured name is the one in JIYUE_TEST_OPENAI_MODEL), reasoning present: {}",
        out.model,
        out.reasoning.is_some()
    );
    assert!(!out.content.trim().is_empty() || out.is_reasoning_only());
    assert_eq!(seen, out.content);

    if let Ok(embed_model) = std::env::var("JIYUE_TEST_OPENAI_EMBED_MODEL") {
        let embedder = Provider::new(
            ProviderConfig {
                base_url: provider.config().base_url.clone(),
                model: embed_model,
                kind: ProviderKind::OpenAiCompatible,
            },
            std::env::var("JIYUE_TEST_OPENAI_KEY").ok(),
        )
        .expect("embedder");
        let vectors = embedder
            .embed(&["hello".to_string(), "world".to_string()])
            .expect("live embeddings succeed");
        assert_eq!(vectors.len(), 2);
        assert!(!vectors[0].is_empty());
    }
}
