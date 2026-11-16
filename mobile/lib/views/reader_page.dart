import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown/flutter_markdown.dart';
import 'package:url_launcher/url_launcher.dart';
import '../bridge/vault_service.dart';
import '../models/note.dart';
import '../services/platform_bridge.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import '../widgets/ai_polish_sheet.dart';
import '../widgets/jiyue_controls.dart';
import '../widgets/jiyue_icon.dart';
import 'editor_page.dart';

class _OutlineHeading {
  final int level;
  final String text;
  final int lineIndex;

  const _OutlineHeading({
    required this.level,
    required this.text,
    required this.lineIndex,
  });
}

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
  final ScrollController _scrollController = ScrollController();

  late String _currentPath;
  NoteDetail? _note;
  bool _loading = true;
  bool _isBookmarked = false;

  @override
  void initState() {
    super.initState();
    _currentPath = widget.notePath;
    _loadNote();
  }

  @override
  void dispose() {
    _scrollController.dispose();
    super.dispose();
  }

  Future<void> _loadNote() async {
    final note = await widget.vaultService.getNote(_currentPath);
    if (mounted) {
      setState(() {
        _note = note;
        _isBookmarked =
            note != null && widget.vaultService.isBookmarked(note.path);
        _loading = false;
      });
    }
  }

  String _breadcrumbText() {
    final vault = widget.vaultService.vaultName;
    final clean = (_note?.path ?? _currentPath).replaceAll('\\', '/');
    final slash = clean.lastIndexOf('/');
    if (slash <= 0) return vault;
    final folder = clean.substring(0, slash).split('/').join(' / ');
    return '$vault / $folder';
  }

  List<_OutlineHeading> _extractHeadings(String content) {
    final lines = content.split('\n');
    final headings = <_OutlineHeading>[];
    var inFence = false;

    for (var i = 0; i < lines.length; i++) {
      final trimmed = lines[i].trimLeft();
      if (trimmed.startsWith('```') || trimmed.startsWith('~~~')) {
        inFence = !inFence;
        continue;
      }
      if (inFence) continue;
      final match = RegExp(r'^(#{1,6})\s+(.+?)\s*#*\s*$').firstMatch(trimmed);
      if (match != null) {
        headings.add(
          _OutlineHeading(
            level: match.group(1)!.length,
            text: match.group(2)!.trim(),
            lineIndex: i,
          ),
        );
      }
    }
    return headings;
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

  Future<void> _toggleBookmark() async {
    if (_note == null) return;
    final added = await widget.vaultService.toggleBookmark(_note!.path);
    if (!mounted) return;
    setState(() => _isBookmarked = added);
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(added ? '已加入当前笔记库收藏' : '已从收藏中移除')),
    );
  }

  Future<void> _openLinkedTarget(String rawTarget) async {
    final trimmed = rawTarget.trim();
    if (trimmed.isEmpty || _note == null) return;

    if (trimmed.startsWith('http://') || trimmed.startsWith('https://')) {
      final uri = Uri.tryParse(trimmed);
      if (uri != null) {
        final launched = await launchUrl(
          uri,
          mode: LaunchMode.externalApplication,
        );
        if (!launched && mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('无法打开外部链接')),
          );
        }
      }
      return;
    }

    final resolved = await widget.vaultService.resolveLink(_note!.path, trimmed);
    if (!mounted) return;

    if (resolved != null) {
      await Navigator.of(context).push(
        MaterialPageRoute(
          builder: (_) => ReaderPage(
            notePath: resolved,
            themeProvider: widget.themeProvider,
            vaultService: widget.vaultService,
          ),
        ),
      );
      _loadNote();
      return;
    }

    // Target does not exist yet: offer to create it.
    final beforePipe = trimmed.split('|').first;
    final cleanTitle =
        beforePipe.split('#').first.replaceAll(RegExp(r'\.md$', caseSensitive: false), '').trim();
    if (cleanTitle.isEmpty) return;

    final colors = widget.themeProvider.colors;
    final shouldCreate = await showDialog<bool>(
      context: context,
      builder: (ctx) {
        return JiyueDialog(
          colors: colors,
          icon: 'file-plus',
          title: '双链目标尚未创建',
          content: Text(
            '笔记《$cleanTitle》尚未创建，是否立即创建并进入编辑？',
            style: TextStyle(
              fontSize: JiyueTokens.fontBase,
              color: colors.textSecondary,
              height: 1.45,
            ),
          ),
          actions: [
            JiyueButton(
              colors: colors,
              label: '取消',
              kind: JiyueButtonKind.secondary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(false),
            ),
            const SizedBox(width: 8),
            JiyueButton(
              colors: colors,
              label: '创建笔记',
              icon: 'plus',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(true),
            ),
          ],
        );
      },
    );

    if (shouldCreate == true && mounted) {
      final created = await widget.vaultService.createNote(
        cleanTitle,
        '# $cleanTitle\n\n',
      );
      if (created != null && mounted) {
        await Navigator.of(context).push(
          MaterialPageRoute(
            builder: (_) => EditorPage(
              notePath: created.path,
              initialContent: '# $cleanTitle\n\n',
              themeProvider: widget.themeProvider,
              vaultService: widget.vaultService,
            ),
          ),
        );
        _loadNote();
      }
    }
  }

  void _openOutlineSheet() {
    if (_note == null) return;
    final colors = widget.themeProvider.colors;
    final content = _note!.content;
    final totalLines = content.split('\n').length.clamp(1, 1000000);
    final headings = _extractHeadings(content);

    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (ctx) {
        return JiyueModalSheet(
          colors: colors,
          icon: 'hierarchy-2',
          title: '文档大纲',
          subtitle: '共 ${headings.length} 个章节标题，点击快速定位',
          child: headings.isEmpty
              ? JiyueEmptyState(
                  colors: colors,
                  icon: 'hierarchy-2',
                  title: '当前笔记暂无各级标题',
                  subtitle: '在正文中使用 # 语法添加标题后即可生成大纲',
                )
              : Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: headings.map((h) {
                    return Padding(
                      padding: EdgeInsets.only(
                        left: ((h.level - 1) * 14).toDouble(),
                        bottom: 6,
                      ),
                      child: JiyueSectionCard(
                        colors: colors,
                        padding: const EdgeInsets.symmetric(
                          horizontal: 12,
                          vertical: 9,
                        ),
                        onTap: () {
                          Navigator.of(ctx).pop();
                          if (_scrollController.hasClients) {
                            final max =
                                _scrollController.position.maxScrollExtent;
                            final ratio =
                                (h.lineIndex / totalLines).clamp(0.0, 1.0);
                            _scrollController.animateTo(
                              max * ratio,
                              duration: const Duration(milliseconds: 260),
                              curve: Curves.easeOutCubic,
                            );
                          }
                        },
                        child: Row(
                          children: [
                            JiyueBadge(
                              colors: colors,
                              label: 'H${h.level}',
                              tone: h.level <= 2
                                  ? JiyueBadgeTone.accent
                                  : JiyueBadgeTone.neutral,
                            ),
                            const SizedBox(width: 8),
                            Expanded(
                              child: Text(
                                h.text,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(
                                  fontSize: JiyueTokens.fontBase,
                                  fontWeight: h.level == 1
                                      ? FontWeight.w600
                                      : FontWeight.w500,
                                  color: colors.textPrimary,
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
                    );
                  }).toList(),
                ),
        );
      },
    );
  }

  void _openLinksSheet() {
    if (_note == null) return;
    final colors = widget.themeProvider.colors;
    final backlinks = _note!.backlinks;
    final outlinks = _note!.wikilinks;

    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (ctx) {
        return JiyueModalSheet(
          colors: colors,
          icon: 'affiliate',
          title: '双向链接关联',
          subtitle: '反链 ${backlinks.length} 篇 · 出链 ${outlinks.length} 个',
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              JiyueSectionHeader(
                colors: colors,
                title: '反向链接（引用了当前笔记）',
              ),
              if (backlinks.isEmpty)
                JiyueBanner(
                  colors: colors,
                  tone: JiyueBadgeTone.neutral,
                  message: '暂无其他笔记引用当前文档',
                )
              else
                ...backlinks.map((item) {
                  return JiyueSectionCard(
                    colors: colors,
                    margin: const EdgeInsets.only(bottom: 8),
                    padding: const EdgeInsets.symmetric(
                      horizontal: 12,
                      vertical: 10,
                    ),
                    onTap: () {
                      Navigator.of(ctx).pop();
                      _openLinkedTarget(item.path);
                    },
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          children: [
                            JiyueIcon('file-text', color: colors.accent, size: 14),
                            const SizedBox(width: 6),
                            Expanded(
                              child: Text(
                                item.title,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(
                                  fontSize: JiyueTokens.fontBase,
                                  fontWeight: FontWeight.w600,
                                  color: colors.textPrimary,
                                ),
                              ),
                            ),
                            Text(
                              item.path,
                              style: TextStyle(
                                fontSize: JiyueTokens.fontXs,
                                color: colors.textMuted,
                              ),
                            ),
                          ],
                        ),
                        if (item.snippet.isNotEmpty) ...[
                          const SizedBox(height: 4),
                          Text(
                            item.snippet,
                            maxLines: 2,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontSize: JiyueTokens.fontXs,
                              color: colors.textSecondary,
                              height: 1.4,
                            ),
                          ),
                        ],
                      ],
                    ),
                  );
                }),

              const SizedBox(height: 14),
              JiyueSectionHeader(
                colors: colors,
                title: '正向出链（当前笔记指向）',
              ),
              if (outlinks.isEmpty)
                JiyueBanner(
                  colors: colors,
                  tone: JiyueBadgeTone.neutral,
                  message: '当前笔记未包含 [[双链]] 引用',
                )
              else
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: outlinks.map((target) {
                    return JiyueTagChip(
                      colors: colors,
                      icon: 'link',
                      accent: true,
                      label: '[[$target]]',
                      onTap: () {
                        Navigator.of(ctx).pop();
                        _openLinkedTarget(target);
                      },
                    );
                  }).toList(),
                ),
            ],
          ),
        );
      },
    );
  }

  String _formatModifiedTime(int ms) {
    if (ms <= 0) return '未知';
    final dt = DateTime.fromMillisecondsSinceEpoch(ms);
    String two(int n) => n.toString().padLeft(2, '0');
    return '${dt.year}-${two(dt.month)}-${two(dt.day)} ${two(dt.hour)}:${two(dt.minute)}';
  }

  Future<void> _openMoreActionsSheet() async {
    if (_note == null) return;
    final colors = widget.themeProvider.colors;
    final note = _note!;
    final content = note.content;
    final totalChars = content.length;
    final nonSpaceChars = content.replaceAll(RegExp(r'\s+'), '').length;
    final lineCount = content.isEmpty ? 0 : content.split('\n').length;
    final readMinutes = (nonSpaceChars / 350).ceil().clamp(1, 999);

    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (ctx) {
        return JiyueModalSheet(
          colors: colors,
          icon: 'dots',
          title: '笔记详情与操作',
          subtitle: note.path,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              JiyueSectionHeader(
                colors: colors,
                title: '字数统计与元信息',
                padding: const EdgeInsets.only(bottom: 6),
              ),
              JiyueSectionCard(
                colors: colors,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    _statRow(colors, '总字符 / 有效字符', '$totalChars / $nonSpaceChars 字'),
                    const SizedBox(height: 6),
                    _statRow(colors, '总行数 / 预计阅读', '$lineCount 行 · 约 $readMinutes 分钟'),
                    const SizedBox(height: 6),
                    _statRow(
                      colors,
                      '双向链接',
                      '出链 ${note.wikilinks.length} 个 · 反链 ${note.backlinks.length} 篇',
                    ),
                    const SizedBox(height: 6),
                    _statRow(
                      colors,
                      '修改时间',
                      _formatModifiedTime(note.modifiedMs),
                    ),
                    const SizedBox(height: 6),
                    _statRow(colors, '库内路径', note.path),
                  ],
                ),
              ),

              const SizedBox(height: 14),
              JiyueSectionHeader(
                colors: colors,
                title: '文档操作',
                padding: const EdgeInsets.only(bottom: 8),
              ),
              Row(
                children: [
                  Expanded(
                    child: JiyueButton(
                      colors: colors,
                      label: '复制全文',
                      icon: 'copy',
                      kind: JiyueButtonKind.secondary,
                      size: JiyueControlSize.md,
                      expand: true,
                      onPressed: () {
                        Clipboard.setData(ClipboardData(text: content));
                        Navigator.of(ctx).pop();
                        ScaffoldMessenger.of(context).showSnackBar(
                          const SnackBar(content: Text('已复制 Markdown 全文到剪贴板')),
                        );
                      },
                    ),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: JiyueButton(
                      colors: colors,
                      label: '导出分享',
                      icon: 'download',
                      kind: JiyueButtonKind.secondary,
                      size: JiyueControlSize.md,
                      expand: true,
                      onPressed: () async {
                        Navigator.of(ctx).pop();
                        final sep = Platform.pathSeparator;
                        final fullPath =
                            '${widget.vaultService.vaultPath}$sep${note.path.split('/').join(sep)}';
                        final shared = await PlatformBridge.shareFile(
                          fullPath,
                          title: '${note.title}.md',
                        );
                        if (!shared && mounted) {
                          Clipboard.setData(ClipboardData(text: fullPath));
                          ScaffoldMessenger.of(context).showSnackBar(
                            const SnackBar(
                              content: Text('当前环境不支持系统分享，已复制文件路径'),
                            ),
                          );
                        }
                      },
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 10),
              Row(
                children: [
                  Expanded(
                    child: JiyueButton(
                      colors: colors,
                      label: '重命名',
                      icon: 'pencil',
                      kind: JiyueButtonKind.secondary,
                      size: JiyueControlSize.md,
                      expand: true,
                      onPressed: () {
                        Navigator.of(ctx).pop();
                        _renameCurrentNote();
                      },
                    ),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: JiyueButton(
                      colors: colors,
                      label: '删除笔记',
                      icon: 'trash',
                      kind: JiyueButtonKind.danger,
                      size: JiyueControlSize.md,
                      expand: true,
                      onPressed: () {
                        Navigator.of(ctx).pop();
                        _confirmDeleteCurrentNote();
                      },
                    ),
                  ),
                ],
              ),
            ],
          ),
        );
      },
    );
  }

  Widget _statRow(JiyueThemeColors colors, String label, String value) {
    return Row(
      children: [
        Text(
          label,
          style: TextStyle(
            fontSize: JiyueTokens.fontXs,
            color: colors.textMuted,
          ),
        ),
        const SizedBox(width: 12),
        Expanded(
          child: Text(
            value,
            textAlign: TextAlign.right,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontSize: JiyueTokens.fontSm,
              fontWeight: FontWeight.w500,
              color: colors.textPrimary,
            ),
          ),
        ),
      ],
    );
  }

  Future<void> _renameCurrentNote() async {
    if (_note == null) return;
    final colors = widget.themeProvider.colors;
    final ctrl = TextEditingController(text: _note!.title);

    final newTitle = await showDialog<String>(
      context: context,
      builder: (ctx) {
        return JiyueDialog(
          colors: colors,
          icon: 'pencil',
          title: '重命名笔记',
          content: JiyueTextField(
            colors: colors,
            controller: ctrl,
            autofocus: true,
            label: '新笔记标题',
            hintText: '输入新的标题...',
            onSubmitted: (v) => Navigator.of(ctx).pop(v.trim()),
          ),
          actions: [
            JiyueButton(
              colors: colors,
              label: '取消',
              kind: JiyueButtonKind.secondary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(null),
            ),
            const SizedBox(width: 8),
            JiyueButton(
              colors: colors,
              label: '保存',
              icon: 'check',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(ctrl.text.trim()),
            ),
          ],
        );
      },
    );

    if (newTitle == null || newTitle.isEmpty || newTitle == _note!.title) {
      return;
    }

    final renamedPath = await widget.vaultService.renameNote(
      _note!.path,
      newTitle,
    );
    if (!mounted) return;
    if (renamedPath != null) {
      _currentPath = renamedPath;
      await _loadNote();
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('已重命名为《$newTitle》')),
        );
      }
    } else {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('重命名失败：可能存在同名文件或标题包含非法字符')),
      );
    }
  }

  Future<void> _confirmDeleteCurrentNote() async {
    if (_note == null) return;
    final colors = widget.themeProvider.colors;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) {
        return JiyueDialog(
          colors: colors,
          icon: 'trash',
          title: '删除笔记',
          content: Text(
            '确定永久删除《${_note!.title}》吗？该操作不可撤销。',
            style: TextStyle(
              fontSize: JiyueTokens.fontBase,
              color: colors.textSecondary,
              height: 1.45,
            ),
          ),
          actions: [
            JiyueButton(
              colors: colors,
              label: '取消',
              kind: JiyueButtonKind.secondary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(false),
            ),
            const SizedBox(width: 8),
            JiyueButton(
              colors: colors,
              label: '确认删除',
              icon: 'trash',
              kind: JiyueButtonKind.danger,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(true),
            ),
          ],
        );
      },
    );

    if (confirmed == true && mounted) {
      final ok = await widget.vaultService.deleteNote(_note!.path);
      if (mounted) {
        if (ok) {
          Navigator.of(context).pop(true);
        } else {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('删除失败，请检查写入权限')),
          );
        }
      }
    }
  }

  void _openAiPolish() {
    if (_note == null || _note!.content.trim().isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('笔记内容为空，无法处理')),
      );
      return;
    }
    showModalBottomSheet<void>(
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
          subtitle: '该笔记可能已被移动或删除（$_currentPath）',
        ),
      );
    }

    final totalLinks = _note!.wikilinks.length + _note!.backlinks.length;

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
              tooltip: '笔记详情与更多操作',
              onPressed: _openMoreActionsSheet,
            ),
          ),
        ],
      ),
      body: SingleChildScrollView(
        controller: _scrollController,
        padding: const EdgeInsets.fromLTRB(18, 16, 18, 28),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // 真实库名与子目录面包屑
            Row(
              children: [
                JiyueIcon('folder', color: colors.textMuted, size: 13),
                const SizedBox(width: 6),
                Expanded(
                  child: Text(
                    _breadcrumbText(),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: JiyueTokens.fontXs,
                      color: colors.textMuted,
                    ),
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
                  '${_note!.content.length} 字符 · 出链 ${_note!.wikilinks.length} · 反链 ${_note!.backlinks.length}',
                  style: TextStyle(
                    fontSize: JiyueTokens.fontXs,
                    color: colors.textMuted,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 14),

            // WikiLinks 双链胶囊流（智能解析跳转）
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
                    onTap: () => _openLinkedTarget(link),
                  );
                }).toList(),
              ),
              const SizedBox(height: 14),
            ],

            JiyueDivider(colors: colors),
            const SizedBox(height: 14),

            // Markdown 正文渲染（支持库内路径和外链点击）
            MarkdownBody(
              data: _note!.content,
              selectable: true,
              onTapLink: (_, href, _) {
                if (href != null && href.isNotEmpty) {
                  _openLinkedTarget(href);
                }
              },
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
                  onTap: _openOutlineSheet,
                ),
                _readingAction(
                  colors: colors,
                  icon: _isBookmarked ? 'bookmark-filled' : 'bookmark',
                  label: _isBookmarked ? '已收藏' : '收藏',
                  color: _isBookmarked ? colors.accent : colors.textSecondary,
                  onTap: _toggleBookmark,
                ),
                _readingAction(
                  colors: colors,
                  icon: 'affiliate',
                  label: totalLinks > 0 ? '双链 ($totalLinks)' : '双链',
                  color: colors.textSecondary,
                  onTap: _openLinksSheet,
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
                Flexible(
                  child: Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: JiyueTokens.fontSm,
                      fontWeight: FontWeight.w500,
                      color: color,
                    ),
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
