//! Back and forward history of the notes the user visited: the mouse buttons 4 and 5, Alt+Left and
//! Alt+Right, and the 后退 and 前进 commands.
//!
//! `NavHistory` is the pure part (unit tested). The active note is watched on every frame
//! (`observe`), so every way of opening a note (tabs, links, search, graph, palette) is recorded
//! without touching each caller. Back and forward move `current` themselves, so they are not
//! recorded as new visits.

use crate::RustdianApp;

/// Entries kept in each of the two stacks. The oldest entry is dropped first.
pub const MAX_ENTRIES: usize = 100;

/// The note being shown, the notes before it (`back`, the last element is the nearest) and the notes
/// after it (`forward`, the last element is the nearest).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct NavHistory {
    current: Option<String>,
    back: Vec<String>,
    forward: Vec<String>,
}

impl NavHistory {
    /// The note path the history considers current.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Called with the active note's path on every frame. A new path is a visit: the note left is
    /// pushed on the back stack (a consecutive duplicate is not), and the forward stack is cleared.
    pub fn observe(&mut self, active: Option<&str>) {
        let Some(path) = active else {
            self.current = None;
            return;
        };
        if self.current.as_deref() == Some(path) {
            return;
        }
        if let Some(previous) = self.current.take() {
            push_bounded(&mut self.back, previous);
        }
        self.forward.clear();
        self.current = Some(path.to_string());
    }

    /// Steps back to the nearest earlier note that `keep` accepts. Entries that `keep` rejects are
    /// dropped. The note left goes on the forward stack. Returns the note to open.
    pub fn back_where(&mut self, keep: impl Fn(&str) -> bool) -> Option<String> {
        self.step(false, keep)
    }

    /// Steps forward to the nearest later note that `keep` accepts (see `back_where`).
    pub fn forward_where(&mut self, keep: impl Fn(&str) -> bool) -> Option<String> {
        self.step(true, keep)
    }

    fn step(&mut self, forward: bool, keep: impl Fn(&str) -> bool) -> Option<String> {
        loop {
            let target = if forward {
                self.forward.pop()
            } else {
                self.back.pop()
            }?;
            if !keep(&target) {
                continue;
            }
            if let Some(left) = self.current.take() {
                let other = if forward {
                    &mut self.back
                } else {
                    &mut self.forward
                };
                push_bounded(other, left);
            }
            self.current = Some(target.clone());
            return Some(target);
        }
    }

    /// Renames the paths in every stack (a note or folder was renamed or moved). `map` returns the
    /// new path of an old one.
    pub fn map_paths(&mut self, map: impl Fn(&str) -> String) {
        if let Some(current) = self.current.as_mut() {
            *current = map(current.as_str());
        }
        for path in self.back.iter_mut().chain(self.forward.iter_mut()) {
            *path = map(path.as_str());
        }
    }

    /// Drops the paths that `keep` rejects (notes that were deleted), and collapses the repeats
    /// this leaves behind. A dropped current note is forgotten: the next visit then pushes nothing.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.back.retain(|p| keep(p));
        self.forward.retain(|p| keep(p));
        if self.current.as_deref().is_some_and(|c| !keep(c)) {
            self.current = None;
        }
        self.back.dedup();
        self.forward.dedup();
    }
}

/// Pushes `path` unless it repeats the last entry, and drops the oldest entries past the bound.
fn push_bounded(stack: &mut Vec<String>, path: String) {
    if stack.last() == Some(&path) {
        return;
    }
    stack.push(path);
    if stack.len() > MAX_ENTRIES {
        let extra = stack.len() - MAX_ENTRIES;
        stack.drain(..extra);
    }
}

/// Goes back (`forward` false) or forward to the nearest visit and opens that note. Does nothing
/// when there is no such visit.
pub fn go(app: &mut RustdianApp, forward: bool) {
    let notes = &app.vault.notes;
    let keep = |path: &str| notes.iter().any(|n| n.path == path);
    let target = if forward {
        app.nav.forward_where(keep)
    } else {
        app.nav.back_where(keep)
    };
    if let Some(path) = target
        && let Some(idx) = app.note_index(&path)
    {
        app.open_note_by_index(idx);
    }
}

/// Records the active note of this frame (see `NavHistory::observe`).
pub fn observe(app: &mut RustdianApp) {
    let active = app.active_path().map(str::to_string);
    app.nav.observe(active.as_deref());
}

/// Renames the paths in the history of a file operation (see `files_ui::actions::remap_state`).
pub fn remap(app: &mut RustdianApp, map: impl Fn(&str) -> String) {
    app.nav.map_paths(map);
}

