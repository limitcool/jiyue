//! The default files that Jiyue writes. The comments live here and only here: every section and
//! key is described once. Each template must load back to the defaults of its structs, which a
//! test checks, so a comment and a default cannot drift apart.

/// `<config>/config.toml`: global settings and keyboard shortcuts.
pub const USER_CONFIG: &str = r#"version = 1

# 霁月的全局设置。修改后会自动重新载入，无需重启。
# 文件有语法错误时，程序保持默认值运行，并在设置页顶部显示错误位置。

# 快捷键覆盖：命令编号 = ["快捷键", ...]。留空数组可以关闭该命令的快捷键。
# 例如：save = ["ctrl-shift-y"]
# 快捷键的完整列表见 docs/shortcuts.md。

# 界面偏好：切换主题、侧栏或实时预览时自动保存，下次启动时恢复。
# 启动时的 --theme 参数优先于这里的 theme。
[ui]
# 主题：ink（墨）、fog（雾）、paper（纸）、moss（苔）、graphite（石墨）。
theme = "ink"
# 默认以渲染后的预览显示笔记（实时预览）。
live_preview = true
# 左侧栏（文件、搜索、标签）是否显示。
left_sidebar = true
# 右侧面板（大纲、反向链接）是否显示。
right_sidebar = true

# 字体与字号：对所有笔记库生效，修改后立即生效。字体名留空 = 内置字体。
# 字体名必须是系统中已安装的字体名（设置页的字体列表中可以选择）；找不到时使用内置字体并提示。
[appearance]
# 界面字体（菜单、侧栏、标签等）。留空 = Noto Sans SC。
ui_font = ""
# 正文字体（编辑区与阅读模式）。留空 = 与界面字体相同。
editor_font = ""
# 代码字体（代码块与行内代码）。留空 = JetBrainsMono Nerd Font。
code_font = ""
# 正文字号，单位 px，范围 10 到 36。
editor_font_size = 15.0
# 代码字号，单位 px，范围 9 到 32。填 0 表示跟随正文字号（正文的 90%）。
code_font_size = 0.0
# 行高（相对字号的倍数），范围 1.0 到 2.5。
line_height = 1.73

[keys]
"#;

/// `<config>/state.toml`: machine state, not meant for hand editing.
pub const USER_STATE: &str = r#"version = 1

# 霁月记住的状态（由程序自动写入）。
# recent_vaults: 最近打开的笔记库，最新的在前。
# last_vault: 上次关闭时打开的笔记库（空 = 无）。
recent_vaults = []
last_vault = ""
"#;

/// `<config>/credentials.toml`: secrets in plain text. Owner-only file, never inside a vault.
pub const CREDENTIALS: &str = r#"# 霁月的凭据文件。
# 凭据以明文保存在本文件中，仅当前用户可读。
# 不会写入笔记库，也不会同步。
#
# 每个值可以是三种形式之一：
#   "sk-..."                    直接写明文
#   "env:OPENAI_API_KEY"        运行时读取环境变量（变量未设置时报错）
#   "cmd:pass show openai"      运行命令并取其标准输出（5 秒超时，不经过 shell）
#
# [ai.<服务商编号>]：AI 服务商的 API 密钥（编号见笔记库的 [[ai.providers]]）。
# [sync.<笔记库编号>]：同步凭据（编号见笔记库 [sync] 中的 vault_id）。
version = 1
"#;

/// `<vault>/.jiyue/config.toml`: vault settings.
pub const VAULT_CONFIG: &str = r#"version = 1

# 本笔记库的设置。修改后会自动重新载入。
# 注意：本文件位于笔记库内。不要在这里写入密钥；密钥只放在用户目录的 credentials.toml 中。
# 如果你用 git 管理笔记库，请把 .jiyue/ 加入 .gitignore，或至少忽略本文件。

[vault]
# 笔记库的显示名称（侧栏、窗口标题和最近列表中显示）。留空 = 使用文件夹名。
name = ""

[editor]
# 新建笔记时，用第一个一级标题作为文件名（停止输入约 2 秒后自动改名）。
auto_name_from_heading = true

[daily]
# 日记所在的文件夹（相对笔记库根目录）。
folder = "日记"
# 日记标题和文件名的日期格式，如 YYYY-MM-DD。
date_format = "YYYY-MM-DD"
# 新建日记使用的模板文件（相对笔记库根目录）。留空 = 不使用模板。
template_path = ""

[templates]
# 模板所在的文件夹。
templates_folder = "模板"
# 按文件夹指定新建笔记时使用的模板。键为文件夹（"" 表示整个笔记库），值为模板文件路径。
# 例如：projects = "模板/项目.md"
[templates.folder_templates]

