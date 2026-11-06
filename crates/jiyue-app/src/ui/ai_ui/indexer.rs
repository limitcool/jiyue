//! Vector index service: debounced queue of changed notes, and the background job that chunks,
//! embeds and stores them in `<vault>/.jiyue/index`.
//!
//! The job is resumable: the index file is saved after every note, and a note whose content hash
//! already matches is skipped. Pure parts (queue, hash) are unit-tested.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use jiyue_ai::{
    AiError, CancelToken, EmbeddingProvider, Provider, ProviderEntry, VectorIndex, chunk_note,
    embed_in_batches,
};

use super::context::NoteSnap;
use super::errors;

/// How long after the last save a changed note waits before it is embedded again.
pub const REEMBED_DELAY: Duration = Duration::from_secs(30);
/// Chunks sent per embedding request.
pub const EMBED_BATCH: usize = 16;

/// Changed notes waiting for re-embedding. A note is due `delay` after its last change.
#[derive(Debug, Clone)]
pub struct DebouncedQueue {
    delay: Duration,
    due: BTreeMap<String, Instant>,
}

impl DebouncedQueue {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            due: BTreeMap::new(),
        }
    }

    /// Records a change. Every change restarts the delay for that note.
    pub fn touch(&mut self, path: &str, now: Instant) {
        self.due.insert(path.to_string(), now + self.delay);
    }

    /// Drops a note from the queue (it was deleted, excluded, or indexed right away).
    pub fn forget(&mut self, path: &str) {
        self.due.remove(path);
    }

    /// Removes and returns the notes whose delay has passed, sorted by path.
    pub fn take_due(&mut self, now: Instant) -> Vec<String> {
        let ready: Vec<String> = self
            .due
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(p, _)| p.clone())
            .collect();
        for p in &ready {
            self.due.remove(p);
        }
        ready
    }

    pub fn len(&self) -> usize {
        self.due.len()
    }

    pub fn is_empty(&self) -> bool {
        self.due.is_empty()
    }
}

/// FNV-1a over the UTF-8 bytes. Stable across runs, unlike `DefaultHasher`.
pub fn content_hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Counts shown in the settings and the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexStats {
    pub notes: usize,
    pub chunks: usize,
    pub bytes: u64,
}

/// What one index run should do.
pub struct IndexJob {
    pub dir: PathBuf,
    pub entry: ProviderEntry,
    pub key: Option<String>,
    /// Every note of the vault (or the changed ones only, when `reconcile` is false).
    pub notes: Vec<NoteSnap>,
    /// Remove indexed notes that are not in `notes` any more (full runs only).
    pub reconcile: bool,
    pub cancel: CancelToken,
}

/// Messages from the index thread.
pub enum IndexMsg {
    /// `done` of `total` notes handled so far.
    Progress {
        done: usize,
        total: usize,
    },
    Finished(Result<IndexStats, IndexFailure>),
}

/// Why a run stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexFailure {
    /// Stopped on purpose (settings changed, rebuild requested, app closing).
    Paused,
    /// A problem to show to the user, in Chinese.
    Error(String),
}

/// Runs the job on the calling thread and reports through `tx`.
pub fn run(job: IndexJob, tx: &Sender<IndexMsg>) {
    let result = run_inner(&job, tx);
    let _ = tx.send(IndexMsg::Finished(result));
}

fn run_inner(job: &IndexJob, tx: &Sender<IndexMsg>) -> Result<IndexStats, IndexFailure> {
    std::fs::create_dir_all(&job.dir).map_err(|e| IndexFailure::Error(e.kind().to_string()))?;
    let emb_cfg = job
        .entry
        .embedding_config()
        .ok_or_else(|| IndexFailure::Error("没有配置嵌入模型".into()))?;
    let provider = Provider::new(emb_cfg.clone(), job.key.clone())
        .map_err(|e| IndexFailure::Error(errors::describe(&e)))?;
    let mut index = VectorIndex::open(&job.dir, &emb_cfg.model)
        .map_err(|e| IndexFailure::Error(e.to_string()))?;

    let wanted: HashSet<&str> = job
        .notes
        .iter()
        .filter(|s| !s.excluded)
        .map(|s| s.path.as_str())
        .collect();
    let mut removed = false;
    for path in index.note_paths() {
        let stale = !wanted.contains(path.as_str());
        if stale && (job.reconcile || job.notes.iter().any(|s| s.path == path)) {
            index.remove_note(&path);
            removed = true;
        }
    }
    if removed {
        index
            .save()
            .map_err(|e| IndexFailure::Error(e.to_string()))?;
    }

    let work: Vec<&NoteSnap> = job
        .notes
        .iter()
        .filter(|s| !s.excluded && index.needs_update(&s.path, content_hash(&s.text)))
        .collect();
    let total = work.len();
    for (n, snap) in work.into_iter().enumerate() {
        if job.cancel.is_cancelled() {
            return Err(IndexFailure::Paused);
        }
        embed_note(&mut index, &provider, snap, &job.cancel)?;
        index
            .save()
            .map_err(|e| IndexFailure::Error(e.to_string()))?;
        let _ = tx.send(IndexMsg::Progress { done: n + 1, total });
    }
    index
        .save()
        .map_err(|e| IndexFailure::Error(e.to_string()))?;
    let bytes = std::fs::metadata(job.dir.join("index.bin"))
        .map(|m| m.len())
        .unwrap_or(0);
    Ok(IndexStats {
        notes: index.note_count(),
        chunks: index.chunk_count(),
        bytes,
    })
}

