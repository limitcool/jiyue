//! Importing files into the vault (pasted clipboard images, dropped files) and the text edits that
//! put their links into a note. The job runs on a background thread; the text helpers are pure.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use jiyue_md::attach::{
    AttachSettings, Timestamp, plan_import_checked, sanitize_component, write_import,
};
use jiyue_md::embeds::{generate_block_id, parse_block_ids};

use crate::daily::Date;

/// Something to import: clipboard bytes (named by the app) or a file on disk. `ext` is the image
/// type the clipboard reported (`png`, `jpg`, ...); without it the bytes are sniffed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Clipboard {
        bytes: Vec<u8>,
        ext: Option<&'static str>,
    },
    File(PathBuf),
}

/// File extension for a clipboard image type.
pub fn ext_for_format(format: gpui::ImageFormat) -> Option<&'static str> {
    use gpui::ImageFormat as F;
    match format {
        F::Png => Some("png"),
        F::Jpeg => Some("jpg"),
        F::Gif => Some("gif"),
        F::Webp => Some("webp"),
        F::Bmp => Some("bmp"),
        F::Svg => Some("svg"),
        _ => None,
    }
}

/// The name of a pasted clipboard image: `<prefix> <stamp>.<ext>`, or no name when the type is
/// unknown (the bytes are then sniffed).
fn clipboard_name(job: &Job, ext: Option<&str>) -> Option<String> {
    let ext = ext?;
    let now = job.now;
    let prefix = sanitize_component(&job.settings.default_pasted_prefix);
    let prefix = if prefix.is_empty() {
        "Pasted image".to_string()
    } else {
        prefix
    };
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.y, now.m, now.d, now.hh, now.mm, now.ss
    );
    Some(format!("{prefix} {stamp}.{ext}"))
}

/// Everything a background import needs. It owns copies, so it can run off the UI thread.
#[derive(Debug, Clone)]
pub struct Job {
    pub root: PathBuf,
    pub settings: AttachSettings,
    /// The note the links go into (vault-relative).
    pub note_path: String,
    pub sources: Vec<Source>,
    pub now: Timestamp,
    /// File names that already exist somewhere in the vault (for the link form).
    pub name_taken: HashSet<String>,
}

/// The result of importing one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Saved { dest_rel: String, link: String },
    Failed(String),
}

/// Imports every source of `job`, in order. Blocking file work: run it on a background thread.
pub fn run_job(job: &Job) -> Vec<Outcome> {
    job.sources.iter().map(|src| import_one(job, src)).collect()
}

fn import_one(job: &Job, src: &Source) -> Outcome {
    let (bytes, name) = match src {
        Source::Clipboard { bytes, ext } => (bytes.clone(), clipboard_name(job, *ext)),
        Source::File(path) => match fs::read(path) {
            Ok(bytes) => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string);
                (bytes, name)
            }
            Err(e) => return Outcome::Failed(format!("读取失败：{e}")),
        },
    };
    if bytes.is_empty() {
        return Outcome::Failed("文件是空的".to_string());
    }
    let root = &job.root;
    let plan = plan_import_checked(
        &job.settings,
        &job.note_path,
        name.as_deref(),
        &bytes,
        job.now,
        &|rel: &str| fs::read(root.join(rel)).ok(),
        &|file: &str| job.name_taken.contains(file),
    );
    match write_import(root, &plan, &bytes) {
        Ok(()) => Outcome::Saved {
            dest_rel: plan.dest_rel,
            link: plan.link_text,
        },
        Err(e) => Outcome::Failed(format!("保存失败：{e}")),
    }
}

/// The current local date and the time of day, for generated names.
pub fn now_stamp() -> Timestamp {
    let date = Date::today();
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86_400;
    Timestamp {
        y: date.year as u32,
        m: date.month,
        d: date.day,
        hh: (secs / 3600) as u32,
        mm: ((secs % 3600) / 60) as u32,
        ss: (secs % 60) as u32,
    }
}

/// Byte offset of character column `col` on line `row` (both zero based). Past the end of the
/// line it is the line end; past the end of the text it is the text length.
pub fn offset_of(text: &str, row: usize, col: usize) -> usize {
    let mut start = 0usize;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if i == row {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let body = body.strip_suffix('\r').unwrap_or(body);
            return match body.char_indices().nth(col) {
                Some((byte, _)) => start + byte,
                None => start + body.len(),
            };
        }
        start += line.len();
    }
    text.len()
}

/// Inserts `insert` at (`row`, `col`) and returns the new text with the caret after the insert
/// (zero based row and column).
pub fn insert_at(text: &str, row: usize, col: usize, insert: &str) -> (String, usize, usize) {
    let at = offset_of(text, row, col);
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(&text[..at]);
    out.push_str(insert);
    out.push_str(&text[at..]);
    let newlines = insert.matches('\n').count();
    if newlines == 0 {
        (out, row, col + insert.chars().count())
    } else {
        let last = insert.rsplit('\n').next().unwrap_or("");
        (out, row + newlines, last.chars().count())
    }
}

