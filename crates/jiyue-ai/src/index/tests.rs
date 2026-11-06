use std::sync::Mutex;
use std::time::Instant;

use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jiyue-ai-index-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn chunk(path: &str, text: &str, start: usize) -> Chunk {
    Chunk {
        path: path.into(),
        heading_path: "H".into(),
        text: text.into(),
        range: start..start + text.len(),
    }
}

fn entry(path: &str, text: &str, v: Vec<f32>) -> (Chunk, Vec<f32>) {
    (chunk(path, text, 0), v)
}

#[test]
fn cosine_matches_hand_computed_values() {
    assert!((cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
    assert!((cosine(&[1.0, 1.0], &[2.0, 2.0]) - 1.0).abs() < 1e-6);
    assert!((cosine(&[1.0, 0.0], &[-3.0, 0.0]) + 1.0).abs() < 1e-6);
    let expected = (1.0f32 * 4.0 + 2.0 * 0.0 + 3.0 * 1.0) / (14.0f32.sqrt() * 17.0f32.sqrt());
    assert!((cosine(&[1.0, 2.0, 3.0], &[4.0, 0.0, 1.0]) - expected).abs() < 1e-6);
    assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
    assert_eq!(cosine(&[], &[]), 0.0);
}

#[test]
fn unrolled_dot_matches_naive_for_awkward_lengths() {
    for n in [1usize, 7, 8, 9, 15, 16, 17, 768, 1537] {
        let a: Vec<f32> = (0..n).map(|i| (i as f32 * 0.37).sin()).collect();
        let b: Vec<f32> = (0..n).map(|i| (i as f32 * 0.11).cos()).collect();
        let naive: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((dot(&a, &b) - naive).abs() < 1e-3, "n={n}");
    }
}

#[test]
fn query_ranks_by_cosine_and_respects_k() {
    let dir = temp_dir("rank");
    let mut idx = VectorIndex::open(&dir, "m1").expect("open");
    idx.update_note(
        "a.md",
        1,
        vec![
            entry("a.md", "about cats", vec![1.0, 0.0]),
            entry("a.md", "about dogs", vec![0.0, 1.0]),
        ],
    )
    .expect("update a");
    idx.update_note("b.md", 2, vec![entry("b.md", "mixed", vec![1.0, 1.0])])
        .expect("update b");
    assert_eq!(idx.dimension(), 2);
    assert_eq!(idx.note_count(), 2);
    assert_eq!(idx.chunk_count(), 3);

    let hits = idx.query(&[1.0, 0.1], 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].text, "about cats");
    assert!(hits[0].score >= hits[1].score);
    assert!(hits[0].score > 0.99);
    assert_eq!(hits[0].heading_path, "H");

    assert_eq!(
        idx.query(&[1.0, 0.1], 100).len(),
        3,
        "k larger than the index"
    );
    assert!(idx.query(&[1.0, 0.1], 0).is_empty());
}

#[test]
fn stored_vectors_are_normalised_so_magnitude_does_not_matter() {
    let dir = temp_dir("norm");
    let mut idx = VectorIndex::open(&dir, "m").expect("open");
    idx.update_note(
        "x.md",
        1,
        vec![
            entry("x.md", "big", vec![100.0, 0.0]),
            entry("x.md", "small", vec![0.5, 0.5]),
        ],
    )
    .expect("update");
    let hits = idx.query(&[3.0, 0.0], 2);
    assert_eq!(hits[0].text, "big");
    assert!((hits[0].score - 1.0).abs() < 1e-5);
}

#[test]
fn query_with_wrong_dimension_or_empty_index_returns_nothing() {
    let dir = temp_dir("empty");
    let idx = VectorIndex::open(&dir, "m").expect("open");
    assert!(idx.query(&[1.0, 2.0], 5).is_empty());
    assert_eq!(idx.dimension(), 0);

    let mut idx = idx;
    idx.update_note("a.md", 1, vec![entry("a.md", "t", vec![1.0, 0.0, 0.0])])
        .expect("update");
    assert!(idx.query(&[1.0, 0.0], 5).is_empty());
    assert!(
        idx.query(&[0.0, 0.0, 0.0], 5).is_empty(),
        "zero query has no direction"
    );
}

#[test]
fn needs_update_tracks_file_hash() {
    let dir = temp_dir("needs");
    let mut idx = VectorIndex::open(&dir, "m").expect("open");
    assert!(idx.needs_update("a.md", 5));
    idx.update_note("a.md", 5, vec![])
        .expect("empty note is recorded");
    assert!(!idx.needs_update("a.md", 5));
    assert!(idx.needs_update("a.md", 6));
}

#[test]
fn remove_note_forgets_it() {
    let dir = temp_dir("remove");
    let mut idx = VectorIndex::open(&dir, "m").expect("open");
    idx.update_note("a.md", 1, vec![entry("a.md", "t", vec![1.0])])
        .expect("update");
    assert!(idx.remove_note("a.md"));
    assert!(!idx.remove_note("a.md"));
    assert_eq!(idx.chunk_count(), 0);
    assert!(idx.note_paths().is_empty());
    assert_eq!(
        idx.dimension(),
        0,
        "dimension resets when the index is empty"
    );
}

#[test]
fn update_replaces_previous_chunks_of_the_note() {
    let dir = temp_dir("replace");
    let mut idx = VectorIndex::open(&dir, "m").expect("open");
    idx.update_note(
        "a.md",
        1,
        vec![
            entry("a.md", "old one", vec![1.0, 0.0]),
            entry("a.md", "old two", vec![0.0, 1.0]),
        ],
    )
    .expect("first");
    idx.update_note("a.md", 2, vec![entry("a.md", "new", vec![1.0, 0.0])])
        .expect("second");
    assert_eq!(idx.chunk_count(), 1);
    assert_eq!(idx.query(&[1.0, 0.0], 5)[0].text, "new");
    assert!(!idx.needs_update("a.md", 2));
}

#[test]
fn dimension_and_value_validation() {
    let dir = temp_dir("validate");
    let mut idx = VectorIndex::open(&dir, "m").expect("open");
    idx.update_note("a.md", 1, vec![entry("a.md", "t", vec![1.0, 0.0])])
        .expect("update");
    assert_eq!(
        idx.update_note("b.md", 1, vec![entry("b.md", "t", vec![1.0, 0.0, 0.0])]),
        Err(IndexError::DimensionMismatch {
            expected: 2,
            got: 3
        })
    );
    assert!(matches!(
        idx.update_note("b.md", 1, vec![entry("b.md", "t", vec![])]),
        Err(IndexError::InvalidVector(_))
    ));
    assert!(matches!(
        idx.update_note("b.md", 1, vec![entry("b.md", "t", vec![f32::NAN, 0.0])]),
        Err(IndexError::InvalidVector(_))
    ));
    assert!(
        !idx.needs_update("a.md", 1),
        "failed updates change nothing"
    );
    assert!(idx.needs_update("b.md", 1));
}

#[test]
fn save_and_reopen_keeps_content_and_query_results() {
    let dir = temp_dir("persist");
    {
        let mut idx = VectorIndex::open(&dir, "text-embed-3").expect("open");
        idx.update_note(
            "notes/a.md",
            42,
            vec![(
                Chunk {
                    path: "notes/a.md".into(),
                    heading_path: "Top > Sub".into(),
                    text: "持久化的中文内容".into(),
                    range: 10..30,
                },
                vec![0.6, 0.8],
            )],
        )
        .expect("update");
        idx.update_note("empty.md", 7, vec![]).expect("empty");
        idx.save().expect("save");
    }
    let idx = VectorIndex::open(&dir, "text-embed-3").expect("reopen");
    assert_eq!(idx.note_count(), 2);
    assert_eq!(idx.dimension(), 2);
    assert!(!idx.needs_update("notes/a.md", 42));
    assert!(!idx.needs_update("empty.md", 7));
    let hits = idx.query(&[0.6, 0.8], 1);
    assert_eq!(hits[0].path, "notes/a.md");
    assert_eq!(hits[0].text, "持久化的中文内容");
    assert_eq!(hits[0].range, 10..30);
    assert_eq!(hits[0].heading_path, "Top > Sub");
    assert!((hits[0].score - 1.0).abs() < 1e-5);
    assert_eq!(
        idx.note_paths(),
        vec!["empty.md".to_string(), "notes/a.md".to_string()]
    );
    assert!(!dir.join("index.bin.tmp").exists());
}

#[test]
fn switching_embedding_model_invalidates_the_index() {
    let dir = temp_dir("model");
    {
        let mut idx = VectorIndex::open(&dir, "model-a").expect("open");
        idx.update_note("a.md", 1, vec![entry("a.md", "t", vec![1.0, 0.0])])
            .expect("update");
        idx.save().expect("save");
    }
    let mut other = VectorIndex::open(&dir, "model-b").expect("open other");
    assert_eq!(other.chunk_count(), 0);
    assert_eq!(other.dimension(), 0);
    assert!(other.needs_update("a.md", 1));
    // Saving the empty index replaces the old file, so the old model cannot come back.
    other.save().expect("save");
    let again = VectorIndex::open(&dir, "model-a").expect("reopen as a");
    assert_eq!(again.chunk_count(), 0);
}

#[test]
fn damaged_files_are_treated_as_empty_not_errors() {
    let dir = temp_dir("damaged");
    {
        let mut idx = VectorIndex::open(&dir, "m").expect("open");
        idx.update_note("a.md", 1, vec![entry("a.md", "some text", vec![1.0, 0.0])])
            .expect("update");
        idx.save().expect("save");
    }
    let path = dir.join(FILE_NAME);
    let good = fs::read(&path).expect("read");

    // Truncated.
    fs::write(&path, &good[..good.len() - 5]).expect("write");
    assert_eq!(VectorIndex::open(&dir, "m").expect("open").chunk_count(), 0);

    // One flipped byte in the middle breaks the checksum.
    let mut flipped = good.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0xff;
    fs::write(&path, &flipped).expect("write");
    assert_eq!(VectorIndex::open(&dir, "m").expect("open").chunk_count(), 0);

    // Garbage.
    fs::write(&path, b"hello").expect("write");
    assert_eq!(VectorIndex::open(&dir, "m").expect("open").chunk_count(), 0);

    // Intact again.
    fs::write(&path, &good).expect("write");
    assert_eq!(VectorIndex::open(&dir, "m").expect("open").chunk_count(), 1);
}

#[test]
fn missing_directory_is_created_on_save() {
    let dir = temp_dir("mkdir").join("nested").join("index");
    let mut idx = VectorIndex::open(&dir, "m").expect("open");
    idx.update_note("a.md", 1, vec![]).expect("update");
    idx.save().expect("save creates directories");
    assert!(dir.join(FILE_NAME).exists());
}

#[test]
fn save_without_changes_does_not_touch_the_file() {
    let dir = temp_dir("clean");
    let mut idx = VectorIndex::open(&dir, "m").expect("open");
    idx.save().expect("nothing to write");
    assert!(!dir.join(FILE_NAME).exists());
}

struct FakeEmbedder {
    calls: Mutex<Vec<usize>>,
    short_by: usize,
}

impl EmbeddingProvider for FakeEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        self.calls.lock().expect("lock").push(texts.len());
        let n = texts.len().saturating_sub(self.short_by);
        Ok((0..n).map(|i| vec![i as f32 + 1.0, 1.0]).collect())
    }
}

