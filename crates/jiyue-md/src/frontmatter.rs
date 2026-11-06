//! Lossless YAML frontmatter for notes that start with `---` ... `---` (or `...`).
//!
//! Reading uses `yaml-rust2` to validate the block and to decode values. Editing works on
//! whole top-level entries only: a key line plus its continuation lines. Everything else
//! (comments, other keys, their quoting, line endings, blank lines) is copied byte for byte.
//!
//! Safety rules:
//! - A block that is not valid YAML, or whose top level is not a mapping, or that repeats a
//!   key, is treated as "no frontmatter". Edit functions return such text unchanged.
//! - Every edit result is re-checked. If it would not parse, the original text is returned.
//! - Nested mappings are never rewritten.

use std::ops::Range;

use yaml_rust2::{Yaml, YamlLoader};

/// The frontmatter block of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    /// The YAML text between the delimiter lines, exactly as written.
    pub raw: String,
    /// Byte range of the whole block, from offset 0 through the closing delimiter line
    /// including its line terminator. The note body starts at `range.end`.
    pub range: Range<usize>,
}

/// A decoded frontmatter value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Number(f64),
    Bool(bool),
    /// Unquoted `YYYY-MM-DD`.
    Date(String),
    /// Unquoted `YYYY-MM-DDTHH:MM[:SS...]` or with a space instead of `T`.
    DateTime(String),
    /// Inline `[a, b]` or block `- a` sequences. Items are decoded to strings.
    List(Vec<String>),
    Null,
}

/// A top-level frontmatter property.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub key: String,
    pub value: Value,
}

/// One top-level entry: the key line plus its continuation lines.
#[derive(Debug, Clone)]
struct Entry {
    /// Key with surrounding quotes removed.
    key: String,
    /// Byte offset of the first byte of the key line.
    start: usize,
    /// Byte offset of the `:` that ends the key.
    colon: usize,
    /// Byte offset just after the last line of the entry, including its newline.
    end: usize,
}

struct Parsed {
    fm: Frontmatter,
    /// Byte offset of the first closing-delimiter byte (start of the closing line).
    close_start: usize,
    eol: &'static str,
    entries: Vec<Entry>,
}

enum Scan {
    /// No frontmatter block.
    None,
    /// A block is present but cannot be used. Text must be left untouched.
    Invalid,
    Valid(Parsed),
}

/// Limits that keep `yaml-rust2` away from stack-exhausting inputs.
const MAX_INDENT: usize = 128;
const MAX_BRACKET_DEPTH: usize = 64;

fn find_nl(bytes: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .iter()
        .position(|&b| b == b'\n')
        .map(|p| p + from)
}

