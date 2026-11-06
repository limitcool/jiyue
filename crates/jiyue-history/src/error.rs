use std::fmt;

use crate::model::EntryId;

#[derive(Debug)]
pub enum HistoryError {
    Io(std::io::Error),
    /// Another `History` already holds the lock on this directory.
    Locked,
    NotFound(EntryId),
    /// The entry's blob file is missing.
    BlobMissing {
        hash: String,
    },
    /// The blob exists but cannot be decoded or does not match its hash.
    Corrupt {
        hash: String,
        reason: String,
    },
    /// The content is larger than `MAX_FILE_BYTES`.
    TooLarge {
        size: u64,
    },
    /// The content is not valid UTF-8 (diff only works on text).
    NotText,
    InvalidPath(String),
}

pub type Result<T> = std::result::Result<T, HistoryError>;

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::Io(e) => write!(f, "history i/o error: {e}"),
            HistoryError::Locked => write!(f, "history directory is locked by another process"),
            HistoryError::NotFound(id) => write!(f, "history entry {id} not found"),
            HistoryError::BlobMissing { hash } => write!(f, "blob {hash} is missing"),
            HistoryError::Corrupt { hash, reason } => write!(f, "blob {hash} is corrupt: {reason}"),
            HistoryError::TooLarge { size } => write!(f, "content of {size} bytes is too large"),
            HistoryError::NotText => write!(f, "content is not valid UTF-8 text"),
            HistoryError::InvalidPath(p) => write!(f, "invalid note path: {p:?}"),
        }
    }
}

impl std::error::Error for HistoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HistoryError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for HistoryError {
    fn from(e: std::io::Error) -> Self {
        HistoryError::Io(e)
    }
}
