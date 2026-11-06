//! Public data types.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Files larger than this are never snapshotted.
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;

/// Identifier of one history entry. Ids are never reused while the store
/// exists (the next id survives compaction and removals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntryId(pub u64);

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for EntryId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.trim().parse::<u64>().map(EntryId)
    }
}

/// Why a snapshot was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Auto,
    Manual,
    BeforeSync,
    BeforeRestore,
    Deleted,
}

/// Metadata supplied with a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    pub kind: Kind,
    pub label: Option<String>,
}

impl SnapshotMeta {
    pub fn auto() -> Self {
        Self {
            kind: Kind::Auto,
            label: None,
        }
    }

    pub fn manual(label: Option<String>) -> Self {
        Self {
            kind: Kind::Manual,
            label,
        }
    }

    pub fn before_sync() -> Self {
        Self {
            kind: Kind::BeforeSync,
            label: None,
        }
    }

    pub fn before_restore() -> Self {
        Self {
            kind: Kind::BeforeRestore,
            label: None,
        }
    }

    pub fn deleted() -> Self {
        Self {
            kind: Kind::Deleted,
            label: None,
        }
    }
}

/// One stored version of one note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: EntryId,
    pub path: String,
    pub time_ms: i64,
    /// Uncompressed size in bytes.
    pub size: u64,
    /// Lowercase hex blake3 of the uncompressed content.
    pub hash: String,
    pub kind: Kind,
    pub label: Option<String>,
}

/// Result of `History::snapshot_outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotOutcome {
    Created(EntryId),
    /// Identical to the latest snapshot of the note; nothing was written.
    Unchanged,
    /// Larger than `MAX_FILE_BYTES`; ignored.
    TooLarge {
        size: u64,
    },
}

/// A note whose most recent entry is a deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedNote {
    pub path: String,
    /// The `Deleted` entry (its content is the last known bytes).
    pub id: EntryId,
    pub time_ms: i64,
    pub size: u64,
}

/// What a retention pass removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcReport {
    pub removed_entries: usize,
    pub kept_entries: usize,
    pub removed_blobs: usize,
    /// Bytes reclaimed from blob files (compressed size on disk).
    pub freed_bytes: u64,
}

/// Result of `History::verify`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub entries_checked: usize,
    /// Entries whose blob file does not exist.
    pub missing_blobs: Vec<EntryId>,
    /// Entries whose blob cannot be decoded or does not match its hash.
    pub corrupt_blobs: Vec<EntryId>,
    /// Blob files that no entry references (harmless, removed by the next GC).
    pub orphan_blobs: usize,
    /// Files in the blob tree whose names are not hashes (e.g. leftover temp files).
    pub stray_files: usize,
    /// Index lines that could not be parsed (e.g. a truncated tail after a crash).
    pub bad_index_lines: usize,
}

impl VerifyReport {
    /// True when no entry is lost or damaged and the index is clean.
    pub fn is_ok(&self) -> bool {
        self.missing_blobs.is_empty() && self.corrupt_blobs.is_empty() && self.bad_index_lines == 0
    }
}

/// Aggregate numbers about the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub entries: usize,
    pub notes: usize,
    pub unique_blobs: usize,
    /// Sum of uncompressed sizes of unique blobs.
    pub logical_bytes: u64,
    /// Sum of compressed blob file sizes.
    pub bytes_on_disk: u64,
    pub index_bytes: u64,
}
