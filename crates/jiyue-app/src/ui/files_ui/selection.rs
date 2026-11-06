//! Multi-selection of the file tree: plain click, Ctrl+click (toggle), Shift+click (range from the
//! anchor in the visible order) and Ctrl+A. Pure logic, unit tested.

/// How a tree row was clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickMode {
    /// A plain click: the row alone is selected.
    Plain,
    /// Ctrl+click: the row is added to or removed from the selection.
    Toggle,
    /// Shift+click: the rows from the anchor to the row are selected.
    Range,
}

/// The selected rows (note paths and folder keys) and the anchor of Shift+click.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    items: Vec<String>,
    anchor: Option<String>,
}

impl Selection {
    /// The selected rows, in the order they were selected.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.items.iter().any(|p| p == path)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.anchor = None;
    }

    /// Applies a click on `path`. `visible` is the list of rows in the order they are shown.
    pub fn click(&mut self, visible: &[String], path: &str, mode: ClickMode) {
        match mode {
            ClickMode::Plain => {
                self.items = vec![path.to_string()];
                self.anchor = Some(path.to_string());
            }
            ClickMode::Toggle => {
                if let Some(pos) = self.items.iter().position(|p| p == path) {
                    self.items.remove(pos);
                } else {
                    self.items.push(path.to_string());
                }
                self.anchor = Some(path.to_string());
            }
            ClickMode::Range => {
                let target = visible.iter().position(|p| p == path);
                let anchor = self
                    .anchor
                    .as_ref()
                    .and_then(|a| visible.iter().position(|p| p == a));
                match (anchor, target) {
                    (Some(a), Some(t)) => {
                        let (lo, hi) = if a <= t { (a, t) } else { (t, a) };
                        self.items = visible[lo..=hi].to_vec();
                    }
                    // No anchor (or the row is not visible): a plain click.
                    _ => {
                        self.items = vec![path.to_string()];
                        self.anchor = Some(path.to_string());
                    }
                }
            }
        }
    }

    /// Ctrl+A: every visible row is selected.
    pub fn select_all(&mut self, visible: &[String]) {
        self.items = visible.to_vec();
        if self.anchor.is_none() {
            self.anchor = visible.first().cloned();
        }
    }
}

/// The rows an action acts on: the selection without the rows whose parent folder is selected too
/// (a folder and a note inside it must not both be moved). `is_dir` tells the folders apart.
/// The order of the selection is kept.
pub fn effective_roots(items: &[String], is_dir: impl Fn(&str) -> bool) -> Vec<String> {
    let folders: Vec<&String> = items.iter().filter(|p| is_dir(p)).collect();
    items
        .iter()
        .filter(|p| {
            !folders
                .iter()
                .any(|f| f.as_str() != p.as_str() && p.starts_with(&format!("{}/", f)))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rows() -> Vec<String> {
        v(&["a", "a/x.md", "a/y.md", "b.md", "c", "c/z.md"])
    }

    #[test]
    fn plain_click_selects_one_row() {
        let mut s = Selection::default();
        s.click(&rows(), "a", ClickMode::Toggle);
        s.click(&rows(), "b.md", ClickMode::Plain);
        assert_eq!(s.items(), v(&["b.md"]).as_slice());
    }

    #[test]
    fn ctrl_click_toggles() {
        let mut s = Selection::default();
        s.click(&rows(), "a/x.md", ClickMode::Plain);
        s.click(&rows(), "b.md", ClickMode::Toggle);
        assert!(s.contains("a/x.md") && s.contains("b.md"));
        s.click(&rows(), "a/x.md", ClickMode::Toggle);
        assert_eq!(s.items(), v(&["b.md"]).as_slice());
    }

    #[test]
    fn shift_click_selects_the_visible_range_from_the_anchor() {
        let mut s = Selection::default();
        s.click(&rows(), "a/x.md", ClickMode::Plain);
        s.click(&rows(), "c", ClickMode::Range);
        assert_eq!(s.items(), v(&["a/x.md", "a/y.md", "b.md", "c"]).as_slice());
        // Upwards from the same anchor.
        s.click(&rows(), "a", ClickMode::Range);
        assert_eq!(s.items(), v(&["a", "a/x.md"]).as_slice());
    }

    #[test]
    fn shift_click_without_anchor_is_a_plain_click() {
        let mut s = Selection::default();
        s.click(&rows(), "c/z.md", ClickMode::Range);
        assert_eq!(s.items(), v(&["c/z.md"]).as_slice());
    }

    #[test]
    fn select_all_takes_every_visible_row() {
        let mut s = Selection::default();
        s.select_all(&rows());
        assert_eq!(s.len(), 6);
    }

    #[test]
    fn clear_empties_selection_and_anchor() {
        let mut s = Selection::default();
        s.click(&rows(), "b.md", ClickMode::Plain);
        s.clear();
        assert!(s.is_empty());
        s.click(&rows(), "c", ClickMode::Range);
        assert_eq!(s.items(), v(&["c"]).as_slice());
    }

    #[test]
    fn roots_drop_items_inside_a_selected_folder() {
        let items = v(&["a", "a/x.md", "b.md", "a/sub/y.md"]);
        let roots = effective_roots(&items, |p| p == "a");
        assert_eq!(roots, v(&["a", "b.md"]));
    }

    #[test]
    fn roots_keep_a_note_when_only_a_sibling_folder_is_selected() {
        let items = v(&["ab/x.md", "a"]);
        let roots = effective_roots(&items, |p| p == "a");
        assert_eq!(roots, v(&["ab/x.md", "a"]));
    }

    #[test]
    fn roots_keep_order() {
        let items = v(&["c.md", "a.md", "b"]);
        assert_eq!(
            effective_roots(&items, |p| p == "b"),
            v(&["c.md", "a.md", "b"])
        );
    }
}
