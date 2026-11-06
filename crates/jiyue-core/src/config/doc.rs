//! Generic loading and saving of the TOML configuration files.
//!
//! Loading: `toml` parses the text, `serde_path_to_error` names the key that fails, the span of the
//! failing value gives a line and column. Keys that the struct does not know are compared with the
//! struct's default serialisation and reported as warnings; they stay in the file on save.
//!
//! Saving: the typed value is serialised with `toml` and merged into the text that is already on
//! disk through `toml_edit`. Comments, key order and unknown keys written by hand survive. Keys
//! listed as maps (`prune` paths) lose the entries that the value no longer has.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use toml_edit::{DocumentMut, Item, Table, Value};

/// Version written at the top of every configuration file.
pub const CONFIG_VERSION: i64 = 1;

/// A problem that stops a file from being read. Carries the file, a 1-based line and column, and
/// the key that failed when the parser knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub key: Option<String>,
    pub message: String,
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file.display(), self.line, self.column)?;
        if let Some(key) = &self.key {
            write!(f, " 键 `{key}`")?;
        }
        write!(f, "：{}", self.message)
    }
}

/// Result of loading one file. A broken file gives `error` and the default value, and the caller
/// must not overwrite it.
#[derive(Debug, Clone)]
pub struct Loaded<T> {
    pub value: T,
    /// Non-fatal problems: unknown keys, a missing version.
    pub warnings: Vec<String>,
    /// Set when the file exists but cannot be used. `value` is then the default.
    pub error: Option<ConfigIssue>,
    /// `true` when the file does not exist.
    pub missing: bool,
}

impl<T: Default> Loaded<T> {
    /// A missing file: defaults, no problem.
    pub fn missing() -> Self {
        Self {
            value: T::default(),
            warnings: Vec::new(),
            error: None,
            missing: true,
        }
    }
}

/// Converts a byte offset in `text` into a 1-based (line, column). Columns count characters.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..floor_char_boundary(text, offset)];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = before[line_start..].chars().count() + 1;
    (line, column)
}

fn floor_char_boundary(text: &str, mut i: usize) -> usize {
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Reads the version of a parsed table. `Ok` with a warning when missing; an error for any value
/// other than `CONFIG_VERSION`.
fn check_version(table: &toml::Table, warnings: &mut Vec<String>) -> Result<(), String> {
    match table.get("version") {
        None => {
            warnings.push("文件没有 version 字段，按版本 1 读取".to_string());
            Ok(())
        }
        Some(toml::Value::Integer(v)) if *v == CONFIG_VERSION => Ok(()),
        Some(other) => Err(format!(
            "不支持的 version = {other}（本版本只认识 version = {CONFIG_VERSION}）"
        )),
    }
}

/// Parses `text` into `T`. `file` is only used to build the issues.
pub fn load_text<T: Serialize + DeserializeOwned + Default>(file: &Path, text: &str) -> Loaded<T> {
    load_text_with(file, text, &[])
}

/// Like [`load_text`]. `free` lists the dotted paths of map tables: every key below them is
/// allowed and never reported as unknown.
pub fn load_text_with<T: Serialize + DeserializeOwned + Default>(
    file: &Path,
    text: &str,
    free: &[&str],
) -> Loaded<T> {
    let issue = |offset: usize, key: Option<String>, message: String| {
        let (line, column) = line_col(text, offset);
        Loaded {
            value: T::default(),
            warnings: Vec::new(),
            error: Some(ConfigIssue {
                file: file.to_path_buf(),
                line,
                column,
                key,
                message,
            }),
            missing: false,
        }
    };
    let table: toml::Table = match toml::from_str(text) {
        Ok(t) => t,
        Err(e) => {
            let offset = e.span().map_or(0, |s| s.start);
            return issue(offset, None, first_line(e.message()));
        }
    };
    let mut warnings = Vec::new();
    if let Err(message) = check_version(&table, &mut warnings) {
        return Loaded {
            value: T::default(),
            warnings,
            error: Some(ConfigIssue {
                file: file.to_path_buf(),
                line: 1,
                column: 1,
                key: Some("version".to_string()),
                message,
            }),
            missing: false,
        };
    }
    let value: T = match serde_path_to_error::deserialize(toml::Deserializer::new(text)) {
        Ok(v) => v,
        Err(e) => {
            let key = e.path().to_string();
            let key = (key != ".").then_some(key);
            let inner = e.into_inner();
            let offset = inner.span().map_or(0, |s| s.start);
            return issue(offset, key, first_line(inner.message()));
        }
    };
    let schema: toml::Table = toml::to_string(&T::default())
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default();
    let mut unknown = Vec::new();
    unknown_keys(&table, &schema, "", free, &mut unknown);
    for key in unknown {
        warnings.push(format!("未知的键 `{key}`（已保留在文件中，不起作用）"));
    }
    Loaded {
        value,
        warnings,
        error: None,
        missing: false,
    }
}

/// Collects the dotted paths of keys that the schema (the serialised default value) does not
/// have. A key below a map (`free`) is never unknown. Array elements are checked against the
/// first element of the schema's array when it has one.
fn unknown_keys(
    actual: &toml::Table,
    schema: &toml::Table,
    prefix: &str,
    free: &[&str],
    out: &mut Vec<String>,
) {
    for (key, value) in actual {
        if prefix.is_empty() && key == "version" {
            continue;
        }
        let path = join(prefix, key);
        if free.contains(&path.as_str()) {
            continue;
        }
        match schema.get(key) {
            None => out.push(path),
            Some(toml::Value::Table(sub)) => {
                if let toml::Value::Table(actual_sub) = value {
                    unknown_keys(actual_sub, sub, &path, free, out);
                }
            }
            Some(toml::Value::Array(schema_items)) => {
                if let (toml::Value::Array(items), Some(toml::Value::Table(sub))) =
                    (value, schema_items.first())
                {
                    for (i, item) in items.iter().enumerate() {
                        if let toml::Value::Table(item_table) = item {
                            unknown_keys(item_table, sub, &format!("{path}[{i}]"), free, out);
                        }
                    }
                }
            }
            Some(_) => {}
        }
    }
}

fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or("").trim().to_string()
}

