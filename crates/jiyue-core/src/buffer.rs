use std::sync::Arc;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// Summary of a text buffer's metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextSummary {
    pub bytes: usize,
    pub chars: usize,
    pub newlines: usize,
}

impl TextSummary {
    /// Computes summary metrics for `text` in a single pass without heap allocation.
    pub fn of_text(text: &str) -> Self {
        let bytes = text.len();
        let chars = text.chars().count();
        let newlines = text.bytes().filter(|&b| b == b'\n').count();
        Self {
            bytes,
            chars,
            newlines,
        }
    }

    /// Total display lines represented by this summary.
    pub fn lines(&self) -> usize {
        if self.bytes == 0 {
            0
        } else {
            self.newlines + 1
        }
    }

    /// Updates cached metrics in O(|removed| + |inserted|) after replacing `removed` with `inserted`.
    pub fn apply_delta(&mut self, removed: &str, inserted: &str) {
        self.bytes = self.bytes - removed.len() + inserted.len();
        self.chars = self.chars - removed.chars().count() + inserted.chars().count();
        self.newlines = self.newlines - removed.bytes().filter(|&b| b == b'\n').count()
            + inserted.bytes().filter(|&b| b == b'\n').count();
    }
}

/// Edit transaction for Undo/Redo history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditOperation {
    pub offset: usize,
    pub old_text: String,
    pub new_text: String,
}

/// Compact text buffer backed by shared immutable `Arc<str>` storage and cached metrics.
#[derive(Debug, Clone)]
pub struct ZedBuffer {
    text: Arc<str>,
    summary: TextSummary,
    pub undo_stack: Vec<EditOperation>,
    pub redo_stack: Vec<EditOperation>,
}

impl Default for ZedBuffer {
    fn default() -> Self {
        Self::new("")
    }
}

impl ZedBuffer {
    pub fn new(initial_text: &str) -> Self {
        Self::from_arc(Arc::from(initial_text))
    }

