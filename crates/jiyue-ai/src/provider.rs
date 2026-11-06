//! HTTP providers: OpenAI-compatible and Anthropic chat, OpenAI-compatible embeddings.
//!
//! The public API is blocking. Each call runs on a shared multi-thread tokio runtime owned by this
//! crate, so callers only need a plain background thread. Timeouts: connect 10 s and 60 s idle
//! between reads (both configurable through `HttpSettings`). Retries (exponential backoff,
//! `Retry-After` honoured) happen only before any response body bytes arrive. Cancelling is
//! checked between reads, at least every 200 ms even when the server stalls.

mod anthropic;
mod openai;
mod sse;

use std::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde_json::Value;

use crate::util::{redact, truncate_chars};
use crate::{
    AiError, CancelToken, ChatProvider, ChatRequest, ChatResponse, ChatRole, EmbeddingProvider,
    ProviderConfig, ProviderKind,
};

/// How many times and how long to back off before giving up on a retryable failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt. 0 disables retries.
    pub max_retries: u32,
    /// First backoff delay; doubles per attempt.
    pub base_delay: Duration,
    /// Upper bound for one backoff. A `Retry-After` longer than this is not waited for; the
    /// error is returned immediately instead.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(8),
        }
    }
}

/// Network behaviour of a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpSettings {
    pub connect_timeout: Duration,
    /// Longest silence allowed between two reads from the server.
    pub idle_timeout: Duration,
    pub retry: RetryPolicy,
    /// Ask for a streamed response. When the server answers with plain JSON anyway, that is
    /// accepted too. Set to false to skip streaming entirely.
    pub stream: bool,
}

impl Default for HttpSettings {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(60),
            retry: RetryPolicy::default(),
            stream: true,
        }
    }
}

const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 4096;
/// How often a blocked read wakes up to look at the cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(200);
/// Largest response body accepted for non-streamed answers and embeddings.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
/// Error bodies are shown to the user (after redaction), so keep them short.
const ERROR_TEXT_MAX_CHARS: usize = 300;

static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();

fn runtime() -> Result<&'static tokio::runtime::Runtime, AiError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(AiError::Other(
            "jiyue-ai calls are blocking; run them on a plain background thread".into(),
        ));
    }
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("jiyue-ai")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| AiError::Other(format!("cannot start the AI runtime: {e}")))
}

/// API key wrapper whose Debug output never shows the value.
#[derive(Clone)]
struct ApiKey(String);

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

/// A chat and embedding client for one configured provider.
#[derive(Clone)]
pub struct Provider {
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    settings: HttpSettings,
    client: reqwest::Client,
}

impl fmt::Debug for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provider")
            .field("kind", &self.cfg.kind)
            .field("base_url", &self.cfg.base_url)
            .field("model", &self.cfg.model)
            .field("has_api_key", &self.key.is_some())
            .field("settings", &self.settings)
            .finish()
    }
}

impl Provider {
    /// Creates a provider with default network settings. An empty key is treated as no key
    /// (useful for local servers such as Ollama).
    pub fn new(config: ProviderConfig, api_key: Option<String>) -> Result<Self, AiError> {
        Self::with_settings(config, api_key, HttpSettings::default())
    }

