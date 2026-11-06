//! Note templates.
//!
//! [`render_template`] expands `{{...}}` variables from a [`TemplateCtx`]. The context carries
//! the current time as plain numbers, so rendering is a pure function.
//!
//! Variables:
//! - `{{date}}`, `{{date:FORMAT}}`, `{{date+1d}}`, `{{date-1w:YYYY-MM-DD}}`
//! - `{{time}}`, `{{time:FORMAT}}`, `{{datetime}}` (`2026-11-05 14:30`), `{{weekday}}`
//! - `{{title}}`, `{{path}}`, `{{folder}}`, `{{vault}}`
//! - `{{clipboard}}`, `{{selection}}`, `{{uuid}}` (one new id per occurrence)
//! - `{{cursor}}` is removed from the text and reported as `cursor_offset` (first one wins)
//!
//! Date offsets use `d` (days), `w` (weeks), `M` (months, day clamped) and `y` (years).
//! Format tokens: `YYYY YY MM M DD D HH H mm m ss s ddd dddd`; `[text]` is literal.
//! `\{{` writes `{{` without expanding. Unknown variables are left exactly as written.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use jiyue_core::daily::{Date, days_in_month, weekday_name_zh};
use serde::{Deserialize, Serialize};

/// Default templates folder, relative to the vault root.
pub const DEFAULT_TEMPLATE_FOLDER: &str = "模板";

/// Language used for weekday names and locale-dependent output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    /// Simplified Chinese: 星期三, 周三.
    Zh,
    /// English: Wednesday, Wed.
    En,
}

/// A wall-clock moment as plain numbers. No validation happens at construction time;
/// out-of-range fields are clamped when rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    pub y: i32,
    pub m: u32,
    pub d: u32,
    pub h: u32,
    pub mi: u32,
    pub s: u32,
}

/// Everything a template may reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateCtx {
    pub now: Moment,
    pub title: String,
    /// Vault-relative path of the note being created, with `/` separators.
    pub path: String,
    pub vault: String,
    pub clipboard: String,
    pub selection: String,
    pub locale: Locale,
    /// Seed for `{{uuid}}`. The same seed always yields the same ids.
    pub uuid_seed: u64,
}

impl TemplateCtx {
    /// A context with empty clipboard and selection and seed 0.
    pub fn new(now: Moment, title: &str, path: &str, vault: &str, locale: Locale) -> Self {
        Self {
            now,
            title: title.to_string(),
            path: path.to_string(),
            vault: vault.to_string(),
            clipboard: String::new(),
            selection: String::new(),
            locale,
            uuid_seed: 0,
        }
    }
}

/// Output of [`render_template`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedTemplate {
    pub text: String,
    /// Byte offset in `text` where `{{cursor}}` was.
    pub cursor_offset: Option<usize>,
}

/// Expands the template. See the module docs for the variable list.
pub fn render_template(template: &str, ctx: &TemplateCtx) -> RenderedTemplate {
    let mut eval = Eval { ctx, uuid_count: 0 };
    let mut out = String::with_capacity(template.len() + 32);
    let mut cursor = None;
    let mut rest = template;
    loop {
        let Some(p) = rest.find("{{") else {
            out.push_str(rest);
            break;
        };
        if p > 0 && rest.as_bytes()[p - 1] == b'\\' {
            // Escaped: emit the braces literally and do not expand what follows.
            out.push_str(&rest[..p - 1]);
            out.push_str("{{");
            rest = &rest[p + 2..];
            continue;
        }
        out.push_str(&rest[..p]);
        let after = &rest[p + 2..];
        let Some(e) = after.find("}}") else {
            out.push_str(&rest[p..]);
            break;
        };
        let inner = after[..e].trim();
        rest = &after[e + 2..];
        if inner == "cursor" {
            if cursor.is_none() {
                cursor = Some(out.len());
            }
            continue;
        }
        match eval.var(inner) {
            Some(text) => out.push_str(&text),
            None => {
                out.push_str("{{");
                out.push_str(&after[..e]);
                out.push_str("}}");
            }
        }
    }
    RenderedTemplate {
        text: out,
        cursor_offset: cursor,
    }
}

