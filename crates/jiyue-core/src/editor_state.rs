//! Editing core: text buffer + cursor/selection + undo history. Pure data, no UI.
//! Owned by the editor-state agent.

use std::collections::VecDeque;
use std::ops::Range;

use crate::buffer::ZedBuffer;

/// Maximum number of undo transactions kept.
const MAX_HISTORY: usize = 1000;
const ZWJ: char = '\u{200D}';

/// Selection in UTF-8 byte offsets. `head` is the caret; `anchor == head` means no selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn collapsed(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.head
    }

    /// Ordered byte range covered by the selection.
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

/// How an edit may merge with the previous history transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Typing,
    Backspace,
    Delete,
    Other,
}

/// One undo step: a single replacement plus the selection and state marks around it.
#[derive(Debug, Clone)]
struct Transaction {
    offset: usize,
    deleted: String,
    inserted: String,
    before: Selection,
    after: Selection,
    before_state: u64,
    after_state: u64,
    kind: Kind,
}

/// Text, selection, goal column and undo history for one editor.
#[derive(Debug, Clone)]
pub struct EditorState {
    buf: ZedBuffer,
    selection: Selection,
    goal_col: Option<usize>,
    undo_stack: VecDeque<Transaction>,
    redo_stack: Vec<Transaction>,
    /// True while the last transaction may still absorb the next compatible edit.
    can_merge: bool,
    /// Identifies the current document state; a new id is minted on every change.
    state_id: u64,
    /// The state id recorded by the last save (or load).
    saved_state: u64,
    next_state: u64,
}

impl EditorState {
    /// New editor with the caret at 0. `\r\n` is normalised to `\n`.
    pub fn new(text: &str) -> Self {
        let mut state = Self {
            buf: ZedBuffer::new(""),
            selection: Selection::default(),
            goal_col: None,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            can_merge: false,
            state_id: 0,
            saved_state: 0,
            next_state: 0,
        };
        state.set_text(text);
        state
    }

    pub fn text(&self) -> &str {
        self.buf.as_str()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Sets the selection; offsets are clamped down to char boundaries and the text length.
    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        let sel = self.clamp_sel(Selection { anchor, head });
        self.set_sel_raw(sel);
    }

    pub fn cursor(&self) -> usize {
        self.selection.head
    }

    pub fn selected_text(&self) -> &str {
        self.text().get(self.selection.range()).unwrap_or("")
    }

    pub fn has_selection(&self) -> bool {
        !self.selection.is_collapsed()
    }

    // ----- editing ---------------------------------------------------------

    /// Inserts text at the caret, replacing the selection first. A single typed character
    /// is grouped with neighbouring typing in the undo history.
    pub fn insert_text(&mut self, s: &str) {
        self.insert_impl(s, true);
    }

    /// Like `insert_text` but always its own undo step.
    pub fn paste(&mut self, s: &str) {
        self.insert_impl(s, false);
    }

    fn insert_impl(&mut self, s: &str, typing_ok: bool) {
        let s = normalize(s);
        let range = self.selection.range();
        let kind = if typing_ok && !self.has_selection() && is_single_typed_char(&s) {
            Kind::Typing
        } else {
            Kind::Other
        };
        let caret = range.start + s.len();
        self.apply_edit(
            range.start,
            range.end,
            &s,
            kind,
            Selection::collapsed(caret),
        );
    }

    /// Generic replacement of `start..end` with `s` (one undo step). Offsets are clamped and
    /// `new_selection` is interpreted in the text after the replacement.
    pub fn replace_range(&mut self, start: usize, end: usize, s: &str, new_selection: Selection) {
        let s = normalize(s);
        let a = self.clamp_offset(start.min(end));
        let b = self.clamp_offset(start.max(end)).max(a);
        if a == b && s.is_empty() {
            let sel = self.clamp_sel(new_selection);
            self.set_sel_raw(sel);
            return;
        }
        self.apply_edit(a, b, &s, Kind::Other, new_selection);
    }

    /// Deletes the selection, or the character before the caret.
    pub fn backspace(&mut self) {
        if self.has_selection() {
            self.delete_selection();
        } else {
            let c = self.cursor();
            if c > 0 {
                let start = prev_unit_start(self.text(), c);
                self.apply_edit(start, c, "", Kind::Backspace, Selection::collapsed(start));
            }
        }
    }

