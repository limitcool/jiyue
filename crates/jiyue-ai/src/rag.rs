//! Retrieval for question answering: vector search (when embeddings exist) fused with lexical
//! search by reciprocal rank fusion (RRF).
//!
//! The app supplies the lexical search as a callback (it wires `jiyue-search` in), so this crate
//! does not depend on the search crate. Exclusion (folders, tags) is also a callback.
//!
//! Without an embedder or index, only the lexical results are returned.

use std::collections::{BTreeMap, HashSet};

use crate::index::VectorIndex;
use crate::{AiError, EmbeddingProvider};

/// Reciprocal rank fusion constant. 60 is the value from the original RRF paper.
pub const RRF_K: f64 = 60.0;

/// One lexical search hit, best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexicalHit {
    pub path: String,
    pub snippet: String,
}

/// Which rankings produced a context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextSource {
    Vector,
    Lexical,
    Both,
}

/// A note passage chosen for the prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    pub path: String,
    /// The best matching chunk when vector search found one, otherwise the lexical snippet.
    pub text: String,
    pub source: ContextSource,
    /// Fused RRF score; higher is better.
    pub score: f64,
}

/// Inputs for `retrieve`.
pub struct RetrieveCtx<'a> {
    /// Embedder for the question. `None` disables vector search.
    pub embedder: Option<&'a dyn EmbeddingProvider>,
    /// Chunk index. `None` disables vector search.
    pub index: Option<&'a VectorIndex>,
    /// `(query, limit) -> hits`, best first. Wired to jiyue-search by the app.
    pub lexical: &'a dyn Fn(&str, usize) -> Vec<LexicalHit>,
    /// True for paths that must never be returned (excluded folder or tag).
    pub is_excluded: &'a dyn Fn(&str) -> bool,
    /// Maximum number of contexts returned.
    pub top_k: usize,
    /// Candidates taken from each ranking before fusion.
    pub candidates: usize,
}

impl<'a> RetrieveCtx<'a> {
    /// Lexical-only context with sensible defaults (8 results, 20 candidates per ranking).
    pub fn lexical_only(
        lexical: &'a dyn Fn(&str, usize) -> Vec<LexicalHit>,
        is_excluded: &'a dyn Fn(&str) -> bool,
    ) -> Self {
        Self {
            embedder: None,
            index: None,
            lexical,
            is_excluded,
            top_k: 8,
            candidates: 20,
        }
    }
}

#[derive(Default)]
struct Fused {
    score: f64,
    vector_text: Option<String>,
    lexical_text: Option<String>,
}

/// Finds notes relevant to `question`. Returns at most `ctx.top_k` contexts, one per note,
/// ordered by fused score.
///
/// An error only comes from the embedder (for example a network failure). The caller can then
/// retry with `RetrieveCtx { embedder: None, .. }` to get lexical results only.
pub fn retrieve(question: &str, ctx: &RetrieveCtx<'_>) -> Result<Vec<Context>, AiError> {
    let question = question.trim();
    if question.is_empty() || ctx.top_k == 0 {
        return Ok(Vec::new());
    }
    let candidates = ctx.candidates.max(1);

    // Vector ranking, one entry per note (its best chunk).
    let mut vector_ranking: Vec<(String, String)> = Vec::new();
    if let (Some(embedder), Some(index)) = (ctx.embedder, ctx.index)
        && index.chunk_count() > 0
    {
        let vectors = embedder.embed(&[question.to_string()])?;
        if let Some(q) = vectors.first() {
            // Over-fetch so exclusions and per-note dedupe still leave enough candidates.
            let hits = index.query(q, candidates.saturating_mul(4));
            let mut seen = HashSet::new();
            for hit in hits {
                if (ctx.is_excluded)(&hit.path) || !seen.insert(hit.path.clone()) {
                    continue;
                }
                vector_ranking.push((hit.path, hit.text));
                if vector_ranking.len() >= candidates {
                    break;
                }
            }
        }
    }

    // Lexical ranking, one entry per note.
    let mut lexical_ranking: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();
    for hit in (ctx.lexical)(question, candidates.saturating_mul(2)) {
        if (ctx.is_excluded)(&hit.path) || !seen.insert(hit.path.clone()) {
            continue;
        }
        lexical_ranking.push((hit.path, hit.snippet));
        if lexical_ranking.len() >= candidates {
            break;
        }
    }

    Ok(fuse(&vector_ranking, &lexical_ranking, ctx.top_k))
}

