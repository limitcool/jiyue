//! OpenAI-compatible wire format: `/chat/completions` (streamed or not) and `/embeddings`.

use serde_json::{Value, json};

use super::role_str;
use crate::{AiError, ChatRequest, ChatResponse};

/// Builds the request body. The system prompt, if any, goes first as a system message.
pub(crate) fn chat_body(model: &str, req: &ChatRequest, stream: bool) -> Value {
    let mut messages = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    for m in &req.messages {
        messages.push(json!({ "role": role_str(m.role), "content": m.content }));
    }
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(n) = req.max_tokens {
        body["max_tokens"] = json!(n);
    }
    body
}

/// Parses a complete (non-streamed) chat response.
pub(crate) fn parse_full(text: &str) -> Result<ChatResponse, AiError> {
    let v: Value = serde_json::from_str(text)
        .map_err(|_| AiError::BadResponse("response is not JSON".into()))?;
    if let Some(msg) = error_text(&v) {
        return Err(AiError::BadResponse(format!("provider error: {msg}")));
    }
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| AiError::BadResponse("no choices in response".into()))?;
    let content = match choice.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(_) => {
            return Err(AiError::BadResponse(
                "unexpected message content type".into(),
            ));
        }
    };
    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    let reasoning = choice
        .get("message")
        .and_then(reasoning_of)
        .filter(|r| !r.is_empty());
    Ok(ChatResponse {
        content,
        finish_reason,
        reasoning,
        model: model_of(&v),
    })
}

