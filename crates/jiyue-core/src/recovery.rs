//! Crash recovery journal. While a note has unsaved edits, a copy of its text is kept under
//! `<vault>/.jiyue/recovery/`: `<key>.md` holds the text, `<key>.json` the note path, the disk
//! modification time when the unsaved edit began, and the time of the copy. A successful save
//! (or a close) removes the pair. On start-up the leftovers are judged against the disk file (see
//! [`judge`]).
//!
//! Only the pure parts (key, verdict, serialisation) carry logic worth testing here; the file
//! helpers are thin wrappers over [`crate::fsops::write_atomic`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsops::{self, FsError};

/// Folder under `<vault>/.jiyue` that holds the journal.
const RECOVERY_DIR: &str = "recovery";
const TEXT_SUFFIX: &str = "md";
const META_SUFFIX: &str = "json";

/// One recovery copy: the note it belongs to, and the unsaved text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    /// Vault-relative path of the note, with forward slashes.
    pub path: String,
    /// Modification time (ms since the Unix epoch) of the disk file when the unsaved edit began.
    /// `None` when the file did not exist then.
    pub base_mtime_ms: Option<i64>,
    /// When this copy was taken (ms since the Unix epoch).
    pub saved_at_ms: i64,
    /// The unsaved text. Stored in the `.md` file, not in the json.
    #[serde(skip)]
    pub text: String,
}

/// What a leftover journal means, judged against the disk file at start-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The disk file already holds this text: the save finished but the journal was not removed.
    /// Nothing is lost; the journal can go.
    Stale,
    /// The note's file no longer exists. The journal holds the only copy of the text.
    DiskMissing,
    /// The disk file holds different text, so the journal has unsaved work to recover.
    /// `disk_changed` is true when the file was modified after the unsaved edit began.
    Differs { disk_changed: bool },
}

/// Judges a journal against the disk file. `disk` is the file's text (`None` when missing), the
/// two times are modification times in ms since the Unix epoch.
pub fn judge(
    journal_text: &str,
    disk: Option<&str>,
    disk_mtime_ms: Option<i64>,
    base_mtime_ms: Option<i64>,
) -> Verdict {
    match disk {
        None => Verdict::DiskMissing,
        Some(text) if text == journal_text => Verdict::Stale,
        Some(_) => Verdict::Differs {
            disk_changed: match (base_mtime_ms, disk_mtime_ms) {
                (Some(base), Some(now)) => now > base,
                _ => false,
            },
        },
    }
}

