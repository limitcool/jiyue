//! Pure layout model of the split view (分屏): two panes side by side or stacked, the share of
//! the first pane, the focus, and the tab state of the pane that does not have the focus.
//! No gpui types live here, so everything below is unit-tested directly.
//!
//! The focused pane's tabs are kept in `vault.open_tabs` (so every existing feature keeps working
//! on the focused note). The other pane is stored here as note paths, which survive index shifts
//! when notes are added or removed.

/// Smallest width (or height, when stacked) a pane may be dragged to, in px.
pub const MIN_PANE_PX: f32 = 320.0;
/// Width of the divider between the panes, in px.
pub const DIVIDER_PX: f32 = 6.0;

/// How the two panes are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    /// Side by side (向右拆分): the panes are left and right.
    Right,
    /// Stacked (向下拆分): the panes are top and bottom.
    Down,
}

/// The tabs of one pane, as note paths in tab order, and its active tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneState {
    pub tabs: Vec<String>,
    pub active: Option<String>,
}

/// A split view. `focused` is the physical pane with the focus (`0` is left or top); the pane
/// that does not have the focus is stored in `other`.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitState {
    pub dir: SplitDir,
    /// Share of the first pane (left or top) of the space the two panes and the divider take.
    pub ratio: f32,
    pub focused: usize,
    pub other: PaneState,
}

/// Keeps the split ratio inside what the two minimum pane sizes allow, for a container of
/// `total_px` (its width when side by side, its height when stacked). A container too small for
/// two panes of the minimum size gets an even split.
pub fn clamp_ratio(ratio: f32, total_px: f32) -> f32 {
    if !ratio.is_finite() || !total_px.is_finite() || total_px <= 0.0 {
        return 0.5;
    }
    if total_px - DIVIDER_PX < 2.0 * MIN_PANE_PX {
        return 0.5;
    }
    let lowest = MIN_PANE_PX / total_px;
    let highest = (total_px - DIVIDER_PX - MIN_PANE_PX) / total_px;
    ratio.clamp(lowest, highest)
}

/// The ratio for a divider whose centre is `offset_px` from the start of the container of
/// `total_px`, clamped like `clamp_ratio`.
pub fn ratio_from_offset(offset_px: f32, total_px: f32) -> f32 {
    if !total_px.is_finite() || total_px <= 0.0 {
        return 0.5;
    }
    clamp_ratio(offset_px / total_px, total_px)
}

/// The pane that Ctrl+Alt+Left/Up (`forward == false`) or Ctrl+Alt+Right/Down (`forward == true`)
/// moves the focus to. `None` when the focus is already at that end.
pub fn neighbour(focused: usize, forward: bool) -> Option<usize> {
    match (focused, forward) {
        (0, true) => Some(1),
        (1, false) => Some(0),
        _ => None,
    }
}

/// What closing a tab does to the split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// The pane keeps at least one tab (or there is no split): nothing else changes.
    KeepPane,
    /// The pane has no tabs left: the pane closes and the other pane fills the view.
    ClosePane,
}

/// Closing a tab in a split view: a pane that loses its last tab closes.
pub fn after_tab_close(split: bool, tabs_left: usize) -> CloseOutcome {
    if split && tabs_left == 0 {
        CloseOutcome::ClosePane
    } else {
        CloseOutcome::KeepPane
    }
}

/// Drops the tabs whose note no longer exists and repairs the active tab. Returns true when the
/// pane has no tab left.
pub fn prune_pane(pane: &mut PaneState, exists: impl Fn(&str) -> bool) -> bool {
    pane.tabs.retain(|p| exists(p));
    let active_ok = pane
        .active
        .as_ref()
        .is_some_and(|a| pane.tabs.iter().any(|p| p == a));
    if !active_ok {
        pane.active = pane.tabs.first().cloned();
    }
    pane.tabs.is_empty()
}

/// Position of the active tab of `pane` (`0` when it has none).
pub fn active_position(pane: &PaneState) -> usize {
    pane.active
        .as_ref()
        .and_then(|a| pane.tabs.iter().position(|p| p == a))
        .unwrap_or(0)
}

/// Maps every note path of `pane` (its tabs and its active tab) with `map`, after a rename or a
/// move of notes. Returns true when a path changed.
pub fn remap_pane(pane: &mut PaneState, map: impl Fn(&str) -> String) -> bool {
    let before = pane.clone();
    for p in pane.tabs.iter_mut() {
        *p = map(p);
    }
    if let Some(a) = pane.active.as_mut() {
        *a = map(a);
    }
    *pane != before
}

