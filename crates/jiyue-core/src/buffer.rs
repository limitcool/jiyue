use std::sync::Arc;

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
        let text: Arc<str> = Arc::from(initial_text);
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
        self.text = Arc::from(updated);
        self.summary = TextSummary::of_text(&self.text);
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

/// A Markdown note in the vault backed by a ZedBuffer
#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub path: String,
    pub buffer: ZedBuffer,
    pub tags: Vec<String>,
    pub wikilinks: Vec<String>,
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
        };
        note.refresh_metadata();
        note
    }

    pub fn content(&self) -> &str {
        self.buffer.as_str()
    }

    /// Shared handle to the note's text (O(1) clone, zero byte copy).
    pub fn content_arc(&self) -> Arc<str> {
        self.buffer.arc()
    }

    /// Set note content directly and refresh cached tags and wikilinks.
    pub fn set_content(&mut self, new_content: &str) {
        self.buffer = ZedBuffer::new(new_content);
        self.refresh_metadata();
    }

    fn refresh_metadata(&mut self) {
        self.extract_tags();
        self.wikilinks = wikilink_targets(self.content());
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
}
