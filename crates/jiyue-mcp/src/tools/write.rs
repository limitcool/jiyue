//! Tools that change notes: create, append, update (with an expected hash), set a frontmatter
//! property, append to the daily note, and delete (only when the server allows it).
//!
//! Every write goes through `jiyue_core::fsops::write_atomic`, keeps LF line endings, respects
//! the 5 MiB limit and refuses anything outside `.md` and `.txt` notes.

use std::fs;
use std::path::Path;

use jiyue_core::daily::{
    DEFAULT_TEMPLATE, DailySettings, Date, format_date, local_hhmm, render_template,
};
use jiyue_core::fsops::{self, TRASH_DIR};
use jiyue_md::embeds;
use jiyue_md::frontmatter::{self, Value as Fm};
use serde_json::Value as Json;

use super::util::{
    daily_rel, hash_hex, is_hash, iso_date, normalize_content, normalize_eol, opt_str,
    parse_iso_date, required_str,
};
use crate::app::App;
use crate::vault::{MAX_NOTE_BYTES, exists, read_note_text};

/// Longest property key accepted by `set_property`.
const MAX_KEY_CHARS: usize = 200;

/// `create_note {path, content}`
pub fn create_note(app: &mut App, args: &Json) -> Result<String, String> {
    let rel = required_str(args, "path")?;
    let content = required_str(args, "content")?;
    let abs = app.root.resolve_note(rel)?;
    if exists(&abs) {
        return Err(format!(
            "{rel} already exists; use update_note to change it"
        ));
    }
    let text = normalize_content(content)?;
    let hash = store(app, rel, &abs, text.as_bytes())?;
    Ok(format!(
        "created {rel}\nhash: {hash}\nbytes: {}",
        text.len()
    ))
}

/// `append_to_note {path, text, heading?}`
pub fn append_to_note(app: &mut App, args: &Json) -> Result<String, String> {
    let rel = required_str(args, "path")?;
    let addition = required_str(args, "text")?;
    let heading = opt_str(args, "heading")?;
    let abs = app.root.resolve_note(rel)?;
    if !exists(&abs) {
        return Err(format!("note not found: {rel}"));
    }
    let current = read_note_text(&abs, rel)?;
    let updated = insert_text(&current, heading, &normalize_eol(addition))?;
    let text = normalize_content(&updated)?;
    let hash = store(app, rel, &abs, text.as_bytes())?;
    let place = match heading {
        Some(h) => format!("under heading {h:?}"),
        None => "at the end".to_string(),
    };
    Ok(format!(
        "appended to {rel} {place}\nhash: {hash}\nbytes: {}",
        text.len()
    ))
}

/// `update_note {path, content, expected_hash}`. Refused when the note changed since it was read.
pub fn update_note(app: &mut App, args: &Json) -> Result<String, String> {
    let rel = required_str(args, "path")?;
    let content = required_str(args, "content")?;
    let expected = required_str(args, "expected_hash")?
        .trim()
        .to_ascii_lowercase();
    if !is_hash(&expected) {
        return Err(
            "expected_hash must be the 64-character hash returned by read_note".to_string(),
        );
    }
    let abs = app.root.resolve_note(rel)?;
    if !exists(&abs) {
        return Err(format!("note not found: {rel}"));
    }
    let current_bytes = crate::vault::read_note_bytes(&abs, rel)?;
    let current = hash_hex(&current_bytes);
    if current != expected {
        return Err(format!(
            "hash mismatch for {rel}: expected {expected}, current {current}. The note changed since it was read; read it again and retry."
        ));
    }
    let text = normalize_content(content)?;
    let hash = store(app, rel, &abs, text.as_bytes())?;
    Ok(format!(
        "updated {rel}\nprevious hash: {current}\nhash: {hash}\nbytes: {}",
        text.len()
    ))
}

/// `set_property {path, key, value}`. The value is a string, number, boolean, null, or a list of
/// strings. Other frontmatter lines are left as they are.
pub fn set_property(app: &mut App, args: &Json) -> Result<String, String> {
    let rel = required_str(args, "path")?;
    let key = required_str(args, "key")?.trim();
    if key.is_empty() || key.chars().count() > MAX_KEY_CHARS || key.contains('\n') {
        return Err("key must be a non-empty single line of at most 200 characters".to_string());
    }
    let raw = args
        .get("value")
        .ok_or_else(|| "missing required argument \"value\"".to_string())?;
    let value = json_to_frontmatter(raw)?;
    let abs = app.root.resolve_note(rel)?;
    if !exists(&abs) {
        return Err(format!("note not found: {rel}"));
    }
    let current = read_note_text(&abs, rel)?;
    let updated = frontmatter::set_property(&current, key, &value);
    if frontmatter::get(&updated, key).as_ref() != Some(&value) {
        return Err(format!(
            "cannot set {key:?} in {rel}: its frontmatter block is not valid YAML or cannot hold this value"
        ));
    }
    let text = normalize_content(&updated)?;
    let hash = store(app, rel, &abs, text.as_bytes())?;
    Ok(format!(
        "set {key} in {rel}\nhash: {hash}\nbytes: {}",
        text.len()
    ))
}

