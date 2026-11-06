//! Daily notes: date arithmetic, date-name formatting, templates and the
//! `Vault` operations that create or open the note for a given day.
//!
//! All logic takes the date as a parameter; only [`Date::today`] and
//! [`local_hhmm`] read the system clock.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::ops::Bound;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::buffer::Note;
use crate::vault::Vault;

/// Default template for a new daily note (Simplified Chinese).
pub const DEFAULT_TEMPLATE: &str =
    "# {{date:YYYY年M月D日}} {{weekday}}\n\n## 今日待办\n- [ ] \n\n## 记录\n\n## 回顾\n";

const WEEKDAYS_ZH: [&str; 7] = [
    "星期一",
    "星期二",
    "星期三",
    "星期四",
    "星期五",
    "星期六",
    "星期日",
];

// ---------------------------------------------------------------------------
// Calendar date
// ---------------------------------------------------------------------------

/// A proleptic Gregorian calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// Number of days in `month` of `year` (month must be 1..=12).
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Gregorian leap year rule.
pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12; // March = 0
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date for days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl Date {
    /// Validated constructor; `None` for impossible dates such as 2026-02-30.
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// Today's date in local time.
    pub fn today() -> Self {
        local_clock().0
    }

    /// Days since 1970-01-01.
    pub fn to_days(self) -> i64 {
        days_from_civil(self.year as i64, self.month as i64, self.day as i64)
    }

    /// Inverse of [`Date::to_days`].
    pub fn from_days(days: i64) -> Self {
        let (y, m, d) = civil_from_days(days);
        Self {
            year: y as i32,
            month: m as u32,
            day: d as u32,
        }
    }

    /// The date `delta` days after (or before, if negative) this one.
    pub fn add_days(self, delta: i64) -> Self {
        Self::from_days(self.to_days() + delta)
    }

    /// Day of week with Monday = 0 ... Sunday = 6.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday (index 3 with Monday = 0).
        (self.to_days() + 3).rem_euclid(7) as u32
    }
}

/// Simplified Chinese weekday name, e.g. "星期三".
pub fn weekday_name_zh(date: Date) -> &'static str {
    WEEKDAYS_ZH
        .get(date.weekday() as usize)
        .copied()
        .unwrap_or("")
}

// ---------------------------------------------------------------------------
// Local clock
// ---------------------------------------------------------------------------

/// Local (date, hour, minute). Windows uses GetLocalTime; other platforms fall
/// back to UTC derived from SystemTime.
#[cfg(windows)]
fn local_clock() -> (Date, u32, u32) {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct WinSystemTime {
        wYear: u16,
        wMonth: u16,
        wDayOfWeek: u16,
        wDay: u16,
        wHour: u16,
        wMinute: u16,
        wSecond: u16,
        wMilliseconds: u16,
    }

    unsafe extern "system" {
        fn GetLocalTime(lp_system_time: *mut WinSystemTime);
    }

    let mut st = WinSystemTime {
        wYear: 1970,
        wMonth: 1,
        wDayOfWeek: 0,
        wDay: 1,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    // SAFETY: GetLocalTime writes one SYSTEMTIME (layout matches the repr(C)
    // struct above: eight u16 fields) through the pointer. `st` is a live,
    // exclusively borrowed local for the duration of the call.
    unsafe {
        GetLocalTime(&mut st as *mut WinSystemTime);
    }

    let date = Date::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32)
        .unwrap_or_else(|| Date::from_days(0));
    (date, st.wHour as u32, st.wMinute as u32)
}

#[cfg(not(windows))]
fn local_clock() -> (Date, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    (
        Date::from_days(days),
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
    )
}

/// Current local time as "HH:MM".
pub fn local_hhmm() -> String {
    let (_, h, m) = local_clock();
    format!("{:02}:{:02}", h, m)
}

// ---------------------------------------------------------------------------
// Date format tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Year4,
    Year2,
    Month2,
    Month1,
    Day2,
    Day1,
}

