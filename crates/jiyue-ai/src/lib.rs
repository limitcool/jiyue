//! jiyue-ai: BYOK chat and embedding providers (OpenAI-compatible and Anthropic), vector index,
//! RAG retrieval, prompt builders and the AI settings types.
//!
//! This crate does not depend on other Jiyue crates. The app passes data in through callbacks
//! (lexical search, folder/tag exclusion) so the crate stays testable on its own.
//!
//! All network calls are blocking from the caller's point of view: they run an internal tokio
//! runtime and must be called from a background thread, never from inside an async runtime.
//!
//! API keys are not stored here: the app resolves them from `credentials.toml` (see
//! `jiyue_core::config`) and passes them in per request.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub mod chunk;
pub mod config;
pub mod index;
pub mod prompts;
pub mod provider;
pub mod rag;
mod util;

#[cfg(test)]
mod testutil;

pub use chunk::{Chunk, chunk_note};
pub use config::{AiConfig, ConfigError, ProviderEntry};
pub use index::{ChunkHit, VectorIndex, embed_in_batches};
pub use provider::{HttpSettings, Provider, RetryPolicy};
pub use rag::{Context, ContextSource, LexicalHit, RetrieveCtx, retrieve};

/// Who wrote a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// One message in a chat request or response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Cooperative cancellation flag shared between the UI and a running request.
///
/// Cloning shares the flag. Equality compares identity, not the flag value.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. Idempotent.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl PartialEq for CancelToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CancelToken(cancelled={})", self.is_cancelled())
    }
}

/// A chat completion request.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Optional system prompt. Sent as a system message (OpenAI) or the `system` field
    /// (Anthropic). System messages inside `messages` are handled the same way.
    #[serde(default)]
    pub system: Option<String>,
    /// Not serialized. Checked between stream reads and before retries.
    #[serde(skip)]
    pub cancel: CancelToken,
}

impl ChatRequest {
    /// Convenience constructor with no temperature, no token cap, no system prompt and a fresh
    /// cancel token.
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            temperature: None,
            max_tokens: None,
            system: None,
            cancel: CancelToken::new(),
        }
    }
}

// Manual Debug: prompts contain note text, so only sizes are printed.
impl fmt::Debug for ChatRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let chars: usize = self
            .messages
            .iter()
            .map(|m| m.content.chars().count())
            .sum();
        f.debug_struct("ChatRequest")
            .field("model", &self.model)
            .field("messages", &self.messages.len())
            .field("message_chars", &chars)
            .field(
                "system_chars",
                &self.system.as_ref().map(|s| s.chars().count()),
            )
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

/// The final result of a chat completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatResponse {
    /// The answer text. Reasoning is never mixed into it.
    pub content: String,
    pub finish_reason: Option<String>,
    /// Reasoning text of reasoning models (`reasoning_content` / `reasoning` on OpenAI-compatible
    /// servers, `thinking` blocks on Anthropic). Shown collapsed by the UI, never sent as answer.
    #[serde(default)]
    pub reasoning: Option<String>,
    /// The model name the server reports for this answer. Gateways may alias the configured name.
    #[serde(default)]
    pub model: Option<String>,
}

impl ChatResponse {
    /// True when the server returned reasoning but no answer text (for example because the
    /// output limit ran out during reasoning). Callers must tell the user; it is not a plain
    /// success.
    pub fn is_reasoning_only(&self) -> bool {
        self.content.trim().is_empty()
            && self
                .reasoning
                .as_deref()
                .is_some_and(|r| !r.trim().is_empty())
    }
}

/// Errors from a provider. Messages never contain API keys or note text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    /// Connection, timeout or stream-level failure.
    Network(String),
    /// HTTP 401 or 403.
    Unauthorized,
    /// HTTP 429. `retry_after` comes from the `Retry-After` header when present.
    RateLimited { retry_after: Option<Duration> },
    /// HTTP 400/404/413/422 or another client-side problem; the server message (redacted).
    BadRequest(String),
    /// HTTP 5xx or other unexpected status, with the status code and server message.
    Server(u16, String),
    /// The response could not be parsed (malformed JSON or SSE, missing fields).
    BadResponse(String),
    /// The caller cancelled through `CancelToken`.
    Cancelled,
    /// Anything else (configuration problems, unsupported operations).
    Other(String),
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AiError::Network(msg) => write!(f, "network error: {msg}"),
            AiError::Unauthorized => write!(f, "unauthorized (check the API key)"),
            AiError::RateLimited {
                retry_after: Some(d),
            } => {
                write!(f, "rate limited, retry after {} s", d.as_secs())
            }
            AiError::RateLimited { retry_after: None } => write!(f, "rate limited"),
            AiError::BadRequest(msg) => write!(f, "bad request: {msg}"),
            AiError::Server(status, msg) => write!(f, "server error {status}: {msg}"),
            AiError::BadResponse(msg) => write!(f, "unexpected response: {msg}"),
            AiError::Cancelled => write!(f, "cancelled"),
            AiError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AiError {}

/// Wire protocol of a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAiCompatible,
    Anthropic,
}

/// Where and how to reach a provider. The API key is not part of this struct; it comes from the
/// secret store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    pub kind: ProviderKind,
}

/// Streaming chat provider.
pub trait ChatProvider: Send + Sync {
    /// Streams deltas through `on_delta` and returns the full response when done.
    fn chat_stream(
        &self,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, AiError>;
}

/// Text embedding provider.
pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError>;
}

/// Stand-in provider that refuses every call. Nothing is sent anywhere.
#[derive(Debug, Clone)]
pub struct UnimplementedProvider {
    pub config: ProviderConfig,
}

impl ChatProvider for UnimplementedProvider {
    fn chat_stream(
        &self,
        _req: &ChatRequest,
        _on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, AiError> {
        Err(AiError::Other("not implemented".into()))
    }
}

impl EmbeddingProvider for UnimplementedProvider {
    fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        Err(AiError::Other("not implemented".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_provider_refuses_to_call_out() {
        let provider = UnimplementedProvider {
            config: ProviderConfig {
                base_url: "https://example.invalid/v1".into(),
                model: "m".into(),
                kind: ProviderKind::OpenAiCompatible,
            },
        };
        let req = ChatRequest::new(
            provider.config.model.clone(),
            vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
        );
        let mut seen = String::new();
        let result = provider.chat_stream(&req, &mut |d| seen.push_str(d));
        assert_eq!(result, Err(AiError::Other("not implemented".into())));
        assert!(seen.is_empty());
        assert_eq!(
            provider.embed(&["x".to_string()]),
            Err(AiError::Other("not implemented".into()))
        );
    }

    #[test]
    fn cancel_token_is_shared_between_clones() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
        assert_ne!(a, CancelToken::new());
        assert_eq!(a, b);
    }

    #[test]
    fn chat_request_debug_hides_message_text() {
        let req = ChatRequest::new(
            "m",
            vec![ChatMessage {
                role: ChatRole::User,
                content: "SECRET NOTE TEXT".into(),
            }],
        );
        let dbg = format!("{req:?}");
        assert!(!dbg.contains("SECRET NOTE TEXT"));
        assert!(dbg.contains("message_chars"));
    }

    #[test]
    fn errors_display_without_debug_noise() {
        assert_eq!(AiError::Cancelled.to_string(), "cancelled");
        assert_eq!(
            AiError::Server(500, "boom".into()).to_string(),
            "server error 500: boom"
        );
        assert!(
            AiError::RateLimited {
                retry_after: Some(Duration::from_secs(3))
            }
            .to_string()
            .contains("3 s")
        );
    }
}
