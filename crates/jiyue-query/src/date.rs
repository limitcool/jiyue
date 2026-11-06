//! Calendar helpers. Times are naive wall-clock values: no time zone database and no leap
//! seconds. Days are counted from 1970-01-01; date-times are milliseconds from the same epoch.

use serde::Serialize;

pub const DAY_MS: i64 = 86_400_000;

/// Days from 1970-01-01 to the given civil date. `m` is 1..=12 and `d` is 1..=31.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil (year, month, day) of a day number.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m_raw = if mp < 10 { mp + 3 } else { mp - 9 };
    let m = m_raw as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Number of days in a month; 0 for an invalid month.
pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

pub fn valid_ymd(y: i64, m: u32, d: u32) -> bool {
    (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)
}

/// Days to milliseconds without overflow.
pub fn days_to_ms(days: i64) -> i64 {
    days.saturating_mul(DAY_MS)
}

/// Adds calendar months to a day number. The day of month is clamped to the length of the
/// target month (Jan 31 + 1 month = Feb 28 or 29).
pub fn add_months(days: i64, months: i64) -> i64 {
    let (y, m, d) = civil_from_days(days);
    let total = y
        .saturating_mul(12)
        .saturating_add(i64::from(m) - 1)
        .saturating_add(months);
    let ny = total.div_euclid(12);
    if ny.abs() > 5_000_000 {
        return days;
    }
    let nm = (total.rem_euclid(12) + 1) as u32;
    let nd = d.min(days_in_month(ny, nm));
    days_from_civil(ny, nm, nd)
}

fn digits(b: &[u8]) -> Option<u32> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut v: u32 = 0;
    for &c in b {
        v = v.checked_mul(10)?.checked_add(u32::from(c - b'0'))?;
    }
    Some(v)
}

fn two(s: &str) -> Option<u32> {
    if s.len() == 2 {
        digits(s.as_bytes())
    } else {
        None
    }
}

/// Parses `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS][Z]` (a space may replace `T`).
/// Returns the epoch milliseconds and whether a time part was present.
pub fn parse_datetime_parts(input: &str) -> Option<(i64, bool)> {
    let s = input.trim();
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y = i64::from(digits(&b[0..4])?);
    let mo = digits(&b[5..7])?;
    let d = digits(&b[8..10])?;
    if !valid_ymd(y, mo, d) {
        return None;
    }
    let day_ms = days_to_ms(days_from_civil(y, mo, d));
    if b.len() == 10 {
        return Some((day_ms, false));
    }
    if b[10] != b'T' && b[10] != b' ' {
        return None;
    }
    let mut rest = s.get(11..)?;
    if let Some(r) = rest.strip_suffix('Z') {
        rest = r;
    }
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let hh = two(parts[0])?;
    let mm = two(parts[1])?;
    let ss = match parts.get(2) {
        Some(p) => two(p)?,
        None => 0,
    };
    if hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    let tod = i64::from(hh) * 3_600_000 + i64::from(mm) * 60_000 + i64::from(ss) * 1000;
    Some((day_ms.saturating_add(tod), true))
}

/// Strict `YYYY-MM-DD`.
pub fn parse_date(input: &str) -> Option<i64> {
    match parse_datetime_parts(input) {
        Some((ms, false)) => Some(ms.div_euclid(DAY_MS)),
        _ => None,
    }
}

/// `YYYY-MM-DD` or a date-time, as epoch milliseconds (dates are midnight).
pub fn parse_datetime(input: &str) -> Option<i64> {
    parse_datetime_parts(input).map(|(ms, _)| ms)
}

