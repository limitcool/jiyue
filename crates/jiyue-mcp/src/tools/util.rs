//! Argument parsing, output shaping and small helpers shared by the tool handlers.

use blake3::Hasher;
use jiyue_core::daily::{DailySettings, Date, format_date, parse_date};
use jiyue_core::fsops::{is_ignored, safe_rel_path};
use serde_json::Value;

use crate::vault::MAX_NOTE_BYTES;

/// The string argument `key`, if present. Errors when it is present but not a string.
pub fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("argument {key:?} must be a string")),
    }
}

/// The required string argument `key`.
pub fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    opt_str(args, key)?.ok_or_else(|| format!("missing required argument {key:?}"))
}

/// The optional non-negative integer argument `key`.
pub fn opt_u64(args: &Value, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("argument {key:?} must be a non-negative integer")),
    }
}

/// Pretty-printed JSON text for a tool result.
pub fn json_text(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// The longest prefix of `text` that is at most `max` bytes and ends on a character boundary,
/// with a flag that says whether anything was cut.
pub fn truncate_utf8(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// Caps a whole tool result, adding a note that says how much was dropped.
pub fn cap_output(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let total = text.len();
    let (head, _) = truncate_utf8(&text, max);
    format!(
        "{head}\n[output truncated: showing {} of {total} bytes; narrow the request or lower the limit]",
        head.len()
    )
}

/// Converts CRLF and lone CR line endings to LF.
pub fn normalize_eol(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Normalises content to be written: LF line endings, and at most the note size limit.
pub fn normalize_content(raw: &str) -> Result<String, String> {
    let text = normalize_eol(raw);
    if text.len() as u64 > MAX_NOTE_BYTES {
        return Err(format!(
            "content is {} bytes; the limit is {} bytes",
            text.len(),
            MAX_NOTE_BYTES
        ));
    }
    Ok(text)
}

/// Checks a folder argument. The empty string means the vault root. A trailing slash is allowed.
pub fn normalize_folder(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let folder = trimmed.strip_suffix('/').unwrap_or(trimmed);
    if folder.is_empty() {
        return Ok(String::new());
    }
    safe_rel_path(folder).map_err(|err| format!("invalid folder {raw:?}: {err}"))?;
    if is_ignored(folder) {
        return Err(format!("folder {raw:?} is an ignored or hidden folder"));
    }
    Ok(folder.to_string())
}

/// BLAKE3 hash of `bytes` as lower-case hex.
pub fn hash_hex(bytes: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex().to_string()
}

/// True for a 64-character hex string (a BLAKE3 digest).
pub fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `YYYY-MM-DD` for a date.
pub fn iso_date(date: Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

/// Parses a `YYYY-MM-DD` argument.
pub fn parse_iso_date(text: &str) -> Result<Date, String> {
    parse_date(text.trim(), "YYYY-MM-DD")
        .ok_or_else(|| format!("date must look like 2026-11-04, got {text:?}"))
}

/// Vault-relative path of the daily note for `date`, from the vault's daily settings.
pub fn daily_rel(settings: &DailySettings, date: Date) -> Result<String, String> {
    let folder = normalize_folder(&settings.folder)?;
    let title = format_date(date, &settings.date_format);
    if title.is_empty() || title.contains('/') || title.contains('\\') {
        return Err(format!(
            "daily date format {:?} produces an invalid file name",
            settings.date_format
        ));
    }
    Ok(if folder.is_empty() {
        format!("{title}.md")
    } else {
        format!("{folder}/{title}.md")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn string_arguments_are_typed() {
        let args = json!({"a": "x", "n": 3, "z": null});
        assert_eq!(opt_str(&args, "a"), Ok(Some("x")));
        assert_eq!(opt_str(&args, "z"), Ok(None));
        assert_eq!(opt_str(&args, "missing"), Ok(None));
        assert!(opt_str(&args, "n").is_err());
        assert!(required_str(&args, "missing").is_err());
    }

    #[test]
    fn integer_arguments_are_typed() {
        let args = json!({"n": 3, "neg": -1, "s": "3"});
        assert_eq!(opt_u64(&args, "n"), Ok(Some(3)));
        assert!(opt_u64(&args, "neg").is_err());
        assert!(opt_u64(&args, "s").is_err());
        assert_eq!(opt_u64(&args, "none"), Ok(None));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let text = "ab中文";
        assert_eq!(truncate_utf8(text, 100), (text, false));
        // "ab" is 2 bytes; the next char starts at 2 and takes 3 bytes.
        assert_eq!(truncate_utf8(text, 4), ("ab", true));
        assert_eq!(truncate_utf8(text, 5), ("ab中", true));
    }

    #[test]
    fn cap_output_adds_a_note_only_when_cut() {
        assert_eq!(cap_output("short".into(), 100), "short");
        let capped = cap_output("x".repeat(50), 10);
        assert!(capped.starts_with("xxxxxxxxxx\n[output truncated"));
        assert!(capped.contains("of 50 bytes"));
    }

    #[test]
    fn eol_normalisation_handles_crlf_and_cr() {
        assert_eq!(normalize_eol("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(normalize_eol("plain"), "plain");
    }

    #[test]
    fn content_size_limit_is_enforced() {
        assert!(normalize_content("ok").is_ok());
        let big = "x".repeat(MAX_NOTE_BYTES as usize + 1);
        assert!(normalize_content(&big).is_err());
    }

    #[test]
    fn folders_are_validated() {
        assert_eq!(normalize_folder(""), Ok(String::new()));
        assert_eq!(normalize_folder("  "), Ok(String::new()));
        assert_eq!(normalize_folder("projects/"), Ok("projects".to_string()));
        assert!(normalize_folder("/abs").is_err());
        assert!(normalize_folder("../up").is_err());
        assert!(normalize_folder(".trash").is_err());
        assert!(normalize_folder("a\\b").is_err());
    }

    #[test]
    fn hashes_are_hex_digests() {
        let h = hash_hex(b"hello");
        assert_eq!(h.len(), 64);
        assert!(is_hash(&h));
        assert!(!is_hash("abc"));
        assert!(!is_hash(&"g".repeat(64)));
    }

    #[test]
    fn iso_dates_round_trip() {
        let date = parse_iso_date("2026-11-04").expect("date");
        assert_eq!(iso_date(date), "2026-11-04");
        assert!(parse_iso_date("2026-13-40").is_err());
        assert!(parse_iso_date("yesterday").is_err());
    }

    #[test]
    fn daily_paths_follow_settings() {
        let date = parse_iso_date("2026-11-04").expect("date");
        let settings = DailySettings::default();
        assert_eq!(
            daily_rel(&settings, date).expect("rel"),
            "日记/2026-11-04.md"
        );
        let flat = DailySettings {
            folder: String::new(),
            ..DailySettings::default()
        };
        assert_eq!(daily_rel(&flat, date).expect("rel"), "2026-11-04.md");
        let bad = DailySettings {
            date_format: "YYYY/MM/DD".into(),
            ..DailySettings::default()
        };
        assert!(daily_rel(&bad, date).is_err());
    }
}
