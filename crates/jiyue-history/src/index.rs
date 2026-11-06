//! The append-only index log (`index.jsonl`): one JSON record per line.
//!
//! Records are the only source of truth for metadata. A truncated or garbage
//! line is skipped on load and counted; a line that is not newline-terminated
//! gets a newline before the next append so no good record is glued onto
//! garbage.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::blobs;
use crate::model::Kind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Record {
    /// Next entry id to hand out (written by compaction).
    Seq {
        next: u64,
    },
    Put {
        id: u64,
        path: String,
        time_ms: i64,
        size: u64,
        hash: String,
        kind: Kind,
        label: Option<String>,
    },
    Rename {
        old: String,
        new: String,
    },
    Label {
        id: u64,
        label: Option<String>,
    },
    Remove {
        ids: Vec<u64>,
    },
}

pub struct Loaded {
    pub records: Vec<Record>,
    pub bad_lines: usize,
    pub needs_newline: bool,
}

/// Read every parsable record. A missing file is an empty log.
pub fn load(path: &Path) -> io::Result<Loaded> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Loaded {
                records: Vec::new(),
                bad_lines: 0,
                needs_newline: false,
            });
        }
        Err(e) => return Err(e),
    };
    let mut records = Vec::new();
    let mut bad_lines = 0;
    for line in bytes.split(|b| *b == b'\n') {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        match serde_json::from_slice::<Record>(line) {
            Ok(r) => records.push(r),
            Err(_) => bad_lines += 1,
        }
    }
    let needs_newline = bytes.last().is_some_and(|b| *b != b'\n');
    Ok(Loaded {
        records,
        bad_lines,
        needs_newline,
    })
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

pub struct IndexLog {
    path: PathBuf,
    file: Option<File>,
    /// Number of records in the file (valid ones only, as loaded plus appended).
    pub records: usize,
    /// Size of the file in bytes.
    pub bytes: u64,
}

impl IndexLog {
    pub fn open(path: &Path) -> io::Result<IndexLog> {
        let file = open_append(path)?;
        let bytes = file.metadata()?.len();
        Ok(IndexLog {
            path: path.to_path_buf(),
            file: Some(file),
            records: 0,
            bytes,
        })
    }

    /// fsync the log file (group commit point; see `History::flush`).
    pub fn sync(&mut self) -> io::Result<()> {
        match self.file.as_ref() {
            Some(f) => f.sync_data(),
            None => Ok(()),
        }
    }

    pub fn append(&mut self, rec: &Record) -> io::Result<()> {
        let mut line = serde_json::to_vec(rec).map_err(io::Error::other)?;
        line.push(b'\n');
        self.append_raw(&line)?;
        self.records += 1;
        Ok(())
    }

    pub fn append_raw(&mut self, data: &[u8]) -> io::Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("index log is closed"))?;
        file.write_all(data)?;
        self.bytes += data.len() as u64;
        Ok(())
    }

    /// Replace the whole log with `records` (compaction). Atomic: temp file,
    /// fsync, rename. The old handle is closed first (required on Windows).
    pub fn rewrite(&mut self, records: &[Record], tag: u64) -> io::Result<()> {
        let mut buf = Vec::new();
        for r in records {
            serde_json::to_writer(&mut buf, r).map_err(io::Error::other)?;
            buf.push(b'\n');
        }
        self.file = None;
        let res = blobs::write_atomic(&self.path, &buf, tag, true);
        self.file = Some(open_append(&self.path)?);
        res?;
        self.records = records.len();
        self.bytes = buf.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_records() -> Vec<Record> {
        vec![
            Record::Seq { next: 9 },
            Record::Put {
                id: 1,
                path: "a.md".into(),
                time_ms: 10,
                size: 3,
                hash: blobs::hash_hex(b"abc"),
                kind: Kind::Auto,
                label: None,
            },
            Record::Put {
                id: 2,
                path: "dir/b.md".into(),
                time_ms: 11,
                size: 0,
                hash: blobs::hash_hex(b""),
                kind: Kind::Manual,
                label: Some("v1".into()),
            },
            Record::Rename {
                old: "a.md".into(),
                new: "c.md".into(),
            },
            Record::Label { id: 1, label: None },
            Record::Remove { ids: vec![1, 2] },
        ]
    }

    #[test]
    fn every_record_variant_round_trips_through_json() {
        for rec in sample_records() {
            let text = serde_json::to_string(&rec).expect("serialize");
            let back: Record = serde_json::from_str(&text).expect("deserialize");
            assert_eq!(back, rec);
        }
    }

    #[test]
    fn records_carry_an_op_tag() {
        let text = serde_json::to_string(&Record::Seq { next: 1 }).expect("serialize");
        assert!(text.contains("\"op\":\"seq\""));
    }

    #[test]
    fn unknown_op_is_rejected() {
        assert!(serde_json::from_str::<Record>(r#"{"op":"explode"}"#).is_err());
    }

    #[test]
    fn load_missing_file_is_empty() {
        let p = std::env::temp_dir().join(format!("jiyue-index-missing-{}", std::process::id()));
        let _ = fs::remove_file(&p);
        let loaded = load(&p).expect("load");
        assert!(loaded.records.is_empty());
        assert_eq!(loaded.bad_lines, 0);
        assert!(!loaded.needs_newline);
    }

    #[test]
    fn load_skips_bad_lines_and_flags_missing_newline() {
        let p = std::env::temp_dir().join(format!("jiyue-index-bad-{}", std::process::id()));
        let good = serde_json::to_string(&Record::Seq { next: 4 }).expect("ser");
        let content = format!("{good}\n\nnot json\n{{\"op\":\"seq\",\"ne");
        fs::write(&p, content).expect("write");
        let loaded = load(&p).expect("load");
        assert_eq!(loaded.records, vec![Record::Seq { next: 4 }]);
        assert_eq!(loaded.bad_lines, 2);
        assert!(loaded.needs_newline);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn append_then_load_sees_the_record() {
        let p = std::env::temp_dir().join(format!("jiyue-index-append-{}", std::process::id()));
        let _ = fs::remove_file(&p);
        let mut log = IndexLog::open(&p).expect("open");
        log.append(&Record::Seq { next: 7 }).expect("append");
        assert_eq!(log.records, 1);
        drop(log);
        let loaded = load(&p).expect("load");
        assert_eq!(loaded.records, vec![Record::Seq { next: 7 }]);
        let _ = fs::remove_file(&p);
    }
}
