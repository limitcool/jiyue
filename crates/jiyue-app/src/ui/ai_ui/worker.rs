//! Background work for the AI features: chat streaming, question retrieval, connection tests and
//! the key lookup. Every function here blocks, so it runs on a plain thread (see `spawn`).
//!
//! The API key is read here, on the worker thread, and never stored in the UI state.

use std::path::Path;
use std::sync::mpsc::{Sender, channel};
use std::thread;

use jiyue_ai::prompts::Prompt;
use jiyue_ai::{
    AiError, CancelToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    Context as RagContext, EmbeddingProvider, Provider, ProviderEntry, RetrieveCtx, VectorIndex,
    retrieve,
};

use super::context::{self, NoteSnap};
use super::errors;

/// Id of the environment-defined test provider. Never written to `ai.json`.
pub const TEST_ID: &str = "test-env";
/// Name of the test provider in the UI.
pub const TEST_NAME: &str = "测试";

/// Candidates per ranking and results kept for a question.
const TOP_K: usize = 6;
const CANDIDATES: usize = 12;

/// Provider configured through `JIYUE_TEST_OPENAI_*` (debug builds only).
#[derive(Clone)]
pub struct TestEnv {
    pub base: String,
    pub key: String,
    pub model: String,
    pub embed: Option<String>,
}

// Manual Debug: the key must never be printed.
impl std::fmt::Debug for TestEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestEnv")
            .field("base", &self.base)
            .field("model", &self.model)
            .field("embed", &self.embed)
            .finish_non_exhaustive()
    }
}

/// Reads the test override. Only active in debug builds; release builds always return `None`.
pub fn test_env_from_process() -> Option<TestEnv> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let base = std::env::var("JIYUE_TEST_OPENAI_BASE").ok()?;
    let key = std::env::var("JIYUE_TEST_OPENAI_KEY").ok()?;
    let model = std::env::var("JIYUE_TEST_OPENAI_MODEL").unwrap_or_else(|_| "test-chat".into());
    let embed = std::env::var("JIYUE_TEST_OPENAI_EMBED")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let base = base.trim().to_string();
    let key = key.trim().to_string();
    if base.is_empty() || key.is_empty() {
        return None;
    }
    Some(TestEnv {
        base,
        key,
        model,
        embed,
    })
}

/// The provider as the test override describes it.
pub fn test_entry(env: &TestEnv) -> ProviderEntry {
    ProviderEntry {
        id: TEST_ID.into(),
        name: TEST_NAME.into(),
        kind: jiyue_ai::ProviderKind::OpenAiCompatible,
        base_url: env.base.clone(),
        chat_model: env.model.clone(),
        embedding_model: env.embed.clone(),
    }
}

/// The API key for a provider: the test override, or the value in `credentials.toml`, resolved
/// now (`env:` and `cmd:` forms run here). Errors name the provider, never the value.
pub fn api_key(id: &str, test: Option<&TestEnv>) -> Result<Option<String>, AiError> {
    if id == TEST_ID {
        return Ok(test.map(|t| t.key.clone()));
    }
    let loaded = jiyue_core::config::credentials::load();
    if let Some(issue) = loaded.error {
        return Err(AiError::Other(format!("凭据文件有错误：{issue}")));
    }
    let Some(entry) = loaded.value.ai.get(id) else {
        return Ok(None);
    };
    if entry.api_key.trim().is_empty() {
        return Ok(None);
    }
    jiyue_core::config::secret::resolve_secret(&entry.api_key)
        .map(|s| Some(s.expose().to_string()))
        .map_err(|e| AiError::Other(format!("服务「{id}」的密钥无法读取：{e}")))
}

/// Whether a key is stored for `id` (never reads the key into the UI).
pub fn key_saved(id: &str, test: Option<&TestEnv>) -> bool {
    matches!(api_key(id, test), Ok(Some(_)))
}

/// How the stored key of `id` is given (literal, `env:`, `cmd:`); never the value.
pub fn key_display(id: &str) -> crate::ui::config_ui::SecretDisplay {
    let loaded = jiyue_core::config::credentials::load();
    crate::ui::config_ui::display_of(loaded.value.ai.get(id).map(|e| e.api_key.as_str()))
}

/// Messages of one streamed chat request.
pub enum ChatMsg {
    Delta(String),
    Done(Result<ChatResponse, AiError>),
}

/// Starts a streamed chat on a new thread. Deltas and the final result arrive on the receiver.
pub fn spawn_chat(
    entry: ProviderEntry,
    test: Option<TestEnv>,
    messages: Vec<ChatMessage>,
    cancel: CancelToken,
) -> std::sync::mpsc::Receiver<ChatMsg> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let result = run_chat(&entry, test.as_ref(), messages, &cancel, &tx);
        let _ = tx.send(ChatMsg::Done(result));
    });
    rx
}