struct Eval<'a> {
    ctx: &'a TemplateCtx,
    uuid_count: u64,
}

impl Eval<'_> {
    fn var(&mut self, inner: &str) -> Option<String> {
        let ctx = self.ctx;
        match inner {
            "title" => Some(ctx.title.clone()),
            "path" => Some(ctx.path.clone()),
            "folder" => Some(folder_of(&ctx.path).to_string()),
            "vault" => Some(ctx.vault.clone()),
            "clipboard" => Some(ctx.clipboard.clone()),
            "selection" => Some(ctx.selection.clone()),
            "weekday" => Some(weekday_long(date_of(ctx.now), ctx.locale)),
            // `2026-11-05 14:30`: the date and the time together.
            "datetime" => Some(format_fmt(
                "YYYY-MM-DD HH:mm",
                date_of(ctx.now),
                clamp_time(ctx.now),
                ctx.locale,
            )),
            "uuid" => {
                let v = uuid_v4(ctx.uuid_seed, self.uuid_count);
                self.uuid_count += 1;
                Some(v)
            }
            _ => self.datetime(inner),
        }
    }

    /// `date` / `time` with optional arithmetic and format.
    fn datetime(&self, inner: &str) -> Option<String> {
        let (is_date, rest) = if let Some(r) = inner.strip_prefix("date") {
            (true, r)
        } else {
            let r = inner.strip_prefix("time")?;
            (false, r)
        };
        let (offset, rest) = if rest.starts_with('+') || rest.starts_with('-') {
            if !is_date {
                return None;
            }
            parse_offset(rest)?
        } else {
            (None, rest)
        };
        let fmt = if rest.is_empty() {
            if is_date { "YYYY-MM-DD" } else { "HH:mm" }
        } else {
            rest.strip_prefix(':')?
        };
        let now = self.ctx.now;
        let date = match offset {
            Some(off) => apply_offset(date_of(now), off),
            None => date_of(now),
        };
        Some(format_fmt(fmt, date, clamp_time(now), self.ctx.locale))
    }
}

#[derive(Debug, Clone, Copy)]
enum Unit {
    Day,
    Week,
    Month,
    Year,
}

/// Parses a leading `+N<unit>` or `-N<unit>`. Returns the offset and the remaining text.
fn parse_offset(s: &str) -> Option<(Option<(i64, Unit)>, &str)> {
    let sign: i64 = if s.starts_with('-') { -1 } else { 1 };
    let body = &s[1..];
    let digits = body.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 6 {
        return None;
    }
    let n: i64 = body[..digits].parse().ok()?;
    let unit_char = body[digits..].chars().next()?;
    let unit = match unit_char {
        'd' => Unit::Day,
        'w' => Unit::Week,
        'M' => Unit::Month,
        'y' => Unit::Year,
        _ => return None,
    };
    let rest = &body[digits + 1..];
    Some((Some((sign * n, unit)), rest))
}

fn apply_offset(date: Date, (n, unit): (i64, Unit)) -> Date {
    match unit {
        Unit::Day => date.add_days(n),
        Unit::Week => date.add_days(n * 7),
        Unit::Month => add_months(date, n),
        Unit::Year => add_months(date, n * 12),
    }
}

fn add_months(date: Date, months: i64) -> Date {
    let total = date.year as i64 * 12 + (date.month as i64 - 1) + months;
    let year = total.div_euclid(12) as i32;
    let month = (total.rem_euclid(12) + 1) as u32;
    let day = date.day.min(days_in_month(year, month));
    Date::new(year, month, day).unwrap_or(date)
}

