# 更新日志 · Changelog

版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。每个版本的说明同时用中文和英文书写。
Versions follow [Semantic Versioning](https://semver.org/). Every entry is written in Chinese and English.

## [0.2.0] - 2026-11-07

### 中文

**新增**
- **自动检查更新系统**：在“关于”页面支持检测 GitHub 最新版本，显示版本号、更新日志与各平台安装包（Windows MSI、macOS DMG、Linux AppImage），支持 SHA-256 校验；内置智能回退机制，在遇到 GitHub API 403 匿名访问限流时自动从 Releases 网页兜底解析，确保更新检测始终可用。
- **全新欢迎引导页**：在首次打开笔记库或未打开任何笔记时展示优雅的欢迎页，提供“新建笔记”、“快速切换”、“每日笔记”、“知识图谱”等核心快捷入口，并展示最近访问的笔记卡片与常用快捷键指引。
- **全量多语言国际化（i18n）**：基于 Fluent 引擎支持简体中文（zh-CN）与英语（en-US）实时热切换，完整覆盖编辑器、外观、每日笔记、同步、AI、快捷键、笔记库与关于等全部设置面板、操作按钮与输入框占位符；设置弹窗左侧导航栏底部新增常驻中英快速切换条，语言偏好持久化保存。
- **主题明暗模式设置**：外观面板支持将主题指定为“跟随主题”、“暗色 (Dark)”或“浅色 (Light)”模式，并在切换后即时刷新界面且落盘保存。

**优化与修复**
- **分屏（Split View）分割线与交互重构**：修复左右/上下分屏时中间没有清晰分割线的视觉缺陷；重构为自适应主题色的 1px 贯通分割线与 6px 舒适鼠标拖拽命中区，并新增鼠标悬浮与拖拽时的主题强调色高亮反馈；顶栏标签组同步加入对齐分隔线。
- **下拉菜单交互与宽度修复**：修复设置页中字体选择等下拉菜单宽度对齐问题，并支持点击下拉菜单外部区域直接关闭。
- **设置面板 UI 细节排版**：修复设置页单选按钮组、开关、文本输入框在不同语言与缩放下的对齐与挤压问题。

---

### English

**Added**
- **Automatic update checking**: Check for the latest releases from GitHub directly in the About page, showing version details, changelogs, and platform-specific installers (Windows MSI, macOS DMG, Linux AppImage) with SHA-256 checksums; includes a resilient fallback parser that gracefully recovers from GitHub API HTTP 403 anonymous rate limits.
- **Welcome onboarding page**: A clean and helpful welcome screen displayed on empty states or initial launch, featuring quick actions (New Note, Quick Switcher, Daily Notes, Knowledge Graph), recent note cards, and keyboard shortcut tips.
- **Comprehensive internationalization (i18n)**：Full bilingual support (Simplified Chinese & English) powered by Fluent with instant live-reloading across all settings dialogs (Editor, Appearance, Daily Notes, Sync, AI, Shortcuts, Vaults, About), control buttons, and input placeholders; persistent locale settings and a quick language toggle in the settings sidebar.
- **Theme mode preference**: Explicitly configure appearance to match the theme default, Force Dark, or Force Light, with instant UI refresh and persistent storage.

**Improved & Fixed**
- **Split view divider & interaction overhaul**: Fixed the missing visual divider between panes; engineered a 1px theme-adaptive visible line centered inside a 6px comfortable drag hit target with dynamic hover/drag accent highlighting; synchronized tab strips with aligned dividers.
- **Dropdown menus & layout fixes**: Fixed dropdown width alignment (e.g. font pickers) and added click-outside-to-dismiss behavior.
- **Settings UI refinements**: Fixed flex-shrink squishing and height alignment on radio buttons, toggles, and inputs across languages.

## [0.1.1] - 2026-11-06

感谢 ZoftTy 在 [#1](https://github.com/limitcool/jiyue/issues/1) 中提交的详细反馈，这个版本修复了其中的全部问题。
Thanks to ZoftTy for the detailed report in [#1](https://github.com/limitcool/jiyue/issues/1); this release fixes everything in it.

### 中文

**新增**
- 字体与字号设置：界面字体、正文字体、代码字体，正文字号、代码字号和行高，带实时预览，改动立即生效；找不到的字体会自动回退并提示。
- 鼠标前进、后退键，以及 `Alt+←`、`Alt+→`，在访问过的笔记之间导航。
- 文件树支持 `Shift`、`Ctrl` 多选，并可对选中的文件批量移动、删除到回收站、加入书签、复制路径，也可以整体拖动。
- 点击侧栏里的标签会搜索该标签，按住 `Ctrl` 点击可以叠加多个标签。
- 笔记库可以重命名：显示名称，以及磁盘上的文件夹名称。
- 属性的名称现在可以修改。
- 一篇笔记可以添加多个标签：回车连续添加，逗号或空格一次添加多个，空输入框按退格删除最后一个。
- 知识图谱支持鼠标滚轮缩放，双击空白处恢复适应窗口。
- 窗口大小和位置、侧栏显示状态会被记住。
- 笔记里的 `[文字](网址)` 链接和直接写出的 http、https、mailto 地址可以打开，用系统浏览器打开：阅读模式下直接点击；实时预览和源码模式下按住 `Ctrl`（macOS 为 `Cmd`）点击。

**修复**
- 主题每次退出后都会恢复成默认值，现在会被保存；侧栏显示、实时预览开关也一并修复。保存某一部分配置时不再丢掉其他设置。
- 历史版本之间切换时不再闪烁。
- 统一了圆角、图标间距和输入框获得焦点时的样式，弹窗和设置页的圆角不再被直角背景盖住。
- 标签页很多时，标签栏会自动滚动到当前标签，两端有箭头按钮。
- 点击知识图谱中的节点，现在在新标签页打开（`Ctrl` 点击或中键在后台标签页打开）。
- 知识图谱右下角的节点数和连接数现在按当前筛选后实际显示的内容计算，不再重复计数。
- 阅读模式下鼠标移到链接上会变成手形指针。
- 批量移动文件现在作为一次操作撤销，撤销会还原全部项目。

### English

**Added**
- Font settings: interface font, body font and code font, body size, code size and line height, with a live preview. Changes apply at once, and a font that is not installed falls back to the default with a note.
- Mouse back and forward buttons, and `Alt+Left` / `Alt+Right`, to move between visited notes.
- Multi-select in the file tree with `Shift` and `Ctrl`, and batch move, delete to the trash, bookmark and copy path on the selection. A selection can also be dragged as a whole.
- Clicking a tag in the sidebar searches for it; `Ctrl`+click adds the tag to the current search.
- A vault can be renamed: its display name, and its folder on disk.
- Property names can now be renamed.
- A note can have several tags: Enter adds one after another, commas or spaces add several at once, and Backspace in an empty field removes the last one.
- The knowledge graph zooms with the mouse wheel; double-click on empty space fits it to the window.
- The window size and position and the sidebar visibility are remembered.
- Links in notes can be opened: `[text](url)` links and bare http, https or mailto addresses open in the system browser. In reading mode a click opens them; in live preview and source mode, hold `Ctrl` (`Cmd` on macOS) and click.

**Fixed**
- The theme went back to the default every time the app quit. It is now saved, together with the sidebar and live preview switches. Saving one part of the configuration no longer drops other settings.
- No more flicker when switching between versions in the history dialog.
- Consistent corners, icon spacing and focus style for inputs; dialogs and the settings page no longer have square backgrounds showing through rounded corners.
- With many tabs open, the tab strip scrolls to the active tab, with arrow buttons at the ends.
- A node clicked in the knowledge graph now opens in a new tab (`Ctrl`+click or middle click opens it in a background tab).
- The node and link counts at the bottom right of the graph now match what is shown after filters, without double counting.
- In reading mode the pointer becomes a hand over links.
- A batch move is one undo step: undo puts back every moved item.

## [0.1.0] - 2026-11-06

### 中文

第一个版本。
- 本地优先的 Markdown 笔记：笔记就是你选的文件夹里的普通 `.md` 文件，兼容 Obsidian 笔记库。
- 实时预览：光标所在的内容显示源码，其余渲染；支持表格、提示块、代码块、图片和笔记嵌入、公式、Mermaid 图表、脚注；另有阅读模式和源码模式。
- 双链、反向链接、知识图谱、标签、属性、每日笔记、模板、书签、标签页和分屏、`/` 插入菜单。
- 全文搜索（支持中文）、本地历史版本、导出 HTML。
- 可选的 S3 同步（三方合并、冲突副本、版本历史）、自带密钥的 AI 助手、供外部工具使用的 MCP 服务。
- 原子保存和崩溃恢复，五套主题，可自定义快捷键。
- 安装包：Windows（x64、ARM64 的 MSI）、macOS（Apple 芯片、Intel 的 dmg）、Linux（x86_64、ARM64 的 deb 和 AppImage）。

### English

First release.
- Local-first Markdown notes: a note is a plain `.md` file in a folder you choose, and Obsidian vaults open as they are.
- Live preview: the content under the caret shows its source and the rest is rendered; tables, callouts, code blocks, images and note embeds, math, Mermaid diagrams and footnotes; plus reading and source modes.
- Wikilinks, backlinks, the knowledge graph, tags, properties, daily notes, templates, bookmarks, tabs and split view, and the `/` insert menu.
- Full-text search with Chinese support, local version history, and export to HTML.
- Optional S3 sync (three-way merge, conflict copies, version history), a bring-your-own-key AI assistant, and an MCP server for external tools.
- Atomic saves and crash recovery, five themes, and customisable shortcuts.
- Installers: Windows (MSI for x64 and ARM64), macOS (dmg for Apple silicon and Intel), Linux (deb and AppImage for x86_64 and ARM64).
