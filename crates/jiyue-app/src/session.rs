//! Pure state logic for the note editor session: view modes, debounce bookkeeping, the save
//! status shown in the status bar, and the path-keyed editor map. No gpui types live here, so
//! everything below is unit-tested directly.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Delay after the last edit before a note is written to disk.
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(1500);
/// Delay before the derived views (graph, backlinks) are rebuilt after an edit.
pub const GRAPH_DEBOUNCE: Duration = Duration::from_millis(300);
/// How long the status bar shows `保存中…` after a write.
pub const SAVING_FLASH: Duration = Duration::from_millis(400);
/// How long the status bar shows `保存失败` after a failed write.
pub const SAVE_ERROR_SHOW: Duration = Duration::from_secs(3);

/// How the active note is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// The editable text editor (focused).
    Source,
    /// The rendered preview; a click on the text switches to `Source`.
    Live,
    /// The rendered preview only; clicks do nothing.
    Read,
}

impl ViewMode {
    /// The mode for a `live_preview` setting value: rendered when on, editor when off.
    pub fn from_live_preview(live: bool) -> Self {
        if live { Self::Live } else { Self::Source }
    }

    /// A single click on the rendered text: `Live` becomes `Source`, other modes stay.
    pub fn on_click(self) -> Self {
        match self {
            Self::Live => Self::Source,
            other => other,
        }
    }

    /// Escape: `Source` returns to `Live`, other modes stay.
    pub fn on_escape(self) -> Self {
        match self {
            Self::Source => Self::Live,
            other => other,
        }
    }

    pub fn is_source(self) -> bool {
        self == Self::Source
    }
}

/// When the `live_preview` setting changed since it was last seen, the mode follows it.
pub fn mode_after_setting(seen: bool, now: bool) -> Option<ViewMode> {
    if seen == now {
        None
    } else {
        Some(ViewMode::from_live_preview(now))
    }
}

/// Restartable delay bookkeeping. `restart` starts a new delay and returns its token; a timer
/// that wakes up with an older token must do nothing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Debounce {
    generation: u64,
}

impl Debounce {
    pub fn restart(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    pub fn is_current(&self, token: u64) -> bool {
        self.generation == token
    }
}

/// What the status bar shows about the active note's file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStatus {
    Saved,
    Unsaved,
    Saving,
    Failed,
}

/// Failure wins over saving, saving over unsaved, unsaved over saved.
pub fn save_status(dirty: bool, saving: bool, failed: bool) -> SaveStatus {
    if failed {
        SaveStatus::Failed
    } else if saving {
        SaveStatus::Saving
    } else if dirty {
        SaveStatus::Unsaved
    } else {
        SaveStatus::Saved
    }
}

/// Removes every entry whose path `keep` rejects and returns the removed paths.
pub fn retain_paths<V>(map: &mut HashMap<String, V>, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let removed: Vec<String> = map.keys().filter(|k| !keep(k)).cloned().collect();
    for path in &removed {
        map.remove(path);
    }
    removed
}

/// Destination position of the tab at `from` when it is dropped into the gap `slot` (`0..=len`,
/// the gap in front of tab `slot`; `len` is the gap after the last tab). Returns `None` when the
/// tab would stay where it is or the input is out of range.
pub fn reorder_dest(len: usize, from: usize, slot: usize) -> Option<usize> {
    if from >= len || slot > len {
        return None;
    }
    let to = if slot > from { slot - 1 } else { slot };
    (to != from).then_some(to)
}

/// Moves the item at `from` to position `to` (both positions in the list before the move).
/// Returns false, and changes nothing, when either position is out of range.
pub fn move_item<T>(list: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from >= list.len() || to >= list.len() {
        return false;
    }
    let item = list.remove(from);
    list.insert(to, item);
    true
}

/// The active tab's position after the tab at `from` moved to `to`. The active tab follows its
/// own item; the tabs in between shift by one.
pub fn active_after_move(active: usize, from: usize, to: usize) -> usize {
    if active == from {
        to
    } else if from < active && active <= to {
        active - 1
    } else if to <= active && active < from {
        active + 1
    } else {
        active
    }
}