#[test]
fn embed_in_batches_splits_reports_progress_and_orders_output() {
    let texts: Vec<String> = (0..7).map(|i| format!("t{i}")).collect();
    let fake = FakeEmbedder {
        calls: Mutex::new(Vec::new()),
        short_by: 0,
    };
    let mut progress = Vec::new();
    let out = embed_in_batches(
        &fake,
        &texts,
        3,
        &mut |d, t| progress.push((d, t)),
        &CancelToken::new(),
    )
    .expect("ok");
    assert_eq!(out.len(), 7);
    assert_eq!(*fake.calls.lock().expect("lock"), vec![3, 3, 1]);
    assert_eq!(progress, vec![(3, 7), (6, 7), (7, 7)]);
}

#[test]
fn embed_in_batches_zero_batch_size_still_works() {
    let texts: Vec<String> = vec!["a".into(), "b".into()];
    let fake = FakeEmbedder {
        calls: Mutex::new(Vec::new()),
        short_by: 0,
    };
    let out = embed_in_batches(&fake, &texts, 0, &mut |_, _| {}, &CancelToken::new()).expect("ok");
    assert_eq!(out.len(), 2);
    assert_eq!(*fake.calls.lock().expect("lock"), vec![1, 1]);
}

#[test]
fn embed_in_batches_honours_cancel_between_batches() {
    let texts: Vec<String> = (0..10).map(|i| i.to_string()).collect();
    let fake = FakeEmbedder {
        calls: Mutex::new(Vec::new()),
        short_by: 0,
    };
    let token = CancelToken::new();
    let t2 = token.clone();
    let res = embed_in_batches(
        &fake,
        &texts,
        4,
        &mut |done, _| {
            if done >= 4 {
                t2.cancel();
            }
        },
        &token,
    );
    assert_eq!(res, Err(AiError::Cancelled));
    assert_eq!(
        fake.calls.lock().expect("lock").len(),
        1,
        "only the first batch was sent"
    );
}

