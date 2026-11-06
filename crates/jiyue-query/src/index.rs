//! The property index: one [`NoteRecord`] per note, updated incrementally.
//!
//! The index stores metadata only (no note bodies). The note text is the source of truth; the
//! caller re-feeds changed files with [`PropertyIndex::upsert`], which is a no-op when the
//! content, size and timestamps are unchanged.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use jiyue_md::embeds::{LinkKind, parse_links};

use crate::cell::Cell;

/// Everything the query engine knows about one note.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteRecord {
    /// Vault-relative path with forward slashes, for example `项目/rust.md`.
    pub path: String,
    /// File name without the `.md` extension.
    pub name: String,
    /// Folder part of the path. Empty for notes at the root.
    pub folder: String,
    /// Frontmatter `tags` and inline `#tags` in the body, without `#`, de-duplicated.
    pub tags: Vec<String>,
    /// Frontmatter properties (top level only). Built-in names are kept here too but the
    /// built-in value wins in queries; use `prop("name")` to reach the frontmatter value.
    pub props: BTreeMap<String, Cell>,
    pub ctime_ms: i64,
    pub mtime_ms: i64,
    pub size: u64,
    /// Targets of wiki links, embeds and Markdown links in the body, in source order.
    pub outgoing_links: Vec<String>,
    /// Latin words plus one per CJK character.
    pub word_count: usize,
    /// FNV-1a hash of the text, used to skip unchanged notes.
    pub content_hash: u64,
}

/// Index of all notes in a vault.
#[derive(Debug, Default)]
pub struct PropertyIndex {
    notes: BTreeMap<String, NoteRecord>,
    /// Lowercase file name -> paths with that name.
    stems: HashMap<String, BTreeSet<String>>,
    /// Property name -> number of notes that have it.
    prop_counts: BTreeMap<String, usize>,
    /// Tag -> number of notes with it.
    tag_counts: BTreeMap<String, usize>,
    /// Folder (and each ancestor) -> number of notes below it.
    folder_counts: BTreeMap<String, usize>,
}

/// Turns a vault path into the canonical form used as the index key.
pub fn normalize_path(path: &str) -> String {
    let mut p = path.trim().replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    p.trim_start_matches('/').to_string()
}

fn folder_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

fn name_of(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = match file.len().checked_sub(3) {
        Some(cut)
            if file
                .get(cut..)
                .is_some_and(|e| e.eq_ignore_ascii_case(".md")) =>
        {
            file.get(..cut).unwrap_or(file)
        }
        _ => file,
    };
    stem.to_string()
}

/// The file name of a path without its `.md` extension.
pub fn stem_of(path: &str) -> String {
    name_of(path)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '/'
}

/// Inline `#tags` in the body. Skips fenced code, inline code and headings (`# Title`).
fn inline_tags(body: &str, out: &mut Vec<String>) {
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut in_code = false;
        let mut i = 0usize;
        while i < chars.len() {
            let c = chars.get(i).copied().unwrap_or(' ');
            if c == '`' {
                in_code = !in_code;
                i += 1;
                continue;
            }
            let at_word_start = i == 0 || chars.get(i - 1).is_some_and(|p| p.is_whitespace());
            if c == '#' && !in_code && at_word_start {
                let mut j = i + 1;
                while chars.get(j).is_some_and(|&x| is_tag_char(x)) {
                    j += 1;
                }
                let raw: String = chars.get(i + 1..j).unwrap_or(&[]).iter().collect();
                let tag = raw.trim_end_matches('/').to_string();
                let has_letter = tag.chars().any(|x| !x.is_ascii_digit());
                if !tag.is_empty() && has_letter && !out.contains(&tag) {
                    out.push(tag);
                }
                i = j.max(i + 1);
                continue;
            }
            i += 1;
        }
    }
}

