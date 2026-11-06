//! Pure single-line text model behind `LineInput`: boundaries, word classes, password masking,
//! selection, undo and redo. No gpui here, so every rule is unit tested.
//!
//! All offsets are UTF-8 byte offsets into `text`, always on char boundaries.

use std::ops::Range;

/// Glyph drawn for each character in password mode.
pub const BULLET: char = '•';

/// Coarse class used by word moves and double click selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharClass {
    /// Latin letters, digits and `_`.
    Word,
    /// CJK ideographs, kana and hangul: each character is its own word.
    Cjk,
    Space,
    Punct,
}

pub fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if is_cjk(c) {
        CharClass::Cjk
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // kana
        | 0x3400..=0x4DBF // CJK extension A
        | 0x4E00..=0x9FFF // CJK unified ideographs
        | 0xAC00..=0xD7AF // hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
        | 0x3000..=0x303F // CJK punctuation (own class is fine: each mark is a unit)
        | 0xFF00..=0xFFEF // full-width forms
    )
}

/// Byte offset of the character before `i` (0 at the start).
pub fn prev_boundary(text: &str, i: usize) -> usize {
    let i = snap(text, i);
    text[..i]
        .char_indices()
        .next_back()
        .map_or(0, |(idx, _)| idx)
}

/// Byte offset of the character after `i` (the end at the end).
pub fn next_boundary(text: &str, i: usize) -> usize {
    let i = snap(text, i);
    match text[i..].chars().next() {
        Some(c) => i + c.len_utf8(),
        None => text.len(),
    }
}

/// Start of the word before `i`: skips spaces, then one CJK character or one run of the same class.
pub fn prev_word_start(text: &str, i: usize) -> usize {
    let mut pos = snap(text, i);
    // Skip whitespace backwards.
    while pos > 0 {
        let p = prev_boundary(text, pos);
        if char_class(text[p..].chars().next().unwrap_or(' ')) != CharClass::Space {
            break;
        }
        pos = p;
    }
    if pos == 0 {
        return 0;
    }
    let first = prev_boundary(text, pos);
    let class = char_class(text[first..].chars().next().unwrap_or(' '));
    pos = first;
    if class == CharClass::Cjk {
        return pos;
    }
    while pos > 0 {
        let p = prev_boundary(text, pos);
        let c = text[p..].chars().next().unwrap_or(' ');
        if char_class(c) != class {
            break;
        }
        pos = p;
    }
    pos
}

/// End of the word after `i`: skips spaces, then one CJK character or one run of the same class.
pub fn next_word_end(text: &str, i: usize) -> usize {
    let mut pos = snap(text, i);
    while pos < text.len() {
        let c = text[pos..].chars().next().unwrap_or(' ');
        if char_class(c) != CharClass::Space {
            break;
        }
        pos += c.len_utf8();
    }
    if pos >= text.len() {
        return text.len();
    }
    let first = text[pos..].chars().next().unwrap_or(' ');
    let class = char_class(first);
    pos += first.len_utf8();
    if class == CharClass::Cjk {
        return pos;
    }
    while pos < text.len() {
        let c = text[pos..].chars().next().unwrap_or(' ');
        if char_class(c) != class {
            break;
        }
        pos += c.len_utf8();
    }
    pos
}

/// Range selected by a double click at byte `i`: the run of the same class under the caret.
/// At the very end the run before the end is used.
pub fn word_range_at(text: &str, i: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }
    let i = snap(text, i);
    // Prefer the character after the caret; at the end use the one before it.
    let probe = if i < text.len() {
        i
    } else {
        prev_boundary(text, i)
    };
    let class = char_class(text[probe..].chars().next().unwrap_or(' '));
    let mut start = probe;
    while start > 0 {
        let p = prev_boundary(text, start);
        let c = text[p..].chars().next().unwrap_or(' ');
        if char_class(c) != class || class == CharClass::Cjk {
            break;
        }
        start = p;
    }
    let mut end = next_boundary(text, probe);
    if class != CharClass::Cjk {
        while end < text.len() {
            let c = text[end..].chars().next().unwrap_or(' ');
            if char_class(c) != class {
                break;
            }
            end += c.len_utf8();
        }
    }
    start..end
}

/// Text drawn for `text`: the same text, or one bullet per character in password mode.
pub fn display_text(text: &str, password: bool) -> String {
    if password {
        text.chars().map(|_| BULLET).collect()
    } else {
        text.to_string()
    }
}