/// Reads and parses a file. A missing file gives `Loaded::missing`; an unreadable one is an error.
pub fn load_file<T: Serialize + DeserializeOwned + Default>(path: &Path) -> Loaded<T> {
    load_file_with(path, &[])
}

/// Reads and parses a file; see [`load_text_with`] for `free`.
pub fn load_file_with<T: Serialize + DeserializeOwned + Default>(
    path: &Path,
    free: &[&str],
) -> Loaded<T> {
    match std::fs::read_to_string(path) {
        Ok(text) => load_text_with(path, &text, free),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded::missing(),
        Err(e) => Loaded {
            value: T::default(),
            warnings: Vec::new(),
            error: Some(ConfigIssue {
                file: path.to_path_buf(),
                line: 1,
                column: 1,
                key: None,
                message: format!("无法读取文件（{}）", e.kind()),
            }),
            missing: false,
        },
    }
}

/// Merges `value` into the document `base` (or into `template` when there is no file yet) and
/// returns the new text. `prune` lists dotted table paths that are maps: entries absent from
/// `value` are removed there. Every other table keeps keys it does not know.
pub fn merge_text<T: Serialize>(
    base: Option<&str>,
    template: &str,
    value: &T,
    prune: &[&str],
) -> Result<String, String> {
    merge_text_inner(base, template, value, prune, true)
}

fn merge_text_inner<T: Serialize>(
    base: Option<&str>,
    template: &str,
    value: &T,
    prune: &[&str],
    owns_root: bool,
) -> Result<String, String> {
    let fresh_text = toml::to_string(value).map_err(|e| e.to_string())?;
    let fresh: DocumentMut = fresh_text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut doc: DocumentMut = base
        .unwrap_or(template)
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    if !doc.contains_key("version") {
        doc.insert("version", toml_edit::value(CONFIG_VERSION));
    }
    merge_table(doc.as_table_mut(), fresh.as_table(), "", prune, owns_root);
    Ok(doc.to_string())
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Leaf names of `Option` fields: an `Option` that is `None` is not written, so an absent key with
/// one of these names means "unset" and is removed from the file on save.
pub const OPTIONAL_KEYS: &[&str] = &[
    "active_chat",
    "active_embedding",
    "embedding_model",
    "last_vault",
    "title",
    "active",
    "other_active",
    "split",
];

fn merge_table(old: &mut Table, new: &Table, prefix: &str, prune: &[&str], owns_root: bool) {
    let prune_here = prune.contains(&prefix);
    // Optional keys are removed when they are absent from `new`, except at the root of a partial
    // save: there the root keys are other sections that the value does not hold.
    let optional_here = !prefix.is_empty() || owns_root;
    let stale: Vec<String> = old
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| {
            !new.contains_key(k)
                && (prune_here || (optional_here && OPTIONAL_KEYS.contains(&k.as_str())))
        })
        .collect();
    for key in stale {
        old.remove(&key);
    }
    for (key, new_item) in new.iter() {
        let path = join(prefix, key);
        match old.get_mut(key) {
            Some(Item::Table(old_t)) if new_item.is_table() => {
                if let Some(new_t) = new_item.as_table() {
                    merge_table(old_t, new_t, &path, prune, owns_root);
                }
            }
            Some(Item::ArrayOfTables(old_a)) if new_item.is_array_of_tables() => {
                if let Some(new_a) = new_item.as_array_of_tables() {
                    for (i, new_t) in new_a.iter().enumerate() {
                        if let Some(old_t) = old_a.get_mut(i) {
                            merge_table(old_t, new_t, &format!("{path}[{i}]"), prune, owns_root);
                        } else {
                            old_a.push(new_t.clone());
                        }
                    }
                    while old_a.len() > new_a.len() {
                        old_a.remove(old_a.len() - 1);
                    }
                }
            }
            Some(Item::Value(old_v)) if new_item.is_value() => {
                if let Some(new_v) = new_item.as_value() {
                    replace_value(old_v, new_v);
                }
            }
            // A value written as an inline table (`folder = { Fixed = "x" }`) stays an inline
            // table. Writing it as a `[table]` header here would put that header inside the
            // header of the section above it.
            Some(Item::Value(old_v)) if new_item.is_table() => {
                if let Some(new_t) = new_item.as_table() {
                    let inline = new_t.clone().into_inline_table();
                    replace_value(old_v, &Value::InlineTable(inline));
                }
            }
            Some(slot) => *slot = new_item.clone(),
            None => {
                old.insert(key, new_item.clone());
            }
        }
    }
}

