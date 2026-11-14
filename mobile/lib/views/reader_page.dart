import 'package:flutter/material.dart';
import 'package:flutter_markdown/flutter_markdown.dart';
import '../bridge/vault_service.dart';
import '../models/note.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import '../widgets/jiyue_icon.dart';
import 'editor_page.dart';

class ReaderPage extends StatefulWidget {
  final String notePath;
  final ThemeProvider themeProvider;
  final VaultService vaultService;

  const ReaderPage({
    super.key,
    required this.notePath,
    required this.themeProvider,
    required this.vaultService,
  });

  @override
  State<ReaderPage> createState() => _ReaderPageState();
}

class _ReaderPageState extends State<ReaderPage> {
  NoteDetail? _note;
  bool _loading = true;
  bool _isBookmarked = false;

  @override
  void initState() {
    super.initState();
    _loadNote();
  }

  Future<void> _loadNote() async {
    final note = await widget.vaultService.getNote(widget.notePath);
    if (mounted) {
      setState(() {
        _note = note;
        _loading = false;
      });
    }
  }

  void _openEditor() async {
    if (_note == null) return;
    final updated = await Navigator.of(context).push<bool>(
      MaterialPageRoute(
        builder: (_) => EditorPage(
          notePath: _note!.path,
          initialContent: _note!.content,
          themeProvider: widget.themeProvider,
          vaultService: widget.vaultService,
        ),
      ),
    );
    if (updated == true) {
      _loadNote();
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;

    if (_loading) {
      return Scaffold(
        backgroundColor: colors.chrome,
        body: Center(
          child: CircularProgressIndicator(color: colors.accent),
        ),
      );
    }

    if (_note == null) {
      return Scaffold(
        backgroundColor: colors.chrome,
        appBar: AppBar(backgroundColor: colors.chrome),
        body: Center(
          child: Text('笔记不存在', style: TextStyle(color: colors.textPrimary)),
        ),
      );
    }

    return Scaffold(
      backgroundColor: colors.chrome,
      appBar: AppBar(
        backgroundColor: colors.chrome,
        elevation: 0,
        leading: IconButton(
          icon: JiyueIcon('chevron-left', color: colors.textPrimary, size: 24),
          onPressed: () => Navigator.of(context).pop(),
        ),
        title: Container(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 3),
          decoration: BoxDecoration(
            color: colors.sheet,
            borderRadius: BorderRadius.circular(18),
            border: Border.all(color: colors.border),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              // 阅读态胶囊 (选中)
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                decoration: BoxDecoration(
                  color: colors.accent,
                  borderRadius: BorderRadius.circular(15),
                ),
                child: Row(
                  children: [
                    JiyueIcon('eye', color: colors.onAccent, size: 14),
                    const SizedBox(width: 4),
                    Text(
                      '阅读',
                      style: TextStyle(
                        fontSize: 12,
                        fontWeight: FontWeight.bold,
                        color: colors.onAccent,
                      ),
                    ),
                  ],
                ),
              ),
              // 编辑态胶囊 (点击切换)
              InkWell(
                onTap: _openEditor,
                borderRadius: BorderRadius.circular(15),
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                  child: Row(
                    children: [
                      JiyueIcon('pencil', color: colors.textMuted, size: 14),
                      const SizedBox(width: 4),
                      Text(
                        '编辑',
                        style: TextStyle(fontSize: 12, color: colors.textMuted),
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
        centerTitle: true,
        actions: [
          IconButton(
            icon: JiyueIcon('palette', color: colors.textSecondary, size: 20),
            onPressed: () {
              // 快速在 墨 和 雾 之间切换
              final next = widget.themeProvider.themeId == JiyueThemeId.ink
                  ? JiyueThemeId.fog
                  : JiyueThemeId.ink;
              widget.themeProvider.setTheme(next);
            },
          ),
          IconButton(
            icon: JiyueIcon('dots', color: colors.textSecondary, size: 20),
            onPressed: () {
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(content: Text('更多操作：导出、字数统计、历史版本')),
              );
            },
          ),
        ],
      ),
      body: Stack(
        children: [
          // 正文滚动区
          SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(20, 10, 20, 90),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                // 面包屑
                Row(
                  children: [
                    JiyueIcon('folder', color: colors.textMuted, size: 13),
                    const SizedBox(width: 6),
                    Text(
                      'Jiyue 知识库',
                      style: TextStyle(fontSize: 11, color: colors.textMuted),
                    ),
                  ],
                ),
                const SizedBox(height: 8),

                // 大标题
                Text(
                  _note!.title,
                  style: TextStyle(
                    fontSize: 22,
                    fontWeight: FontWeight.bold,
                    color: colors.textPrimary,
                  ),
                ),
                const SizedBox(height: 8),

                // 元信息
                Row(
                  children: [
                    JiyueIcon('clock', color: colors.textMuted, size: 12),
                    const SizedBox(width: 6),
                    Text(
                      '${_note!.content.length} 字符 · 包含 ${_note!.wikilinks.length} 个双向链接',
                      style: TextStyle(fontSize: 11, color: colors.textMuted),
                    ),
                  ],
                ),
                const SizedBox(height: 16),

                // WikiLinks 双链胶囊横向流
                if (_note!.wikilinks.isNotEmpty) ...[
                  Wrap(
                    spacing: 8,
                    runSpacing: 6,
                    children: _note!.wikilinks.map((link) {
                      return InkWell(
                        onTap: () {
                          Navigator.of(context).push(
                            MaterialPageRoute(
                              builder: (_) => ReaderPage(
                                notePath: '$link.md',
                                themeProvider: widget.themeProvider,
                                vaultService: widget.vaultService,
                              ),
                            ),
                          );
                        },
                        borderRadius: BorderRadius.circular(10),
                        child: Container(
                          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
                          decoration: BoxDecoration(
                            color: colors.accent.withAlpha(25),
                            borderRadius: BorderRadius.circular(10),
                            border: Border.all(color: colors.accent.withAlpha(80)),
                          ),
                          child: Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              JiyueIcon('link', color: colors.accent, size: 13),
                              const SizedBox(width: 6),
                              Text(
                                '[[$link]]',
                                style: TextStyle(
                                  fontSize: 12,
                                  fontWeight: FontWeight.bold,
                                  color: colors.accent,
                                ),
                              ),
                            ],
                          ),
                        ),
                      );
                    }).toList(),
                  ),
                  const SizedBox(height: 18),
                ],

                // Markdown 正文渲染
                MarkdownBody(
                  data: _note!.content,
                  selectable: true,
                  styleSheet: MarkdownStyleSheet(
                    p: TextStyle(fontSize: 14, height: 1.6, color: colors.textPrimary),
                    h1: TextStyle(fontSize: 20, fontWeight: FontWeight.bold, color: colors.textPrimary),
                    h2: TextStyle(fontSize: 17, fontWeight: FontWeight.bold, color: colors.textPrimary),
                    h3: TextStyle(fontSize: 15, fontWeight: FontWeight.bold, color: colors.textPrimary),
                    code: TextStyle(
                      fontSize: 12,
                      fontFamily: 'monospace',
                      backgroundColor: colors.raised,
                      color: colors.accent,
                    ),
                    codeblockDecoration: BoxDecoration(
                      color: colors.raised,
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(color: colors.border),
                    ),
                    blockquoteDecoration: BoxDecoration(
                      color: colors.sheet,
                      borderRadius: BorderRadius.circular(6),
                      border: Border(left: BorderSide(color: colors.accent, width: 4)),
                    ),
                    listBullet: TextStyle(color: colors.accent),
                  ),
                ),
              ],
            ),
          ),

          // 底部悬浮阅读工具栏
          Positioned(
            left: 24,
            right: 24,
            bottom: 20,
            child: Container(
              height: 48,
              decoration: BoxDecoration(
                color: colors.sheet,
                borderRadius: BorderRadius.circular(24),
                border: Border.all(color: colors.border),
                boxShadow: [
                  BoxShadow(
                    color: Colors.black.withAlpha(colors.isDark ? 80 : 25),
                    blurRadius: 12,
                    offset: const Offset(0, 4),
                  ),
                ],
              ),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                children: [
                  _readingAction('hierarchy-2', '大纲', colors.textSecondary, () {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(content: Text('大纲结构导航就绪')),
                    );
                  }),
                  _readingAction(
                    _isBookmarked ? 'bookmark-filled' : 'bookmark',
                    _isBookmarked ? '已收藏' : '收藏',
                    _isBookmarked ? colors.accent : colors.textSecondary,
                    () => setState(() => _isBookmarked = !_isBookmarked),
                  ),
                  _readingAction('affiliate', '图谱', colors.textSecondary, () {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(content: Text('已抽取当前笔记与其他笔记的拓扑关联')),
                    );
                  }),
                  _readingAction('pencil', '快速编辑', colors.accent, _openEditor),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _readingAction(String icon, String label, Color color, VoidCallback onTap) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(16),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        child: Row(
          children: [
            JiyueIcon(icon, color: color, size: 16),
            const SizedBox(width: 4),
            Text(label, style: TextStyle(fontSize: 11, color: color)),
          ],
        ),
      ),
    );
  }
}
