//! Pure logic of the version history: throttling, labels, times and diff counts. No I/O.

use jiyue_history::{DEFAULT_MIN_INTERVAL_MS, DiffLine, DiffTag, Kind, should_snapshot};

/// Most versions listed for one note (newest first).
pub const LIST_LIMIT: usize = 200;
/// How long an automatic snapshot must wait after the previous one of the same note. `None`
/// when one is due now; otherwise the milliseconds left.
pub fn throttle_remaining(latest_ms: Option<i64>, now_ms: i64) -> Option<i64> {
    if should_snapshot(latest_ms, now_ms, DEFAULT_MIN_INTERVAL_MS) {
        return None;
    }
    // `should_snapshot` is false only when `latest` is set and `now` is not before it.
    let last = latest_ms.unwrap_or(now_ms);
    let due_at = last.saturating_add(DEFAULT_MIN_INTERVAL_MS);
    Some(due_at.saturating_sub(now_ms).max(1))
}

/// Fingerprint of a note's text, to recognise the text the app last saved on disk.
pub fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Label of the reason a version was taken.
pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Auto => "自动",
        Kind::Manual => "手动",
        Kind::BeforeSync => "同步前",
        Kind::BeforeRestore => "恢复前",
        Kind::Deleted => "删除前",
    }
}

/// Byte count for display: `812 B`, `4.2 KB`, `1.5 MB`.
pub fn bytes_text(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let v = n as f64;
    if v < KB {
        format!("{n} B")
    } else if v < MB {
        format!("{:.1} KB", v / KB)
    } else {
        format!("{:.1} MB", v / MB)
    }
}

/// Local wall-clock parts of a Unix time in milliseconds: (year, month, day, hour, minute).
/// Falls back to UTC when the local offset is unknown.
pub fn local_parts(ms: i64) -> Option<(i32, u8, u8, u8, u8)> {
    let nanos = (ms as i128).checked_mul(1_000_000)?;
    let utc = time::OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?;
    let offset = time::UtcOffset::local_offset_at(utc).unwrap_or(time::UtcOffset::UTC);
    let t = utc.to_offset(offset);
    Some((t.year(), t.month() as u8, t.day(), t.hour(), t.minute()))
}

/// Local time as `2026-11-05 14:30`.
pub fn local_time_text(ms: i64) -> String {
    match local_parts(ms) {
        Some((y, mo, d, h, mi)) => format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}"),
        None => "时间未知".to_owned(),
    }
}

/// Lines added and removed going from `old` to `new` (the diff's `Insert` and `Delete` lines).
pub fn line_counts(diff: &[DiffLine]) -> (usize, usize) {
    let added = diff.iter().filter(|l| l.tag == DiffTag::Insert).count();
    let removed = diff.iter().filter(|l| l.tag == DiffTag::Delete).count();
    (added, removed)
}

/// `+3 −1`, the change marks of a version row.
pub fn change_marks(added: usize, removed: usize) -> String {
    format!("+{added} −{removed}")
}

/// Text under the dialog when the history store cannot be used.
pub fn unavailable_text(error: &str, locked: bool) -> String {
    if locked {
        "历史库正被另一个 Jiyue 窗口或服务使用，暂时无法记录和查看历史版本。关闭另一个程序后重新打开笔记库即可。".to_owned()
    } else {
        format!("无法打开历史库，历史版本已停用：{error}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiyue_history::{DAY_MS, diff_text};

    #[test]
    fn throttle_is_due_when_never_snapshotted() {
        assert_eq!(throttle_remaining(None, 1_000), None);
    }

    #[test]
    fn throttle_reports_the_time_left() {
        let last = 10_000_000;
        let left = throttle_remaining(Some(last), last + 30_000);
        assert_eq!(left, Some(DEFAULT_MIN_INTERVAL_MS - 30_000));
    }

    #[test]
    fn throttle_is_due_after_the_interval() {
        let last = 10_000_000;
        assert_eq!(
            throttle_remaining(Some(last), last + DEFAULT_MIN_INTERVAL_MS),
            None
        );
    }

    #[test]
    fn throttle_treats_a_backwards_clock_as_due() {
        assert_eq!(throttle_remaining(Some(5_000_000), 1_000), None);
    }

    #[test]
    fn text_fingerprint_follows_the_text() {
        assert_eq!(text_hash("abc"), text_hash("abc"));
        assert_ne!(text_hash("abc"), text_hash("abd"));
    }

    #[test]
    fn kind_labels_are_chinese() {
        assert_eq!(kind_label(Kind::Auto), "自动");
        assert_eq!(kind_label(Kind::BeforeRestore), "恢复前");
        assert_eq!(kind_label(Kind::Deleted), "删除前");
    }

    #[test]
    fn byte_counts_pick_a_unit() {
        assert_eq!(bytes_text(0), "0 B");
        assert_eq!(bytes_text(1023), "1023 B");
        assert_eq!(bytes_text(1024), "1.0 KB");
        assert_eq!(bytes_text(1536), "1.5 KB");
        assert_eq!(bytes_text(3 * 1024 * 1024 / 2), "1.5 MB");
    }

    #[test]
    fn local_time_is_formatted() {
        // Whatever the local offset is, the shape must hold.
        let text = local_time_text(DAY_MS * 20_000);
        assert_eq!(text.len(), 16, "{text}");
        assert_eq!(&text[4..5], "-");
        assert_eq!(&text[13..14], ":");
    }

    #[test]
    fn local_parts_of_epoch_is_1970() {
        let (y, _, _, _, _) = local_parts(0).expect("epoch converts");
        assert!(y == 1969 || y == 1970, "year {y}");
    }

    #[test]
    fn counts_come_from_insert_and_delete_lines() {
        let diff = diff_text("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(line_counts(&diff), (2, 1));
        assert_eq!(change_marks(2, 1), "+2 −1");
    }

    #[test]
    fn identical_text_has_no_changes() {
        let diff = diff_text("x\n", "x\n");
        assert_eq!(line_counts(&diff), (0, 0));
    }

    #[test]
    fn unavailable_text_names_the_lock() {
        assert!(unavailable_text("", true).contains("另一个 Jiyue"));
        assert!(unavailable_text("disk full", false).contains("disk full"));
    }
}