/// Drops the history entries of the note `path` and of the notes under the folder `path`.
pub fn forget(app: &mut RustdianApp, path: &str) {
    let prefix = format!("{path}/");
    app.nav
        .retain(|p| p != path && !p.starts_with(prefix.as_str()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &str) -> bool {
        true
    }

    fn visit(nav: &mut NavHistory, path: &str) {
        nav.observe(Some(path));
    }

    #[test]
    fn first_visit_records_nothing() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        assert_eq!(nav.current(), Some("a.md"));
        assert!(!nav.can_back());
        assert!(!nav.can_forward());
    }

    #[test]
    fn visits_fill_the_back_stack_in_order() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "b.md");
        visit(&mut nav, "c.md");
        assert_eq!(nav.back_where(all).as_deref(), Some("b.md"));
        assert_eq!(nav.back_where(all).as_deref(), Some("a.md"));
        assert_eq!(nav.back_where(all), None);
        assert_eq!(nav.current(), Some("a.md"));
    }

    #[test]
    fn back_then_forward_round_trips() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "b.md");
        nav.back_where(all);
        assert!(nav.can_forward());
        assert_eq!(nav.forward_where(all).as_deref(), Some("b.md"));
        assert_eq!(nav.current(), Some("b.md"));
        assert!(!nav.can_forward());
    }

    #[test]
    fn a_new_visit_clears_the_forward_stack() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "b.md");
        nav.back_where(all);
        visit(&mut nav, "c.md");
        assert!(!nav.can_forward());
        assert_eq!(nav.back_where(all).as_deref(), Some("a.md"));
    }

    #[test]
    fn going_back_is_not_recorded_as_a_visit() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "b.md");
        let target = nav.back_where(all).unwrap_or_default();
        // The frame after the back command shows the target: no new entry.
        nav.observe(Some(target.as_str()));
        assert_eq!(nav.current(), Some("a.md"));
        assert!(!nav.can_back());
        assert!(nav.can_forward());
    }

    #[test]
    fn consecutive_duplicates_are_not_pushed() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "b.md");
        visit(&mut nav, "a.md");
        visit(&mut nav, "a.md");
        visit(&mut nav, "b.md");
        // a, b, a, b: the repeated visit of a is not a second entry.
        assert_eq!(nav.back.len(), 3);
    }

    #[test]
    fn dedup_on_retain_collapses_repeats() {
        let mut nav = NavHistory {
            current: Some("c.md".into()),
            back: vec!["x.md".into(), "x.md".into()],
            forward: Vec::new(),
        };
        nav.retain(|_| true);
        assert_eq!(nav.back, vec!["x.md".to_string()]);
    }

    #[test]
    fn observe_none_forgets_current_without_a_push() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        nav.observe(None);
        assert_eq!(nav.current(), None);
        assert!(!nav.can_back());
        visit(&mut nav, "b.md");
        assert!(!nav.can_back());
    }

    #[test]
    fn back_skips_notes_that_no_longer_exist() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "gone.md");
        visit(&mut nav, "c.md");
        let keep = |p: &str| p != "gone.md";
        assert_eq!(nav.back_where(keep).as_deref(), Some("a.md"));
    }

    #[test]
    fn the_back_stack_is_bounded() {
        let mut nav = NavHistory::default();
        for i in 0..(MAX_ENTRIES + 50) {
            visit(&mut nav, &format!("n{i}.md"));
        }
        assert_eq!(nav.back.len(), MAX_ENTRIES);
        // The nearest entry is the note shown just before the current one.
        assert_eq!(
            nav.back.last().map(String::as_str),
            Some(format!("n{}.md", MAX_ENTRIES + 48).as_str())
        );
        let mut count = 0;
        while nav.back_where(all).is_some() {
            count += 1;
        }
        assert_eq!(count, MAX_ENTRIES);
    }

    #[test]
    fn the_forward_stack_is_bounded() {
        let mut nav = NavHistory::default();
        for i in 0..(MAX_ENTRIES + 10) {
            visit(&mut nav, &format!("n{i}.md"));
        }
        while nav.back_where(all).is_some() {}
        assert_eq!(nav.forward.len(), MAX_ENTRIES);
    }

    #[test]
    fn map_paths_follows_a_rename() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "old.md");
        visit(&mut nav, "b.md");
        nav.map_paths(|p| {
            if p == "old.md" {
                "new.md".to_string()
            } else {
                p.to_string()
            }
        });
        assert_eq!(nav.back_where(all).as_deref(), Some("new.md"));
    }

    #[test]
    fn retain_drops_deleted_notes_and_collapses_repeats() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "x.md");
        visit(&mut nav, "a.md");
        visit(&mut nav, "y.md");
        nav.retain(|p| p != "x.md");
        assert_eq!(nav.back, vec!["a.md".to_string()]);
        assert_eq!(nav.current(), Some("y.md"));
    }

    #[test]
    fn retain_forgets_a_deleted_current_note() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "b.md");
        nav.retain(|p| p != "b.md");
        assert_eq!(nav.current(), None);
        assert!(nav.can_back());
        visit(&mut nav, "c.md");
        assert_eq!(nav.back, vec!["a.md".to_string()]);
    }

    #[test]
    fn retain_drops_a_whole_folder_by_prefix() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "dir/a.md");
        visit(&mut nav, "keep.md");
        visit(&mut nav, "dir/b.md");
        nav.retain(|p| !p.starts_with("dir/"));
        assert_eq!(nav.back, vec!["keep.md".to_string()]);
        assert_eq!(nav.current(), None);
    }

    #[test]
    fn forward_skips_missing_entries_too() {
        let mut nav = NavHistory::default();
        visit(&mut nav, "a.md");
        visit(&mut nav, "b.md");
        visit(&mut nav, "c.md");
        nav.back_where(all);
        nav.back_where(all);
        assert_eq!(nav.current(), Some("a.md"));
        let keep = |p: &str| p != "b.md";
        assert_eq!(nav.forward_where(keep).as_deref(), Some("c.md"));
    }
}