/// The tab that Ctrl+PageUp / Ctrl+PageDown switches to: wraps around the strip.
pub fn cycle_index(len: usize, active: usize, forward: bool) -> Option<usize> {
    if len == 0 || active >= len {
        return None;
    }
    Some(if forward {
        (active + 1) % len
    } else {
        (active + len - 1) % len
    })
}

/// The position the active tab moves to with Ctrl+Shift+PageUp / PageDown: one step, stopping at
/// the ends of the strip. `None` when it cannot move.
pub fn step_index(len: usize, active: usize, forward: bool) -> Option<usize> {
    if len == 0 || active >= len {
        return None;
    }
    if forward && active + 1 < len {
        Some(active + 1)
    } else if !forward && active > 0 {
        Some(active - 1)
    } else {
        None
    }
}

/// Workspace layout as saved in the `[workspace]` section of `<vault>/.jiyue/state.toml`. The tabs and the active tab
/// are those of the focused pane; `split` holds the other pane when the view is split. Every
/// field has a default, so a file written by an older build (or a partial one) still loads.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceFile {
    pub version: u32,
    pub tabs: Vec<String>,
    pub active: Option<String>,
    pub split: Option<SplitFile>,
}

/// The second pane of a split view, and the split geometry.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitFile {
    /// `true` for a stacked split (向下拆分), `false` for side by side (向右拆分).
    pub down: bool,
    /// Share of the first pane (left or top), `0..1`.
    pub ratio: f32,
    /// Which pane has the focus: `0` first (left or top), `1` second.
    pub focused: usize,
    pub other_tabs: Vec<String>,
    pub other_active: Option<String>,
}

