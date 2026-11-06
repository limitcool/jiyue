霁月 Jiyue 便携版
====================

本地优先的 Markdown 笔记应用。笔记是普通的 .md 文件，可以直接打开 Obsidian 的库文件夹。

文件
  jiyue.exe        应用程序
  jiyue-mcp.exe    MCP 服务（让 AI 助手读写笔记库，见项目文档 docs/mcp.md）

运行
  双击 jiyue.exe，或在终端中运行：
    jiyue.exe --vault "D:\我的笔记"
  --vault 指定笔记库文件夹，不存在时会创建。不指定时使用当前目录下的 vault 文件夹。
  --theme paper|ink|moss|graphite|fog 选择主题。

便携版不会安装快捷方式或修改 PATH。应用的数据保存在笔记库的 .jiyue 文件夹和
%LOCALAPPDATA%\Jiyue\cache 中；只有在使用 AI 或同步时，密钥才会以明文保存在 credentials.toml 中（用户配置目录，仅当前用户可读）。
删除程序文件夹即可卸载，笔记不受影响。

系统要求：Windows 10 或 11，64 位。
