import 'package:flutter/material.dart';
import 'package:flutter_markdown/flutter_markdown.dart';
import '../bridge/vault_service.dart';
import '../models/note.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import '../widgets/ai_polish_sheet.dart';
import '../widgets/jiyue_controls.dart';
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

  Future<void> _openEditor() async {
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

  void _openAiPolish() {
    if (_note == null || _note!.content.trim().isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('笔记内容为空，无法处理')),
      );
      return;
    }
    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (_) => AiPolishSheet(
        initialText: _note!.content,
        vaultService: widget.vaultService,
        themeProvider: widget.themeProvider,
        onApply: (polished, isReplace) async {
          final newContent =
              isReplace ? polished : '${_note!.content}\n\n$polished';
          final ok = await widget.vaultService.saveNote(_note!.path, newContent);
          if (mounted) {
            if (ok) {
              _loadNote();
              ScaffoldMessenger.of(context).showSnackBar(
                SnackBar(
                  content: Text(isReplace ? '已更新笔记内容' : '已插入内容到末尾'),
                ),
              );
            } else {
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(content: Text('保存更新失败')),
              );
            }
          }
        },
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;

    if (_loading) {
      return Scaffold(
        backgroundColor: colors.surface,
        body: Center(
          child: CircularProgressIndicator(color: colors.accent),
        ),
      );
    }

    if (_note == null) {
      return Scaffold(
        backgroundColor: colors.surface,
        appBar: AppBar(
          backgroundColor: colors.panel,
          surfaceTintColor: Colors.transparent,
          elevation: 0,
          leading: Center(
            child: JiyueIconButton(
              colors: colors,
              icon: 'chevron-left',
              size: 34,
              iconSize: 18,
              onPressed: () => Navigator.of(context).pop(),
            ),
          ),
          bottom: PreferredSize(
            preferredSize: const Size.fromHeight(1),
            child: JiyueDivider(colors: colors),
          ),
        ),
        body: JiyueEmptyState(
          colors: colors,
          icon: 'file-text',
          title: '笔记不存在',
          subtitle: '该笔记可能已被移动或删除',
        ),
      );
    }

    return Scaffold(
      backgroundColor: colors.surface,
      appBar: AppBar(
        backgroundColor: colors.panel,
        surfaceTintColor: Colors.transparent,
        scrolledUnderElevation: 0,
        elevation: 0,
        centerTitle: true,
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(1),
          child: JiyueDivider(colors: colors),
        ),
        leading: Center(
          child: JiyueIconButton(
            colors: colors,
            icon: 'chevron-left',
            size: 34,
            iconSize: 18,
            tooltip: '返回',
            onPressed: () => Navigator.of(context).pop(),
          ),
        ),
        title: JiyueSegmented(
          colors: colors,
          items: const [
            ('阅读', 'eye'),
            ('编辑', 'pencil'),
          ],
          selectedIndex: 0,
          onChanged: (idx) {
            if (idx == 1) _openEditor();
          },
        ),
        actions: [
          JiyueIconButton(
            colors: colors,
            icon: 'sparkles',
            size: 32,
            iconSize: 17,
            tooltip: 'AI 智能润色 / 摘要',
            onPressed: _openAiPolish,
          ),
          JiyueIconButton(
            colors: colors,
            icon: 'palette',
            size: 32,
            iconSize: 17,
            tooltip: '切换墨/雾主题',
            onPressed: () {
              final next = widget.themeProvider.themeId == JiyueThemeId.ink
                  ? JiyueThemeId.fog
                  : JiyueThemeId.ink;
              widget.themeProvider.setTheme(next);
            },
          ),
          Padding(
            padding: const EdgeInsets.only(right: 8),
            child: JiyueIconButton(
              colors: colors,
              icon: 'dots',
              size: 32,
              iconSize: 17,
              tooltip: '更多操作',
              onPressed: () {
                ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(content: Text('更多操作：导出、字数统计、历史版本')),
                );
              },
            ),
          ),
        ],
      ),
      body: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(18, 16, 18, 28),
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
                  style: TextStyle(
                    fontSize: JiyueTokens.fontXs,
                    color: colors.textMuted,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),

            // 大标题
            Text(
              _note!.title,
              style: TextStyle(
                fontSize: JiyueTokens.fontXl,
                fontWeight: FontWeight.w700,
                color: colors.textPrimary,
              ),
            ),
            const SizedBox(height: 6),

            // 元信息
            Row(
              children: [
                JiyueIcon('clock', color: colors.textMuted, size: 12),
                const SizedBox(width: 6),
                Text(
                  '${_note!.content.length} 字符 · 包含 ${_note!.wikilinks.length} 个双向链接',
                  style: TextStyle(
                    fontSize: JiyueTokens.fontXs,
                    color: colors.textMuted,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 14),

            // WikiLinks 双链胶囊流
            if (_note!.wikilinks.isNotEmpty) ...[
              Wrap(
                spacing: 6,
                runSpacing: 6,
                children: _note!.wikilinks.map((link) {
                  return JiyueTagChip(
                    colors: colors,
                    icon: 'link',
                    accent: true,
                    label: '[[$link]]',
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
                  );
                }).toList(),
              ),
              const SizedBox(height: 14),
            ],

            JiyueDivider(colors: colors),
            const SizedBox(height: 14),

            // Markdown 正文渲染
            MarkdownBody(
              data: _note!.content,
              selectable: true,
              styleSheet: MarkdownStyleSheet(
                p: TextStyle(
                  fontSize: JiyueTokens.fontMd,
                  height: 1.65,
                  color: colors.textPrimary,
                ),
                h1: TextStyle(
                  fontSize: JiyueTokens.fontXl,
                  fontWeight: FontWeight.w700,
                  color: colors.textPrimary,
                ),
                h2: TextStyle(
                  fontSize: JiyueTokens.fontLg,
                  fontWeight: FontWeight.w600,
                  color: colors.textPrimary,
                ),
                h3: TextStyle(
                  fontSize: JiyueTokens.fontMd,
                  fontWeight: FontWeight.w600,
                  color: colors.textPrimary,
                ),
                code: TextStyle(
                  fontSize: JiyueTokens.fontSm,
                  fontFamily: 'monospace',
                  backgroundColor: colors.elevated,
                  color: colors.accent,
                ),
                codeblockDecoration: BoxDecoration(
                  color: colors.elevated,
                  borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
                  border: Border.all(color: colors.borderSubtle, width: 1),
                ),
                blockquoteDecoration: BoxDecoration(
                  color: colors.panel,
                  borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                  border: Border(
                    left: BorderSide(color: colors.accent, width: 3),
                  ),
                ),
                listBullet: TextStyle(color: colors.accent),
              ),
            ),
          ],
        ),
      ),
      bottomNavigationBar: Container(
        decoration: BoxDecoration(
          color: colors.panel,
          border: Border(
            top: BorderSide(color: colors.borderSubtle, width: 1),
          ),
        ),
        child: SafeArea(
          top: false,
          child: SizedBox(
            height: 48,
            child: Row(
              children: [
                _readingAction(
                  colors: colors,
                  icon: 'hierarchy-2',
                  label: '大纲',
                  color: colors.textSecondary,
                  onTap: () {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(content: Text('大纲结构导航就绪')),
                    );
                  },
                ),
                _readingAction(
                  colors: colors,
                  icon: _isBookmarked ? 'bookmark-filled' : 'bookmark',
                  label: _isBookmarked ? '已收藏' : '收藏',
                  color: _isBookmarked ? colors.accent : colors.textSecondary,
                  onTap: () => setState(() => _isBookmarked = !_isBookmarked),
                ),
                _readingAction(
                  colors: colors,
                  icon: 'affiliate',
                  label: '图谱',
                  color: colors.textSecondary,
                  onTap: () {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(content: Text('已抽取当前笔记与其他笔记的拓扑关联')),
                    );
                  },
                ),
                _readingAction(
                  colors: colors,
                  icon: 'pencil',
                  label: '编辑',
                  color: colors.accent,
                  onTap: _openEditor,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _readingAction({
    required JiyueThemeColors colors,
    required String icon,
    required String label,
    required Color color,
    required VoidCallback onTap,
  }) {
    return Expanded(
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
          splashColor: colors.active,
          highlightColor: colors.hover,
          child: SizedBox(
            height: double.infinity,
            child: Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                JiyueIcon(icon, color: color, size: 15),
                const SizedBox(width: 5),
                Text(
                  label,
                  style: TextStyle(
                    fontSize: JiyueTokens.fontSm,
                    fontWeight: FontWeight.w500,
                    color: color,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