fn detect_eol(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Rough guard against deeply nested input before handing it to the YAML parser.
fn too_deep(inner: &str) -> bool {
    let mut depth = 0usize;
    for line in inner.lines() {
        if line.len() - line.trim_start_matches(' ').len() > MAX_INDENT {
            return true;
        }
    }
    for c in inner.bytes() {
        match c {
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_BRACKET_DEPTH {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Splits `key: value` at the first `:` followed by a space, tab or line end.
/// Returns the unquoted key and the byte index of the colon within `content`.
fn key_of(content: &str) -> Option<(String, usize)> {
    let b = content.as_bytes();
    let first = *b.first()?;
    if first == b'"' || first == b'\'' {
        let mut i = 1usize;
        loop {
            if i >= b.len() {
                return None;
            }
            if first == b'"' && b[i] == b'\\' {
                i += 2;
                continue;
            }
            if b[i] == first {
                break;
            }
            i += 1;
        }
        let key = content.get(1..i)?.to_string();
        let mut j = i + 1;
        while j < b.len() && matches!(b[j], b' ' | b'\t') {
            j += 1;
        }
        if j >= b.len() || b[j] != b':' {
            return None;
        }
        return Some((key, j));
    }
    if matches!(
        first,
        b'[' | b'{' | b'?' | b'-' | b'#' | b'&' | b'*' | b'!' | b'|' | b'>' | b'%' | b'@' | b'`'
    ) {
        return None;
    }
    let idx = (0..b.len())
        .find(|&i| b[i] == b':' && (i + 1 == b.len() || matches!(b[i + 1], b' ' | b'\t')))?;
    let key = content.get(..idx)?.trim_end();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), idx))
}

/// True for a block-sequence item line at column 0 or indented: `- x`, `-`, `-\tx`.
fn seq_line(content: &str) -> bool {
    let t = content.trim_start_matches([' ', '\t']);
    t == "-" || t.starts_with("- ") || t.starts_with("-\t")
}

/// Scans the lines between the delimiters and groups them into top-level entries.
fn scan_entries(text: &str, open_end: usize, close_start: usize) -> Vec<Entry> {
    let bytes = text.as_bytes();
    let mut entries: Vec<Entry> = Vec::new();
    let mut cur: Option<Entry> = None;
    let mut pos = open_end;
    while pos < close_start {
        let nl = find_nl(&bytes[..close_start], pos).unwrap_or(close_start);
        let next = (nl + 1).min(close_start);
        let content = text.get(pos..nl).unwrap_or("").trim_end_matches('\r');
        if content.trim().is_empty() || content.starts_with('#') {
            // Blank lines and top-level comments belong to no entry.
        } else if content.starts_with([' ', '\t']) || seq_line(content) {
            if let Some(e) = cur.as_mut() {
                e.end = next;
            }
        } else if let Some((key, colon_in)) = key_of(content) {
            if let Some(e) = cur.take() {
                entries.push(e);
            }
            cur = Some(Entry {
                key,
                start: pos,
                colon: pos + colon_in,
                end: next,
            });
        }
        pos = next;
    }
    if let Some(e) = cur.take() {
        entries.push(e);
    }
    entries
}

fn to_real(s: &str) -> Option<f64> {
    match s {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => Some(f64::NAN),
        _ => s.parse::<f64>().ok(),
    }
}

/// `YYYY-MM-DD` with a plausible month and day.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|x| x.iter().all(u8::is_ascii_digit));
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u32 = s.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(0);
    let day: u32 = s.get(8..10).and_then(|d| d.parse().ok()).unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn is_datetime(s: &str) -> bool {
    let Some(date) = s.get(..10) else {
        return false;
    };
    if !is_date(date) {
        return false;
    }
    let rest = &s[10..];
    let Some(sep) = rest.as_bytes().first() else {
        return false;
    };
    matches!(sep, b'T' | b't' | b' ')
        && rest
            .get(1..2)
            .is_some_and(|d| d.bytes().all(|c| c.is_ascii_digit()))
        && rest.len() > 2
}

fn scalar_string(y: &Yaml) -> Option<String> {
    match y {
        Yaml::String(s) | Yaml::Real(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        Yaml::Null => Some(String::new()),
        _ => None,
    }
}

/// Converts a parsed YAML value to a [`Value`]. `raw` is the source text after the colon.
fn to_value(y: &Yaml, raw: &str) -> Value {
    let quoted = raw.starts_with('"') || raw.starts_with('\'');
    match y {
        Yaml::Real(s) => match to_real(s) {
            Some(n) => Value::Number(n),
            None => Value::Text(s.clone()),
        },
        Yaml::Integer(i) => Value::Number(*i as f64),
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::String(s) => {
            if !quoted && is_date(s) {
                Value::Date(s.clone())
            } else if !quoted && is_datetime(s) {
                Value::DateTime(s.clone())
            } else {
                Value::Text(s.clone())
            }
        }
        Yaml::Null => Value::Null,
        Yaml::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match scalar_string(it) {
                    Some(s) => out.push(s),
                    None => return Value::Text(raw.to_string()),
                }
            }
            Value::List(out)
        }
        _ => Value::Text(raw.to_string()),
    }
}

/// Parses one entry as a standalone YAML document and returns its value node.
fn entry_yaml(text: &str, e: &Entry) -> Option<Yaml> {
    let snippet = text.get(e.start..e.end)?;
    let docs = YamlLoader::load_from_str(snippet).ok()?;
    match docs.into_iter().next()? {
        Yaml::Hash(h) => h.iter().next().map(|(_, v)| v.clone()),
        _ => None,
    }
}

fn parse_entry(text: &str, e: &Entry) -> Option<Value> {
    let y = entry_yaml(text, e)?;
    let raw = text.get(e.colon + 1..e.end)?.trim();
    Some(to_value(&y, raw))
}

/// Full scan of the block. Returns `Invalid` for any block that cannot be used.
fn scan(text: &str) -> Scan {
    let bytes = text.as_bytes();
    let bom = if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let Some(first_nl) = find_nl(bytes, bom) else {
        return Scan::None;
    };
    let first = text.get(bom..first_nl).unwrap_or("");
    if first.trim_end() != "---" {
        return Scan::None;
    }
    let eol = if first.ends_with('\r') { "\r\n" } else { "\n" };
    let open_end = first_nl + 1;

    let mut pos = open_end;
    let (close_start, block_end) = loop {
        if pos >= bytes.len() {
            return Scan::None;
        }
        let nl = find_nl(bytes, pos).unwrap_or(bytes.len());
        let next = if nl < bytes.len() { nl + 1 } else { nl };
        let line = text.get(pos..nl).unwrap_or("").trim_end();
        if line == "---" || line == "..." {
            break (pos, next);
        }
        pos = next;
    };

    let inner = text.get(open_end..close_start).unwrap_or("");
    if too_deep(inner) {
        return Scan::Invalid;
    }
    let docs = match YamlLoader::load_from_str(inner) {
        Ok(d) => d,
        Err(_) => return Scan::Invalid,
    };
    let expected = match docs.len() {
        0 => 0,
        1 => match &docs[0] {
            Yaml::Hash(h) => h.len(),
            Yaml::Null => 0,
            _ => return Scan::Invalid,
        },
        _ => return Scan::Invalid,
    };
    let entries = scan_entries(text, open_end, close_start);
    if entries.len() != expected {
        return Scan::Invalid;
    }
    let mut seen = std::collections::HashSet::new();
    if !entries.iter().all(|e| seen.insert(e.key.as_str())) {
        return Scan::Invalid;
    }
    Scan::Valid(Parsed {
        fm: Frontmatter {
            raw: inner.to_string(),
            range: 0..block_end,
        },
        close_start,
        eol,
        entries,
    })
}

fn find_entry<'a>(p: &'a Parsed, key: &str) -> Option<&'a Entry> {
    p.entries.iter().find(|e| e.key == key)
}

fn splice(text: &str, r: Range<usize>, insert: &str) -> String {
    let (Some(head), Some(tail)) = (text.get(..r.start), text.get(r.end..)) else {
        return text.to_string();
    };
    let mut out = String::with_capacity(head.len() + insert.len() + tail.len());
    out.push_str(head);
    out.push_str(insert);
    out.push_str(tail);
    out
}

/// Returns `candidate` only if it still holds a valid frontmatter block. Otherwise returns
/// `original`, so an edit can never leave a broken block behind.
fn finish_with(original: &str, candidate: String) -> String {
    match scan(&candidate) {
        Scan::Valid(_) => candidate,
        _ => original.to_string(),
    }
}

/// Splits off the frontmatter block. Returns the block (if valid) and the offset where the
/// body starts (`0` when there is no block).
pub fn split(text: &str) -> (Option<Frontmatter>, usize) {
    match scan(text) {
        Scan::Valid(p) => {
            let end = p.fm.range.end;
            (Some(p.fm), end)
        }
        _ => (None, 0),
    }
}

