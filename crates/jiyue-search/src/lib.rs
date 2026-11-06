//! jiyue-search: full-text index over the note library (tantivy, CJK bigrams, query syntax).
//!
//! Query language: bare terms (AND), `"phrases"`, `-exclusion`, `OR` between terms,
//! `tag:x` (also `tag:#x`), `path:x` (substring), `file:x` (file name), `title:x`.
//! Unknown syntax degrades to plain terms and never errors.
//!
//! Writes are buffered until [`SearchIndex::commit`]. Searches see committed data only.

mod analysis;
mod query;
mod snippet;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use tantivy::collector::TopDocs;
use tantivy::schema::{
    IndexRecordOption, NumericOptions, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
    Value,
};
use tantivy::tokenizer::TextAnalyzer;
use tantivy::{
    DocAddress, DocId, Index, IndexReader, IndexWriter, ReloadPolicy, Score, SegmentReader,
    TantivyDocument, Term,
};

use crate::analysis::{ANALYZER_NAME, JiyueTokenizer, truncate_to};

/// A note as the index sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoteDoc {
    pub path: String,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub mtime_ms: i64,
}

/// One search result. `highlights` are byte ranges into `snippet`.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub highlights: Vec<Range<usize>>,
    pub score: f32,
}

/// Errors from the search index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    Io(String),
    Other(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchError::Io(msg) => write!(f, "search index io error: {msg}"),
            SearchError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for SearchError {}

impl From<std::io::Error> for SearchError {
    fn from(err: std::io::Error) -> Self {
        SearchError::Io(err.to_string())
    }
}

fn other<E: fmt::Display>(err: E) -> SearchError {
    SearchError::Other(err.to_string())
}

/// Bodies longer than this are truncated before indexing.
pub const MAX_BODY_BYTES: usize = 1 << 20;

/// Bumped whenever the schema or the analyzer changes. A mismatch forces a rebuild.
/// 2: identifier-like words also index their parts (`render_visible` -> `render`, `visible`).
pub const SCHEMA_VERSION: u32 = 2;

const VERSION_FILE: &str = "jiyue-search.version";
const INDEX_SUBDIR: &str = "tantivy";
const WRITER_BUDGET: usize = 50 * 1024 * 1024;

/// Field handles for the schema.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fields {
    pub path: tantivy::schema::Field,
    pub title: tantivy::schema::Field,
    pub body: tantivy::schema::Field,
    pub file: tantivy::schema::Field,
    pub tags: tantivy::schema::Field,
    pub mtime: tantivy::schema::Field,
}

impl Fields {
    fn from_schema(schema: &Schema) -> Option<Self> {
        Some(Self {
            path: schema.get_field("path").ok()?,
            title: schema.get_field("title").ok()?,
            body: schema.get_field("body").ok()?,
            file: schema.get_field("file").ok()?,
            tags: schema.get_field("tags").ok()?,
            mtime: schema.get_field("mtime_ms").ok()?,
        })
    }
}

fn build_schema() -> Schema {
    let analyzed = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(ANALYZER_NAME)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let mut builder = Schema::builder();
    builder.add_text_field("path", STRING | STORED);
    builder.add_text_field("title", analyzed.clone().set_stored());
    builder.add_text_field("body", analyzed.clone().set_stored());
    builder.add_text_field("file", analyzed);
    builder.add_text_field("tags", STRING | STORED);
    builder.add_i64_field(
        "mtime_ms",
        NumericOptions::default().set_fast().set_stored(),
    );
    builder.build()
}

fn register_analyzer(index: &Index) {
    index
        .tokenizers()
        .register(ANALYZER_NAME, TextAnalyzer::from(JiyueTokenizer));
}

struct Inner {
    reader: IndexReader,
    writer: Mutex<IndexWriter<TantivyDocument>>,
    fields: Fields,
    needs_rebuild: AtomicBool,
}

/// Full-text index stored in a directory (planned: `.jiyue/search`).
///
/// Clones share the same index and writer. Keep one writer per directory at a time.
#[derive(Clone)]
pub struct SearchIndex {
    dir: PathBuf,
    inner: Arc<Inner>,
}

impl fmt::Debug for SearchIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchIndex")
            .field("dir", &self.dir)
            .field("needs_rebuild", &self.needs_rebuild())
            .finish()
    }
}

impl SearchIndex {
    /// Opens the index in `dir`, creating it if needed.
    ///
    /// `needs_rebuild()` is true when the index was just created, when its schema version does
    /// not match, or when its files were unreadable (then the index was wiped). The caller
    /// should then call [`SearchIndex::rebuild`] with every note. A successful rebuild writes
    /// the "ready" marker, so later opens report `false`.
    pub fn open(dir: &Path) -> Result<Self, SearchError> {
        fs::create_dir_all(dir)?;
        let index_dir = dir.join(INDEX_SUBDIR);
        let version_ok = fs::read_to_string(dir.join(VERSION_FILE))
            .map(|s| s.trim() == version_marker())
            .unwrap_or(false);
        let existing = if version_ok && index_dir.join("meta.json").is_file() {
            open_existing(&index_dir)
        } else {
            None
        };
        let (index, needs_rebuild) = match existing {
            Some(index) => (index, false),
            None => (recreate(dir)?, true),
        };
        assemble(index, dir.to_path_buf(), needs_rebuild)
    }