    /// Deletes the selection, or the character after the caret.
    pub fn delete_forward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
        } else {
            let c = self.cursor();
            if c < self.len() {
                let end = unit_end(self.text(), c);
                self.apply_edit(c, end, "", Kind::Delete, Selection::collapsed(c));
            }
        }
    }

    /// Ctrl+Backspace: deletes the selection, or back to the start of the previous word.
    pub fn delete_word_backward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
        } else {
            let c = self.cursor();
            let start = word_left(self.text(), c);
            self.apply_edit(start, c, "", Kind::Other, Selection::collapsed(start));
        }
    }

    /// Ctrl+Delete: deletes the selection, or forward to the end of the next word.
    pub fn delete_word_forward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
        } else {
            let c = self.cursor();
            let end = word_right(self.text(), c);
            self.apply_edit(c, end, "", Kind::Other, Selection::collapsed(c));
        }
    }

    /// Deletes from the start of the caret's line to the caret (or the selection).
    pub fn delete_to_line_start(&mut self) {
        if self.has_selection() {
            self.delete_selection();
        } else {
            let c = self.cursor();
            let ls = self.line_start_of(c);
            self.apply_edit(ls, c, "", Kind::Other, Selection::collapsed(ls));
        }
    }

    /// Removes every line touched by the selection, including its newline.
    pub fn delete_line(&mut self) {
        let r = self.selection.range();
        let ls = self.line_start_of(r.start);
        let le = self.line_end_of(r.end);
        let len = self.len();
        let (a, b, caret) = if le < len {
            (ls, le + 1, ls)
        } else if ls > 0 {
            (ls - 1, le, ls - 1)
        } else {
            (0, len, 0)
        };
        self.apply_edit(a, b, "", Kind::Other, Selection::collapsed(caret));
    }

    /// Cuts the selection and returns it. Without a selection nothing is removed and "" is returned.
    pub fn cut(&mut self) -> String {
        let text = self.copy();
        if self.has_selection() {
            self.delete_selection();
        }
        text
    }

    /// Returns the selected text ("" without a selection).
    pub fn copy(&self) -> String {
        self.selected_text().to_string()
    }

    /// Duplicates the lines touched by the selection below them; the selection moves to the copy.
    pub fn duplicate_line(&mut self) {
        let r = self.selection.range();
        let ls = self.line_start_of(r.start);
        let le = self.line_end_of(r.end);
        let block = self.text()[ls..le].to_string();
        let mut ins = String::with_capacity(block.len() + 1);
        ins.push('\n');
        ins.push_str(&block);
        let shift = ins.len();
        let sel = Selection {
            anchor: self.selection.anchor + shift,
            head: self.selection.head + shift,
        };
        self.apply_edit(le, le, &ins, Kind::Other, sel);
    }

    /// Swaps the touched lines with the line above. No-op on the first line.
    pub fn move_line_up(&mut self) {
        let r = self.selection.range();
        let ls = self.line_start_of(r.start);
        let le = self.line_end_of(r.end);
        if ls == 0 {
            return;
        }
        let pls = self.line_start_of(ls - 1);
        let prev = self.text()[pls..ls - 1].to_string();
        let block = self.text()[ls..le].to_string();
        let new = format!("{block}\n{prev}");
        let shift = ls - pls;
        let sel = Selection {
            anchor: self.selection.anchor.saturating_sub(shift),
            head: self.selection.head.saturating_sub(shift),
        };
        self.apply_edit(pls, le, &new, Kind::Other, sel);
    }

    /// Swaps the touched lines with the line below. No-op on the last line.
    pub fn move_line_down(&mut self) {
        let r = self.selection.range();
        let ls = self.line_start_of(r.start);
        let le = self.line_end_of(r.end);
        if le >= self.len() {
            return;
        }
        let nls = le + 1;
        let nle = self.line_end_of(nls);
        let block = self.text()[ls..le].to_string();
        let next = self.text()[nls..nle].to_string();
        let new = format!("{next}\n{block}");
        let shift = nle - le;
        let sel = Selection {
            anchor: self.selection.anchor + shift,
            head: self.selection.head + shift,
        };
        self.apply_edit(ls, nle, &new, Kind::Other, sel);
    }

    // ----- movement --------------------------------------------------------

    /// Left by one grapheme-like unit. Without `extend`, a selection collapses to its start.
    pub fn move_left(&mut self, extend: bool) {
        if !extend && self.has_selection() {
            let t = self.selection.range().start;
            self.move_head(t, false);
        } else {
            let t = prev_unit_start(self.text(), self.cursor());
            self.move_head(t, extend);
        }
    }

    /// Right by one grapheme-like unit. Without `extend`, a selection collapses to its end.
    pub fn move_right(&mut self, extend: bool) {
        if !extend && self.has_selection() {
            let t = self.selection.range().end;
            self.move_head(t, false);
        } else {
            let t = unit_end(self.text(), self.cursor());
            self.move_head(t, extend);
        }
    }

    /// Up one line, keeping the goal column (in chars).
    pub fn move_up(&mut self, extend: bool) {
        let (row, col) = self.offset_to_row_col(self.cursor());
        let goal = self.goal_col.unwrap_or(col);
        let target = if row == 0 {
            0
        } else {
            self.row_col_to_offset(row - 1, goal)
        };
        self.move_head(target, extend);
        self.goal_col = Some(goal);
    }

    /// Down one line, keeping the goal column (in chars).
    pub fn move_down(&mut self, extend: bool) {
        let (row, col) = self.offset_to_row_col(self.cursor());
        let goal = self.goal_col.unwrap_or(col);
        let target = if row + 1 >= self.line_count() {
            self.len()
        } else {
            self.row_col_to_offset(row + 1, goal)
        };
        self.move_head(target, extend);
        self.goal_col = Some(goal);
    }

    pub fn move_word_left(&mut self, extend: bool) {
        let t = word_left(self.text(), self.cursor());
        self.move_head(t, extend);
    }

    pub fn move_word_right(&mut self, extend: bool) {
        let t = word_right(self.text(), self.cursor());
        self.move_head(t, extend);
    }

    /// Smart home: first press goes to the first non-space character, a second press to column 0.
    pub fn move_line_start(&mut self, extend: bool) {
        let c = self.cursor();
        let ls = self.line_start_of(c);
        let le = self.line_end_of(c);
        let line = &self.text()[ls..le];
        let fs = ls
            + line
                .find(|ch: char| !ch.is_whitespace())
                .unwrap_or(line.len());
        let target = if c == fs && fs != ls { ls } else { fs };
        self.move_head(target, extend);
    }

    pub fn move_line_end(&mut self, extend: bool) {
        let t = self.line_end_of(self.cursor());
        self.move_head(t, extend);
    }

    pub fn move_doc_start(&mut self, extend: bool) {
        self.move_head(0, extend);
    }

    pub fn move_doc_end(&mut self, extend: bool) {
        let t = self.len();
        self.move_head(t, extend);
    }

    pub fn select_all(&mut self) {
        let len = self.len();
        self.set_sel_raw(Selection {
            anchor: 0,
            head: len,
        });
    }

    /// Selects the run of word characters (or whitespace, or punctuation, or one script) at `offset`.
    pub fn select_word_at(&mut self, offset: usize) {
        let range = word_range_at(self.text(), offset);
        self.set_sel_raw(Selection {
            anchor: range.start,
            head: range.end,
        });
    }

    /// Selects the whole line at `offset`, including its trailing newline when it has one.
    pub fn select_line_at(&mut self, offset: usize) {
        let ls = self.line_start_of(offset);
        let le = self.line_end_of(offset);
        let end = if le < self.len() { le + 1 } else { le };
        self.set_sel_raw(Selection {
            anchor: ls,
            head: end,
        });
    }

    // ----- geometry --------------------------------------------------------

    /// Number of lines; an empty document has one line, "a\n" has two.
    pub fn line_count(&self) -> usize {
        self.text().bytes().filter(|&b| b == b'\n').count() + 1
    }

    /// Byte range of a row, without its trailing newline. Rows past the end give an empty range at EOF.
    pub fn line_range(&self, row: usize) -> Range<usize> {
        let text = self.text();
        let mut start = 0;
        for _ in 0..row {
            match text[start..].find('\n') {
                Some(i) => start += i + 1,
                None => return text.len()..text.len(),
            }
        }
        let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        start..end
    }

    /// Row and column (column counted in chars).
    pub fn offset_to_row_col(&self, offset: usize) -> (usize, usize) {
        let text = self.text();
        let o = self.clamp_offset(offset);
        let row = text[..o].bytes().filter(|&b| b == b'\n').count();
        let ls = line_start_in(text, o);
        let col = text[ls..o].chars().count();
        (row, col)
    }

    /// Offset of (row, col); row is clamped to the last line and col to the line length.
    pub fn row_col_to_offset(&self, row: usize, col: usize) -> usize {
        let row = row.min(self.line_count() - 1);
        let r = self.line_range(row);
        match self.text()[r.clone()].char_indices().nth(col) {
            Some((i, _)) => r.start + i,
            None => r.end,
        }
    }

    // ----- history ---------------------------------------------------------

    pub fn undo(&mut self) -> bool {
        let Some(tx) = self.undo_stack.pop_back() else {
            return false;
        };
        let end = (tx.offset + tx.inserted.len()).min(self.len());
        self.raw_replace(tx.offset, end, &tx.deleted);
        self.selection = self.clamp_sel(tx.before);
        self.state_id = tx.before_state;
        self.goal_col = None;
        self.can_merge = false;
        self.redo_stack.push(tx);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(tx) = self.redo_stack.pop() else {
            return false;
        };
        let end = (tx.offset + tx.deleted.len()).min(self.len());
        self.raw_replace(tx.offset, end, &tx.inserted);
        self.selection = self.clamp_sel(tx.after);
        self.state_id = tx.after_state;
        self.goal_col = None;
        self.can_merge = false;
        self.push_undo(tx);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// True when the text differs from the last saved (or loaded) state.
    pub fn is_dirty(&self) -> bool {
        self.state_id != self.saved_state
    }

    pub fn mark_saved(&mut self) {
        self.saved_state = self.state_id;
    }

    /// Replaces the whole text, clears history and selection, and marks the result as saved.
    pub fn set_text(&mut self, text: &str) {
        self.buf = ZedBuffer::new(&normalize(text));
        self.selection = Selection::default();
        self.goal_col = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.can_merge = false;
        let st = self.fresh_state();
        self.state_id = st;
        self.saved_state = st;
    }

    // ----- internals -------------------------------------------------------

    fn fresh_state(&mut self) -> u64 {
        self.next_state = self.next_state.wrapping_add(1);
        self.next_state
    }

    fn clamp_offset(&self, offset: usize) -> usize {
        floor_boundary(self.text(), offset)
    }

    fn clamp_sel(&self, sel: Selection) -> Selection {
        Selection {
            anchor: self.clamp_offset(sel.anchor),
            head: self.clamp_offset(sel.head),
        }
    }

    fn line_start_of(&self, offset: usize) -> usize {
        line_start_in(self.text(), offset)
    }

    fn line_end_of(&self, offset: usize) -> usize {
        line_end_in(self.text(), offset)
    }

    /// Sets the selection without touching history, unless it actually changes.
    fn set_sel_raw(&mut self, sel: Selection) {
        let sel = self.clamp_sel(sel);
        if sel != self.selection {
            self.can_merge = false;
            self.goal_col = None;
            self.selection = sel;
        }
    }

    fn move_head(&mut self, target: usize, extend: bool) {
        let head = self.clamp_offset(target);
        let anchor = if extend { self.selection.anchor } else { head };
        self.can_merge = false;
        self.goal_col = None;
        self.set_sel_raw(Selection { anchor, head });
    }

    fn delete_selection(&mut self) {
        let r = self.selection.range();
        self.apply_edit(
            r.start,
            r.end,
            "",
            Kind::Other,
            Selection::collapsed(r.start),
        );
    }

    /// Replaces the text without recording history (buffer and its own stacks only).
    fn raw_replace(&mut self, start: usize, end: usize, s: &str) {
        if end > start {
            self.buf.delete(start, end - start);
        }
        if !s.is_empty() {
            self.buf.insert(start, s);
        }
        self.buf.undo_stack.clear();
        self.buf.redo_stack.clear();
    }

    fn push_undo(&mut self, tx: Transaction) {
        self.undo_stack.push_back(tx);
        while self.undo_stack.len() > MAX_HISTORY {
            self.undo_stack.pop_front();
        }
    }

    /// The single funnel for every text change. Records history and selection.
    fn apply_edit(&mut self, start: usize, end: usize, s: &str, kind: Kind, new_sel: Selection) {
        let start = self.clamp_offset(start);
        let end = self.clamp_offset(end).max(start);
        if start == end && s.is_empty() {
            return;
        }
        let before = self.selection;
        let old_state = self.state_id;
        let deleted = self.text().get(start..end).unwrap_or("").to_string();
        self.raw_replace(start, end, s);

        let after = self.clamp_sel(new_sel);
        let new_state = self.fresh_state();
        self.state_id = new_state;
        self.selection = after;
        self.goal_col = None;
        self.redo_stack.clear();

        let merged = self.can_merge && self.try_merge(kind, start, end, &deleted, s);
        if merged {
            if let Some(last) = self.undo_stack.back_mut() {
                last.after = after;
                last.after_state = new_state;
            }
        } else {
            self.push_undo(Transaction {
                offset: start,
                deleted,
                inserted: s.to_string(),
                before,
                after,
                before_state: old_state,
                after_state: new_state,
                kind,
            });
        }
        self.can_merge = matches!(kind, Kind::Typing | Kind::Backspace | Kind::Delete);
    }

    /// Folds an edit into the top transaction when the grouping rules allow it.
    fn try_merge(
        &mut self,
        kind: Kind,
        start: usize,
        end: usize,
        deleted: &str,
        inserted: &str,
    ) -> bool {
        let Some(last) = self.undo_stack.back_mut() else {
            return false;
        };
        if last.kind != kind {
            return false;
        }
        match kind {
            Kind::Typing => {
                if last.inserted.is_empty() || last.offset + last.inserted.len() != start {
                    return false;
                }
                let ends_blank = last
                    .inserted
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
                let starts_word = inserted.chars().next().is_some_and(|c| !c.is_whitespace());
                if ends_blank && starts_word {
                    return false;
                }
                last.inserted.push_str(inserted);
            }
            Kind::Backspace => {
                if last.offset != end {
                    return false;
                }
                last.deleted.insert_str(0, deleted);
                last.offset = start;
            }
            Kind::Delete => {
                if last.offset != start {
                    return false;
                }
                last.deleted.push_str(deleted);
            }
            Kind::Other => return false,
        }
        true
    }
}