impl Tok {
    /// Accepted digit counts when parsing.
    fn digit_range(self) -> (usize, usize) {
        match self {
            Tok::Year4 => (4, 4),
            Tok::Year2 => (2, 2),
            Tok::Month2 => (2, 2),
            Tok::Month1 => (1, 2),
            Tok::Day2 => (2, 2),
            Tok::Day1 => (1, 2),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Lit(String),
    Tok(Tok),
}

/// Split a format string into literal text and tokens. Longest token wins
/// (YYYY before YY, MM before M, DD before D); text inside [brackets] is literal.
fn tokenize(fmt: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut rest = fmt;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[')
            && let Some(end) = after.find(']')
        {
            lit.push_str(&after[..end]);
            rest = &after[end + 1..];
            continue;
        }
        let token = if rest.starts_with("YYYY") {
            Some((Tok::Year4, 4))
        } else if rest.starts_with("YY") {
            Some((Tok::Year2, 2))
        } else if rest.starts_with("MM") {
            Some((Tok::Month2, 2))
        } else if rest.starts_with('M') {
            Some((Tok::Month1, 1))
        } else if rest.starts_with("DD") {
            Some((Tok::Day2, 2))
        } else if rest.starts_with('D') {
            Some((Tok::Day1, 1))
        } else {
            None
        };
        match token {
            Some((tok, len)) => {
                if !lit.is_empty() {
                    out.push(Piece::Lit(std::mem::take(&mut lit)));
                }
                out.push(Piece::Tok(tok));
                rest = &rest[len..];
            }
            None => {
                let ch = rest.chars().next().unwrap_or('\0');
                lit.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    if !lit.is_empty() {
        out.push(Piece::Lit(lit));
    }
    out
}

/// Format a date with a daily-note format string such as "YYYY-MM-DD" or
/// "YYYY年M月D日". Use [brackets] for literal text, e.g. "[Day] D".
pub fn format_date(date: Date, fmt: &str) -> String {
    let mut out = String::new();
    for piece in tokenize(fmt) {
        match piece {
            Piece::Lit(s) => out.push_str(&s),
            Piece::Tok(tok) => match tok {
                Tok::Year4 => out.push_str(&format!("{:04}", date.year)),
                Tok::Year2 => out.push_str(&format!("{:02}", date.year.rem_euclid(100))),
                Tok::Month2 => out.push_str(&format!("{:02}", date.month)),
                Tok::Month1 => out.push_str(&date.month.to_string()),
                Tok::Day2 => out.push_str(&format!("{:02}", date.day)),
                Tok::Day1 => out.push_str(&date.day.to_string()),
            },
        }
    }
    out
}

/// Recursively match `pieces` against `input`, collecting numeric token values.
/// Variable-width tokens (M, D) backtrack over their allowed digit counts.
fn match_pieces(pieces: &[Piece], input: &str, found: &mut Vec<(Tok, u32)>) -> bool {
    let Some((first, rest_pieces)) = pieces.split_first() else {
        return input.is_empty();
    };
    match first {
        Piece::Lit(s) => match input.strip_prefix(s.as_str()) {
            Some(remaining) => match_pieces(rest_pieces, remaining, found),
            None => false,
        },
        Piece::Tok(tok) => {
            let (min, max) = tok.digit_range();
            let leading_digits = input.bytes().take_while(|b| b.is_ascii_digit()).count();
            let upper = leading_digits.min(max);
            for n in (min..=upper).rev() {
                // Digits are ASCII, so splitting at byte `n` is on a char boundary.
                let (num, remaining) = input.split_at(n);
                let Ok(value) = num.parse::<u32>() else {
                    continue;
                };
                found.push((*tok, value));
                if match_pieces(rest_pieces, remaining, found) {
                    return true;
                }
                found.pop();
            }
            false
        }
    }
}

/// Strict inverse of [`format_date`]: the whole input must match the format and
/// the result must be a valid calendar date. Two-digit years map to 2000-2099.
pub fn parse_date(s: &str, fmt: &str) -> Option<Date> {
    let pieces = tokenize(fmt);
    let mut found = Vec::new();
    if !match_pieces(&pieces, s, &mut found) {
        return None;
    }
    let mut year: Option<i32> = None;
    let mut month: Option<u32> = None;
    let mut day: Option<u32> = None;
    for (tok, value) in found {
        match tok {
            Tok::Year4 | Tok::Year2 => {
                let y = if tok == Tok::Year2 {
                    2000 + value as i32
                } else {
                    value as i32
                };
                if year.is_some_and(|prev| prev != y) {
                    return None;
                }
                year = Some(y);
            }
            Tok::Month2 | Tok::Month1 => {
                if month.is_some_and(|prev| prev != value) {
                    return None;
                }
                month = Some(value);
            }
            Tok::Day2 | Tok::Day1 => {
                if day.is_some_and(|prev| prev != value) {
                    return None;
                }
                day = Some(value);
            }
        }
    }
    Date::new(year?, month?, day?)
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

/// Render a template. Placeholders:
/// `{{date}}` (formatted with `date_format`), `{{date:FORMAT}}`, `{{title}}`,
/// `{{weekday}}`, `{{time}}` (the supplied `hhmm`). Unknown placeholders are
/// left unchanged.
pub fn render_template(
    tpl: &str,
    date: Date,
    title: &str,
    date_format: &str,
    hhmm: &str,
) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = after[..end].trim();
        let replacement = match inner {
            "date" => Some(format_date(date, date_format)),
            "title" => Some(title.to_string()),
            "weekday" => Some(weekday_name_zh(date).to_string()),
            "time" => Some(hhmm.to_string()),
            _ => inner.strip_prefix("date:").map(|f| format_date(date, f)),
        };
        match replacement {
            Some(text) => out.push_str(&text),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Daily-note settings: the `[daily]` section of `<vault>/.jiyue/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DailySettings {
    /// Folder relative to the vault root.
    pub folder: String,
    /// Format used for the note title and file name.
    pub date_format: String,
    /// Template file relative to the vault root. Empty = the built-in template.
    pub template_path: String,
}

impl Default for DailySettings {
    fn default() -> Self {
        Self {
            folder: "日记".to_string(),
            date_format: "YYYY-MM-DD".to_string(),
            template_path: String::new(),
        }
    }
}

impl DailySettings {
    /// Path of the vault's configuration file.
    pub fn settings_path(vault_root: &Path) -> PathBuf {
        crate::config::vault_file(vault_root, crate::config::VAULT_CONFIG_FILE)
    }

    /// Load the `[daily]` section; a missing, broken or invalid file yields the defaults (the
    /// configuration store reports the problem).
    pub fn load(vault_root: &Path) -> Self {
        crate::config::load_section::<Self>(&Self::settings_path(vault_root), "daily").value
    }

    /// Save the `[daily]` section, keeping the rest of the file. Refused while the file is broken.
    pub fn save(&self, vault_root: &Path) -> io::Result<()> {
        crate::config::save_section(
            &Self::settings_path(vault_root),
            "daily",
            self,
            crate::config::templates::VAULT_CONFIG,
            &[],
        )
        .map_err(io::Error::other)
    }
}

// ---------------------------------------------------------------------------
// Vault operations
// ---------------------------------------------------------------------------

fn norm_path(path: &str) -> String {
    path.replace('\\', "/")
}

impl Vault {
    /// Settings for this vault.
    pub fn daily_settings(&self) -> DailySettings {
        DailySettings::load(&self.root_dir)
    }

    /// Path-relative-to-vault of the daily note for `date`.
    fn daily_rel_path(settings: &DailySettings, date: Date) -> PathBuf {
        let title = format_date(date, &settings.date_format);
        PathBuf::from(&settings.folder).join(format!("{}.md", title))
    }

    /// Absolute path of the daily note for `date`.
    pub fn daily_path(&self, date: Date) -> PathBuf {
        let settings = self.daily_settings();
        self.root_dir.join(Self::daily_rel_path(&settings, date))
    }

    /// The date `delta_days` away from `date`.
    pub fn daily_neighbor(&self, date: Date, delta_days: i64) -> Date {
        date.add_days(delta_days)
    }

    /// Read the configured template, falling back to [`DEFAULT_TEMPLATE`].
    fn daily_template(&self, settings: &DailySettings) -> String {
        let path = settings.template_path.trim().to_string();
        Some(path)
            .filter(|p| !p.is_empty())
            .and_then(|p| fs::read_to_string(self.root_dir.join(p)).ok())
            .unwrap_or_else(|| DEFAULT_TEMPLATE.to_string())
    }

    /// Open the daily note for `date`, creating it from the template if it does
    /// not exist. An existing file is never overwritten. Returns the note index,
    /// which becomes the active note in a (new or existing) tab.
    pub fn open_or_create_daily(&mut self, date: Date) -> io::Result<usize> {
        let settings = self.daily_settings();
        let title = format_date(date, &settings.date_format);
        let rel = Self::daily_rel_path(&settings, date);
        let rel_str = rel.to_string_lossy().into_owned();
        let abs = self.root_dir.join(&rel);
        let wanted = norm_path(&rel_str);

        let idx = match self
            .notes
            .iter()
            .position(|n| norm_path(&n.path).eq_ignore_ascii_case(&wanted))
        {
            Some(idx) => idx,
            None => {
                let content = if abs.is_file() {
                    fs::read_to_string(&abs)?
                } else {
                    let template = self.daily_template(&settings);
                    let content = render_template(
                        &template,
                        date,
                        &title,
                        &settings.date_format,
                        &local_hhmm(),
                    );
                    if let Some(parent) = abs.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    // Written whole (temp file, then rename): a crash never leaves a half note.
                    crate::fsops::write_atomic(&abs, content.as_bytes())
                        .map_err(|err| io::Error::other(err.to_string()))?;
                    content
                };
                self.notes.push(Note::new(&title, &rel_str, &content));
                self.notes.len() - 1
            }
        };

        self.rebuild_graph();
        self.active_note_index = idx;
        match self.open_tabs.iter().position(|&t| t == idx) {
            Some(pos) => self.active_tab_index = pos,
            None => {
                self.open_tabs.push(idx);
                self.active_tab_index = self.open_tabs.len() - 1;
            }
        }
        Ok(idx)
    }

    /// All dates that have a note in the daily folder (read from disk).
    fn daily_all_dates(&self, settings: &DailySettings) -> BTreeSet<Date> {
        let mut dates = BTreeSet::new();
        let dir = self.root_dir.join(&settings.folder);
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && let Some(date) = parse_date(stem, &settings.date_format)
                {
                    dates.insert(date);
                }
            }
        }
        dates
    }

    /// Days of the given month that have a daily note.
    pub fn daily_dates_in_month(&self, year: i32, month: u32) -> BTreeSet<u32> {
        let settings = self.daily_settings();
        self.daily_all_dates(&settings)
            .into_iter()
            .filter(|d| d.year == year && d.month == month)
            .map(|d| d.day)
            .collect()
    }

    /// True if `note` lives directly in the daily folder and its title parses as a date.
    pub fn is_daily_note(&self, note: &Note) -> bool {
        let settings = self.daily_settings();
        let norm = norm_path(&note.path);
        let parent = norm.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        if !parent.eq_ignore_ascii_case(settings.folder.trim_matches('/')) {
            return false;
        }
        Path::new(&note.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|stem| parse_date(stem, &settings.date_format))
            .is_some()
    }

    /// Nearest existing daily note strictly before (or after, if `forward`) `date`.
    pub fn daily_adjacent_existing(&self, date: Date, forward: bool) -> Option<Date> {
        let settings = self.daily_settings();
        let all = self.daily_all_dates(&settings);
        if forward {
            all.range((Bound::Excluded(date), Bound::Unbounded))
                .next()
                .copied()
        } else {
            all.range((Bound::Unbounded, Bound::Excluded(date)))
                .next_back()
                .copied()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> Date {
        Date::new(y, m, day).expect("valid test date")
    }

    fn temp_vault(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jiyue-daily-test-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn format_and_parse_round_trip() {
        let formats = [
            "YYYY-MM-DD",
            "YYYY年M月D日",
            "YY/M/D",
            "[Daily] YYYY.MM.DD",
            "YYYY[年]MM[月]DD[日]",
            "DD-MM-YYYY",
        ];
        let dates = [
            d(2026, 11, 4),
            d(2024, 2, 29),
            d(2000, 1, 9),
            d(2026, 12, 31),
        ];
        for fmt in formats {
            for date in dates {
                let text = format_date(date, fmt);
                assert_eq!(
                    parse_date(&text, fmt),
                    Some(date),
                    "format {fmt} text {text}"
                );
            }
        }
    }

    #[test]
    fn format_specific_outputs() {
        let date = d(2026, 11, 4);
        assert_eq!(format_date(date, "YYYY-MM-DD"), "2026-11-04");
        assert_eq!(format_date(date, "YYYY年M月D日"), "2026年11月4日");
        assert_eq!(format_date(date, "YY/M/D"), "26/11/4");
        assert_eq!(format_date(date, "[Day] D"), "Day 4");
        assert_eq!(format_date(date, "[YYYY] YYYY"), "YYYY 2026");
    }

    #[test]
    fn parse_is_strict() {
        assert_eq!(parse_date("2026-11-04x", "YYYY-MM-DD"), None);
        assert_eq!(parse_date("2026-11", "YYYY-MM-DD"), None);
        assert_eq!(parse_date("2026-02-30", "YYYY-MM-DD"), None);
        assert_eq!(parse_date("2026-13-01", "YYYY-MM-DD"), None);
        assert_eq!(parse_date("2026-1-04", "YYYY-MM-DD"), None);
        assert_eq!(parse_date("日记", "YYYY-MM-DD"), None);
    }

    #[test]
    fn parse_variable_width_tokens() {
        assert_eq!(parse_date("2026-1-4", "YYYY-M-D"), Some(d(2026, 1, 4)));
        assert_eq!(parse_date("2026-11-14", "YYYY-M-D"), Some(d(2026, 11, 14)));
        // Greedy first, then backtracks: "11" + "4" wins over "1" + "14".
        assert_eq!(parse_date("2026114", "YYYYMD"), Some(d(2026, 11, 4)));
    }

    #[test]
    fn leap_years() {
        assert!(Date::new(2024, 2, 29).is_some());
        assert!(Date::new(2000, 2, 29).is_some());
        assert!(Date::new(2100, 2, 29).is_none());
        assert!(Date::new(2023, 2, 29).is_none());
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(2100));
    }

    #[test]
    fn add_days_crosses_boundaries() {
        assert_eq!(d(2026, 12, 31).add_days(1), d(2027, 1, 1));
        assert_eq!(d(2026, 1, 1).add_days(-1), d(2025, 12, 31));
        assert_eq!(d(2026, 3, 1).add_days(-1), d(2026, 2, 28));
        assert_eq!(d(2024, 2, 28).add_days(1), d(2024, 2, 29));
        assert_eq!(d(2026, 1, 31).add_days(29), d(2026, 3, 1));
        assert_eq!(d(2026, 11, 4).add_days(365), d(2027, 11, 4));
        assert_eq!(d(2026, 11, 4).add_days(0), d(2026, 11, 4));
    }

    #[test]
    fn civil_days_round_trip() {
        assert_eq!(d(1970, 1, 1).to_days(), 0);
        for days in -800_000..800_000i64 {
            if days % 997 == 0 {
                assert_eq!(Date::from_days(days).to_days(), days);
            }
        }
    }

    #[test]
    fn ordering() {
        assert!(d(2026, 11, 4) < d(2026, 11, 5));
        assert!(d(2026, 11, 30) < d(2026, 12, 1));
        assert!(d(2025, 12, 31) < d(2026, 1, 1));
    }

    #[test]
    fn weekdays() {
        assert_eq!(d(2026, 11, 4).weekday(), 2);
        assert_eq!(weekday_name_zh(d(2026, 11, 4)), "星期三");
        assert_eq!(weekday_name_zh(d(1970, 1, 1)), "星期四");
        assert_eq!(weekday_name_zh(d(2026, 11, 8)), "星期日");
        assert_eq!(weekday_name_zh(d(2026, 11, 2)), "星期一");
        assert_eq!(weekday_name_zh(d(2024, 2, 29)), "星期四");
    }

    #[test]
    fn default_template_renders() {
        let rendered = render_template(
            DEFAULT_TEMPLATE,
            d(2026, 11, 4),
            "2026-11-04",
            "YYYY-MM-DD",
            "08:30",
        );
        assert_eq!(
            rendered,
            "# 2026年11月4日 星期三\n\n## 今日待办\n- [ ] \n\n## 记录\n\n## 回顾\n"
        );
    }

    #[test]
    fn template_placeholders() {
        let tpl = "{{title}}|{{date}}|{{date:YY/MM/DD}}|{{weekday}}|{{time}}|{{unknown}}|{{date:[Day] D}}";
        let out = render_template(tpl, d(2026, 11, 4), "T", "YYYY-MM-DD", "21:05");
        assert_eq!(out, "T|2026-11-04|26/11/04|星期三|21:05|{{unknown}}|Day 4");
        let unclosed = render_template("a {{date", d(2026, 11, 4), "T", "YYYY-MM-DD", "00:00");
        assert_eq!(unclosed, "a {{date");
    }

    #[test]
    fn settings_defaults_and_round_trip() {
        let dir = temp_vault("settings");
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(DailySettings::load(&dir), DailySettings::default());

        fs::create_dir_all(dir.join(".jiyue")).unwrap();
        fs::write(
            dir.join(".jiyue").join("config.toml"),
            "version = 1\n[daily\nfolder = ",
        )
        .unwrap();
        assert_eq!(DailySettings::load(&dir), DailySettings::default());

        let custom = DailySettings {
            folder: "Journal".to_string(),
            date_format: "YYYY/MM/DD".to_string(),
            template_path: "tpl/daily.md".to_string(),
        };
        // The broken file is never overwritten: the save is refused and the defaults stay in use.
        assert!(custom.save(&dir).is_err());
        assert!(
            fs::read_to_string(dir.join(".jiyue").join("config.toml"))
                .unwrap()
                .contains("[daily")
        );

        fs::write(
            dir.join(".jiyue").join("config.toml"),
            "version = 1
",
        )
        .unwrap();
        custom.save(&dir).unwrap();
        assert_eq!(DailySettings::load(&dir), custom);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_or_create_is_idempotent_and_keeps_edits() {
        let dir = temp_vault("create");
        let mut vault = Vault::open_or_create(&dir);
        let date = d(2026, 11, 4);
        let path = vault.daily_path(date);
        assert!(path.ends_with(PathBuf::from("日记").join("2026-11-04.md")));
        assert!(!path.exists());

        let idx = vault.open_or_create_daily(date).unwrap();
        assert!(path.is_file());
        let on_disk = fs::read_to_string(&path).unwrap();
        assert!(on_disk.starts_with("# 2026年11月4日 星期三"));
        assert_eq!(vault.notes[idx].title, "2026-11-04");
        assert_eq!(vault.active_note_index, idx);
        assert!(vault.is_daily_note(&vault.notes[idx]));

        let tabs_before = vault.open_tabs.len();
        let notes_before = vault.notes.len();

        // Edit the note and save it, then reopen: content must be kept.
        vault.notes[idx].set_content("# mine\n\nedited\n");
        vault.save_note_to_disk(idx).unwrap();
        let idx2 = vault.open_or_create_daily(date).unwrap();
        assert_eq!(idx2, idx);
        assert_eq!(vault.notes.len(), notes_before);
        assert_eq!(vault.open_tabs.len(), tabs_before);
        assert_eq!(fs::read_to_string(&path).unwrap(), "# mine\n\nedited\n");

        // A file that exists on disk but is not in the list is loaded, not overwritten.
        fs::write(vault.daily_path(d(2026, 11, 5)), "from disk").unwrap();
        let idx3 = vault.open_or_create_daily(d(2026, 11, 5)).unwrap();
        assert_eq!(vault.notes[idx3].content(), "from disk");
        assert_eq!(
            fs::read_to_string(vault.daily_path(d(2026, 11, 5))).unwrap(),
            "from disk"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dates_in_month_and_adjacent_existing() {
        let dir = temp_vault("navigation");
        let mut vault = Vault::open_or_create(&dir);
        for date in [
            d(2026, 11, 2),
            d(2026, 11, 4),
            d(2026, 11, 10),
            d(2026, 12, 1),
        ] {
            vault.open_or_create_daily(date).unwrap();
        }

        assert_eq!(
            vault.daily_dates_in_month(2026, 11),
            BTreeSet::from([2, 4, 10])
        );
        assert_eq!(vault.daily_dates_in_month(2026, 12), BTreeSet::from([1]));
        assert!(vault.daily_dates_in_month(2026, 10).is_empty());

        assert_eq!(
            vault.daily_adjacent_existing(d(2026, 11, 4), true),
            Some(d(2026, 11, 10))
        );
        assert_eq!(
            vault.daily_adjacent_existing(d(2026, 11, 4), false),
            Some(d(2026, 11, 2))
        );
        assert_eq!(
            vault.daily_adjacent_existing(d(2026, 11, 3), true),
            Some(d(2026, 11, 4))
        );
        assert_eq!(
            vault.daily_adjacent_existing(d(2026, 11, 10), true),
            Some(d(2026, 12, 1))
        );
        assert_eq!(vault.daily_adjacent_existing(d(2026, 12, 1), true), None);
        assert_eq!(vault.daily_adjacent_existing(d(2026, 11, 2), false), None);

        assert_eq!(vault.daily_neighbor(d(2026, 11, 30), 1), d(2026, 12, 1));

        let sample = vault
            .notes
            .iter()
            .find(|n| n.title == "欢迎使用 Jiyue")
            .expect("sample note exists");
        assert!(!vault.is_daily_note(sample));

        let _ = fs::remove_dir_all(&dir);
    }
}
