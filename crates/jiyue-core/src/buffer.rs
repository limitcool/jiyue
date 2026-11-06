use gpui_sum_tree::{ContextLessSummary, Dimension, Item, SumTree};
use serde::{Deserialize, Serialize};

/// Summary of a text chunk stored in Zed's SumTree
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextSummary {
    pub bytes: usize,
    pub chars: usize,
    pub newlines: usize,
}

impl TextSummary {
    /// Total display lines represented by this summary
    pub fn lines(&self) -> usize {
        if self.bytes == 0 {
            0
        } else {
            self.newlines + 1
        }
    }
}

impl ContextLessSummary for TextSummary {
    fn zero() -> Self {
        Self::default()
    }

    fn add_summary(&mut self, other: &Self) {
        self.bytes += other.bytes;
        self.chars += other.chars;
        self.newlines += other.newlines;
    }
}

/// Byte offset dimension for seeking in SumTree
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct ByteOffset(pub usize);

impl<'a> Dimension<'a, TextSummary> for ByteOffset {
    fn zero(_cx: ()) -> Self {
        ByteOffset(0)
    }

    fn add_summary(&mut self, summary: &'a TextSummary, _cx: ()) {
        self.0 += summary.bytes;
    }
}

/// Line row dimension for seeking in SumTree
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Row(pub usize);

impl<'a> Dimension<'a, TextSummary> for Row {
    fn zero(_cx: ()) -> Self {
        Row(0)
    }

    fn add_summary(&mut self, summary: &'a TextSummary, _cx: ()) {
        self.0 += summary.newlines;
    }
}

/// Leaf node item in the SumTree text rope
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextChunk {
    pub text: String,
}

impl TextChunk {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl Item for TextChunk {
    type Summary = TextSummary;

    fn summary(&self, _cx: ()) -> Self::Summary {
        let bytes = self.text.len();
        let chars = self.text.chars().count();
        let newlines = self.text.bytes().filter(|&b| b == b'\n').count();
        TextSummary {
            bytes,
            chars,
            newlines,
        }
    }
}

/// Edit transaction for Zed-style Undo/Redo history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditOperation {
    pub offset: usize,
    pub old_text: String,
    pub new_text: String,
}

/// High-performance text buffer model inspired by Zed's rope and SumTree
#[derive(Debug, Clone)]
pub struct ZedBuffer {
    pub tree: SumTree<TextChunk>,
    pub text: String,
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
        let mut buffer = Self {
            tree: SumTree::new(()),
            text: initial_text.to_string(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        };
        buffer.rebuild_tree();
        buffer
    }

    /// Rebuild SumTree chunks from raw text
    fn rebuild_tree(&mut self) {
        let mut tree = SumTree::new(());
        // Chunk by lines to allow fine-grained seeking and balancing
        if self.text.is_empty() {
            tree.push(TextChunk::new(""), ());
        } else {
            for line in self.text.split_inclusive('\n') {
                tree.push(TextChunk::new(line), ());
            }
        }
        self.tree = tree;
    }

    /// Total text as str
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Insert text at byte offset with undo tracking
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

        self.text.insert_str(offset, new_str);
        self.rebuild_tree();
    }

    /// Delete text range with undo tracking
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

        self.text.replace_range(offset..end, "");
        self.rebuild_tree();
    }

    /// Undo last edit operation
    pub fn undo(&mut self) -> bool {
        if let Some(op) = self.undo_stack.pop() {
            let offset = op.offset;
            let len = op.new_text.len();
            let end = (offset + len).min(self.text.len());
            self.text.replace_range(offset..end, &op.old_text);
            self.redo_stack.push(op);
            self.rebuild_tree();
            true
        } else {
            false
        }
    }

    /// Redo last undone operation
    pub fn redo(&mut self) -> bool {
        if let Some(op) = self.redo_stack.pop() {
            let offset = op.offset;
            let len = op.old_text.len();
            let end = (offset + len).min(self.text.len());
            self.text.replace_range(offset..end, &op.new_text);
            self.undo_stack.push(op);
            self.rebuild_tree();
            true
        } else {
            false
        }
    }

    /// Summary computed directly by the SumTree root
    pub fn summary(&self) -> TextSummary {
        self.tree.summary().clone()
    }

    pub fn lines(&self) -> usize {
        self.summary().lines()
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
            if !target.is_empty() && !links.contains(&target.to_string()) {
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
        let id = path.to_lowercase().replace(' ', "-");
        let buffer = ZedBuffer::new(content);
        let mut note = Self {
            id,
            title: title.to_string(),
            path: path.to_string(),
            buffer,
            tags: Vec::new(),
        };
        note.extract_tags();
        note
    }

    pub fn content(&self) -> &str {
        self.buffer.as_str()
    }

    /// Set note content directly
    pub fn set_content(&mut self, new_content: &str) {
        self.buffer = ZedBuffer::new(new_content);
        self.extract_tags();
    }

    /// Extract hashtags `#tag` from content
    pub fn extract_tags(&mut self) {
        let mut tags = Vec::new();
        for word in self.content().split_whitespace() {
            if word.starts_with('#') && word.len() > 1 && !word.starts_with("##") {
                let tag = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
                if !tag.is_empty() && !tags.contains(&tag.to_string()) {
                    tags.push(tag.to_string());
                }
            }
        }
        self.tags = tags;
    }

    /// Extract wikilinks `[[Note Title]]` from note content
    pub fn extract_wikilinks(&self) -> Vec<String> {
        wikilink_targets(self.content())
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