fn embed_note(
    index: &mut VectorIndex,
    provider: &dyn EmbeddingProvider,
    snap: &NoteSnap,
    cancel: &CancelToken,
) -> Result<(), IndexFailure> {
    let hash = content_hash(&snap.text);
    let chunks = chunk_note(&snap.path, &snap.text);
    if chunks.is_empty() {
        return index
            .update_note(&snap.path, hash, Vec::new())
            .map_err(|e| IndexFailure::Error(e.to_string()));
    }
    let texts: Vec<String> = chunks.iter().map(|c| c.embedding_text()).collect();
    let vectors = embed_in_batches(provider, &texts, EMBED_BATCH, &mut |_, _| {}, cancel).map_err(
        |e| match e {
            AiError::Cancelled => IndexFailure::Paused,
            other => IndexFailure::Error(errors::describe(&other)),
        },
    )?;
    let pairs: Vec<_> = chunks.into_iter().zip(vectors).collect();
    index
        .update_note(&snap.path, hash, pairs)
        .map_err(|e| IndexFailure::Error(e.to_string()))
}

/// Short status text for the status bar and settings: `索引 3/10`.
pub fn progress_label(done: usize, total: usize) -> String {
    format!("AI 索引 {done}/{total}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_waits_for_the_delay_after_the_last_change() {
        let mut q = DebouncedQueue::new(Duration::from_secs(30));
        let t0 = Instant::now();
        q.touch("a.md", t0);
        q.touch("b.md", t0);
        assert!(q.take_due(t0 + Duration::from_secs(29)).is_empty());
        // A second save of a.md restarts its delay.
        q.touch("a.md", t0 + Duration::from_secs(20));
        let due = q.take_due(t0 + Duration::from_secs(31));
        assert_eq!(due, vec!["b.md".to_string()]);
        assert_eq!(q.len(), 1);
        let due = q.take_due(t0 + Duration::from_secs(50));
        assert_eq!(due, vec!["a.md".to_string()]);
        assert!(q.is_empty());
    }

    #[test]
    fn forgotten_notes_are_not_due() {
        let mut q = DebouncedQueue::new(Duration::from_secs(1));
        let t0 = Instant::now();
        q.touch("gone.md", t0);
        q.forget("gone.md");
        assert!(q.take_due(t0 + Duration::from_secs(10)).is_empty());
    }

    #[test]
    fn hash_is_stable_and_sensitive_to_content() {
        assert_eq!(content_hash("笔记"), content_hash("笔记"));
        assert_ne!(content_hash("笔记"), content_hash("笔记 "));
        // Known FNV-1a value for the empty input.
        assert_eq!(content_hash(""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn progress_label_format() {
        assert_eq!(progress_label(3, 10), "AI 索引 3/10");
    }

    /// Embedding stub that returns deterministic vectors and counts the calls.
    struct Fake;
    impl EmbeddingProvider for Fake {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Ok(texts
                .iter()
                .map(|t| vec![t.chars().count() as f32 + 1.0, 1.0])
                .collect())
        }
    }

    #[test]
    fn embed_note_stores_chunks_and_skips_unchanged_notes_on_resume() {
        let dir = std::env::temp_dir().join(format!("jiyue-ai-idx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let mut index = VectorIndex::open(&dir, "fake-model").expect("open");
        let snap = NoteSnap {
            path: "甲.md".into(),
            title: "甲".into(),
            text: "# 标题\n\n这是一段足够长的中文正文，用来生成至少一个分块。".into(),
            tags: vec![],
            excluded: false,
        };
        assert!(index.needs_update(&snap.path, content_hash(&snap.text)));
        embed_note(&mut index, &Fake, &snap, &CancelToken::new()).expect("embed");
        index.save().expect("save");
        assert!(index.chunk_count() >= 1);
        assert!(!index.needs_update(&snap.path, content_hash(&snap.text)));
        // Reopening keeps the stored state for the same model.
        let reopened = VectorIndex::open(&dir, "fake-model").expect("reopen");
        assert!(!reopened.needs_update(&snap.path, content_hash(&snap.text)));
        let other = VectorIndex::open(&dir, "other-model").expect("other model");
        assert_eq!(other.note_count(), 0, "another model starts empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelled_embedding_leaves_the_note_pending() {
        let dir = std::env::temp_dir().join(format!("jiyue-ai-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let mut index = VectorIndex::open(&dir, "fake").expect("open");
        let snap = NoteSnap {
            path: "c.md".into(),
            title: "c".into(),
            text: "足够长的文字，用于分块测试，足够长的文字，用于分块测试。".into(),
            tags: vec![],
            excluded: false,
        };
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            embed_note(&mut index, &Fake, &snap, &cancel),
            Err(IndexFailure::Paused)
        );
        assert!(index.needs_update("c.md", content_hash(&snap.text)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
