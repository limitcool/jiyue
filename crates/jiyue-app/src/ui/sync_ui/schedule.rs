//! Pure scheduling and status text: when an automatic run is due, how long to back off after a
//! network failure, how progress and the last sync time read in Chinese. No clocks are read here;
//! callers pass durations and timestamps in.

use std::time::Duration;

use jiyue_sync::SyncReport;

use super::config::Interval;

/// Longest pause after repeated network failures.
pub const BACKOFF_MAX_SECS: u64 = 900;
/// Pause after the first failure.
pub const BACKOFF_FIRST_SECS: u64 = 30;

/// Seconds to wait before the next automatic run after `failures` failures in a row.
/// 0 failures: no wait. Then 30, 60, 120, 240, 480 and 900 seconds at most.
pub fn backoff_secs(failures: u32) -> u64 {
    if failures == 0 {
        return 0;
    }
    let shift = (failures - 1).min(10);
    (BACKOFF_FIRST_SECS << shift).min(BACKOFF_MAX_SECS)
}

/// True when an automatic run is due: the interval has passed since the last run ended.
pub fn auto_due(since_last_end: Duration, interval: Interval) -> bool {
    since_last_end >= Duration::from_secs(interval.secs())
}

/// True when a save-triggered run is due: the debounce window has passed since the last save.
pub fn save_due(since_last_save: Duration, debounce_secs: u64) -> bool {
    since_last_save >= Duration::from_secs(debounce_secs)
}

/// Progress text for the status bar while a run is going.
pub fn progress_text(done: usize, total: usize) -> String {
    if total == 0 {
        "同步中…".to_owned()
    } else {
        format!("同步中 {done}/{total}")
    }
}

/// Action names reported by `run_sync`, in Chinese.
pub fn action_label(action: &str) -> &'static str {
    match action {
        "scan" => "扫描本地文件",
        "upload" => "上传",
        "download" => "下载",
        "merge" => "合并",
        "conflict" => "冲突",
        "delete-remote" => "删除远端",
        "delete-local" => "删除本地",
        _ => "处理中",
    }
}

/// Relative time for "last synced" texts, e.g. `刚刚`, `3 分钟前`, `2 小时前`, `5 天前`.
/// `now_ms` and `then_ms` are Unix milliseconds. A time in the future counts as now.
pub fn relative_zh(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..=59 => "刚刚".to_owned(),
        60..=3599 => format!("{} 分钟前", secs / 60),
        3600..=86_399 => format!("{} 小时前", secs / 3600),
        86_400..=2_591_999 => format!("{} 天前", secs / 86_400),
        _ => "一个月以上前".to_owned(),
    }
}

/// Byte count with a unit, e.g. `512 B`, `3.4 KB`, `12.0 MB`.
pub fn bytes_text(n: u64) -> String {
    const KB: f64 = 1024.0;
    let v = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if v < KB * KB {
        format!("{:.1} KB", v / KB)
    } else {
        format!("{:.1} MB", v / (KB * KB))
    }
}

/// One-line summary of a finished run, for toasts and the modal header.
pub fn report_summary(report: &SyncReport) -> String {
    format!(
        "上传 {}，下载 {}，合并 {}，冲突 {}，删除 {}，错误 {}",
        report.uploaded.len(),
        report.downloaded.len(),
        report.merged.len(),
        report.conflicts.len(),
        report.deleted_local.len() + report.deleted_remote.len(),
        report.errors.len()
    )
}

/// Paths the run changed on disk: downloads, merges, conflict copies and local deletions.
/// Used for `ExtEvent::VaultFilesChanged`.
pub fn changed_paths(report: &SyncReport) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    out.extend(report.downloaded.iter().cloned());
    out.extend(report.merged.iter().cloned());
    out.extend(
        report
            .conflicts
            .iter()
            .flat_map(|c| [c.path.clone(), c.copy_path.clone()]),
    );
    out.extend(report.deleted_local.iter().cloned());
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiyue_sync::{ConflictKind, ConflictRecord};

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff_secs(0), 0);
        assert_eq!(backoff_secs(1), 30);
        assert_eq!(backoff_secs(2), 60);
        assert_eq!(backoff_secs(3), 120);
        assert_eq!(backoff_secs(5), 480);
        assert_eq!(backoff_secs(6), 900);
        assert_eq!(backoff_secs(40), 900);
    }

    #[test]
    fn auto_run_waits_for_the_whole_interval() {
        assert!(!auto_due(Duration::from_secs(299), Interval::M5));
        assert!(auto_due(Duration::from_secs(300), Interval::M5));
        assert!(auto_due(Duration::from_secs(30), Interval::S30));
    }

    #[test]
    fn save_debounce_resets_by_caller() {
        assert!(!save_due(Duration::from_secs(9), 10));
        assert!(save_due(Duration::from_secs(10), 10));
    }

    #[test]
    fn progress_text_is_chinese() {
        assert_eq!(progress_text(12, 40), "同步中 12/40");
        assert_eq!(progress_text(0, 0), "同步中…");
    }

    #[test]
    fn relative_time_in_chinese() {
        let now = 10_000_000_000;
        assert_eq!(relative_zh(now, now - 5_000), "刚刚");
        assert_eq!(relative_zh(now, now - 3 * 60_000), "3 分钟前");
        assert_eq!(relative_zh(now, now - 2 * 3_600_000), "2 小时前");
        assert_eq!(relative_zh(now, now - 5 * 86_400_000), "5 天前");
        assert_eq!(relative_zh(now, now - 90 * 86_400_000), "一个月以上前");
        assert_eq!(relative_zh(now, now + 1000), "刚刚");
    }

    #[test]
    fn bytes_units() {
        assert_eq!(bytes_text(512), "512 B");
        assert_eq!(bytes_text(2048), "2.0 KB");
        assert_eq!(bytes_text(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn summary_and_changed_paths() {
        let report = SyncReport {
            uploaded: vec!["a.md".into()],
            downloaded: vec!["b.md".into(), "a.md".into()],
            conflicts: vec![ConflictRecord {
                path: "c.md".into(),
                copy_path: "c (冲突副本 PC 20261105-1200).md".into(),
                kind: ConflictKind::Markers,
            }],
            deleted_local: vec!["d.md".into()],
            ..SyncReport::default()
        };
        assert_eq!(
            report_summary(&report),
            "上传 1，下载 2，合并 0，冲突 1，删除 1，错误 0"
        );
        let changed = changed_paths(&report);
        assert!(changed.contains(&"b.md".to_owned()));
        assert!(changed.contains(&"d.md".to_owned()));
        assert_eq!(changed.iter().filter(|p| p.as_str() == "a.md").count(), 1);
    }
}