[attachments]
# 附件保存位置。可选值：
#   "VaultRoot"                      笔记库根目录
#   { Fixed = "附件" }               固定文件夹
#   "SameAsNote"                     与笔记同一文件夹
#   { SubfolderOfNote = "assets" }   笔记所在文件夹下的子文件夹
folder = { Fixed = "附件" }
# 链接写法："WikiEmbed" 为 ![[name.png]]，"Markdown" 为 ![](相对路径)。
link_style = "WikiEmbed"
# 粘贴的图片没有名字时使用的前缀，后面跟时间戳。
default_pasted_prefix = "Pasted image"

[sync]
# 本笔记库的编号（由程序生成）。同步凭据按此编号存放在 credentials.toml 的 [sync.<编号>] 中。
vault_id = ""
# 远端类型："s3" 或 "webdav"。
backend = "s3"
s3_endpoint = ""
s3_region = "us-east-1"
s3_bucket = ""
# 笔记库在桶中的前缀，如 jiyue/my-notes/。
s3_root_prefix = ""
# 访问密钥 ID、用户名与密钥/密码一样保存在 credentials.toml 的 [sync.<编号>] 中，不写在这里。
# 路径式寻址（endpoint/bucket/key），自建的 RustFS、MinIO 等通常需要开启。
s3_path_style = true
webdav_endpoint = ""
webdav_root = ""
# 总开关。关闭时不会同步，手动同步也会被拒绝。
enabled = false
auto_sync = false
# 自动同步的间隔："s30"、"m1"、"m5"、"m15"。
interval = "m5"
sync_on_save = false
save_debounce_secs = 10
sync_on_start = true
# 关闭窗口时同步一次（最多等待 8 秒）。
sync_on_exit = true
device_name = ""
# 额外的忽略规则，每行一条，与内置规则合并。
ignore_patterns = []

[ai]
# 是否启用 AI 功能。
enabled = false
# 对话和向量索引使用的服务商编号（见下方 [[ai.providers]]）。不写 = 不使用。
# active_chat = "openai"
# active_embedding = "openai"
# 这些文件夹中的笔记永远不会发送给模型（按路径前缀，整段匹配）。
excluded_folders = []
# 带这些标签（不带 #，不区分大小写）的笔记永远不会发送给模型。
excluded_tags = []

# 服务商列表。每个服务商的 API 密钥放在 credentials.toml 的 [ai.<id>] 中。
# kind 为 "open_ai_compatible" 或 "anthropic"。embedding_model 留空 = 不生成向量。
#[[ai.providers]]
#id = "openai"
#name = "OpenAI"
#kind = "open_ai_compatible"
#base_url = "https://api.openai.com/v1"
#chat_model = "gpt-4o-mini"
#embedding_model = "text-embedding-3-small"
"#;

/// `<vault>/.jiyue/state.toml`: vault UI state, written by the app.
pub const VAULT_STATE: &str = r#"version = 1

# 本笔记库的界面状态（由程序自动写入）：标签页、分栏、搜索历史、折叠的分组。
search_history = []
"#;

/// `<vault>/.jiyue/bookmarks.toml`: bookmarks, written by the app.
pub const BOOKMARKS: &str = r#"version = 1

# 书签（由程序自动写入）。
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_templates_load_to_the_defaults_of_their_structs() {
        use crate::config::{UserConfig, UserState};
        let config: crate::config::Loaded<UserConfig> = crate::config::doc::load_text_with(
            std::path::Path::new("config.toml"),
            USER_CONFIG,
            &["keys"],
        );
        assert!(config.error.is_none(), "{:?}", config.error);
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.value, UserConfig::default());
        let state: crate::config::Loaded<UserState> =
            crate::config::doc::load_text(std::path::Path::new("state.toml"), USER_STATE);
        assert!(state.error.is_none(), "{:?}", state.error);
        assert!(state.warnings.is_empty(), "{:?}", state.warnings);
        assert_eq!(state.value, UserState::default());
    }

    #[test]
    fn every_template_is_valid_toml_with_version_one() {
        for (name, text) in [
            ("user config", USER_CONFIG),
            ("user state", USER_STATE),
            ("credentials", CREDENTIALS),
            ("vault config", VAULT_CONFIG),
            ("vault state", VAULT_STATE),
            ("bookmarks", BOOKMARKS),
        ] {
            let table: toml::Table = toml::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                table.get("version").and_then(|v| v.as_integer()),
                Some(1),
                "{name}"
            );
        }
    }
}