/// Replaces a value and keeps the comment written around it.
fn replace_value(old: &mut Value, new: &Value) {
    let decor = old.decor().clone();
    *old = new.clone();
    *old.decor_mut() = decor;
}

/// Writes `value` to `path`, merged with the existing file. Refuses to write when `existing` is a
/// file that did not load (the caller passes its text only when it loaded cleanly).
pub fn save_file<T: Serialize>(
    path: &Path,
    value: &T,
    template: &str,
    prune: &[&str],
) -> Result<(), String> {
    save_with(path, value, template, prune, true)
}

/// Writes a value that holds one section of the file (see `merge_text_partial`): the other sections
/// and their root keys are kept as they are.
pub fn save_file_partial<T: Serialize>(
    path: &Path,
    value: &T,
    template: &str,
    prune: &[&str],
) -> Result<(), String> {
    save_with(path, value, template, prune, false)
}

fn save_with<T: Serialize>(
    path: &Path,
    value: &T,
    template: &str,
    prune: &[&str],
    owns_root: bool,
) -> Result<(), String> {
    let base = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("无法读取 {}：{}", path.display(), e.kind())),
    };
    let text = merge_text_inner(base.as_deref(), template, value, prune, owns_root)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::fsops::write_atomic(path, text.as_bytes()).map_err(|e| e.to_string())
}

/// Parses a document for direct edits (used by the credentials file).
pub fn parse_doc(text: &str) -> Result<DocumentMut, String> {
    text.parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())
}