    /// Opens an empty in-memory index. Nothing is persisted; `needs_rebuild()` is `false`.
    pub fn open_in_ram() -> Result<Self, SearchError> {
        let index = Index::create_in_ram(build_schema());
        assemble(index, PathBuf::new(), false)
    }

    /// The directory this index lives in (empty for RAM indexes).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// True when the caller must call [`SearchIndex::rebuild`] to repopulate the index.
    pub fn needs_rebuild(&self) -> bool {
        self.inner.needs_rebuild.load(Ordering::SeqCst)
    }

    /// Adds or replaces one note. Buffered until [`SearchIndex::commit`].
    pub fn upsert(&mut self, doc: &NoteDoc) -> Result<(), SearchError> {
        let fields = self.inner.fields;
        let tantivy_doc = to_document(fields, doc);
        let writer = self.lock_writer()?;
        writer.delete_term(Term::from_field_text(fields.path, &doc.path));
        writer.add_document(tantivy_doc).map_err(other)?;
        Ok(())
    }

    /// Removes the note at `path`. Buffered until [`SearchIndex::commit`].
    pub fn remove(&mut self, path: &str) -> Result<(), SearchError> {
        let fields = self.inner.fields;
        let writer = self.lock_writer()?;
        writer.delete_term(Term::from_field_text(fields.path, path));
        Ok(())
    }

    /// Makes pending changes visible to searches.
    pub fn commit(&mut self) -> Result<(), SearchError> {
        {
            let mut writer = self.lock_writer()?;
            writer.commit().map_err(other)?;
        }
        self.inner.reader.reload().map_err(other)?;
        Ok(())
    }

    /// Replaces the whole index with `docs` and commits. Clears `needs_rebuild()`.
    pub fn rebuild(&mut self, docs: impl Iterator<Item = NoteDoc>) -> Result<(), SearchError> {
        let fields = self.inner.fields;
        {
            let mut writer = self.lock_writer()?;
            writer.delete_all_documents().map_err(other)?;
            for doc in docs {
                writer
                    .add_document(to_document(fields, &doc))
                    .map_err(other)?;
            }
            writer.commit().map_err(other)?;
        }
        self.inner.reader.reload().map_err(other)?;
        if !self.dir.as_os_str().is_empty() {
            fs::write(self.dir.join(VERSION_FILE), version_marker())?;
        }
        self.inner.needs_rebuild.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Number of committed notes.
    pub fn count(&self) -> usize {
        self.inner.reader.searcher().num_docs() as usize
    }

    /// Searches committed notes. Errors are reported as an empty result; use
    /// [`SearchIndex::try_search`] to see them.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        self.try_search(query, limit).unwrap_or_default()
    }

    /// Searches committed notes, returning the hits ordered by score (ties by path).
    ///
    /// Blank queries, queries with only exclusions, and `limit == 0` give an empty result.
    pub fn try_search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, SearchError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let parsed = query::parse(query);
        let fields = self.inner.fields;
        let Some(built) = query::build(&parsed, fields) else {
            return Ok(Vec::new());
        };
        let highlight_terms = query::highlight_terms(&parsed);
        let searcher = self.inner.reader.searcher();
        let now = now_ms();

        // Negated phrases are checked on the candidates, so the first pass may need to fetch
        // more than `limit` documents. Each pass restarts from scratch with a larger window.
        let mut fetch = if built.exclusions.is_empty() {
            limit
        } else {
            limit.saturating_mul(2).max(32)
        };
        loop {
            let top = top_docs(&searcher, built.query.as_ref(), fetch, now)?;
            let fetched = top.len();
            let mut hits = Vec::with_capacity(limit.min(fetched));
            for (score, address) in top {
                let doc: TantivyDocument = searcher.doc(address).map_err(other)?;
                let path = stored_text(&doc, fields.path);
                let title = stored_text(&doc, fields.title);
                let body = stored_text(&doc, fields.body);
                if !built.exclusions.is_empty()
                    && query::is_excluded(&built.exclusions, &title, file_stem(&path), &body)
                {
                    continue;
                }
                let snip = snippet::build(&body, &highlight_terms);
                hits.push(Hit {
                    path,
                    title,
                    snippet: snip.text,
                    highlights: snip.highlights,
                    score,
                });
                if hits.len() == limit {
                    break;
                }
            }
            if hits.len() == limit || fetched < fetch {
                hits.sort_by(|a, b| {
                    b.score
                        .total_cmp(&a.score)
                        .then_with(|| a.path.cmp(&b.path))
                });
                return Ok(hits);
            }
            fetch = fetch.saturating_mul(4);
        }
    }

    fn lock_writer(&self) -> Result<MutexGuard<'_, IndexWriter<TantivyDocument>>, SearchError> {
        self.inner
            .writer
            .lock()
            .map_err(|_| SearchError::Other("index writer lock poisoned".into()))
    }
}

