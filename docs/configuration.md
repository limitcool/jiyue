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
| `config.toml` | user | Global settings: `[ui]`, `[appearance]`, and keyboard shortcuts (`[keys]`). 全局设置、字体与快捷键 |
| `state.toml` | user | Recent vaults, the last opened vault and the window position (`[window]`). 最近笔记库与窗口位置 |
| `credentials.toml` | user | API keys and sync secrets, plain text. 凭据（明文） |
| `.jiyue/config.toml` | vault | Vault settings: `[vault]`, `[editor]`, `[daily]`, `[templates]`, `[attachments]`, `[sync]`, `[ai]`. 笔记库设置 |
| `.jiyue/state.toml` | vault | Window layout (tabs, split) and search history. 界面状态 |
| `.jiyue/bookmarks.toml` | vault | Bookmarks. 书签 |

The sync engine's own data (`.jiyue/sync/`), version history, the search index and the vector index
are internal machine data and are not configuration.
同步引擎的数据（`.jiyue/sync/`）、版本历史、搜索索引和向量索引属于内部数据，不是配置。

## Example: `config.toml` (user) / 示例：用户级 config.toml

```toml
version = 1

# Interface preferences: saved when you change the theme, a sidebar or live preview.
# 界面偏好：切换主题、侧栏或实时预览时自动保存，下次启动时恢复。
[ui]
theme = "ink"                 # ink, fog, paper, moss, graphite / 主题
live_preview = true           # show notes rendered by default / 默认以渲染后的预览显示笔记
left_sidebar = true           # files, search, tags / 左侧栏
right_sidebar = true          # outline, backlinks / 右侧面板

# Fonts and sizes, for every vault. An empty font name = the built-in font.
# 字体与字号，对所有笔记库生效。字体名留空 = 内置字体。
[appearance]
ui_font = ""                  # menus, sidebars, labels / 界面字体
editor_font = ""              # note text and read mode; empty = ui_font / 正文字体
code_font = ""                # code blocks and inline code / 代码字体
editor_font_size = 15.0       # px, 10 to 36 / 正文字号
code_font_size = 0.0          # px, 9 to 32; 0 = 90 % of the text size / 代码字号
line_height = 1.73            # multiple of the font size, 1.0 to 2.5 / 行高

# Shortcut overrides: command id = ["chord", ...]. An empty list turns the shortcut off.
# 快捷键覆盖：命令编号 = ["快捷键"]，留空数组表示关闭该快捷键。
[keys]
"file.save" = ["ctrl-shift-s"]
"view.theme" = []
```

The command ids are listed in [shortcuts.md](shortcuts.md).

## Interface and fonts / 界面与字体

Most of these values can also be changed in the settings page (外观 page for fonts and sizes); the app
writes them to `config.toml`. Changes apply at once.
这些设置大多也可以在设置页修改（字体与字号在「外观」页），程序会把修改写入 `config.toml`，并立即生效。

**`[ui]`**

| Key | Default | Meaning / 含义 |
|---|---|---|
| `theme` | `"ink"` | `ink`, `fog`, `paper`, `moss` or `graphite`. An unknown name falls back to `ink`. / 未知名称使用 `ink` |
| `live_preview` | `true` | Notes open rendered, not as source. / 以渲染后的预览打开笔记 |
| `left_sidebar` | `true` | Left sidebar shown. / 显示左侧栏 |
| `right_sidebar` | `true` | Right panel (outline, backlinks) shown. / 显示右侧面板 |

Theme at startup, first match wins: the `--theme` command-line flag (when it names a known theme), then
the saved `theme`, then the default `ink`.
启动时主题的优先级：命令行 `--theme` 参数（须为已知主题）> 已保存的 `theme` > 默认 `ink`。

**`[appearance]`**

| Key | Default | Range | Meaning / 含义 |
|---|---|---|---|
| `ui_font` | `""` (Noto Sans SC) | installed font name | Menus, sidebars and labels. / 菜单、侧栏和标签 |
| `editor_font` | `""` (same as `ui_font`) | installed font name | Note text and read mode. / 正文和阅读模式 |
| `code_font` | `""` (JetBrainsMono Nerd Font) | installed font name | Code blocks and inline code. / 代码块和行内代码 |
| `editor_font_size` | `15.0` | 10 to 36 px | Note text size; headings scale with it. / 正文字号，标题按比例放大 |
| `code_font_size` | `0.0` | `0`, or 9 to 32 px | `0` follows the note text (90 %). / `0` 表示跟随正文（90%） |
| `line_height` | `1.73` | 1.0 to 2.5 | Line spacing as a multiple of the font size. / 行高，相对字号的倍数 |

Font names are matched without regard to case. A number outside its range is set to the nearest limit,
and the settings page says so. A font that is not installed is replaced by the built-in font for that
setting, and a warning is shown in the settings page (for example `未找到字体 X，已使用默认字体`).
字体名不区分大小写。超出范围的数字会调整到边界值，并在设置页提示。未安装的字体会换成该项的内置字体，并在设置页显示警告。

The sizes change the note text and code only. Menus, sidebars and other controls keep their size.
字号只作用于笔记正文和代码，菜单、侧栏和其他控件的大小不变。

An older file may still contain `ui_scale`, from a version that had an interface scale setting. It is no
longer used: it stays in the file and is reported as an unknown key, like any other unknown key.
旧版本文件中可能还有 `ui_scale`。该键已不再起作用，会保留在文件中，并和其他未知键一样给出警告。

## Example: `.jiyue/config.toml` (vault) / 示例：笔记库级 config.toml

```toml
version = 1

[vault]
# Display name in the sidebar, window title and recent list (up to 60 characters).
# Empty = the folder name. Renaming the folder keeps the display name.
# 显示名称（侧栏、窗口标题和最近列表）。留空 = 文件夹名。重命名文件夹后显示名称保持不变。
name = ""

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

## Example: `state.toml` (user) / 示例：用户级 state.toml

```toml
version = 1

# Written by the app; you rarely need to edit it. 由程序自动写入。
# Recent vaults, newest first, and the vault that was open when the app closed.
recent_vaults = ["D:\\Notes\\Work", "D:\\Notes\\Journal"]
last_vault = "D:\\Notes\\Work"

# Window position and size, written when the window closes. 关闭窗口时写入的位置和大小。
[window]
mode = "maximized"            # windowed, maximized or fullscreen / 窗口模式
x = 120.0                     # pixels / 像素
y = 80.0
width = 1280.0
height = 820.0
```

A saved window is restored at the next start only when it is at least 480 × 320 pixels and at least
100 pixels of it lie on a screen. Otherwise the app opens at its default size and position. `width = 0`
means that no position has been saved yet.
保存的窗口只有在尺寸不小于 480 × 320、且至少 100 像素位于屏幕内时才会在下次启动时恢复，否则使用默认尺寸和位置。`width = 0` 表示尚未保存过位置。

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
