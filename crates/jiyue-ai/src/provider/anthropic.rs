//! Anthropic Messages wire format: `/v1/messages`, streamed (SSE events) or plain JSON.

use serde_json::{Value, json};

use super::role_str;
use crate::{AiError, ChatRequest, ChatResponse, ChatRole};

/// Builds the request body. System messages (from `messages` or `system`) are merged into the
/// top-level `system` field because the Messages API has no system role.
pub(crate) fn chat_body(
    model: &str,
    req: &ChatRequest,
    stream: bool,
    default_max_tokens: u32,
) -> Result<Value, AiError> {
    let mut system_parts: Vec<&str> = Vec::new();
    if let Some(s) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
        system_parts.push(s);
    }
    let mut messages = Vec::new();
    for m in &req.messages {
        match m.role {
            ChatRole::System => {
                if !m.content.trim().is_empty() {
                    system_parts.push(&m.content);
                }
            }
            ChatRole::User | ChatRole::Assistant => {
                messages.push(json!({ "role": role_str(m.role), "content": m.content }));
            }
        }
    }
    if messages.is_empty() {
        return Err(AiError::BadRequest(
            "the request has no user message".into(),
        ));
    }
    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens.unwrap_or(default_max_tokens),
        "messages": messages,
        "stream": stream,
    });
    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    Ok(body)
}

/// Parses a complete (non-streamed) Messages response.
pub(crate) fn parse_full(text: &str) -> Result<ChatResponse, AiError> {
    let v: Value = serde_json::from_str(text)
        .map_err(|_| AiError::BadResponse("response is not JSON".into()))?;
    if v.get("type").and_then(Value::as_str) == Some("error") {
        return Err(AiError::BadResponse(format!(
            "provider error: {}",
            error_message(&v)
        )));
    }
    let blocks = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::BadResponse("no content in response".into()))?;
    let content: String = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    // Extended thinking arrives as `thinking` blocks; they are kept apart from the answer.
    let reasoning: String = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("thinking"))
        .filter_map(|b| b.get("thinking").and_then(Value::as_str))
        .collect();
    let finish_reason = v
        .get("stop_reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ChatResponse {
        content,
        finish_reason,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        model: v.get("model").and_then(Value::as_str).map(str::to_string),
    })
}

fn error_message(v: &Value) -> String {
    v.get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("unknown error")
        .to_string()
}

/// Incremental state for a streamed Messages response.
#[derive(Debug, Default)]
pub(crate) struct StreamState {
    content: String,
    reasoning: String,
    model: Option<String>,
    stop_reason: Option<String>,
    done: bool,
}