/// Content of the version marker file. It is written only by a successful rebuild.
fn version_marker() -> String {
    format!("jiyue-search schema {SCHEMA_VERSION}")
}

/// Opens an existing index if it is readable and has the expected fields. `None` means it
/// is corrupt or incompatible and should be recreated.
fn open_existing(index_dir: &Path) -> Option<Index> {
    let index = Index::open_in_dir(index_dir).ok()?;
    Fields::from_schema(&index.schema())?;
    register_analyzer(&index);
    index.reader().ok()?;
    Some(index)
}

/// Wipes the index under `dir` (and its version marker), then creates an empty index.
fn recreate(dir: &Path) -> Result<Index, SearchError> {
    let index_dir = dir.join(INDEX_SUBDIR);
    let marker = dir.join(VERSION_FILE);
    if marker.exists() {
        fs::remove_file(&marker)?;
    }
    if index_dir.exists() {
        fs::remove_dir_all(&index_dir)?;
    }
    fs::create_dir_all(&index_dir)?;
    Index::create_in_dir(&index_dir, build_schema()).map_err(other)
}

fn assemble(index: Index, dir: PathBuf, needs_rebuild: bool) -> Result<SearchIndex, SearchError> {
    register_analyzer(&index);
    let fields = Fields::from_schema(&index.schema())
        .ok_or_else(|| SearchError::Other("search index schema is missing fields".into()))?;
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .map_err(other)?;
    let writer = index
        .writer::<TantivyDocument>(WRITER_BUDGET)
        .map_err(other)?;
    Ok(SearchIndex {
        dir,
        inner: Arc::new(Inner {
            reader,
            writer: Mutex::new(writer),
            fields,
            needs_rebuild: AtomicBool::new(needs_rebuild),
        }),
    })
}

fn to_document(fields: Fields, note: &NoteDoc) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    doc.add_text(fields.path, &note.path);
    doc.add_text(fields.title, &note.title);
    doc.add_text(fields.body, truncate_to(&note.body, MAX_BODY_BYTES));
    doc.add_text(fields.file, file_stem(&note.path));
    let tags: BTreeSet<String> = note
        .tags
        .iter()
        .map(|t| query::normalize_tag(t))
        .filter(|t| !t.is_empty())
        .collect();
    for tag in &tags {
        doc.add_text(fields.tags, tag);
    }
    doc.add_i64(fields.mtime, note.mtime_ms);
    doc
}

/// File name without directory or extension. Accepts `/` and `\` separators.
fn file_stem(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// Top `k` documents for `query` by recency-tweaked BM25, best first.
fn top_docs(
    searcher: &tantivy::Searcher,
    query: &dyn tantivy::query::Query,
    k: usize,
    now: i64,
) -> Result<Vec<(f32, DocAddress)>, SearchError> {
    let collector = TopDocs::with_limit(k).tweak_score(move |segment: &SegmentReader| {
        let mtime = segment.fast_fields().i64("mtime_ms").ok();
        move |doc: DocId, score: Score| {
            let mtime_ms = mtime.as_ref().and_then(|col| col.first(doc)).unwrap_or(0);
            recency_score(score, mtime_ms, now)
        }
    });
    searcher.search(query, &collector).map_err(other)
}

fn stored_text(doc: &TantivyDocument, field: tantivy::schema::Field) -> String {
    doc.get_first(field)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Small recency boost: at most +10% for a note edited today, decaying over about a year.
fn recency_score(score: f32, mtime_ms: i64, now_ms: i64) -> f32 {
    let age_days = (now_ms.saturating_sub(mtime_ms).max(0)) as f64 / 86_400_000.0;
    let recency = 1.0 / (1.0 + age_days / 365.0);
    score * (1.0 + 0.1 * recency as f32)
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn file_stem_handles_both_separators_and_extensions() {
        assert_eq!(file_stem("想法/读书笔记.md"), "读书笔记");
        assert_eq!(file_stem("dir\\note.md"), "note");
        assert_eq!(file_stem("README"), "README");
        assert_eq!(file_stem(".hidden"), ".hidden");
        assert_eq!(file_stem("a/b.c.md"), "b.c");
    }

    #[test]
    fn recency_boost_is_small_and_monotone() {
        let now = 1_000_000_000_000;
        let fresh = recency_score(1.0, now, now);
        let old = recency_score(1.0, now - 3 * 365 * 86_400_000, now);
        assert!(fresh > old);
        assert!(fresh <= 1.1 + 1e-6);
        assert!(old >= 1.0);
        assert_eq!(recency_score(0.0, now, now), 0.0);
    }

    #[test]
    fn version_marker_mentions_schema_version() {
        assert!(version_marker().ends_with(&SCHEMA_VERSION.to_string()));
    }
}
