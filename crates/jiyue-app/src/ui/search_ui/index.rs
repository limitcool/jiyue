//! Keeps the note index in `<vault>/.jiyue/index/search` current.
//!
//! - On `ExtEvent::Started` the index is opened in the background. When it needs a rebuild (new,
//!   wrong version or empty) every note is read from disk in chunks, then written in one rebuild.
//! - Saves write the note at once; edits are debounced (`CHANGE_DEBOUNCE_MS`); vault file changes
//!   upsert or remove the file. Writes are queued and run one job at a time, so they never
//!   reorder, and every job runs off the UI thread through `jobs::spawn_blocking`.
//! - While a build runs, `status` reports `(done, total)` for the status bar.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{Context, Task};
use jiyue_search::{NoteDoc, SearchIndex};

use super::logic::{self, CHANGE_DEBOUNCE_MS};
use crate::RustdianApp;
use crate::jobs::spawn_blocking;
use crate::session::Debounce;
use crate::ui::ext::ToastKind;
use jiyue_core::buffer::Note;

/// Notes read per background job while a build runs.
pub const BUILD_CHUNK: usize = 40;

/// One index write.
pub enum Op {
    /// Upsert this document (from memory: a save or an edit).
    Doc(NoteDoc),
    /// Remove the note at this path.
    Remove(String),
    /// Re-read this vault path from disk: upsert it, or remove it when the file is gone.
    Sync(String),
}

/// A full build in progress: the notes still to read and the documents read so far.
pub struct Build {
    pub total: usize,
    pub done: usize,
    queue: VecDeque<(String, String)>,
    docs: Vec<NoteDoc>,
}

/// State of the index pipeline (owned by `search_ui::State`).
#[derive(Default)]
pub struct Service {
    /// Vault root, set once the index was asked to open.
    root: Option<PathBuf>,
    /// The open index. Cloning shares the same index (cheap).
    pub index: Option<SearchIndex>,
    /// Writes waiting for the next job.
    ops: Vec<Op>,
    /// A job is running; the next one starts when it finishes.
    busy: bool,
    pub build: Option<Build>,
    /// Edited note paths waiting for the debounce.
    changed: Vec<String>,
    change_debounce: Debounce,
    change_task: Option<Task<()>>,
}

/// Forward-slash form of a vault path. The index keys notes by this form.
pub fn norm(path: &str) -> String {
    path.replace('\\', "/")
}

/// Milliseconds since the Unix epoch, or 0 for times before it.
fn ms_of(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn now_ms() -> i64 {
    ms_of(SystemTime::now())
}

/// A document from text in memory.
pub fn doc_from_text(path: &str, title: &str, text: &str, mtime_ms: i64) -> NoteDoc {
    NoteDoc {
        path: norm(path),
        title: title.to_string(),
        body: text.to_string(),
        tags: logic::note_tags(text),
        mtime_ms,
    }
}

/// A document read from `root.join(rel)`. `None` when the file cannot be read as text.
pub fn doc_from_disk(root: &Path, rel: &str, title: &str) -> Option<NoteDoc> {
    let full = root.join(rel);
    let text = fs::read_to_string(&full).ok()?;
    let mtime_ms = fs::metadata(&full)
        .and_then(|m| m.modified())
        .map(ms_of)
        .unwrap_or(0);
    Some(doc_from_text(rel, title, &text, mtime_ms))
}

/// Applies queued writes in order, then commits once. Errors are reported as text.
pub fn apply(index: &mut SearchIndex, root: &Path, ops: Vec<Op>) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Doc(doc) => index.upsert(&doc).map_err(|e| e.to_string())?,
            Op::Remove(path) => index.remove(&norm(&path)).map_err(|e| e.to_string())?,
            Op::Sync(path) => {
                let key = norm(&path);
                let title = Path::new(&key)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                match doc_from_disk(root, &key, &title) {
                    Some(doc) => index.upsert(&doc).map_err(|e| e.to_string())?,
                    None => index.remove(&key).map_err(|e| e.to_string())?,
                }
            }
        }
    }
    index.commit().map_err(|e| e.to_string())
}

/// Opens (creating when needed) the index in `dir`. The flag is true when a full build is needed.
fn open_index(dir: &Path) -> Result<(SearchIndex, bool), String> {
    let index = SearchIndex::open(dir).map_err(|e| e.to_string())?;
    let rebuild = index.needs_rebuild() || index.count() == 0;
    Ok((index, rebuild))
}

