# FAQ

## General

**Where are my notes?**
In the vault folder you chose (`--vault <dir>`). Each note is a plain `.md` file in that folder or its sub-folders. Jiyue keeps its own data in a hidden `.jiyue` folder inside the vault.

**Which platforms are supported?**
Windows 10 and 11, 64-bit, is the primary platform and is tested by hand. macOS (Apple silicon) and Linux (x86_64) are built and tested by CI but have not been tried by hand yet. There is no mobile version. See [ci.md](ci.md).

**Does Jiyue need the internet?**
No. Everything works offline. The network is used only for sync (S3 or WebDAV) and AI (your own provider), and only when you turn them on.

**Does Jiyue collect telemetry or send crash reports?**
No. Crash information stays in `.jiyue\crash.log` in the vault.

**Can I change the keyboard shortcuts?**
Yes. Open Settings (Ctrl+,) and choose 快捷键. The defaults and the details are in [shortcuts.md](shortcuts.md).

**How do I export a note or the vault?**
Use the export commands in the command palette. A note becomes one HTML file; a vault becomes a folder of HTML pages. For PDF, open the HTML in a browser and print it to PDF.

## Migrating from Obsidian

**Can I use my Obsidian vault in Jiyue?**
Yes. Open the vault folder itself, with no conversion:

```powershell
jiyue.exe --vault "D:\Obsidian\MyVault"
```

Or create a shortcut whose target is `jiyue.exe` with the arguments `--vault "D:\Obsidian\MyVault"`.

**What happens to Obsidian's files?**
Jiyue does not read or change the `.obsidian` folder. It skips it when it scans the vault, and sync never uploads it. Obsidian settings (plugins, themes, hotkeys) do not carry over.

**Do my links, embeds and properties still work?**
Standard Obsidian syntax is supported: `[[wikilinks]]`, `![[embeds]]`, `^block` references and YAML frontmatter properties. Plugin-specific syntax is not interpreted.

**Will Obsidian and Jiyue conflict if I use both?**
They work on the same files, so do not edit the same note in both at the same time. Jiyue notices changes made on disk. Keep Obsidian's own sync (if any) and Jiyue's sync from writing the same vault.

**My attachments are in a different folder. What do I set?**
Jiyue has its own attachment setting (附件设置), stored in the `[attachments]` section of `.jiyue\config.toml`. It does not read Obsidian's setting. Choose the same folder in Jiyue's attachment settings if you want new pasted images to go to the same place.

**Is my Obsidian vault changed when I open it in Jiyue?**
Notes you edit in Jiyue are rewritten through a safe write (see below). Jiyue creates its `.jiyue` folder in the vault. Nothing else is touched unless you use a command that changes notes.

## Data safety

**How does Jiyue save files?**
A note is written to a temporary file in the same folder, flushed to disk, and then renamed over the old file. A failed save leaves the old file in place.

**What if the app crashes with unsaved text?**
While a note has unsaved changes, a copy is kept in `.jiyue\recovery`. On the next start the copy is offered back. Panics are also logged to `.jiyue\crash.log`.

**I deleted a note by mistake.**
Deleted notes go to `.trash` in the vault. Restore them from the trash view. Earlier versions of a note are in the version history (`.jiyue\history`), which you can browse and restore from.

**Is version history a backup?**
No. It is kept in the same vault folder, so it is lost with the vault. Keep a backup of the whole folder (for example with sync to S3 with versioning, or your own backup tool).

**Can I delete the `.jiyue` folder?**
Yes, but you lose the history, the recovery copies, the search index (it is rebuilt), the sync state (the next sync compares local and remote again and may create conflict copies), and the AI index (it is rebuilt when needed). Your notes are not in that folder.

## Search and links

**Search finds nothing for a new note.**
Search is updated in the background after changes. Wait a moment, or delete `.jiyue\index` to rebuild it (the next start re-reads all notes).

**What is the search syntax?**
Words, `"exact phrases"`, `-word` to exclude, `OR` between terms, and `tag:`, `path:`, `file:`, `title:` filters. Chinese text is indexed by character pairs, so no dictionary is needed.

**Backlinks show nothing.**
Backlinks are notes that link to the current note with `[[current note]]`. If a link is missing from the list, check the spelling of the link target.

## Sync and AI

**Where are my sync and AI passwords stored?**
In `credentials.toml` in your user configuration folder, in plain text and readable only by your user account. They are never written to the vault, never synced and never logged. To keep them out of the file entirely, write `env:NAME` or `cmd:COMMAND` as the value. See [configuration.md](configuration.md).

**Does sync delete files on the server?**
Sync propagates deletions. Local content that would be overwritten or deleted by a sync is moved to `.trash\sync` first. Use bucket versioning on S3 for an extra safety net. See [sync.md](sync.md).

**Does AI send my whole vault to the provider?**
No. Only what you send in a question or action is sent, and you see a preview before a question is sent. Folders and tags you exclude are never sent. See [ai.md](ai.md).

## Installer and program files

**Windows says the installer is from an unknown publisher.**
The installer is not code-signed yet. Signing needs a code-signing certificate. Until then, Windows SmartScreen may warn you.

**Where is the program installed?**
`%LOCALAPPDATA%\Programs\Jiyue`, for the current user only. No administrator rights are needed.

**Does uninstall delete my notes?**
No. The installer removes only program files, shortcuts, registry entries and the PATH entry it added. Your vault folders stay.

**"Open with Jiyue" does not open my note.**
Jiyue accepts a file path on the command line: `jiyue.exe "C:\path\to\note.md"`. The vault is the nearest parent folder that has a `.jiyue` or `.obsidian` folder, otherwise the file's own folder. A second launch opens a second window. Only `.md` files are indexed by a vault, so `.txt` and `.markdown` files are not opened.

**The `jiyue-mcp` command is not found in a terminal.**
Either install the optional "把 jiyue-mcp 加入 PATH" feature (see [installer/README.md](../installer/README.md)), or use the full path to `jiyue-mcp.exe`. Open a new terminal after installing, so it reads the updated PATH.

**Where is the MCP server documented?**
[mcp.md](mcp.md).