#[test]
fn embed_in_batches_rejects_short_responses() {
    let texts: Vec<String> = vec!["a".into(), "b".into()];
    let fake = FakeEmbedder {
        calls: Mutex::new(Vec::new()),
        short_by: 1,
    };
    let res = embed_in_batches(&fake, &texts, 2, &mut |_, _| {}, &CancelToken::new());
    assert!(matches!(res, Err(AiError::BadResponse(_))));
}

/// Micro-benchmark: brute-force query over 50k chunks of 768 dimensions. Run with
/// `cargo test -p jiyue-ai --release -- --ignored --nocapture bench_brute_force_query`.
#[test]
#[ignore]
fn bench_brute_force_query_50k_x_768() {
    const N_NOTES: usize = 5_000;
    const CHUNKS_PER_NOTE: usize = 10;
    const DIM: usize = 768;
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        ((seed >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    };
    let dir = temp_dir("bench");
    let mut idx = VectorIndex::open(&dir, "bench").expect("open");
    let build_start = Instant::now();
    for n in 0..N_NOTES {
        let path = format!("notes/{n}.md");
        let chunks: Vec<(Chunk, Vec<f32>)> = (0..CHUNKS_PER_NOTE)
            .map(|c| {
                let v: Vec<f32> = (0..DIM).map(|_| next()).collect();
                (chunk(&path, "benchmark chunk text", c * 100), v)
            })
            .collect();
        idx.update_note(&path, n as u64, chunks).expect("update");
    }
    println!(
        "built index: {} chunks in {:?}",
        idx.chunk_count(),
        build_start.elapsed()
    );
    let queries: Vec<Vec<f32>> = (0..20)
        .map(|_| (0..DIM).map(|_| next()).collect())
        .collect();
    let mut best = f32::MIN;
    let started = Instant::now();
    for q in &queries {
        let hits = idx.query(q, 10);
        best = best.max(hits.first().map(|h| h.score).unwrap_or(f32::MIN));
    }
    let per_query = started.elapsed() / queries.len() as u32;
    println!(
        "BENCH brute-force query: {} chunks x {} dims, top-10, avg {:?} per query (best score {best:.3})",
        idx.chunk_count(),
        DIM,
        per_query
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn parallel_query_matches_exhaustive_ranking() {
    // Large enough to take the multi-threaded path (PARALLEL_MIN_CHUNKS).
    const N_NOTES: usize = 900;
    const CHUNKS: usize = 10;
    const DIM: usize = 16;
    let mut seed: u64 = 12345;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        ((seed >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    };
    let dir = temp_dir("parallel");
    let mut idx = VectorIndex::open(&dir, "p").expect("open");
    let mut all: Vec<(String, String, Vec<f32>)> = Vec::new();
    for n in 0..N_NOTES {
        let path = format!("p/{n}.md");
        let mut chunks = Vec::new();
        for c in 0..CHUNKS {
            let v: Vec<f32> = (0..DIM).map(|_| next()).collect();
            let text = format!("{path}#{c}");
            chunks.push((chunk(&path, &text, c), v.clone()));
            all.push((path.clone(), text, v));
        }
        idx.update_note(&path, n as u64, chunks).expect("update");
    }
    let query: Vec<f32> = (0..DIM).map(|_| next()).collect();
    let k = 15;
    let got = idx.query(&query, k);
    assert_eq!(got.len(), k);

    let mut expected: Vec<(f32, String)> = all
        .iter()
        .map(|(_, text, v)| (cosine(&query, v), text.clone()))
        .collect();
    expected.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (hit, (score, text)) in got.iter().zip(expected.iter()) {
        assert_eq!(&hit.text, text);
        assert!(
            (hit.score - score).abs() < 1e-4,
            "{} vs {}",
            hit.score,
            score
        );
    }
    let _ = fs::remove_dir_all(&dir);
}