/// Clamps the plain numbers into a valid calendar date.
fn date_of(now: Moment) -> Date {
    let month = now.m.clamp(1, 12);
    let day = now.d.clamp(1, days_in_month(now.y, month));
    Date::new(now.y, month, day).unwrap_or(Date {
        year: now.y,
        month,
        day,
    })
}

fn clamp_time(now: Moment) -> (u32, u32, u32) {
    (now.h.min(23), now.mi.min(59), now.s.min(59))
}

fn weekday_long(date: Date, locale: Locale) -> String {
    match locale {
        Locale::Zh => weekday_name_zh(date).to_string(),
        Locale::En => [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ][date.weekday() as usize]
            .to_string(),
    }
}

fn weekday_short(date: Date, locale: Locale) -> String {
    match locale {
        Locale::Zh => {
            let long = weekday_name_zh(date);
            let last = long.chars().last().unwrap_or('一');
            format!("周{last}")
        }
        Locale::En => {
            ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][date.weekday() as usize].to_string()
        }
    }
}

/// Longest tokens first, so `YYYY` wins over `YY` and `Y`-prefixed text, `dddd` over `ddd`.
const TOKENS: [&str; 14] = [
    "YYYY", "dddd", "ddd", "YY", "MM", "DD", "HH", "mm", "ss", "M", "D", "H", "m", "s",
];