    pub fn with_settings(
        config: ProviderConfig,
        api_key: Option<String>,
        settings: HttpSettings,
    ) -> Result<Self, AiError> {
        let base = config.base_url.trim();
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            return Err(AiError::Other(
                "the base URL must start with https:// or http://".into(),
            ));
        }
        let key = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .map(ApiKey);
        let client = reqwest::Client::builder()
            .connect_timeout(settings.connect_timeout)
            .read_timeout(settings.idle_timeout)
            .build()
            .map_err(|e| AiError::Other(format!("cannot create the HTTP client: {e}")))?;
        Ok(Self {
            cfg: ProviderConfig {
                base_url: base.trim_end_matches('/').to_string(),
                ..config
            },
            key,
            settings,
            client,
        })
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.cfg
    }

    pub fn has_api_key(&self) -> bool {
        self.key.is_some()
    }

    fn chat_url(&self) -> String {
        match self.cfg.kind {
            ProviderKind::OpenAiCompatible => format!("{}/chat/completions", self.cfg.base_url),
            ProviderKind::Anthropic => {
                if self.cfg.base_url.ends_with("/v1") {
                    format!("{}/messages", self.cfg.base_url)
                } else {
                    format!("{}/v1/messages", self.cfg.base_url)
                }
            }
        }
    }

    fn embeddings_url(&self) -> String {
        format!("{}/embeddings", self.cfg.base_url)
    }

    /// Adds authentication headers for the provider's wire protocol.
    fn authorize(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = match self.cfg.kind {
            ProviderKind::OpenAiCompatible => match &self.key {
                Some(k) => rb.bearer_auth(&k.0),
                None => rb,
            },
            ProviderKind::Anthropic => {
                let rb = rb.header("anthropic-version", ANTHROPIC_VERSION);
                match &self.key {
                    Some(k) => rb.header("x-api-key", &k.0),
                    None => rb,
                }
            }
        };
        rb.header(reqwest::header::CONTENT_TYPE, "application/json")
    }

    fn key_str(&self) -> Option<&str> {
        self.key.as_ref().map(|k| k.0.as_str())
    }

    /// Removes the API key from any message text before it leaves this crate.
    fn scrub(&self, err: AiError) -> AiError {
        let key = self.key_str();
        let clean = |s: String| redact(&s, key);
        match err {
            AiError::Network(s) => AiError::Network(clean(s)),
            AiError::BadRequest(s) => AiError::BadRequest(clean(s)),
            AiError::Server(code, s) => AiError::Server(code, clean(s)),
            AiError::BadResponse(s) => AiError::BadResponse(clean(s)),
            AiError::Other(s) => AiError::Other(clean(s)),
            other => other,
        }
    }

    /// Sends one POST, retrying retryable failures before any body is read.
    async fn post_with_retry(
        &self,
        url: &str,
        body: &[u8],
        accept: &str,
        cancel: &CancelToken,
    ) -> Result<reqwest::Response, AiError> {
        let policy = self.settings.retry;
        let mut attempt: u32 = 0;
        loop {
            check_cancel(cancel)?;
            let request = self.authorize(
                self.client
                    .post(url)
                    .header(reqwest::header::ACCEPT, accept)
                    .body(body.to_vec()),
            );
            let outcome = until_cancelled(request.send(), cancel).await?;
            let (err, retry_after) = match outcome {
                Ok(resp) if resp.status().is_success() => return Ok(resp),
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let retry_after = parse_retry_after(resp.headers());
                    let text = read_body(resp, cancel)
                        .await
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default();
                    (
                        map_status(status, &text, self.key_str(), retry_after),
                        retry_after,
                    )
                }
                Err(e) => (AiError::Network(describe_error(&e)), None),
            };
            if !is_retryable(&err) || attempt >= policy.max_retries {
                return Err(self.scrub(err));
            }
            let delay = match retry_after {
                Some(d) => {
                    if d > policy.max_delay {
                        return Err(self.scrub(err));
                    }
                    d
                }
                None => backoff(policy, attempt),
            };
            sleep_cancellable(delay, cancel).await?;
            attempt += 1;
        }
    }

    async fn chat_async(
        &self,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, AiError> {
        let stream = self.settings.stream;
        let model = if req.model.trim().is_empty() {
            self.cfg.model.as_str()
        } else {
            req.model.as_str()
        };
        let body = match self.cfg.kind {
            ProviderKind::OpenAiCompatible => openai::chat_body(model, req, stream),
            ProviderKind::Anthropic => {
                anthropic::chat_body(model, req, stream, ANTHROPIC_DEFAULT_MAX_TOKENS)?
            }
        };
        let bytes = serde_json::to_vec(&body)
            .map_err(|e| AiError::Other(format!("cannot encode the request: {e}")))?;
        let accept = if stream {
            "text/event-stream, application/json"
        } else {
            "application/json"
        };
        let url = self.chat_url();
        let resp = self
            .post_with_retry(&url, &bytes, accept, &req.cancel)
            .await?;

        let is_sse = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.trim_start().starts_with("text/event-stream"));

        // An event-stream answer is read as a stream even when `stream` was turned off: some
        // gateways ignore the flag and always stream.
        let result = if is_sse {
            self.consume_stream(resp, req.cancel.clone(), on_delta)
                .await
        } else {
            let bytes = read_body(resp, &req.cancel).await?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            // A sniffed event stream has already delivered its deltas while it was parsed.
            let sniffed = looks_like_sse(&text);
            let full = if sniffed {
                parse_sse_body(self.cfg.kind, &text, on_delta)
            } else {
                match self.cfg.kind {
                    ProviderKind::OpenAiCompatible => openai::parse_full(&text),
                    ProviderKind::Anthropic => anthropic::parse_full(&text),
                }
            }?;
            if !sniffed && !full.content.is_empty() {
                on_delta(&full.content);
            }
            Ok(full)
        };
        result.map_err(|e| self.scrub(e))
    }

    async fn consume_stream(
        &self,
        mut resp: reqwest::Response,
        cancel: CancelToken,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, AiError> {
        let mut decoder = sse::SseDecoder::default();
        let mut state = match self.cfg.kind {
            ProviderKind::OpenAiCompatible => StreamKind::OpenAi(openai::StreamState::default()),
            ProviderKind::Anthropic => StreamKind::Anthropic(anthropic::StreamState::default()),
        };
        let mut done = false;
        while !done {
            check_cancel(&cancel)?;
            match tokio::time::timeout(CANCEL_POLL, resp.chunk()).await {
                // Nothing arrived within the poll window; loop around to check the cancel flag.
                // The idle timeout itself is enforced by the HTTP client.
                Err(_) => continue,
                Ok(Ok(Some(bytes))) => {
                    for data in decoder.push(&bytes) {
                        if state.on_data(&data, on_delta)? {
                            done = true;
                            break;
                        }
                    }
                }
                Ok(Ok(None)) => {
                    for data in decoder.finish() {
                        if state.on_data(&data, on_delta)? {
                            break;
                        }
                    }
                    break;
                }
                Ok(Err(e)) => {
                    return Err(AiError::Network(format!(
                        "stream interrupted: {}",
                        describe_error(&e)
                    )));
                }
            }
        }
        check_cancel(&cancel)?;
        state.finish()
    }

    async fn embed_async(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        let model = self.cfg.model.as_str();
        let body = serde_json::json!({ "model": model, "input": texts });
        let bytes = serde_json::to_vec(&body)
            .map_err(|e| AiError::Other(format!("cannot encode the request: {e}")))?;
        let cancel = CancelToken::new();
        let resp = self
            .post_with_retry(&self.embeddings_url(), &bytes, "application/json", &cancel)
            .await
            .map_err(|e| self.scrub(e))?;
        let bytes = read_body(resp, &cancel).await.map_err(|e| self.scrub(e))?;
        let text = String::from_utf8_lossy(&bytes);
        openai::parse_embeddings(&text, texts.len()).map_err(|e| self.scrub(e))
    }
}