/// The note in the vault with this path (either separator).
fn find_note<'a>(app: &'a RustdianApp, path: &str) -> Option<&'a Note> {
    let key = norm(path);
    app.vault.notes.iter().find(|n| norm(&n.path) == key)
}

/// Starts the index. Called once, on `ExtEvent::Started`.
pub fn start(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let root = app.vault.root_dir.clone();
    let service = &mut app.ext.search.service;
    if service.root.is_some() {
        return;
    }
    service.root = Some(root.clone());
    let dir = logic::index_dir(&root);
    spawn_blocking(
        cx,
        move || open_index(&dir),
        |app: &mut RustdianApp, result, cx: &mut Context<RustdianApp>| {
            match result {
                Ok((index, rebuild)) => {
                    app.ext.search.service.index = Some(index);
                    if rebuild && !app.vault.notes.is_empty() {
                        start_build(app);
                    }
                }
                Err(err) => app.toast(ToastKind::Warn, format!("搜索索引不可用：{err}")),
            }
            pump(app, cx);
            cx.notify();
        },
    );
}

/// Queues a full build of every note in the vault.
fn start_build(app: &mut RustdianApp) {
    let queue: VecDeque<(String, String)> = app
        .vault
        .notes
        .iter()
        .map(|n| (norm(&n.path), n.title.clone()))
        .collect();
    let total = queue.len();
    app.ext.search.service.build = Some(Build {
        total,
        done: 0,
        queue,
        docs: Vec::new(),
    });
}

/// `(done, total)` while a build runs, for the status bar.
pub fn progress(app: &RustdianApp) -> Option<(usize, usize)> {
    app.ext
        .search
        .service
        .build
        .as_ref()
        .map(|b| (b.done, b.total))
}

/// The open index, for searches. `None` until it is open.
pub fn index(app: &RustdianApp) -> Option<SearchIndex> {
    app.ext.search.service.index.clone()
}

/// What the next job does.
enum Next {
    Chunk(Vec<(String, String)>),
    Rebuild(Vec<NoteDoc>),
    Ops(Vec<Op>),
    Idle,
}

/// Starts the next job if none runs. Builds take priority; writes wait until a build is done.
pub fn pump(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let next = {
        let svc = &mut app.ext.search.service;
        match (svc.busy, svc.index.is_some(), svc.root.is_some()) {
            (false, true, true) => {
                if let Some(build) = svc.build.as_mut() {
                    if build.queue.is_empty() {
                        Next::Rebuild(std::mem::take(&mut build.docs))
                    } else {
                        let take = build.queue.len().min(BUILD_CHUNK);
                        Next::Chunk(build.queue.drain(..take).collect())
                    }
                } else if svc.ops.is_empty() {
                    Next::Idle
                } else {
                    Next::Ops(std::mem::take(&mut svc.ops))
                }
            }
            _ => Next::Idle,
        }
    };
    let svc = &mut app.ext.search.service;
    let (Some(index), Some(root)) = (svc.index.clone(), svc.root.clone()) else {
        return;
    };
    match next {
        Next::Idle => {}
        Next::Chunk(chunk) => {
            svc.busy = true;
            let count = chunk.len();
            spawn_blocking(
                cx,
                move || {
                    let docs: Vec<NoteDoc> = chunk
                        .iter()
                        .filter_map(|(path, title)| doc_from_disk(&root, path, title))
                        .collect();
                    docs
                },
                move |app: &mut RustdianApp, docs: Vec<NoteDoc>, cx: &mut Context<RustdianApp>| {
                    let svc = &mut app.ext.search.service;
                    svc.busy = false;
                    if let Some(build) = svc.build.as_mut() {
                        build.done = (build.done + count).min(build.total);
                        build.docs.extend(docs);
                    }
                    pump(app, cx);
                    cx.notify();
                },
            );
        }
        Next::Rebuild(docs) => {
            svc.busy = true;
            spawn_blocking(
                cx,
                move || {
                    let mut index = index;
                    index.rebuild(docs.into_iter()).map_err(|e| e.to_string())
                },
                |app: &mut RustdianApp,
                 result: Result<(), String>,
                 cx: &mut Context<RustdianApp>| {
                    let svc = &mut app.ext.search.service;
                    svc.busy = false;
                    svc.build = None;
                    if let Err(err) = result {
                        app.toast(ToastKind::Warn, format!("搜索索引重建失败：{err}"));
                    }
                    pump(app, cx);
                    cx.notify();
                },
            );
        }
        Next::Ops(ops) => {
            svc.busy = true;
            spawn_blocking(
                cx,
                move || {
                    let mut index = index;
                    apply(&mut index, &root, ops)
                },
                |app: &mut RustdianApp,
                 result: Result<(), String>,
                 cx: &mut Context<RustdianApp>| {
                    app.ext.search.service.busy = false;
                    if let Err(err) = result {
                        app.toast(ToastKind::Warn, format!("搜索索引更新失败：{err}"));
                    }
                    pump(app, cx);
                    cx.notify();
                },
            );
        }
    }
}