/// `append_to_daily {text, date?}`. Creates the daily note from the template when it is missing.
pub fn append_to_daily(app: &mut App, args: &Json) -> Result<String, String> {
    let addition = required_str(args, "text")?;
    let date = match opt_str(args, "date")? {
        Some(text) => parse_iso_date(text)?,
        None => Date::today(),
    };
    let settings = DailySettings::load(app.root.path());
    let rel = daily_rel(&settings, date)?;
    let abs = app.root.resolve_note(&rel)?;
    let (base, created) = if exists(&abs) {
        (read_note_text(&abs, &rel)?, false)
    } else {
        (new_daily_content(app, &settings, date), true)
    };
    let updated = insert_text(&base, None, &normalize_eol(addition))?;
    let text = normalize_content(&updated)?;
    let hash = store(app, &rel, &abs, text.as_bytes())?;
    let verb = if created {
        "created and appended to"
    } else {
        "appended to"
    };
    Ok(format!(
        "{verb} the daily note {rel} ({})\nhash: {hash}\nbytes: {}",
        iso_date(date),
        text.len()
    ))
}

/// `delete_note {path}`. Moves the note into `.trash` (recoverable from the app).
pub fn delete_note(app: &mut App, args: &Json) -> Result<String, String> {
    let rel = required_str(args, "path")?;
    let abs = app.root.resolve_note(rel)?;
    if !exists(&abs) {
        return Err(format!("note not found: {rel}"));
    }
    let entry = fsops::delete_to_trash(app.root.path(), rel)
        .map_err(|err| format!("cannot delete {rel}: {err}"))?;
    app.catalog.invalidate(rel)?;
    Ok(format!("moved {rel} to {TRASH_DIR}/{}", entry.name))
}

/// Writes `bytes` to `abs` atomically and forgets the cached facts of `rel`. Returns the hash.
fn store(app: &mut App, rel: &str, abs: &Path, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() as u64 > MAX_NOTE_BYTES {
        return Err(format!("{rel} would exceed the 5 MiB limit"));
    }
    fsops::write_atomic(abs, bytes).map_err(|err| format!("cannot write {rel}: {err}"))?;
    app.catalog.invalidate(rel)?;
    Ok(hash_hex(bytes))
}

/// Inserts `addition` at the end of the note, or at the end of the section under `heading`.
/// The result ends with a newline. Pure function, so it is easy to test.
pub fn insert_text(current: &str, heading: Option<&str>, addition: &str) -> Result<String, String> {
    if addition.is_empty() {
        return Err("text is empty".to_string());
    }
    let mut block = addition.to_string();
    if !block.ends_with('\n') {
        block.push('\n');
    }
    match heading {
        None => {
            let mut out = current.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&block);
            Ok(out)
        }
        Some(h) => {
            let range = embeds::heading_section(current, h)
                .ok_or_else(|| format!("heading not found: {h:?}"))?;
            let section = current
                .get(range.clone())
                .ok_or_else(|| "heading range is not on a character boundary".to_string())?;
            let insert_at = range.start + section.trim_end().len();
            let line_end = match current[insert_at..].find('\n') {
                Some(offset) => insert_at + offset + 1,
                None => current.len(),
            };
            let mut out = String::with_capacity(current.len() + block.len() + 1);
            out.push_str(&current[..line_end]);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&block);
            out.push_str(&current[line_end..]);
            Ok(out)
        }
    }
}

/// Text of a new daily note: the configured template (if readable inside the vault) or the
/// built-in default, rendered for `date`.
fn new_daily_content(app: &App, settings: &DailySettings, date: Date) -> String {
    let title = format_date(date, &settings.date_format);
    let template = Some(settings.template_path.trim().to_string())
        .filter(|p| !p.is_empty())
        .and_then(|p| app.root.resolve(&p).ok())
        .and_then(|abs| fs::read_to_string(abs).ok())
        .unwrap_or_else(|| DEFAULT_TEMPLATE.to_string());
    normalize_eol(&render_template(
        &template,
        date,
        &title,
        &settings.date_format,
        &local_hhmm(),
    ))
}

