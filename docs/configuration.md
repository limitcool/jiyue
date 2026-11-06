# Configuration / 配置

Jiyue keeps its settings in TOML files. Settings are either **user-level** (one set for the whole
machine) or **vault-level** (they travel with the notes, inside the vault's `.jiyue` folder).
霁月的设置保存在 TOML 文件中，分为两层：**用户级**（整台电脑共用）和**笔记库级**（位于笔记库的 `.jiyue` 文件夹内）。

Every file starts with `version = 1`. Missing keys take their defaults. Unknown keys are kept on save
and reported as warnings. Comments you write by hand are kept when the app saves the file.
每个文件都以 `version = 1` 开头。缺少的键使用默认值；未知的键会保留并给出警告；程序保存文件时会保留你手写的注释。

## File locations / 文件位置

| Level | Windows | macOS | Linux |
|---|---|---|---|
| User folder (user level) | `%LOCALAPPDATA%\Jiyue` | `~/Library/Application Support/Jiyue` | `$XDG_CONFIG_HOME/jiyue` or `~/.config/jiyue` |

Set the environment variable `JIYUE_CONFIG_DIR` to another folder to use that folder as the user
folder instead (used by tests and developer scripts).
设置环境变量 `JIYUE_CONFIG_DIR` 可以把用户文件夹指向其他位置（测试和开发脚本使用）。

| File | Level | Purpose / 用途 |
|---|---|---|
| `config.toml` | user | Global settings and keyboard shortcuts (`[keys]`). 全局设置与快捷键 |
| `state.toml` | user | Recent vaults and the last opened vault. 最近笔记库 |
| `credentials.toml` | user | API keys and sync secrets, plain text. 凭据（明文） |
| `.jiyue/config.toml` | vault | Vault settings: `[editor]`, `[daily]`, `[templates]`, `[attachments]`, `[sync]`, `[ai]`. 笔记库设置 |
| `.jiyue/state.toml` | vault | Window layout (tabs, split) and search history. 界面状态 |
| `.jiyue/bookmarks.toml` | vault | Bookmarks. 书签 |

The sync engine's own data (`.jiyue/sync/`), version history, the search index and the vector index
are internal machine data and are not configuration.
同步引擎的数据（`.jiyue/sync/`）、版本历史、搜索索引和向量索引属于内部数据，不是配置。

## Example: `config.toml` (user) / 示例：用户级 config.toml

```toml
version = 1

# Shortcut overrides: command id = ["chord", ...]. An empty list turns the shortcut off.
# 快捷键覆盖：命令编号 = ["快捷键"]，留空数组表示关闭该快捷键。
[keys]
"file.save" = ["ctrl-shift-s"]
"view.theme" = []
```

The command ids are listed in [shortcuts.md](shortcuts.md).

## Example: `.jiyue/config.toml` (vault) / 示例：笔记库级 config.toml

```toml
version = 1

[editor]
# Name a new note after its first heading. 新建笔记时用首个标题作为文件名。
auto_name_from_heading = true

[daily]
folder = "日记"
date_format = "YYYY-MM-DD"
template_path = ""            # empty = built-in template / 空 = 内置模板

[attachments]
folder = { Fixed = "附件" }   # or "VaultRoot", "SameAsNote", { SubfolderOfNote = "assets" }
link_style = "WikiEmbed"      # or "Markdown"

[sync]
backend = "s3"                # or "webdav"
s3_endpoint = "https://s3.example.com"
s3_region = "us-east-1"
s3_bucket = "notes"
s3_root_prefix = "jiyue/my-notes/"
s3_path_style = true
enabled = true
auto_sync = true
interval = "m5"               # "s30", "m1", "m5", "m15"

[ai]
enabled = true
active_chat = "openai"
# active_embedding = "openai"
excluded_folders = ["private"]
excluded_tags = ["secret"]

[[ai.providers]]
id = "openai"
name = "OpenAI"
kind = "open_ai_compatible"
base_url = "https://api.openai.com/v1"
chat_model = "gpt-4o-mini"
# embedding_model = "text-embedding-3-small"
```

## Example: `credentials.toml` / 示例：credentials.toml

```toml
version = 1

# Plain text, readable by your user only. Never stored in a vault.
# 明文保存，仅当前用户可读，绝不写入笔记库。
[ai.openai]
api_key = "env:OPENAI_API_KEY"        # read from the environment when used / 使用时读取环境变量

[sync.k3j9x2a1]                        # the vault_id from [sync] in the vault's config.toml
access_key_id = "AKIA..."              # S3 access key id (WebDAV: leave empty)
secret_access_key = "cmd:pass show jiyue/s3"
username = ""                          # WebDAV user name (S3: leave empty)
password = ""                          # WebDAV password
```

## Secret values: three forms / 密钥的三种写法

| Form | Meaning |
|---|---|
| `"sk-..."` | A literal value, stored in the file. 明文 |
| `"env:NAME"` | Read the environment variable `NAME` each time it is needed. If it is not set, the app reports it. 运行时读取环境变量 |
| `"cmd:COMMAND"` | Run `COMMAND` without a shell and use its trimmed standard output. Timeout 5 seconds. The command is taken from your own file; the output is never logged. 运行命令取输出（5 秒超时，不经过 shell） |

Use `env:` or `cmd:` (for example with a password manager's CLI) to keep the secret out of the file.
推荐用 `env:` 或 `cmd:`（例如密码管理器的命令行工具），让密钥不落在文件里。

## Security / 安全说明

- **Plain text.** `credentials.toml` holds secrets without encryption. Anyone who can read the file
  as your user account can read the keys. This is a deliberate trade-off: no OS keychain, no
  database, nothing to unlock. 文件是明文的，能以你的账户读取文件的人就能读到密钥；这是有意的取舍。
- **Permissions.** The file is created owner-only: mode 0600 on macOS and Linux; on Windows an
  `icacls` call removes inherited access and grants only your user. If that fails, the settings page
  shows a warning. A file that other users can read is reported when it is loaded.
  文件权限：macOS/Linux 为 0600；Windows 通过 icacls 只授予当前用户。失败时设置页会提示。
- **Never in a vault.** The app refuses to use a user folder that sits inside an open vault.
  笔记库内的文件夹不能作为用户文件夹。
- **Never synced or exported.** Secrets are not written to the vault, the sync data, the recovery
  journal, the crash log, exports, MCP output or screenshots.
- **Keep vaults out of git, or ignore `.jiyue/`.** If you keep a vault in a repository, add this to
  its `.gitignore` so the vault-level settings (including sync endpoints) are not committed:
  如果笔记库放在 git 中，请在 `.gitignore` 中加入：

  ```gitignore
  .jiyue/
  ```

## Errors and resetting / 错误与重置

- A file with a syntax or value error is **not** used and **not** overwritten. The app keeps running
  on defaults, shows an error toast, and shows a banner at the top of the settings page with the file,
  line, column and key. Use **打开配置文件** to fix it. The file is reloaded automatically when it is
  saved (a toast says 配置已重新载入).
  出错的文件不会被使用，也不会被覆盖。程序使用默认值运行，设置页顶部显示文件、行、列和键名。修正后自动重新载入。
- **重置为默认** renames the broken file to `<name>.bad-<unix time>` (nothing is lost) and writes a new
  default file with comments in its place.
  **重置为默认** 会把损坏的文件改名为 `<名称>.bad-<时间戳>`，并写入带注释的默认文件。
- To reset everything, quit the app and delete the user folder (the vault's own files are untouched).
  要全部重置，退出程序后删除用户文件夹（笔记库内的文件不受影响）。

## Editing / 编辑

Edit the files with any editor, or use the settings pages. The app watches the files: an external
change is reloaded within a few seconds. Keep the file encoding UTF-8.
可以用任何编辑器修改，也可以用设置页修改。程序会检查文件变化，几秒内自动重新载入。文件请使用 UTF-8 编码。
