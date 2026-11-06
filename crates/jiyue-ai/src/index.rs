//! Vector index for chunk embeddings, plus batched embedding helper.
//!
//! The index lives in `<dir>/index.bin` (the caller passes `<vault>/.jiyue/index`). It is a
//! single compact binary file with a header (magic, format version, embedding model id and
//! dimension), then one record per note: path, file content hash, and for each chunk its
//! content hash, byte range, heading path, text and an f32 vector. A trailing FNV-1a checksum
//! detects truncated or corrupted files; such a file is treated as empty and rebuilt.
//!
//! Vectors are L2-normalised on insert, so the dot product equals cosine similarity. Query is an
//! exact brute-force scan over all chunks; no approximate index.
//!
//! Switching the embedding model (different model id) discards the stored index on `open`.

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::fmt;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::chunk::Chunk;
use crate::util::{atomic_write, fnv1a64};
use crate::{AiError, CancelToken, EmbeddingProvider};

const FILE_NAME: &str = "index.bin";
const MAGIC: &[u8; 8] = b"STRIDX01";
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    Io(String),
    DimensionMismatch { expected: usize, got: usize },
    InvalidVector(String),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Io(m) => write!(f, "index file error: {m}"),
            IndexError::DimensionMismatch { expected, got } => {
                write!(
                    f,
                    "embedding dimension {got} does not match the index ({expected})"
                )
            }
            IndexError::InvalidVector(m) => write!(f, "invalid embedding: {m}"),
        }
    }
}

impl std::error::Error for IndexError {}

/// One retrieved chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkHit {
    pub path: String,
    pub range: Range<usize>,
    /// Cosine similarity in [-1, 1].
    pub score: f32,
    pub text: String,
    pub heading_path: String,
}

#[derive(Debug, Clone)]
struct StoredChunk {
    hash: u64,
    range: Range<usize>,
    heading_path: String,
    text: String,
}

#[derive(Debug, Clone)]
struct NoteEntry {
    file_hash: u64,
    chunks: Vec<StoredChunk>,
    /// `chunks.len() * dim` values, row-major, each row normalised.
    vectors: Vec<f32>,
}

/// Chunk embeddings for a vault, persisted in `<dir>/index.bin`.
#[derive(Debug, Clone)]
pub struct VectorIndex {
    file: PathBuf,
    model: String,
    dim: usize,
    notes: BTreeMap<String, NoteEntry>,
    dirty: bool,
}

impl VectorIndex {
    /// Opens the index in `dir` for embedding model `model_id`.
    ///
    /// A missing file, a file written for another model, or a damaged file all give an empty
    /// index (the caller re-embeds). Only real I/O errors are returned.
    pub fn open(dir: &Path, model_id: &str) -> Result<Self, IndexError> {
        let file = dir.join(FILE_NAME);
        let mut index = Self {
            file: file.clone(),
            model: model_id.to_string(),
            dim: 0,
            notes: BTreeMap::new(),
            dirty: false,
        };
        let bytes = match fs::read(&file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(index),
            Err(e) => return Err(IndexError::Io(e.kind().to_string())),
        };
        match decode(&bytes) {
            Ok(loaded) if loaded.model == model_id => {
                index.dim = loaded.dim;
                index.notes = loaded.notes;
            }
            // Another model, or a damaged file: start empty and overwrite on the next save.
            _ => index.dirty = true,
        }
        Ok(index)
    }

    pub fn model_id(&self) -> &str {
        &self.model
    }

    /// Embedding dimension, or 0 while the index is empty.
    pub fn dimension(&self) -> usize {
        self.dim
    }

    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    pub fn chunk_count(&self) -> usize {
        self.notes.values().map(|n| n.chunks.len()).sum()
    }

    /// Paths of all indexed notes, sorted. Use this to find notes that were deleted.
    pub fn note_paths(&self) -> Vec<String> {
        self.notes.keys().cloned().collect()
    }

    /// True when the note is missing or its content hash changed.
    pub fn needs_update(&self, path: &str, file_hash: u64) -> bool {
        self.notes
            .get(path)
            .is_none_or(|entry| entry.file_hash != file_hash)
    }

    /// Replaces everything stored for `path`. An empty `chunks` list is valid and records that
    /// the note has nothing to embed. Vectors must all have the same non-zero length.
    pub fn update_note(
        &mut self,
        path: &str,
        file_hash: u64,
        chunks: Vec<(Chunk, Vec<f32>)>,
    ) -> Result<(), IndexError> {
        let mut dim = self.dim;
        for (_, v) in &chunks {
            if v.is_empty() {
                return Err(IndexError::InvalidVector("empty vector".into()));
            }
            if dim == 0 {
                dim = v.len();
            }
            if v.len() != dim {
                return Err(IndexError::DimensionMismatch {
                    expected: dim,
                    got: v.len(),
                });
            }
            if v.iter().any(|x| !x.is_finite()) {
                return Err(IndexError::InvalidVector("non-finite value".into()));
            }
        }
        let mut stored = Vec::with_capacity(chunks.len());
        let mut vectors = Vec::with_capacity(chunks.len() * dim);
        for (chunk, mut v) in chunks {
            normalize(&mut v);
            vectors.extend_from_slice(&v);
            stored.push(StoredChunk {
                hash: fnv1a64(chunk.text.as_bytes()),
                range: chunk.range,
                heading_path: chunk.heading_path,
                text: chunk.text,
            });
        }
        self.dim = dim;
        self.notes.insert(
            path.to_string(),
            NoteEntry {
                file_hash,
                chunks: stored,
                vectors,
            },
        );
        self.dirty = true;
        Ok(())
    }

