# Jiyue 路线图与并行开发计划

更新:2026-11-04。本文是所有 agent 共同遵守的契约:目录、接口、文件所有权、验收标准。

## 0. 决策记录
- 项目名:Strata → Stria → Jiyue(霁月)。
- 笔记 = 纯 Markdown 文件(与 Obsidian 库互通)。文件是唯一真相,索引和缓存都放在 `.jiyue/`,可随时重建。
- 同步传输用 **S3(优先)+ WebDAV(次要)**,统一用 `opendal` 封装;利用 **S3 桶版本控制**做远端历史与误删恢复(运行时探测是否开启,未开启则优雅降级,绝不自动改用户的桶配置)。
- 冲突:本地保存"上次同步基准",文本按**三方合并**,重叠冲突生成**冲突副本**并提示;二进制保留两份。不上 CRDT(以后需要实时共编再说)。
- AI:**自带密钥(BYOK)**,默认关闭;OpenAI 兼容接口 + Anthropic;密钥存 Windows 凭据管理器(`keyring`),不写明文;发送给模型的内容对用户可见。另提供本地 **MCP 服务**让外部 agent 读写笔记库。
- "memraid" 按 **Mermaid** 图表处理:用 `mermaid-rs-renderer`(纯 Rust)出 SVG,缓存后在预览里显示。
- 不做:插件系统、Canvas 白板、块结构文档模型。
- 移动端:GPUI 无移动版,以后另选 UI 技术,共用 `jiyue-core` / `jiyue-sync` / `jiyue-search` / `jiyue-ai`。

## 1. 目标目录(第 0 波完成后)
```
D:\dev\rustdian\            git 仓库根(.gitignore 忽略 target、*.exe、截图、临时目录)
  Cargo.toml  Cargo.lock  rust-toolchain.toml   [workspace] members = ["crates/*"]
  crates\
    jiyue-core\          lib,无 gpui:buffer, daily, editor_state, md_edit, vault(笔记库/Note/链接图), 以及后续 fsops, watch, links, embeds, attach
    jiyue-search\        lib:tantivy 全文索引 + 中文分词 + 查询语法
    jiyue-sync\          lib:RemoteStore(opendal: S3/WebDAV), 本地索引, 同步计划, 三方合并, 冲突, 版本历史
    jiyue-ai\            lib:ChatProvider/EmbeddingProvider, RAG, 提示词, 密钥存取
    jiyue-mcp\           bin `jiyue-mcp`:stdio MCP 服务(第 2 波)
    jiyue-app\           bin `jiyue`:GPUI 界面(main.rs, theme.rs, session.rs, ui\*, assets\)
  demo-vault\  design\  docs\(docs\dev\PLAN.md, docs\dev\AGENT_RULES.md)  dev\  scripts\  installer\
```
包名 `rustdian` → `jiyue`,可执行文件 `jiyue.exe`。

