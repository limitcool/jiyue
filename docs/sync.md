# Sync (S3 and WebDAV)

Sync copies one vault to a remote folder so you can use it on another computer. It is optional and off by default. Jiyue talks to the remote directly from your machine; there is no Jiyue server.

## What is synced

- Notes and attachments inside the vault folder.
- Not synced: `.jiyue/`, `.trash/`, `.git/`, and the patterns you add in the ignore list of the sync settings. Sync also ignores the workspace and cache files of Obsidian (`.obsidian/workspace*.json`, `.obsidian/cache`) and `.DS_Store`.
- Sync state (what was last synced, the common ancestor of each note, the list of conflicts, the device name) is kept locally in `.jiyue/sync/`. Deleting that folder is safe; the next sync compares local and remote again and may create conflict copies.

## Setting up S3

Open the settings, category 同步, choose S3, and fill in:

| Field | What to enter |
| --- | --- |
| 服务器地址 (endpoint) | `https://` or `http://` address. For RustFS or MinIO, the server's own address. For AWS, the regional endpoint. |
| 区域 (region) | The bucket's region. S3-compatible servers usually accept `us-east-1`. |
| 存储桶 (bucket) | An existing bucket. Jiyue does not create buckets. |
| 路径前缀 (root prefix) | The folder in the bucket that holds this vault. Empty means `jiyue/<vault name>/`. |
| Access Key ID | Your access key ID. |
| Secret Access Key | Stored in plain text in `credentials.toml` in your user configuration folder (owner-only, never in the vault). You can use `env:NAME` or `cmd:COMMAND` instead. |
| 路径风格寻址 (path-style) | On by default. Keep it on for MinIO, RustFS and most self-hosted servers. |

Use "test connection" in the settings before enabling sync. The check reports whether the bucket can be written and deleted, whether versioning is on, and whether conditional writes are supported.

Use a dedicated bucket or a dedicated prefix with a key that only has access to it.

## Setting up WebDAV

| Field | What to enter |
| --- | --- |
| 服务器地址 | The WebDAV root URL, for example `https://dav.example.com/dav`. |
| 用户名 / 密码 | The WebDAV account. The password is stored in plain text in `credentials.toml` (see [configuration.md](configuration.md)). |
| 远端目录 (remote folder) | The folder on the server that holds this vault. |

WebDAV servers differ in what they support; the connection check shows what is available. WebDAV has no version history from Jiyue's side.

## Versioned bucket (S3 only)

Remote version history and restore depend on **S3 bucket versioning**, which you turn on in your storage provider's console or with the server's tools.

- Jiyue checks whether versioning is enabled. If it is not, sync still works; only the remote version history is unavailable.
- Jiyue never changes the bucket's settings, including versioning.
- Versioning keeps the older copies of a note when it is overwritten or deleted on the remote. With it on, the app's remote history view for the open note lists those versions, shows one read-only with a diff against the editor text, and can restore it into the editor as one undoable edit (the restored text is uploaded by the next sync).
- Turning versioning on applies to new versions only; objects written before that have no earlier versions.

## When sync runs

Sync is never run without your setting. The options in the 同步 category:

- Manual: the sync button in the app.
- 定时同步 (interval): every 30 seconds, 1 minute, 5 minutes (default) or 15 minutes.
- 保存后同步 (after save): waits 10 seconds, 30 seconds or 1 minute after the last save.
- 启动时同步 (on start, default on): once when the vault opens.
- 退出时同步 (on exit, default on): once before the window closes; waits at most 8 seconds.

## How changes are combined

Each sync compares local files, remote objects and the last synced state:

- A file changed on one side only is copied to the other side.
- A file changed on both sides is merged line by line with a three-way merge, using the last synced copy as the common ancestor. If the merge is clean, the result is written to both sides.
- Overlapping edits make a **conflict copy** next to the note, named like `note (冲突副本 <device> 20261104-1530).md`. Both versions are kept; the conflict list in the sync settings shows them, and you choose what to keep. Binary files keep both versions.
- Content that a sync would overwrite or delete locally is moved to `.trash/sync` first, so it can be found again.
- Conditional writes (`If-Match`, `If-None-Match`) are used when the server supports them, so a file changed by another device during a run is re-planned rather than overwritten.

## Limits and status

- Real-world validation with WebDAV servers and with sync conflicts is still on the project's list (see the README roadmap). Test with a copy of your vault first.
- Sync is file based. It does not merge at the level of blocks or live cursors.
- The roadmap (README) lists reloading open notes after a sync as not finished.
