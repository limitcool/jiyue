# Roadmap · 路线图

[English](#english) | [简体中文](#简体中文)

---

## English

Jiyue is currently in **beta** across Windows, macOS, Linux and Android. This page lists what is planned and what is not. It is a statement of intent, not a promise of dates; items are listed roughly in the order we expect to do them.

### Next

- **Verify sync on real servers.** S3 sync, including conflict copies, interrupted runs and remote version history, has been tested end to end on one S3-compatible server (RustFS). Still to do: more S3 providers, and WebDAV against real servers (it is implemented, but only unit-tested so far).
- **Reload after sync.** An open note should refresh by itself when a sync changes it on disk, and warn when you have unsaved edits to it.
- **First real runs on macOS and Linux.** CI builds and tests both, but manual daily usage polish continues. Known rough edges: the custom tab bar cannot drag the window on macOS, Linux windows lack edge resizing and shadows, and fonts are taken from the system (Noto Sans SC and JetBrainsMono Nerd Font are expected).
- **Signed installers.** Code-signed Windows installers, and a signed and notarised macOS build. The build scripts already have an optional signing step; a certificate is the missing piece.
- **Smaller improvements.** A Shift+Enter line break in text inputs; a second launch that opens in the running window instead of a new one; opening `.txt` and `.markdown` files; sync uploads in parallel (500 notes take about 14 seconds on the first sync today); a lower typing cost when editing the frontmatter of a very large note.

### Later

- **PDF export** without going through a browser. Today a note is exported as HTML and printed to PDF from a browser.
- **Bundled fonts**, so the interface looks the same on every system.
- **Whole-interface scaling (all sizes).** Scale menus, sidebars and widgets together with the text. Today the font sizes in the settings change the note text and code only.
- **Mobile feature parity & iOS.** Bring the Android client on par with desktop captures/todos, attachment insertion, and system share capture, and consider iOS support atop `jiyue-ffi`.

### Done

Live preview with rendered tables, callouts, multi-language code blocks, images, note embeds, math, Mermaid and footnotes · Read and Source modes · floating selection format bubble toolbar and smart GFM table `Tab`/`Enter` editing · wikilinks, backlinks, one-click unlinked mention linking, hover link previews, local & global graph, tags and properties · inline `[[link]]` / `![[embed]]` / `[[Note#heading]]` / `#tag` completions · full-text search with Chinese support · daily notes, local captures/todos stream, templates and bookmarks · tabs with drag reordering and split view · the `/` insert menu · local version history and export to HTML · attachments via clipboard paste & drag-and-drop · S3 sync with three-way merge and conflict copies · bring-your-own-key AI assistant · atomic saves and crash recovery · customisable shortcuts · five themes · bilingual UI (`zh-CN` / `en-US`) · Android client (`Flutter` + `jiyue-ffi`) with desktop-to-mobile QR pairing · switching and remembering vaults · Windows installer and CI for Windows, macOS, Linux and Android.

### Not planned

- A **plugin system**. Features are built into the app.
- A **canvas** or whiteboard.
- A **block-based document model**. Notes stay plain Markdown files.
- **Database or query views** (Dataview-style tables over notes). Jiyue is a notes app, not a database.
- **Real-time co-editing.** Sync merges changes between devices; it does not edit live with other people.

---

## 简体中文

霁月目前已覆盖 Windows、macOS、Linux 桌面端与 Android 移动端，处于**公开测试版**阶段。这一页写计划做什么、不做什么。它说明方向，不承诺日期；条目大致按我们预计的先后顺序排列。

### 接下来

- **在真实服务器上验证同步。** S3 同步（包括冲突副本、中断后继续、远端历史版本）已经在一台兼容 S3 的服务器（RustFS）上做过端到端测试。还要做的是：更多 S3 服务商，以及在真实服务器上验证 WebDAV（已经实现，但目前只有单元测试）。
- **同步后自动重新加载。** 同步改动了磁盘上的文件时，已打开的笔记应当自动刷新；如果你对它有未保存的修改，要给出提示。
- **完善 macOS 和 Linux 的真实桌面体验。** CI 会构建并测试这两个系统，日常手动打磨仍在继续。已知的毛边：macOS 上自绘的标签栏拖不动窗口，Linux 窗口没有边缘缩放和阴影，字体取自系统（需要 Noto Sans SC 和 JetBrainsMono Nerd Font）。
- **签名的安装包。** 为 Windows 安装包做代码签名，为 macOS 构建做签名和公证。构建脚本里已经有可选的签名步骤，缺的是证书。
- **较小的改进。** 输入框里的 Shift+Enter 换行；第二次启动时在已运行的窗口里打开，而不是新开窗口；打开 `.txt` 和 `.markdown` 文件；同步上传改为并行（目前首次同步 500 篇笔记约需 14 秒）；降低在超大笔记里编辑 frontmatter 时的输入开销。

### 以后

- **不经过浏览器的 PDF 导出。** 现在是导出 HTML，再用浏览器打印成 PDF。
- **随应用附带字体，**让界面在各个系统上看起来一致。
- **整个界面的缩放（所有尺寸统一缩放）。** 菜单、侧栏和控件与文字一起缩放。现在设置里的字号只改变笔记正文和代码。
- **移动端能力对齐与 iOS 支持。** 让 Android 端追平桌面端本地随手记/待办、图片附件插入与系统分享入库体验，并在 `jiyue-ffi` 之上评估 iOS 支持。

### 已完成

实时预览（表格、提示块、多语言高亮代码、图片、嵌入笔记、公式、Mermaid 和脚注都渲染）· 阅读和源码模式 · 选中文本浮动格式条与 GFM 表格智能 `Tab`/`Enter` 跳格续行 · 双链、反链、未链接提及一键转双链、双链悬停预览、局部与全局图谱、标签和属性 · 内联 `[[双链]]` / `![[嵌入]]` / `[[笔记#标题]]` / `#标签` 自动补全 · 支持中文的全文搜索 · 每日笔记、纯本地随手记与待办流、模板和书签 · 可拖动排序的标签页和分屏 · `/` 插入菜单 · 本地历史版本和导出 HTML · 剪贴板粘贴与外部拖拽导入附件 · 带三方合并和冲突副本的 S3 同步 · 自带密钥的 AI 助手 · 原子保存和崩溃恢复 · 可自定义的快捷键 · 五套主题与中英双语切换 · Android 客户端（`Flutter` + `jiyue-ffi`）与桌面扫码配对 · 切换并记住笔记库 · Windows 安装包，以及 Windows、macOS、Linux 和 Android 的 CI。

### 不计划做

- **插件系统。** 功能直接做进应用里。
- **画布**或白板。
- **基于块的文档模型。** 笔记始终是普通的 Markdown 文件。
- **数据库或查询视图**（类似 Dataview 的笔记表格）。霁月是笔记应用，不是数据库。
- **实时多人协作编辑。** 同步是在设备之间合并改动，不是和别人实时一起编辑。
