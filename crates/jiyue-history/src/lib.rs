//! Local note version history ("file recovery" / time machine).
//!
//! Everything lives under one directory (`<vault>/.jiyue/history/`):
//!
//! ```text
//! LOCK              advisory lock; one `History` per directory
//! index.jsonl       append-only log of records (put / rename / label / remove / seq)
//! blobs/<aa>/<hash> content-addressed, blake3 + lz4 (pure Rust), deduplicated
//! ```
//!
//! The library has no UI and no network access. All paths are vault-relative
//! with forward slashes. Time is always passed in (`now_ms`), so the caller
//! controls the clock.

mod blobs;
mod diff;
mod error;
mod history;
mod index;
mod model;
mod retention;

pub use diff::{DiffLine, DiffTag, diff_text};
pub use error::{HistoryError, Result};
pub use history::{History, system_now_ms};
pub use model::{
    DeletedNote, Entry, EntryId, GcReport, Kind, MAX_FILE_BYTES, SnapshotMeta, SnapshotOutcome,
    Stats, VerifyReport,
};
pub use retention::{
    DAY_MS, DEFAULT_MIN_INTERVAL_MS, HOUR_MS, Retention, WEEK_MS, should_snapshot,
    should_snapshot_kind,
};
