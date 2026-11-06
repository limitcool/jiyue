//! MCP tools: their JSON schemas and the dispatch that enforces `--read-only` and
//! `--allow-delete`.

pub mod read;
pub mod util;
pub mod write;

use serde_json::{Value, json};

use crate::app::App;
use crate::config::Config;
use util::cap_output;

/// Names of the tools that change notes.
pub const WRITE_TOOLS: [&str; 5] = [
    "create_note",
    "append_to_note",
    "update_note",
    "set_property",
    "append_to_daily",
];

/// Result of one tool call: text for the client and whether it is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub text: String,
    pub is_error: bool,
}

impl ToolOutput {
    fn ok(text: String) -> Self {
        Self {
            text,
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

/// The tools this server offers under `config`. Write tools are left out with `--read-only`,
/// and `delete_note` only appears with `--allow-delete`.
pub fn definitions(config: &Config) -> Vec<Value> {
    let mut out = vec![
        tool(
            "list_notes",
            "List notes in the vault (paths, titles, sizes), optionally inside one folder.",
            json!({
                "folder": {"type": "string", "description": "Folder relative to the vault root. Empty for the whole vault."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "description": "Maximum notes to return (default 200)."}
            }),
            &[],
            false,
        ),
        tool(
            "read_note",
            "Read one note with its content hash. Use heading to read a single section. The hash is needed by update_note.",
            json!({
                "path": {"type": "string", "description": "Vault-relative path such as 'folder/note.md'."},
                "heading": {"type": "string", "description": "Optional heading text; returns only that section."},
                "max_bytes": {"type": "integer", "minimum": 1, "description": "Optional cap on returned content bytes."}
            }),
            &["path"],
            false,
        ),
        tool(
            "search_notes",
            "Full-text search over all notes. Supports words, \"phrases\", -exclusion, OR, tag:x, path:x, file:x and title:x.",
            json!({
                "query": {"type": "string", "description": "Search query in Jiyue syntax."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": "Maximum hits (default 10)."}
            }),
            &["query"],
            false,
        ),
        tool(
            "get_backlinks",
            "List the notes that link to a note, with the number of links from each.",
            json!({"path": {"type": "string", "description": "Vault-relative path of the target note."}}),
            &["path"],
            false,
        ),
        tool(
            "get_outgoing_links",
            "List the links and embeds a note contains, with the note each one resolves to.",
            json!({"path": {"type": "string", "description": "Vault-relative path of the note."}}),
            &["path"],
            false,
        ),
        tool(
            "list_tags",
            "List every tag in the vault with the number of notes that use it.",
            json!({}),
            &[],
            false,
        ),
        tool(
            "notes_by_tag",
            "List the notes that carry a tag (case-insensitive, without the leading #).",
            json!({"tag": {"type": "string", "description": "Tag name, for example 'project'."}}),
            &["tag"],
            false,
        ),
        tool(
            "get_properties",
            "Read a note's frontmatter properties as JSON.",
            json!({"path": {"type": "string", "description": "Vault-relative path of the note."}}),
            &["path"],
            false,
        ),
        tool(
            "get_daily_note",
            "Read the daily note for a date (default today). Says so when it does not exist.",
            json!({"date": {"type": "string", "description": "Date as YYYY-MM-DD. Default: today."}}),
            &[],
            false,
        ),
    ];
    if !config.read_only {
        out.extend([
            tool(
                "create_note",
                "Create a new note. Fails if the file exists. Missing folders are created.",
                json!({
                    "path": {"type": "string", "description": "Vault-relative path ending in .md or .txt."},
                    "content": {"type": "string", "description": "Full text of the new note."}
                }),
                &["path", "content"],
                true,
            ),
            tool(
                "append_to_note",
                "Append text to a note: at the end, or at the end of the section under a heading.",
                json!({
                    "path": {"type": "string", "description": "Vault-relative path of an existing note."},
                    "text": {"type": "string", "description": "Text to append."},
                    "heading": {"type": "string", "description": "Optional heading text to append under."}
                }),
                &["path", "text"],
                true,
            ),
            tool(
                "update_note",
                "Replace a note's content. Requires the hash from read_note; refused if the note changed since.",
                json!({
                    "path": {"type": "string", "description": "Vault-relative path of an existing note."},
                    "content": {"type": "string", "description": "New full text of the note."},
                    "expected_hash": {"type": "string", "description": "64-character hash returned by read_note."}
                }),
                &["path", "content", "expected_hash"],
                true,
            ),
            tool(
                "set_property",
                "Set one frontmatter property. Other properties and the rest of the note are unchanged.",
                json!({
                    "path": {"type": "string", "description": "Vault-relative path of the note."},
                    "key": {"type": "string", "description": "Property name."},
                    "value": {"description": "String, number, boolean, null, or a list of strings."}
                }),
                &["path", "key", "value"],
                true,
            ),
            tool(
                "append_to_daily",
                "Append text to the daily note for a date (default today), creating it from the template if needed.",
                json!({
                    "text": {"type": "string", "description": "Text to append to the daily note."},
                    "date": {"type": "string", "description": "Date as YYYY-MM-DD. Default: today."}
                }),
                &["text"],
                true,
            ),
        ]);
        if config.allow_delete {
            out.push(tool(
                "delete_note",
                "Move a note into the vault's .trash folder. Recoverable from the app.",
                json!({"path": {"type": "string", "description": "Vault-relative path of the note."}}),
                &["path"],
                true,
            ));
        }
    }
    out
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str], write: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        },
        "annotations": {
            "readOnlyHint": !write,
            "openWorldHint": false
        }
    })
}

