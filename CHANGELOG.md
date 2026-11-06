# 更新日志 · Changelog

版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。每个版本的说明同时用中文和英文书写。
Versions follow [Semantic Versioning](https://semver.org/). Every entry is written in Chinese and English.

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

**修复**
- 主题每次退出后都会恢复成默认值，现在会被保存；侧栏显示、实时预览开关也一并修复。保存某一部分配置时不再丢掉其他设置。
- 历史版本之间切换时不再闪烁。
- 统一了圆角、图标间距和输入框获得焦点时的样式，弹窗和设置页的圆角不再被直角背景盖住。
- 标签页很多时，标签栏会自动滚动到当前标签，两端有箭头按钮。
- 点击知识图谱中的节点，现在在新标签页打开（`Ctrl` 点击或中键在后台标签页打开）。
- 知识图谱右下角的节点数和连接数现在按当前筛选后实际显示的内容计算，不再重复计数。
- 阅读模式下鼠标移到链接上会变成手形指针。

**已知问题**
- 批量移动文件不能作为一个整体撤销，撤销只会还原最后一项。
- 笔记里 `[文字](网址)` 形式的链接暂时不能点击打开。

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

**Fixed**
- The theme went back to the default every time the app quit. It is now saved, together with the sidebar and live preview switches. Saving one part of the configuration no longer drops other settings.
- No more flicker when switching between versions in the history dialog.
- Consistent corners, icon spacing and focus style for inputs; dialogs and the settings page no longer have square backgrounds showing through rounded corners.
- With many tabs open, the tab strip scrolls to the active tab, with arrow buttons at the ends.
- A node clicked in the knowledge graph now opens in a new tab (`Ctrl`+click or middle click opens it in a background tab).
- The node and link counts at the bottom right of the graph now match what is shown after filters, without double counting.
- In reading mode the pointer becomes a hand over links.

**Known issues**
- A batch move cannot be undone as a whole; undo restores only the last item.
- Links written as `[text](url)` in a note cannot be opened by clicking yet.

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