/// Byte offset in the displayed text that corresponds to byte `src` of `text`.
pub fn display_index(text: &str, src: usize, password: bool) -> usize {
    let src = snap(text, src);
    if !password {
        return src;
    }
    let chars = text[..src].chars().count();
    chars * BULLET.len_utf8()
}

/// Byte offset in `text` nearest to byte `disp` of the displayed text.
pub fn source_index(text: &str, disp: usize, password: bool) -> usize {
    if !password {
        return disp.min(text.len());
    }
    let chars = disp / BULLET.len_utf8();
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(idx, _)| idx)
}

/// Number of characters in `text`.
pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// The part of `insert` that still fits when `removed` characters leave a text of `chars` characters.
/// `max` of `None` means no limit.
pub fn fit_insert(chars: usize, removed: usize, insert: &str, max: Option<usize>) -> String {
    let Some(max) = max else {
        return insert.to_string();
    };
    let room = max.saturating_sub(chars.saturating_sub(removed));
    insert.chars().take(room).collect()
}

/// Clipboard text as one line: line breaks become spaces.
pub fn single_line(s: &str) -> String {
    s.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// How an edit joins the undo history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Delete,
    /// Paste, cut, IME composition, programmatic edits: always its own step.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    anchor: usize,
    head: usize,
}

/// Text, selection and history of one line input.
#[derive(Clone, Debug, Default)]
pub struct TextState {
    text: String,
    anchor: usize,
    head: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_kind: Option<EditKind>,
    max_len: Option<usize>,
}

/// Most undo steps kept.
pub const UNDO_LIMIT: usize = 200;

impl TextState {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            anchor: text.len(),
            head: text.len(),
            ..Self::default()
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn max_len(&self) -> Option<usize> {
        self.max_len
    }

    pub fn set_max_len(&mut self, max: Option<usize>) {
        self.max_len = max;
    }

    /// Selected range, ordered.
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn head(&self) -> usize {
        self.head
    }

    pub fn reversed(&self) -> bool {
        self.head < self.anchor
    }

    /// The caret (the moving end of the selection).
    pub fn cursor(&self) -> usize {
        self.head
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.range()]
    }

    /// Sets the selection; both ends are snapped to char boundaries and clamped to the text.
    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        self.anchor = snap(&self.text, anchor);
        self.head = snap(&self.text, head);
    }

    pub fn move_to(&mut self, i: usize) {
        self.set_selection(i, i);
    }

    /// Moves the caret, or extends the selection from its anchor.
    pub fn move_head(&mut self, i: usize, extend: bool) {
        if extend {
            self.head = snap(&self.text, i);
        } else {
            self.move_to(i);
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.head = self.text.len();
    }

    /// Replaces the whole text. Clears the history and moves the caret to the end.
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.anchor = self.text.len();
        self.head = self.text.len();
        self.undo.clear();
        self.redo.clear();
        self.last_kind = None;
    }

    /// Records the state before an edit of `kind`, unless it continues the previous step.
    /// Typing groups until a whitespace character; deletes group with deletes.
    pub fn begin_step(&mut self, kind: EditKind, inserted: &str) {
        let continues = self.last_kind == Some(kind)
            && kind != EditKind::Other
            && !inserted.chars().any(char::is_whitespace);
        if !continues {
            self.undo.push(self.snapshot());
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.last_kind = Some(kind);
        self.redo.clear();
    }

    /// Replaces `range` with `insert` (truncated to `max_len`), leaves the caret after it and
    /// returns true when the text changed. Does not touch the history; callers use `begin_step`.
    pub fn replace_raw(&mut self, range: Range<usize>, insert: &str) -> bool {
        let start = snap(&self.text, range.start);
        let end = snap(&self.text, range.end).max(start);
        let removed = self.text[start..end].chars().count();
        let total = self.text.chars().count();
        let fitted = fit_insert(total, removed, insert, self.max_len);
        if fitted.is_empty() && start == end {
            return false;
        }
        let mut next = String::with_capacity(self.text.len() + fitted.len());
        next.push_str(&self.text[..start]);
        next.push_str(&fitted);
        next.push_str(&self.text[end..]);
        self.text = next;
        let caret = start + fitted.len();
        self.anchor = caret;
        self.head = caret;
        true
    }

    /// Replaces `range` with `insert` as one history step of `kind`. True when the text changed.
    pub fn replace(&mut self, range: Range<usize>, insert: &str, kind: EditKind) -> bool {
        let removed = self
            .text
            .get(range.clone())
            .map_or(0, |s| s.chars().count());
        let fitted = fit_insert(self.text.chars().count(), removed, insert, self.max_len);
        if fitted.is_empty() && removed == 0 {
            return false;
        }
        self.begin_step(kind, insert);
        self.replace_raw(range, insert)
    }

    /// Inserts typed or pasted text at the selection.
    pub fn insert(&mut self, insert: &str, kind: EditKind) -> bool {
        self.replace(self.range(), insert, kind)
    }

    /// Deletes the selection, or the character before the caret when nothing is selected.
    pub fn backspace(&mut self) -> bool {
        let range = if self.range().is_empty() {
            prev_boundary(&self.text, self.head)..self.head
        } else {
            self.range()
        };
        if range.is_empty() {
            return false;
        }
        self.replace(range, "", EditKind::Delete)
    }

    /// Deletes the selection, or the character after the caret.
    pub fn delete_forward(&mut self) -> bool {
        let range = if self.range().is_empty() {
            self.head..next_boundary(&self.text, self.head)
        } else {
            self.range()
        };
        if range.is_empty() {
            return false;
        }
        self.replace(range, "", EditKind::Delete)
    }

    /// Deletes back to the previous word start (or the selection).
    pub fn delete_word_backward(&mut self) -> bool {
        let range = if self.range().is_empty() {
            prev_word_start(&self.text, self.head)..self.head
        } else {
            self.range()
        };
        self.replace(range, "", EditKind::Delete)
    }

    /// Deletes forward to the next word end (or the selection).
    pub fn delete_word_forward(&mut self) -> bool {
        let range = if self.range().is_empty() {
            self.head..next_word_end(&self.text, self.head)
        } else {
            self.range()
        };
        self.replace(range, "", EditKind::Delete)
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(prev);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(next);
        true
    }

    /// Undo steps currently available (for tests and status display).
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            anchor: self.anchor,
            head: self.head,
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.anchor = s.anchor;
        self.head = s.head;
        self.last_kind = None;
    }
}