/// Parses a file's text for direct edits, or the template when the file does not exist.
pub fn edit_base(path: &Path, template: &str) -> Result<DocumentMut, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => parse_doc(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => parse_doc(template),
        Err(e) => Err(format!("无法读取 {}：{}", path.display(), e.kind())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct Sample {
        name: String,
        count: u32,
        tags: Vec<String>,
        inner: Inner,
        map: std::collections::BTreeMap<String, String>,
        items: Vec<Item2>,
    }

    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct Inner {
        flag: bool,
    }

    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct Item2 {
        id: String,
    }

    const TEMPLATE: &str = "version = 1\n";

    #[test]
    fn line_and_column_count_characters() {
        let text = "a = 1\nname = \"中文\" x\n";
        // The `x` on line 2 follows 12 characters, so it sits in column 13.
        let offset = text.find('x').expect("x");
        assert_eq!(line_col(text, offset), (2, 13));
        assert_eq!(line_col(text, 0), (1, 1));
    }

    #[test]
    fn default_round_trip_through_merge_and_load() {
        let value = Sample {
            name: "霁月".into(),
            count: 3,
            tags: vec!["a".into(), "b".into()],
            inner: Inner { flag: true },
            map: [("k".to_string(), "v".to_string())].into_iter().collect(),
            items: vec![Item2 { id: "x".into() }],
        };
        let text = merge_text(None, TEMPLATE, &value, &["map"]).expect("merge");
        let loaded: Loaded<Sample> = load_text_with(Path::new("t.toml"), &text, &["map"]);
        assert!(loaded.error.is_none());
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.value, value);
    }

    #[test]
    fn comments_and_key_order_survive_a_save() {
        let base = "version = 1\n# 我的注释\nname = \"old\" # 行尾\ncount = 1\n\n[inner]\n# 开关\nflag = false\n";
        let value = Sample {
            name: "new".into(),
            count: 9,
            inner: Inner { flag: true },
            ..Sample::default()
        };
        let text = merge_text(Some(base), TEMPLATE, &value, &[]).expect("merge");
        assert!(text.contains("# 我的注释"), "{text}");
        assert!(text.contains("# 行尾"), "{text}");
        assert!(text.contains("# 开关"), "{text}");
        assert!(text.find("name").expect("name") < text.find("count").expect("count"));
        let loaded: Loaded<Sample> = load_text(Path::new("t.toml"), &text);
        assert_eq!(loaded.value.name, "new");
        assert_eq!(loaded.value.count, 9);
        assert!(loaded.value.inner.flag);
    }

    #[test]
    fn unknown_keys_are_kept_on_save_and_reported() {
        let base = "version = 1\nname = \"a\"\nmystery = 42\n\n[inner]\nextra = \"keep\"\n";
        let loaded: Loaded<Sample> = load_text(Path::new("t.toml"), base);
        assert!(loaded.error.is_none());
        assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
        assert!(loaded.warnings.iter().any(|w| w.contains("mystery")));
        assert!(loaded.warnings.iter().any(|w| w.contains("inner.extra")));
        let text = merge_text(Some(base), TEMPLATE, &loaded.value, &[]).expect("merge");
        assert!(text.contains("mystery = 42"), "{text}");
        assert!(text.contains("extra = \"keep\""), "{text}");
    }

    #[test]
    fn invalid_value_reports_file_line_column_and_key() {
        let text = "version = 1\nname = \"ok\"\ncount = \"many\"\n";
        let loaded: Loaded<Sample> = load_text(Path::new("/vault/.jiyue/config.toml"), text);
        let issue = loaded.error.expect("error");
        assert_eq!(issue.line, 3);
        assert_eq!(issue.column, 9, "column points at the value");
        assert_eq!(issue.key.as_deref(), Some("count"));
        let shown = issue.to_string();
        assert!(shown.contains("config.toml:3:9"), "{shown}");
        assert!(shown.contains("`count`"), "{shown}");
    }

    #[test]
    fn syntax_error_has_a_position() {
        let loaded: Loaded<Sample> = load_text(Path::new("c.toml"), "version = 1\nname = \n");
        let issue = loaded.error.expect("error");
        assert_eq!(issue.line, 2);
    }

    #[test]
    fn unsupported_version_is_an_error() {
        let loaded: Loaded<Sample> = load_text(Path::new("c.toml"), "version = 2\n");
        let issue = loaded.error.expect("error");
        assert!(issue.message.contains("version"));
    }

    #[test]
    fn missing_version_is_only_a_warning() {
        let loaded: Loaded<Sample> = load_text(Path::new("c.toml"), "name = \"x\"\n");
        assert!(loaded.error.is_none());
        assert_eq!(loaded.value.name, "x");
        assert!(loaded.warnings.iter().any(|w| w.contains("version")));
    }

    #[test]
    fn maps_are_pruned_and_tables_are_not() {
        let base = "version = 1\nunknown_top = 1\n[map]\nold = \"gone\"\nkeep = \"x\"\n";
        let value = Sample {
            map: [("keep".to_string(), "y".to_string())]
                .into_iter()
                .collect(),
            ..Sample::default()
        };
        let text = merge_text(Some(base), TEMPLATE, &value, &["map"]).expect("merge");
        assert!(!text.contains("old = "), "{text}");
        assert!(text.contains("keep = \"y\""), "{text}");
        assert!(text.contains("unknown_top = 1"), "{text}");
    }

    #[test]
    fn array_of_tables_keeps_unknown_keys_per_element() {
        let base =
            "version = 1\n\n[[items]]\nid = \"a\"\nnote = \"mine\"\n\n[[items]]\nid = \"b\"\n";
        let value = Sample {
            items: vec![Item2 { id: "a2".into() }],
            ..Sample::default()
        };
        let text = merge_text(Some(base), TEMPLATE, &value, &[]).expect("merge");
        let loaded: Loaded<Sample> = load_text(Path::new("c.toml"), &text);
        assert_eq!(loaded.value.items.len(), 1);
        assert_eq!(loaded.value.items[0].id, "a2");
        assert!(text.contains("note = \"mine\""), "{text}");
    }

    #[test]
    fn save_file_writes_and_reads_back_in_a_temp_dir() {
        let dir = std::env::temp_dir().join(format!("jiyue-doc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        let value = Sample {
            name: "disk".into(),
            ..Sample::default()
        };
        save_file(&path, &value, TEMPLATE, &[]).expect("save");
        let loaded: Loaded<Sample> = load_file(&path);
        assert_eq!(loaded.value.name, "disk");
        let missing: Loaded<Sample> = load_file(&dir.join("none.toml"));
        assert!(missing.missing);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