    /// Forgets a note. Returns whether it was indexed.
    pub fn remove_note(&mut self, path: &str) -> bool {
        let removed = self.notes.remove(path).is_some();
        if removed {
            self.dirty = true;
        }
        if self.notes.is_empty() {
            self.dim = 0;
        }
        removed
    }

    /// Writes the index if anything changed since it was opened or last saved.
    pub fn save(&mut self) -> Result<(), IndexError> {
        if !self.dirty {
            return Ok(());
        }
        let bytes = encode(self);
        atomic_write(&self.file, &bytes).map_err(|e| IndexError::Io(e.kind().to_string()))?;
        self.dirty = false;
        Ok(())
    }

    /// Returns the `k` most similar chunks, best first. A query whose length differs from the
    /// index dimension (or an empty index) returns no hits.
    pub fn query(&self, vector: &[f32], k: usize) -> Vec<ChunkHit> {
        if k == 0 || self.dim == 0 || vector.len() != self.dim {
            return Vec::new();
        }
        let mut q = vector.to_vec();
        if !q.iter().all(|x| x.is_finite()) || normalize(&mut q) == 0.0 {
            return Vec::new();
        }
        let notes: Vec<(&str, &NoteEntry)> =
            self.notes.iter().map(|(p, n)| (p.as_str(), n)).collect();
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, 8);
        let heaps = if threads == 1 || self.chunk_count() < PARALLEL_MIN_CHUNKS {
            vec![scan(&notes, &q, self.dim, k)]
        } else {
            let per_thread = notes.len().div_ceil(threads).max(1);
            std::thread::scope(|s| {
                let handles: Vec<_> = notes
                    .chunks(per_thread)
                    .map(|part| {
                        let q = &q;
                        s.spawn(move || scan(part, q, self.dim, k))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_default())
                    .collect::<Vec<_>>()
            })
        };
        let mut merged: BinaryHeap<Reverse<Candidate<'_>>> = BinaryHeap::with_capacity(k + 1);
        for heap in heaps {
            for Reverse(cand) in heap {
                push_bounded(&mut merged, k, cand);
            }
        }
        let mut best: Vec<Candidate<'_>> = merged.into_iter().map(|Reverse(c)| c).collect();
        best.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.path.cmp(b.path)));
        best.into_iter()
            .map(|c| ChunkHit {
                path: c.path.to_string(),
                range: c.chunk.range.clone(),
                score: c.score,
                text: c.chunk.text.clone(),
                heading_path: c.chunk.heading_path.clone(),
            })
            .collect()
    }

    /// Content hash of a stored chunk, for callers that want to skip unchanged chunks.
    pub fn chunk_hashes(&self, path: &str) -> Vec<u64> {
        self.notes
            .get(path)
            .map(|n| n.chunks.iter().map(|c| c.hash).collect())
            .unwrap_or_default()
    }
}

/// Below this many chunks a query runs on the calling thread only.
const PARALLEL_MIN_CHUNKS: usize = 8_000;

/// Scores every chunk of `notes` against `q` and keeps the best `k` in a min-heap.
fn scan<'a>(
    notes: &[(&'a str, &'a NoteEntry)],
    q: &[f32],
    dim: usize,
    k: usize,
) -> BinaryHeap<Reverse<Candidate<'a>>> {
    let mut heap = BinaryHeap::with_capacity(k + 1);
    for &(path, note) in notes {
        for (i, chunk) in note.chunks.iter().enumerate() {
            let row = &note.vectors[i * dim..(i + 1) * dim];
            let score = dot(q, row);
            push_bounded(&mut heap, k, Candidate { score, path, chunk });
        }
    }
    heap
}

fn push_bounded<'a>(heap: &mut BinaryHeap<Reverse<Candidate<'a>>>, k: usize, cand: Candidate<'a>) {
    if heap.len() < k {
        heap.push(Reverse(cand));
    } else if heap
        .peek()
        .is_some_and(|Reverse(worst)| cand.score > worst.score)
    {
        heap.pop();
        heap.push(Reverse(cand));
    }
}

#[derive(Clone, Copy)]
struct Candidate<'a> {
    score: f32,
    path: &'a str,
    chunk: &'a StoredChunk,
}

impl PartialEq for Candidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate<'_> {}