impl StreamState {
    /// Handles one SSE payload. Returns `Ok(true)` on `message_stop`.
    pub(crate) fn on_data(
        &mut self,
        data: &str,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<bool, AiError> {
        let data = data.trim();
        if data.is_empty() {
            return Ok(false);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|_| AiError::BadResponse("malformed SSE event (not JSON)".into()))?;
        match v.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if self.model.is_none() {
                    self.model = v
                        .get("message")
                        .and_then(|m| m.get("model"))
                        .and_then(Value::as_str)
                        .filter(|m| !m.is_empty())
                        .map(str::to_string);
                }
                Ok(false)
            }
            Some("content_block_delta") => {
                let delta = v.get("delta");
                let kind = delta.and_then(|d| d.get("type")).and_then(Value::as_str);
                match kind {
                    Some("text_delta") => {
                        if let Some(piece) =
                            delta.and_then(|d| d.get("text")).and_then(Value::as_str)
                            && !piece.is_empty()
                        {
                            self.content.push_str(piece);
                            on_delta(piece);
                        }
                    }
                    // Thinking is collected apart and never passed to `on_delta`.
                    Some("thinking_delta") => {
                        if let Some(piece) = delta
                            .and_then(|d| d.get("thinking"))
                            .and_then(Value::as_str)
                        {
                            self.reasoning.push_str(piece);
                        }
                    }
                    _ => {}
                }
                Ok(false)
            }
            Some("message_delta") => {
                if let Some(reason) = v
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop_reason = Some(reason.to_string());
                }
                Ok(false)
            }
            Some("message_stop") => {
                self.done = true;
                Ok(true)
            }
            Some("error") => Err(AiError::BadResponse(format!(
                "stream error: {}",
                error_message(&v)
            ))),
            // ping, message_start, content_block_start/stop and unknown future events.
            _ => Ok(false),
        }
    }

    pub(crate) fn finish(self) -> Result<ChatResponse, AiError> {
        if self.done {
            Ok(ChatResponse {
                content: self.content,
                finish_reason: self.stop_reason,
                reasoning: (!self.reasoning.is_empty()).then_some(self.reasoning),
                model: self.model,
            })
        } else {
            Err(AiError::Network(
                "connection closed before the message finished".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessage;

    fn msgs(system_msg: bool) -> ChatRequest {
        let mut v = Vec::new();
        if system_msg {
            v.push(ChatMessage {
                role: ChatRole::System,
                content: "from message".into(),
            });
        }
        v.push(ChatMessage {
            role: ChatRole::User,
            content: "question".into(),
        });
        ChatRequest::new("claude", v)
    }

    #[test]
    fn body_moves_system_out_of_messages() {
        let mut r = msgs(true);
        r.system = Some("from field".into());
        let body = chat_body("claude", &r, true, 1024).expect("body");
        assert_eq!(body["system"], "from field\n\nfrom message");
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn body_respects_max_tokens_and_temperature_and_requires_user_message() {
        let mut r = msgs(false);
        r.max_tokens = Some(77);
        r.temperature = Some(0.2);
        let body = chat_body("c", &r, false, 1024).expect("body");
        assert_eq!(body["max_tokens"], 77);
        assert_eq!(body["temperature"], json!(0.2f32));
        assert!(body.get("system").is_none());

        let only_system = ChatRequest::new(
            "c",
            vec![ChatMessage {
                role: ChatRole::System,
                content: "x".into(),
            }],
        );
        assert!(matches!(
            chat_body("c", &only_system, true, 1024),
            Err(AiError::BadRequest(_))
        ));
    }

    #[test]
    fn parse_full_joins_text_blocks() {
        let text = r#"{"type":"message","content":[{"type":"text","text":"Hel"},{"type":"tool_use"},{"type":"text","text":"lo"}],"stop_reason":"end_turn"}"#;
        let r = parse_full(text).expect("parses");
        assert_eq!(r.content, "Hello");
        assert_eq!(r.finish_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn parse_full_reports_error_objects() {
        let err =
            parse_full(r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#);
        assert!(matches!(err, Err(AiError::BadResponse(m)) if m.contains("busy")));
        assert!(matches!(parse_full("[]"), Err(AiError::BadResponse(_))));
    }

    #[test]
    fn stream_collects_text_deltas_until_message_stop() {
        let events = [
            r#"{"type":"message_start","message":{"id":"x"}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi "}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"there"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        let mut st = StreamState::default();
        let mut seen = String::new();
        let mut sink = |d: &str| seen.push_str(d);
        let mut done = false;
        for ev in events {
            done = st.on_data(ev, &mut sink).expect("event ok");
            if done {
                break;
            }
        }
        assert!(done);
        let out = st.finish().expect("finished");
        assert_eq!(out.content, "Hi there");
        assert_eq!(out.finish_reason.as_deref(), Some("end_turn"));
        assert_eq!(seen, "Hi there");
    }

    #[test]
    fn stream_error_event_and_missing_stop_are_errors() {
        let mut st = StreamState::default();
        let mut sink = |_: &str| {};
        assert!(matches!(
            st.on_data(r#"{"type":"error","error":{"type":"x","message":"nope"}}"#, &mut sink),
            Err(AiError::BadResponse(m)) if m.contains("nope")
        ));
        st.on_data(
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"a"}}"#,
            &mut sink,
        )
        .expect("delta");
        assert!(matches!(st.finish(), Err(AiError::Network(_))));
        let mut st2 = StreamState::default();
        assert!(matches!(
            st2.on_data("{oops", &mut sink),
            Err(AiError::BadResponse(_))
        ));
    }
}