/// Version written by this build.
pub const WORKSPACE_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_maps_to_default_mode() {
        assert_eq!(ViewMode::from_live_preview(true), ViewMode::Live);
        assert_eq!(ViewMode::from_live_preview(false), ViewMode::Source);
    }

    #[test]
    fn click_only_leaves_live() {
        assert_eq!(ViewMode::Live.on_click(), ViewMode::Source);
        assert_eq!(ViewMode::Read.on_click(), ViewMode::Read);
        assert_eq!(ViewMode::Source.on_click(), ViewMode::Source);
    }

    #[test]
    fn escape_only_leaves_source() {
        assert_eq!(ViewMode::Source.on_escape(), ViewMode::Live);
        assert_eq!(ViewMode::Live.on_escape(), ViewMode::Live);
        assert_eq!(ViewMode::Read.on_escape(), ViewMode::Read);
    }

    #[test]
    fn click_then_escape_round_trip() {
        let m = ViewMode::Live.on_click().on_escape();
        assert_eq!(m, ViewMode::Live);
    }

    #[test]
    fn settings_change_switches_mode_only_on_change() {
        assert_eq!(mode_after_setting(true, true), None);
        assert_eq!(mode_after_setting(true, false), Some(ViewMode::Source));
        assert_eq!(mode_after_setting(false, true), Some(ViewMode::Live));
    }

    #[test]
    fn debounce_only_latest_token_is_current() {
        let mut d = Debounce::default();
        let first = d.restart();
        let second = d.restart();
        assert!(!d.is_current(first));
        assert!(d.is_current(second));
    }

    #[test]
    fn save_status_priority() {
        assert_eq!(save_status(false, false, false), SaveStatus::Saved);
        assert_eq!(save_status(true, false, false), SaveStatus::Unsaved);
        assert_eq!(save_status(true, true, false), SaveStatus::Saving);
        assert_eq!(save_status(false, true, true), SaveStatus::Failed);
    }

    #[test]
    fn retain_paths_keeps_open_notes_and_reports_removed() {
        let mut map: HashMap<String, u32> = HashMap::new();
        map.insert("a.md".into(), 1);
        map.insert("b.md".into(), 2);
        map.insert("c.md".into(), 3);
        let removed = retain_paths(&mut map, |p| p != "b.md");
        assert_eq!(removed, vec!["b.md".to_string()]);
        assert!(map.contains_key("a.md"));
        assert!(!map.contains_key("b.md"));
        assert!(map.contains_key("c.md"));
    }

    #[test]
    fn reorder_moves_left_and_right() {
        // Gap 0 is before tab 0; gap 3 is after the last of three tabs.
        assert_eq!(reorder_dest(3, 0, 3), Some(2));
        assert_eq!(reorder_dest(3, 2, 0), Some(0));
        assert_eq!(reorder_dest(3, 0, 2), Some(1));
        assert_eq!(reorder_dest(3, 1, 2), None, "dropped right back in place");
        assert_eq!(reorder_dest(3, 1, 1), None, "gap before itself is a no-op");
        assert_eq!(reorder_dest(3, 3, 0), None, "source out of range");
        assert_eq!(reorder_dest(3, 0, 4), None, "slot out of range");
    }

    #[test]
    fn move_item_keeps_the_rest_in_order() {
        let mut v = vec!["a", "b", "c", "d"];
        assert!(move_item(&mut v, 0, 2));
        assert_eq!(v, vec!["b", "c", "a", "d"]);
        assert!(move_item(&mut v, 3, 0));
        assert_eq!(v, vec!["d", "b", "c", "a"]);
        assert!(!move_item(&mut v, 9, 0));
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn active_tab_follows_its_item() {
        // Moving the active tab itself.
        assert_eq!(active_after_move(1, 1, 3), 3);
        // Moving a tab from before the active one to after it shifts the active one left.
        assert_eq!(active_after_move(2, 0, 3), 1);
        // Moving a tab from after the active one to before it shifts the active one right.
        assert_eq!(active_after_move(1, 3, 0), 2);
        // Unrelated tabs leave it alone.
        assert_eq!(active_after_move(0, 2, 3), 0);
    }

    #[test]
    fn switching_wraps_and_moving_stops_at_the_ends() {
        assert_eq!(cycle_index(3, 2, true), Some(0));
        assert_eq!(cycle_index(3, 0, false), Some(2));
        assert_eq!(cycle_index(3, 1, true), Some(2));
        assert_eq!(cycle_index(0, 0, true), None);
        assert_eq!(step_index(3, 2, true), None);
        assert_eq!(step_index(3, 0, false), None);
        assert_eq!(step_index(3, 0, true), Some(1));
        assert_eq!(step_index(3, 1, false), Some(0));
    }

    #[test]
    fn workspace_round_trips_with_split() {
        let file = WorkspaceFile {
            version: WORKSPACE_VERSION,
            tabs: vec!["a.md".into(), "b.md".into()],
            active: Some("b.md".into()),
            split: Some(SplitFile {
                down: true,
                ratio: 0.25,
                focused: 1,
                other_tabs: vec!["c.md".into()],
                other_active: Some("c.md".into()),
            }),
        };
        let text = toml::to_string(&file).expect("serialise");
        assert_eq!(toml::from_str::<WorkspaceFile>(&text).expect("parse"), file);
    }

    #[test]
    fn partial_workspace_loads_as_single_pane() {
        let file: WorkspaceFile = toml::from_str("tabs = [\"a.md\"]").expect("partial file loads");
        assert_eq!(file.tabs, vec!["a.md".to_string()]);
        assert!(file.split.is_none());
        let empty: WorkspaceFile = toml::from_str("").expect("empty file loads");
        assert!(empty.tabs.is_empty());
    }

    #[test]
    fn broken_workspace_is_an_error() {
        assert!(toml::from_str::<WorkspaceFile>("tabs = [").is_err());
        assert!(toml::from_str::<WorkspaceFile>("tabs = 5").is_err());
    }

    #[test]
    fn retain_paths_keyed_by_path_survives_index_shift() {
        // Indices change when notes are removed; paths do not.
        let mut map: HashMap<String, &str> = HashMap::new();
        map.insert("x/one.md".into(), "editor one");
        map.insert("two.md".into(), "editor two");
        let removed = retain_paths(&mut map, |p| p == "two.md");
        assert_eq!(removed, vec!["x/one.md".to_string()]);
        assert_eq!(map.get("two.md"), Some(&"editor two"));
    }
}