## 2. 接口约定(第 0 波创建骨架,后续 agent 只填充实现,不改签名;确需改动要在报告里说明)
### jiyue-sync(阻塞式 API,内部自带 tokio 运行时,调用方在后台线程使用)
```rust
pub struct RemoteEntry { pub path: String, pub size: u64, pub etag: Option<String>, pub modified_ms: Option<i64>, pub version_id: Option<String>, pub is_dir: bool }
pub struct PutOptions { pub if_match: Option<String>, pub if_none_match_star: bool }
pub enum StoreError { NotFound, PreconditionFailed, Unauthorized, Network(String), Other(String) }
pub trait RemoteStore: Send + Sync {
    fn list(&self, prefix: &str) -> Result<Vec<RemoteEntry>, StoreError>;          // 递归
    fn stat(&self, path: &str) -> Result<Option<RemoteEntry>, StoreError>;
    fn get(&self, path: &str, version_id: Option<&str>) -> Result<Vec<u8>, StoreError>;
    fn put(&self, path: &str, data: &[u8], opts: &PutOptions) -> Result<RemoteEntry, StoreError>;
    fn delete(&self, path: &str) -> Result<(), StoreError>;
    fn list_versions(&self, path: &str) -> Result<Vec<RemoteEntry>, StoreError>;   // 未开启版本控制时返回空
    fn supports_versions(&self) -> bool;
}
pub fn three_way_merge(base: &str, local: &str, remote: &str) -> MergeResult;       // Clean(String) | Conflict{ merged_with_markers: String, hunks: usize }
pub fn plan_sync(local: &LocalSnapshot, state: &SyncState, remote: &[RemoteEntry]) -> Vec<SyncAction>;
```
### jiyue-search
```rust
pub struct NoteDoc { pub path: String, pub title: String, pub body: String, pub tags: Vec<String>, pub mtime_ms: i64 }
pub struct Hit { pub path: String, pub title: String, pub snippet: String, pub highlights: Vec<std::ops::Range<usize>>, pub score: f32 }
pub struct SearchIndex;  // open(dir), upsert(&NoteDoc), remove(path), commit(), search(&str, limit) -> Vec<Hit>, rebuild(impl Iterator<Item=NoteDoc>)
// 查询语法:词、"短语"、-排除、tag:x、path:x、file:x;中文用二元组分词,无需词典。
```
### jiyue-ai
```rust
pub trait ChatProvider: Send + Sync { fn chat_stream(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse, AiError>; }
pub trait EmbeddingProvider: Send + Sync { fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError>; }
// 另有:ProviderConfig(base_url, model, kind: OpenAiCompatible | Anthropic), Secrets(keyring), VectorIndex(.jiyue/index), rag::retrieve(...)
```

## 3. 波次

### 第 0 波(串行,先做)— 工程基础
- 备份;`git init`、`.gitignore`;把现有单 crate 拆成上面的 workspace;改名 jiyue;建 jiyue-search/-sync/-ai 骨架(含上面的类型与 trait,`unimplemented` 以外的部分可编译、有占位测试);`cargo test` 全过、应用可启动。
- 验收:`cargo build --release` 通过;`jiyue.exe --theme ink --vault demo-vault` 与拆分前外观一致;全部既有测试(406)仍通过;工具脚本路径更新。

### 第 1 波(并行,纯库,互不重叠)
| 代号 | 范围(只改自己的目录/文件) | 验收 |
|---|---|---|
| W1-A | jiyue-core: `fsops.rs`(新建/重命名/移动/删除/回收站 `.trash`)、`links.rs`(重命名时更新 `[[旧名]]`、别名、标题锚点、折叠大小写)、`watch.rs`(notify 监听,忽略自己写入,去抖,产生事件) | 单元测试 + 临时目录集成测试;重命名链接更新含边界用例 |
| W1-B | jiyue-search: tantivy 索引、CJK 二元分词、查询语法、增量更新、高亮片段 | 中英混合语料测试,10k 篇笔记基准 |
| W1-C | jiyue-core: `embeds.rs`(解析 `![[笔记]]`、`![[图.png\|300]]`、`![](path)`、块引用 `^id`、前置属性 frontmatter 解析/回写)、`attach.rs`(导入文件到附件目录、去重命名、生成链接文本、图片尺寸解析) | 解析器模糊测试 + 单元测试 |
| W1-D1 | jiyue-sync: opendal 实现 S3 与 WebDAV 的 `RemoteStore`(条件写入、版本列表/按版本读取、能力探测),用 opendal memory 服务测试;另写可选的 MinIO 集成测试说明 | 全部 trait 方法有测试 |
| W1-D2 | jiyue-sync: `LocalSnapshot`/`SyncState`(本地基准快照存 `.jiyue/sync`)、`plan_sync`、`three_way_merge`(用 `similar`)、冲突副本命名、重命名检测(按哈希)、忽略规则 | 属性测试:任意操作序列收敛;冲突不丢数据 |
| W1-E | jiyue-ai: OpenAI 兼容与 Anthropic 的流式聊天、嵌入、重试/超时/取消、`keyring` 存取、向量索引(增量,按文件哈希)、RAG 检索(无嵌入时退回 jiyue-search) | 本地模拟服务器测试;无网络可跑 |
| W1-F | jiyue-app: 实时预览阶段 1(`pulldown-cmark` 语法跨度 → 源码视图里按语法着色/标题变大/行内样式,标记符号淡化但可见),只改 `ui/text_input.rs` 与新文件 `ui/md_style.rs` | 截图对照;IME/撤销/选区不回归 |

