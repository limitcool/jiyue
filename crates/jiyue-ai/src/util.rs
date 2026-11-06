//! Small helpers shared across modules: atomic file writes, a stable hash, and secret redaction.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Writes `bytes` to `path` through a temporary sibling file and a rename, so readers never see a
/// half-written file. Creates parent directories.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "file".into());
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    // On Windows, rename replaces an existing destination (MoveFileEx with REPLACE_EXISTING).
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// FNV-1a 64-bit hash. Stable across runs and Rust versions, unlike `DefaultHasher`, so it is safe
/// to persist in the index file.
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Replaces every occurrence of `secret` in `text` with a fixed mask. Empty secrets are ignored.
pub(crate) fn redact(text: &str, secret: Option<&str>) -> String {
    match secret {
        Some(s) if !s.is_empty() => text.replace(s, "[redacted]"),
        _ => text.to_string(),
    }
}

/// Truncates to at most `max` characters, appending an ellipsis when cut.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_known_vector() {
        // Reference value for FNV-1a 64 of "a".
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn redact_masks_secret_and_ignores_empty() {
        assert_eq!(
            redact("key sk-123 bad", Some("sk-123")),
            "key [redacted] bad"
        );
        assert_eq!(redact("nothing", Some("")), "nothing");
        assert_eq!(redact("nothing", None), "nothing");
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        assert_eq!(truncate_chars("你好世界", 2), "你好…");
        assert_eq!(truncate_chars("你好", 5), "你好");
    }

    #[test]
    fn atomic_write_replaces_file_and_leaves_no_tmp() {
        let dir = std::env::temp_dir().join(format!("jiyue-ai-util-{}", std::process::id()));
        let path = dir.join("x.json");
        atomic_write(&path, b"one").expect("first write");
        atomic_write(&path, b"two").expect("second write");
        assert_eq!(fs::read(&path).expect("read"), b"two");
        assert!(!dir.join("x.json.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