/// `YYYY-MM-DD`.
pub fn format_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD HH:MM`, with `:SS` when the seconds are not zero.
pub fn format_datetime(ms: i64) -> String {
    let days = ms.div_euclid(DAY_MS);
    let tod = ms.rem_euclid(DAY_MS);
    let (hh, mm, ss) = (tod / 3_600_000, (tod / 60_000) % 60, (tod / 1000) % 60);
    let date = format_date(days);
    if ss == 0 {
        format!("{date} {hh:02}:{mm:02}")
    } else {
        format!("{date} {hh:02}:{mm:02}:{ss:02}")
    }
}

/// `YYYY-MM-DDTHH:MM:SS`, the form written to frontmatter.
pub fn format_iso(ms: i64) -> String {
    let days = ms.div_euclid(DAY_MS);
    let tod = ms.rem_euclid(DAY_MS);
    let (hh, mm, ss) = (tod / 3_600_000, (tod / 60_000) % 60, (tod / 1000) % 60);
    format!("{}T{hh:02}:{mm:02}:{ss:02}", format_date(days))
}

/// A duration literal such as `7d`. Calendar units (`mo`, `y`) are kept apart because their
/// length depends on the start date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Duration {
    pub months: i64,
    pub ms: i64,
}

impl Duration {
    pub fn display(&self) -> String {
        if self.months != 0 {
            if self.months % 12 == 0 {
                format!("{}y", self.months / 12)
            } else {
                format!("{}mo", self.months)
            }
        } else if self.ms % DAY_MS == 0 {
            format!("{}d", self.ms / DAY_MS)
        } else if self.ms % 3_600_000 == 0 {
            format!("{}h", self.ms / 3_600_000)
        } else if self.ms % 60_000 == 0 {
            format!("{}min", self.ms / 60_000)
        } else {
            format!("{}s", self.ms / 1000)
        }
    }
}

/// Builds a duration from a number and a unit: `s`, `min`, `h`, `d`, `w`, `mo`, `y`.
pub fn duration_from_literal(n: f64, unit: &str) -> Result<Duration, String> {
    if !n.is_finite() || n < 0.0 {
        return Err(format!("invalid duration amount {n}"));
    }
    let fixed = |per_unit_ms: f64| -> Result<Duration, String> {
        let v = (n * per_unit_ms).round();
        if v > 9.0e15 {
            return Err(format!("duration {n}{unit} is too large"));
        }
        Ok(Duration {
            months: 0,
            ms: v as i64,
        })
    };
    let calendar = |per_unit_months: f64| -> Result<Duration, String> {
        if n.fract() != 0.0 {
            return Err(format!(
                "`{n}{unit}` must be a whole number of months or years"
            ));
        }
        let v = n * per_unit_months;
        if v > 1.0e6 {
            return Err(format!("duration {n}{unit} is too large"));
        }
        Ok(Duration {
            months: v as i64,
            ms: 0,
        })
    };
    match unit {
        "s" => fixed(1000.0),
        "min" => fixed(60_000.0),
        "h" => fixed(3_600_000.0),
        "d" => fixed(DAY_MS as f64),
        "w" => fixed(7.0 * DAY_MS as f64),
        "mo" => calendar(1.0),
        "y" => calendar(12.0),
        "m" => Err("`m` is ambiguous: use `min` for minutes or `mo` for months".to_string()),
        other => Err(format!("unknown duration unit `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip_over_wide_range() {
        for days in (-200_000..200_000).step_by(37) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "day {days}");
        }
    }

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(format_date(days_from_civil(2026, 11, 4)), "2026-11-04");
        assert_eq!(
            days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28),
            2
        );
    }

    #[test]
    fn leap_years() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2100));
        assert!(is_leap_year(2000));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 13), 0);
    }

    #[test]
    fn add_months_clamps_to_month_end() {
        let jan31 = days_from_civil(2026, 1, 31);
        assert_eq!(format_date(add_months(jan31, 1)), "2026-02-28");
        let jan31_leap = days_from_civil(2024, 1, 31);
        assert_eq!(format_date(add_months(jan31_leap, 1)), "2024-02-29");
        let mar31 = days_from_civil(2026, 3, 31);
        assert_eq!(format_date(add_months(mar31, -1)), "2026-02-28");
        assert_eq!(format_date(add_months(jan31, 12)), "2027-01-31");
        assert_eq!(format_date(add_months(jan31, -13)), "2024-12-31");
    }

    #[test]
    fn add_months_guard_for_huge_values() {
        let d = days_from_civil(2026, 1, 1);
        assert_eq!(add_months(d, i64::MAX), d);
        assert_eq!(add_months(d, i64::MIN), d);
    }

    #[test]
    fn parse_date_is_strict() {
        assert!(parse_date("2026-03-01").is_some());
        assert_eq!(parse_date("2026-02-29"), None);
        assert!(parse_date("2024-02-29").is_some());
        assert_eq!(parse_date("2026-3-01"), None);
        assert_eq!(parse_date("2026-03-01T10:00"), None);
        assert_eq!(parse_date("hello"), None);
        assert_eq!(parse_date(""), None);
    }

    #[test]
    fn parse_datetime_accepts_common_forms() {
        let (ms, has_time) = parse_datetime_parts("2026-03-01T10:30").unwrap_or((0, true));
        assert!(has_time);
        assert_eq!(format_datetime(ms), "2026-03-01 10:30");
        assert!(parse_datetime("2026-03-01 08:00:15").is_some());
        assert!(parse_datetime("2026-03-01T08:00:15Z").is_some());
        assert_eq!(parse_datetime("2026-03-01T24:00"), None);
        assert_eq!(parse_datetime("2026-03-01T10"), None);
        assert_eq!(parse_datetime("2026-03-01X10:00"), None);
    }

    #[test]
    fn format_iso_round_trips() {
        let ms = parse_datetime("2026-07-09 23:59:58").unwrap_or(0);
        assert_eq!(format_iso(ms), "2026-07-09T23:59:58");
        assert_eq!(format_datetime(ms), "2026-07-09 23:59:58");
    }

    #[test]
    fn durations_units() {
        assert_eq!(
            duration_from_literal(7.0, "d").map(|d| d.ms),
            Ok(7 * DAY_MS)
        );
        assert_eq!(
            duration_from_literal(2.0, "w").map(|d| d.ms),
            Ok(14 * DAY_MS)
        );
        assert_eq!(
            duration_from_literal(3.0, "h").map(|d| d.ms),
            Ok(3 * 3_600_000)
        );
        assert_eq!(
            duration_from_literal(30.0, "min").map(|d| d.ms),
            Ok(1_800_000)
        );
        assert_eq!(duration_from_literal(2.0, "y").map(|d| d.months), Ok(24));
        assert!(duration_from_literal(1.5, "mo").is_err());
        assert!(duration_from_literal(1.0, "m").is_err());
        assert!(duration_from_literal(1.0, "days").is_err());
        assert!(duration_from_literal(1e300, "d").is_err());
        assert!(duration_from_literal(f64::NAN, "d").is_err());
    }

    #[test]
    fn duration_display() {
        let d = duration_from_literal(2.0, "w").unwrap_or(Duration { months: 0, ms: 0 });
        assert_eq!(d.display(), "14d");
        let m = duration_from_literal(1.0, "mo").unwrap_or(Duration { months: 0, ms: 0 });
        assert_eq!(m.display(), "1mo");
    }

    #[test]
    fn days_to_ms_saturates() {
        assert_eq!(days_to_ms(i64::MAX), i64::MAX);
    }
}