/// A note was written to disk: index its text now.
pub fn note_saved(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    let Some(note) = find_note(app, path) else {
        return;
    };
    let doc = doc_from_text(&note.path, &note.title, note.content(), now_ms());
    app.ext.search.service.ops.push(Op::Doc(doc));
    pump(app, cx);
}

/// A note was edited: index it after the debounce, so typing does not write per keystroke.
pub fn note_changed(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    let key = norm(path);
    let svc = &mut app.ext.search.service;
    let token = svc.change_debounce.restart();
    if !svc.changed.contains(&key) {
        svc.changed.push(key);
    }
    svc.change_task = Some(cx.spawn(async move |this, cx| {
        cx.background_executor()
            .timer(Duration::from_millis(CHANGE_DEBOUNCE_MS))
            .await;
        let _ = this.update(cx, |app, cx| flush_changed(app, token, cx));
    }));
}

/// Runs when the change debounce ends: the edited notes are queued from memory.
fn flush_changed(app: &mut RustdianApp, token: u64, cx: &mut Context<RustdianApp>) {
    if !app.ext.search.service.change_debounce.is_current(token) {
        return;
    }
    let paths = std::mem::take(&mut app.ext.search.service.changed);
    let now = now_ms();
    let ops: Vec<Op> = paths
        .iter()
        .map(|p| match find_note(app, p) {
            Some(note) => Op::Doc(doc_from_text(&note.path, &note.title, note.content(), now)),
            None => Op::Remove(p.clone()),
        })
        .collect();
    app.ext.search.service.ops.extend(ops);
    pump(app, cx);
}

/// Files of the vault changed on disk (vault-relative paths).
pub fn files_changed(app: &mut RustdianApp, paths: &[String], cx: &mut Context<RustdianApp>) {
    let svc = &mut app.ext.search.service;
    svc.ops.extend(paths.iter().map(|p| Op::Sync(norm(p))));
    pump(app, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_use_forward_slashes_and_tags() {
        let doc = doc_from_text("想法\\a.md", "a", "#rust 正文", 7);
        assert_eq!(doc.path, "想法/a.md");
        assert_eq!(doc.tags, vec!["rust"]);
        assert_eq!(doc.mtime_ms, 7);
    }

    #[test]
    fn apply_upserts_removes_and_commits() {
        let mut index = SearchIndex::open_in_ram().expect("ram index");
        let root = std::env::temp_dir().join("jiyue-apply-nonexistent-dir");
        let ops = vec![
            Op::Doc(doc_from_text("a.md", "沉积", "沉积作用", 1)),
            Op::Doc(doc_from_text("b\\c.md", "其他", "无关内容", 1)),
        ];
        apply(&mut index, &root, ops).expect("apply");
        assert_eq!(index.count(), 2);
        assert_eq!(index.search("沉积", 10).len(), 1);
        // Upsert replaces, remove deletes; a missing file under Sync is removed.
        apply(
            &mut index,
            &root,
            vec![
                Op::Doc(doc_from_text("a.md", "沉积", "新的内容", 2)),
                Op::Remove("b/c.md".to_string()),
                Op::Sync("gone.md".to_string()),
            ],
        )
        .expect("apply");
        assert_eq!(index.count(), 1);
        assert!(index.search("沉积作用", 10).is_empty());
        assert_eq!(index.search("新的内容", 10).len(), 1);
    }

    #[test]
    fn sync_reads_the_file_from_disk() {
        let dir = std::env::temp_dir().join(format!("jiyue-sync-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("想法")).expect("dir");
        fs::write(dir.join("想法").join("读书.md"), "#书 地质年代").expect("write");
        let mut index = SearchIndex::open_in_ram().expect("ram index");
        apply(&mut index, &dir, vec![Op::Sync("想法/读书.md".to_string())]).expect("apply");
        let hits = index.search("地质年代", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "想法/读书.md");
        assert_eq!(hits[0].title, "读书");
        let _ = fs::remove_dir_all(&dir);
    }
}