fn run_chat(
    entry: &ProviderEntry,
    test: Option<&TestEnv>,
    messages: Vec<ChatMessage>,
    cancel: &CancelToken,
    tx: &Sender<ChatMsg>,
) -> Result<ChatResponse, AiError> {
    let key = api_key(&entry.id, test)?;
    let provider = Provider::new(entry.chat_config(), key)?;
    let mut req = ChatRequest::new(entry.chat_model.clone(), messages);
    req.cancel = cancel.clone();
    provider.chat_stream(&req, &mut |d: &str| {
        let _ = tx.send(ChatMsg::Delta(d.to_string()));
    })
}

/// Result of preparing a question: the prompt to preview and what it is built from.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub prompt: Prompt,
    pub contexts: Vec<RagContext>,
    /// Whether vector search contributed.
    pub vector_used: bool,
    /// A non-fatal note for the user, such as a failed embedding call.
    pub warning: Option<String>,
}

/// Everything needed to prepare a question on the worker thread.
pub struct PrepareJob {
    pub question: String,
    pub notes: Vec<NoteSnap>,
    /// Current note as `(path, text)`, when the user asked for it.
    pub current: Option<(String, String)>,
    /// Selected text, when the user asked for it.
    pub selection: Option<String>,
    pub vault_search: bool,
    pub index_dir: std::path::PathBuf,
    pub embed: Option<(ProviderEntry, Option<TestEnv>)>,
    pub test: Option<TestEnv>,
}

/// Retrieves notes for a question and builds the prompt. Runs the embedding call first when the
/// vector index is ready, so the preview lists exactly what will be sent.
pub fn prepare(job: &PrepareJob) -> Result<Prepared, AiError> {
    let contexts_wanted = job.vault_search;
    let mut contexts: Vec<RagContext> = Vec::new();
    let mut vector_used = false;
    let mut warning = None;
    if contexts_wanted {
        let search = build_lexical(&job.notes);
        let lexical = |q: &str, n: usize| -> Vec<jiyue_ai::LexicalHit> {
            match &search {
                Some(s) => s
                    .search(q, n)
                    .into_iter()
                    .map(|h| jiyue_ai::LexicalHit {
                        path: h.path,
                        snippet: h.snippet,
                    })
                    .collect(),
                None => Vec::new(),
            }
        };
        let excluded: std::collections::HashSet<&str> = job
            .notes
            .iter()
            .filter(|s| s.excluded)
            .map(|s| s.path.as_str())
            .collect();
        let is_excluded = |p: &str| excluded.contains(p);

        let mut embedder_key: Option<Provider> = None;
        if let Some((entry, test)) = &job.embed
            && let Some(cfg) = entry.embedding_config()
        {
            match api_key(&entry.id, test.as_ref()).and_then(|k| Provider::new(cfg.clone(), k)) {
                Ok(p) => embedder_key = Some(p),
                Err(e) => warning = Some(errors::describe(&e)),
            }
        }
        let index = if embedder_key.is_some() {
            open_index(&job.index_dir, job.embed.as_ref())
        } else {
            None
        };
        let embedder: Option<&dyn EmbeddingProvider> =
            embedder_key.as_ref().map(|p| p as &dyn EmbeddingProvider);

        let ctx = RetrieveCtx {
            embedder: if index.is_some() { embedder } else { None },
            index: index.as_ref(),
            lexical: &lexical,
            is_excluded: &is_excluded,
            top_k: TOP_K,
            candidates: CANDIDATES,
        };
        match retrieve(&job.question, &ctx) {
            Ok(found) => {
                vector_used = index.is_some() && embedder.is_some();
                contexts = found;
            }
            Err(e) => {
                // The question embedding failed: fall back to keyword search only.
                warning = Some(format!(
                    "向量检索失败，已改用关键词检索：{}",
                    errors::describe(&e)
                ));
                let fallback = RetrieveCtx {
                    embedder: None,
                    index: None,
                    lexical: &lexical,
                    is_excluded: &is_excluded,
                    top_k: TOP_K,
                    candidates: CANDIDATES,
                };
                contexts = retrieve(&job.question, &fallback)?;
            }
        }
    }
    let current_ref = job.current.as_ref().map(|(p, t)| (p.as_str(), t.as_str()));
    let prompt = context::question_prompt(
        &job.question,
        &contexts,
        current_ref,
        job.selection.as_deref(),
        job.vault_search,
    );
    Ok(Prepared {
        prompt,
        contexts,
        vector_used,
        warning,
    })
}

