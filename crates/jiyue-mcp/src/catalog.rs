//! The in-memory view of the vault: every note's facts, a RAM search index, and the link graph.
//!
//! The catalog never reads the on-disk index of the GUI app. [`Catalog::refresh`] walks the
//! vault, compares each note's size and modification time with what it last analysed, and
//! re-analyses only the notes that changed. It runs before every tool call that needs the
//! catalog, so results follow external edits without a file watcher.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use jiyue_core::fsops::is_ignored;
use jiyue_core::links::{self, LinkKind, LinkOccurrence, NoteRef};
use jiyue_search::{Hit, NoteDoc, SearchIndex};

use crate::analysis::{NoteFacts, analyze_note, file_stem, normalize_tag};
use crate::vault::{MAX_NOTE_BYTES, VaultRoot, is_note_path};

/// Folders deeper than this are not scanned.
const MAX_SCAN_DEPTH: usize = 64;

/// Everything the catalog keeps about one note.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteMeta {
    pub path: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub links: Vec<LinkOccurrence>,
    pub mtime_ms: i64,
    pub size: u64,
}

/// One outgoing link of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutLink {
    pub target: String,
    pub kind: &'static str,
    pub anchor: Option<String>,
    /// Vault-relative path of the note it points to, when it resolves.
    pub resolved: Option<String>,
    /// True for URLs and `mailto:` links.
    pub external: bool,
}

/// One note that links to a target, with the number of links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    pub path: String,
    pub title: String,
    pub count: usize,
}

/// What one refresh changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RefreshReport {
    pub analyzed: usize,
    pub removed: usize,
}

/// Notes, search index and link graph for one vault.
pub struct Catalog {
    root: VaultRoot,
    notes: BTreeMap<String, NoteMeta>,
    index: SearchIndex,
    dirty: bool,
}

impl Catalog {
    /// An empty catalog for `root`. Call [`Catalog::refresh`] before using it.
    pub fn new(root: VaultRoot) -> Result<Self, String> {
        let index = SearchIndex::open_in_ram()
            .map_err(|err| format!("cannot create the search index: {err}"))?;
        Ok(Self {
            root,
            notes: BTreeMap::new(),
            index,
            dirty: false,
        })
    }

    /// Brings the catalog in line with the disk: analyses new and changed notes, drops deleted
    /// ones, and commits the search index when anything changed.
    pub fn refresh(&mut self) -> Result<RefreshReport, String> {
        let scanned = scan_vault(self.root.path())?;
        let mut report = RefreshReport::default();

        for (rel, mtime_ms, size) in &scanned {
            let stale = match self.notes.get(rel) {
                Some(meta) => meta.mtime_ms != *mtime_ms || meta.size != *size,
                None => true,
            };
            if stale {
                self.analyze_file(rel, *mtime_ms, *size)?;
                report.analyzed += 1;
            }
        }

        let present: HashSet<&str> = scanned.iter().map(|(rel, _, _)| rel.as_str()).collect();
        let gone: Vec<String> = self
            .notes
            .keys()
            .filter(|rel| !present.contains(rel.as_str()))
            .cloned()
            .collect();
        for rel in &gone {
            self.notes.remove(rel);
            self.index
                .remove(rel)
                .map_err(|err| format!("search index: {err}"))?;
            report.removed += 1;
        }

        if self.dirty || report != RefreshReport::default() {
            self.index
                .commit()
                .map_err(|err| format!("search index commit: {err}"))?;
            self.dirty = false;
        }
        Ok(report)
    }

    /// Forgets the cached facts of `rel` so that the next refresh reads it again. Used after
    /// the server writes or deletes a note.
    pub fn invalidate(&mut self, rel: &str) -> Result<(), String> {
        self.notes.remove(rel);
        self.index
            .remove(rel)
            .map_err(|err| format!("search index: {err}"))?;
        self.dirty = true;
        Ok(())
    }

