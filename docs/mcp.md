# MCP server (jiyue-mcp)

`jiyue-mcp` is a local Model Context Protocol server. It runs on stdin and stdout, works on one vault, and lets an AI client list, search, read and (unless started read-only) edit your notes. It does not open a network port and does not call any AI provider itself.

## Command line

```text
jiyue-mcp --vault <dir> [--read-only] [--allow-delete] [--max-read-bytes N]
```

| Option | Meaning |
| --- | --- |
| `--vault <dir>` | Required. The vault folder. |
| `--read-only` | Disables every tool that writes. |
| `--allow-delete` | Adds the `delete_note` tool, which moves a note to `.trash`. Cannot be combined with `--read-only`. |
| `--max-read-bytes N` | Cap on note text per tool result or resource read. Default 262144 (256 KiB); allowed 1024 to 8388608. |
| `--help` | Prints the usage. |

Logs go to stderr; stdout carries protocol messages only.

Protocol versions accepted: `2025-06-18` (latest), `2025-03-26` and `2024-11-05`. The server announces `tools` and `resources` capabilities. Prompts are not offered (`prompts/list` returns an empty list).

## Tools

Read tools (always available):

| Tool | What it does |
| --- | --- |
| `list_notes` | Lists notes (path, title, size), optionally inside one folder. `limit` up to 1000, default 200. |
| `read_note` | Reads one note with its content hash. `heading` returns one section; `max_bytes` caps the text. |
| `search_notes` | Full-text search in Jiyue query syntax: words, `"phrases"`, `-exclusion`, `OR`, `tag:x`, `path:x`, `file:x`, `title:x`. `limit` up to 50, default 10. |
| `get_backlinks` | Notes that link to a note, with the number of links from each. |
| `get_outgoing_links` | Links and embeds a note contains, and the note each resolves to. |
| `list_tags` | Every tag with the number of notes that use it. |
| `notes_by_tag` | Notes that carry a tag (case-insensitive, without `#`). |
| `get_properties` | A note's frontmatter properties as JSON. |
| `get_daily_note` | The daily note for a date (default today). Says so when it does not exist. |

Write tools (not available with `--read-only`):

| Tool | What it does |
| --- | --- |
| `create_note` | Creates a note (`.md` or `.txt`). Fails if the file exists; missing folders are created. |
| `append_to_note` | Appends text at the end, or at the end of the section under a heading. |
| `update_note` | Replaces the content. Needs the hash from `read_note`; refused if the note changed since. |
| `set_property` | Sets one frontmatter property; the rest of the note is unchanged. |
| `append_to_daily` | Appends to the daily note for a date, creating it from the template if needed. |

Delete tool (only with `--allow-delete`):

| Tool | What it does |
| --- | --- |
| `delete_note` | Moves a note into `.trash`. Recoverable from the app. |

Paths are vault-relative (for example `folder/note.md`). Paths that leave the vault, and hidden folders such as `.obsidian` or `.jiyue`, are refused.

## Resources

- `resources/list` returns one resource per note: URI `jiyue://note/<vault-relative path>`.
- `resources/read` returns the note text for such a URI.

## Client configuration

Use the absolute path to `jiyue-mcp.exe`. Backslashes must be escaped in JSON.

### Claude Desktop

Edit `%APPDATA%\Claude\claude_desktop_config.json` (create it if it does not exist):

```json
{
  "mcpServers": {
    "jiyue": {
      "command": "C:\\Path\\To\\jiyue-mcp.exe",
      "args": ["--vault", "D:\\Notes", "--read-only"]
    }
  }
}
```

Restart Claude Desktop after changing the file. Remove `--read-only` only if you want the assistant to create and edit notes.

### Claude Code

```powershell
claude mcp add jiyue -- "C:\Path\To\jiyue-mcp.exe" --vault "D:\Notes" --read-only
```

Check with `claude mcp list`.

### Other stdio clients

Start the command `jiyue-mcp --vault <dir>` with the working directory of your choice. The server needs no environment variables and no network.

## Safety notes

- Start with `--read-only` until you trust the client.
- Writes go through the same atomic write as the app. `update_note` checks the hash, so an edit made in the app after `read_note` is not silently overwritten.
- An MCP client sees the text of the notes it reads. Do not give a client access to a vault you would not show it.
- The app and the server can work on the same vault at the same time. When the app sees a note change on disk, it reloads it or shows a conflict bar for a note with unsaved edits.