impl PartialOrd for Candidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.path.cmp(self.path))
    }
}

/// Scales `v` to unit length in place and returns the original norm. Zero vectors stay zero.
fn normalize(v: &mut [f32]) -> f32 {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 && norm.is_finite() {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    norm
}

/// Dot product written with eight independent accumulators so the compiler vectorises it.
#[inline]
pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (ca, ra) = a.as_chunks::<8>();
    let (cb, rb) = b.as_chunks::<8>();
    for (x, y) in ca.iter().zip(cb) {
        for lane in 0..8 {
            acc[lane] += x[lane] * y[lane];
        }
    }
    let mut sum: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        sum += x * y;
    }
    sum
}

/// Cosine similarity of two vectors (normalising both). 0.0 when either is zero or lengths
/// differ.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot(a, b) / (na * nb)
}

// ---------- binary format ----------

fn encode(index: &VectorIndex) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_u32(&mut out, FORMAT_VERSION);
    put_str(&mut out, &index.model);
    put_u32(&mut out, index.dim as u32);
    put_u32(&mut out, index.notes.len() as u32);
    for (path, note) in &index.notes {
        put_str(&mut out, path);
        out.extend_from_slice(&note.file_hash.to_le_bytes());
        put_u32(&mut out, note.chunks.len() as u32);
        for c in &note.chunks {
            out.extend_from_slice(&c.hash.to_le_bytes());
            out.extend_from_slice(&(c.range.start as u64).to_le_bytes());
            out.extend_from_slice(&(c.range.end as u64).to_le_bytes());
            put_str(&mut out, &c.heading_path);
            put_str(&mut out, &c.text);
        }
        for v in &note.vectors {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    let checksum = fnv1a64(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    out
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("length overflow")?;
        if end > self.data.len() {
            return Err("truncated".into());
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(b);
        Ok(u64::from_le_bytes(arr))
    }

    fn string(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        let b = self.take(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| "invalid UTF-8".to_string())
    }
}

struct Decoded {
    model: String,
    dim: usize,
    notes: BTreeMap<String, NoteEntry>,
}

fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    if bytes.len() < MAGIC.len() + 8 {
        return Err("too short".into());
    }
    let (body, tail) = bytes.split_at(bytes.len() - 8);
    let mut arr = [0u8; 8];
    arr.copy_from_slice(tail);
    if u64::from_le_bytes(arr) != fnv1a64(body) {
        return Err("checksum mismatch".into());
    }
    let mut r = Reader { data: body, pos: 0 };
    if r.take(MAGIC.len())? != MAGIC {
        return Err("bad magic".into());
    }
    if r.u32()? != FORMAT_VERSION {
        return Err("unsupported version".into());
    }
    let model = r.string()?;
    let dim = r.u32()? as usize;
    let note_count = r.u32()? as usize;
    let mut notes = BTreeMap::new();
    for _ in 0..note_count {
        let path = r.string()?;
        let file_hash = r.u64()?;
        let chunk_count = r.u32()? as usize;
        let mut chunks = Vec::with_capacity(chunk_count.min(4096));
        for _ in 0..chunk_count {
            let hash = r.u64()?;
            let start = r.u64()? as usize;
            let end = r.u64()? as usize;
            if start > end {
                return Err("invalid range".into());
            }
            let heading_path = r.string()?;
            let text = r.string()?;
            chunks.push(StoredChunk {
                hash,
                range: start..end,
                heading_path,
                text,
            });
        }
        let vec_bytes = chunk_count
            .checked_mul(dim)
            .and_then(|n| n.checked_mul(4))
            .ok_or("vector size overflow")?;
        let raw = r.take(vec_bytes)?;
        let vectors: Vec<f32> = raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        notes.insert(
            path,
            NoteEntry {
                file_hash,
                chunks,
                vectors,
            },
        );
    }
    if r.pos != body.len() {
        return Err("trailing bytes".into());
    }
    Ok(Decoded { model, dim, notes })
}

// ---------- batched embedding ----------

/// Embeds `texts` in batches of `batch_size` (at least 1). `progress(done, total)` is called after
/// each batch. Cancellation is checked before each batch and returns `AiError::Cancelled`.
pub fn embed_in_batches(
    provider: &dyn EmbeddingProvider,
    texts: &[String],
    batch_size: usize,
    progress: &mut dyn FnMut(usize, usize),
    cancel: &CancelToken,
) -> Result<Vec<Vec<f32>>, AiError> {
    let batch_size = batch_size.max(1);
    let total = texts.len();
    let mut out = Vec::with_capacity(total);
    for batch in texts.chunks(batch_size) {
        if cancel.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let vectors = provider.embed(batch)?;
        if vectors.len() != batch.len() {
            return Err(AiError::BadResponse(format!(
                "expected {} embeddings, got {}",
                batch.len(),
                vectors.len()
            )));
        }
        out.extend(vectors);
        progress(out.len(), total);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