enum StreamKind {
    OpenAi(openai::StreamState),
    Anthropic(anthropic::StreamState),
}

/// True when a body that was not sent as an event stream is one anyway (`data:` or `event:`
/// lines at its start).
fn looks_like_sse(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("data:") || t.starts_with("event:")
}

/// Parses an event-stream body that arrived without the event-stream content type.
fn parse_sse_body(
    kind: ProviderKind,
    text: &str,
    on_delta: &mut dyn FnMut(&str),
) -> Result<ChatResponse, AiError> {
    let mut decoder = sse::SseDecoder::default();
    let mut events = decoder.push(text.as_bytes());
    events.extend(decoder.finish());
    let mut state = StreamKind::new(kind);
    for data in events {
        if state.on_data(&data, on_delta)? {
            break;
        }
    }
    state.finish()
}

impl StreamKind {
    fn new(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::OpenAiCompatible => StreamKind::OpenAi(openai::StreamState::default()),
            ProviderKind::Anthropic => StreamKind::Anthropic(anthropic::StreamState::default()),
        }
    }

    fn on_data(&mut self, data: &str, on_delta: &mut dyn FnMut(&str)) -> Result<bool, AiError> {
        match self {
            StreamKind::OpenAi(s) => s.on_data(data, on_delta),
            StreamKind::Anthropic(s) => s.on_data(data, on_delta),
        }
    }

    fn finish(self) -> Result<ChatResponse, AiError> {
        match self {
            StreamKind::OpenAi(s) => s.finish(),
            StreamKind::Anthropic(s) => s.finish(),
        }
    }
}

impl ChatProvider for Provider {
    fn chat_stream(
        &self,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, AiError> {
        check_cancel(&req.cancel)?;
        runtime()?.block_on(self.chat_async(req, on_delta))
    }
}

impl EmbeddingProvider for Provider {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        if self.cfg.kind == ProviderKind::Anthropic {
            return Err(AiError::Other(
                "Anthropic has no embeddings API; configure an OpenAI-compatible embedding provider"
                    .into(),
            ));
        }
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        runtime()?.block_on(self.embed_async(texts))
    }
}