    /// Creates a buffer directly from shared immutable storage without reallocating or copying bytes.
    pub fn from_arc(text: Arc<str>) -> Self {
        let summary = TextSummary::of_text(&text);
        Self {
            text,
            summary,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Shared handle to the immutable text storage (O(1) clone).
    pub fn arc(&self) -> Arc<str> {
        Arc::clone(&self.text)
    }

    /// Total text as str.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    fn set_text(&mut self, updated: String) {
        self.text = Arc::from(updated.into_boxed_str());
        self.summary = TextSummary::of_text(&self.text);
    }

    /// Replaces `range` with `new_str` in a single allocation and updates summary metrics
    /// in O(|deleted| + |inserted|), without recording undo history.
    pub fn replace_range_raw(&mut self, range: std::ops::Range<usize>, new_str: &str) {
        let start = range.start.min(self.text.len());
        let end = range.end.min(self.text.len()).max(start);
        if start == end && new_str.is_empty() {
            return;
        }
        let deleted = &self.text[start..end];
        self.summary.apply_delta(deleted, new_str);

        let new_len = self.text.len() - (end - start) + new_str.len();
        let mut updated = String::with_capacity(new_len);
        updated.push_str(&self.text[..start]);
        updated.push_str(new_str);
        updated.push_str(&self.text[end..]);
        self.text = Arc::from(updated.into_boxed_str());
    }

    /// Insert text at byte offset with undo tracking.
    pub fn insert(&mut self, offset: usize, new_str: &str) {
        if offset > self.text.len() {
            return;
        }
        let op = EditOperation {
            offset,
            old_text: String::new(),
            new_text: new_str.to_string(),
        };
        self.undo_stack.push(op);
        self.redo_stack.clear();

        let mut updated = self.text.to_string();
        updated.insert_str(offset, new_str);
        self.set_text(updated);
    }

    /// Delete text range with undo tracking.
    pub fn delete(&mut self, offset: usize, len: usize) {
        if offset >= self.text.len() || len == 0 {
            return;
        }
        let end = (offset + len).min(self.text.len());
        let deleted = self.text[offset..end].to_string();

        let op = EditOperation {
            offset,
            old_text: deleted,
            new_text: String::new(),
        };
        self.undo_stack.push(op);
        self.redo_stack.clear();

        let mut updated = self.text.to_string();
        updated.replace_range(offset..end, "");
        self.set_text(updated);
    }

    /// Undo last edit operation.
    pub fn undo(&mut self) -> bool {
        if let Some(op) = self.undo_stack.pop() {
            let offset = op.offset;
            let len = op.new_text.len();
            let end = (offset + len).min(self.text.len());
            let mut updated = self.text.to_string();
            updated.replace_range(offset..end, &op.old_text);
            self.redo_stack.push(op);
            self.set_text(updated);
            true
        } else {
            false
        }
    }

    /// Redo last undone operation.
    pub fn redo(&mut self) -> bool {
        if let Some(op) = self.redo_stack.pop() {
            let offset = op.offset;
            let len = op.old_text.len();
            let end = (offset + len).min(self.text.len());
            let mut updated = self.text.to_string();
            updated.replace_range(offset..end, &op.new_text);
            self.undo_stack.push(op);
            self.set_text(updated);
            true
        } else {
            false
        }
    }

    /// Cached text summary metrics.
    pub fn summary(&self) -> TextSummary {
        self.summary
    }

    pub fn lines(&self) -> usize {
        self.summary.lines()
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Outline heading item
#[derive(Debug, Clone)]
pub struct HeadingItem {
    pub level: usize,
    pub text: String,
    pub line_number: usize,
}

/// Disk fingerprint `(byte_len, modified_unix_nanos)` used to skip untouched notes during reload.
pub fn file_disk_stamp(meta: &std::fs::Metadata) -> Option<(u64, u128)> {
    let nanos = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((meta.len(), nanos))
}

/// Extracts `aliases` / `alias` values from a leading YAML frontmatter block (`--- ... ---`),
/// without invoking a full YAML parser when no frontmatter is present.
pub fn extract_frontmatter_aliases(content: &str) -> Vec<String> {
    if !content.starts_with("---") {
        return Vec::new();
    }
    let mut lines = content.split('\n');
    let first = lines.next().unwrap_or("");
    if first.trim_end_matches('\r') != "---" {
        return Vec::new();
    }
    let mut block_lines: Vec<&str> = Vec::new();
    let mut closed = false;
    for raw in lines {
        let line = raw.trim_end_matches('\r');
        if line == "---" || line == "..." {
            closed = true;
            break;
        }
        block_lines.push(line);
    }
    if !closed {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < block_lines.len() {
        let line = block_lines[i];
        let is_top_level = !line.starts_with([' ', '\t', '#']) && !line.trim().is_empty();
        if !is_top_level {
            i += 1;
            continue;
        }
        let Some((raw_key, raw_rest)) = line.split_once(':') else {
            i += 1;
            continue;
        };
        let key = raw_key.trim();
        if !key.eq_ignore_ascii_case("aliases") && !key.eq_ignore_ascii_case("alias") {
            i += 1;
            continue;
        }
        let val = strip_yaml_inline_comment(raw_rest).trim();
        if val.starts_with('[') && val.ends_with(']') && val.len() >= 2 {
            for item in val[1..val.len() - 1].split(',') {
                let cleaned = unquote_yaml_scalar(item.trim());
                if !cleaned.is_empty() && !out.iter().any(|x| x == cleaned) {
                    out.push(cleaned.to_string());
                }
            }
            i += 1;
        } else if !val.is_empty() {
            let cleaned = unquote_yaml_scalar(val);
            if !cleaned.is_empty() && !out.iter().any(|x| x == cleaned) {
                out.push(cleaned.to_string());
            }
            i += 1;
        } else {
            i += 1;
            while i < block_lines.len() {
                let sub = block_lines[i];
                let sub_trim = sub.trim();
                if sub_trim.is_empty() || sub_trim.starts_with('#') {
                    i += 1;
                    continue;
                }
                if !sub.starts_with([' ', '\t']) {
                    break;
                }
                if let Some(after_dash) = sub_trim.strip_prefix('-') {
                    let item = strip_yaml_inline_comment(after_dash).trim();
                    let cleaned = unquote_yaml_scalar(item);
                    if !cleaned.is_empty() && !out.iter().any(|x| x == cleaned) {
                        out.push(cleaned.to_string());
                    }
                }
                i += 1;
            }
        }
    }
    out
}

fn strip_yaml_inline_comment(s: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let bytes = s.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'#' if !in_single
                && !in_double
                && (idx == 0 || bytes[idx - 1].is_ascii_whitespace()) =>
            {
                return &s[..idx];
            }
            _ => {}
        }
    }
    s
}

fn unquote_yaml_scalar(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].trim()
    } else {
        s
    }
}

