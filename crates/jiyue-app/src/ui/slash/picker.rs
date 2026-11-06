//! The pickers that the image, 嵌入笔记 and 双链 commands open: which rows they list, the link text
//! a row inserts, and whether the placeholder the command left is still in the note. Pure logic.
//!
//! The image picker lists the image files of the vault (`![[name.png]]`, the name alone when no
//! other file has it, the vault path otherwise). The note pickers list the notes by title
//! (`![[Title]]` for 嵌入笔记, `[[Title]]` for 双链; the path without `.md` when two notes share the
//! title). The first row of the image picker, 从电脑选择图片…, is not a file: it has no link.

use std::collections::HashMap;
use std::ops::Range;

use crate::ui::palette::note_rows;

/// Extensions the image picker lists (lower case, without the dot).
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"];

/// Which list a command's picker shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// `![[` + file + `]]`, the vault's image files.
    Image,
    /// `![[` + title + `]]`, the notes.
    Embed,
    /// `[[` + title + `]]`, the notes.
    Link,
}

/// One row that inserts a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// What the row shows first: the file name, or the note title.
    pub label: String,
    /// The vault-relative path, shown under the label.
    pub detail: String,
    /// The text the row inserts, brackets included.
    pub link: String,
}

/// True when `rel` names an image file the picker lists.
pub fn is_image(rel: &str) -> bool {
    rel.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The last segment of a vault-relative path.
pub fn base_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Lower-cased `s`, the key the uniqueness checks compare with.
fn key(s: &str) -> String {
    s.to_lowercase()
}

/// The image rows of the vault: every image file, in path order.
pub fn image_candidates(files: &[String]) -> Vec<Candidate> {
    let mut names: HashMap<String, usize> = HashMap::new();
    for f in files {
        *names.entry(key(base_name(f))).or_insert(0) += 1;
    }
    files
        .iter()
        .filter(|f| is_image(f))
        .map(|rel| {
            let base = base_name(rel);
            let unique = names.get(&key(base)).copied().unwrap_or(0) <= 1;
            let target = if unique { base } else { rel.as_str() };
            Candidate {
                label: base.to_string(),
                detail: rel.clone(),
                link: format!("![[{target}]]"),
            }
        })
        .collect()
}

/// The note rows: `(title, path)` pairs, in the order given. The link is the title when no other
/// note has it (case-insensitive), the path without `.md` otherwise.
pub fn note_candidates(notes: &[(String, String)], target: Target) -> Vec<Candidate> {
    let mut titles: HashMap<String, usize> = HashMap::new();
    for (title, _) in notes {
        *titles.entry(key(title)).or_insert(0) += 1;
    }
    notes
        .iter()
        .map(|(title, path)| {
            let unique = titles.get(&key(title)).copied().unwrap_or(0) <= 1;
            let inner = if unique {
                title.clone()
            } else {
                path.strip_suffix(".md").unwrap_or(path).to_string()
            };
            let link = match target {
                Target::Embed => format!("![[{inner}]]"),
                Target::Link | Target::Image => format!("[[{inner}]]"),
            };
            Candidate {
                label: title.clone(),
                detail: path.clone(),
                link,
            }
        })
        .collect()
}

/// The rows shown for `query`: `None` is the first row (从电脑选择图片…) when `browse` is set, and
/// `Some(i)` is candidate `i`. Candidates are matched on their labels, as the palette matches notes.
pub fn visible(labels: &[String], browse: bool, query: &str) -> Vec<Option<usize>> {
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut rows = Vec::new();
    if browse {
        rows.push(None);
    }
    rows.extend(note_rows(&refs, &[], query).into_iter().map(Some));
    rows
}

/// True when the placeholder the command left (`![[]]` or `[[]]`) is still at `anchor` in `doc`,
/// so the picker's choice can replace it.
pub fn anchor_matches(doc: &str, anchor: &Range<usize>, placeholder: &str) -> bool {
    doc.get(anchor.clone()) == Some(placeholder)
}

/// The caret range after `link` has replaced the placeholder at `anchor`: just after the link.
pub fn caret_after(anchor: &Range<usize>, link: &str) -> Range<usize> {
    let end = anchor.start + link.len();
    end..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn notes(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(t, p)| (t.to_string(), p.to_string()))
            .collect()
    }

    #[test]
    fn image_extensions_are_case_insensitive() {
        assert!(is_image("photos/cat.PNG"));
        assert!(is_image("a.jpeg"));
        assert!(is_image("x.svg"));
        assert!(!is_image("notes/a.md"));
        assert!(!is_image("noext"));
        assert!(!is_image("dir.x/file"));
    }

    #[test]
    fn image_rows_list_only_images_in_path_order() {
        let files = s(&["a.md", "img/b.png", "img/c.txt", "d.JPG"]);
        let rows = image_candidates(&files);
        let labels: Vec<&str> = rows.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["b.png", "d.JPG"]);
        assert_eq!(rows[0].detail, "img/b.png");
    }

    #[test]
    fn image_link_is_the_name_when_unique_and_the_path_when_not() {
        let files = s(&["img/cat.png", "old/cat.png", "dog.webp"]);
        let rows = image_candidates(&files);
        assert_eq!(rows[0].link, "![[img/cat.png]]");
        assert_eq!(rows[1].link, "![[old/cat.png]]");
        assert_eq!(rows[2].link, "![[dog.webp]]");
    }

    #[test]
    fn note_link_uses_the_title_or_the_path_when_titles_repeat() {
        let list = notes(&[
            ("Rust", "a/Rust.md"),
            ("rust", "b/rust.md"),
            ("Go", "Go.md"),
        ]);
        let links: Vec<String> = note_candidates(&list, Target::Link)
            .into_iter()
            .map(|c| c.link)
            .collect();
        assert_eq!(links, vec!["[[a/Rust]]", "[[b/rust]]", "[[Go]]"]);
    }

    #[test]
    fn embed_links_add_the_bang() {
        let list = notes(&[("Plan", "Plan.md")]);
        let rows = note_candidates(&list, Target::Embed);
        assert_eq!(rows[0].link, "![[Plan]]");
        assert_eq!(rows[0].label, "Plan");
        assert_eq!(rows[0].detail, "Plan.md");
    }

    #[test]
    fn browse_row_comes_first_and_is_always_shown() {
        let labels = s(&["cat.png", "dog.png"]);
        assert_eq!(visible(&labels, true, "zzz"), vec![None]);
        assert_eq!(visible(&labels, true, "dog"), vec![None, Some(1)]);
        assert_eq!(visible(&labels, false, "cat"), vec![Some(0)]);
    }

    #[test]
    fn the_placeholder_must_still_be_in_place() {
        let doc = "text ![[]] tail";
        assert!(anchor_matches(doc, &(5..10), "![[]]"));
        assert!(!anchor_matches("text ![[x]] tail", &(5..10), "![[]]"));
        assert!(
            !anchor_matches(doc, &(5..40), "![[]]"),
            "out of range is not a match"
        );
    }

    #[test]
    fn caret_goes_after_the_inserted_link() {
        assert_eq!(caret_after(&(2..7), "![[cat.png]]"), 14..14);
    }

    #[test]
    fn replacing_the_placeholder_gives_the_link_in_the_note() {
        let doc = "x ![[]] y";
        let anchor = 2..7;
        assert!(anchor_matches(doc, &anchor, "![[]]"));
        let link = "![[cat.png]]";
        let out = format!("{}{}{}", &doc[..anchor.start], link, &doc[anchor.end..]);
        assert_eq!(out, "x ![[cat.png]] y");
        assert_eq!(caret_after(&anchor, link), 14..14);
    }
}