/// Converts a JSON argument to a frontmatter value.
pub fn json_to_frontmatter(value: &Json) -> Result<Fm, String> {
    match value {
        Json::String(s) => Ok(Fm::Text(s.clone())),
        Json::Bool(b) => Ok(Fm::Bool(*b)),
        Json::Number(n) => n
            .as_f64()
            .filter(|f| f.is_finite())
            .map(Fm::Number)
            .ok_or_else(|| "number is out of range".to_string()),
        Json::Null => Ok(Fm::Null),
        Json::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Json::String(s) => out.push(s.clone()),
                    _ => return Err("list values must all be strings".to_string()),
                }
            }
            Ok(Fm::List(out))
        }
        Json::Object(_) => Err("object values are not supported; use a string, number, boolean, null or a list of strings".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, app_for, vault_with, write_file};
    use crate::tools::read::render_note;
    use serde_json::json;

    fn read_hash(app: &App, rel: &str) -> String {
        let out = render_note(app, rel, None, 1 << 20).expect("read");
        out.lines()
            .nth(1)
            .expect("hash line")
            .trim_start_matches("hash: ")
            .to_string()
    }

    #[test]
    fn create_note_writes_a_new_file_and_refuses_existing_ones() {
        let (dir, mut app) = vault_with("create", &[]);
        let out = create_note(
            &mut app,
            &json!({"path": "deep/new.md", "content": "# New\r\nline\r\n"}),
        )
        .expect("create");
        assert!(out.starts_with("created deep/new.md"));
        let on_disk = fs::read_to_string(dir.path().join("deep/new.md")).expect("file");
        assert_eq!(on_disk, "# New\nline\n");
        assert!(create_note(&mut app, &json!({"path": "deep/new.md", "content": "x"})).is_err());
    }

    #[test]
    fn create_note_refuses_non_notes_and_traversal() {
        let (dir, mut app) = vault_with("create-bad", &[]);
        assert!(create_note(&mut app, &json!({"path": "x.png", "content": "x"})).is_err());
        assert!(create_note(&mut app, &json!({"path": "../x.md", "content": "x"})).is_err());
        assert!(create_note(&mut app, &json!({"path": "/abs.md", "content": "x"})).is_err());
        assert!(create_note(&mut app, &json!({"path": ".trash/x.md", "content": "x"})).is_err());
        assert!(!dir.path().join("x.png").exists());
    }

    #[test]
    fn create_note_refuses_oversized_content() {
        let (_dir, mut app) = vault_with("create-big", &[]);
        let big = "x".repeat(MAX_NOTE_BYTES as usize + 10);
        assert!(create_note(&mut app, &json!({"path": "big.md", "content": big})).is_err());
    }

    #[test]
    fn create_note_is_visible_to_search_on_the_next_call() {
        let (_dir, mut app) = vault_with("create-search", &[]);
        create_note(
            &mut app,
            &json!({"path": "fresh.md", "content": "zebra crossing"}),
        )
        .expect("create");
        app.catalog.refresh().expect("refresh");
        let after = app.catalog.search("zebra", 5).expect("search");
        assert_eq!(after.len(), 1);
    }

    #[test]
    fn append_adds_to_the_end_with_a_newline_boundary() {
        let (dir, mut app) = vault_with("append-end", &[("log.md", "first line")]);
        append_to_note(&mut app, &json!({"path": "log.md", "text": "second"})).expect("append");
        assert_eq!(
            fs::read_to_string(dir.path().join("log.md")).expect("file"),
            "first line\nsecond\n"
        );
    }

    #[test]
    fn append_under_a_heading_goes_before_the_next_heading() {
        let text = "# Top\nintro\n\n## Tasks\n- a\n\n## Notes\nkeep\n";
        let (dir, mut app) = vault_with("append-heading", &[("h.md", text)]);
        append_to_note(
            &mut app,
            &json!({"path": "h.md", "text": "- b", "heading": "Tasks"}),
        )
        .expect("append");
        let got = fs::read_to_string(dir.path().join("h.md")).expect("file");
        assert_eq!(
            got,
            "# Top\nintro\n\n## Tasks\n- a\n- b\n\n## Notes\nkeep\n"
        );
    }

    #[test]
    fn append_refuses_unknown_headings_and_missing_notes() {
        let (_dir, mut app) = vault_with("append-bad", &[("h.md", "# A\n")]);
        assert!(
            append_to_note(
                &mut app,
                &json!({"path": "h.md", "text": "x", "heading": "Zed"})
            )
            .is_err()
        );
        assert!(append_to_note(&mut app, &json!({"path": "none.md", "text": "x"})).is_err());
        assert!(append_to_note(&mut app, &json!({"path": "h.md", "text": ""})).is_err());
    }

    #[test]
    fn insert_text_handles_missing_trailing_newlines() {
        assert_eq!(insert_text("", None, "a").expect("ok"), "a\n");
        assert_eq!(insert_text("x", None, "a\n").expect("ok"), "x\na\n");
        assert_eq!(
            insert_text("# H", Some("H"), "item").expect("ok"),
            "# H\nitem\n"
        );
        assert_eq!(
            insert_text("# H\nbody", Some("H"), "item").expect("ok"),
            "# H\nbody\nitem\n"
        );
    }

    #[test]
    fn update_requires_the_current_hash() {
        let (dir, mut app) = vault_with("update", &[("u.md", "version one")]);
        let stale = read_hash(&app, "u.md");
        write_file(dir.path(), "u.md", "version two");
        let err = update_note(
            &mut app,
            &json!({"path": "u.md", "content": "mine", "expected_hash": stale}),
        )
        .expect_err("mismatch is refused");
        assert!(err.contains("hash mismatch"));
        assert_eq!(
            fs::read_to_string(dir.path().join("u.md")).expect("file"),
            "version two"
        );
    }

    #[test]
    fn update_with_the_right_hash_replaces_the_note() {
        let (dir, mut app) = vault_with("update-ok", &[("u.md", "old")]);
        let hash = read_hash(&app, "u.md");
        let out = update_note(
            &mut app,
            &json!({"path": "u.md", "content": "new text", "expected_hash": hash.to_uppercase()}),
        )
        .expect("update");
        assert!(out.contains("previous hash: "));
        assert_eq!(
            fs::read_to_string(dir.path().join("u.md")).expect("file"),
            "new text"
        );
        assert_ne!(read_hash(&app, "u.md"), hash);
    }

    #[test]
    fn update_rejects_malformed_hashes_and_missing_notes() {
        let (_dir, mut app) = vault_with("update-bad", &[("u.md", "old")]);
        assert!(
            update_note(
                &mut app,
                &json!({"path": "u.md", "content": "x", "expected_hash": "abc"})
            )
            .is_err()
        );
        assert!(
            update_note(
                &mut app,
                &json!({"path": "gone.md", "content": "x", "expected_hash": "a".repeat(64)})
            )
            .is_err()
        );
    }

    #[test]
    fn set_property_adds_and_replaces_keys_only() {
        let text = "---\ntitle: Old\nmood: calm\n---\nbody text\n";
        let (dir, mut app) = vault_with("prop", &[("p.md", text)]);
        set_property(
            &mut app,
            &json!({"path": "p.md", "key": "title", "value": "New"}),
        )
        .expect("set");
        set_property(
            &mut app,
            &json!({"path": "p.md", "key": "tags", "value": ["a", "b"]}),
        )
        .expect("set");
        let got = fs::read_to_string(dir.path().join("p.md")).expect("file");
        assert!(got.contains("title: New"), "got {got:?}");
        assert!(got.contains("mood: calm"));
        assert!(got.ends_with("body text\n"));
        assert_eq!(
            frontmatter::get(&got, "tags"),
            Some(Fm::List(vec!["a".into(), "b".into()]))
        );
    }

    #[test]
    fn set_property_creates_frontmatter_when_missing() {
        let (dir, mut app) = vault_with("prop-new", &[("p.md", "plain")]);
        set_property(
            &mut app,
            &json!({"path": "p.md", "key": "done", "value": true}),
        )
        .expect("set");
        let got = fs::read_to_string(dir.path().join("p.md")).expect("file");
        assert!(got.starts_with("---\n"));
        assert_eq!(frontmatter::get(&got, "done"), Some(Fm::Bool(true)));
        assert!(got.ends_with("plain\n") || got.ends_with("plain"));
    }

    #[test]
    fn set_property_refuses_bad_keys_values_and_broken_frontmatter() {
        let (dir, mut app) = vault_with("prop-bad", &[("p.md", "---\nkey: [unclosed\n---\nx\n")]);
        assert!(set_property(&mut app, &json!({"path": "p.md", "key": "", "value": 1})).is_err());
        assert!(
            set_property(
                &mut app,
                &json!({"path": "p.md", "key": "a", "value": {"x": 1}})
            )
            .is_err()
        );
        assert!(
            set_property(
                &mut app,
                &json!({"path": "p.md", "key": "a", "value": [1, 2]})
            )
            .is_err()
        );
        assert!(set_property(&mut app, &json!({"path": "p.md", "key": "a"})).is_err());
        assert!(
            set_property(&mut app, &json!({"path": "p.md", "key": "a", "value": "v"})).is_err()
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("p.md")).expect("file"),
            "---\nkey: [unclosed\n---\nx\n"
        );
    }

    #[test]
    fn json_values_map_to_frontmatter() {
        assert_eq!(json_to_frontmatter(&json!(null)), Ok(Fm::Null));
        assert_eq!(json_to_frontmatter(&json!(2.5)), Ok(Fm::Number(2.5)));
        assert_eq!(
            json_to_frontmatter(&json!(["x"])),
            Ok(Fm::List(vec!["x".into()]))
        );
        assert!(json_to_frontmatter(&json!([1])).is_err());
    }

    #[test]
    fn append_to_daily_creates_the_note_from_the_template_then_appends() {
        let dir = TempDir::new("daily-write");
        write_file(
            dir.path(),
            ".jiyue/config.toml",
            "version = 1\n[daily]\nfolder = \"Diary\"\ndate_format = \"YYYY-MM-DD\"\n",
        );
        let mut app = app_for(dir.path(), false, false);
        let out = append_to_daily(
            &mut app,
            &json!({"text": "- met Ana", "date": "2026-11-04"}),
        )
        .expect("daily");
        assert!(out.starts_with("created and appended to the daily note Diary/2026-11-04.md"));
        let got = fs::read_to_string(dir.path().join("Diary/2026-11-04.md")).expect("file");
        assert!(got.starts_with("# 2026年11月4日"));
        assert!(got.contains("## 今日待办"));
        assert!(got.ends_with("- met Ana\n"));
        let again = append_to_daily(&mut app, &json!({"text": "second", "date": "2026-11-04"}))
            .expect("daily");
        assert!(again.starts_with("appended to the daily note"));
        assert!(
            fs::read_to_string(dir.path().join("Diary/2026-11-04.md"))
                .expect("file")
                .ends_with("second\n")
        );
    }

    #[test]
    fn append_to_daily_uses_a_custom_template_inside_the_vault() {
        let dir = TempDir::new("daily-template");
        write_file(dir.path(), "tpl.md", "Day {{date}} - {{title}}\n");
        write_file(
            dir.path(),
            ".jiyue/config.toml",
            "version = 1\n[daily]\nfolder = \"\"\ndate_format = \"YYYY-MM-DD\"\ntemplate_path = \"tpl.md\"\n",
        );
        let mut app = app_for(dir.path(), false, false);
        append_to_daily(&mut app, &json!({"text": "x", "date": "2026-01-02"})).expect("daily");
        let got = fs::read_to_string(dir.path().join("2026-01-02.md")).expect("file");
        assert!(
            got.starts_with("Day 2026-01-02 - 2026-01-02\n"),
            "got {got:?}"
        );
    }

    #[test]
    fn delete_note_moves_the_file_to_trash() {
        let (dir, mut app) = vault_with("delete", &[("gone.md", "bye")]);
        let out = delete_note(&mut app, &json!({"path": "gone.md"})).expect("delete");
        assert!(out.starts_with("moved gone.md to .trash/"));
        assert!(!dir.path().join("gone.md").exists());
        assert!(delete_note(&mut app, &json!({"path": "gone.md"})).is_err());
        app.catalog.refresh().expect("refresh");
        assert!(app.catalog.search("bye", 5).expect("search").is_empty());
    }

    #[test]
    fn delete_note_refuses_non_notes_and_traversal() {
        let (dir, mut app) = vault_with("delete-bad", &[]);
        write_file(dir.path(), "keep.png", "x");
        assert!(delete_note(&mut app, &json!({"path": "keep.png"})).is_err());
        assert!(delete_note(&mut app, &json!({"path": "../x.md"})).is_err());
        assert!(dir.path().join("keep.png").exists());
    }

    #[test]
    fn writes_keep_the_catalog_in_step_with_the_disk() {
        let (_dir, mut app) = vault_with("catalog-step", &[("a.md", "alpha")]);
        let hash = read_hash(&app, "a.md");
        update_note(
            &mut app,
            &json!({"path": "a.md", "content": "omega", "expected_hash": hash}),
        )
        .expect("update");
        app.catalog.refresh().expect("refresh");
        assert!(app.catalog.search("alpha", 5).expect("search").is_empty());
        assert_eq!(app.catalog.search("omega", 5).expect("search").len(), 1);
    }
}