/// A Markdown note in the vault backed by a ZedBuffer
#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub path: String,
    pub buffer: ZedBuffer,
    pub tags: Vec<String>,
    pub wikilinks: Vec<String>,
    pub aliases: Vec<String>,
    pub disk_stamp: Option<(u64, u128)>,
}

/// The wikilink targets of a note's text, in order and without repeats (the part before `|`).
pub fn wikilink_targets(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut pos = 0;
    while let Some(start) = content[pos..].find("[[") {
        let actual_start = pos + start + 2;
        if let Some(end) = content[actual_start..].find("]]") {
            let link_text = &content[actual_start..actual_start + end];
            let target = link_text.split('|').next().unwrap_or(link_text).trim();
            if !target.is_empty() && !links.iter().any(|existing| existing == target) {
                links.push(target.to_string());
            }
            pos = actual_start + end + 2;
        } else {
            break;
        }
    }
    links
}

impl Note {
    pub fn new(title: &str, path: &str, content: &str) -> Self {
        Self::new_with_stamp(title, path, content, None)
    }

    pub fn new_with_stamp(
        title: &str,
        path: &str,
        content: &str,
        disk_stamp: Option<(u64, u128)>,
    ) -> Self {
        let clean_path = path.replace('\\', "/");
        let id = clean_path.to_lowercase().replace(' ', "-");
        let buffer = ZedBuffer::new(content);
        let mut note = Self {
            id,
            title: title.to_string(),
            path: clean_path,
            buffer,
            tags: Vec::new(),
            wikilinks: Vec::new(),
            aliases: Vec::new(),
            disk_stamp,
        };
        note.refresh_metadata();
        note
    }

    /// Updates `title`, `path`, and derived `id` in place (used when reusing unchanged moved/renamed notes).
    pub fn rename_in_place(&mut self, title: &str, rel_path: &str) {
        let clean_path = rel_path.replace('\\', "/");
        self.id = clean_path.to_lowercase().replace(' ', "-");
        self.title = title.to_string();
        self.path = clean_path;
    }

    pub fn content(&self) -> &str {
        self.buffer.as_str()
    }

    /// Shared handle to the note's text (O(1) clone, zero byte copy).
    pub fn content_arc(&self) -> Arc<str> {
        self.buffer.arc()
    }

    /// Replaces the note buffer from shared immutable storage without reallocating text and
    /// returns `(tags_changed, links_changed, aliases_changed)`.
    pub fn set_content_arc(&mut self, arc: Arc<str>) -> (bool, bool, bool) {
        self.buffer = ZedBuffer::from_arc(arc);
        self.disk_stamp = None;
        self.refresh_metadata_diff()
    }

    /// Set note content directly and refresh cached tags, wikilinks, and frontmatter aliases.
    pub fn set_content(&mut self, new_content: &str) {
        self.buffer = ZedBuffer::new(new_content);
        self.disk_stamp = None;
        self.refresh_metadata();
    }

    fn refresh_metadata(&mut self) {
        self.extract_tags();
        self.wikilinks = wikilink_targets(self.content());
        self.aliases = extract_frontmatter_aliases(self.content());
    }

    fn refresh_metadata_diff(&mut self) -> (bool, bool, bool) {
        let old_tags = std::mem::take(&mut self.tags);
        let old_links = std::mem::take(&mut self.wikilinks);
        let old_aliases = std::mem::take(&mut self.aliases);
        self.extract_tags();
        self.wikilinks = wikilink_targets(self.content());
        self.aliases = extract_frontmatter_aliases(self.content());
        (
            self.tags != old_tags,
            self.wikilinks != old_links,
            self.aliases != old_aliases,
        )
    }

