import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/note.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import '../widgets/bottom_bar.dart';
import '../widgets/daily_card.dart';
import '../widgets/jiyue_controls.dart';
import '../widgets/jiyue_icon.dart';
import '../widgets/note_card.dart';
import '../widgets/settings_sheet.dart';
import 'editor_page.dart';
import 'reader_page.dart';

class VaultPage extends StatefulWidget {
  final ThemeProvider themeProvider;
  final VaultService vaultService;

  const VaultPage({
    super.key,
    required this.themeProvider,
    required this.vaultService,
  });

  @override
  State<VaultPage> createState() => _VaultPageState();
}

class _VaultPageState extends State<VaultPage> {
  final TextEditingController _searchController = TextEditingController();
  final FocusNode _searchFocusNode = FocusNode();

  int _currentTab = 0;
  List<NoteSummary> _notes = [];
  String _selectedTag = '全部';
  String _searchQuery = '';
  bool _syncing = false;
  String _syncStatusText = '已同步';

  @override
  void initState() {
    super.initState();
    _refreshNotes();
  }

  @override
  void dispose() {
    _searchController.dispose();
    _searchFocusNode.dispose();
    super.dispose();
  }

  Future<void> _refreshNotes() async {
    final list = await widget.vaultService.listNotes();
    if (mounted) {
      setState(() {
        _notes = list;
      });
    }
  }