### 第 2 波(并行,界面集成;我先预接线:AppCommand、状态字段、占位模块,保证各 agent 不改 main.rs)
- 搜索面板(Ctrl+Shift+F)与文内查找替换(Ctrl+F/H)。
- 文件操作界面:新建文件夹、重命名、拖动移动、删除到回收站、右键菜单;文件监听接入(外部改动刷新、未保存冲突提示);重命名后链接更新提示。
- 附件界面:粘贴/拖入图片、渲染图片、`![[笔记]]` 嵌入、块引用、frontmatter 属性面板。
- 同步界面:S3/WebDAV 设置、状态指示、手动/自动同步、冲突处理界面、远端版本历史浏览与恢复。
- AI 界面:侧栏对话(当前笔记/选区/全库)、选区操作(改写/缩写/扩写/翻译/修正/总结,diff 预览接受/拒绝)、自动标题/标签/摘要、相关笔记推荐、设置页。
- `jiyue-mcp`:stdio MCP 服务(列出/搜索/读/建/追加/改笔记,标签,反链,今日日记)。

### 第 3 波
- 实时预览阶段 2(光标行外隐藏标记)、阶段 3(代码块/提示块/表格/图片/勾选框块级组件);阅读模式独立排版。
- Mermaid(`mermaid-rs-renderer` → SVG → 缓存 → 显示,主题色映射,出错回退)。
- 公式(先做调研 spike,给出方案再实现)。
- 模板、别名、书签、出链面板、`/` 命令菜单、分屏、快捷键自定义、脚注、导出(HTML/PDF)、版本历史本地快照。

### 第 4 波
性能与稳定性(大库、长文档)、崩溃保护、安装包、文档。

## 4. 所有 agent 的共同规则
- 只改自己名下的文件;改别人的先在报告里提出。共用编译目录:遇到锁就等,别人文件编译失败就 `sleep 60` 重试。
- 绝不 `find /` 全盘搜索;GPUI 源码在 `D:\cache\cargo\registry\src\index.crates.io-1949cf8c6b5b557f\gpui-0.2.2`。
- 运行程序前先复制 exe;用临时复制的笔记库做测试,不改 `demo-vault`。
- Python 只用 `D:\venvs\automation\Scripts\python.exe`。新依赖需写进报告并说明用途与体积。
- 每个任务必须带测试并保证 `cargo test` 全过;界面类任务必须用脚本驱动真实输入并读截图验证。
- 不做 git 提交(由用户决定)。

## 5. 当前进度与待办(2026-11-05 更新)
第 0、1、2 波、2.5 波(集成修复)已完成并提交;项目改名为 Jiyue(霁月),logo 已换。

### 进行中(按此顺序)
1. 实时预览阶段 2、3(光标行外隐藏标记;代码块/表格/提示块/图片/勾选框做成块级组件),阅读模式独立排版,Mermaid 与公式在预览中显示。
2. 接线:版本历史界面、导出入口、模板(均已完成)。`/` 行内插入菜单待定;数据库/查询视图已决定不做(这是笔记软件)。

### 已决定先不做(记在路线图里,之后再开)
3. 硬化与发布:大库与长文档性能、崩溃保护、WebDAV 与同步冲突的真实验证、同步后监听自动重载、安装包、文档;以及标签页拖动排序、输入框 Shift+Enter、分屏、书签、脚注、快捷键自定义、旧 `.stria` 数据迁移。
- 移动端:另选 UI 技术,复用 core/sync/search/ai。