/// Appends `insert` on a new line at the end of the text. Returns the new text and the caret
/// position (zero based row and column) at its end.
pub fn append(text: &str, insert: &str) -> (String, usize, usize) {
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(insert);
    let row = out.matches('\n').count();
    let col = out.rsplit('\n').next().unwrap_or("").chars().count();
    (out, row, col)
}

/// The block id a line ends with: `text ^id`, or a standalone `^id` line. Ids are letters,
/// digits and `-`.
pub fn block_id_on_line(line: &str) -> Option<&str> {
    let last = line.split_whitespace().last()?;
    let id = last.strip_prefix('^')?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then_some(id)
}

/// The block id that labels the block on line `row` (zero based): the id already on the line,
/// or a new one appended to the line as ` ^id`. Returns the new text and the id. `None` for a
/// blank line, which labels nothing.
pub fn ensure_block_id(text: &str, row: usize, seed: u64) -> Option<(String, String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let line = *lines.get(row)?;
    if line.trim().is_empty() {
        return None;
    }
    if let Some(id) = block_id_on_line(line) {
        return Some((text.to_string(), id.to_string()));
    }
    let existing: Vec<String> = parse_block_ids(text).into_iter().map(|b| b.id).collect();
    let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
    let id = generate_block_id(&refs, seed);
    let cr = line.ends_with('\r');
    let body = line.trim_end_matches('\r').trim_end();
    let mut new_lines: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
    new_lines[row] = format!("{body} ^{id}{}", if cr { "\r" } else { "" });
    Some((new_lines.join("\n"), id))
}