  Future<void> _triggerSync() async {
    setState(() {
      _syncing = true;
      _syncStatusText = '同步中...';
    });

    await Future.delayed(const Duration(milliseconds: 1200));

    if (mounted) {
      setState(() {
        _syncing = false;
        _syncStatusText = '已同步';
      });
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('S3 / WebDAV 云端同步完成')),
      );
    }
  }

  Future<void> _openDailyQuickLog() async {
    final controller = TextEditingController();
    final colors = widget.themeProvider.colors;

    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (ctx) {
        return JiyueModalSheet(
          colors: colors,
          icon: 'calendar-event',
          title: '追加今日速记随笔',
          subtitle: '内容将自动附带时间戳写入今日日记',
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              JiyueTextField(
                colors: colors,
                controller: controller,
                autofocus: true,
                maxLines: 4,
                hintText: '记录此刻灵感、待办或备忘...',
              ),
              const SizedBox(height: 14),
              JiyueButton(
                colors: colors,
                label: '追加到今日笔记',
                icon: 'check',
                kind: JiyueButtonKind.primary,
                size: JiyueControlSize.lg,
                expand: true,
                onPressed: () async {
                  final text = controller.text.trim();
                  if (text.isNotEmpty) {
                    await widget.vaultService.appendToDaily(text);
                    if (ctx.mounted) Navigator.of(ctx).pop();
                    _refreshNotes();
                  }
                },
              ),
            ],
          ),
        );
      },
    );
  }

  Future<void> _createNewNote() async {
    final titleController = TextEditingController();
    final colors = widget.themeProvider.colors;

    final title = await showDialog<String>(
      context: context,
      builder: (ctx) {
        return JiyueDialog(
          colors: colors,
          icon: 'file-plus',
          title: '新建笔记',
          content: JiyueTextField(
            colors: colors,
            controller: titleController,
            autofocus: true,
            label: '笔记标题',
            hintText: '输入笔记标题...',
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
              label: '创建',
              icon: 'check',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              onPressed: () =>
                  Navigator.of(ctx).pop(titleController.text.trim()),
            ),
          ],
        );
      },
    );

    if (title != null && title.isNotEmpty) {
      final note = await widget.vaultService.createNote(title, '# $title\n\n');
      if (note != null && mounted) {
        _refreshNotes();
        Navigator.of(context)
            .push(
              MaterialPageRoute(
                builder: (_) => EditorPage(
                  notePath: note.path,
                  initialContent: '# $title\n\n',
                  themeProvider: widget.themeProvider,
                  vaultService: widget.vaultService,
                ),
              ),
            )
            .then((_) => _refreshNotes());
      }
    }
  }

  void _openSettings() {
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (_) => SettingsModalSheet(
        themeProvider: widget.themeProvider,
        vaultService: widget.vaultService,
        noteCount: _notes.length,
      ),
    ).then((_) {
      if (mounted) {
        setState(() => _currentTab = 0);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;

    final filteredNotes = _notes.where((n) {
      final matchesSearch =
          _searchQuery.isEmpty ||
          n.title.toLowerCase().contains(_searchQuery.toLowerCase()) ||
          n.preview.toLowerCase().contains(_searchQuery.toLowerCase()) ||
          n.tags.any(
            (t) => t.toLowerCase().contains(_searchQuery.toLowerCase()),
          );
      final matchesTag =
          _selectedTag == '全部' || n.tags.contains(_selectedTag);
      return matchesSearch && matchesTag;
    }).toList();

    return Scaffold(
      backgroundColor: colors.canvas,
      appBar: AppBar(
        backgroundColor: colors.panel,
        surfaceTintColor: Colors.transparent,
        scrolledUnderElevation: 0,
        elevation: 0,
        titleSpacing: 16,
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(1),
          child: JiyueDivider(colors: colors),
        ),
        title: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              height: JiyueTokens.controlMd,
              padding: const EdgeInsets.symmetric(horizontal: 10),
              decoration: BoxDecoration(
                color: colors.elevated,
                borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
                border: Border.all(color: colors.border, width: 1),
              ),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Image.asset(
                    'assets/logo/icon-256.png',
                    width: 18,
                    height: 18,
                  ),
                  const SizedBox(width: 8),
                  Text(
                    widget.vaultService.vaultName,
                    style: TextStyle(
                      fontSize: JiyueTokens.fontBase,
                      fontWeight: FontWeight.w600,
                      color: colors.textPrimary,
                    ),
                  ),
                  const SizedBox(width: 6),
                  JiyueIcon(
                    'selector',
                    color: colors.textMuted,
                    size: 13,
                  ),
                ],
              ),
            ),
          ],
        ),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 16),
            child: Center(
              child: JiyueButton(
                colors: colors,
                label: _syncStatusText,
                icon: 'refresh',
                loading: _syncing,
                kind: JiyueButtonKind.secondary,
                size: JiyueControlSize.xs,
                onPressed: _syncing ? null : _triggerSync,
              ),
            ),
          ),
        ],
      ),
      body: Stack(
        children: [
          RefreshIndicator(
            color: colors.accent,
            backgroundColor: colors.surface,
            onRefresh: _refreshNotes,
            child: ListView(
              padding: const EdgeInsets.only(top: 10, bottom: 88),
              children: [
                // 搜索栏
                Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 16,
                    vertical: 4,
                  ),
                  child: JiyueTextField(
                    colors: colors,
                    controller: _searchController,
                    focusNode: _searchFocusNode,
                    prefixIcon: 'search',
                    hintText: '搜索笔记正文、标题或 #标签...',
                    onChanged: (v) => setState(() => _searchQuery = v),
                    suffix: _searchQuery.isNotEmpty
                        ? JiyueIconButton(
                            colors: colors,
                            icon: 'x',
                            size: 28,
                            iconSize: 14,
                            tooltip: '清空搜索',
                            onPressed: () {
                              _searchController.clear();
                              setState(() => _searchQuery = '');
                            },
                          )
                        : null,
                  ),
                ),

                const SizedBox(height: 8),

                // 快速标签横向筛选流
                SizedBox(
                  height: 30,
                  child: ListView(
                    scrollDirection: Axis.horizontal,
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    children: ['全部', '#架构', '#Rust', '#日记', '#同步', '#设计']
                        .map((tag) {
                      final isSelected = _selectedTag == tag;
                      return Padding(
                        padding: const EdgeInsets.only(right: 6),
                        child: JiyueChoiceChip(
                          colors: colors,
                          label: tag,
                          selected: isSelected,
                          onTap: () => setState(() => _selectedTag = tag),
                        ),
                      );
                    }).toList(),
                  ),
                ),

                const SizedBox(height: 6),

                // 今日速记 Hero 卡片
                DailyHeroCard(
                  colors: colors,
                  onTapRecord: _openDailyQuickLog,
                ),

                // 列表小标题
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 12, 16, 6),
                  child: JiyueSectionHeader(
                    colors: colors,
                    title: '最近编辑 (${filteredNotes.length} 篇)',
                    padding: EdgeInsets.zero,
                    trailing: JiyueBadge(
                      colors: colors,
                      label: colors.name,
                      tone: JiyueBadgeTone.neutral,
                    ),
                  ),
                ),

                // 笔记列表
                if (filteredNotes.isEmpty)
                  JiyueEmptyState(
                    colors: colors,
                    icon: 'file-text',
                    title: '未找到相关笔记',
                    subtitle: _searchQuery.isNotEmpty || _selectedTag != '全部'
                        ? '尝试调整搜索关键词或切换其他标签'
                        : '点击右下角按钮创建第一篇笔记',
                  )
                else
                  ...filteredNotes.map((note) {
                    return NoteCard(
                      note: note,
                      colors: colors,
                      onTap: () {
                        Navigator.of(context)
                            .push(
                              MaterialPageRoute(
                                builder: (_) => ReaderPage(
                                  notePath: note.path,
                                  themeProvider: widget.themeProvider,
                                  vaultService: widget.vaultService,
                                ),
                              ),
                            )
                            .then((_) => _refreshNotes());
                      },
                    );
                  }),
              ],
            ),
          ),

          // 右下角克制悬浮新建按钮
          Positioned(
            right: 16,
            bottom: 16,
            child: JiyueButton(
              colors: colors,
              label: '新建笔记',
              icon: 'plus',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.lg,
              onPressed: _createNewNote,
            ),
          ),
        ],
      ),
      bottomNavigationBar: JiyueBottomBar(
        currentIndex: _currentTab,
        colors: colors,
        onTabSelected: (idx) {
          if (idx == 0) {
            _searchFocusNode.unfocus();
            setState(() => _currentTab = 0);
          } else if (idx == 1) {
            setState(() => _currentTab = 1);
            _openDailyQuickLog().then((_) {
              if (mounted) {
                setState(() => _currentTab = 0);
              }
            });
          } else if (idx == 2) {
            setState(() => _currentTab = 2);
            _searchFocusNode.requestFocus();
          } else if (idx == 3) {
            setState(() => _currentTab = 3);
            _openSettings();
          }
        },
      ),
    );
  }
}
