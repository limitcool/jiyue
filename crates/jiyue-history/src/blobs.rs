//! Content-addressed blob files: `blobs/<aa>/<hash>`.
//!
//! Each file is `lz4_flex::compress_prepend_size(raw)` (pure Rust). The name
//! is the blake3 hash of the uncompressed content, so identical content is
//! stored once across all notes.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::model::MAX_FILE_BYTES;

/// Lowercase hex blake3 of the uncompressed bytes.
pub fn hash_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// True for exactly 64 lowercase hex characters.
pub fn is_valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Location of a blob. Only call with a hash that passed `is_valid_hash`.
pub fn blob_path(root: &Path, hash: &str) -> PathBuf {
    root.join(&hash[..2]).join(hash)
}

pub fn encode(raw: &[u8]) -> Vec<u8> {
    lz4_flex::compress_prepend_size(raw)
}

fn decode(stored: &[u8]) -> Result<Vec<u8>, String> {
    if stored.len() < 4 {
        return Err("truncated header".to_string());
    }
    let declared = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]) as usize;
    if declared > MAX_FILE_BYTES {
        return Err(format!("declared size {declared} exceeds the limit"));
    }
    lz4_flex::decompress_size_prepended(stored).map_err(|e| e.to_string())
}

#[derive(Debug)]
pub enum BlobError {
    Missing,
    Corrupt(String),
    Io(io::Error),
}

/// Read and fully verify a blob (decode and hash check).
pub fn read_blob(root: &Path, hash: &str) -> Result<Vec<u8>, BlobError> {
    let stored = match fs::read(blob_path(root, hash)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(BlobError::Missing),
        Err(e) => return Err(BlobError::Io(e)),
    };
    let raw = decode(&stored).map_err(BlobError::Corrupt)?;
    if hash_hex(&raw) != hash {
        return Err(BlobError::Corrupt("hash mismatch".to_string()));
    }
    Ok(raw)
}

/// Write `data` to `path` atomically: temp file, rename. With `sync`, the file
/// and the directory are fsynced too. `tag` makes the temp name unique within
/// one process.
pub fn write_atomic(path: &Path, data: &[u8], tag: u64, sync: bool) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("blob");
    let tmp = parent.join(format!("{name}.{tag}.tmp"));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(data)?;
        if sync {
            file.sync_all()?;
        }
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if sync {
        sync_dir(parent);
    }
    Ok(())
}

/// Create a file that must not exist yet and write `data` into it in one go.
/// Used for fresh blobs: the index record that references a blob is appended
/// only after this returns, so a torn file is never referenced. A torn file
/// left by a crash is an unreferenced orphan that the next GC removes.
pub fn write_new(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(data)
}

/// Flush one existing file to disk. Opens it for writing, because FlushFileBuffers
/// on Windows needs write access.
pub fn sync_file(path: &Path) -> io::Result<()> {
    fs::OpenOptions::new().write(true).open(path)?.sync_all()
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "jiyue-history-unit-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&p).expect("create unit temp dir");
        p
    }

    #[test]
    fn hash_is_stable_and_valid() {
        let h = hash_hex(b"hello");
        assert_eq!(h.len(), 64);
        assert!(is_valid_hash(&h));
        assert_eq!(h, hash_hex(b"hello"));
        assert_ne!(h, hash_hex(b"hello!"));
    }

    #[test]
    fn hash_of_empty_input_is_valid() {
        assert!(is_valid_hash(&hash_hex(b"")));
    }

    #[test]
    fn valid_hash_rejects_bad_shapes() {
        assert!(!is_valid_hash(""));
        assert!(!is_valid_hash("abc"));
        assert!(!is_valid_hash(&"A".repeat(64)));
        assert!(!is_valid_hash(&"g".repeat(64)));
        assert!(!is_valid_hash(&format!("{}/", "a".repeat(63))));
        assert!(!is_valid_hash(&"../".repeat(21)));
    }

    #[test]
    fn encode_decode_round_trip() {
        let raw = b"line one\nline two\n".repeat(200);
        let stored = encode(&raw);
        assert!(stored.len() < raw.len(), "repetitive text must compress");
        assert_eq!(decode(&stored).expect("decode"), raw);
    }

    #[test]
    fn decode_rejects_truncated_header() {
        assert!(decode(&[1, 2]).is_err());
    }

    #[test]
    fn decode_rejects_huge_declared_size() {
        let mut bytes = (u32::MAX).to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0u8; 8]);
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_garbage_payload() {
        let mut bytes = 100u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0xff; 32]);
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_of_empty_content_round_trips() {
        assert_eq!(decode(&encode(b"")).expect("decode"), Vec::<u8>::new());
    }

    #[test]
    fn read_blob_reports_missing() {
        let root = temp_root("missing");
        let h = hash_hex(b"x");
        assert!(matches!(read_blob(&root, &h), Err(BlobError::Missing)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_blob_detects_hash_mismatch() {
        let root = temp_root("mismatch");
        let h = hash_hex(b"expected");
        write_atomic(&blob_path(&root, &h), &encode(b"other"), 1, true).expect("write");
        assert!(matches!(read_blob(&root, &h), Err(BlobError::Corrupt(_))));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_blob_detects_undecodable_file() {
        let root = temp_root("undecodable");
        let h = hash_hex(b"data");
        write_atomic(&blob_path(&root, &h), b"\x05\x00\x00\x00nonsense", 1, true).expect("write");
        assert!(matches!(read_blob(&root, &h), Err(BlobError::Corrupt(_))));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_atomic_replaces_existing_file() {
        let root = temp_root("replace");
        let path = root.join("f.txt");
        write_atomic(&path, b"one", 1, false).expect("first");
        write_atomic(&path, b"two", 2, true).expect("second");
        assert_eq!(fs::read(&path).expect("read"), b"two");
        let leftovers = fs::read_dir(&root)
            .expect("list")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn blob_path_uses_two_char_fanout() {
        let h = hash_hex(b"fanout");
        let p = blob_path(Path::new("root"), &h);
        assert_eq!(
            p.parent()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str()),
            Some(&h[..2])
        );
    }
}
