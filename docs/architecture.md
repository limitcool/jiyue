# Architecture

Jiyue is a Cargo workspace at the repository root (`Cargo.toml`, crates in `crates/`). The workspace has one GUI binary, one MCP server binary, and library crates. The notes themselves are plain Markdown files; everything else is derived data that can be rebuilt.

## Crate map

| Crate | Kind | Role | Depends on |
| --- | --- | --- | --- |
| `jiyue-app` (package `jiyue`, binary `jiyue.exe`) | GPUI application | Window, editor, file tree, panels, dialogs, background jobs. Build script embeds the icon and version info. | core, search, sync, ai, md, history, export, math, diagram |
| `jiyue-mcp` (binary `jiyue-mcp.exe`) | stdio MCP server | Tools and `jiyue://note/` resources over one vault. | core, md, search |
| `jiyue-core` | library, no GUI | Vault and note library, link graph, file operations (`fsops`), file watching (`watch`), recovery journal (`recovery`), daily notes, text buffer and editing helpers. | none |
| `jiyue-md` | library | Markdown pieces used by the vault: embeds, block references, frontmatter, attachment paths. | none |
| `jiyue-search` | library | Full-text index (tantivy) with Chinese bigram tokenisation and the query syntax. | none |
| `jiyue-history` | library | Local version history: content-addressed blobs, an append-only index, retention rules. | none |
| `jiyue-sync` | library | Remote stores (S3 and WebDAV through opendal), sync planning, three-way merge, conflict copies, local sync state. | none |
| `jiyue-ai` | library | OpenAI-compatible and Anthropic clients, streaming chat, embeddings, vector index, retrieval, prompt builders, AI config and key storage. | none |
| `jiyue-export` | library | Note templates and HTML export of a note or a whole vault. | core, md |
| `jiyue-math` | library | LaTeX formulas to SVG (no browser). | none |
| `jiyue-diagram` | library | Mermaid to SVG, then rasterised for the preview. | none |
| `jiyue-query` | library | A "database over notes" experiment. It is part of the workspace but not used by `jiyue-app` or `jiyue-mcp`. | md |

## Data flow

```
              +-------------------- vault folder (Markdown files) --------------------+
              |                                                                       |
  user  ->  jiyue-app  ---- read/write notes (atomic) ---->  notes/*.md               |
    |           |                                                                     |
    |           +-- jiyue-core (library, links, watch, fsops, recovery journal)       |
    |           +-- jiyue-search  -> .jiyue/index/search                              |
    |           +-- jiyue-history -> .jiyue/history                                   |
    |           +-- jiyue-sync    -> .jiyue/sync/, S3 / WebDAV (only when configured) |
    |           +-- jiyue-ai      -> .jiyue/index, HTTPS to the provider (only when enabled)
    |           +-- jiyue-export  -> HTML files you choose                            |
    |                                                                                 |
  AI agent -> jiyue-mcp (stdio) -- core, md, search --> notes/*.md                  |
              +---------------------------------------------------------------------+
```

Key points:

- The app and the MCP server work on the same files. They do not share a process or a database. The MCP server keeps its own in-memory catalog (notes, search index, link graph) and refreshes it before each tool call by comparing file sizes and modification times, so edits made by the app or by other programs are picked up.
- The GUI never blocks on network or disk-wide scans. Sync, AI calls and index builds run on background threads (`jobs::spawn_blocking` in the app) and report back through a channel polled by the UI.
- Files written by the app are registered with the watch module, so the app does not report its own writes as external changes.

## Where the app's state lives

Everything the app writes is inside the vault's `.jiyue` folder, except the Mermaid font-metric cache, which goes to `%LOCALAPPDATA%\Jiyue\cache` (the app sets `XDG_CACHE_HOME` to that folder at start-up). Secrets (sync credentials and AI keys) are in `credentials.toml` in the user configuration folder, in plain text and owner-only; see [configuration.md](configuration.md). Settings are TOML files (`config.toml`, `state.toml`, and the vault's `.jiyue/config.toml`, `state.toml`, `bookmarks.toml`). The full folder list is in the README.

## Editor and views

- Each open note has one `TextEditor` (in `ui/text_input.rs`), kept by path. Autosave runs after a short delay; a recovery copy is written while text changes.
- View modes (`session.rs`): Source (editable), Live (rendered, click to edit), Read (rendered only).
- The rendered preview and the source highlighting are built from `pulldown-cmark` spans (`ui/md_style.rs`, `ui/live/`), with formulas and Mermaid diagrams rendered by `jiyue-math` and `jiyue-diagram`.

## Build outputs

- `cargo build --release -p jiyue` produces `jiyue.exe`; `cargo build --release -p jiyue-mcp` produces `jiyue-mcp.exe`.
- The installer is built by `installer/build.ps1` (see `installer/README.md`).