    /// Extract hashtags `#tag` from content
    pub fn extract_tags(&mut self) {
        let mut tags = Vec::new();
        for word in self.content().split_whitespace() {
            if word.starts_with('#') && word.len() > 1 && !word.starts_with("##") {
                let tag = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
                if !tag.is_empty() && !tags.iter().any(|existing| existing == tag) {
                    tags.push(tag.to_string());
                }
            }
        }
        self.tags = tags;
    }

    /// Extract wikilinks `[[Note Title]]` from cached metadata.
    pub fn extract_wikilinks(&self) -> Vec<String> {
        self.wikilinks.clone()
    }

    /// Extract headings for the Outline pane
    pub fn extract_headings(&self) -> Vec<HeadingItem> {
        let mut headings = Vec::new();
        for (i, line) in self.content().lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                let level = trimmed.chars().take_while(|&c| c == '#').count();
                if level <= 6 && trimmed.chars().nth(level) == Some(' ') {
                    let text = trimmed[level + 1..].trim().to_string();
                    headings.push(HeadingItem {
                        level,
                        text,
                        line_number: i + 1,
                    });
                }
            }
        }
        headings
    }

    /// Word count of note content
    pub fn word_count(&self) -> usize {
        self.content().split_whitespace().count()
    }

    /// Character count
    pub fn char_count(&self) -> usize {
        self.buffer.summary().chars
    }

    /// Line count from Zed SumTree
    pub fn line_count(&self) -> usize {
        self.buffer.lines().max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zed_buffer_sumtree_and_undo() {
        let mut buffer = ZedBuffer::new("First line\nSecond line");
        assert_eq!(buffer.lines(), 2);

        buffer.insert(buffer.len(), "\nThird line");
        assert_eq!(buffer.lines(), 3);
        assert!(buffer.as_str().contains("Third line"));

        let undone = buffer.undo();
        assert!(undone);
        assert_eq!(buffer.lines(), 2);
        assert!(!buffer.as_str().contains("Third line"));

        let redone = buffer.redo();
        assert!(redone);
        assert_eq!(buffer.lines(), 3);
        assert!(buffer.as_str().contains("Third line"));
    }

    #[test]
    fn test_note_wikilinks_and_tags() {
        let note = Note::new(
            "Test Note",
            "test.md",
            "# Test Note\nThis references [[Another Note]] and #rust #gpui",
        );
        let links = note.extract_wikilinks();
        assert_eq!(links, vec!["Another Note"]);
        assert!(note.tags.contains(&"rust".to_string()));
        assert!(note.tags.contains(&"gpui".to_string()));
    }

    #[test]
    fn zed_buffer_replace_range_raw_updates_text_and_summary() {
        let mut buf = ZedBuffer::new("hello\n世界");
        assert_eq!(buf.summary().bytes, "hello\n世界".len());
        assert_eq!(buf.summary().chars, 8);
        assert_eq!(buf.summary().newlines, 1);

        // Replace middle ASCII + newline with CJK + two newlines
        buf.replace_range_raw(5..6, "\n新行\n");
        let expected = "hello\n新行\n世界";
        assert_eq!(buf.as_str(), expected);
        assert_eq!(buf.summary(), TextSummary::of_text(expected));
    }

    #[test]
    fn extract_frontmatter_aliases_and_note_diff_work() {
        let doc = "---\ntitle: Foo\naliases:\n  - Bar\n  - \"Baz Qux\" # inline\n---\n# Heading\n";
        let mut note = Note::new("Foo", "Foo.md", doc);
        assert_eq!(note.aliases, vec!["Bar".to_string(), "Baz Qux".to_string()]);

        let arc: Arc<str> = Arc::from(doc.replace("# Heading", "# Heading updated"));
        let diff = note.set_content_arc(arc);
        assert_eq!(diff, (false, false, false));

        let arc2: Arc<str> = Arc::from(format!("{}\n[[Target]] #tag1", note.content()));
        let diff2 = note.set_content_arc(arc2);
        assert_eq!(diff2, (true, true, false));
    }
}
