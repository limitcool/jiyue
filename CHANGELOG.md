# 更新日志 · Changelog

版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。每个版本的说明同时用中文和英文书写。
Versions follow [Semantic Versioning](https://semver.org/). Every entry is written in Chinese and English.

## [0.3.6] - 2026-11-16

### 中文

**链接导航与库内路径解析重构**
- **实时预览折叠态蓝链免 Ctrl 单击直达与精确字形命中**：
  - 在实时预览（Live）模式下，处于折叠状态的双链 `[[...]]`、Markdown 链接 `[文本](...)` 及脚注引用悬停即显示手型光标，左键单击即可直接跳转；按住 `Alt + 单击` 可绕过跳转直接进入源码编辑，已展开链接与源码模式仍保持 `Ctrl + 单击` 跳转。
  - 链接点击与悬停改用字形包围盒（`hit_index_at`）精确判定，彻底消除中文蓝链末尾字符边缘滑向后续标点（如 `:`）导致的点击失灵，同时确保点击行尾空白处正常放置光标。
- **相对子目录、YAML 别名（`aliases`）、文件夹与索引页智能解析**：
  - 放开带 `/` 相对路径双链限制并接入全库 `resolve_link`（支持相对路径、`aliases` / `alias` 前置属性），对面向文件夹的入口链接（如知识库主页的一级主题目录）智能匹配 `<目录>/<同名>.md`、`README.md`、`index.md`、`_indexes/<名称>.md` 及去序号前缀的 `_indexes/01-xxx.md`；纯文件夹则在左侧栏自动展开并滚动定位，缺失子目录双链支持安全新建。
  - 新增库内相对路径 Markdown 链接 `[文本](路径.md)` 点击跳转支持（含 `%20` 编码还原、中文/Emoji 含空格路径与 `<...>` 包裹格式），并打通 `Live` / `Read` / `Source` 三种视图模式的 `#标题` 与 `#^块ID` 锚点跳转。

**大型知识库跟手度与渲染帧率深度优化**
- **消除渲染期同步读盘 `.jiyue/config.toml` 风暴**：
  - 为 `Vault::is_daily_note` 新增内存配置缓存 `daily_settings_cache`，仅在加载知识库、修改设置或配置文件热重载时刷新，消除文件树遍历每篇笔记产生的海量同步磁盘 I/O。
- **侧栏文件树与长标签列表虚拟化**：
  - 左侧栏文件树（无内联创建/重命名行时）与超 20 项的标签列表全面切换为 `gpui::uniform_list` 虚拟滚动，仅构建并绘制视口可见行；保留 `reveal_path` 定位自动居中滚动，显著降低大型 Obsidian 知识库下的悬停与滚动延迟。
- **右侧栏检查器缓存、编辑器零拷贝与非编辑帧直通快路径**：
  - 为右侧大纲与 YAML 属性面板（基于笔记路径与 `Arc::ptr_eq` $O(1)$ 校验）及正文元数据“创建于”日期标签增加内存缓存，消除侧栏悬停触发顶层重绘时的重复全文扫描与 `fs::metadata` 调用。
  - 将 `TextEditor` 鼠标悬停探测优化为二分 `style_at` $O(\log N)$ 查询 + `HoverMemo` 同字形 $O(1)$ 早退，并将裸链接检测改为无堆分配字节扫描；新增 `EditorState::text_arc()` 与 `state_id()` 快照，在 `TextElement::prepaint` 中为 500ms 光标闪烁/悬停/侧栏联动等非编辑帧建立全量布局直通快路径与视口二分裁剪。

---

### English

**Link Navigation & Vault Path Resolution Overhaul**
- **Direct Plain Click on Collapsed Links in Live Preview & Glyph-Exact Hit Testing**:
  - Collapsed wikilinks `[[...]]`, Markdown links `[text](...)`, and footnote references in Live preview now show a pointer cursor on hover and navigate on a plain left click (`Alt + click` enters source editing; revealed links and Source mode retain `Ctrl + click`).
  - Added glyph-bounds hit testing (`hit_index_at`) so clicking the trailing edge of CJK link text no longer slips onto adjacent punctuation (such as `:`) and clicking trailing line whitespace places the caret normally.
- **Subdirectory Paths, Frontmatter `aliases`, Folder & Index Resolution**:
  - Removed the `safe_note_title` restriction on `/` paths and wired wikilink navigation and hover previews through `resolve_link` with cached `aliases`/`alias` frontmatter metadata.
  - Links targeting vault directories fall back cleanly to `<dir>/<dir>.md`, `README.md`, `index.md`, `_indexes/<name>.md`, or numeric-prefixed index notes before revealing and scrolling to the folder in the sidebar; missing subdirectory targets are created cleanly via `create_link_note_at`.
  - Enabled clicking vault-relative Markdown links (`[text](path.md)`, percent-encoded URLs, `<...>` destinations, and spaces in CJK/Emoji folder names) and cross-mode `#Heading` / `#^block` anchor scrolling across `Live`, `Read`, and `Source` views.

**Large Vault Responsiveness & Frame-Time Optimization**
- **Eliminated Per-Note `.jiyue/config.toml` Synchronous Disk I/O**:
  - Cached daily note settings in memory on `Vault` (`daily_settings_cache`), refreshed only on vault load, settings update, or config reload.
- **Virtualized Sidebar File Tree & Long Tag Lists**:
  - Switched the sidebar file tree (when no inline rename/create input is active) and long tag lists (> 20 tags) to `gpui::uniform_list` virtual scrolling with cached flattened rows and center-aligned `reveal_path` scrolling.
- **Inspector/Metadata Caching & Zero-Copy Editor Fast Paths**:
  - Cached right-panel outline headings and YAML frontmatter blocks keyed by `(note.path, content_arc)` via `Arc::ptr_eq`, and cached file creation labels to remove repeated `fs::metadata` calls.
  - Optimized editor `on_mouse_move` with binary-searched `StyleCache::style_at` ($O(\log N)$), allocation-free `bare_url_at` scanning, and `HoverMemo` $O(1)$ glyph-level early return; added `EditorState::text_arc()`/`state_id()` and a full layout bypass fast path plus viewport `partition_point` culling in `TextElement::prepaint` for non-editing repaints (cursor blink, hover, sidebar hover).

## [0.3.5] - 2026-11-16

### 中文

**大型笔记库性能与内存专项治理**
- **流式搜索索引与按需写入器管理**：
  - 将 `jiyue-search` 中的 Tantivy `IndexWriter` 改为按需创建单线程写入器（15 MB 堆预算），并在提交与全库重建完成后立即释放，避免闲置常驻内存占用。
  - 索引构建改为分块流式读取与写入，移除建库前将整库全文一次性囤积在内存队列中的行为。
- **文本缓冲与双链缓存轻量化**：
  - 移除 `gpui_sum_tree` 依赖，将 `ZedBuffer` 重构为不可变共享 `Arc<str>` 与紧凑统计摘要（字节数、字符数、换行数），消除海量行级堆碎片分配。
  - 在 `Note` 解析时预提取并缓存双链目标，避免反链检索时对未命中笔记执行全文小写转换与重复正则扫描。
- **并行加载、修订号门控与渲染热路径去重**：
  - `Vault` 新增确定性多线程并行加载与 `revision` 修订号追踪，并新增零正文拷贝图谱构建链路（`GraphLinkSource`）。
  - 为 `LiveContext`、侧边栏文件树与标签列表、右侧栏反链面板引入按修订号缓存；大纲页签切换时跳过无意义的反链预计算，彻底消除每帧全库对比与冗余分配。

**Android 正式发布签名与应用内自更新闭环**
- **正式发布签名与跨版本覆盖安装体系**：
  - 在 [`build.gradle.kts`](mobile/android/app/build.gradle.kts) 与发布流水线中完整接入 Android 正式发布签名证书（注：从 `v0.3.4` 及更早测试签名版升级至 `v0.3.5` 时需最后一次卸载重装以切换签名证书，自 `v0.3.5` 起后续版本均可直接覆盖更新）。
- **应用内流式下载与原生系统安装器拉起**：
  - 新增 `FileProvider` 与 `MainActivity` 原生更新通道，支持读取真实已安装版本号、未知来源安装权限检查与引导，并在应用的 `updates/` 缓存目录中流式分块下载 APK，自动复用已校验缓存与清理未完成 `.part` 文件。
  - 重构 [`UpdateDialog`](mobile/lib/widgets/update_dialog.dart) 与设置页版本卡片：支持实时进度条、取消保护与内联错误/网页兜底展示（解决原先 `SnackBar` 被设置抽屉遮挡的问题）。
  - 为移动端版本检查增加 12 秒请求超时与 GitHub API `403`/`429` 限流时的网页端发布资产回退解析，并补齐 `<queries>` 协议声明。

**桌面端设置中心重构与 Windows 静默更新优化**
- **设置中心组件布局与交互精修**：
  - 重构桌面端设置中心卡片分组、字体与行高步进器（防止文字溢出），完善下拉选择器浮动层级、滚动与文本截断表现，并统一 AI、同步、附件、模板、知识库、快捷键与微记面板的表单控件排布及中英双语文案。
- **Windows 无黑框子进程与更新弹窗文案清理**：
  - 在 Windows 桌面端自动更新静默安装（[`update.rs`](crates/jiyue-app/src/update.rs)）及外部凭据命令调用（[`secret.rs`](crates/jiyue-core/src/config/secret.rs)）中注入 `CREATE_NO_WINDOW` 标志，消除更新或读取凭据时弹出黑色控制台窗口的问题。
  - 清理桌面端更新说明弹窗中的原始 Markdown 语法标记，提升阅读整洁度。

---

### English

**Large Vault Memory & Performance Optimization**
- **On-Demand Tantivy Writer & Streamed Indexing**:
  - Converted `jiyue-search`'s Tantivy `IndexWriter` to an on-demand single-threaded writer (15 MB budget) that is dropped immediately after `commit()` and rebuild completion.
  - Replaced full-vault in-memory document buffering with chunked streaming indexing during search index rebuilds.
- **Lightweight Text Buffer & Link Caching**:
  - Removed the `gpui_sum_tree` dependency and redesigned `ZedBuffer` around shared `Arc<str>` slices with compact `TextSummary` metrics, eliminating per-line heap fragmentation.
  - Pre-extracted and cached wikilink targets on `Note` to skip full-body lowercasing and repeated scans on non-matching notes during backlink lookups.
- **Parallel Loading, Revision Gating & Render Loop Caching**:
  - Added deterministic multi-threaded vault loading (`load_notes_parallel`), `Vault.revision` tracking, and zero-body-copy graph building (`GraphLinkSource`).
  - Gated `LiveContext` snapshot syncs, sidebar file-tree/tag caches, and right-panel backlink computations on `vault.revision`, and stopped computing backlinks unconditionally when viewing the outline tab.

**Android Release Signing & In-App Updater**
- **Release Signing Pipeline**:
  - Configured proper Android release keystore signing in [`build.gradle.kts`](mobile/android/app/build.gradle.kts) and CI/CD workflows (upgrading from `v0.3.4` or earlier debug-signed builds requires a one-time uninstall/reinstall to switch certificates; upgrades from `v0.3.5` onward support seamless in-place installation).
- **In-App Streaming APK Download & Native Installer**:
  - Added Android `FileProvider` configuration and a native `MethodChannel` bridge in [`MainActivity.kt`](mobile/android/app/src/main/kotlin/cool/jiyue/jiyue_mobile/MainActivity.kt) for reading installed package versions, checking/requesting unknown-source install permissions, and launching the system package installer.
  - Upgraded [`UpdateService`](mobile/lib/services/update_service.dart), [`UpdateDialog`](mobile/lib/widgets/update_dialog.dart), and [`SettingsSheet`](mobile/lib/widgets/settings_sheet.dart) with 12-second timeouts, GitHub API `403`/`429` web-fallback asset parsing, chunked APK download with progress tracking, cancellation cleanup (`.part` removal), cache reuse, and inline update error/status feedback inside the settings sheet.

**Desktop Settings Polish & Windows Silent Update**
- **Desktop Settings Refinement**:
  - Refined settings section grouping, font/line-height stepper layouts, and floating dropdown menus (`occlude`, scrolling, text truncation), and standardized control layouts and zh-CN/en-US localization across AI, Sync, Attachments, Templates, Vaults, Keymaps, and Memos settings.
- **Windowless Windows Subprocesses & Clean Release Notes**:
  - Added `CREATE_NO_WINDOW` flags to Windows silent MSI updates ([`update.rs`](crates/jiyue-app/src/update.rs)) and secret command execution ([`secret.rs`](crates/jiyue-core/src/config/secret.rs)) to eliminate flashing console windows, and stripped raw Markdown markers in the desktop update prompt.

## [0.3.4] - 2026-11-15

### 中文

**核心重构与视觉升级**
- **跨端单一事实来源设计系统（Jiyue Design System v3.0 & SSOT Tokens）**：
  - 建立跨桌面端（GPUI）与移动端（Flutter）的统一设计令牌工程体系，自动同步生成 [`theme_tokens.rs`](crates/jiyue-app/src/theme_tokens.rs) 与 [`jiyue_theme.dart`](mobile/lib/theme/jiyue_theme.dart)。
  - 全面规范雾白（Fog）、宣纸（Paper）、苔影（Moss）、玄墨（Ink）、深夜（Dusk）5 套内置主题的表面层级（Canvas / Chrome / Elevated / Floating / Scrim）、高对比文本色阶、`1px` 细腻边框、圆角、间距与字阶令牌。
- **桌面端 Linear / Zed 克制现代工具风重构**：
  - 彻底移除外层冗余留白与悬浮套娃卡片，重构为零间隙（0px gap）平铺分栏与 `1px border_subtle` 精准分割的现代工作区架构。
  - 统一 `40px` 顶栏（面包屑与动作区）与 `24px` 底部状态栏视觉节奏；将左侧栏多模式切换收敛整合为高辨识度的分段控制器。
  - 扩充 `controls.rs` 母版组件库（新增 `segmented`、`segmented_full`、`badge_tag`、`kbd_hint` 等标准件），全量清理 `ai_ui`、`memos_ui`、`settings`、`export_config`、`bookmarks`、`history_ui`、`search_ui` 等面板中的硬编码色值与手写裸 `div`。
- **移动端母版控件库落成与全页面重构**：
  - 新建 [`jiyue_controls.dart`](mobile/lib/widgets/jiyue_controls.dart)，统一封装 16 个高频标准控件（`JiyueButton`、`JiyueIconButton`、`JiyueBadge`、`JiyueChoiceChip`、`JiyueTagChip`、`JiyueSegmented`、`JiyueSwitch`、`JiyueSwitchRow`、`JiyueTextField`、`JiyueSectionHeader`、`JiyueSectionCard`、`JiyueDivider`、`JiyueBanner`、`JiyueEmptyState`、`JiyueModalSheet`、`JiyueDialog`）。
  - 全量重构知识库主页、阅读页、编辑页、扫码页以及设置抽屉、S3/Memos/AI 配置抽屉、AI 润色面板与版本自更新弹窗，消除散落的硬编码 `Color(...)`、未封装原生 Material 表单控件与非规范 Emoji 装饰。
- **双端 Tabler SVG 图标同步与自动化防回归审计**：
  - 在桌面端与移动端同步补齐并注册 `layout-sidebar`、`bolt`、`camera-rotate`、`download`、`qrcode`、`refresh`、`sparkles` 7 枚标准 SVG 图标，根除历史幽灵图标引用问题。
  - 新增 [`ui_ssot_test.dart`](mobile/test/ui_ssot_test.dart) 静态红线测试并对齐 [`unit_test.dart`](mobile/test/unit_test.dart) 全主题令牌断言，防止未来引入硬编码色值或失效图标资源。

---

### English

**Design System & Cross-Platform UI Overhaul**
- **Single Source of Truth Design System (Jiyue Design System v3.0 & SSOT Tokens)**:
  - Established a unified design token pipeline syncing both desktop (GPUI) and mobile (Flutter) themes via [`theme_tokens.rs`](crates/jiyue-app/src/theme_tokens.rs) and [`jiyue_theme.dart`](mobile/lib/theme/jiyue_theme.dart).
  - Standardized surface layering (Canvas / Chrome / Elevated / Floating / Scrim), typography scales, spacing, radii, and `1px` crisp borders across all 5 built-in themes: Fog, Paper, Moss, Ink, and Dusk.
- **Desktop Linear / Zed Modern Workspace Overhaul**:
  - Replaced nested floating cards and excess outer margins with a zero-gap tiled workspace separated by crisp `1px border_subtle` hairlines.
  - Unified the `40px` header/breadcrumb bar and `24px` bottom status bar rhythm, and consolidated sidebar view switching into a segmented control.
  - Expanded the shared `controls.rs` primitive library (`segmented`, `segmented_full`, `badge_tag`, `kbd_hint`, etc.) and eliminated ad-hoc styled divs and hardcoded colors across AI, Memos, Settings, Export, Bookmarks, History, and Search panels.
- **Mobile Shared Component Library & Full View Refactor**:
  - Introduced [`jiyue_controls.dart`](mobile/lib/widgets/jiyue_controls.dart) with 16 standardized UI primitives (`JiyueButton`, `JiyueIconButton`, `JiyueBadge`, `JiyueChoiceChip`, `JiyueTagChip`, `JiyueSegmented`, `JiyueSwitchRow`, `JiyueTextField`, `JiyueSectionCard`, `JiyueModalSheet`, `JiyueDialog`, etc.).
  - Refactored Vault, Reader, Editor, and QR Scanner pages along with Settings, S3/Memos/AI config sheets, AI Polish sheet, and Update dialog to eliminate hardcoded colors, unstyled Material form widgets, and emoji decorations.
- **Cross-Platform Tabler SVG Sync & Automated SSOT Guardrails**:
  - Synced and registered 7 missing standard SVG icons (`layout-sidebar`, `bolt`, `camera-rotate`, `download`, `qrcode`, `refresh`, `sparkles`) across both desktop and mobile asset bundles, fixing broken icon references.
  - Added [`ui_ssot_test.dart`](mobile/test/ui_ssot_test.dart) static audit tests and updated [`unit_test.dart`](mobile/test/unit_test.dart) to enforce SSOT color, radius, and asset integrity.

## [0.3.3] - 2026-11-15

### 中文

**优化与改进**
- **FigCraft 驱动的统一设计系统与全量组件库重构（Design System v2.0）**：
  - 基于 FigCraft 规范落成全项目统一的基础控件系统，覆盖默认（Default）、悬浮（Hover）、按压（Active）、聚焦（Focus）与禁用（Disabled）5 大标准交互状态。
  - **基础按钮分级收敛**：补齐 `ButtonSize::Large` (36px)，统一全项目按钮尺寸标准为 Large 36px、Medium 28px、Small 22px，Primary / Secondary 按钮默认注入 `.shadow_sm()` 抗混叠微羽化边缘，消除硬边缘切边感。
  - **公共选择与切换控件**：全面引入 `choice_chip`（胶囊选项）、`checkbox_toggle`（带说明复选框）、`toggle_view`（图表/表格/源文本视图分段切换）等高级公共组件。
  - **输入框与表单控件升级**：`LineInput` 基准高度统一提升至 32px，内边距规范化为 8px，极大提升点击感与视觉舒适度；新增 `error: bool` 状态 API，聚焦时自动附加高辨识度 `.shadow_sm()` 聚焦光晕环（Focus Ring），提供清晰明确的操作反馈。
- **外观主题卡片四角锯齿感彻底消除**：
  - 深度重构 `AppearanceView` 的主题预览卡片，根除因内部双色侧边栏直角切边暴露导致的四个圆角锯齿断裂缺陷；
  - 采用内外双层高精度圆角匹配与平滑抗混叠边框，无论明亮还是暗色主题，卡片边缘与高亮蓝框圆滑无瑕。
- **Memos、设置与导出面板样式彻底统一**：
  - 彻底拔除全项目历史散落的手写裸 `div`、内联自定义状态切换器，全量收敛替换为统一的 controls 与 choice_chip。
  - 标签过滤区（微记/全部微记/Tag 筛选）与视图切换器风格完全契合现代化设计规范。
- **CI / CD 构建效能诊断与优化**：
  - 针对 GitHub Actions 构建耗时展开深度诊断，查明 macOS x64 与 Windows arm64 历史耗时较长的核心瓶颈在于缺少跨构建缓存命中；
  - 优化 Actions 依赖版本（升级 `actions/setup-java@v5` 等），确保发布构建稳定高速运行。

---

### English

**Improvements**
- **FigCraft-driven Design System v2.0 & Component Unification**:
  - Unified project-wide controls with full interactive states: Default, Hover, Active, Focus, and Disabled.
  - **Button System Standardization**: Added `ButtonSize::Large` (36px), standardizing on Large 36px, Medium 28px, and Small 22px. Primary and Secondary buttons now feature `.shadow_sm()` anti-aliased micro-feathering to eliminate harsh aliasing.
  - **Shared Choice & Toggle Widgets**: Introduced `choice_chip`, `checkbox_toggle`, and `toggle_view` widgets across settings, memos, and export dialogues.
  - **Input & Form Enhancements**: Raised `LineInput` standard height to 32px with 8px padding; added `error: bool` state and `.shadow_sm()` glowing focus rings.
- **Theme Preview Card Anti-Aliasing**:
  - Fixed corner aliasing on theme preview cards by matching interior split panes to outer rounded borders with anti-aliased outlines.
- **Memos, Settings & Export Panels Cleanup**:
  - Replaced inline ad-hoc divs and raw buttons with standardized design system controls across Memos filters, AI settings, and export options.
- **CI/CD Optimization**:
  - Diagnosed GitHub Actions workflow bottlenecks, verified cross-run compiler cache hits for macOS x64 and Windows arm64, and upgraded workflow dependencies (`actions/setup-java@v5`).

## [0.3.2] - 2026-11-15

### 中文

**新增**
- **跨端配置一键导出与手机扫码同步**：
  - PC 端设置新增「📱 手机扫码同步」面板，原生渲染高精度配置二维码，并支持一键导出剪贴板加密/脱敏配置载荷（覆盖 S3 云同步、Memos 及 AI 服务）。
  - 移动端引入扫码镜头与剪贴板识别器，扫码或读取剪贴板即可一键预览并持久化双端配置，彻底告别移动端繁琐手动录入。
- **全端 AI 智能润色与速记工具箱**：
  - PC 端编辑器：文本选区上方自动浮现呼吸态快捷胶囊（`✨ AI 润色 / 助手`），支持全局快捷键 `Ctrl+J` 唤起。
  - PC 端 AI 侧边栏：新增「当前笔记快捷工具箱」，提供智能润色、提炼摘要、提取核心标签、智能命名建议与一键内容替换。
  - 移动端：实现统一的 `AiPolishSheet` 润色交互面板，预置智能润色、学术专业、生动通俗、提炼摘要四种专业风格，完美融入系统长按选区上下文菜单（`contextMenuBuilder`）与阅读态顶栏入口。
  - 适配 Anthropic Messages 协议与 DeepSeek 思考模型，实现思考链（thinking）与回答内容的高效解耦分离。
- **移动端应用内版本自更新 (In-App Updates)**：
  - 移动端实现自动对接 GitHub Releases 最新发布接口，智能挑选适配当前移动端架构的 APK 安装包（优先识别 arm64-v8a）。
  - 精美自更新弹窗：支持 Markdown 格式更新日志原生渲染、版本对照徽章、安装包体积显示与一键唤起系统下载。
  - 侧边栏设置新增「软件版本与自更新」模块，带动态加载状态与友好限流容错。

---

### English

**Added**
- **Cross-device config export & mobile QR scanner sync**:
  - Added "📱 Mobile QR Sync" panel on desktop with native QR matrix rendering and clipboard JSON export for S3, Memos, and AI configurations.
  - Implemented mobile QR camera scanner and clipboard reader for instant configuration preview and persistence across devices.
- **Cross-platform AI smart polish & note assistant toolbox**:
  - Desktop editor: Floating selection pill (`✨ AI Polish / Assistant`) and `Ctrl+J` shortcut trigger for instantaneous context actions.
  - Desktop AI side panel: Dedicated "Current Note Toolbox" featuring smart polish, executive summary, tag extraction, title generation, and in-place replacement.
  - Mobile app: Complete `AiPolishSheet` modal supporting Smart, Academic, Casual, and Summary modes, seamlessly integrated into Android native text selection context menus and reader view.
  - Full support for Anthropic Messages API and DeepSeek reasoning models with thinking/answer block separation.
- **Mobile In-App update checker**:
  - Implemented automatic GitHub Releases update check with semantic version comparison and mobile APK asset matching (prioritizing arm64-v8a).
  - Rich update dialog featuring native Markdown changelog rendering, version badge diffs, download sizes, and one-tap external browser download triggers.
  - Added dedicated "Updates & About" card in mobile settings with animated loading indicators.

## [0.3.1] - 2026-11-15

### 中文

**优化与变更**
- **默认启动进入纯净欢迎页并剥离 demo-vault**：
  - 彻底移除以往打开空笔记库时自动在磁盘注入的 5 篇示例笔记及写死的默认打开标签页，避免污染用户文件夹。
  - 从源码仓库中完全清理移除 `demo-vault` 目录。
  - 启动应用且上次未保留打开笔记时，主工作区直接呈现精美欢迎页（Welcome Page），并新增“切换笔记库”快捷操作。
- **Android 移动端应用体验升级**：
  - 应用名称正式更新为“霁月”，告别默认工程名。
  - 全面配置高清官方自适应与圆形桌面启动图标（支持 Android 8.0+ Adaptive Icons 及各类启动器规范）。
- **Release 构建流智能增量复用**：
  - 优化 GitHub Actions 发布流水线，在移动端代码无变更时自动复用已发布的 Android APK，大幅缩减发布打包耗时；同时支持手动强制重构选项。

---

### English

**Improvements & Changes**
- **Default to clean Welcome Page & remove demo-vault**:
  - Completely removed automatic sample note injection into empty vaults and hardcoded restore tabs, ensuring user directories remain untainted.
  - Fully removed the `demo-vault` directory from the repository.
  - When starting the app without open tabs, the central workspace directly displays the clean Welcome Page, complete with a new "Switch Vault" quick action.
- **Android mobile experience enhancement**:
  - Standardized application display label to "霁月 (Jiyue)".
  - Added official high-resolution standard and circular adaptive launcher icons across all mipmap densities (Android 8.0+ adaptive icon support).
- **Smart Release workflow asset caching**:
  - Optimized the GitHub Actions release workflow to automatically detect changes and reuse previous Android APK assets when mobile sources are untouched, dramatically reducing build times while retaining manual force-rebuild capabilities.

## [0.3.0] - 2026-11-14

### 中文

**新增**
- **Memos 待办与微记系统（桌面与移动灵感收件箱）**：
  - 全新独立核心库 `crates/jiyue-memos`，完整封装 Memos v1 REST API 服务。
  - 左侧栏原生 GPUI 待办看板：支持待办清单（过滤 Markdown 中的 `- [ ]`）与全部微记时间线视图，支持直接点击勾选框切换完成状态（带划线完成视觉反馈）。
  - 顶部快速记事栏：输入文字或任务回车直接推送到 Memos 服务，无需打开浏览器。
  - 今日日记联动（Daily Notes Digest）：面板工具栏提供一键归档按钮，将今日的所有闪念与待办格式化追加到今日日记末尾。
  - 一键插入笔记：卡片支持一键将微记正文直接插入到当前光标所在笔记。
  - 安全凭据隔离：设置面板提供专用 Memos 实例与 Access Token 配置，密钥独立保存在本机的 `credentials.toml` 中，保障隐私安全。
- **Android APK 自动化构建与发布**：
  - 将移动端 Android APK 打包无缝整合至主发布工作流（`.github/workflows/release.yml`），由 `v*` 标签或手动触发。
  - 构建产物直接作为独立的 `Jiyue-0.3.0-android.apk` 安装包并入 GitHub Releases，免解压缩，安卓手机或浏览器直接一键下载安装。
  - 环境采用与工程完全匹配的 Flutter 3.29.2 与 Java 17，并在 CI 环境中自动直连官方源保障构建稳定性。

---

### English

**Added**
- **Memos & Todo Dashboard (Capture & Inbox System)**:
  - New standalone crate `crates/jiyue-memos` wrapping the Memos v1 REST API.
  - Native GPUI sidebar panel: Filter pending todo tasks (`- [ ]`) or browse full timeline memos; interactive checkbox toggling with optimistic strike-through visual feedback.
  - Quick capture bar: Instantly post thoughts or tasks directly to Memos from the sidebar without opening a browser.
  - Daily Notes integration: One-click action to format and append today's memos and tasks into the active daily note.
  - Insert to note: One-click action to insert memo markdown content directly into the active editor at the cursor.
  - Secure credentials isolation: Dedicated Memos settings page; access tokens are safely stored in local `credentials.toml`, preventing accidental git leaks.
- **Android APK Build & Unified Release Workflow**:
  - Integrated Android APK packaging directly into the main release workflow (`.github/workflows/release.yml`), triggered alongside desktop releases on tags.
  - Releases uncompressed `Jiyue-0.3.0-android.apk` binary alongside desktop packages on GitHub Releases for fast, direct downloading on mobile devices.
  - Pinned Flutter 3.29.2 and Java 17 with CI official Maven/Google repository connectivity for reliable unattended builds.

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