/// In-memory keyword index over the allowed notes, built for one question. `None` when the
/// index cannot be created; retrieval then finds no keyword matches.
fn build_lexical(notes: &[NoteSnap]) -> Option<jiyue_search::SearchIndex> {
    let mut index = jiyue_search::SearchIndex::open_in_ram().ok()?;
    for doc in context::lexical_docs(notes) {
        let _ = index.upsert(&doc);
    }
    let _ = index.commit();
    Some(index)
}

fn open_index(dir: &Path, embed: Option<&(ProviderEntry, Option<TestEnv>)>) -> Option<VectorIndex> {
    let (entry, _) = embed?;
    let model = entry.embedding_config()?.model;
    let index = VectorIndex::open(dir, &model).ok()?;
    if index.chunk_count() == 0 {
        None
    } else {
        Some(index)
    }
}

/// Sends a tiny chat request to check the settings. `key` is the key typed in the form when
/// there is one; otherwise the stored key is used.
pub fn test_connection(
    entry: &ProviderEntry,
    key: Option<String>,
    test: Option<&TestEnv>,
) -> Result<(), AiError> {
    let key = match key {
        Some(k) => Some(k),
        None => api_key(&entry.id, test)?,
    };
    let provider = Provider::new(entry.chat_config(), key)?;
    let req = ChatRequest::new(
        entry.chat_model.clone(),
        vec![ChatMessage {
            role: jiyue_ai::ChatRole::User,
            content: "ping".into(),
        }],
    );
    // No output limit: a reasoning model would spend a tiny budget on reasoning and never answer.
    provider.chat_stream(&req, &mut |_: &str| {}).map(|_| ())
}

/// Starts `work` on a new thread and returns its receiver.
pub fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> std::sync::mpsc::Receiver<T> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiyue_ai::AiConfig;

    #[test]
    fn test_env_is_absent_without_variables() {
        // The test process does not set the override variables.
        assert!(test_env_from_process().is_none());
    }

    #[test]
    fn test_env_debug_hides_the_key() {
        let env = TestEnv {
            base: "http://127.0.0.1:9/v1".into(),
            key: "sk-secret-value".into(),
            model: "m".into(),
            embed: None,
        };
        let text = format!("{env:?}");
        assert!(!text.contains("sk-secret-value"));
        let entry = test_entry(&env);
        assert_eq!(entry.id, TEST_ID);
        assert!(entry.embedding_config().is_none());
    }

    #[test]
    fn prepare_without_vault_search_keeps_only_the_current_note() {
        let job = PrepareJob {
            question: "这篇讲了什么".into(),
            notes: Vec::new(),
            current: Some(("x/a.md".into(), "正文".into())),
            selection: None,
            vault_search: false,
            index_dir: std::env::temp_dir(),
            embed: None,
            test: None,
        };
        let prepared = prepare(&job).expect("prepare");
        assert!(prepared.contexts.is_empty());
        assert!(!prepared.vector_used);
        assert!(
            prepared
                .prompt
                .preview
                .items
                .iter()
                .any(|(l, _)| l.starts_with("Current note"))
        );
    }

    #[test]
    fn prepare_skips_excluded_notes_entirely() {
        let cfg = AiConfig {
            excluded_folders: vec!["私人".into()],
            ..AiConfig::default()
        };
        let notes = vec![
            NoteSnap::new(&cfg, "私人/秘密.md", "秘密", "口令是白兰地", &[]),
            NoteSnap::new(&cfg, "公开.md", "公开", "口令是苹果", &[]),
        ];
        let job = PrepareJob {
            question: "口令是什么".into(),
            notes,
            current: None,
            selection: None,
            vault_search: true,
            index_dir: std::env::temp_dir().join("jiyue-no-index-here"),
            embed: None,
            test: None,
        };
        let prepared = prepare(&job).expect("prepare");
        let paths: Vec<&str> = prepared.contexts.iter().map(|c| c.path.as_str()).collect();
        assert!(!paths.iter().any(|p| p.starts_with("私人")), "{paths:?}");
        let user = prepared
            .prompt
            .messages
            .iter()
            .find(|m| m.role == jiyue_ai::ChatRole::User)
            .expect("user");
        assert!(!user.content.contains("白兰地"));
    }

    #[test]
    fn selection_is_part_of_the_prompt_and_its_preview() {
        let job = PrepareJob {
            question: "改成更正式的语气".into(),
            notes: Vec::new(),
            current: None,
            selection: Some("嘿，我们走吧".into()),
            vault_search: false,
            index_dir: std::env::temp_dir(),
            embed: None,
            test: None,
        };
        let prepared = prepare(&job).expect("prepare");
        assert!(
            prepared
                .prompt
                .preview
                .items
                .iter()
                .any(|(l, _)| l == "选中文字")
        );
        assert!(
            prepared
                .prompt
                .messages
                .iter()
                .any(|m| m.content.contains("嘿，我们走吧"))
        );
    }
}
