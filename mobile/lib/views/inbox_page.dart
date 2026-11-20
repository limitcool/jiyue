import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';
import '../bridge/vault_service.dart';
import '../models/note.dart';
import '../services/platform_bridge.dart';
import '../theme/jiyue_theme.dart';
import '../widgets/jiyue_controls.dart';
import '../widgets/jiyue_icon.dart';

enum _CaptureMode { stream, todo, daily }

class InboxPage extends StatefulWidget {
  final VaultService vaultService;
  final JiyueThemeColors colors;
  final ValueChanged<String> onOpenNote;
  final VoidCallback onVaultMutated;

  const InboxPage({
    super.key,
    required this.vaultService,
    required this.colors,
    required this.onOpenNote,
    required this.onVaultMutated,
  });

  @override
  State<InboxPage> createState() => _InboxPageState();
}

class _InboxPageState extends State<InboxPage> {
  final TextEditingController _inputCtrl = TextEditingController();

  InboxSummary _summary = InboxSummary.empty;
  bool _loading = true;
  bool _submitting = false;
  int _section = 0; // 0: 随手记流, 1: 笔记待办
  bool _showDoneTodos = false;
  _CaptureMode _mode = _CaptureMode.stream;

  @override
  void initState() {
    super.initState();
    widget.vaultService.addListener(_onVaultChanged);
    _load();
  }

  @override
  void dispose() {
    widget.vaultService.removeListener(_onVaultChanged);
    _inputCtrl.dispose();
    super.dispose();
  }

  void _onVaultChanged() {
    if (mounted) _load();
  }

  Future<void> _load() async {
    final res = await widget.vaultService.getInboxSummary();
    if (mounted) {
      setState(() {
        _summary = res;
        _loading = false;
      });
    }
  }