fn format_fmt(fmt: &str, date: Date, (h, mi, s): (u32, u32, u32), locale: Locale) -> String {
    let mut out = String::with_capacity(fmt.len() + 8);
    let mut rest = fmt;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[')
            && let Some(end) = after.find(']')
        {
            out.push_str(&after[..end]);
            rest = &after[end + 1..];
            continue;
        }
        let token = TOKENS.iter().find(|t| rest.starts_with(*t));
        match token {
            Some(t) => {
                out.push_str(&token_value(t, date, (h, mi, s), locale));
                rest = &rest[t.len()..];
            }
            None => {
                let ch = rest.chars().next().unwrap_or(' ');
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

fn token_value(t: &str, date: Date, (h, mi, s): (u32, u32, u32), locale: Locale) -> String {
    match t {
        "YYYY" => format!("{:04}", date.year),
        "YY" => format!("{:02}", date.year.rem_euclid(100)),
        "MM" => format!("{:02}", date.month),
        "M" => date.month.to_string(),
        "DD" => format!("{:02}", date.day),
        "D" => date.day.to_string(),
        "HH" => format!("{:02}", h),
        "H" => h.to_string(),
        "mm" => format!("{:02}", mi),
        "m" => mi.to_string(),
        "ss" => format!("{:02}", s),
        "s" => s.to_string(),
        "ddd" => weekday_short(date, locale),
        "dddd" => weekday_long(date, locale),
        _ => String::new(),
    }
}

/// Folder part of a vault-relative path (`a/b/c.md` gives `a/b`, `c.md` gives ``).
pub fn folder_of(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(i) => &path[..i],
        None => "",
    }
}

/// A deterministic UUID-shaped id: the same seed and counter always give the same string.
fn uuid_v4(seed: u64, counter: u64) -> String {
    let mut state = seed ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let a = splitmix64(&mut state);
    let b = splitmix64(&mut state);
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// ---------------------------------------------------------------------------
// Template files
// ---------------------------------------------------------------------------

/// A template file found in the templates folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateInfo {
    /// File name without the `.md` extension.
    pub name: String,
    /// Vault-relative path with `/` separators.
    pub path: String,
}

/// True when `s` is a plain relative path made of normal components only.
fn is_safe_rel(s: &str) -> bool {
    !s.is_empty()
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Lists the `.md` files directly inside `folder` (relative to `root`), sorted by name.
/// An empty `folder` means [`DEFAULT_TEMPLATE_FOLDER`]. A missing folder or an unsafe path
/// gives an empty list. Hidden files and subfolders are skipped.
pub fn list_templates(root: &Path, folder: &str) -> Vec<TemplateInfo> {
    let folder = match folder.trim() {
        "" => DEFAULT_TEMPLATE_FOLDER,
        f => f,
    };
    if !is_safe_rel(folder) {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(root.join(folder)) else {
        return Vec::new();
    };
    let mut out: Vec<TemplateInfo> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if name.starts_with('.') || !entry.file_type().ok()?.is_file() {
                return None;
            }
            let stem = name
                .strip_suffix(".md")
                .or_else(|| name.strip_suffix(".MD"))?;
            Some(TemplateInfo {
                name: stem.to_string(),
                path: format!("{folder}/{name}"),
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Reads a template file given its vault-relative path. `None` when unreadable or unsafe.
pub fn read_template(root: &Path, rel_path: &str) -> Option<String> {
    let norm = rel_path.replace('\\', "/");
    if !is_safe_rel(&norm) || !norm.to_ascii_lowercase().ends_with(".md") {
        return None;
    }
    fs::read_to_string(root.join(norm)).ok()
}

// ---------------------------------------------------------------------------
// Per-folder default templates
// ---------------------------------------------------------------------------

/// Which template new notes use: the `[templates]` section of `<vault>/.jiyue/config.toml`.
///
/// `folder_templates` maps a vault-relative folder (`""` = whole vault) to a template path.
/// The longest matching folder wins, so `projects/x` overrides `projects`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateRules {
    pub templates_folder: String,
    pub folder_templates: BTreeMap<String, String>,
}

impl Default for TemplateRules {
    fn default() -> Self {
        Self {
            templates_folder: DEFAULT_TEMPLATE_FOLDER.to_string(),
            folder_templates: BTreeMap::new(),
        }
    }
}

impl TemplateRules {
    /// Path of the settings file for a vault.
    pub fn settings_path(root: &Path) -> PathBuf {
        jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE)
    }

    /// Loads the rules. A missing, unreadable or malformed file gives the defaults.
    pub fn load(root: &Path) -> Self {
        let parsed: Self =
            jiyue_core::config::load_section(&Self::settings_path(root), "templates").value;
        parsed.sanitized()
    }

    /// Writes the rules atomically (temporary file, then rename).
    pub fn save(&self, root: &Path) -> io::Result<()> {
        jiyue_core::config::save_section(
            &Self::settings_path(root),
            "templates",
            self,
            jiyue_core::config::templates::VAULT_CONFIG,
            &["templates.folder_templates"],
        )
        .map_err(io::Error::other)
    }

    /// Template for notes created in `folder`, if a rule applies.
    pub fn default_template_for(&self, folder: &str) -> Option<&str> {
        let folder = normalize_folder(folder);
        self.folder_templates
            .iter()
            .filter(|(key, _)| {
                key.is_empty()
                    || folder.as_str() == key.as_str()
                    || folder.starts_with(&format!("{key}/"))
            })
            .max_by_key(|(key, _)| key.len())
            .map(|(_, template)| template.as_str())
    }

    /// Sets (or with an empty template, removes) the rule for `folder`.
    pub fn set_default(&mut self, folder: &str, template: &str) {
        let key = normalize_folder(folder);
        let template = template.trim().replace('\\', "/");
        if template.is_empty() {
            self.folder_templates.remove(&key);
        } else {
            self.folder_templates.insert(key, template);
        }
    }

    fn sanitized(mut self) -> Self {
        if !is_safe_rel(&self.templates_folder) {
            self.templates_folder = DEFAULT_TEMPLATE_FOLDER.to_string();
        }
        self.folder_templates = self
            .folder_templates
            .into_iter()
            .map(|(k, v)| (normalize_folder(&k), v))
            .collect();
        self
    }
}

fn normalize_folder(folder: &str) -> String {
    folder
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Moment {
        // 2026-03-04 (Wednesday) 09:05:07
        Moment {
            y: 2026,
            m: 3,
            d: 4,
            h: 9,
            mi: 5,
            s: 7,
        }
    }

    fn ctx() -> TemplateCtx {
        let mut c = TemplateCtx::new(now(), "周报", "工作/周报.md", "demo", Locale::Zh);
        c.clipboard = "剪贴板".into();
        c.selection = "选中文字".into();
        c.uuid_seed = 42;
        c
    }

    fn r(t: &str) -> String {
        render_template(t, &ctx()).text
    }

    fn r_en(t: &str) -> String {
        let mut c = ctx();
        c.locale = Locale::En;
        render_template(t, &c).text
    }

    #[test]
    fn default_date_and_time() {
        assert_eq!(r("{{date}}"), "2026-03-04");
        assert_eq!(r("{{time}}"), "09:05");
    }

    #[test]
    fn date_with_format_tokens() {
        assert_eq!(r("{{date:YYYY年M月D日}}"), "2026年3月4日");
        assert_eq!(r("{{date:YY/MM/DD}}"), "26/03/04");
        assert_eq!(r("{{date:YYYYMMDD}}"), "20260304");
        assert_eq!(r("{{date:D/M}}"), "4/3");
        assert_eq!(r("{{date:[Day] D}}"), "Day 4");
    }

    #[test]
    fn time_with_format_tokens() {
        assert_eq!(r("{{time:HH:mm:ss}}"), "09:05:07");
        assert_eq!(r("{{time:H:m:s}}"), "9:5:7");
        assert_eq!(r("{{time:HH时mm分}}"), "09时05分");
    }

    #[test]
    fn weekday_zh_and_en() {
        assert_eq!(r("{{weekday}}"), "星期三");
        assert_eq!(r_en("{{weekday}}"), "Wednesday");
        assert_eq!(r("{{date:ddd}}"), "周三");
        assert_eq!(r("{{date:dddd}}"), "星期三");
        assert_eq!(r_en("{{date:ddd}}"), "Wed");
        assert_eq!(r_en("{{date:dddd}}"), "Wednesday");
    }

    #[test]
    fn title_path_folder_vault() {
        assert_eq!(r("# {{title}}"), "# 周报");
        assert_eq!(r("{{path}}"), "工作/周报.md");
        assert_eq!(r("{{folder}}"), "工作");
        assert_eq!(r("{{vault}}"), "demo");
    }

    #[test]
    fn folder_of_root_note_is_empty() {
        let mut c = ctx();
        c.path = "note.md".into();
        assert_eq!(render_template("[{{folder}}]", &c).text, "[]");
        assert_eq!(folder_of("a\\b\\c.md"), "a\\b");
    }

    #[test]
    fn clipboard_and_selection() {
        assert_eq!(r("{{clipboard}}|{{selection}}"), "剪贴板|选中文字");
    }

    #[test]
    fn uuid_is_deterministic_and_unique_per_occurrence() {
        let out = r("{{uuid}} {{uuid}}");
        let parts: Vec<&str> = out.split(' ').collect();
        assert_eq!(parts.len(), 2);
        assert_ne!(parts[0], parts[1]);
        assert_eq!(out, r("{{uuid}} {{uuid}}"));
        let id = parts[0];
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        assert!(matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn uuid_changes_with_seed() {
        let mut c = ctx();
        let a = render_template("{{uuid}}", &c).text;
        c.uuid_seed = 43;
        let b = render_template("{{uuid}}", &c).text;
        assert_ne!(a, b);
    }

    #[test]
    fn date_day_arithmetic() {
        assert_eq!(r("{{date+1d}}"), "2026-03-05");
        assert_eq!(r("{{date-1d}}"), "2026-03-03");
        assert_eq!(r("{{date+10d:YYYY-MM-DD}}"), "2026-03-14");
        assert_eq!(r("{{date-5d:YYYY-MM-DD}}"), "2026-02-27");
    }

    #[test]
    fn date_week_month_year_arithmetic() {
        assert_eq!(r("{{date-1w:YYYY-MM-DD}}"), "2026-02-25");
        assert_eq!(r("{{date+2w}}"), "2026-03-18");
        assert_eq!(r("{{date+1M}}"), "2026-04-04");
        assert_eq!(r("{{date-3M}}"), "2025-12-04");
        assert_eq!(r("{{date+1y}}"), "2027-03-04");
    }

    #[test]
    fn month_arithmetic_clamps_day() {
        let mut c = ctx();
        c.now = Moment {
            y: 2026,
            m: 1,
            d: 31,
            h: 0,
            mi: 0,
            s: 0,
        };
        assert_eq!(render_template("{{date+1M}}", &c).text, "2026-02-28");
        c.now.y = 2024;
        assert_eq!(render_template("{{date+1M}}", &c).text, "2024-02-29");
        assert_eq!(render_template("{{date-11M}}", &c).text, "2023-02-28");
    }

    #[test]
    fn arithmetic_across_year_boundary() {
        let mut c = ctx();
        c.now = Moment {
            y: 2026,
            m: 1,
            d: 2,
            h: 0,
            mi: 0,
            s: 0,
        };
        assert_eq!(render_template("{{date-3d}}", &c).text, "2025-12-30");
        assert_eq!(
            render_template("{{date+30d:YYYY-MM-DD}}", &c).text,
            "2026-02-01"
        );
    }

    #[test]
    fn arithmetic_with_weekday() {
        assert_eq!(r("{{date+1d:dddd}}"), "星期四");
        assert_eq!(r_en("{{date+1d:dddd}}"), "Thursday");
    }

    #[test]
    fn bad_arithmetic_is_left_as_written() {
        assert_eq!(r("{{date+1x}}"), "{{date+1x}}");
        assert_eq!(r("{{date+d}}"), "{{date+d}}");
        assert_eq!(r("{{time+1h}}"), "{{time+1h}}");
        assert_eq!(r("{{date+1d/YYYY}}"), "{{date+1d/YYYY}}");
        assert_eq!(r("{{date+99999999d}}"), "{{date+99999999d}}");
    }

    #[test]
    fn cursor_is_removed_and_reported() {
        let out = render_template("标题\n{{cursor}}\n尾", &ctx());
        assert_eq!(out.text, "标题\n\n尾");
        assert_eq!(out.cursor_offset, Some("标题\n".len()));
    }

    #[test]
    fn first_cursor_wins_and_all_are_removed() {
        let out = render_template("a{{cursor}}b{{cursor}}c", &ctx());
        assert_eq!(out.text, "abc");
        assert_eq!(out.cursor_offset, Some(1));
    }

    #[test]
    fn no_cursor_gives_none() {
        assert_eq!(render_template("plain", &ctx()).cursor_offset, None);
    }

    #[test]
    fn escaped_braces_are_literal() {
        assert_eq!(r("\\{{date}}"), "{{date}}");
        assert_eq!(r("\\{{cursor}} {{title}}"), "{{cursor}} 周报");
    }

    #[test]
    fn escaped_cursor_does_not_set_offset() {
        let out = render_template("\\{{cursor}}", &ctx());
        assert_eq!(out.text, "{{cursor}}");
        assert_eq!(out.cursor_offset, None);
    }

    #[test]
    fn unknown_variables_stay_as_written() {
        assert_eq!(r("{{nope}}"), "{{nope}}");
        assert_eq!(r("{{ nope }}"), "{{ nope }}");
        assert_eq!(r("{{ title }}"), "周报");
        assert_eq!(r("{{}}"), "{{}}");
        assert_eq!(r("[{{date:}}]"), "[]");
    }

    #[test]
    fn unclosed_braces_are_kept() {
        assert_eq!(r("start {{date"), "start {{date");
        assert_eq!(r("no vars at all"), "no vars at all");
        assert_eq!(r(""), "");
    }

    #[test]
    fn cjk_text_around_variables_is_preserved() {
        assert_eq!(
            r("今天是{{date:M月D日}}，{{weekday}}。"),
            "今天是3月4日，星期三。"
        );
    }

    #[test]
    fn frontmatter_is_kept_intact() {
        let tpl =
            "---\ntags: [日记]\ncreated: {{date}}\n# comment kept\nempty:\n---\n\n# {{title}}\n";
        let out = r(tpl);
        assert_eq!(
            out,
            "---\ntags: [日记]\ncreated: 2026-03-04\n# comment kept\nempty:\n---\n\n# 周报\n"
        );
    }

    #[test]
    fn list_templates_reads_only_markdown_in_folder() {
        let root = crate::test_util::temp_root("list");
        fs::create_dir_all(root.join("模板/子目录")).unwrap();
        fs::write(root.join("模板/日记.md"), "x").unwrap();
        fs::write(root.join("模板/周报.md"), "y").unwrap();
        fs::write(root.join("模板/图.png"), "z").unwrap();
        fs::write(root.join("模板/.hidden.md"), "h").unwrap();
        fs::write(root.join("模板/子目录/深.md"), "d").unwrap();
        let list = list_templates(&root, "");
        let names: Vec<&str> = list.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["周报", "日记"]);
        assert_eq!(list[0].path, "模板/周报.md");
        crate::test_util::cleanup(&root);
    }

    #[test]
    fn list_templates_custom_folder_and_missing_folder() {
        let root = crate::test_util::temp_root("custom");
        fs::create_dir_all(root.join("tpl")).unwrap();
        fs::write(root.join("tpl/a.md"), "a").unwrap();
        assert_eq!(list_templates(&root, "tpl").len(), 1);
        assert!(list_templates(&root, "missing").is_empty());
        assert!(list_templates(&root, "../escape").is_empty());
        assert!(list_templates(&root, "/abs").is_empty());
        crate::test_util::cleanup(&root);
    }

    #[test]
    fn read_template_rejects_unsafe_paths() {
        let root = crate::test_util::temp_root("read");
        fs::create_dir_all(root.join("模板")).unwrap();
        fs::write(root.join("模板/a.md"), "内容").unwrap();
        assert_eq!(read_template(&root, "模板/a.md").as_deref(), Some("内容"));
        assert_eq!(read_template(&root, r"模板\a.md").as_deref(), Some("内容"));
        assert!(read_template(&root, "../a.md").is_none());
        assert!(read_template(&root, "模板/a.txt").is_none());
        crate::test_util::cleanup(&root);
    }

    #[test]
    fn rules_default_when_missing_or_corrupt() {
        let root = crate::test_util::temp_root("rules-missing");
        assert_eq!(TemplateRules::load(&root), TemplateRules::default());
        fs::create_dir_all(root.join(".jiyue")).unwrap();
        fs::write(TemplateRules::settings_path(&root), "{ not json").unwrap();
        assert_eq!(TemplateRules::load(&root), TemplateRules::default());
        fs::write(TemplateRules::settings_path(&root), "[1,2,3]").unwrap();
        assert_eq!(TemplateRules::load(&root), TemplateRules::default());
        crate::test_util::cleanup(&root);
    }

    #[test]
    fn rules_partial_file_fills_defaults() {
        let root = crate::test_util::temp_root("rules-partial");
        fs::create_dir_all(root.join(".jiyue")).unwrap();
        fs::write(
            TemplateRules::settings_path(&root),
            "version = 1
[templates.folder_templates]
\"日记\" = \"模板/日记.md\"
",
        )
        .unwrap();
        let rules = TemplateRules::load(&root);
        assert_eq!(rules.templates_folder, DEFAULT_TEMPLATE_FOLDER);
        assert_eq!(rules.default_template_for("日记"), Some("模板/日记.md"));
        crate::test_util::cleanup(&root);
    }

    #[test]
    fn rules_unsafe_templates_folder_is_replaced() {
        let root = crate::test_util::temp_root("rules-unsafe");
        fs::create_dir_all(root.join(".jiyue")).unwrap();
        fs::write(
            TemplateRules::settings_path(&root),
            "version = 1
[templates]
templates_folder = \"../../etc\"
",
        )
        .unwrap();
        assert_eq!(
            TemplateRules::load(&root).templates_folder,
            DEFAULT_TEMPLATE_FOLDER
        );
        crate::test_util::cleanup(&root);
    }

    #[test]
    fn rules_save_and_load_round_trip() {
        let root = crate::test_util::temp_root("rules-roundtrip");
        let mut rules = TemplateRules {
            templates_folder: "tpl".into(),
            ..TemplateRules::default()
        };
        rules.set_default("projects", "tpl/project.md");
        rules.set_default("/projects/x/", "tpl/x.md");
        rules.save(&root).unwrap();
        assert!(!root.join(".jiyue/config.toml.tmp").exists());
        let loaded = TemplateRules::load(&root);
        assert_eq!(loaded, rules);
        crate::test_util::cleanup(&root);
    }

    #[test]
    fn rules_longest_folder_wins() {
        let mut rules = TemplateRules::default();
        rules.set_default("", "模板/默认.md");
        rules.set_default("projects", "模板/项目.md");
        rules.set_default("projects/x", "模板/x.md");
        assert_eq!(
            rules.default_template_for("projects/x/deep"),
            Some("模板/x.md")
        );
        assert_eq!(
            rules.default_template_for("projects/y"),
            Some("模板/项目.md")
        );
        assert_eq!(
            rules.default_template_for("projectsX"),
            Some("模板/默认.md")
        );
        assert_eq!(rules.default_template_for(""), Some("模板/默认.md"));
    }

    #[test]
    fn rules_no_match_without_root_rule() {
        let mut rules = TemplateRules::default();
        rules.set_default("projects", "模板/项目.md");
        assert_eq!(rules.default_template_for("journal"), None);
        assert_eq!(rules.default_template_for("projects"), Some("模板/项目.md"));
    }

    #[test]
    fn rules_empty_template_removes_rule() {
        let mut rules = TemplateRules::default();
        rules.set_default("a", "t.md");
        rules.set_default("a", "  ");
        assert!(rules.folder_templates.is_empty());
    }

    #[test]
    fn rules_backslash_folders_are_normalised() {
        let mut rules = TemplateRules::default();
        rules.set_default(r"a\b", "t.md");
        assert_eq!(rules.default_template_for("a/b/c"), Some("t.md"));
    }

    #[test]
    fn rules_save_overwrites_previous() {
        let root = crate::test_util::temp_root("rules-overwrite");
        let mut rules = TemplateRules::default();
        rules.set_default("a", "one.md");
        rules.save(&root).unwrap();
        rules.set_default("a", "two.md");
        rules.save(&root).unwrap();
        assert_eq!(
            TemplateRules::load(&root).default_template_for("a"),
            Some("two.md")
        );
        crate::test_util::cleanup(&root);
    }

    #[test]
    fn uuid_v4_format_is_valid_for_many_counters() {
        for counter in 0..200 {
            let id = uuid_v4(7, counter);
            assert_eq!(id.len(), 36);
            assert!(
                id.chars()
                    .all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
            assert_eq!(id.as_bytes()[14], b'4');
        }
    }

    #[test]
    fn invalid_moment_is_clamped_not_panicking() {
        let mut c = ctx();
        c.now = Moment {
            y: 2026,
            m: 99,
            d: 99,
            h: 99,
            mi: 99,
            s: 99,
        };
        let out = render_template("{{date}} {{time}}", &c).text;
        assert_eq!(out, "2026-12-31 23:59");
    }

    #[test]
    fn leap_day_and_weekday_for_unusual_year() {
        let mut c = ctx();
        c.now = Moment {
            y: 2024,
            m: 2,
            d: 29,
            h: 12,
            mi: 0,
            s: 0,
        };
        assert_eq!(render_template("{{weekday}}", &c).text, "星期四");
        assert_eq!(render_template("{{date+1y}}", &c).text, "2025-02-28");
    }
}