/// All top-level properties in document order. Empty when there is no valid block.
pub fn properties(text: &str) -> Vec<Property> {
    match scan(text) {
        Scan::Valid(p) => p
            .entries
            .iter()
            .filter_map(|e| {
                parse_entry(text, e).map(|value| Property {
                    key: e.key.clone(),
                    value,
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The value of one property.
pub fn get(text: &str, key: &str) -> Option<Value> {
    properties(text)
        .into_iter()
        .find(|p| p.key == key)
        .map(|p| p.value)
}

fn push_unique(out: &mut Vec<String>, s: &str) {
    let t = s.trim().trim_start_matches('#').trim();
    if !t.is_empty() && !out.iter().any(|x| x == t) {
        out.push(t.to_string());
    }
}

/// Tags from the `tags` or `tag` property. Accepts a list or a string separated by commas or
/// whitespace. Leading `#` is removed and duplicates are dropped.
pub fn tags(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for p in properties(text) {
        if p.key != "tags" && p.key != "tag" {
            continue;
        }
        match p.value {
            Value::List(items) => {
                for item in &items {
                    push_unique(&mut out, item);
                }
            }
            Value::Text(s) => {
                for tok in s.split(|c: char| c == ',' || c.is_whitespace()) {
                    push_unique(&mut out, tok);
                }
            }
            Value::Number(n) => push_unique(&mut out, &n.to_string()),
            _ => {}
        }
    }
    out
}

/// Aliases from the `aliases` or `alias` property. A string is one alias (it may contain
/// spaces and commas).
pub fn aliases(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for p in properties(text) {
        if p.key != "aliases" && p.key != "alias" {
            continue;
        }
        match p.value {
            Value::List(items) => {
                for item in items {
                    let t = item.trim();
                    if !t.is_empty() {
                        out.push(t.to_string());
                    }
                }
            }
            Value::Text(s) => {
                let t = s.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

/// The `title` property as text, when it is a non-empty string or number.
pub fn title_from(text: &str) -> Option<String> {
    match get(text, "title")? {
        Value::Text(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// True when `s` can be written as a plain YAML scalar and reads back as the same string.
fn plain_ok(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    if s.trim() != s || s.chars().any(char::is_control) {
        return false;
    }
    // Flow indicators are quoted too, because the value may sit inside an inline list.
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) || s.contains([',', '[', ']', '{', '}']) {
        return false;
    }
    if s.contains(": ") || s.contains(" #") || s.ends_with(':') {
        return false;
    }
    if is_date(s) || is_datetime(s) || s.parse::<f64>().is_ok() {
        return false;
    }
    if matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off" | "y" | "n"
    ) {
        return false;
    }
    let probe = format!("k: {s}");
    match YamlLoader::load_from_str(&probe) {
        Ok(docs) => match docs.first() {
            Some(Yaml::Hash(h)) => {
                h.len() == 1
                    && matches!(h.iter().next(), Some((_, Yaml::String(v))) if v.as_str() == s)
            }
            _ => false,
        },
        Err(_) => false,
    }
}

fn render_scalar(s: &str) -> String {
    if plain_ok(s) { s.to_string() } else { quote(s) }
}

fn render_number(n: f64) -> String {
    if n.is_nan() {
        ".nan".to_string()
    } else if n.is_infinite() {
        if n > 0.0 {
            ".inf".to_string()
        } else {
            "-.inf".to_string()
        }
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn render_value(v: &Value) -> String {
    match v {
        Value::Text(s) => render_scalar(s),
        Value::Number(n) => render_number(*n),
        Value::Bool(b) => b.to_string(),
        Value::Date(s) | Value::DateTime(s) => {
            if is_date(s) || is_datetime(s) {
                s.clone()
            } else {
                render_scalar(s)
            }
        }
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(|s| render_scalar(s)).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Null => "null".to_string(),
    }
}

/// Sets `key` to `value`. Replaces the key's line in place, or appends a new line before the
/// closing delimiter, or creates a frontmatter block when there is none. Other lines are
/// untouched.
pub fn set_property(text: &str, key: &str, value: &Value) -> String {
    let line = format!("{}: {}", render_scalar(key), render_value(value));
    match scan(text) {
        Scan::Invalid => text.to_string(),
        Scan::None => {
            let eol = detect_eol(text);
            let mut out = format!("---{eol}{line}{eol}---{eol}");
            out.push_str(text);
            finish_with(text, out)
        }
        Scan::Valid(p) => {
            let insert = format!("{line}{}", p.eol);
            let candidate = match find_entry(&p, key) {
                Some(e) => splice(text, e.start..e.end, &insert),
                None => splice(text, p.close_start..p.close_start, &insert),
            };
            finish_with(text, candidate)
        }
    }
}

/// Removes the top-level property `key`. Text is unchanged when the key is absent.
pub fn remove_property(text: &str, key: &str) -> String {
    match scan(text) {
        Scan::Valid(p) => match find_entry(&p, key) {
            Some(e) => finish_with(text, splice(text, e.start..e.end, "")),
            None => text.to_string(),
        },
        _ => text.to_string(),
    }
}

/// Parses the scalar of one block item line, such as `- "a b"`.
fn seq_value(content: &str) -> Option<String> {
    let t = content.trim_start_matches([' ', '\t']);
    let rest = t.strip_prefix('-')?.trim();
    if rest.is_empty() {
        return None;
    }
    let docs = YamlLoader::load_from_str(&format!("k: {rest}")).ok()?;
    match docs.into_iter().next()? {
        Yaml::Hash(h) => h.iter().next().and_then(|(_, v)| scalar_string(v)),
        _ => None,
    }
}

/// Items of a list-valued entry, or `None` when the entry is a mapping or a non-scalar list.
fn entry_items(y: &Yaml) -> Option<Vec<String>> {
    match y {
        Yaml::Array(items) => items.iter().map(scalar_string).collect(),
        Yaml::Null => Some(Vec::new()),
        other => scalar_string(other).map(|s| vec![s]),
    }
}

/// The text after the colon on the key line only (continuation lines excluded).
fn first_line_rest<'a>(text: &'a str, e: &Entry) -> &'a str {
    let rest = text.get(e.colon + 1..e.end).unwrap_or("");
    rest.split('\n').next().unwrap_or("").trim()
}

fn key_part(text: &str, e: &Entry) -> String {
    text.get(e.start..e.colon)
        .unwrap_or("")
        .trim_end()
        .to_string()
}

/// Indentation used by the block items of an entry, or two spaces.
fn item_indent(text: &str, e: &Entry) -> String {
    let body = text.get(e.start..e.end).unwrap_or("");
    for line in body.lines().skip(1) {
        if seq_line(line) {
            let ws = line.len() - line.trim_start_matches([' ', '\t']).len();
            return line[..ws].to_string();
        }
    }
    "  ".to_string()
}

/// Adds `item` to the list property `key`, creating the property if needed. Existing items
/// are kept. Block lists get a new `- item` line; inline lists are rewritten on their line.
/// A property holding a mapping is left untouched.
pub fn add_to_list(text: &str, key: &str, item: &str) -> String {
    let p = match scan(text) {
        Scan::Invalid => return text.to_string(),
        Scan::None => {
            let eol = detect_eol(text);
            let mut out = format!(
                "---{eol}{}:{eol}  - {}{eol}---{eol}",
                render_scalar(key),
                render_scalar(item)
            );
            out.push_str(text);
            return finish_with(text, out);
        }
        Scan::Valid(p) => p,
    };
    let Some(e) = find_entry(&p, key) else {
        let insert = format!(
            "{}:{eol}  - {}{eol}",
            render_scalar(key),
            render_scalar(item),
            eol = p.eol
        );
        return finish_with(text, splice(text, p.close_start..p.close_start, &insert));
    };
    let Some(y) = entry_yaml(text, e) else {
        return text.to_string();
    };
    let Some(mut items) = entry_items(&y) else {
        return text.to_string();
    };
    if items.iter().any(|x| x == item) {
        return text.to_string();
    }
    let raw_value = first_line_rest(text, e);
    let block_style = raw_value.is_empty() && matches!(y, Yaml::Array(_) | Yaml::Null);
    let candidate = if block_style {
        let indent = match y {
            Yaml::Array(_) => item_indent(text, e),
            _ => "  ".to_string(),
        };
        let line = format!("{indent}- {}{}", render_scalar(item), p.eol);
        splice(text, e.end..e.end, &line)
    } else {
        items.push(item.to_string());
        let line = format!(
            "{}: {}{}",
            key_part(text, e),
            render_value(&Value::List(items)),
            p.eol
        );
        splice(text, e.start..e.end, &line)
    };
    finish_with(text, candidate)
}

/// Removes `item` from the list property `key`. Every matching item is removed. An emptied
/// list stays as `key: []`. Text is unchanged when the item is absent.
pub fn remove_from_list(text: &str, key: &str, item: &str) -> String {
    let Scan::Valid(p) = scan(text) else {
        return text.to_string();
    };
    let Some(e) = find_entry(&p, key) else {
        return text.to_string();
    };
    let Some(y) = entry_yaml(text, e) else {
        return text.to_string();
    };
    let Some(items) = entry_items(&y) else {
        return text.to_string();
    };
    if !items.iter().any(|x| x == item) {
        return text.to_string();
    }
    let raw_value = first_line_rest(text, e);
    let block_style = raw_value.is_empty() && matches!(y, Yaml::Array(_));
    let remaining: Vec<String> = items.into_iter().filter(|x| x != item).collect();

    let candidate = if block_style {
        if remaining.is_empty() {
            splice(
                text,
                e.start..e.end,
                &format!("{}: []{}", key_part(text, e), p.eol),
            )
        } else {
            let body = text.get(e.start..e.end).unwrap_or("");
            let mut out = String::new();
            for (idx, line) in body.split_inclusive('\n').enumerate() {
                let content = line.trim_end_matches(['\r', '\n']);
                if idx > 0 && seq_line(content) && seq_value(content).as_deref() == Some(item) {
                    continue;
                }
                out.push_str(line);
            }
            splice(text, e.start..e.end, &out)
        }
    } else {
        let line = format!(
            "{}: {}{}",
            key_part(text, e),
            render_value(&Value::List(remaining)),
            p.eol
        );
        splice(text, e.start..e.end, &line)
    };
    finish_with(text, candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm(text: &str) -> String {
        format!("---\n{text}---\nBody\n")
    }

    fn val(text: &str, key: &str) -> Value {
        get(text, key).unwrap_or(Value::Null)
    }

    fn assert_valid(text: &str) {
        assert!(
            matches!(scan(text), Scan::Valid(_)),
            "expected valid frontmatter in {text:?}"
        );
    }

    #[test]
    fn no_frontmatter_plain_text() {
        assert_eq!(split("hello"), (None, 0));
        assert!(properties("hello").is_empty());
    }

    #[test]
    fn split_basic_block() {
        let t = fm("title: A\n");
        let (f, body) = split(&t);
        let f = f.unwrap_or_else(|| unreachable!());
        assert_eq!(f.raw, "title: A\n");
        assert_eq!(body, t.len() - "Body\n".len());
        assert_eq!(&t[body..], "Body\n");
        assert_eq!(f.range, 0..body);
    }

    #[test]
    fn split_crlf() {
        let t = "---\r\ntitle: A\r\n---\r\nBody";
        let (f, body) = split(t);
        assert!(f.is_some());
        assert_eq!(&t[body..], "Body");
        assert_eq!(val(t, "title"), Value::Text("A".into()));
    }

    #[test]
    fn dots_terminator_accepted() {
        let t = "---\ntitle: A\n...\nBody";
        let (f, body) = split(t);
        assert!(f.is_some());
        assert_eq!(&t[body..], "Body");
    }

    #[test]
    fn empty_block_is_valid() {
        let t = "---\n---\nBody";
        let (f, _) = split(t);
        assert!(f.is_some());
        assert!(properties(t).is_empty());
    }

    #[test]
    fn unclosed_block_is_not_frontmatter() {
        let t = "---\ntitle: A\nBody";
        assert_eq!(split(t), (None, 0));
    }

    #[test]
    fn thematic_break_is_not_frontmatter() {
        // A `---` line followed by text and another `---` is a scalar, not a mapping.
        let t = "---\njust text\n---\nBody";
        assert_eq!(split(t), (None, 0));
        assert_eq!(scan_kind(t), "invalid");
    }

    fn scan_kind(t: &str) -> &'static str {
        match scan(t) {
            Scan::None => "none",
            Scan::Invalid => "invalid",
            Scan::Valid(_) => "valid",
        }
    }

    #[test]
    fn first_line_must_be_exactly_three_dashes() {
        assert_eq!(scan_kind("----\na: 1\n---\n"), "none");
        assert_eq!(scan_kind("--- x\na: 1\n---\n"), "none");
    }

    #[test]
    fn trailing_spaces_on_delimiters_allowed() {
        assert_eq!(scan_kind("---  \na: 1\n---\t\n"), "valid");
    }

    #[test]
    fn bom_before_frontmatter() {
        let t = "\u{feff}---\ntitle: X\n---\nBody";
        assert_eq!(scan_kind(t), "valid");
        assert_eq!(val(t, "title"), Value::Text("X".into()));
    }

    #[test]
    fn invalid_yaml_is_no_frontmatter() {
        let t = fm("a: [1, 2\n");
        assert_eq!(scan_kind(&t), "invalid");
        assert_eq!(split(&t), (None, 0));
        assert!(properties(&t).is_empty());
    }

    #[test]
    fn invalid_yaml_text_untouched_by_edits() {
        let t = fm("a: [1, 2\n");
        assert_eq!(set_property(&t, "b", &Value::Bool(true)), t);
        assert_eq!(remove_property(&t, "a"), t);
        assert_eq!(add_to_list(&t, "tags", "x"), t);
        assert_eq!(remove_from_list(&t, "tags", "x"), t);
    }

    #[test]
    fn duplicate_keys_are_invalid() {
        let t = fm("a: 1\na: 2\n");
        assert_eq!(scan_kind(&t), "invalid");
        assert_eq!(set_property(&t, "a", &Value::Number(3.0)), t);
    }

    #[test]
    fn scalar_types() {
        let t = fm("text: hello\nnum: 3.5\nint: 42\nflag: true\nnothing: null\nempty:\ntilde: ~\n");
        assert_eq!(val(&t, "text"), Value::Text("hello".into()));
        assert_eq!(val(&t, "num"), Value::Number(3.5));
        assert_eq!(val(&t, "int"), Value::Number(42.0));
        assert_eq!(val(&t, "flag"), Value::Bool(true));
        assert_eq!(val(&t, "nothing"), Value::Null);
        assert_eq!(val(&t, "empty"), Value::Null);
        assert_eq!(val(&t, "tilde"), Value::Null);
    }

    #[test]
    fn date_and_datetime_detection() {
        let t =
            fm("d: 2026-11-04\ndt: 2026-11-04T10:30:00\ndt2: 2026-11-04 10:30\nbad: 2026-13-40\n");
        assert_eq!(val(&t, "d"), Value::Date("2026-11-04".into()));
        assert_eq!(val(&t, "dt"), Value::DateTime("2026-11-04T10:30:00".into()));
        assert_eq!(val(&t, "dt2"), Value::DateTime("2026-11-04 10:30".into()));
        assert_eq!(val(&t, "bad"), Value::Text("2026-13-40".into()));
    }

    #[test]
    fn quoted_date_stays_text() {
        let t = fm("d: \"2026-11-04\"\n");
        assert_eq!(val(&t, "d"), Value::Text("2026-11-04".into()));
    }

    #[test]
    fn quoted_strings_unwrapped() {
        let t = fm("a: \"x: y # z\"\nb: 'it''s'\nc: \"tab\\there\"\n");
        assert_eq!(val(&t, "a"), Value::Text("x: y # z".into()));
        assert_eq!(val(&t, "b"), Value::Text("it's".into()));
        assert_eq!(val(&t, "c"), Value::Text("tab\there".into()));
    }

    #[test]
    fn inline_list() {
        let t = fm("tags: [a, b c, \"d,e\"]\n");
        assert_eq!(
            val(&t, "tags"),
            Value::List(vec!["a".into(), "b c".into(), "d,e".into()])
        );
    }

    #[test]
    fn block_list() {
        let t = fm("tags:\n  - one\n  - two\nnext: 1\n");
        assert_eq!(
            val(&t, "tags"),
            Value::List(vec!["one".into(), "two".into()])
        );
        assert_eq!(val(&t, "next"), Value::Number(1.0));
    }

    #[test]
    fn block_list_at_column_zero() {
        let t = fm("tags:\n- one\n- two\n");
        assert_eq!(
            val(&t, "tags"),
            Value::List(vec!["one".into(), "two".into()])
        );
    }

    #[test]
    fn list_items_numbers_and_bools_become_strings() {
        let t = fm("v: [1, true, 2.5]\n");
        assert_eq!(
            val(&t, "v"),
            Value::List(vec!["1".into(), "true".into(), "2.5".into()])
        );
    }

    #[test]
    fn nested_map_is_text_raw() {
        let t = fm("meta:\n  owner: me\n  id: 7\nafter: yes\n");
        match val(&t, "meta") {
            Value::Text(raw) => {
                assert!(raw.contains("owner: me"));
                assert!(raw.contains("id: 7"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(val(&t, "after"), Value::Text("yes".into()));
    }

    #[test]
    fn inline_map_is_text_raw() {
        let t = fm("m: {a: 1}\n");
        assert_eq!(val(&t, "m"), Value::Text("{a: 1}".into()));
    }

    #[test]
    fn block_scalar_value() {
        let t = fm("desc: |\n  line one\n  line two\nnext: x\n");
        assert_eq!(val(&t, "desc"), Value::Text("line one\nline two\n".into()));
        assert_eq!(val(&t, "next"), Value::Text("x".into()));
    }

    #[test]
    fn infinity_and_nan() {
        let t = fm("a: .inf\nb: -.inf\nc: .nan\n");
        assert_eq!(val(&t, "a"), Value::Number(f64::INFINITY));
        assert_eq!(val(&t, "b"), Value::Number(f64::NEG_INFINITY));
        match val(&t, "c") {
            Value::Number(n) => assert!(n.is_nan()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn properties_keep_order() {
        let t = fm("z: 1\na: 2\nm: 3\n");
        let keys: Vec<String> = properties(&t).into_iter().map(|p| p.key).collect();
        assert_eq!(keys, vec!["z", "a", "m"]);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored_as_entries() {
        let t = fm("# a comment\n\ntitle: T\n# another\nx: 1\n");
        assert_eq!(properties(&t).len(), 2);
        assert_eq!(val(&t, "title"), Value::Text("T".into()));
    }

    #[test]
    fn quoted_key_names() {
        let t = fm("\"my key\": 1\n'other': 2\n");
        assert_eq!(val(&t, "my key"), Value::Number(1.0));
        assert_eq!(val(&t, "other"), Value::Number(2.0));
    }

    #[test]
    fn keys_with_colons_inside() {
        let t = fm("a:b: 1\n");
        assert_eq!(val(&t, "a:b"), Value::Number(1.0));
    }

    #[test]
    fn tags_from_list_and_string() {
        let t = fm("tags: [\"#one\", two, \"#one\"]\n");
        assert_eq!(tags(&t), vec!["one", "two"]);
        let t2 = fm("tag: \"#alpha #beta, gamma\"\n");
        assert_eq!(tags(&t2), vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn tags_none_when_absent() {
        assert!(tags(&fm("title: x\n")).is_empty());
        assert!(tags("no frontmatter #tag").is_empty());
    }

    #[test]
    fn tags_cjk() {
        let t = fm("tags: [笔记, \"#数学 物理\"]\n");
        // A list item is one tag even when it contains a space (Obsidian tags have no spaces,
        // so the value is kept whole rather than guessed apart).
        assert_eq!(tags(&t), vec!["笔记", "数学 物理"]);
    }

    #[test]
    fn tags_number_is_a_tag() {
        let t = fm("tags: 2026\n");
        assert_eq!(tags(&t), vec!["2026"]);
    }

    #[test]
    fn aliases_string_is_single_alias() {
        let t = fm("aliases: My Note, with comma\n");
        assert_eq!(aliases(&t), vec!["My Note, with comma"]);
    }

    #[test]
    fn aliases_list_and_alias_key() {
        let t = fm("aliases:\n  - One\n  - \"  Two  \"\n  - \"\"\n");
        assert_eq!(aliases(&t), vec!["One", "Two"]);
        let t2 = fm("alias: Solo\n");
        assert_eq!(aliases(&t2), vec!["Solo"]);
    }

    #[test]
    fn title_from_text_and_number() {
        assert_eq!(title_from(&fm("title: Hello\n")), Some("Hello".into()));
        assert_eq!(title_from(&fm("title: 2026\n")), Some("2026".into()));
        assert_eq!(title_from(&fm("title: \"  \"\n")), None);
        assert_eq!(title_from(&fm("title: true\n")), None);
        assert_eq!(title_from("no fm"), None);
    }

    #[test]
    fn set_property_creates_block() {
        let out = set_property("Body text\n", "title", &Value::Text("New".into()));
        assert_eq!(out, "---\ntitle: New\n---\nBody text\n");
        assert_valid(&out);
    }

    #[test]
    fn set_property_creates_block_crlf() {
        let out = set_property("Body\r\n", "a", &Value::Number(1.0));
        assert_eq!(out, "---\r\na: 1\r\n---\r\nBody\r\n");
    }

    #[test]
    fn set_property_on_empty_text() {
        let out = set_property("", "k", &Value::Bool(false));
        assert_eq!(out, "---\nk: false\n---\n");
    }

    #[test]
    fn set_property_appends_before_closing() {
        let t = fm("a: 1\n# keep me\nb: 'q'\n");
        let out = set_property(&t, "c", &Value::Text("new".into()));
        assert_eq!(out, "---\na: 1\n# keep me\nb: 'q'\nc: new\n---\nBody\n");
    }

    #[test]
    fn set_property_replaces_only_its_line() {
        let t = fm("# top\na: 1\nb: 'quoted'\n");
        let out = set_property(&t, "a", &Value::Number(2.0));
        assert_eq!(out, "---\n# top\na: 2\nb: 'quoted'\n---\nBody\n");
    }

    #[test]
    fn set_property_replaces_multiline_entry() {
        let t = fm("list:\n  - a\n  - b\nafter: 1\n");
        let out = set_property(&t, "list", &Value::List(vec!["x".into()]));
        assert_eq!(out, "---\nlist: [x]\nafter: 1\n---\nBody\n");
    }

    #[test]
    fn set_property_keeps_crlf() {
        let t = "---\r\na: 1\r\n---\r\nBody\r\n";
        let out = set_property(t, "b", &Value::Text("x".into()));
        assert_eq!(out, "---\r\na: 1\r\nb: x\r\n---\r\nBody\r\n");
    }

    #[test]
    fn set_property_quotes_when_needed() {
        let t = fm("a: 1\n");
        let out = set_property(&t, "s", &Value::Text("yes: maybe".into()));
        assert!(out.contains("s: \"yes: maybe\"\n"), "got {out}");
        let back = get(&out, "s");
        assert_eq!(back, Some(Value::Text("yes: maybe".into())));
    }

    #[test]
    fn set_property_round_trips_tricky_strings() {
        let samples = [
            "true",
            "123",
            "1e3",
            "2026-11-04",
            "null",
            "~",
            "",
            " padded ",
            "# hash",
            "a #b",
            "line\nbreak",
            "tab\there",
            "quote\"s",
            "back\\slash",
            "- dash",
            "[bracket]",
            "é 中文 🙂",
        ];
        for s in samples {
            let t = fm("x: 1\n");
            let out = set_property(&t, "k", &Value::Text(s.to_string()));
            assert_valid(&out);
            assert_eq!(
                get(&out, "k"),
                Some(Value::Text(s.to_string())),
                "sample {s:?}"
            );
        }
    }

    #[test]
    fn set_property_round_trips_numbers_and_bools() {
        let t = fm("");
        for n in [0.0, -3.0, 2.5, 1e16, 0.1] {
            let out = set_property(&t, "n", &Value::Number(n));
            assert_eq!(get(&out, "n"), Some(Value::Number(n)), "number {n}");
        }
        let out = set_property(&t, "b", &Value::Bool(false));
        assert_eq!(get(&out, "b"), Some(Value::Bool(false)));
    }

    #[test]
    fn set_property_date_round_trip() {
        let t = fm("");
        let out = set_property(&t, "d", &Value::Date("2026-11-04".into()));
        assert_eq!(get(&out, "d"), Some(Value::Date("2026-11-04".into())));
        let out2 = set_property(&t, "d", &Value::DateTime("2026-11-04T09:00:00".into()));
        assert_eq!(
            get(&out2, "d"),
            Some(Value::DateTime("2026-11-04T09:00:00".into()))
        );
    }

    #[test]
    fn set_null_value() {
        let t = fm("");
        let out = set_property(&t, "n", &Value::Null);
        assert_eq!(get(&out, "n"), Some(Value::Null));
    }

    #[test]
    fn set_property_with_non_finite_number() {
        let t = fm("");
        let out = set_property(&t, "n", &Value::Number(f64::INFINITY));
        assert_eq!(get(&out, "n"), Some(Value::Number(f64::INFINITY)));
    }

    #[test]
    fn set_property_key_needing_quotes() {
        let t = fm("");
        let out = set_property(&t, "a: b", &Value::Number(1.0));
        assert_eq!(get(&out, "a: b"), Some(Value::Number(1.0)));
    }

    #[test]
    fn set_property_keeps_nested_map_untouched() {
        let t = fm("meta:\n  x: 1\nk: 0\n");
        let out = set_property(&t, "k", &Value::Number(5.0));
        assert_eq!(out, "---\nmeta:\n  x: 1\nk: 5\n---\nBody\n");
    }

    #[test]
    fn set_property_on_document_with_body_containing_dashes() {
        let t = "---\na: 1\n---\nBody\n---\nnot frontmatter\n";
        let out = set_property(t, "a", &Value::Number(9.0));
        assert_eq!(out, "---\na: 9\n---\nBody\n---\nnot frontmatter\n");
    }

    #[test]
    fn set_property_prepends_when_document_starts_with_rule() {
        // "---" followed by text that is not YAML and not closed: a plain thematic break.
        let t = "---\nplain text only\n";
        let out = set_property(t, "a", &Value::Number(1.0));
        assert!(out.starts_with("---\na: 1\n---\n"));
        assert!(out.ends_with("---\nplain text only\n"));
    }

    #[test]
    fn remove_property_removes_entry_lines() {
        let t = fm("a: 1\nlist:\n  - x\n  - y\nb: 2\n");
        let out = remove_property(&t, "list");
        assert_eq!(out, "---\na: 1\nb: 2\n---\nBody\n");
    }

    #[test]
    fn remove_missing_property_is_noop() {
        let t = fm("a: 1\n");
        assert_eq!(remove_property(&t, "zzz"), t);
        assert_eq!(remove_property("plain", "a"), "plain");
    }

    #[test]
    fn remove_last_property_keeps_empty_block() {
        let t = fm("only: 1\n");
        let out = remove_property(&t, "only");
        assert_eq!(out, "---\n---\nBody\n");
        assert_valid(&out);
    }

    #[test]
    fn remove_property_keeps_comments_around() {
        let t = fm("# head\na: 1\n# middle\nb: 2\n");
        let out = remove_property(&t, "a");
        assert_eq!(out, "---\n# head\n# middle\nb: 2\n---\nBody\n");
    }

    #[test]
    fn add_to_list_creates_missing_key_block_style() {
        let t = fm("a: 1\n");
        let out = add_to_list(&t, "tags", "new");
        assert_eq!(out, "---\na: 1\ntags:\n  - new\n---\nBody\n");
        assert_eq!(val(&out, "tags"), Value::List(vec!["new".into()]));
    }

    #[test]
    fn add_to_list_creates_block_when_no_frontmatter() {
        let out = add_to_list("Body", "tags", "x");
        assert_eq!(out, "---\ntags:\n  - x\n---\nBody");
    }

    #[test]
    fn add_to_list_appends_to_block_list() {
        let t = fm("tags:\n  - one\nafter: 1\n");
        let out = add_to_list(&t, "tags", "two");
        assert_eq!(out, "---\ntags:\n  - one\n  - two\nafter: 1\n---\nBody\n");
    }

    #[test]
    fn add_to_list_appends_to_column_zero_block_list() {
        let t = fm("tags:\n- one\n");
        let out = add_to_list(&t, "tags", "two");
        assert_eq!(out, "---\ntags:\n- one\n- two\n---\nBody\n");
    }

    #[test]
    fn add_to_list_rewrites_inline_list() {
        let t = fm("tags: [a, b]  \nz: 1\n");
        let out = add_to_list(&t, "tags", "c");
        assert_eq!(out, "---\ntags: [a, b, c]\nz: 1\n---\nBody\n");
    }

    #[test]
    fn add_to_list_existing_item_is_noop() {
        let t = fm("tags: [a, b]\n");
        assert_eq!(add_to_list(&t, "tags", "a"), t);
    }

    #[test]
    fn add_to_list_scalar_becomes_list() {
        let t = fm("tags: solo\n");
        let out = add_to_list(&t, "tags", "two");
        assert_eq!(out, "---\ntags: [solo, two]\n---\nBody\n");
    }

    #[test]
    fn add_to_list_null_value_becomes_block() {
        let t = fm("tags:\n");
        let out = add_to_list(&t, "tags", "x");
        assert_eq!(out, "---\ntags:\n  - x\n---\nBody\n");
    }

    #[test]
    fn add_to_list_quotes_item_when_needed() {
        let t = fm("tags:\n  - a\n");
        let out = add_to_list(&t, "tags", "x, y");
        assert_eq!(
            get(&out, "tags"),
            Some(Value::List(vec!["a".into(), "x, y".into()]))
        );
    }

    #[test]
    fn add_to_list_on_mapping_is_noop() {
        let t = fm("meta:\n  a: 1\n");
        assert_eq!(add_to_list(&t, "meta", "x"), t);
    }

    #[test]
    fn add_to_list_crlf() {
        let t = "---\r\ntags:\r\n  - a\r\n---\r\nBody";
        let out = add_to_list(t, "tags", "b");
        assert_eq!(out, "---\r\ntags:\r\n  - a\r\n  - b\r\n---\r\nBody");
    }

    #[test]
    fn remove_from_list_block_item() {
        let t = fm("tags:\n  - one\n  - two\n  - three\n");
        let out = remove_from_list(&t, "tags", "two");
        assert_eq!(out, "---\ntags:\n  - one\n  - three\n---\nBody\n");
    }

    #[test]
    fn remove_from_list_last_item_leaves_empty_list() {
        let t = fm("tags:\n  - only\nafter: 1\n");
        let out = remove_from_list(&t, "tags", "only");
        assert_eq!(out, "---\ntags: []\nafter: 1\n---\nBody\n");
        assert_eq!(val(&out, "tags"), Value::List(vec![]));
    }

    #[test]
    fn remove_from_list_inline() {
        let t = fm("tags: [a, \"b c\", d]\n");
        let out = remove_from_list(&t, "tags", "b c");
        assert_eq!(out, "---\ntags: [a, d]\n---\nBody\n");
    }

    #[test]
    fn remove_from_list_removes_all_duplicates() {
        let t = fm("tags: [a, b, a]\n");
        let out = remove_from_list(&t, "tags", "a");
        assert_eq!(out, "---\ntags: [b]\n---\nBody\n");
    }

    #[test]
    fn remove_from_list_quoted_block_item() {
        let t = fm("tags:\n  - \"b c\"\n  - d\n");
        let out = remove_from_list(&t, "tags", "b c");
        assert_eq!(out, "---\ntags:\n  - d\n---\nBody\n");
    }

    #[test]
    fn remove_from_list_missing_item_noop() {
        let t = fm("tags: [a]\n");
        assert_eq!(remove_from_list(&t, "tags", "zzz"), t);
        assert_eq!(remove_from_list(&t, "nokey", "a"), t);
    }

    #[test]
    fn remove_from_list_scalar_value() {
        let t = fm("tags: solo\n");
        let out = remove_from_list(&t, "tags", "solo");
        assert_eq!(out, "---\ntags: []\n---\nBody\n");
    }

    #[test]
    fn remove_from_list_on_mapping_noop() {
        let t = fm("meta:\n  a: 1\n");
        assert_eq!(remove_from_list(&t, "meta", "a"), t);
    }

    #[test]
    fn remove_from_list_crlf() {
        let t = "---\r\ntags: [a, b]\r\n---\r\nBody";
        let out = remove_from_list(t, "tags", "a");
        assert_eq!(out, "---\r\ntags: [b]\r\n---\r\nBody");
    }

    #[test]
    fn remove_from_list_block_crlf() {
        let t = "---\r\ntags:\r\n  - a\r\n  - b\r\n---\r\nBody";
        let out = remove_from_list(t, "tags", "a");
        assert_eq!(out, "---\r\ntags:\r\n  - b\r\n---\r\nBody");
    }

    #[test]
    fn add_then_remove_round_trip() {
        let t = fm("title: T\n");
        let step1 = add_to_list(&t, "tags", "x");
        let step2 = add_to_list(&step1, "tags", "y");
        assert_eq!(tags(&step2), vec!["x", "y"]);
        let step3 = remove_from_list(&step2, "tags", "x");
        assert_eq!(tags(&step3), vec!["y"]);
        assert_eq!(title_from(&step3), Some("T".into()));
    }

    #[test]
    fn edits_preserve_body_exactly() {
        let body = "# Heading\n\n  indented [[link]]\r\n\n```\n---\n```\n";
        let t = format!("---\na: 1\n---\n{body}");
        let out = set_property(&t, "a", &Value::Number(2.0));
        assert!(out.ends_with(body));
        let out2 = add_to_list(&out, "tags", "t");
        assert!(out2.ends_with(body));
        let (_, start) = split(&out2);
        assert_eq!(&out2[start..], body);
    }

    #[test]
    fn oversized_indent_treated_as_invalid() {
        let deep = format!("a:\n{}b: 1\n", " ".repeat(200));
        let t = fm(&deep);
        assert_eq!(scan_kind(&t), "invalid");
    }

    #[test]
    fn deep_brackets_rejected_without_crash() {
        let nested = format!("a: {}{}\n", "[".repeat(200), "]".repeat(200));
        let t = fm(&nested);
        assert_eq!(scan_kind(&t), "invalid");
    }

    #[test]
    fn huge_frontmatter_is_linear() {
        let mut inner = String::new();
        for i in 0..20_000 {
            inner.push_str(&format!("key{i}: value {i}\n"));
        }
        let t = fm(&inner);
        assert_eq!(properties(&t).len(), 20_000);
        let out = set_property(&t, "key5", &Value::Number(0.0));
        assert_eq!(get(&out, "key5"), Some(Value::Number(0.0)));
    }

    #[test]
    fn huge_body_does_not_break_split() {
        let mut body = String::from("---\n");
        body.push_str(&"word ".repeat(200_000));
        let t = format!("---\nt: 1\n---\n{body}");
        let (f, start) = split(&t);
        assert!(f.is_some());
        assert_eq!(&t[start..], body);
    }

    #[test]
    fn yaml_document_end_marker_inside_is_invalid() {
        let t = "---\na: 1\n---\nb: 2\n---\n";
        // The first closing delimiter ends the block. The rest is body.
        assert_eq!(scan_kind(t), "valid");
        assert_eq!(properties(t).len(), 1);
    }

    #[test]
    fn ansi_and_control_chars_in_values_do_not_panic() {
        let t = "---\na: \"\\x1b[31m\"\n---\n";
        let _ = properties(t);
        let _ = set_property(t, "b", &Value::Text("\u{7}bell".into()));
    }

    #[test]
    fn robustness_random_frontmatter_never_panics() {
        let pieces = [
            "---\n",
            "---",
            "...\n",
            "\n",
            "\r\n",
            "a: 1\n",
            "a:",
            " b",
            "- x\n",
            "  - y\n",
            "# c\n",
            "\"q\": 'v'\n",
            "[",
            "]",
            "{",
            "}",
            "k: [1, 2]\n",
            "d: 2026-01-01\n",
            ": \n",
            "\t",
            "é",
            "中: 文\n",
            "x: |\n  block\n",
            "? k\n",
            "&a v\n",
            "*a\n",
            "!tag v\n",
            "k: \"unterminated\n",
        ];
        let mut state: u64 = 0xdead_beef_cafe_f00d;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        for _ in 0..2000 {
            let n = next() % 25;
            let mut s = String::new();
            if next() % 4 != 0 {
                s.push_str("---\n");
            }
            for _ in 0..n {
                s.push_str(pieces[next() % pieces.len()]);
            }
            let (f, start) = split(&s);
            assert!(s.is_char_boundary(start));
            if let Some(f) = f {
                assert!(f.range.end <= s.len());
                assert!(s.is_char_boundary(f.range.end));
            }
            let _ = properties(&s);
            let _ = tags(&s);
            let _ = aliases(&s);
            let _ = title_from(&s);
            let out = set_property(&s, "k", &Value::Text("v".into()));
            let _ = remove_property(&out, "k");
            let out2 = add_to_list(&s, "tags", "t");
            let _ = remove_from_list(&out2, "tags", "t");
            // Every edit either yields a valid block or leaves the text unchanged, and the body
            // after the block is never modified.
            for edited in [&out, &out2] {
                assert!(matches!(scan(edited), Scan::Valid(_)) || edited == &s);
                let (_, b_in) = split(&s);
                let (_, b_out) = split(edited);
                assert_eq!(&s[b_in..], &edited[b_out..]);
            }
        }
    }

    #[test]
    fn value_types_compare_as_expected() {
        assert_eq!(Value::Null, Value::Null);
        assert_ne!(Value::Text("1".into()), Value::Number(1.0));
    }
}