  Future<void> _submit() async {
    final text = _inputCtrl.text.trim();
    if (text.isEmpty || _submitting) return;
    setState(() => _submitting = true);
    try {
      switch (_mode) {
        case _CaptureMode.stream:
          await widget.vaultService.appendToInbox(text, asTodo: false);
          break;
        case _CaptureMode.todo:
          await widget.vaultService.appendToInbox(text, asTodo: true);
          break;
        case _CaptureMode.daily:
          await widget.vaultService.appendToDaily(text);
          break;
      }
      _inputCtrl.clear();
      await _load();
      widget.onVaultMutated();
      if (mounted) {
        final msg = switch (_mode) {
          _CaptureMode.stream => '已记入随手记',
          _CaptureMode.todo => '已添加待办',
          _CaptureMode.daily => '已追加到今日日记',
        };
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text(msg)),
        );
      }
    } finally {
      if (mounted) setState(() => _submitting = false);
    }
  }

  Future<void> _pickAndImportImage() async {
    final bytes = await PlatformBridge.pickImageBytes();
    if (bytes == null || bytes.isEmpty) return;

    setState(() => _submitting = true);
    try {
      final tmpDir = await getTemporaryDirectory();
      final stamp = DateTime.now().millisecondsSinceEpoch;
      final tmpFile = File('${tmpDir.path}/jiyue_pick_$stamp.png');
      await tmpFile.writeAsBytes(bytes, flush: true);

      final res = await widget.vaultService.importFileToInbox(
        tmpFile.path,
        noteText: _inputCtrl.text.trim(),
      );
      if (res.success) {
        _inputCtrl.clear();
        await _load();
        widget.onVaultMutated();
      }
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text(res.success ? '图片已导入随手记' : res.message),
          ),
        );
      }
    } finally {
      if (mounted) setState(() => _submitting = false);
    }
  }

  Future<void> _openTodayDaily() async {
    final daily = await widget.vaultService.getOrCreateDailyNote();
    if (mounted) {
      widget.onOpenNote(daily.path);
    }
  }

  Future<void> _toggleTodo(InboxTodo todo) async {
    final ok = await widget.vaultService.toggleInboxTodo(
      todo.notePath,
      todo.lineIndex,
    );
    if (ok) {
      await _load();
      widget.onVaultMutated();
    }
  }

  Future<void> _deleteCapture(InboxCapture item) async {
    final ok = await widget.vaultService.deleteInboxEntry(
      item.notePath,
      item.lineIndex,
    );
    if (ok) {
      await _load();
      widget.onVaultMutated();
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('已移除条目')),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.colors;
    final openTodos = _summary.todos.where((t) => !t.done).toList();
    final visibleTodos =
        _showDoneTodos ? _summary.todos : openTodos;

    return RefreshIndicator(
      color: colors.accent,
      backgroundColor: colors.surface,
      onRefresh: _load,
      child: ListView(
        padding: const EdgeInsets.only(top: 10, bottom: 24),
        children: [
          // 顶部快速捕捉卡片
          JiyueSectionCard(
            colors: colors,
            margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
            padding: const EdgeInsets.all(12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Row(
                  children: [
                    JiyueIcon('bolt', color: colors.accent, size: 15),
                    const SizedBox(width: 6),
                    Expanded(
                      child: Text(
                        '快速捕捉',
                        style: TextStyle(
                          fontSize: JiyueTokens.fontBase,
                          fontWeight: FontWeight.w600,
                          color: colors.textPrimary,
                        ),
                      ),
                    ),
                    JiyueButton(
                      colors: colors,
                      label: '今日日记',
                      icon: 'calendar-event',
                      kind: JiyueButtonKind.ghost,
                      size: JiyueControlSize.xs,
                      onPressed: _openTodayDaily,
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                JiyueTextField(
                  colors: colors,
                  controller: _inputCtrl,
                  maxLines: 3,
                  hintText: switch (_mode) {
                    _CaptureMode.stream => '记录此刻想法、摘录或备忘，写入随手记...',
                    _CaptureMode.todo => '输入待办事项（将写入 - [ ] ）...',
                    _CaptureMode.daily => '追加到今日日记的时间线...',
                  },
                ),
                const SizedBox(height: 10),
                Row(
                  children: [
                    Expanded(
                      child: SingleChildScrollView(
                        scrollDirection: Axis.horizontal,
                        child: Row(
                          children: [
                            JiyueChoiceChip(
                              colors: colors,
                              label: '随手记',
                              icon: 'bulb',
                              selected: _mode == _CaptureMode.stream,
                              onTap: () => setState(() => _mode = _CaptureMode.stream),
                            ),
                            const SizedBox(width: 6),
                            JiyueChoiceChip(
                              colors: colors,
                              label: '待办',
                              icon: 'checkbox',
                              selected: _mode == _CaptureMode.todo,
                              onTap: () => setState(() => _mode = _CaptureMode.todo),
                            ),
                            const SizedBox(width: 6),
                            JiyueChoiceChip(
                              colors: colors,
                              label: '日记',
                              icon: 'calendar',
                              selected: _mode == _CaptureMode.daily,
                              onTap: () => setState(() => _mode = _CaptureMode.daily),
                            ),
                          ],
                        ),
                      ),
                    ),
                    const SizedBox(width: 6),
                    JiyueIconButton(
                      colors: colors,
                      icon: 'photo',
                      tooltip: '导入图片到随手记',
                      onPressed: _submitting ? null : _pickAndImportImage,
                    ),
                    const SizedBox(width: 6),
                    JiyueButton(
                      colors: colors,
                      label: '记录',
                      icon: 'check',
                      kind: JiyueButtonKind.primary,
                      size: JiyueControlSize.sm,
                      loading: _submitting,
                      onPressed: _submit,
                    ),
                  ],
                ),
              ],
            ),
          ),

          // 分段切换：随手记 vs 笔记待办
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
            child: Row(
              children: [
                JiyueChoiceChip(
                  colors: colors,
                  label: '随手记 (${_summary.captures.length})',
                  icon: 'bulb',
                  selected: _section == 0,
                  onTap: () => setState(() => _section = 0),
                ),
                const SizedBox(width: 8),
                JiyueChoiceChip(
                  colors: colors,
                  label: '笔记待办 (${openTodos.length})',
                  icon: 'checkbox',
                  selected: _section == 1,
                  onTap: () => setState(() => _section = 1),
                ),
                const Spacer(),
                if (_section == 1)
                  JiyueChoiceChip(
                    colors: colors,
                    label: _showDoneTodos ? '隐藏已完成' : '显示已完成',
                    selected: _showDoneTodos,
                    onTap: () => setState(() => _showDoneTodos = !_showDoneTodos),
                  ),
              ],
            ),
          ),

          if (_loading)
            const Padding(
              padding: EdgeInsets.all(32),
              child: Center(child: CircularProgressIndicator()),
            )
          else if (_section == 0)
            ..._buildCapturesList(colors)
          else
            ..._buildTodosList(colors, visibleTodos),
        ],
      ),
    );
  }

  List<Widget> _buildCapturesList(JiyueThemeColors colors) {
    if (_summary.captures.isEmpty) {
      return [
        JiyueEmptyState(
          colors: colors,
          icon: 'bulb',
          title: '暂无随手记',
          subtitle: '上方输入灵感即可写入 ${_summary.streamPath}，或从其他应用分享图文到这里',
        ),
      ];
    }

    return _summary.captures.map((item) {
      return JiyueSectionCard(
        colors: colors,
        margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
        padding: const EdgeInsets.all(12),
        onTap: () => widget.onOpenNote(item.notePath),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                if (item.dateLabel.isNotEmpty)
                  JiyueBadge(
                    colors: colors,
                    label: item.dateLabel,
                    tone: JiyueBadgeTone.neutral,
                  ),
                if (item.timeLabel != null && item.timeLabel!.isNotEmpty) ...[
                  const SizedBox(width: 6),
                  JiyueIcon('clock', color: colors.textMuted, size: 12),
                  const SizedBox(width: 3),
                  Text(
                    item.timeLabel!,
                    style: TextStyle(
                      fontSize: JiyueTokens.fontXs,
                      color: colors.textMuted,
                    ),
                  ),
                ],
                const Spacer(),
                JiyueIconButton(
                  colors: colors,
                  icon: 'copy',
                  size: 26,
                  iconSize: 13,
                  tooltip: '复制内容',
                  onPressed: () {
                    Clipboard.setData(ClipboardData(text: item.content));
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(content: Text('已复制随手记内容')),
                    );
                  },
                ),
                JiyueIconButton(
                  colors: colors,
                  icon: 'trash',
                  size: 26,
                  iconSize: 13,
                  tooltip: '移除条目',
                  onPressed: () => _deleteCapture(item),
                ),
              ],
            ),
            const SizedBox(height: 6),
            Text(
              item.content,
              style: TextStyle(
                fontSize: JiyueTokens.fontBase,
                height: 1.45,
                color: colors.textPrimary,
              ),
            ),
            const SizedBox(height: 6),
            Row(
              children: [
                JiyueIcon('file-text', color: colors.textMuted, size: 12),
                const SizedBox(width: 4),
                Flexible(
                  child: Text(
                    item.noteTitle,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: JiyueTokens.fontXs,
                      color: colors.textMuted,
                    ),
                  ),
                ),
                if (item.tags.isNotEmpty) ...[
                  const SizedBox(width: 8),
                  Wrap(
                    spacing: 4,
                    children: item.tags
                        .take(2)
                        .map((tag) => JiyueTagChip(colors: colors, label: tag))
                        .toList(),
                  ),
                ],
              ],
            ),
          ],
        ),
      );
    }).toList();
  }

  List<Widget> _buildTodosList(
    JiyueThemeColors colors,
    List<InboxTodo> items,
  ) {
    if (items.isEmpty) {
      return [
        JiyueEmptyState(
          colors: colors,
          icon: 'checkbox',
          title: '没有待处理的笔记待办',
          subtitle: '在随手记、日记或笔记中使用 - [ ] 即可在这里汇总并原地勾选',
        ),
      ];
    }

    return items.map((todo) {
      return JiyueSectionCard(
        colors: colors,
        margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        onTap: () => widget.onOpenNote(todo.notePath),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            InkWell(
              onTap: () => _toggleTodo(todo),
              borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
              child: Padding(
                padding: const EdgeInsets.all(2),
                child: JiyueIcon(
                  todo.done ? 'checkbox' : 'square',
                  color: todo.done ? colors.accent : colors.textMuted,
                  size: 18,
                ),
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    todo.text,
                    style: TextStyle(
                      fontSize: JiyueTokens.fontBase,
                      color: todo.done ? colors.textMuted : colors.textPrimary,
                      decoration:
                          todo.done ? TextDecoration.lineThrough : null,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Row(
                    children: [
                      JiyueBadge(
                        colors: colors,
                        label: switch (todo.group) {
                          'today' => '今日日记',
                          'core' => '核心目录',
                          _ => '笔记',
                        },
                        tone: todo.group == 'today'
                            ? JiyueBadgeTone.accent
                            : JiyueBadgeTone.neutral,
                      ),
                      const SizedBox(width: 6),
                      Expanded(
                        child: Text(
                          todo.noteTitle,
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
                ],
              ),
            ),
          ],
        ),
      );
    }).toList();
  }
}