fn word_count(body: &str) -> usize {
    let mut n = 0usize;
    let mut in_word = false;
    for c in body.chars() {
        if is_cjk(c) {
            n += 1;
            in_word = false;
        } else if c.is_alphanumeric() {
            if !in_word {
                n += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    n
}

/// Adds one tag, trimmed, without leading `#`, unless empty or already present.
fn push_tag(out: &mut Vec<String>, raw: &str) {
    let t = raw.trim().trim_start_matches('#').trim();
    if !t.is_empty() && !out.iter().any(|x| x == t) {
        out.push(t.to_string());
    }
}

/// Tags from one frontmatter `tags` value: a list, a comma or space separated string, or a number.
fn frontmatter_tags(value: &jiyue_md::frontmatter::Value, out: &mut Vec<String>) {
    use jiyue_md::frontmatter::Value;
    match value {
        Value::List(items) => {
            for item in items {
                push_tag(out, item);
            }
        }
        Value::Text(s) => {
            for tok in s.split(|c: char| c == ',' || c.is_whitespace()) {
                push_tag(out, tok);
            }
        }
        Value::Number(n) => push_tag(out, &n.to_string()),
        _ => {}
    }
}

fn build_record(path: &str, text: &str, mtime_ms: i64, ctime_ms: i64) -> NoteRecord {
    let (_, body_start) = jiyue_md::frontmatter::split(text);
    let body = text.get(body_start..).unwrap_or("");

    // One frontmatter scan serves both the properties and the `tags` / `tag` values.
    let fm = jiyue_md::frontmatter::properties(text);
    let mut tags: Vec<String> = Vec::new();
    for p in &fm {
        if p.key == "tags" || p.key == "tag" {
            frontmatter_tags(&p.value, &mut tags);
        }
    }
    let mut props = BTreeMap::new();
    for p in fm {
        props.insert(p.key, Cell::from_value(&p.value));
    }
    inline_tags(body, &mut tags);

    let mut links: Vec<String> = Vec::new();
    for link in parse_links(body) {
        if link.external || matches!(link.kind, LinkKind::MdImage) {
            continue;
        }
        let target = link.target.trim();
        if !target.is_empty() && !links.iter().any(|l| l == target) {
            links.push(target.to_string());
        }
    }

    NoteRecord {
        path: path.to_string(),
        name: name_of(path),
        folder: folder_of(path),
        tags,
        props,
        ctime_ms,
        mtime_ms,
        size: text.len() as u64,
        outgoing_links: links,
        word_count: word_count(body),
        content_hash: fnv1a(text.as_bytes()),
    }
}

fn adjust(map: &mut BTreeMap<String, usize>, key: &str, up: bool) {
    if up {
        *map.entry(key.to_string()).or_insert(0) += 1;
        return;
    }
    let gone = match map.get_mut(key) {
        Some(v) => {
            *v = v.saturating_sub(1);
            *v == 0
        }
        None => false,
    };
    if gone {
        map.remove(key);
    }
}

/// Folder path and every ancestor: `a/b/c` gives `a`, `a/b`, `a/b/c`.
fn folder_chain(folder: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for seg in folder.split('/').filter(|s| !s.is_empty()) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(seg);
        out.push(acc.clone());
    }
    out
}

fn cell_bytes(c: &Cell) -> usize {
    match c {
        Cell::Text(s) => 24 + s.len(),
        Cell::List(items) => 24 + items.iter().map(cell_bytes).sum::<usize>(),
        _ => 16,
    }
}

impl PropertyIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or refreshes a note from its text. Returns `false` when nothing changed.
    pub fn upsert(&mut self, path: &str, text: &str, mtime_ms: i64, ctime_ms: i64) -> bool {
        let path = normalize_path(path);
        if path.is_empty() {
            return false;
        }
        let hash = fnv1a(text.as_bytes());
        let size = text.len() as u64;
        if let Some(old) = self.notes.get(&path)
            && old.content_hash == hash
            && old.size == size
            && old.mtime_ms == mtime_ms
            && old.ctime_ms == ctime_ms
        {
            return false;
        }
        let rec = build_record(&path, text, mtime_ms, ctime_ms);
        self.remove_record(&path);
        self.insert_record(rec);
        true
    }

    /// Removes a note. Returns `false` when it was not indexed.
    pub fn remove(&mut self, path: &str) -> bool {
        self.remove_record(&normalize_path(path)).is_some()
    }

    /// Moves a note's record to a new path. Content is not re-read, so the caller should
    /// rewrite links in other notes separately. Returns `false` when `old` was not indexed.
    pub fn rename(&mut self, old: &str, new: &str) -> bool {
        let old = normalize_path(old);
        let new = normalize_path(new);
        if old.is_empty() || new.is_empty() {
            return false;
        }
        let Some(mut rec) = self.remove_record(&old) else {
            return false;
        };
        self.remove_record(&new);
        rec.path = new;
        rec.name = name_of(&rec.path);
        rec.folder = folder_of(&rec.path);
        self.insert_record(rec);
        true
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    pub fn get(&self, path: &str) -> Option<&NoteRecord> {
        self.notes.get(&normalize_path(path))
    }

    /// All records, in path order.
    pub fn records(&self) -> impl Iterator<Item = &NoteRecord> {
        self.notes.values()
    }

    /// Number of notes having each frontmatter property.
    pub fn property_counts(&self) -> &BTreeMap<String, usize> {
        &self.prop_counts
    }

    /// Number of notes having each tag.
    pub fn tag_counts(&self) -> &BTreeMap<String, usize> {
        &self.tag_counts
    }

    /// Every folder that contains notes, including parents of nested folders.
    pub fn folders(&self) -> impl Iterator<Item = &str> {
        self.folder_counts.keys().map(String::as_str)
    }

    /// Resolves link text to a note path: an exact path (with or without `.md`) first, then
    /// the first note whose file name matches case-insensitively.
    pub fn resolve_link(&self, text: &str) -> Option<String> {
        let t = normalize_path(text);
        let t = match t.len().checked_sub(3) {
            Some(cut) if t.get(cut..).is_some_and(|e| e.eq_ignore_ascii_case(".md")) => {
                t.get(..cut).unwrap_or("").to_string()
            }
            _ => t,
        };
        if t.is_empty() {
            return None;
        }
        if t.contains('/') {
            let with_ext = format!("{t}.md");
            if self.notes.contains_key(&with_ext) {
                return Some(with_ext);
            }
            if self.notes.contains_key(&t) {
                return Some(t);
            }
        }
        let key = t.rsplit('/').next().unwrap_or(&t).to_lowercase();
        self.stems
            .get(&key)
            .and_then(|set| set.iter().next().cloned())
    }

    /// For each target note, the sorted source paths that link to it. Links that do not
    /// resolve and self-links are ignored.
    pub fn backlink_sources(
        &self,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> HashMap<String, Vec<String>> {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for rec in self.notes.values() {
            for link in &rec.outgoing_links {
                let Some(target) = resolve(link) else {
                    continue;
                };
                if target == rec.path {
                    continue;
                }
                let sources = out.entry(target).or_default();
                if sources.last() != Some(&rec.path) {
                    sources.push(rec.path.clone());
                }
            }
        }
        out
    }

    /// Number of distinct notes linking to each target note, computed from outgoing links.
    pub fn backlink_counts(
        &self,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> BTreeMap<String, usize> {
        self.backlink_sources(resolve)
            .into_iter()
            .map(|(k, v)| (k, v.len()))
            .collect()
    }

    /// Rough heap usage in bytes. An estimate for budgeting, not an exact measurement.
    pub fn approx_memory_bytes(&self) -> usize {
        let mut total = 0usize;
        for (k, r) in &self.notes {
            total += 160 + k.len() * 2 + r.name.len() + r.folder.len();
            total += r.tags.iter().map(|t| 24 + t.len()).sum::<usize>();
            for (pk, pv) in &r.props {
                total += 48 + pk.len() + cell_bytes(pv);
            }
            total += r.outgoing_links.iter().map(|l| 24 + l.len()).sum::<usize>();
        }
        for (k, set) in &self.stems {
            total += 48 + k.len() + set.iter().map(|p| 24 + p.len()).sum::<usize>();
        }
        for map in [&self.prop_counts, &self.tag_counts, &self.folder_counts] {
            total += map.keys().map(|k| 56 + k.len()).sum::<usize>();
        }
        total
    }

    fn insert_record(&mut self, rec: NoteRecord) {
        for chain in folder_chain(&rec.folder) {
            adjust(&mut self.folder_counts, &chain, true);
        }
        for key in rec.props.keys() {
            adjust(&mut self.prop_counts, key, true);
        }
        for tag in &rec.tags {
            adjust(&mut self.tag_counts, tag, true);
        }
        self.stems
            .entry(rec.name.to_lowercase())
            .or_default()
            .insert(rec.path.clone());
        self.notes.insert(rec.path.clone(), rec);
    }

    fn remove_record(&mut self, path: &str) -> Option<NoteRecord> {
        let rec = self.notes.remove(path)?;
        for chain in folder_chain(&rec.folder) {
            adjust(&mut self.folder_counts, &chain, false);
        }
        for key in rec.props.keys() {
            adjust(&mut self.prop_counts, key, false);
        }
        for tag in &rec.tags {
            adjust(&mut self.tag_counts, tag, false);
        }
        let key = rec.name.to_lowercase();
        let empty = match self.stems.get_mut(&key) {
            Some(set) => {
                set.remove(path);
                set.is_empty()
            }
            None => false,
        };
        if empty {
            self.stems.remove(&key);
        }
        Some(rec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_folder_and_normalization() {
        assert_eq!(name_of("a/b/Note.md"), "Note");
        assert_eq!(name_of("a/b/Note.MD"), "Note");
        assert_eq!(name_of("plain"), "plain");
        assert_eq!(folder_of("a/b/Note.md"), "a/b");
        assert_eq!(folder_of("Note.md"), "");
        assert_eq!(normalize_path(".\\a\\b.md"), "a/b.md");
        assert_eq!(folder_chain("a/b/c"), vec!["a", "a/b", "a/b/c"]);
    }

    #[test]
    fn inline_tags_skip_code_and_headings() {
        let mut out = Vec::new();
        inline_tags(
            "# Heading\n#real/tag and `#code` and #123\n```\n#fenced\n```\ntext #Dup #Dup",
            &mut out,
        );
        assert_eq!(out, vec!["real/tag", "Dup"]);
    }

    #[test]
    fn word_count_mixes_latin_and_cjk() {
        assert_eq!(word_count("hello world"), 2);
        assert_eq!(word_count("项目 rust"), 3);
    }

    #[test]
    fn record_from_text() {
        let text = "---\ntitle: Hi\ntags: [a, b]\n---\nBody [[Other]] and ![[img.png]] #c\n";
        let r = build_record("x/Hi.md", text, 5, 3);
        assert_eq!(r.name, "Hi");
        assert_eq!(r.folder, "x");
        assert_eq!(r.tags, vec!["a", "b", "c"]);
        assert_eq!(r.outgoing_links, vec!["Other", "img.png"]);
        assert_eq!(r.props.get("title"), Some(&Cell::Text("Hi".into())));
        assert_eq!(r.size, text.len() as u64);
    }
}