/// Reasoning text of a message or delta object. Reasoning models use `reasoning_content`; some
/// gateways use `reasoning`. Only plain strings count.
fn reasoning_of(obj: &Value) -> Option<String> {
    ["reasoning_content", "reasoning"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

/// The `model` field of a response or stream chunk, when it is a non-empty string.
fn model_of(v: &Value) -> Option<String> {
    v.get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
}

/// Parses `/embeddings` output. Items are ordered by their `index` field when present.
pub(crate) fn parse_embeddings(text: &str, expected: usize) -> Result<Vec<Vec<f32>>, AiError> {
    let v: Value = serde_json::from_str(text)
        .map_err(|_| AiError::BadResponse("embedding response is not JSON".into()))?;
    if let Some(msg) = error_text(&v) {
        return Err(AiError::BadResponse(format!("provider error: {msg}")));
    }
    let items = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::BadResponse("no data array in embedding response".into()))?;
    if items.len() != expected {
        return Err(AiError::BadResponse(format!(
            "expected {expected} embeddings, got {}",
            items.len()
        )));
    }
    let mut slots: Vec<Option<Vec<f32>>> = vec![None; expected];
    for (pos, item) in items.iter().enumerate() {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .map(|i| i as usize)
            .unwrap_or(pos);
        if index >= expected || slots[index].is_some() {
            return Err(AiError::BadResponse(
                "embedding index out of range or repeated".into(),
            ));
        }
        let arr = item
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or_else(|| AiError::BadResponse("embedding item has no vector".into()))?;
        let mut vec = Vec::with_capacity(arr.len());
        for x in arr {
            let f = x
                .as_f64()
                .ok_or_else(|| AiError::BadResponse("embedding value is not a number".into()))?;
            if !f.is_finite() {
                return Err(AiError::BadResponse(
                    "embedding contains a non-finite value".into(),
                ));
            }
            vec.push(f as f32);
        }
        if vec.is_empty() {
            return Err(AiError::BadResponse("embedding vector is empty".into()));
        }
        slots[index] = Some(vec);
    }
    let out: Option<Vec<Vec<f32>>> = slots.into_iter().collect();
    let out = out.ok_or_else(|| AiError::BadResponse("missing embedding index".into()))?;
    let dim = out.first().map(Vec::len).unwrap_or(0);
    if out.iter().any(|v| v.len() != dim) {
        return Err(AiError::BadResponse(
            "embeddings have different dimensions".into(),
        ));
    }
    Ok(out)
}

fn error_text(v: &Value) -> Option<String> {
    let err = v.get("error")?;
    Some(
        err.get("message")
            .and_then(Value::as_str)
            .or_else(|| err.as_str())
            .unwrap_or("unknown error")
            .to_string(),
    )
}

/// Incremental state for a streamed response.
#[derive(Debug, Default)]
pub(crate) struct StreamState {
    content: String,
    reasoning: String,
    model: Option<String>,
    finish_reason: Option<String>,
    done: bool,
}

impl StreamState {
    /// Handles one SSE payload. Returns `Ok(true)` when the stream is complete.
    pub(crate) fn on_data(
        &mut self,
        data: &str,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<bool, AiError> {
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return Ok(true);
        }
        if data.is_empty() {
            return Ok(false);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|_| AiError::BadResponse("malformed SSE event (not JSON)".into()))?;
        if let Some(msg) = error_text(&v) {
            return Err(AiError::BadResponse(format!("stream error: {msg}")));
        }
        if self.model.is_none() {
            self.model = model_of(&v);
        }
        if let Some(choice) = v.get("choices").and_then(|c| c.get(0)) {
            if let Some(piece) = choice
                .get("delta")
                .and_then(|d| d.get("content"))
                .and_then(Value::as_str)
                && !piece.is_empty()
            {
                self.content.push_str(piece);
                on_delta(piece);
            }
            // Reasoning is collected apart from the answer and not passed to `on_delta`.
            if let Some(piece) = choice.get("delta").and_then(reasoning_of) {
                self.reasoning.push_str(&piece);
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(reason.to_string());
            }
        }
        Ok(false)
    }

    /// Called when the connection ends. A stream that ended without `[DONE]` and without a
    /// `finish_reason` was cut short.
    pub(crate) fn finish(self) -> Result<ChatResponse, AiError> {
        if self.done || self.finish_reason.is_some() {
            let reasoning = (!self.reasoning.is_empty()).then_some(self.reasoning);
            Ok(ChatResponse {
                content: self.content,
                finish_reason: self.finish_reason,
                reasoning,
                model: self.model,
            })
        } else {
            Err(AiError::Network(
                "connection closed before the response finished".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, ChatRole};

    fn req(system: Option<&str>) -> ChatRequest {
        let mut r = ChatRequest::new(
            "m",
            vec![
                ChatMessage {
                    role: ChatRole::User,
                    content: "hi".into(),
                },
                ChatMessage {
                    role: ChatRole::Assistant,
                    content: "yo".into(),
                },
            ],
        );
        r.system = system.map(str::to_string);
        r
    }

    #[test]
    fn body_puts_system_first_and_sets_stream() {
        let body = chat_body("gpt", &req(Some("be brief")), true);
        assert_eq!(body["model"], "gpt");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "be brief");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn body_skips_blank_system_and_includes_optional_fields() {
        let mut r = req(Some("   "));
        r.temperature = Some(0.5);
        r.max_tokens = Some(64);
        let body = chat_body("m", &r, false);
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(2));
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_tokens"], 64);
        assert_eq!(body["temperature"], json!(0.5));
    }

    #[test]
    fn parse_full_reads_content_and_finish() {
        let r = parse_full(
            r#"{"choices":[{"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#,
        )
        .expect("parses");
        assert_eq!(r.content, "hello");
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn parse_full_accepts_null_content_and_reports_errors() {
        let r = parse_full(r#"{"choices":[{"message":{"content":null}}]}"#).expect("parses");
        assert_eq!(r.content, "");
        assert!(matches!(
            parse_full(r#"{"error":{"message":"model not found"}}"#),
            Err(AiError::BadResponse(m)) if m.contains("model not found")
        ));
        assert!(matches!(parse_full("<html>"), Err(AiError::BadResponse(_))));
        assert!(matches!(
            parse_full(r#"{"choices":[]}"#),
            Err(AiError::BadResponse(_))
        ));
    }

    #[test]
    fn stream_collects_deltas_until_done() {
        let mut st = StreamState::default();
        let mut seen = String::new();
        let mut sink = |d: &str| seen.push_str(d);
        assert!(
            !st.on_data(r#"{"choices":[{"delta":{"content":"He"}}]}"#, &mut sink)
                .unwrap()
        );
        assert!(
            !st.on_data(r#"{"choices":[{"delta":{"content":"llo"}}]}"#, &mut sink)
                .unwrap()
        );
        assert!(
            !st.on_data(
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
                &mut sink
            )
            .unwrap()
        );
        assert!(st.on_data("[DONE]", &mut sink).unwrap());
        let out = st.finish().expect("finished");
        assert_eq!(out.content, "Hello");
        assert_eq!(out.finish_reason.as_deref(), Some("stop"));
        assert_eq!(seen, "Hello");
    }

    #[test]
    fn stream_without_terminator_is_an_error() {
        let mut st = StreamState::default();
        let mut sink = |_: &str| {};
        st.on_data(r#"{"choices":[{"delta":{"content":"x"}}]}"#, &mut sink)
            .unwrap();
        assert!(matches!(st.finish(), Err(AiError::Network(_))));
    }

    #[test]
    fn stream_rejects_malformed_json_and_error_events() {
        let mut st = StreamState::default();
        let mut sink = |_: &str| {};
        assert!(matches!(
            st.on_data("{not json", &mut sink),
            Err(AiError::BadResponse(_))
        ));
        assert!(matches!(
            st.on_data(r#"{"error":{"message":"rate"}}"#, &mut sink),
            Err(AiError::BadResponse(m)) if m.contains("rate")
        ));
    }

    #[test]
    fn embeddings_are_ordered_by_index() {
        let text = r#"{"data":[
            {"index":1,"embedding":[0.0,1.0]},
            {"index":0,"embedding":[1.0,0.0]}]}"#;
        let v = parse_embeddings(text, 2).expect("parses");
        assert_eq!(v, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    }

    #[test]
    fn embeddings_validate_count_dims_and_values() {
        assert!(parse_embeddings(r#"{"data":[]}"#, 1).is_err());
        assert!(
            parse_embeddings(
                r#"{"data":[{"embedding":[1.0]},{"embedding":[1.0,2.0]}]}"#,
                2
            )
            .is_err()
        );
        assert!(parse_embeddings(r#"{"data":[{"embedding":["x"]}]}"#, 1).is_err());
        assert!(
            parse_embeddings(
                r#"{"data":[{"index":0,"embedding":[1.0]},{"index":0,"embedding":[1.0]}]}"#,
                2
            )
            .is_err()
        );
        assert!(parse_embeddings(r#"{"data":[{"embedding":[]}]}"#, 1).is_err());
        assert!(parse_embeddings("nope", 1).is_err());
    }
}