// ----- free helpers (pure string functions) -----------------------------------

fn normalize(s: &str) -> String {
    if s.contains("\r\n") {
        s.replace("\r\n", "\n")
    } else {
        s.to_string()
    }
}

fn is_single_typed_char(s: &str) -> bool {
    let mut chars = s.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c != '\n')
}

/// Largest char boundary <= pos (and <= len).
fn floor_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn line_start_in(s: &str, offset: usize) -> usize {
    let o = floor_boundary(s, offset);
    s[..o].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end_in(s: &str, offset: usize) -> usize {
    let o = floor_boundary(s, offset);
    s[o..].find('\n').map_or(s.len(), |i| o + i)
}

/// Combining marks and variation selectors that attach to the preceding character.
fn is_extend(c: char) -> bool {
    matches!(
        c as u32,
        0x0300..=0x036F | 0x3099..=0x309A | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF
    )
}

fn is_joiner(c: char) -> bool {
    c == ZWJ || is_extend(c)
}

/// End of the grapheme-like unit starting at `start` (a char, its combining marks, and ZWJ sequences).
/// Units never cross a newline.
fn unit_end(s: &str, start: usize) -> usize {
    let start = floor_boundary(s, start);
    let Some(rest) = s.get(start..) else {
        return start;
    };
    let mut it = rest.char_indices().peekable();
    let Some((_, first)) = it.next() else {
        return start;
    };
    let mut end = start + first.len_utf8();
    if first == '\n' {
        return end;
    }
    let mut after_zwj = first == ZWJ;
    while let Some(&(i, c)) = it.peek() {
        let c_end = start + i + c.len_utf8();
        if after_zwj {
            if c == '\n' {
                break;
            }
            after_zwj = false;
            end = c_end;
            it.next();
        } else if c == ZWJ {
            after_zwj = true;
            end = c_end;
            it.next();
        } else if is_extend(c) {
            end = c_end;
            it.next();
        } else {
            break;
        }
    }
    end
}

/// Start of the unit that precedes `offset`.
fn prev_unit_start(s: &str, offset: usize) -> usize {
    let offset = floor_boundary(s, offset);
    if offset == 0 {
        return 0;
    }
    if s.as_bytes()[offset - 1] == b'\n' {
        return offset - 1;
    }
    let mut pos = line_start_in(s, offset);
    loop {
        let end = unit_end(s, pos);
        if end >= offset || end <= pos {
            return pos;
        }
        pos = end;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Newline,
    Word,
    Han,
    Hiragana,
    Katakana,
    Hangul,
    Punct,
}

fn class_of(c: char) -> Class {
    let u = c as u32;
    if c == '\n' {
        Class::Newline
    } else if c.is_whitespace() {
        Class::Space
    } else if matches!(u, 0x3040..=0x309F) {
        Class::Hiragana
    } else if c == '\u{30FB}' {
        Class::Punct
    } else if matches!(u, 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D) {
        Class::Katakana
    } else if matches!(u, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF) {
        Class::Hangul
    } else if matches!(
        u,
        0x3005 | 0x3007 | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    ) {
        Class::Han
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

fn is_blank(k: Class) -> bool {
    matches!(k, Class::Space | Class::Newline)
}

/// Ctrl+Left target: skip blanks, then one run of the same class.
fn word_left(s: &str, pos: usize) -> usize {
    let mut p = floor_boundary(s, pos);
    let mut cls: Option<Class> = None;
    while let Some(c) = s[..p].chars().next_back() {
        let width = c.len_utf8();
        if is_joiner(c) {
            p -= width;
            continue;
        }
        let k = class_of(c);
        match cls {
            None if is_blank(k) => p -= width,
            None => {
                cls = Some(k);
                p -= width;
            }
            Some(cur) if k == cur => p -= width,
            Some(_) => break,
        }
    }
    p
}

/// Ctrl+Right target: skip blanks, then one run of the same class.
fn word_right(s: &str, pos: usize) -> usize {
    let mut p = floor_boundary(s, pos);
    let mut cls: Option<Class> = None;
    while let Some(c) = s[p..].chars().next() {
        let width = c.len_utf8();
        if is_joiner(c) {
            p += width;
            continue;
        }
        let k = class_of(c);
        match cls {
            None if is_blank(k) => p += width,
            None => {
                cls = Some(k);
                p += width;
            }
            Some(cur) if k == cur => p += width,
            Some(_) => break,
        }
    }
    p
}

/// Run of same-class characters around `offset` (double-click selection).
fn word_range_at(s: &str, offset: usize) -> Range<usize> {
    let o = floor_boundary(s, offset);
    let cls = s[o..]
        .chars()
        .find(|c| !is_joiner(*c))
        .or_else(|| s[..o].chars().rev().find(|c| !is_joiner(*c)))
        .map(class_of);
    let Some(cls) = cls else {
        return o..o;
    };
    let mut start = o;
    while let Some(c) = s[..start].chars().next_back() {
        if is_joiner(c) || class_of(c) == cls {
            start -= c.len_utf8();
        } else {
            break;
        }
    }
    let mut end = o;
    while let Some(c) = s[end..].chars().next() {
        if is_joiner(c) || class_of(c) == cls {
            end += c.len_utf8();
        } else {
            break;
        }
    }
    start..end
}

// ----- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Editor with the caret at `caret`.
    fn at(text: &str, caret: usize) -> EditorState {
        let mut e = EditorState::new(text);
        e.set_selection(caret, caret);
        e
    }

    /// Editor with selection anchor..head.
    fn sel(text: &str, anchor: usize, head: usize) -> EditorState {
        let mut e = EditorState::new(text);
        e.set_selection(anchor, head);
        e
    }

    fn type_chars(e: &mut EditorState, s: &str) {
        for c in s.chars() {
            e.insert_text(&c.to_string());
        }
    }

    // --- construction and basic state ---

    #[test]
    fn new_puts_caret_at_zero() {
        let e = EditorState::new("abc");
        assert_eq!(e.cursor(), 0);
        assert!(!e.has_selection());
        assert_eq!(e.selection(), Selection { anchor: 0, head: 0 });
    }

    #[test]
    fn new_empty_document() {
        let e = EditorState::new("");
        assert_eq!(e.len(), 0);
        assert!(e.is_empty());
        assert_eq!(e.line_count(), 1);
    }

    #[test]
    fn new_normalises_crlf() {
        let e = EditorState::new("a\r\nb");
        assert_eq!(e.text(), "a\nb");
        assert_eq!(e.len(), 3);
    }

    #[test]
    fn new_keeps_lone_cr() {
        let e = EditorState::new("a\rb");
        assert_eq!(e.text(), "a\rb");
    }

    #[test]
    fn selection_accessor_round_trip() {
        let e = sel("hello", 1, 4);
        assert_eq!(e.selection(), Selection { anchor: 1, head: 4 });
        assert_eq!(e.cursor(), 4);
        assert!(e.has_selection());
        assert_eq!(e.selected_text(), "ell");
    }

    #[test]
    fn selection_range_is_ordered_for_backward_selection() {
        let e = sel("hello", 4, 1);
        assert_eq!(e.selection().range(), 1..4);
        assert_eq!(e.selected_text(), "ell");
    }

    // --- insertion ---

    #[test]
    fn insert_at_caret() {
        let mut e = at("ac", 1);
        e.insert_text("b");
        assert_eq!(e.text(), "abc");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn insert_multi_char_moves_caret_to_end_of_insert() {
        let mut e = at("", 0);
        e.insert_text("hello");
        assert_eq!(e.cursor(), 5);
    }

    #[test]
    fn insert_replaces_selection() {
        let mut e = sel("hello world", 0, 5);
        e.insert_text("bye");
        assert_eq!(e.text(), "bye world");
        assert_eq!(e.cursor(), 3);
        assert!(!e.has_selection());
    }

    #[test]
    fn insert_normalises_crlf_in_input() {
        let mut e = at("", 0);
        e.insert_text("x\r\ny");
        assert_eq!(e.text(), "x\ny");
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn insert_empty_string_is_noop_without_history() {
        let mut e = at("abc", 1);
        e.insert_text("");
        assert_eq!(e.text(), "abc");
        assert!(!e.can_undo());
        assert!(!e.is_dirty());
    }

    #[test]
    fn insert_multi_line_text_counts_lines() {
        let mut e = at("", 0);
        e.insert_text("a\nb");
        assert_eq!(e.line_count(), 2);
    }

    #[test]
    fn insert_cjk_moves_caret_by_bytes() {
        let mut e = at("", 0);
        e.insert_text("你好");
        assert_eq!(e.len(), 6);
        assert_eq!(e.cursor(), 6);
    }

    #[test]
    fn insert_emoji_moves_caret_by_bytes() {
        let mut e = at("", 0);
        e.insert_text("😀");
        assert_eq!(e.len(), 4);
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn insert_in_middle_of_cjk_text_at_boundary() {
        let mut e = at("你好", 3);
        e.insert_text("吗");
        assert_eq!(e.text(), "你吗好");
        assert_eq!(e.cursor(), 6);
    }

    #[test]
    fn paste_normalises_crlf_and_replaces_selection() {
        let mut e = sel("abc", 0, 1);
        e.paste("X\r\nY");
        assert_eq!(e.text(), "X\nYbc");
        assert_eq!(e.cursor(), 3);
    }

    // --- backspace / delete ---

    #[test]
    fn backspace_ascii() {
        let mut e = at("abc", 3);
        e.backspace();
        assert_eq!(e.text(), "ab");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut e = at("abc", 0);
        e.backspace();
        assert_eq!(e.text(), "abc");
        assert!(!e.can_undo());
    }

    #[test]
    fn backspace_cjk_removes_whole_char() {
        let mut e = at("你好", 6);
        e.backspace();
        assert_eq!(e.text(), "你");
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn backspace_emoji_removes_whole_char() {
        let mut e = at("a😀", 5);
        e.backspace();
        assert_eq!(e.text(), "a");
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn backspace_combining_mark_removes_base_and_mark() {
        let mut e = at("e\u{301}", 3);
        e.backspace();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn backspace_zwj_sequence_removes_whole_sequence() {
        let mut e = at("👨\u{200D}👩", 11);
        e.backspace();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn backspace_variation_selector_sequence() {
        let mut e = at("x❤\u{FE0F}", 7);
        e.backspace();
        assert_eq!(e.text(), "x");
    }

    #[test]
    fn backspace_joins_lines_at_newline() {
        let mut e = at("a\nb", 2);
        e.backspace();
        assert_eq!(e.text(), "ab");
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn backspace_with_selection_deletes_selection() {
        let mut e = sel("abcd", 1, 3);
        e.backspace();
        assert_eq!(e.text(), "ad");
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn delete_forward_ascii() {
        let mut e = at("abc", 1);
        e.delete_forward();
        assert_eq!(e.text(), "ac");
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn delete_forward_at_end_is_noop() {
        let mut e = at("abc", 3);
        e.delete_forward();
        assert_eq!(e.text(), "abc");
        assert!(!e.can_undo());
    }

    #[test]
    fn delete_forward_combining_mark_is_one_unit() {
        let mut e = at("e\u{301}x", 0);
        e.delete_forward();
        assert_eq!(e.text(), "x");
    }

    #[test]
    fn delete_forward_variation_selector_is_one_unit() {
        let mut e = at("❤\u{FE0F}x", 0);
        e.delete_forward();
        assert_eq!(e.text(), "x");
    }

    #[test]
    fn delete_forward_with_selection() {
        let mut e = sel("abcd", 0, 2);
        e.delete_forward();
        assert_eq!(e.text(), "cd");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn delete_forward_cjk() {
        let mut e = at("你好", 0);
        e.delete_forward();
        assert_eq!(e.text(), "好");
    }

    // --- grapheme arrows ---

    #[test]
    fn arrow_right_steps_over_combining_mark() {
        let mut e = at("e\u{301}x", 0);
        e.move_right(false);
        assert_eq!(e.cursor(), 3);
        e.move_right(false);
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn arrow_left_steps_over_emoji() {
        let mut e = at("a😀b", 5);
        e.move_left(false);
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn arrow_left_steps_over_cjk() {
        let mut e = at("你好", 6);
        e.move_left(false);
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn arrow_left_over_zwj_sequence() {
        let mut e = at("👨\u{200D}👩x", 11);
        e.move_left(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn arrow_right_at_end_stays() {
        let mut e = at("ab", 2);
        e.move_right(false);
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn arrow_left_at_start_stays() {
        let mut e = at("ab", 0);
        e.move_left(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn arrow_left_collapses_selection_to_start() {
        let mut e = sel("abcd", 1, 3);
        e.move_left(false);
        assert_eq!(e.cursor(), 1);
        assert!(!e.has_selection());
    }

    #[test]
    fn arrow_right_collapses_selection_to_end() {
        let mut e = sel("abcd", 3, 1);
        e.move_right(false);
        assert_eq!(e.cursor(), 3);
        assert!(!e.has_selection());
    }

    #[test]
    fn shift_right_extends_selection() {
        let mut e = at("abc", 0);
        e.move_right(true);
        e.move_right(true);
        assert_eq!(e.selection(), Selection { anchor: 0, head: 2 });
        assert_eq!(e.selected_text(), "ab");
    }

    #[test]
    fn shift_left_back_over_anchor() {
        let mut e = at("abc", 2);
        e.move_left(true);
        assert_eq!(e.selection(), Selection { anchor: 2, head: 1 });
    }

    // --- selection clamping ---

    #[test]
    fn set_selection_clamps_out_of_range() {
        let mut e = EditorState::new("abc");
        e.set_selection(100, 200);
        assert_eq!(e.selection(), Selection { anchor: 3, head: 3 });
    }

    #[test]
    fn set_selection_clamps_mid_char_down() {
        let mut e = EditorState::new("你好");
        e.set_selection(1, 1);
        assert_eq!(e.cursor(), 0);
        e.set_selection(2, 5);
        assert_eq!(e.selection(), Selection { anchor: 0, head: 3 });
    }

    #[test]
    fn selected_text_of_clamped_selection_is_valid() {
        let mut e = EditorState::new("你好");
        e.set_selection(1, 4);
        assert_eq!(e.selected_text(), "你");
    }

    // --- word movement and deletion ---

    #[test]
    fn word_right_across_latin_words() {
        let mut e = at("hello world", 0);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 5);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 11);
    }

    #[test]
    fn word_left_across_latin_words() {
        let mut e = at("hello world", 11);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 6);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn word_right_skips_whitespace_then_word() {
        let mut e = at("foo   bar", 3);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 9);
    }

    #[test]
    fn word_movement_cjk_run_is_one_word() {
        let mut e = at("你好世界 abc", 0);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 12);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 16);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 13);
    }

    #[test]
    fn word_movement_mixed_scripts() {
        let mut e = at("abc你好def", 0);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 3);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 9);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 12);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 9);
    }

    #[test]
    fn word_movement_kana_scripts_split_by_script() {
        let mut e = at("ひらがなカタカナ", 24);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 12);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn word_movement_punctuation_run_is_one_word() {
        let mut e = at("foo...bar", 3);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 6);
        let mut e = at("foo...bar", 9);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 6);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn word_movement_extend_keeps_anchor() {
        let mut e = at("hello world", 0);
        e.move_word_right(true);
        assert_eq!(e.selected_text(), "hello");
    }

    #[test]
    fn word_movement_combining_mark_stays_with_base() {
        let mut e = at("cafe\u{301} ok", 0);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 6);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 9);
        let mut e = at("cafe\u{301} ok", 9);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 7);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn delete_word_backward_removes_previous_word() {
        let mut e = at("hello world", 11);
        e.delete_word_backward();
        assert_eq!(e.text(), "hello ");
        assert_eq!(e.cursor(), 6);
    }

    #[test]
    fn delete_word_backward_skips_trailing_spaces() {
        let mut e = at("hello   ", 8);
        e.delete_word_backward();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn delete_word_forward_removes_next_word() {
        let mut e = at("hello world", 0);
        e.delete_word_forward();
        assert_eq!(e.text(), " world");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn delete_word_backward_cjk() {
        let mut e = at("你好 世界", 13);
        e.delete_word_backward();
        assert_eq!(e.text(), "你好 ");
    }

    #[test]
    fn delete_word_with_selection_deletes_selection_only() {
        let mut e = sel("hello world", 2, 4);
        e.delete_word_backward();
        assert_eq!(e.text(), "heo world");
    }

    #[test]
    fn delete_to_line_start_removes_prefix_of_line() {
        let mut e = at("abc\ndef", 6);
        e.delete_to_line_start();
        assert_eq!(e.text(), "abc\nf");
        assert_eq!(e.cursor(), 4);
    }

    // --- smart home / end ---

    #[test]
    fn smart_home_first_press_goes_to_first_non_space() {
        let mut e = at("   foo", 6);
        e.move_line_start(false);
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn smart_home_second_press_goes_to_column_zero() {
        let mut e = at("   foo", 6);
        e.move_line_start(false);
        e.move_line_start(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn smart_home_without_indent_stays_at_zero() {
        let mut e = at("foo", 2);
        e.move_line_start(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn smart_home_on_second_line() {
        let mut e = at("a\n  bc", 6);
        e.move_line_start(false);
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn smart_home_extend_selects_to_first_non_space() {
        let mut e = at("a\n  bc", 6);
        e.move_line_start(true);
        assert_eq!(e.selection(), Selection { anchor: 6, head: 4 });
    }

    #[test]
    fn line_end_moves_to_newline_position() {
        let mut e = at("abc\ndef", 0);
        e.move_line_end(false);
        assert_eq!(e.cursor(), 3);
        let mut e = at("abc\ndef", 4);
        e.move_line_end(false);
        assert_eq!(e.cursor(), 7);
    }

    #[test]
    fn line_end_extend() {
        let mut e = at("abc\ndef", 1);
        e.move_line_end(true);
        assert_eq!(e.selected_text(), "bc");
    }

    #[test]
    fn doc_start_and_end() {
        let mut e = at("ab\ncd", 4);
        e.move_doc_start(false);
        assert_eq!(e.cursor(), 0);
        e.move_doc_end(false);
        assert_eq!(e.cursor(), 5);
        e.move_doc_start(true);
        assert_eq!(e.selection(), Selection { anchor: 5, head: 0 });
    }

    // --- vertical movement and geometry ---

    #[test]
    fn up_down_keeps_goal_column_across_short_line() {
        let mut e = at("abcdef\nab\nabcdef", 5);
        e.move_down(false);
        assert_eq!(e.cursor(), 9);
        e.move_down(false);
        assert_eq!(e.cursor(), 15);
        e.move_up(false);
        assert_eq!(e.cursor(), 9);
        e.move_up(false);
        assert_eq!(e.cursor(), 5);
    }

    #[test]
    fn up_down_on_empty_line() {
        let mut e = at("abc\n\nxyz", 2);
        e.move_down(false);
        assert_eq!(e.cursor(), 4);
        e.move_down(false);
        assert_eq!(e.cursor(), 7);
    }

    #[test]
    fn up_down_cjk_counts_chars_not_width() {
        let mut e = at("你好吗\nabcdef", 6);
        assert_eq!(e.offset_to_row_col(6), (0, 2));
        e.move_down(false);
        assert_eq!(e.cursor(), 12);
        e.move_up(false);
        assert_eq!(e.cursor(), 6);
    }

    #[test]
    fn up_on_first_row_goes_to_start() {
        let mut e = at("abc", 2);
        e.move_up(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn down_on_last_row_goes_to_end() {
        let mut e = at("abc\nde", 5);
        e.move_down(false);
        assert_eq!(e.cursor(), 6);
    }

    #[test]
    fn horizontal_move_resets_goal_column() {
        let mut e = at("abcdef\nab", 5);
        e.move_down(false);
        assert_eq!(e.cursor(), 9);
        e.move_left(false);
        assert_eq!(e.cursor(), 8);
        e.move_up(false);
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn up_down_extend_keeps_anchor() {
        let mut e = at("abc\ndef", 1);
        e.move_down(true);
        assert_eq!(e.selection(), Selection { anchor: 1, head: 5 });
    }

    #[test]
    fn offset_to_row_col_mid_line() {
        let e = EditorState::new("ab\ncd");
        assert_eq!(e.offset_to_row_col(4), (1, 1));
        assert_eq!(e.offset_to_row_col(0), (0, 0));
        assert_eq!(e.offset_to_row_col(3), (1, 0));
    }

    #[test]
    fn row_col_to_offset_clamps_column() {
        let e = EditorState::new("ab\ncd");
        assert_eq!(e.row_col_to_offset(0, 99), 2);
    }

    #[test]
    fn row_col_to_offset_clamps_row() {
        let e = EditorState::new("ab\ncd");
        assert_eq!(e.row_col_to_offset(9, 0), 3);
    }

    #[test]
    fn line_count_cases() {
        assert_eq!(EditorState::new("").line_count(), 1);
        assert_eq!(EditorState::new("a\n").line_count(), 2);
        assert_eq!(EditorState::new("a\nb\nc").line_count(), 3);
    }

    #[test]
    fn line_range_with_empty_middle_line() {
        let e = EditorState::new("ab\n\ncd");
        assert_eq!(e.line_range(0), 0..2);
        assert_eq!(e.line_range(1), 3..3);
        assert_eq!(e.line_range(2), 4..6);
    }

    #[test]
    fn line_range_trailing_newline_gives_empty_last_line() {
        let e = EditorState::new("a\n");
        assert_eq!(e.line_range(1), 2..2);
    }

    #[test]
    fn line_range_past_end_is_empty_at_eof() {
        let e = EditorState::new("ab");
        assert_eq!(e.line_range(5), 2..2);
    }

    #[test]
    fn offset_row_col_round_trip_with_cjk() {
        let e = EditorState::new("你好\n世界!");
        let off = e.row_col_to_offset(1, 1);
        assert_eq!(&e.text()[off..], "界!");
        assert_eq!(e.offset_to_row_col(off), (1, 1));
    }

    // --- line operations ---

    #[test]
    fn duplicate_middle_line_moves_caret_to_copy() {
        let mut e = at("one\ntwo\nthree", 5);
        e.duplicate_line();
        assert_eq!(e.text(), "one\ntwo\ntwo\nthree");
        assert_eq!(e.cursor(), 9);
    }

    #[test]
    fn duplicate_last_line_without_trailing_newline() {
        let mut e = at("a\nb", 2);
        e.duplicate_line();
        assert_eq!(e.text(), "a\nb\nb");
    }

    #[test]
    fn duplicate_single_line_document() {
        let mut e = at("x", 0);
        e.duplicate_line();
        assert_eq!(e.text(), "x\nx");
    }

    #[test]
    fn duplicate_multi_line_selection() {
        let mut e = sel("1\n2\n3", 0, 3);
        e.duplicate_line();
        assert_eq!(e.text(), "1\n2\n1\n2\n3");
        assert_eq!(e.selected_text(), "1\n2");
    }

    #[test]
    fn move_line_up_swaps_with_previous() {
        let mut e = at("one\ntwo\nthree", 5);
        e.move_line_up();
        assert_eq!(e.text(), "two\none\nthree");
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn move_line_up_on_first_line_is_noop() {
        let mut e = at("one\ntwo", 1);
        e.move_line_up();
        assert_eq!(e.text(), "one\ntwo");
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn move_line_up_last_line_without_trailing_newline() {
        let mut e = at("a\nb", 2);
        e.move_line_up();
        assert_eq!(e.text(), "b\na");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn move_line_up_multi_line_selection() {
        let mut e = sel("1\n2\n3\n4", 2, 5);
        e.move_line_up();
        assert_eq!(e.text(), "2\n3\n1\n4");
        assert_eq!(e.selected_text(), "2\n3");
    }

    #[test]
    fn move_line_down_swaps_with_next() {
        let mut e = at("one\ntwo\nthree", 1);
        e.move_line_down();
        assert_eq!(e.text(), "two\none\nthree");
        assert_eq!(e.cursor(), 5);
    }

    #[test]
    fn move_line_down_last_line_is_noop() {
        let mut e = at("a\nb", 3);
        e.move_line_down();
        assert_eq!(e.text(), "a\nb");
    }

    #[test]
    fn move_line_down_into_last_line_without_trailing_newline() {
        let mut e = at("a\nb", 0);
        e.move_line_down();
        assert_eq!(e.text(), "b\na");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn move_line_down_multi_line_selection() {
        let mut e = sel("1\n2\n3\n4", 0, 3);
        e.move_line_down();
        assert_eq!(e.text(), "3\n1\n2\n4");
        assert_eq!(e.selected_text(), "1\n2");
    }

    #[test]
    fn move_line_down_over_empty_line() {
        let mut e = at("a\n\nb", 0);
        e.move_line_down();
        assert_eq!(e.text(), "\na\nb");
    }

    #[test]
    fn delete_line_middle() {
        let mut e = at("one\ntwo\nthree", 5);
        e.delete_line();
        assert_eq!(e.text(), "one\nthree");
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn delete_last_line_without_trailing_newline() {
        let mut e = at("one\ntwo", 5);
        e.delete_line();
        assert_eq!(e.text(), "one");
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn delete_first_line() {
        let mut e = at("a\nb", 0);
        e.delete_line();
        assert_eq!(e.text(), "b");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn delete_only_line() {
        let mut e = at("abc", 1);
        e.delete_line();
        assert_eq!(e.text(), "");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn delete_line_multi_line_selection() {
        let mut e = sel("1\n2\n3", 0, 3);
        e.delete_line();
        assert_eq!(e.text(), "3");
    }

    // --- clipboard ---

    #[test]
    fn cut_returns_selection_and_removes_it() {
        let mut e = sel("hello", 0, 2);
        assert_eq!(e.cut(), "he");
        assert_eq!(e.text(), "llo");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn cut_without_selection_removes_nothing() {
        let mut e = at("abc", 1);
        assert_eq!(e.cut(), "");
        assert_eq!(e.text(), "abc");
    }

    #[test]
    fn copy_returns_selection_without_editing() {
        let e = sel("hello", 1, 3);
        assert_eq!(e.copy(), "el");
        assert_eq!(e.text(), "hello");
    }

    #[test]
    fn copy_without_selection_is_empty() {
        let e = at("hello", 2);
        assert_eq!(e.copy(), "");
    }

    #[test]
    fn select_all_selects_everything() {
        let mut e = at("ab\ncd", 2);
        e.select_all();
        assert_eq!(e.selected_text(), "ab\ncd");
        assert_eq!(e.selection(), Selection { anchor: 0, head: 5 });
    }

    #[test]
    fn select_word_at_middle_of_word() {
        let mut e = at("hello world", 0);
        e.select_word_at(7);
        assert_eq!(e.selected_text(), "world");
        assert_eq!(e.selection().range(), 6..11);
    }

    #[test]
    fn select_word_at_cjk_run() {
        let mut e = at("你好世界 abc", 0);
        e.select_word_at(3);
        assert_eq!(e.selected_text(), "你好世界");
    }

    #[test]
    fn select_word_at_whitespace_selects_run() {
        let mut e = at("a  b", 0);
        e.select_word_at(2);
        assert_eq!(e.selected_text(), "  ");
    }

    #[test]
    fn select_word_at_end_of_text_uses_previous_char() {
        let mut e = at("abc", 0);
        e.select_word_at(3);
        assert_eq!(e.selected_text(), "abc");
    }

    #[test]
    fn select_word_at_empty_document_is_collapsed() {
        let mut e = at("", 0);
        e.select_word_at(0);
        assert!(!e.has_selection());
    }

    #[test]
    fn select_line_at_includes_newline() {
        let mut e = at("one\ntwo\nthree", 0);
        e.select_line_at(5);
        assert_eq!(e.selected_text(), "two\n");
    }

    #[test]
    fn select_line_at_last_line() {
        let mut e = at("one\ntwo", 0);
        e.select_line_at(6);
        assert_eq!(e.selected_text(), "two");
    }

    #[test]
    fn select_line_then_type_replaces_line_text() {
        let mut e = at("one\ntwo\nthree", 0);
        e.select_line_at(5);
        e.insert_text("X");
        assert_eq!(e.text(), "one\nXthree");
    }

    // --- undo / redo ---

    #[test]
    fn consecutive_typing_merges_into_one_undo_step() {
        let mut e = at("", 0);
        type_chars(&mut e, "abc");
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert!(!e.can_undo());
    }

    #[test]
    fn typing_breaks_group_after_space_before_word() {
        let mut e = at("", 0);
        type_chars(&mut e, "hi there");
        assert!(e.undo());
        assert_eq!(e.text(), "hi ");
        assert!(e.undo());
        assert_eq!(e.text(), "");
    }

    #[test]
    fn typing_keeps_space_in_group_before_next_word() {
        let mut e = at("", 0);
        type_chars(&mut e, "a b");
        assert!(e.undo());
        assert_eq!(e.text(), "a ");
        assert!(e.undo());
        assert_eq!(e.text(), "");
    }

    #[test]
    fn undo_restores_caret_before_edit() {
        let mut e = at("abc", 1);
        e.insert_text("X");
        assert_eq!(e.text(), "aXbc");
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
        assert_eq!(e.selection(), Selection::collapsed(1));
    }

    #[test]
    fn redo_restores_caret_after_edit() {
        let mut e = at("abc", 1);
        e.insert_text("X");
        e.undo();
        assert!(e.redo());
        assert_eq!(e.text(), "aXbc");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn undo_restores_selection_that_was_replaced() {
        let mut e = sel("hello", 1, 4);
        e.insert_text("A");
        assert_eq!(e.text(), "hAo");
        e.undo();
        assert_eq!(e.text(), "hello");
        assert_eq!(e.selection(), Selection { anchor: 1, head: 4 });
    }

    #[test]
    fn backspaces_merge_into_one_step() {
        let mut e = at("abc", 3);
        e.backspace();
        e.backspace();
        assert_eq!(e.text(), "a");
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
        assert!(!e.can_undo());
    }

    #[test]
    fn forward_deletes_merge_into_one_step() {
        let mut e = at("abc", 0);
        e.delete_forward();
        e.delete_forward();
        assert_eq!(e.text(), "c");
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
        assert!(!e.can_undo());
    }

    #[test]
    fn typing_after_backspace_is_separate_step() {
        let mut e = at("ab", 2);
        e.backspace();
        e.insert_text("x");
        assert_eq!(e.text(), "ax");
        e.undo();
        assert_eq!(e.text(), "a");
        e.undo();
        assert_eq!(e.text(), "ab");
    }

    #[test]
    fn caret_movement_breaks_typing_group() {
        let mut e = at("", 0);
        e.insert_text("a");
        e.move_left(false);
        e.insert_text("b");
        assert_eq!(e.text(), "ba");
        e.undo();
        assert_eq!(e.text(), "a");
        e.undo();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn newline_is_its_own_step() {
        let mut e = at("", 0);
        e.insert_text("a");
        e.insert_text("\n");
        e.insert_text("b");
        assert_eq!(e.text(), "a\nb");
        assert!(e.undo());
        assert_eq!(e.text(), "a\n");
        assert!(e.undo());
        assert_eq!(e.text(), "a");
        assert!(e.undo());
        assert_eq!(e.text(), "");
    }

    #[test]
    fn paste_is_its_own_step() {
        let mut e = at("", 0);
        e.insert_text("a");
        e.paste("XY");
        assert_eq!(e.text(), "aXY");
        assert!(e.undo());
        assert_eq!(e.text(), "a");
        assert!(e.undo());
        assert_eq!(e.text(), "");
    }

    #[test]
    fn cut_is_its_own_step() {
        let mut e = sel("abc", 0, 1);
        e.cut();
        assert_eq!(e.text(), "bc");
        e.undo();
        assert_eq!(e.text(), "abc");
        assert_eq!(e.selection(), Selection { anchor: 0, head: 1 });
    }

    #[test]
    fn replace_range_is_one_step_and_sets_selection() {
        let mut e = at("hello", 0);
        e.replace_range(0, 5, "bye", Selection::collapsed(3));
        assert_eq!(e.text(), "bye");
        assert_eq!(e.cursor(), 3);
        assert!(e.undo());
        assert_eq!(e.text(), "hello");
        assert!(!e.can_undo());
    }

    #[test]
    fn replace_range_undo_restores_previous_selection() {
        let mut e = sel("hello", 2, 4);
        e.replace_range(2, 4, "XY", Selection::collapsed(4));
        e.undo();
        assert_eq!(e.selection(), Selection { anchor: 2, head: 4 });
    }

    #[test]
    fn replace_range_clamps_out_of_range_offsets() {
        let mut e = at("abc", 0);
        e.replace_range(10, 20, "z", Selection::collapsed(99));
        assert_eq!(e.text(), "abcz");
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn replace_range_accepts_reversed_bounds() {
        let mut e = at("abcd", 0);
        e.replace_range(3, 1, "X", Selection::collapsed(2));
        assert_eq!(e.text(), "aXd");
    }

    #[test]
    fn replace_range_clamps_mid_char_end() {
        let mut e = at("你好", 0);
        e.replace_range(0, 4, "x", Selection::collapsed(1));
        assert_eq!(e.text(), "x好");
    }

    #[test]
    fn replace_range_noop_still_sets_selection() {
        let mut e = at("abc", 0);
        e.replace_range(1, 1, "", Selection { anchor: 2, head: 2 });
        assert_eq!(e.text(), "abc");
        assert_eq!(e.cursor(), 2);
        assert!(!e.can_undo());
    }

    #[test]
    fn redo_is_cleared_by_new_edit() {
        let mut e = at("", 0);
        e.insert_text("a");
        e.undo();
        assert!(e.can_redo());
        e.insert_text("b");
        assert!(!e.can_redo());
        assert!(!e.redo());
    }

    #[test]
    fn undo_then_redo_round_trip() {
        let mut e = at("", 0);
        e.insert_text("x");
        e.insert_text("\n");
        assert!(e.undo());
        assert!(e.redo());
        assert_eq!(e.text(), "x\n");
        assert!(!e.redo());
    }

    #[test]
    fn undo_and_redo_report_availability() {
        let mut e = at("", 0);
        assert!(!e.undo());
        assert!(!e.redo());
        assert!(!e.can_undo());
        assert!(!e.can_redo());
        e.insert_text("a");
        assert!(e.can_undo());
    }

    #[test]
    fn history_is_capped_at_1000_transactions() {
        let mut e = at("", 0);
        for _ in 0..1100 {
            e.insert_text("\n");
        }
        let mut undone = 0;
        while e.undo() {
            undone += 1;
        }
        assert_eq!(undone, 1000);
        assert_eq!(e.text().matches('\n').count(), 100);
    }

    #[test]
    fn backspace_then_typing_at_same_place_not_merged() {
        let mut e = at("ab", 2);
        e.backspace();
        e.insert_text("b");
        e.undo();
        assert_eq!(e.text(), "a");
    }

    #[test]
    fn line_op_is_separate_step_from_typing() {
        let mut e = at("x", 1);
        e.insert_text("y");
        e.duplicate_line();
        assert_eq!(e.text(), "xy\nxy");
        e.undo();
        assert_eq!(e.text(), "xy");
    }

    // --- dirty tracking ---

    #[test]
    fn fresh_document_is_clean() {
        let e = EditorState::new("abc");
        assert!(!e.is_dirty());
    }

    #[test]
    fn edit_makes_dirty_and_mark_saved_cleans() {
        let mut e = at("abc", 0);
        e.insert_text("x");
        assert!(e.is_dirty());
        e.mark_saved();
        assert!(!e.is_dirty());
    }

    #[test]
    fn undo_back_to_saved_point_is_clean() {
        let mut e = at("x", 1);
        e.insert_text("a");
        e.mark_saved();
        e.insert_text("\n");
        assert!(e.is_dirty());
        e.undo();
        assert!(!e.is_dirty());
        e.redo();
        assert!(e.is_dirty());
    }

    #[test]
    fn undo_past_saved_point_stays_dirty() {
        let mut e = at("x", 1);
        e.insert_text("a");
        e.mark_saved();
        e.undo();
        assert!(e.is_dirty());
    }

    #[test]
    fn new_branch_after_undo_is_dirty_until_matching_state() {
        let mut e = at("x", 1);
        e.insert_text("a");
        e.mark_saved();
        e.undo();
        assert!(e.is_dirty());
        e.insert_text("b");
        assert!(e.is_dirty());
        e.undo();
        assert!(e.is_dirty());
    }

    #[test]
    fn saved_state_survives_redo_back_to_it() {
        let mut e = at("", 0);
        e.insert_text("a");
        e.insert_text("\n");
        e.undo();
        e.mark_saved();
        assert!(!e.is_dirty());
        e.redo();
        assert!(e.is_dirty());
        e.undo();
        assert!(!e.is_dirty());
    }

    #[test]
    fn set_text_clears_history_and_is_clean() {
        let mut e = at("", 0);
        e.insert_text("hello");
        e.set_text("new");
        assert!(!e.can_undo());
        assert!(!e.can_redo());
        assert!(!e.is_dirty());
        assert_eq!(e.text(), "new");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn set_text_normalises_crlf() {
        let mut e = EditorState::new("a");
        e.set_text("x\r\ny");
        assert_eq!(e.text(), "x\ny");
    }

    #[test]
    fn set_text_then_edit_is_dirty() {
        let mut e = EditorState::new("a");
        e.set_text("b");
        e.insert_text("c");
        assert!(e.is_dirty());
        e.undo();
        assert!(!e.is_dirty());
    }

    // --- performance smoke and invariants ---

    #[test]
    fn large_document_edits_round_trip() {
        let line = "line of some text here\n";
        let original = line.repeat(2000);
        let mut e = EditorState::new(&original);
        let mid = (original.len() / 2) / line.len() * line.len();
        e.set_selection(mid, mid);
        for _ in 0..40 {
            e.insert_text("a");
        }
        for _ in 0..40 {
            e.backspace();
        }
        for _ in 0..20 {
            e.move_down(false);
        }
        assert_eq!(e.text(), original);
        assert_eq!(e.line_count(), 2001);
    }

    #[test]
    fn invariants_hold_after_mixed_operations() {
        let mut e = EditorState::new("héllo 世界\n😀 x\n");
        e.select_all();
        e.move_line_down();
        e.duplicate_line();
        e.move_word_right(false);
        e.delete_word_backward();
        e.move_up(true);
        e.insert_text("é");
        let t = e.text();
        assert!(t.is_char_boundary(e.cursor()));
        assert!(t.is_char_boundary(e.selection().anchor));
        assert!(e.cursor() <= t.len());
    }

    #[test]
    fn selection_offsets_always_on_boundaries_after_undo() {
        let mut e = EditorState::new("你好");
        e.set_selection(3, 6);
        e.delete_forward();
        e.undo();
        assert!(e.text().is_char_boundary(e.cursor()));
        assert_eq!(e.selected_text(), "好");
    }
}
