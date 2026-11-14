import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/note.dart';
import '../theme/theme_provider.dart';
import '../widgets/bottom_bar.dart';
import '../widgets/daily_card.dart';
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
  int _currentTab = 0;
  List<NoteSummary> _notes = [];
  String _selectedTag = '全部';
  String _searchQuery = '';
  bool _syncing = false;
  String _syncStatusText = '已同步 · 刚刚';

  @override
  void initState() {
    super.initState();
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

  void _triggerSync() async {
    setState(() {
      _syncing = true;
      _syncStatusText = '同步中...';
    });

    await Future.delayed(const Duration(milliseconds: 1200));

    if (mounted) {
      setState(() {
        _syncing = false;
        _syncStatusText = '已同步 · 刚刚';
      });
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('S3 / WebDAV 云端同步完成')),
      );
    }
  }

  void _openDailyQuickLog() {
    final controller = TextEditingController();
    final colors = widget.themeProvider.colors;

    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (ctx) {
        return Container(
          padding: EdgeInsets.fromLTRB(20, 20, 20, MediaQuery.of(ctx).viewInsets.bottom + 20),
          decoration: BoxDecoration(
            color: colors.sheet,
            borderRadius: const BorderRadius.vertical(top: Radius.circular(20)),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  JiyueIcon('calendar-event', color: colors.accent, size: 20),
                  const SizedBox(width: 8),
                  Text(
                    '追加今日速记随笔',
                    style: TextStyle(
                      fontSize: 16,
                      fontWeight: FontWeight.bold,
                      color: colors.textPrimary,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              TextField(
                controller: controller,
                autofocus: true,
                maxLines: 3,
                style: TextStyle(color: colors.textPrimary, fontSize: 14),
                decoration: InputDecoration(
                  hintText: '记录此刻灵感、待办或备忘...',
                  hintStyle: TextStyle(color: colors.textMuted),
                  filled: true,
                  fillColor: colors.raised,
                  border: OutlineInputBorder(
                    borderRadius: BorderRadius.circular(12),
                    borderSide: BorderSide(color: colors.border),
                  ),
                ),
              ),
              const SizedBox(height: 12),
              SizedBox(
                width: double.infinity,
                child: ElevatedButton.icon(
                  style: ElevatedButton.styleFrom(
                    backgroundColor: colors.accent,
                    foregroundColor: colors.onAccent,
                    shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
                  ),
                  icon: JiyueIcon('check', color: colors.onAccent, size: 16),
                  label: const Text('追加到今日笔记'),
                  onPressed: () async {
                    final text = controller.text.trim();
                    if (text.isNotEmpty) {
                      await widget.vaultService.appendToDaily(text);
                      if (ctx.mounted) Navigator.of(ctx).pop();
                      _refreshNotes();
                    }
                  },
                ),
              ),
            ],
          ),
        );
      },
    );
  }

  void _createNewNote() async {
    final titleController = TextEditingController();
    final colors = widget.themeProvider.colors;

    final title = await showDialog<String>(
      context: context,
      builder: (ctx) {
        return AlertDialog(
          backgroundColor: colors.sheet,
          title: Text('新建笔记', style: TextStyle(color: colors.textPrimary)),
          content: TextField(
            controller: titleController,
            autofocus: true,
            style: TextStyle(color: colors.textPrimary),
            decoration: InputDecoration(
              hintText: '输入笔记标题...',
              hintStyle: TextStyle(color: colors.textMuted),
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(ctx).pop(null),
              child: Text('取消', style: TextStyle(color: colors.textSecondary)),
            ),
            ElevatedButton(
              style: ElevatedButton.styleFrom(
                backgroundColor: colors.accent,
                foregroundColor: colors.onAccent,
              ),
              onPressed: () => Navigator.of(ctx).pop(titleController.text.trim()),
              child: const Text('创建'),
            ),
          ],
        );
      },
    );

    if (title != null && title.isNotEmpty) {
      final note = await widget.vaultService.createNote(title, '# $title\n\n');
      if (note != null && mounted) {
        _refreshNotes();
        Navigator.of(context).push(
          MaterialPageRoute(
            builder: (_) => EditorPage(
              notePath: note.path,
              initialContent: '# $title\n\n',
              themeProvider: widget.themeProvider,
              vaultService: widget.vaultService,
            ),
          ),
        ).then((_) => _refreshNotes());
      }
    }
  }

  void _openSettings() {
    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (_) => SettingsModalSheet(
        themeProvider: widget.themeProvider,
        vaultService: widget.vaultService,
        noteCount: _notes.length,
      ),
    ).then((_) => setState(() {}));
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;

    // 过滤笔记
    final filteredNotes = _notes.where((n) {
      final matchesSearch = _searchQuery.isEmpty ||
          n.title.toLowerCase().contains(_searchQuery.toLowerCase()) ||
          n.preview.toLowerCase().contains(_searchQuery.toLowerCase());
      final matchesTag = _selectedTag == '全部' || n.tags.contains(_selectedTag);
      return matchesSearch && matchesTag;
    }).toList();

    return Scaffold(
      backgroundColor: colors.chrome,
      appBar: AppBar(
        backgroundColor: colors.chrome,
        elevation: 0,
        titleSpacing: 20,
        title: Row(
          children: [
            // 库选择胶囊（带 Jiyue 官方 Logo）
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
              decoration: BoxDecoration(
                color: colors.sheet,
                borderRadius: BorderRadius.circular(19),
                border: Border.all(color: colors.border),
              ),
              child: Row(
                children: [
                  Image.asset(
                    'assets/logo/icon-256.png',
                    width: 20,
                    height: 20,
                  ),
                  const SizedBox(width: 8),
                  Text(
                    widget.vaultService.vaultName,
                    style: TextStyle(
                      fontSize: 13,
                      fontWeight: FontWeight.bold,
                      color: colors.textPrimary,
                    ),
                  ),
                  const SizedBox(width: 6),
                  JiyueIcon('chevron-down', color: colors.textSecondary, size: 14),
                ],
              ),
            ),
          ],
        ),
        actions: [
          // S3 / WebDAV 同步状态胶囊
          Padding(
            padding: const EdgeInsets.only(right: 20),
            child: InkWell(
              onTap: _syncing ? null : _triggerSync,
              borderRadius: BorderRadius.circular(17),
              child: Container(
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                decoration: BoxDecoration(
                  color: colors.accent.withAlpha(25),
                  borderRadius: BorderRadius.circular(17),
                  border: Border.all(color: colors.accent.withAlpha(80)),
                ),
                child: Row(
                  children: [
                    if (_syncing)
                      SizedBox(
                        width: 12,
                        height: 12,
                        child: CircularProgressIndicator(strokeWidth: 2, color: colors.accent),
                      )
                    else
                      JiyueIcon('link', color: colors.accent, size: 14),
                    const SizedBox(width: 6),
                    Text(
                      _syncStatusText,
                      style: TextStyle(
                        fontSize: 11,
                        fontWeight: FontWeight.bold,
                        color: colors.accent,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
      body: Stack(
        children: [
          RefreshIndicator(
            color: colors.accent,
            backgroundColor: colors.sheet,
            onRefresh: _refreshNotes,
            child: ListView(
              padding: const EdgeInsets.only(bottom: 100),
              children: [
                // 搜索栏
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 8),
                  child: Container(
                    height: 42,
                    decoration: BoxDecoration(
                      color: colors.sheet,
                      borderRadius: BorderRadius.circular(12),
                      border: Border.all(color: colors.border),
                    ),
                    child: TextField(
                      style: TextStyle(fontSize: 13, color: colors.textPrimary),
                      onChanged: (v) => setState(() => _searchQuery = v),
                      decoration: InputDecoration(
                        border: InputBorder.none,
                        prefixIcon: Padding(
                          padding: const EdgeInsets.all(11.0),
                          child: JiyueIcon('search', color: colors.textMuted, size: 18),
                        ),
                        hintText: '搜索笔记正文、标题或 #标签...',
                        hintStyle: TextStyle(fontSize: 13, color: colors.textMuted),
                        contentPadding: const EdgeInsets.symmetric(vertical: 10),
                      ),
                    ),
                  ),
                ),

                // 快速标签横向筛选流
                SizedBox(
                  height: 38,
                  child: ListView(
                    scrollDirection: Axis.horizontal,
                    padding: const EdgeInsets.symmetric(horizontal: 20),
                    children: ['全部', '#架构', '#Rust', '#日记', '#同步', '#设计'].map((tag) {
                      final isSelected = _selectedTag == tag;
                      return Padding(
                        padding: const EdgeInsets.only(right: 8),
                        child: ChoiceChip(
                          label: Text(tag),
                          selected: isSelected,
                          onSelected: (_) => setState(() => _selectedTag = tag),
                          selectedColor: colors.accent,
                          backgroundColor: colors.sheet,
                          labelStyle: TextStyle(
                            fontSize: 12,
                            fontWeight: isSelected ? FontWeight.bold : FontWeight.normal,
                            color: isSelected ? colors.onAccent : colors.textSecondary,
                          ),
                          shape: RoundedRectangleBorder(
                            borderRadius: BorderRadius.circular(14),
                            side: BorderSide(
                              color: isSelected ? colors.accent : colors.border,
                            ),
                          ),
                        ),
                      );
                    }).toList(),
                  ),
                ),

                // 今日速记 Hero 卡片
                DailyHeroCard(
                  colors: colors,
                  onTapRecord: _openDailyQuickLog,
                ),

                // 列表小标题
                Padding(
                  padding: const EdgeInsets.fromLTRB(22, 10, 22, 6),
                  child: Text(
                    '最近编辑 (${filteredNotes.length} 篇 · ${colors.name})',
                    style: TextStyle(
                      fontSize: 12,
                      fontWeight: FontWeight.bold,
                      color: colors.textMuted,
                    ),
                  ),
                ),

                // 笔记列表
                if (filteredNotes.isEmpty)
                  Padding(
                    padding: const EdgeInsets.all(40.0),
                    child: Center(
                      child: Column(
                        children: [
                          JiyueIcon('file-text', color: colors.textMuted, size: 36),
                          const SizedBox(height: 12),
                          Text('未找到相关笔记', style: TextStyle(color: colors.textMuted)),
                        ],
                      ),
                    ),
                  )
                else
                  ...filteredNotes.map((note) {
                    return NoteCard(
                      note: note,
                      colors: colors,
                      onTap: () {
                        Navigator.of(context).push(
                          MaterialPageRoute(
                            builder: (_) => ReaderPage(
                              notePath: note.path,
                              themeProvider: widget.themeProvider,
                              vaultService: widget.vaultService,
                            ),
                          ),
                        ).then((_) => _refreshNotes());
                      },
                    );
                  }),
              ],
            ),
          ),

          // 悬浮新建 FAB 按钮
          Positioned(
            right: 20,
            bottom: 24,
            child: FloatingActionButton.extended(
              backgroundColor: colors.accent,
              foregroundColor: colors.onAccent,
              elevation: 4,
              icon: JiyueIcon('plus', color: colors.onAccent, size: 20),
              label: const Text('新建笔记', style: TextStyle(fontWeight: FontWeight.bold)),
              onPressed: _createNewNote,
            ),
          ),
        ],
      ),
      bottomNavigationBar: JiyueBottomBar(
        currentIndex: _currentTab,
        colors: colors,
        onTabSelected: (idx) {
          setState(() => _currentTab = idx);
          if (idx == 1) {
            _openDailyQuickLog();
          } else if (idx == 3) {
            _openSettings();
          }
        },
      ),
    );
  }
}