/// A seed for `generate_block_id` that changes between calls.
pub fn time_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5eed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> Timestamp {
        Timestamp {
            y: 2026,
            m: 11,
            d: 5,
            hh: 9,
            mm: 30,
            ss: 0,
        }
    }

    #[test]
    fn insert_places_text_and_moves_caret() {
        let (t, r, c) = insert_at("abc\ndef", 1, 1, "XY");
        assert_eq!(t, "abc\ndXYef");
        assert_eq!((r, c), (1, 3));
        // Start of the text.
        let (t, r, c) = insert_at("abc", 0, 0, "![[a.png]]");
        assert_eq!(t, "![[a.png]]abc");
        assert_eq!((r, c), (0, 10));
        // Past the end of the line goes to the line end.
        let (t, _, _) = insert_at("ab\ncd", 0, 9, "!");
        assert_eq!(t, "ab!\ncd");
        // Multi-line insert.
        let (t, r, c) = insert_at("x", 0, 1, "\nyz");
        assert_eq!(t, "x\nyz");
        assert_eq!((r, c), (1, 2));
    }

    #[test]
    fn insert_counts_chars_not_bytes() {
        let (t, _, c) = insert_at("笔记本", 0, 2, "!");
        assert_eq!(t, "笔记!本");
        assert_eq!(c, 3);
        assert_eq!(offset_of("笔记本", 0, 1), "笔".len());
    }

    #[test]
    fn append_adds_a_line_at_the_end() {
        assert_eq!(append("", "![[a]]"), ("![[a]]".to_string(), 0, 6));
        assert_eq!(
            append("正文\n", "![[a]]"),
            ("正文\n![[a]]".to_string(), 1, 6)
        );
        assert_eq!(append("正文", "![[a]]"), ("正文\n![[a]]".to_string(), 1, 6));
    }

    #[test]
    fn block_id_is_read_from_line_end_or_standalone() {
        assert_eq!(block_id_on_line("正文 ^abc12"), Some("abc12"));
        assert_eq!(block_id_on_line("^abc12"), Some("abc12"));
        assert_eq!(block_id_on_line("正文 ^"), None);
        assert_eq!(block_id_on_line("正文 ^a_b"), None);
        assert_eq!(block_id_on_line("正文"), None);
    }

    #[test]
    fn ensure_block_id_adds_a_new_id_once() {
        let text = "# 标题\n\n第一段\n第二段\n";
        let (new_text, id) = ensure_block_id(text, 2, 42).expect("id");
        assert_eq!(id.len(), 6);
        assert_eq!(new_text, format!("# 标题\n\n第一段 ^{id}\n第二段\n"));
        // The same paragraph keeps its id on the second call.
        let (again, id2) = ensure_block_id(&new_text, 2, 99).expect("id");
        assert_eq!(id2, id);
        assert_eq!(again, new_text);
        // A blank line labels nothing.
        assert_eq!(ensure_block_id(text, 1, 1), None);
        // The new id is found by the parser as a block reference.
        let parsed = parse_block_ids(&new_text);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, id);
    }

    #[test]
    fn ensure_block_id_keeps_new_ids_unique() {
        let text = "甲 ^ab1234\n\n乙";
        let (new_text, id) = ensure_block_id(text, 2, 7).expect("id");
        assert_ne!(id, "ab1234");
        assert!(new_text.ends_with(&format!("乙 ^{id}")));
    }

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("jiyue-import-{tag}-{nanos}"));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn job(root: PathBuf, sources: Vec<Source>, taken: &[&str]) -> Job {
        Job {
            root,
            settings: AttachSettings::default(),
            note_path: "想法/笔记.md".to_string(),
            sources,
            now: ts(),
            name_taken: taken.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn clipboard_image_is_saved_once_and_reused() {
        let root = temp_root("clip");
        let png = b"\x89PNG\r\n\x1a\nclipboard-bytes".to_vec();
        let j = job(
            root.clone(),
            vec![Source::Clipboard {
                bytes: png.clone(),
                ext: None,
            }],
            &[],
        );
        let first = run_job(&j);
        let expected_name = "Pasted image 20261105-093000.png";
        assert_eq!(
            first,
            vec![Outcome::Saved {
                dest_rel: format!("附件/{expected_name}"),
                link: format!("![[{expected_name}]]"),
            }]
        );
        assert!(root.join("附件").join(expected_name).is_file());
        // The same bytes again: the existing file is reused, nothing new is written.
        let second = run_job(&j);
        assert_eq!(first, second);
        assert!(
            !root
                .join("附件")
                .join("Pasted image 20261105-093000 1.png")
                .exists()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn clipboard_type_sets_the_extension_when_bytes_are_not_sniffable() {
        let root = temp_root("ext");
        let src = Source::Clipboard {
            bytes: b"opaque-bytes".to_vec(),
            ext: Some("jpg"),
        };
        let outcomes = run_job(&job(root.clone(), vec![src], &[]));
        assert_eq!(
            outcomes,
            vec![Outcome::Saved {
                dest_rel: "附件/Pasted image 20261105-093000.jpg".to_string(),
                link: "![[Pasted image 20261105-093000.jpg]]".to_string(),
            }]
        );
        assert_eq!(ext_for_format(gpui::ImageFormat::Png), Some("png"));
        assert_eq!(ext_for_format(gpui::ImageFormat::Tiff), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn different_bytes_get_a_suffix() {
        let root = temp_root("suffix");
        let a = Source::Clipboard {
            bytes: b"\x89PNG\r\n\x1a\nAAAA".to_vec(),
            ext: None,
        };
        let b = Source::Clipboard {
            bytes: b"\x89PNG\r\n\x1a\nBBBB".to_vec(),
            ext: None,
        };
        let outcomes = run_job(&job(root.clone(), vec![a, b], &[]));
        let names: Vec<String> = outcomes
            .iter()
            .filter_map(|o| match o {
                Outcome::Saved { dest_rel, .. } => Some(dest_rel.clone()),
                Outcome::Failed(_) => None,
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "附件/Pasted image 20261105-093000.png".to_string(),
                "附件/Pasted image 20261105-093000 1.png".to_string(),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dropped_file_keeps_its_name_and_clashes_use_the_vault_path() {
        let root = temp_root("drop");
        let src_dir = temp_root("drop-src");
        let src = src_dir.join("a b.pdf");
        fs::write(&src, b"%PDF-1.4 fake").unwrap();

        let outcomes = run_job(&job(root.clone(), vec![Source::File(src.clone())], &[]));
        assert_eq!(
            outcomes,
            vec![Outcome::Saved {
                dest_rel: "附件/a b.pdf".to_string(),
                link: "![[a b.pdf]]".to_string(),
            }]
        );
        assert!(root.join("附件").join("a b.pdf").is_file());

        // A file of the same name elsewhere in the vault: the link uses the full path.
        let other = root.join("别处");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("c.txt"), b"x").unwrap();
        let src2 = src_dir.join("c.txt");
        fs::write(&src2, b"different").unwrap();
        let outcomes = run_job(&job(root.clone(), vec![Source::File(src2)], &["c.txt"]));
        match &outcomes[0] {
            Outcome::Saved { dest_rel, link } => {
                assert_eq!(dest_rel, "附件/c.txt");
                assert_eq!(link, "![[附件/c.txt]]");
            }
            other => panic!("unexpected outcome {other:?}"),
        }
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&src_dir);
    }

    #[test]
    fn missing_or_empty_sources_fail_without_writing() {
        let root = temp_root("fail");
        let outcomes = run_job(&job(
            root.clone(),
            vec![
                Source::File(root.join("nope.png")),
                Source::Clipboard {
                    bytes: Vec::new(),
                    ext: None,
                },
            ],
            &[],
        ));
        assert!(matches!(outcomes[0], Outcome::Failed(_)));
        assert_eq!(outcomes[1], Outcome::Failed("文件是空的".to_string()));
        assert!(!root.join("附件").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
