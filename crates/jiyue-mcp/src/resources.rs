//! MCP resources: every note as `jiyue://note/<vault-relative path>`.

use serde_json::{Value, json};

use crate::app::App;
use crate::tools::util::cap_output;
use crate::vault::{mime_for, read_note_text};

/// URI prefix of note resources.
pub const NOTE_URI_PREFIX: &str = "jiyue://note/";
/// Most recently modified notes returned by `resources/list`.
pub const LIST_LIMIT: usize = 500;

/// `resources/list`: the newest notes first, at most [`LIST_LIMIT`].
pub fn list(app: &mut App) -> Result<Value, String> {
    app.catalog.refresh()?;
    let mut metas: Vec<_> = app.catalog.notes().collect();
    metas.sort_by(|a, b| {
        b.mtime_ms
            .cmp(&a.mtime_ms)
            .then_with(|| a.path.cmp(&b.path))
    });
    let resources: Vec<Value> = metas
        .into_iter()
        .take(LIST_LIMIT)
        .map(|meta| {
            json!({
                "uri": format!("{NOTE_URI_PREFIX}{}", meta.path),
                "name": meta.path,
                "title": meta.title,
                "mimeType": mime_for(&meta.path),
            })
        })
        .collect();
    Ok(json!({"resources": resources}))
}

/// `resources/read`: the text of one note. Errors describe why the URI cannot be served.
pub fn read(app: &mut App, uri: &str) -> Result<Value, String> {
    let rel = uri
        .strip_prefix(NOTE_URI_PREFIX)
        .ok_or_else(|| format!("unsupported resource URI: {uri}"))?;
    let abs = app.root.resolve_note(rel)?;
    let text = read_note_text(&abs, rel)?;
    let text = cap_output(text, app.config.max_read_bytes);
    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": mime_for(rel),
            "text": text,
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::vault_with;
    use serde_json::json;

    #[test]
    fn list_offers_one_resource_per_note_with_the_jiyue_uri() {
        let (_dir, mut app) = vault_with("res-list", &[("a.md", "x"), ("dir/b.txt", "y")]);
        let out = list(&mut app).expect("list");
        let uris: Vec<&str> = out["resources"]
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|r| r["uri"].as_str())
            .collect();
        // Newest first: the order follows modification times, so compare as sets.
        let mut sorted = uris.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec!["jiyue://note/a.md", "jiyue://note/dir/b.txt"]);
        let txt = out["resources"]
            .as_array()
            .and_then(|list| list.iter().find(|r| r["name"] == "dir/b.txt"));
        assert_eq!(
            txt.map(|r| r["mimeType"].clone()),
            Some(json!("text/plain"))
        );
    }

    #[test]
    fn list_is_capped_at_the_limit() {
        let files: Vec<(String, String)> = (0..LIST_LIMIT + 3)
            .map(|i| (format!("n{i:04}.md"), "x".to_string()))
            .collect();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        let (_dir, mut app) = vault_with("res-cap", &refs);
        let out = list(&mut app).expect("list");
        assert_eq!(out["resources"].as_array().map(Vec::len), Some(LIST_LIMIT));
    }

    #[test]
    fn read_returns_the_note_text() {
        let (_dir, mut app) = vault_with("res-read", &[("a.md", "# Hello\nworld")]);
        let out = read(&mut app, "jiyue://note/a.md").expect("read");
        assert_eq!(out["contents"][0]["text"], "# Hello\nworld");
        assert_eq!(out["contents"][0]["mimeType"], "text/markdown");
        assert_eq!(out["contents"][0]["uri"], "jiyue://note/a.md");
    }

    #[test]
    fn read_refuses_other_schemes_traversal_and_missing_notes() {
        let (_dir, mut app) = vault_with("res-bad", &[("a.md", "x")]);
        assert!(read(&mut app, "file:///etc/passwd").is_err());
        assert!(read(&mut app, "jiyue://note/../secret.md").is_err());
        assert!(read(&mut app, "jiyue://note/missing.md").is_err());
        assert!(read(&mut app, "jiyue://note/.obsidian/x.md").is_err());
    }
}