    /// Number of notes known to the catalog.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// True when the catalog holds no notes.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// All notes, sorted by path.
    pub fn notes(&self) -> impl Iterator<Item = &NoteMeta> + '_ {
        self.notes.values()
    }

    /// The note at `rel`. Falls back to a case-insensitive match when that is unambiguous.
    pub fn find(&self, rel: &str) -> Option<&NoteMeta> {
        if let Some(meta) = self.notes.get(rel) {
            return Some(meta);
        }
        let mut matches = self
            .notes
            .values()
            .filter(|meta| meta.path.eq_ignore_ascii_case(rel));
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    /// Full-text search over the committed index.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, String> {
        self.index
            .try_search(query, limit)
            .map_err(|err| format!("search failed: {err}"))
    }

    /// Notes that link to `target`, most links first, then by path.
    pub fn backlinks(&self, target: &NoteMeta) -> Vec<Backlink> {
        let (refs, metas) = self.note_refs();
        let Some(target_idx) = refs.iter().position(|r| r.path == target.path) else {
            return Vec::new();
        };
        let keys = link_keys(target);
        let mut out = Vec::new();
        for meta in &metas {
            if meta.path == target.path {
                continue;
            }
            let count = meta
                .links
                .iter()
                .filter(|occ| may_point_to(occ, &keys))
                .filter(|occ| links::resolve_occurrence(&refs, occ, &meta.path) == Some(target_idx))
                .count();
            if count > 0 {
                out.push(Backlink {
                    path: meta.path.clone(),
                    title: meta.title.clone(),
                    count,
                });
            }
        }
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
        out
    }

    /// Outgoing links of `source`, in document order.
    pub fn outgoing(&self, source: &NoteMeta) -> Vec<OutLink> {
        let (refs, _) = self.note_refs();
        source
            .links
            .iter()
            .map(|occ| {
                let resolved = links::resolve_occurrence(&refs, occ, &source.path)
                    .and_then(|i| refs.get(i))
                    .map(|r| r.path.clone());
                OutLink {
                    target: occ.target.clone(),
                    kind: kind_name(occ.kind),
                    anchor: occ.anchor.clone(),
                    resolved,
                    external: is_external(&occ.target),
                }
            })
            .collect()
    }

    /// Every tag with the number of notes that carry it.
    pub fn tag_counts(&self) -> BTreeMap<String, usize> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for meta in self.notes.values() {
            for tag in &meta.tags {
                *counts.entry(tag.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Paths of the notes that carry `tag` (case-insensitive), sorted.
    pub fn notes_with_tag(&self, tag: &str) -> Vec<String> {
        let wanted = normalize_tag(tag);
        self.notes
            .values()
            .filter(|meta| meta.tags.contains(&wanted))
            .map(|meta| meta.path.clone())
            .collect()
    }

    fn note_refs(&self) -> (Vec<NoteRef>, Vec<&NoteMeta>) {
        let metas: Vec<&NoteMeta> = self.notes.values().collect();
        let refs = metas
            .iter()
            .map(|meta| NoteRef {
                path: meta.path.clone(),
                aliases: meta.aliases.clone(),
            })
            .collect();
        (refs, metas)
    }

    fn analyze_file(&mut self, rel: &str, mtime_ms: i64, size: u64) -> Result<(), String> {
        let abs = self.root.path().join(rel);
        let text = if size > MAX_NOTE_BYTES {
            None
        } else {
            fs::read(&abs)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        };
        match text {
            Some(text) => {
                let NoteFacts {
                    title,
                    aliases,
                    tags,
                    links,
                    body,
                } = analyze_note(rel, &text);
                let doc = NoteDoc {
                    path: rel.to_string(),
                    title: title.clone(),
                    body,
                    tags: tags.clone(),
                    mtime_ms,
                };
                self.index
                    .upsert(&doc)
                    .map_err(|err| format!("search index: {err}"))?;
                self.notes.insert(
                    rel.to_string(),
                    NoteMeta {
                        path: rel.to_string(),
                        title,
                        aliases,
                        tags,
                        links,
                        mtime_ms,
                        size,
                    },
                );
            }
            None => {
                // Too large or not UTF-8: listed, but neither searched nor linked.
                self.index
                    .remove(rel)
                    .map_err(|err| format!("search index: {err}"))?;
                self.notes.insert(
                    rel.to_string(),
                    NoteMeta {
                        path: rel.to_string(),
                        title: file_stem(rel).to_string(),
                        aliases: Vec::new(),
                        tags: Vec::new(),
                        links: Vec::new(),
                        mtime_ms,
                        size,
                    },
                );
            }
        }
        self.dirty = true;
        Ok(())
    }
}

/// Lists every note file under `root` as (path, mtime in ms, size), sorted by path. Symbolic
/// links, ignored folders and files that are not notes are skipped.
pub fn scan_vault(root: &Path) -> Result<Vec<(String, i64, u64)>, String> {
    let mut out = Vec::new();
    scan_dir(root, "", 0, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn scan_dir(
    dir: &Path,
    prefix: &str,
    depth: usize,
    out: &mut Vec<(String, i64, u64)>,
) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH {
        return Ok(());
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if depth == 0 => return Err(format!("cannot read the vault: {err}")),
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if is_ignored(&name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if file_type.is_dir() {
            scan_dir(&entry.path(), &rel, depth + 1, out)?;
        } else if file_type.is_file()
            && is_note_path(&rel)
            && let Ok(meta) = entry.metadata()
        {
            out.push((rel, mtime_ms(&meta), meta.len()));
        }
    }
    Ok(())
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Strings a link target must end with (or equal) to possibly resolve to a note: its lower-case
/// file stem and its lower-case aliases. Used to skip most resolutions in `backlinks`.
fn link_keys(target: &NoteMeta) -> Vec<String> {
    let mut keys = vec![file_stem(&target.path).to_lowercase()];
    keys.extend(target.aliases.iter().map(|a| a.to_lowercase()));
    keys
}

fn may_point_to(occ: &LinkOccurrence, keys: &[String]) -> bool {
    let lowered = occ.target.replace('\\', "/").to_lowercase();
    let stem = lowered
        .strip_suffix(".md")
        .or_else(|| lowered.strip_suffix(".txt"))
        .unwrap_or(&lowered);
    keys.iter()
        .any(|key| stem == key.as_str() || stem.ends_with(&format!("/{key}")))
}

fn kind_name(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Wiki => "wikilink",
        LinkKind::Embed => "embed",
        LinkKind::MdLink => "markdown",
    }
}

fn is_external(target: &str) -> bool {
    let lower = target.trim().to_lowercase();
    lower.contains("://") || lower.starts_with("mailto:")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, write_file};

    fn catalog(dir: &TempDir) -> Catalog {
        let root = VaultRoot::open(dir.path()).expect("root");
        let mut c = Catalog::new(root).expect("catalog");
        c.refresh().expect("refresh");
        c
    }

    #[test]
    fn scan_skips_ignored_folders_symlinks_and_non_notes() {
        let dir = TempDir::new("scan");
        write_file(dir.path(), "a.md", "x");
        write_file(dir.path(), "sub/b.txt", "y");
        write_file(dir.path(), "pic.png", "z");
        write_file(dir.path(), ".trash/gone.md", "x");
        write_file(dir.path(), ".jiyue/idx.md", "x");
        write_file(dir.path(), "tmp.tmp.md", "x");
        let found: Vec<String> = scan_vault(dir.path())
            .expect("scan")
            .into_iter()
            .map(|(p, _, _)| p)
            .collect();
        assert_eq!(found, vec!["a.md", "sub/b.txt", "tmp.tmp.md"]);
    }

    #[test]
    fn refresh_adds_changes_and_removes_notes() {
        let dir = TempDir::new("refresh");
        write_file(dir.path(), "one.md", "alpha");
        let root = VaultRoot::open(dir.path()).expect("root");
        let mut c = Catalog::new(root).expect("catalog");
        let first = c.refresh().expect("refresh");
        assert_eq!(first.analyzed, 1);
        assert_eq!(c.len(), 1);

        let quiet = c.refresh().expect("refresh");
        assert_eq!(quiet, RefreshReport::default());

        write_file(dir.path(), "one.md", "beta gamma longer text");
        write_file(dir.path(), "two.md", "delta");
        let changed = c.refresh().expect("refresh");
        assert_eq!(changed.analyzed, 2);
        assert_eq!(c.len(), 2);

        std::fs::remove_file(dir.path().join("one.md")).expect("remove");
        let removed = c.refresh().expect("refresh");
        assert_eq!(removed.removed, 1);
        assert_eq!(c.len(), 1);
        assert!(c.search("delta", 5).expect("search").len() == 1);
        assert!(c.search("alpha", 5).expect("search").is_empty());
    }

    #[test]
    fn search_finds_bodies_titles_and_respects_query_syntax() {
        let dir = TempDir::new("search");
        write_file(
            dir.path(),
            "rust.md",
            "---\ntitle: Ownership\n---\nBorrow checker rules #lang",
        );
        write_file(dir.path(), "other.md", "Gardening notes");
        let c = catalog(&dir);
        let hits = c.search("borrow", 5).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "rust.md");
        assert_eq!(hits[0].title, "Ownership");
        assert_eq!(c.search("tag:lang", 5).expect("search").len(), 1);
        assert_eq!(
            c.search("file:other", 5).expect("search")[0].path,
            "other.md"
        );
    }

    #[test]
    fn backlinks_count_wiki_embed_and_markdown_links() {
        let dir = TempDir::new("backlinks");
        write_file(dir.path(), "Target.md", "# Target");
        write_file(dir.path(), "a.md", "[[Target]] and [[Target|again]]");
        write_file(dir.path(), "dir/b.md", "![[Target]] [x](../Target.md)");
        write_file(dir.path(), "c.md", "[[Other]]");
        let c = catalog(&dir);
        let target = c.find("Target.md").expect("target");
        let back = c.backlinks(target);
        let got: Vec<(&str, usize)> = back.iter().map(|b| (b.path.as_str(), b.count)).collect();
        assert_eq!(got, vec![("a.md", 2), ("dir/b.md", 2)]);
    }

    #[test]
    fn backlinks_respect_aliases() {
        let dir = TempDir::new("alias-backlinks");
        write_file(dir.path(), "Real.md", "---\naliases: [Nick]\n---\nbody");
        write_file(dir.path(), "x.md", "see [[Nick]]");
        let c = catalog(&dir);
        let back = c.backlinks(c.find("Real.md").expect("real"));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].path, "x.md");
    }

    #[test]
    fn outgoing_links_resolve_or_report_none() {
        let dir = TempDir::new("outgoing");
        write_file(
            dir.path(),
            "Here.md",
            "[[There]] [[Nowhere]] [site](https://example.org) ![[pic.png]]",
        );
        write_file(dir.path(), "There.md", "x");
        let c = catalog(&dir);
        let out = c.outgoing(c.find("Here.md").expect("here"));
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].resolved.as_deref(), Some("There.md"));
        assert_eq!(out[0].kind, "wikilink");
        assert_eq!(out[1].resolved, None);
        assert_eq!(out[2].kind, "embed");
        assert_eq!(out[2].resolved, None);
    }

    #[test]
    fn tag_counts_and_lookup_are_case_insensitive() {
        let dir = TempDir::new("tags");
        write_file(dir.path(), "a.md", "#Rust #web");
        write_file(dir.path(), "b.md", "---\ntags: [rust]\n---\n");
        let c = catalog(&dir);
        let counts = c.tag_counts();
        assert_eq!(counts.get("rust"), Some(&2));
        assert_eq!(counts.get("web"), Some(&1));
        assert_eq!(c.notes_with_tag("RUST"), vec!["a.md", "b.md"]);
    }

    #[test]
    fn find_is_case_insensitive_only_when_unambiguous() {
        let dir = TempDir::new("find");
        write_file(dir.path(), "Note.md", "x");
        let c = catalog(&dir);
        assert!(c.find("note.md").is_some());
        assert!(c.find("missing.md").is_none());
    }

    #[test]
    fn invalidate_forces_a_reread_and_drops_stale_hits() {
        let dir = TempDir::new("invalidate");
        write_file(dir.path(), "x.md", "original words");
        let mut c = catalog(&dir);
        assert_eq!(c.search("original", 5).expect("search").len(), 1);
        std::fs::remove_file(dir.path().join("x.md")).expect("remove");
        c.invalidate("x.md").expect("invalidate");
        c.refresh().expect("refresh");
        assert!(c.search("original", 5).expect("search").is_empty());
        assert!(c.is_empty());
    }

    #[test]
    fn external_and_kind_helpers() {
        assert!(is_external("https://a.b"));
        assert!(is_external("MAILTO:x@y"));
        assert!(!is_external("notes/x.md"));
        assert_eq!(kind_name(LinkKind::MdLink), "markdown");
    }
}
