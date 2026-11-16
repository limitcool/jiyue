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
import '../widgets/s3_config_sheet.dart';
import '../widgets/settings_sheet.dart';
import '../widgets/vault_switcher_sheet.dart';
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
  static const String _kAllFilter = '全部';
  static const String _kBookmarkedFilter = '已收藏';

  final TextEditingController _searchController = TextEditingController();
  final FocusNode _searchFocusNode = FocusNode();

  int _currentTab = 0;
  List<NoteSummary> _notes = [];
  Set<String>? _contentSearchPaths;
  String _selectedFilter = _kAllFilter;
  String _searchQuery = '';
  bool _syncing = false;
  String? _lastVaultId;

  @override
  void initState() {
    super.initState();
    _lastVaultId = widget.vaultService.activeVault?.id;
    widget.vaultService.addListener(_onVaultChanged);
    _refreshNotes();
  }

  @override
  void dispose() {
    widget.vaultService.removeListener(_onVaultChanged);
    _searchController.dispose();
    _searchFocusNode.dispose();
    super.dispose();
  }

  void _onVaultChanged() {
    if (!mounted) return;
    final currentId = widget.vaultService.activeVault?.id;
    if (currentId != _lastVaultId) {
      _lastVaultId = currentId;
      _selectedFilter = _kAllFilter;
      _contentSearchPaths = null;
    }
    _refreshNotes();
  }

  Future<void> _refreshNotes() async {
    final list = await widget.vaultService.listNotes();
    if (mounted) {
      setState(() {
        _notes = list;
      });
    }
  }

  Future<void> _onSearchChanged(String raw) async {
    setState(() {
      _searchQuery = raw;
      if (raw.trim().isEmpty) {
        _contentSearchPaths = null;
      }
    });
    final trimmed = raw.trim();
    if (trimmed.isEmpty) return;
    final hits = await widget.vaultService.searchNotes(trimmed);
    if (mounted && _searchQuery.trim() == trimmed) {
      setState(() {
        _contentSearchPaths = hits.map((e) => e.path).toSet();
      });
    }
  }

  Future<void> _triggerSync() async {
    final active = widget.vaultService.activeVault;
    if (active == null || !active.hasRemoteSync) {
      await S3ConfigSheet.show(
        context,
        vaultService: widget.vaultService,
        colors: widget.themeProvider.colors,
      );
      return;
    }

    setState(() {
      _syncing = true;
    });

    final report = await widget.vaultService.executeSync();

    if (!mounted) return;
    setState(() {
      _syncing = false;
    });

    if (report.success) {
      await _refreshNotes();
    }
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(report.message)),
      );
    }
  }

  List<String> _buildDynamicFilters() {
    final counts = <String, int>{};
    for (final note in _notes) {
      for (final tag in note.tags) {
        final clean = tag.trim();
        if (clean.isEmpty) continue;
        counts[clean] = (counts[clean] ?? 0) + 1;
      }
    }
    final sortedTags = counts.entries.toList()
      ..sort((a, b) {
        final cmp = b.value.compareTo(a.value);
        return cmp != 0 ? cmp : a.key.compareTo(b.key);
      });

    final topTags = sortedTags.take(15).map((e) => e.key);
    final hasBookmarks = widget.vaultService.bookmarks.isNotEmpty;

    return [
      _kAllFilter,
      if (hasBookmarks) _kBookmarkedFilter,
      ...topTags,
    ];
  }

  Future<void> _openVaultSwitcher() async {
    await VaultSwitcherSheet.show(
      context,
      vaultService: widget.vaultService,
      colors: widget.themeProvider.colors,
    );
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
    final activeProfile = widget.vaultService.activeVault;
    final hasRemoteSync = activeProfile?.hasRemoteSync ?? false;
    final filterChips = _buildDynamicFilters();
    final activeFilter =
        filterChips.contains(_selectedFilter) ? _selectedFilter : _kAllFilter;

    final q = _searchQuery.trim().toLowerCase();
    final filteredNotes = _notes.where((n) {
      final matchesSearch =
          q.isEmpty ||
          n.title.toLowerCase().contains(q) ||
          n.path.toLowerCase().contains(q) ||
          n.preview.toLowerCase().contains(q) ||
          n.tags.any((t) => t.toLowerCase().contains(q)) ||
          (_contentSearchPaths?.contains(n.path) ?? false);

      final matchesFilter = switch (activeFilter) {
        _kAllFilter => true,
        _kBookmarkedFilter => widget.vaultService.isBookmarked(n.path),
        _ => n.tags.contains(activeFilter),
      };
      return matchesSearch && matchesFilter;
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
        title: Material(
          color: Colors.transparent,
          child: InkWell(
            onTap: _openVaultSwitcher,
            borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
            splashColor: colors.active,
            highlightColor: colors.hover,
            child: Container(
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
                  Flexible(
                    child: Text(
                      widget.vaultService.vaultName,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: JiyueTokens.fontBase,
                        fontWeight: FontWeight.w600,
                        color: colors.textPrimary,
                      ),
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
          ),
        ),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 16),
            child: Center(
              child: JiyueButton(
                colors: colors,
                label: hasRemoteSync
                    ? (_syncing ? '同步中...' : '同步')
                    : '配置同步',
                icon: hasRemoteSync ? 'refresh' : 'link',
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
                    hintText: '搜索笔记标题、路径、正文或 #标签...',
                    onChanged: _onSearchChanged,
                    suffix: _searchQuery.isNotEmpty
                        ? JiyueIconButton(
                            colors: colors,
                            icon: 'x',
                            size: 28,
                            iconSize: 14,
                            tooltip: '清空搜索',
                            onPressed: () {
                              _searchController.clear();
                              _onSearchChanged('');
                            },
                          )
                        : null,
                  ),
                ),

                // 动态标签与收藏筛选栏
                if (filterChips.length > 1) ...[
                  const SizedBox(height: 8),
                  SizedBox(
                    height: 30,
                    child: ListView(
                      scrollDirection: Axis.horizontal,
                      padding: const EdgeInsets.symmetric(horizontal: 16),
                      children: filterChips.map((chip) {
                        final isSelected = activeFilter == chip;
                        return Padding(
                          padding: const EdgeInsets.only(right: 6),
                          child: JiyueChoiceChip(
                            colors: colors,
                            label: chip,
                            icon: chip == _kBookmarkedFilter
                                ? 'bookmark-filled'
                                : null,
                            selected: isSelected,
                            onTap: () => setState(() => _selectedFilter = chip),
                          ),
                        );
                      }).toList(),
                    ),
                  ),
                ],

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
                    title: '笔记列表 (${filteredNotes.length} 篇)',
                    padding: EdgeInsets.zero,
                    trailing: JiyueBadge(
                      colors: colors,
                      label: activeProfile?.syncBadgeLabel ?? '纯本地',
                      tone: hasRemoteSync
                          ? JiyueBadgeTone.ok
                          : JiyueBadgeTone.neutral,
                    ),
                  ),
                ),

                // 笔记列表
                if (filteredNotes.isEmpty)
                  JiyueEmptyState(
                    colors: colors,
                    icon: 'file-text',
                    title: _notes.isEmpty ? '当前笔记库为空' : '未找到匹配的笔记',
                    subtitle: _notes.isEmpty
                        ? (hasRemoteSync
                            ? '可点击右上角「同步」拉取 PC 笔记，或点击右下角新建笔记'
                            : '点击右下角「新建笔记」开始记录')
                        : '尝试调整搜索关键词或切换其他筛选标签',
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