/// Splits a tab off a single pane. Returns the new pane (holding only the moved tab), the rest
/// of the tabs, and the rest's active position. `None` when the pane has one tab or fewer, or
/// the position is out of range: a split must leave both panes with a tab.
pub fn detach_tab(
    tabs: &[String],
    active_pos: usize,
    pos: usize,
) -> Option<(PaneState, PaneState, usize)> {
    if tabs.len() < 2 || pos >= tabs.len() {
        return None;
    }
    let moved = tabs[pos].clone();
    let mut rest = tabs.to_vec();
    rest.remove(pos);
    let rest_active = if active_pos == pos {
        // The active tab left: the one that took its place (or the last one) becomes active.
        pos.min(rest.len() - 1)
    } else if active_pos > pos {
        active_pos - 1
    } else {
        active_pos
    };
    let new_pane = PaneState {
        tabs: vec![moved.clone()],
        active: Some(moved),
    };
    let rest_pane = PaneState {
        active: rest.get(rest_active).cloned(),
        tabs: rest,
    };
    Some((new_pane, rest_pane, rest_active))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ratio_is_clamped_to_the_minimum_pane_width() {
        // 1000 px: the first pane needs at least 320 px, the second at least 320 px after the divider.
        let lo = clamp_ratio(0.0, 1000.0);
        let hi = clamp_ratio(1.0, 1000.0);
        assert!((lo - 0.32).abs() < 1e-6);
        assert!((hi - (1000.0 - 6.0 - 320.0) / 1000.0).abs() < 1e-6);
        assert!(
            (clamp_ratio(0.5, 1000.0) - 0.5).abs() < 1e-6,
            "inside the range it is kept"
        );
    }

    #[test]
    fn ratio_falls_back_to_even_split_when_too_small() {
        assert_eq!(
            clamp_ratio(0.9, 600.0),
            0.5,
            "600 px cannot hold two 320 px panes"
        );
        assert_eq!(clamp_ratio(0.2, 0.0), 0.5);
        assert_eq!(clamp_ratio(f32::NAN, 1000.0), 0.5);
    }

    #[test]
    fn divider_offset_maps_to_a_clamped_ratio() {
        assert!((ratio_from_offset(500.0, 1000.0) - 0.5).abs() < 1e-6);
        // Dragging to the far left stops at the minimum width.
        assert!((ratio_from_offset(10.0, 1000.0) - 0.32).abs() < 1e-6);
        assert_eq!(ratio_from_offset(10.0, 0.0), 0.5);
    }

    #[test]
    fn focus_moves_between_the_two_panes_only() {
        assert_eq!(neighbour(0, true), Some(1));
        assert_eq!(neighbour(1, false), Some(0));
        assert_eq!(neighbour(0, false), None);
        assert_eq!(neighbour(1, true), None);
    }

    #[test]
    fn closing_the_last_tab_of_a_split_pane_closes_the_pane() {
        assert_eq!(after_tab_close(true, 0), CloseOutcome::ClosePane);
        assert_eq!(after_tab_close(true, 2), CloseOutcome::KeepPane);
        assert_eq!(after_tab_close(false, 0), CloseOutcome::KeepPane);
    }

    #[test]
    fn prune_drops_missing_notes_and_repairs_active() {
        let mut pane = PaneState {
            tabs: paths(&["a.md", "gone.md", "b.md"]),
            active: Some("gone.md".into()),
        };
        let empty = prune_pane(&mut pane, |p| p != "gone.md");
        assert!(!empty);
        assert_eq!(pane.tabs, paths(&["a.md", "b.md"]));
        assert_eq!(pane.active.as_deref(), Some("a.md"));
        assert_eq!(active_position(&pane), 0);

        let mut none = PaneState {
            tabs: paths(&["gone.md"]),
            active: None,
        };
        assert!(prune_pane(&mut none, |_| false));
    }

    #[test]
    fn detach_leaves_both_panes_with_a_tab() {
        let tabs = paths(&["a.md", "b.md", "c.md"]);
        // Moving the active tab (b) out: c takes its place and becomes active.
        let (new, rest, rest_active) = detach_tab(&tabs, 1, 1).expect("split possible");
        assert_eq!(new.tabs, paths(&["b.md"]));
        assert_eq!(new.active.as_deref(), Some("b.md"));
        assert_eq!(rest.tabs, paths(&["a.md", "c.md"]));
        assert_eq!(rest_active, 1);
        assert_eq!(rest.active.as_deref(), Some("c.md"));

        // Moving a tab that is after the active one keeps the active tab.
        let (_, rest, rest_active) = detach_tab(&tabs, 0, 2).expect("split possible");
        assert_eq!(rest.tabs, paths(&["a.md", "b.md"]));
        assert_eq!(rest_active, 0);

        // Moving the last tab when it is active: the new last tab becomes active.
        let (_, rest, rest_active) = detach_tab(&tabs, 2, 2).expect("split possible");
        assert_eq!(rest_active, 1);
        assert_eq!(rest.active.as_deref(), Some("b.md"));
    }

    #[test]
    fn remap_follows_renamed_and_moved_notes_in_the_other_pane() {
        let mut pane = PaneState {
            tabs: paths(&["a.md", "old/b.md", "c.md"]),
            active: Some("old/b.md".into()),
        };
        // A rename of a.md and a move of the folder old/ to new/.
        let changed = remap_pane(&mut pane, |p| match p {
            "a.md" => "a-renamed.md".to_string(),
            p => match p.strip_prefix("old/") {
                Some(rest) => format!("new/{rest}"),
                None => p.to_string(),
            },
        });
        assert!(changed);
        assert_eq!(pane.tabs, paths(&["a-renamed.md", "new/b.md", "c.md"]));
        assert_eq!(pane.active.as_deref(), Some("new/b.md"));
        assert_eq!(active_position(&pane), 1, "the active tab keeps its place");

        // Nothing that matches: nothing changes.
        assert!(!remap_pane(&mut pane, |p| p.to_string()));
    }

    #[test]
    fn detach_needs_two_tabs_and_a_valid_position() {
        assert!(detach_tab(&paths(&["a.md"]), 0, 0).is_none());
        assert!(detach_tab(&paths(&["a.md", "b.md"]), 0, 5).is_none());
        assert!(detach_tab(&[], 0, 0).is_none());
    }
}