/// Reciprocal rank fusion of two per-note rankings (already deduplicated, best first).
fn fuse(
    vector_ranking: &[(String, String)],
    lexical_ranking: &[(String, String)],
    top_k: usize,
) -> Vec<Context> {
    let mut fused: BTreeMap<&str, Fused> = BTreeMap::new();
    for (rank, (path, text)) in vector_ranking.iter().enumerate() {
        let entry = fused.entry(path.as_str()).or_default();
        entry.score += 1.0 / (RRF_K + rank as f64 + 1.0);
        entry.vector_text = Some(text.clone());
    }
    for (rank, (path, text)) in lexical_ranking.iter().enumerate() {
        let entry = fused.entry(path.as_str()).or_default();
        entry.score += 1.0 / (RRF_K + rank as f64 + 1.0);
        entry.lexical_text = Some(text.clone());
    }
    let mut out: Vec<Context> = fused
        .into_iter()
        .map(|(path, f)| {
            let source = match (f.vector_text.is_some(), f.lexical_text.is_some()) {
                (true, true) => ContextSource::Both,
                (true, false) => ContextSource::Vector,
                _ => ContextSource::Lexical,
            };
            let text = f.vector_text.or(f.lexical_text).unwrap_or_default();
            Context {
                path: path.to_string(),
                text,
                source,
                score: f.score,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
    });
    out.truncate(top_k);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    struct Fixed(Vec<f32>);

    impl EmbeddingProvider for Fixed {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Ok(texts.iter().map(|_| self.0.clone()).collect())
        }
    }

    struct Failing;

    impl EmbeddingProvider for Failing {
        fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Err(AiError::Network("offline".into()))
        }
    }

    fn lex(hits: &[(&str, &str)]) -> impl Fn(&str, usize) -> Vec<LexicalHit> + use<> {
        let owned: Vec<LexicalHit> = hits
            .iter()
            .map(|(p, s)| LexicalHit {
                path: p.to_string(),
                snippet: s.to_string(),
            })
            .collect();
        move |_q, limit| owned.iter().take(limit).cloned().collect()
    }

    fn no_exclusion(_: &str) -> bool {
        false
    }

    fn index_with(notes: &[(&str, &str, Vec<f32>)]) -> (std::path::PathBuf, VectorIndex) {
        let dir = std::env::temp_dir().join(format!(
            "jiyue-ai-rag-{}-{}",
            std::process::id(),
            notes.len()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut idx = VectorIndex::open(&dir, "m").expect("open");
        for (path, text, v) in notes {
            let chunk = Chunk {
                path: path.to_string(),
                heading_path: String::new(),
                text: text.to_string(),
                range: 0..text.len(),
            };
            idx.update_note(path, 1, vec![(chunk, v.clone())])
                .expect("update");
        }
        (dir, idx)
    }

    #[test]
    fn empty_question_returns_nothing() {
        let l = lex(&[("a.md", "x")]);
        let ctx = RetrieveCtx::lexical_only(&l, &no_exclusion);
        assert!(retrieve("   ", &ctx).expect("ok").is_empty());
    }

    #[test]
    fn lexical_only_works_without_embeddings() {
        let l = lex(&[("a.md", "snippet a"), ("b.md", "snippet b")]);
        let ctx = RetrieveCtx::lexical_only(&l, &no_exclusion);
        let out = retrieve("question", &ctx).expect("ok");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].path, "a.md");
        assert_eq!(out[0].text, "snippet a");
        assert_eq!(out[0].source, ContextSource::Lexical);
    }

    #[test]
    fn rrf_prefers_notes_found_by_both_rankings() {
        let (dir, idx) = index_with(&[
            ("vec-only.md", "vector text", vec![1.0, 0.0]),
            ("both.md", "both vector text", vec![0.9, 0.1]),
        ]);
        let l = lex(&[("lex-only.md", "lex snippet"), ("both.md", "both snippet")]);
        let embed = Fixed(vec![1.0, 0.0]);
        let ctx = RetrieveCtx {
            embedder: Some(&embed),
            index: Some(&idx),
            lexical: &l,
            is_excluded: &no_exclusion,
            top_k: 10,
            candidates: 10,
        };
        let out = retrieve("q", &ctx).expect("ok");
        assert_eq!(out[0].path, "both.md");
        assert_eq!(out[0].source, ContextSource::Both);
        assert_eq!(
            out[0].text, "both vector text",
            "vector chunk wins over lexical snippet"
        );
        let by_path = |p: &str| out.iter().find(|c| c.path == p).expect("present");
        assert_eq!(by_path("vec-only.md").source, ContextSource::Vector);
        assert_eq!(by_path("lex-only.md").source, ContextSource::Lexical);
        assert!(out.windows(2).all(|w| w[0].score >= w[1].score));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rrf_scores_match_formula() {
        let vector = vec![
            ("a.md".to_string(), "va".to_string()),
            ("b.md".to_string(), "vb".to_string()),
        ];
        let lexical = vec![
            ("b.md".to_string(), "lb".to_string()),
            ("c.md".to_string(), "lc".to_string()),
        ];
        let out = fuse(&vector, &lexical, 10);
        let score = |p: &str| out.iter().find(|c| c.path == p).expect("present").score;
        let expect_b = 1.0 / (RRF_K + 2.0) + 1.0 / (RRF_K + 1.0);
        assert!((score("b.md") - expect_b).abs() < 1e-12);
        assert!((score("a.md") - 1.0 / (RRF_K + 1.0)).abs() < 1e-12);
        assert!((score("c.md") - 1.0 / (RRF_K + 2.0)).abs() < 1e-12);
        assert_eq!(out[0].path, "b.md");
    }

    #[test]
    fn results_are_deduplicated_per_note() {
        let l = lex(&[("a.md", "first"), ("a.md", "second"), ("b.md", "third")]);
        let ctx = RetrieveCtx::lexical_only(&l, &no_exclusion);
        let out = retrieve("q", &ctx).expect("ok");
        assert_eq!(out.iter().filter(|c| c.path == "a.md").count(), 1);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn vector_side_dedupes_chunks_of_the_same_note() {
        let (dir, idx) = index_with(&[("a.md", "chunk one", vec![1.0, 0.0])]);
        // Same note with a second chunk that is also close.
        let mut idx = idx;
        let extra = Chunk {
            path: "a.md".into(),
            heading_path: String::new(),
            text: "chunk two".into(),
            range: 0..9,
        };
        let first = Chunk {
            path: "a.md".into(),
            heading_path: String::new(),
            text: "chunk one".into(),
            range: 0..9,
        };
        idx.update_note(
            "a.md",
            2,
            vec![(first, vec![1.0, 0.0]), (extra, vec![0.99, 0.1])],
        )
        .expect("update");
        let l = lex(&[]);
        let embed = Fixed(vec![1.0, 0.0]);
        let ctx = RetrieveCtx {
            embedder: Some(&embed),
            index: Some(&idx),
            lexical: &l,
            is_excluded: &no_exclusion,
            top_k: 5,
            candidates: 5,
        };
        let out = retrieve("q", &ctx).expect("ok");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "chunk one", "best chunk of the note is used");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn exclusion_predicate_removes_notes_from_both_rankings() {
        let (dir, idx) = index_with(&[
            ("secret/x.md", "hidden", vec![1.0, 0.0]),
            ("open.md", "visible", vec![0.5, 0.5]),
        ]);
        let l = lex(&[("secret/x.md", "hidden lex"), ("open.md", "open lex")]);
        let excluded = |p: &str| p.starts_with("secret/");
        let embed = Fixed(vec![1.0, 0.0]);
        let ctx = RetrieveCtx {
            embedder: Some(&embed),
            index: Some(&idx),
            lexical: &l,
            is_excluded: &excluded,
            top_k: 5,
            candidates: 5,
        };
        let out = retrieve("q", &ctx).expect("ok");
        assert!(out.iter().all(|c| !c.path.starts_with("secret/")));
        assert!(out.iter().any(|c| c.path == "open.md"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn top_k_limits_results() {
        let hits: Vec<(String, String)> = (0..10)
            .map(|i| (format!("n{i}.md"), "s".to_string()))
            .collect();
        let refs: Vec<(&str, &str)> = hits.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let l = lex(&refs);
        let ctx = RetrieveCtx {
            top_k: 3,
            ..RetrieveCtx::lexical_only(&l, &no_exclusion)
        };
        assert_eq!(retrieve("q", &ctx).expect("ok").len(), 3);
    }

    #[test]
    fn embedder_failure_is_reported_and_lexical_fallback_is_possible() {
        let (dir, idx) = index_with(&[("a.md", "t", vec![1.0, 0.0])]);
        let l = lex(&[("lex.md", "snippet")]);
        let failing = Failing;
        let ctx = RetrieveCtx {
            embedder: Some(&failing),
            index: Some(&idx),
            lexical: &l,
            is_excluded: &no_exclusion,
            top_k: 5,
            candidates: 5,
        };
        assert!(matches!(retrieve("q", &ctx), Err(AiError::Network(_))));

        let fallback = RetrieveCtx {
            embedder: None,
            index: None,
            ..ctx
        };
        let out = retrieve("q", &fallback).expect("lexical works");
        assert_eq!(out[0].path, "lex.md");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_index_skips_the_embedder_entirely() {
        let dir = std::env::temp_dir().join(format!("jiyue-ai-rag-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let idx = VectorIndex::open(&dir, "m").expect("open");
        let calls = AtomicUsize::new(0);
        struct Counting<'c>(&'c AtomicUsize);
        impl EmbeddingProvider for Counting<'_> {
            fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
                self.0.fetch_add(1, AtomicOrdering::SeqCst);
                Ok(vec![vec![1.0]])
            }
        }
        let counting = Counting(&calls);
        let l = lex(&[("a.md", "s")]);
        let ctx = RetrieveCtx {
            embedder: Some(&counting),
            index: Some(&idx),
            lexical: &l,
            is_excluded: &no_exclusion,
            top_k: 5,
            candidates: 5,
        };
        let out = retrieve("q", &ctx).expect("ok");
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            0,
            "no embedding call without indexed chunks"
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn candidate_limit_caps_each_ranking() {
        let hits: Vec<(String, String)> = (0..30)
            .map(|i| (format!("n{i}.md"), "s".to_string()))
            .collect();
        let refs: Vec<(&str, &str)> = hits.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let l = lex(&refs);
        let ctx = RetrieveCtx {
            top_k: 50,
            candidates: 4,
            ..RetrieveCtx::lexical_only(&l, &no_exclusion)
        };
        assert_eq!(retrieve("q", &ctx).expect("ok").len(), 4);
    }
}
