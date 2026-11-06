#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use jiyue_history::{DAY_MS, HOUR_MS, History};

pub const T0: i64 = 1_800_000_000_000;
pub const MIN_MS: i64 = 60_000;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique temporary directory, removed on drop.
pub struct TempVault {
    root: PathBuf,
}

impl TempVault {
    pub fn new(tag: &str) -> TempVault {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "jiyue-history-it-{tag}-{}-{n}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp vault");
        TempVault { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<vault>/.jiyue/history`
    pub fn history_dir(&self) -> PathBuf {
        self.root.join(".jiyue").join("history")
    }

    pub fn open(&self) -> History {
        History::open(&self.history_dir()).expect("open history")
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Deterministic pseudo-random numbers (64-bit LCG, Knuth constants).
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n.max(1)
    }
}

/// Text with `lines` lines; `tag` makes the content distinct.
pub fn text(tag: &str, lines: usize) -> Vec<u8> {
    let mut s = String::new();
    for i in 0..lines {
        s.push_str(&format!("{tag} line {i}: the quick brown fox\n"));
    }
    s.into_bytes()
}

pub fn days(n: i64) -> i64 {
    n * DAY_MS
}

pub fn hours(n: i64) -> i64 {
    n * HOUR_MS
}