/// Stable file-name key of a note path (FNV-1a, 64 bit, as 16 hex digits). Backslashes count as
/// slashes, so the same note always gets the same key.
pub fn key_for(note_path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in note_path.replace('\\', "/").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The journal folder of a vault.
pub fn dir_of(root: &Path) -> PathBuf {
    root.join(".jiyue").join(RECOVERY_DIR)
}

fn text_path(root: &Path, note_path: &str) -> PathBuf {
    dir_of(root).join(format!("{}.{TEXT_SUFFIX}", key_for(note_path)))
}

fn meta_path(root: &Path, note_path: &str) -> PathBuf {
    dir_of(root).join(format!("{}.{META_SUFFIX}", key_for(note_path)))
}

/// Writes the journal pair of `journal.path`, text first and then the json. Both writes are
/// atomic, so a crash leaves either the old pair or the new one.
pub fn write(root: &Path, journal: &Journal) -> Result<(), FsError> {
    let meta = serde_json::to_vec_pretty(journal).map_err(|err| FsError::Io(err.to_string()))?;
    fsops::write_atomic(&text_path(root, &journal.path), journal.text.as_bytes())?;
    fsops::write_atomic(&meta_path(root, &journal.path), &meta)
}

/// Removes the journal pair of a note. A missing pair is not an error.
pub fn remove(root: &Path, note_path: &str) -> io::Result<()> {
    for path in [text_path(root, note_path), meta_path(root, note_path)] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Reads every complete journal pair. Unreadable or half-written entries are skipped (their
/// text file is then harmless leftovers). Sorted by note path.
pub fn load_all(root: &Path) -> Vec<Journal> {
    let Ok(entries) = fs::read_dir(dir_of(root)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(META_SUFFIX) {
            continue;
        }
        let Some(journal) = read_pair(&path) else {
            continue;
        };
        out.push(journal);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn read_pair(meta_file: &Path) -> Option<Journal> {
    let meta = fs::read(meta_file).ok()?;
    let mut journal: Journal = serde_json::from_slice(&meta).ok()?;
    let text_file = meta_file.with_extension(TEXT_SUFFIX);
    journal.text = fs::read_to_string(text_file).ok()?;
    // The key is derived from the path, so a mismatch means the pair was damaged.
    if meta_file.file_stem().and_then(|s| s.to_str()) != Some(key_for(&journal.path).as_str()) {
        return None;
    }
    Some(journal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A fresh vault folder under the system temp directory, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "jiyue-recovery-{name}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("scratch dir");
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn journal(path: &str, text: &str) -> Journal {
        Journal {
            path: path.to_string(),
            base_mtime_ms: Some(1_000),
            saved_at_ms: 2_000,
            text: text.to_string(),
        }
    }

    #[test]
    fn key_is_stable_and_separator_blind() {
        assert_eq!(key_for("想法/沉积.md"), key_for("想法\\沉积.md"));
        assert_eq!(key_for("a.md"), key_for("a.md"));
        assert_ne!(key_for("a.md"), key_for("b.md"));
        assert_eq!(key_for("a.md").len(), 16);
    }

    #[test]
    fn judge_missing_file_is_disk_missing() {
        assert_eq!(judge("text", None, None, None), Verdict::DiskMissing);
    }

    #[test]
    fn judge_equal_text_is_stale() {
        assert_eq!(
            judge("same", Some("same"), Some(5), Some(1)),
            Verdict::Stale
        );
    }

    #[test]
    fn judge_different_text_without_newer_disk_is_not_changed() {
        assert_eq!(
            judge("mine", Some("theirs"), Some(900), Some(1_000)),
            Verdict::Differs {
                disk_changed: false
            }
        );
        // No base time recorded: the change cannot be dated, so it is not reported as one.
        assert_eq!(
            judge("mine", Some("theirs"), Some(9_000), None),
            Verdict::Differs {
                disk_changed: false
            }
        );
    }

    #[test]
    fn judge_disk_modified_after_edit_began_is_changed() {
        assert_eq!(
            judge("mine", Some("theirs"), Some(1_500), Some(1_000)),
            Verdict::Differs { disk_changed: true }
        );
    }

    #[test]
    fn write_then_load_round_trips_text_and_meta() {
        let dir = Scratch::new("round-trip");
        let saved = journal("想法/沉积.md", "未保存的正文\n第二行");
        write(&dir.0, &saved).expect("write");
        let loaded = load_all(&dir.0);
        assert_eq!(loaded, vec![saved]);
    }

    #[test]
    fn write_replaces_previous_copy_of_same_note() {
        let dir = Scratch::new("replace");
        write(&dir.0, &journal("a.md", "first")).expect("first");
        write(&dir.0, &journal("a.md", "second")).expect("second");
        let loaded = load_all(&dir.0);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text, "second");
    }

    #[test]
    fn remove_deletes_the_pair_and_tolerates_missing() {
        let dir = Scratch::new("remove");
        write(&dir.0, &journal("a.md", "x")).expect("write");
        remove(&dir.0, "a.md").expect("remove");
        assert!(load_all(&dir.0).is_empty());
        remove(&dir.0, "a.md").expect("second remove is fine");
    }

    #[test]
    fn load_skips_damaged_entries_and_keeps_good_ones() {
        let dir = Scratch::new("damaged");
        write(&dir.0, &journal("good.md", "ok")).expect("good");
        write(&dir.0, &journal("bad.md", "broken")).expect("bad");
        fs::write(meta_path(&dir.0, "bad.md"), b"{ not json").expect("damage");
        let paths: Vec<String> = load_all(&dir.0).into_iter().map(|j| j.path).collect();
        assert_eq!(paths, vec!["good.md".to_string()]);
    }

    #[test]
    fn load_skips_pair_whose_text_file_is_missing() {
        let dir = Scratch::new("no-text");
        write(&dir.0, &journal("a.md", "x")).expect("write");
        fs::remove_file(text_path(&dir.0, "a.md")).expect("drop text");
        assert!(load_all(&dir.0).is_empty());
    }

    #[test]
    fn load_on_missing_folder_is_empty() {
        let dir = Scratch::new("empty");
        assert!(load_all(&dir.0).is_empty());
    }
}