/// Clamps `i` to the text and moves it back to a char boundary.
fn snap(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_split_latin_cjk_space_punct() {
        assert_eq!(char_class('a'), CharClass::Word);
        assert_eq!(char_class('9'), CharClass::Word);
        assert_eq!(char_class('_'), CharClass::Word);
        assert_eq!(char_class('中'), CharClass::Cjk);
        assert_eq!(char_class('あ'), CharClass::Cjk);
        assert_eq!(char_class(' '), CharClass::Space);
        assert_eq!(char_class('.'), CharClass::Punct);
    }

    #[test]
    fn char_boundaries_step_over_multibyte() {
        let s = "a中b";
        assert_eq!(next_boundary(s, 0), 1);
        assert_eq!(next_boundary(s, 1), 4);
        assert_eq!(next_boundary(s, 4), 5);
        assert_eq!(next_boundary(s, 5), 5);
        assert_eq!(prev_boundary(s, 5), 4);
        assert_eq!(prev_boundary(s, 4), 1);
        assert_eq!(prev_boundary(s, 0), 0);
        // Offsets inside a character snap back.
        assert_eq!(next_boundary(s, 2), 4);
    }

    #[test]
    fn word_moves_on_latin_text() {
        let s = "hello, big world";
        assert_eq!(next_word_end(s, 0), 5); // "hello"
        assert_eq!(next_word_end(s, 5), 6); // ","
        assert_eq!(next_word_end(s, 6), 10); // skip space, "big"
        assert_eq!(next_word_end(s, 16), 16);
        assert_eq!(prev_word_start(s, 16), 11); // "world"
        assert_eq!(prev_word_start(s, 11), 7); // skip space -> "big"
        assert_eq!(prev_word_start(s, 0), 0);
    }

    #[test]
    fn cjk_moves_one_character_at_a_time() {
        let s = "中文 ab";
        assert_eq!(next_word_end(s, 0), 3);
        assert_eq!(next_word_end(s, 3), 6);
        assert_eq!(prev_word_start(s, s.len()), 7); // "ab"
        assert_eq!(prev_word_start(s, 7), 3); // skip space -> "文"
        assert_eq!(prev_word_start(s, 3), 0);
    }

    #[test]
    fn double_click_word_ranges() {
        let s = "foo bar,baz";
        assert_eq!(word_range_at(s, 1), 0..3);
        assert_eq!(word_range_at(s, 3), 3..4); // the space run
        assert_eq!(word_range_at(s, 4), 4..7);
        assert_eq!(word_range_at(s, 7), 7..8); // comma
        assert_eq!(word_range_at(s, s.len()), 8..11); // end: word before
        assert_eq!(word_range_at("", 0), 0..0);
        assert_eq!(word_range_at("中文", 3), 3..6); // one CJK character
    }

    #[test]
    fn password_masking_maps_offsets() {
        let s = "a中b";
        assert_eq!(display_text(s, true), "•••");
        assert_eq!(display_text(s, false), s);
        // Byte 4 is "b" (the third character): two characters before it.
        assert_eq!(display_index(s, 4, true), 2 * BULLET.len_utf8());
        assert_eq!(display_index(s, 4, false), 4);
        assert_eq!(source_index(s, BULLET.len_utf8(), true), 1);
        assert_eq!(source_index(s, 2 * BULLET.len_utf8(), true), 4);
        assert_eq!(source_index(s, 100, true), s.len());
        assert_eq!(source_index(s, 100, false), s.len());
    }

    #[test]
    fn max_len_truncates_inserts() {
        assert_eq!(fit_insert(3, 0, "abcd", Some(5)), "ab");
        assert_eq!(fit_insert(3, 2, "abcd", Some(5)), "abcd");
        assert_eq!(fit_insert(5, 0, "x", Some(5)), "");
        assert_eq!(fit_insert(9, 0, "xyz", None), "xyz");

        let mut t = TextState::new("abc");
        t.set_max_len(Some(4));
        assert!(t.insert("中文", EditKind::Other));
        assert_eq!(t.text(), "abc中");
        assert!(!t.insert("x", EditKind::Typing));
        assert_eq!(t.text(), "abc中");
    }

    #[test]
    fn selection_is_clamped_and_snapped() {
        let mut t = TextState::new("中a");
        t.set_selection(1, 99);
        assert_eq!(t.range(), 0..4);
        assert_eq!(t.selected_text(), "中a");
        t.set_selection(4, 0);
        assert!(t.reversed());
        assert_eq!(t.range(), 0..4);
        assert_eq!(t.cursor(), 0);
    }

    #[test]
    fn typing_and_backspace_across_multibyte() {
        let mut t = TextState::new("");
        t.insert("中", EditKind::Typing);
        t.insert("文", EditKind::Typing);
        assert_eq!(t.text(), "中文");
        assert!(t.backspace());
        assert_eq!(t.text(), "中");
        assert_eq!(t.cursor(), 3);
        t.move_to(0);
        assert!(!t.backspace());
        assert!(t.delete_forward());
        assert_eq!(t.text(), "");
    }

    #[test]
    fn undo_groups_typing_and_redo_restores() {
        let mut t = TextState::new("");
        for c in "abc".chars() {
            t.insert(&c.to_string(), EditKind::Typing);
        }
        t.insert(" ", EditKind::Typing); // whitespace starts a new step
        t.insert("d", EditKind::Typing);
        assert_eq!(t.text(), "abc d");
        assert!(t.undo());
        assert_eq!(t.text(), "abc");
        assert!(t.undo());
        assert_eq!(t.text(), "");
        assert!(!t.undo());
        assert!(t.redo());
        assert_eq!(t.text(), "abc");
        assert!(t.redo());
        assert_eq!(t.text(), "abc d");
        assert!(!t.redo());
    }

    #[test]
    fn new_edit_clears_redo_and_other_kinds_do_not_group() {
        let mut t = TextState::new("");
        t.insert("a", EditKind::Other);
        t.insert("b", EditKind::Other);
        assert_eq!(t.undo_len(), 2);
        assert!(t.undo());
        assert_eq!(t.text(), "a");
        t.insert("z", EditKind::Typing);
        assert!(!t.redo());
        assert_eq!(t.text(), "az");
    }

    #[test]
    fn set_text_resets_history_and_caret() {
        let mut t = TextState::new("x");
        t.insert("y", EditKind::Other);
        t.set_text("hello");
        assert_eq!(t.cursor(), 5);
        assert_eq!(t.undo_len(), 0);
        assert!(!t.undo());
    }

    #[test]
    fn word_deletes() {
        let mut t = TextState::new("one two");
        assert!(t.delete_word_backward());
        assert_eq!(t.text(), "one ");
        t.move_to(0);
        assert!(t.delete_word_forward());
        assert_eq!(t.text(), " ");
    }

    #[test]
    fn single_line_clipboard() {
        assert_eq!(single_line("a\r\nb\nc\rd"), "a b c d");
    }
}