/// Runs a tool. `None` means no tool has that name. Disabled tools answer with an error result.
pub fn call(app: &mut App, name: &str, args: &Value) -> Option<ToolOutput> {
    let known = matches!(
        name,
        "list_notes"
            | "read_note"
            | "search_notes"
            | "get_backlinks"
            | "get_outgoing_links"
            | "list_tags"
            | "notes_by_tag"
            | "get_properties"
            | "get_daily_note"
            | "delete_note"
    ) || WRITE_TOOLS.contains(&name);
    if !known {
        return None;
    }
    if !args.is_object() && !args.is_null() {
        return Some(ToolOutput::error("arguments must be a JSON object"));
    }
    if (WRITE_TOOLS.contains(&name) || name == "delete_note") && app.config.read_only {
        return Some(ToolOutput::error(
            "refused: this server runs in --read-only mode, so note changes are disabled",
        ));
    }
    if name == "delete_note" && !app.config.allow_delete {
        return Some(ToolOutput::error(
            "refused: delete_note is disabled; start the server with --allow-delete to enable it",
        ));
    }

    let result = match name {
        "list_notes" => read::list_notes(app, args),
        "read_note" => read::read_note(app, args),
        "search_notes" => read::search_notes(app, args),
        "get_backlinks" => read::get_backlinks(app, args),
        "get_outgoing_links" => read::get_outgoing_links(app, args),
        "list_tags" => read::list_tags(app, args),
        "notes_by_tag" => read::notes_by_tag(app, args),
        "get_properties" => read::get_properties(app, args),
        "get_daily_note" => read::get_daily_note(app, args),
        "create_note" => write::create_note(app, args),
        "append_to_note" => write::append_to_note(app, args),
        "update_note" => write::update_note(app, args),
        "set_property" => write::set_property(app, args),
        "append_to_daily" => write::append_to_daily(app, args),
        "delete_note" => write::delete_note(app, args),
        _ => Err(format!("unknown tool: {name}")),
    };
    let max = app.config.max_read_bytes;
    Some(match result {
        Ok(text) => ToolOutput::ok(cap_output(text, max)),
        Err(message) => ToolOutput::error(message),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_MAX_READ_BYTES;
    use crate::testutil::{TempDir, app_for, config as test_config};

    fn names(defs: &[Value]) -> Vec<String> {
        defs.iter()
            .filter_map(|d| d["name"].as_str().map(String::from))
            .collect()
    }

    #[test]
    fn full_mode_lists_every_tool_except_delete() {
        let dir = TempDir::new("defs-full");
        let defs = definitions(&test_config(dir.path(), false, false));
        let got = names(&defs);
        assert_eq!(got.len(), 9 + 5);
        assert!(got.contains(&"update_note".to_string()));
        assert!(!got.contains(&"delete_note".to_string()));
    }

    #[test]
    fn read_only_mode_lists_only_read_tools() {
        let dir = TempDir::new("defs-ro");
        let defs = definitions(&test_config(dir.path(), true, false));
        let got = names(&defs);
        assert_eq!(got.len(), 9);
        assert!(got.iter().all(|n| !WRITE_TOOLS.contains(&n.as_str())));
        assert!(
            defs.iter()
                .all(|d| d["annotations"]["readOnlyHint"] == true)
        );
    }

    #[test]
    fn allow_delete_adds_delete_note() {
        let dir = TempDir::new("defs-del");
        let got = names(&definitions(&test_config(dir.path(), false, true)));
        assert!(got.contains(&"delete_note".to_string()));
        assert_eq!(got.len(), 15);
    }

    #[test]
    fn every_schema_is_an_object_with_required_fields_present() {
        let dir = TempDir::new("defs-schema");
        for def in definitions(&test_config(dir.path(), false, true)) {
            assert_eq!(def["inputSchema"]["type"], "object");
            let props = def["inputSchema"]["properties"].as_object().expect("props");
            let required = def["inputSchema"]["required"].as_array().expect("required");
            for r in required {
                let key = r.as_str().expect("name");
                assert!(props.contains_key(key), "{key} must be a property");
            }
            assert!(def["description"].as_str().is_some_and(|d| !d.is_empty()));
        }
    }

    #[test]
    fn unknown_tool_is_none_and_non_object_args_are_errors() {
        let dir = TempDir::new("dispatch-unknown");
        let mut app = app_for(dir.path(), false, false);
        assert!(call(&mut app, "nope", &json!({})).is_none());
        let out = call(&mut app, "list_notes", &json!("text")).expect("known");
        assert!(out.is_error);
    }

    #[test]
    fn write_tools_are_refused_in_read_only_mode() {
        let dir = TempDir::new("dispatch-ro");
        let mut app = app_for(dir.path(), true, false);
        for name in WRITE_TOOLS {
            let out = call(&mut app, name, &json!({})).expect("known");
            assert!(out.is_error, "{name} must be refused");
            assert!(out.text.contains("read-only"));
        }
        assert!(!dir.path().join("x.md").exists());
    }

    #[test]
    fn delete_is_refused_without_allow_delete() {
        let dir = TempDir::new("dispatch-nodel");
        let mut app = app_for(dir.path(), false, false);
        let out = call(&mut app, "delete_note", &json!({"path": "x.md"})).expect("known");
        assert!(out.is_error);
        assert!(out.text.contains("--allow-delete"));
    }

    #[test]
    fn tool_errors_are_flagged_and_successes_are_not() {
        let dir = TempDir::new("dispatch-flags");
        std::fs::write(dir.path().join("a.md"), "hello").expect("write");
        let mut app = app_for(dir.path(), false, false);
        let ok = call(&mut app, "read_note", &json!({"path": "a.md"})).expect("known");
        assert!(!ok.is_error);
        let bad = call(&mut app, "read_note", &json!({"path": "../a.md"})).expect("known");
        assert!(bad.is_error);
    }

    #[test]
    fn output_is_capped_to_the_configured_size() {
        let dir = TempDir::new("dispatch-cap");
        for i in 0..40 {
            std::fs::write(dir.path().join(format!("note-{i:02}.md")), "x").expect("write");
        }
        let mut cfg = test_config(dir.path(), false, false);
        cfg.max_read_bytes = 200;
        let mut app = App::open(cfg).expect("open");
        let out = call(&mut app, "list_notes", &json!({"limit": 40})).expect("known");
        assert!(out.text.contains("[output truncated"));
        assert!(out.text.len() < 400);
        assert_ne!(DEFAULT_MAX_READ_BYTES, 200);
    }
}
