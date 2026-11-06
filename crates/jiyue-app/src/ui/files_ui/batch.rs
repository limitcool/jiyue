//! Plans of the batch actions on a multi-selection: the moves (with names that do not collide), the
//! bookmark toggles and the text of the copied paths. Pure logic, unit tested. The UI runs the plans
//! (`actions::move_items`, the menu in `menu.rs`).

use std::path::Path;

use super::pure::{self, MoveCheck};

/// Entries tried for a free name, as in the core's restore naming.
const MAX_UNIQUE_ATTEMPTS: usize = 1000;

/// The moves of a batch: `(source, destination, is_dir)`, plus the rows that do not move.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MovePlan {
    pub moves: Vec<(String, String, bool)>,
    /// Rows that are already in the destination.
    pub unchanged: usize,
    /// Rows that cannot go there (a folder into itself), with the reason.
    pub invalid: Vec<String>,
}

/// Plans the move of `items` (`(path, is_dir)`, effective roots) into `dest_dir`. `taken` tells
/// whether a vault path exists. A destination that is taken, or that an earlier move of the batch
/// already uses, gets the first free `stem N.ext` name.
pub fn plan_moves(
    items: &[(String, bool)],
    dest_dir: &str,
    taken: &dyn Fn(&str) -> bool,
) -> MovePlan {
    let mut plan = MovePlan::default();
    let mut used: Vec<String> = Vec::new();
    for (src, is_dir) in items {
        match pure::move_target(src, *is_dir, dest_dir) {
            MoveCheck::NoOp => plan.unchanged += 1,
            MoveCheck::Invalid(message) => plan.invalid.push(message),
            MoveCheck::Move(wanted) => {
                let is_free = |p: &str| !taken(p) && !used.iter().any(|u| u == p);
                let dst = free_name(&wanted, &is_free);
                used.push(dst.clone());
                plan.moves.push((src.clone(), dst, *is_dir));
            }
        }
    }
    plan
}

/// `rel` when `is_free` accepts it, else `stem N.ext` for the first N that it accepts.
pub fn free_name(rel: &str, is_free: &dyn Fn(&str) -> bool) -> String {
    if is_free(rel) {
        return rel.to_string();
    }
    let (dir, file) = match rel.rfind('/') {
        Some(i) => (&rel[..i], &rel[i + 1..]),
        None => ("", rel),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    for n in 1..MAX_UNIQUE_ATTEMPTS {
        let candidate = if dir.is_empty() {
            format!("{stem} {n}{ext}")
        } else {
            format!("{dir}/{stem} {n}{ext}")
        };
        if is_free(&candidate) {
            return candidate;
        }
    }
    rel.to_string()
}

/// The paths to toggle for a bookmark batch. When every row is bookmarked they are all removed;
/// otherwise the rows that are not bookmarked are added. Returns the paths to toggle, in order.
pub fn bookmark_toggles(paths: &[String], is_bookmarked: &dyn Fn(&str) -> bool) -> Vec<String> {
    let all = !paths.is_empty() && paths.iter().all(|p| is_bookmarked(p));
    paths
        .iter()
        .filter(|p| all || !is_bookmarked(p))
        .cloned()
        .collect()
}

/// The text copied for a batch: the absolute paths, one per line.
pub fn paths_text(root: &Path, rels: &[String]) -> String {
    rels.iter()
        .map(|rel| root.join(rel).display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: &[(&str, bool)]) -> Vec<(String, bool)> {
        v.iter().map(|(p, d)| (p.to_string(), *d)).collect()
    }

    #[test]
    fn moves_keep_names_and_skip_rows_already_there() {
        let plan = plan_moves(&items(&[("a/x.md", false), ("b.md", false)]), "a", &|_| {
            false
        });
        assert_eq!(
            plan.moves,
            vec![("b.md".to_string(), "a/b.md".to_string(), false)]
        );
        assert_eq!(plan.unchanged, 1);
        assert!(plan.invalid.is_empty());
    }

    #[test]
    fn a_folder_cannot_go_into_itself() {
        let plan = plan_moves(&items(&[("a", true), ("c.md", false)]), "a", &|_| false);
        assert_eq!(plan.invalid.len(), 1);
        assert_eq!(plan.moves.len(), 1);
    }

    #[test]
    fn a_taken_name_gets_the_first_free_number() {
        let taken = |p: &str| p == "dest/n.md" || p == "dest/n 1.md";
        let plan = plan_moves(&items(&[("n.md", false)]), "dest", &taken);
        assert_eq!(plan.moves[0].1, "dest/n 2.md");
    }

    #[test]
    fn two_items_with_one_name_do_not_collide_inside_the_batch() {
        let plan = plan_moves(
            &items(&[("x/a.md", false), ("y/a.md", false)]),
            "z",
            &|_| false,
        );
        let dsts: Vec<&str> = plan.moves.iter().map(|m| m.1.as_str()).collect();
        assert_eq!(dsts, vec!["z/a.md", "z/a 1.md"]);
    }

    #[test]
    fn folder_names_without_extension_are_numbered_at_the_end() {
        assert_eq!(
            free_name("dest/proj", &|p| !(p == "dest/proj" || p == "dest/proj 1")),
            "dest/proj 2"
        );
    }

    #[test]
    fn bookmark_batch_adds_the_missing_rows() {
        let marked = |p: &str| p == "a.md";
        let toggles = bookmark_toggles(&["a.md".into(), "b.md".into()], &marked);
        assert_eq!(toggles, vec!["b.md".to_string()]);
    }

    #[test]
    fn bookmark_batch_removes_when_all_are_marked() {
        let marked = |_: &str| true;
        let toggles = bookmark_toggles(&["a.md".into(), "b.md".into()], &marked);
        assert_eq!(toggles.len(), 2);
    }

    #[test]
    fn copied_paths_are_one_per_line() {
        let text = paths_text(Path::new("root"), &["a.md".into(), "d/b.md".into()]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("a.md"));
        assert!(lines[1].ends_with("b.md"));
    }
}