fn check_cancel(cancel: &CancelToken) -> Result<(), AiError> {
    if cancel.is_cancelled() {
        Err(AiError::Cancelled)
    } else {
        Ok(())
    }
}

/// Awaits `fut`, checking the cancel flag every `CANCEL_POLL`. The future is dropped on cancel.
async fn until_cancelled<T>(
    fut: impl std::future::Future<Output = T>,
    cancel: &CancelToken,
) -> Result<T, AiError> {
    tokio::pin!(fut);
    loop {
        match tokio::time::timeout(CANCEL_POLL, fut.as_mut()).await {
            Ok(value) => return Ok(value),
            Err(_) => check_cancel(cancel)?,
        }
    }
}

/// Reads a whole response body, cancellable, capped at `MAX_BODY_BYTES`.
async fn read_body(mut resp: reqwest::Response, cancel: &CancelToken) -> Result<Vec<u8>, AiError> {
    let mut out = Vec::new();
    loop {
        check_cancel(cancel)?;
        match tokio::time::timeout(CANCEL_POLL, resp.chunk()).await {
            Err(_) => continue,
            Ok(Ok(Some(bytes))) => {
                out.extend_from_slice(&bytes);
                if out.len() > MAX_BODY_BYTES {
                    return Err(AiError::BadResponse("response is too large".into()));
                }
            }
            Ok(Ok(None)) => return Ok(out),
            Ok(Err(e)) => return Err(AiError::Network(describe_error(&e))),
        }
    }
}

/// Sleeps for `delay`, waking every 100 ms to honour cancellation.
async fn sleep_cancellable(delay: Duration, cancel: &CancelToken) -> Result<(), AiError> {
    let until = Instant::now() + delay;
    loop {
        check_cancel(cancel)?;
        let now = Instant::now();
        if now >= until {
            return Ok(());
        }
        let slice = (until - now).min(Duration::from_millis(100));
        tokio::time::sleep(slice).await;
    }
}

fn backoff(policy: RetryPolicy, attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
    policy
        .base_delay
        .saturating_mul(factor)
        .min(policy.max_delay)
}

fn is_retryable(err: &AiError) -> bool {
    match err {
        AiError::Network(_) | AiError::RateLimited { .. } => true,
        AiError::Server(code, _) => *code >= 500,
        _ => false,
    }
}

/// Parses `Retry-After` in its delay-seconds form. HTTP-date values are ignored.
fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let raw = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    let secs: f64 = raw.parse().ok()?;
    if secs.is_finite() && (0.0..1.0e6).contains(&secs) {
        Some(Duration::from_secs_f64(secs))
    } else {
        None
    }
}

fn map_status(
    status: u16,
    body: &str,
    key: Option<&str>,
    retry_after: Option<Duration>,
) -> AiError {
    let msg = error_message(body, key);
    match status {
        401 | 403 => AiError::Unauthorized,
        429 => AiError::RateLimited { retry_after },
        400 | 404 | 405 | 409 | 413 | 422 => AiError::BadRequest(msg),
        other => AiError::Server(other, msg),
    }
}

/// Extracts a short, redacted message from an error body. Known JSON shapes (OpenAI and
/// Anthropic) are unwrapped; anything else is shown as plain text.
fn error_message(body: &str, key: Option<&str>) -> String {
    let extracted = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        let err = v.get("error");
        err.and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .or_else(|| err.and_then(Value::as_str))
            .or_else(|| v.get("message").and_then(Value::as_str))
            .map(str::to_string)
    });
    let raw = extracted.unwrap_or_else(|| body.trim().to_string());
    let text = if raw.is_empty() {
        "(empty error body)".to_string()
    } else {
        raw
    };
    truncate_chars(&redact(&text, key), ERROR_TEXT_MAX_CHARS)
}

/// Describes a reqwest error without leaking anything beyond its chain of messages.
fn describe_error(e: &reqwest::Error) -> String {
    let mut out = if e.is_timeout() {
        "timed out".to_string()
    } else if e.is_connect() {
        "could not connect".to_string()
    } else {
        "request failed".to_string()
    };
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    let mut depth = 0;
    while let Some(s) = source {
        if depth >= 3 {
            break;
        }
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
        depth += 1;
    }
    out
}

/// Maps a chat role to the lowercase string used by both wire formats.
pub(crate) fn role_str(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

#[cfg(test)]
mod reasoning_tests;
#[cfg(test)]
mod tests;
