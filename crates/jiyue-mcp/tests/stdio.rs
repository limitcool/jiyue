//! End-to-end tests: spawn the built `jiyue-mcp` binary and talk to it over stdio.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A vault folder for one test, removed on drop.
struct Vault {
    path: PathBuf,
}

impl Vault {
    fn new(label: &str, files: &[(&str, &str)]) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "jiyue-mcp-stdio-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create vault");
        for (rel, content) in files {
            let full = path.join(rel);
            fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
            fs::write(full, content).expect("write note");
        }
        Self { path }
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Runs the server with `args`, sends each request line, and returns the parsed responses.
fn session(args: &[&str], requests: &[String]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jiyue-mcp"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn jiyue-mcp");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for line in requests {
            writeln!(stdin, "{line}").expect("write request");
        }
    }
    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every stdout line is JSON"))
        .collect()
}

fn req(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn call(id: u64, name: &str, arguments: Value) -> String {
    req(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
}

fn response(responses: &[Value], id: u64) -> &Value {
    responses
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no response for id {id}: {responses:?}"))
}

fn tool_text(responses: &[Value], id: u64) -> String {
    response(responses, id)["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn is_error(responses: &[Value], id: u64) -> bool {
    response(responses, id)["result"]["isError"] == true
}

fn blake3_hex(path: &Path) -> String {
    let bytes = fs::read(path).expect("read for hash");
    blake3::hash(&bytes).to_hex().to_string()
}

#[test]
fn full_read_write_session_follows_the_protocol() {
    let vault = Vault::new(
        "rw",
        &[
            (
                "Home.md",
                "---\ntitle: Home\ntags: [start]\n---\n# Home\nWelcome #hello\n[[Ideas]]\n",
            ),
            ("Ideas.md", "# Ideas\n\n## Now\n- first\n"),
            ("pic.png", "not really a picture"),
        ],
    );
    let ideas_path = vault.path.join("Ideas.md");
    let ideas_hash = blake3_hex(&ideas_path);
    let vault_arg = vault.path.to_string_lossy().to_string();

    let requests = vec![
        req(
            1,
            "initialize",
            json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        ),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        req(2, "tools/list", json!({})),
        req(3, "ping", json!({})),
        call(4, "list_notes", json!({})),
        call(5, "read_note", json!({"path": "Home.md"})),
        call(6, "read_note", json!({"path": "../outside.md"})),
        call(7, "read_note", json!({"path": "/etc/passwd"})),
        call(8, "read_note", json!({"path": "C:/Windows/win.ini"})),
        call(9, "read_note", json!({"path": "pic.png"})),
        call(
            10,
            "read_note",
            json!({"path": "Ideas.md", "heading": "Now"}),
        ),
        call(
            11,
            "update_note",
            json!({"path": "Ideas.md", "content": "stale write", "expected_hash": "0".repeat(64)}),
        ),
        call(
            12,
            "update_note",
            json!({"path": "Ideas.md", "content": "# Ideas\nupdated\n", "expected_hash": ideas_hash}),
        ),
        call(13, "search_notes", json!({"query": "updated"})),
        call(14, "search_notes", json!({"query": "tag:start"})),
        call(15, "get_backlinks", json!({"path": "Ideas.md"})),
        call(16, "get_outgoing_links", json!({"path": "Home.md"})),
        call(17, "list_tags", json!({})),
        call(18, "notes_by_tag", json!({"tag": "start"})),
        call(19, "get_properties", json!({"path": "Home.md"})),
        call(
            20,
            "create_note",
            json!({"path": "Fresh/New.md", "content": "born today\r\n"}),
        ),
        call(
            21,
            "create_note",
            json!({"path": "Fresh/New.md", "content": "again"}),
        ),
        call(
            22,
            "append_to_note",
            json!({"path": "Fresh/New.md", "text": "more"}),
        ),
        call(
            23,
            "set_property",
            json!({"path": "Fresh/New.md", "key": "done", "value": true}),
        ),
        call(
            24,
            "append_to_daily",
            json!({"text": "- wrote tests", "date": "2026-11-04"}),
        ),
        call(25, "get_daily_note", json!({"date": "2026-11-04"})),
        call(26, "delete_note", json!({"path": "Fresh/New.md"})),
        call(
            27,
            "update_note",
            json!({"path": ".obsidian/app.md", "content": "x", "expected_hash": "0".repeat(64)}),
        ),
        call(28, "launch_rockets", json!({})),
        req(29, "resources/list", json!({})),
        req(
            30,
            "resources/read",
            json!({"uri": "jiyue://note/Ideas.md"}),
        ),
        req(
            31,
            "resources/read",
            json!({"uri": "jiyue://note/../../secret.md"}),
        ),
        req(32, "no/such/method", json!({})),
        "{this is not json".to_string(),
        req(33, "prompts/list", json!({})),
    ];

    let responses = session(&["--vault", &vault_arg], &requests);

    // Handshake and listing.
    assert_eq!(
        response(&responses, 1)["result"]["protocolVersion"],
        "2025-03-26"
    );
    assert_eq!(
        response(&responses, 1)["result"]["serverInfo"]["name"],
        "jiyue-mcp"
    );
    let tools = response(&responses, 2)["result"]["tools"]
        .as_array()
        .expect("tools")
        .clone();
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"update_note"));
    assert!(
        !names.contains(&"delete_note"),
        "delete is off without --allow-delete"
    );
    assert_eq!(response(&responses, 3)["result"], json!({}));

    // Reads.
    assert!(
        tool_text(&responses, 4).contains("\"total\": 2"),
        "pic.png is not a note"
    );
    assert!(tool_text(&responses, 5).contains("BEGIN NOTE CONTENT"));
    assert!(tool_text(&responses, 5).contains("Welcome #hello"));

    // Path boundary.
    for id in [6, 7, 8, 9] {
        assert!(is_error(&responses, id), "request {id} must be refused");
    }
    assert!(tool_text(&responses, 10).contains("- first"));
    assert!(!tool_text(&responses, 10).contains("# Ideas"));

    // Optimistic concurrency.
    assert!(is_error(&responses, 11));
    assert!(
        tool_text(&responses, 11).is_empty() || tool_text(&responses, 11).contains("hash mismatch")
    );
    assert!(!is_error(&responses, 12), "correct hash must be accepted");
    assert_eq!(
        fs::read_to_string(&ideas_path).expect("read"),
        "# Ideas\nupdated\n"
    );

    // Search, links, tags, properties.
    assert!(tool_text(&responses, 13).contains("Ideas.md"));
    assert!(tool_text(&responses, 14).contains("Home.md"));
    assert!(tool_text(&responses, 15).contains("Home.md"));
    assert!(tool_text(&responses, 16).contains("Ideas.md"));
    assert!(tool_text(&responses, 17).contains("hello"));
    assert!(tool_text(&responses, 18).contains("Home.md"));
    assert!(tool_text(&responses, 19).contains("\"title\": \"Home\""));

    // Writes.
    assert!(!is_error(&responses, 20));
    assert!(vault.path.join("Fresh/New.md").exists());
    assert!(
        is_error(&responses, 21),
        "create over an existing note is refused"
    );
    assert!(!is_error(&responses, 22));
    assert!(!is_error(&responses, 23));
    let fresh = fs::read_to_string(vault.path.join("Fresh/New.md")).expect("read");
    assert!(fresh.contains("born today\nmore\n"), "got {fresh:?}");
    assert!(fresh.contains("done: true"), "got {fresh:?}");
    assert!(!is_error(&responses, 24));
    assert!(vault.path.join("日记/2026-11-04.md").exists());
    assert!(tool_text(&responses, 25).contains("wrote tests"));

    // Delete is disabled without --allow-delete, and nothing is removed.
    assert!(is_error(&responses, 26));
    assert!(vault.path.join("Fresh/New.md").exists());

    // Ignored folder refused on write.
    assert!(is_error(&responses, 27));
    assert!(!vault.path.join(".obsidian").exists());

    // Unknown tool, resources, and protocol errors.
    assert_eq!(response(&responses, 28)["error"]["code"], -32602);
    let res_list = response(&responses, 29)["result"]["resources"]
        .as_array()
        .expect("list")
        .clone();
    assert!(res_list.iter().any(|r| r["uri"] == "jiyue://note/Ideas.md"));
    assert!(
        response(&responses, 30)["result"]["contents"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("updated"))
    );
    assert_eq!(response(&responses, 31)["error"]["code"], -32002);
    assert_eq!(response(&responses, 32)["error"]["code"], -32601);
    let parse_error = responses
        .iter()
        .find(|r| r["error"]["code"] == -32700)
        .expect("parse error reported");
    assert_eq!(parse_error["id"], Value::Null);
    assert_eq!(response(&responses, 33)["result"]["prompts"], json!([]));

    // 33 requests with ids and one garbage line (answered with a parse error). The
    // notification gets no response.
    assert_eq!(
        responses.len(),
        34,
        "one response per message that needs one"
    );
}

#[test]
fn read_only_mode_refuses_every_write() {
    let vault = Vault::new("ro", &[("a.md", "alpha")]);
    let vault_arg = vault.path.to_string_lossy().to_string();
    let requests = vec![
        req(1, "tools/list", json!({})),
        call(2, "create_note", json!({"path": "b.md", "content": "new"})),
        call(3, "append_to_note", json!({"path": "a.md", "text": "more"})),
        call(
            4,
            "update_note",
            json!({"path": "a.md", "content": "x", "expected_hash": "0".repeat(64)}),
        ),
        call(
            5,
            "set_property",
            json!({"path": "a.md", "key": "k", "value": "v"}),
        ),
        call(6, "append_to_daily", json!({"text": "x"})),
        call(7, "delete_note", json!({"path": "a.md"})),
        call(8, "read_note", json!({"path": "a.md"})),
    ];
    let responses = session(&["--vault", &vault_arg, "--read-only"], &requests);
    let names: Vec<&str> = response(&responses, 1)["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(names.len(), 9);
    for id in 2..=7 {
        assert!(
            is_error(&responses, id),
            "write request {id} must be refused"
        );
        assert!(tool_text(&responses, id).contains("read-only"));
    }
    assert!(tool_text(&responses, 8).contains("alpha"));
    assert_eq!(
        fs::read_to_string(vault.path.join("a.md")).expect("read"),
        "alpha"
    );
    assert!(!vault.path.join("b.md").exists());
}

#[test]
fn allow_delete_moves_the_note_to_trash() {
    let vault = Vault::new("del", &[("gone.md", "bye")]);
    let vault_arg = vault.path.to_string_lossy().to_string();
    let requests = vec![
        call(1, "delete_note", json!({"path": "gone.md"})),
        call(2, "delete_note", json!({"path": "../escape.md"})),
    ];
    let responses = session(&["--vault", &vault_arg, "--allow-delete"], &requests);
    assert!(!is_error(&responses, 1));
    assert!(tool_text(&responses, 1).contains(".trash/"));
    assert!(!vault.path.join("gone.md").exists());
    assert!(is_error(&responses, 2));
}

#[test]
fn read_only_and_allow_delete_together_is_a_usage_error() {
    let vault = Vault::new("usage", &[]);
    let vault_arg = vault.path.to_string_lossy().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_jiyue-mcp"))
        .args(["--vault", &vault_arg, "--read-only", "--allow-delete"])
        .stdin(Stdio::null())
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stdout.is_empty(),
        "no protocol output on usage errors"
    );
}

#[test]
fn missing_vault_exits_with_an_error_and_no_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_jiyue-mcp"))
        .args(["--vault", "Z:/definitely/not/a/vault/jiyue-mcp"])
        .stdin(Stdio::null())
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}
