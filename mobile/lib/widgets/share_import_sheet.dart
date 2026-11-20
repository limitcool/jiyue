import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/note.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_controls.dart';
import 'jiyue_icon.dart';

enum _ShareTarget { inbox, todo, daily }

class ShareImportSheet extends StatefulWidget {
  final SharedPayload payload;
  final VaultService vaultService;
  final JiyueThemeColors colors;
  final VoidCallback onImported;

  const ShareImportSheet({
    super.key,
    required this.payload,
    required this.vaultService,
    required this.colors,
    required this.onImported,
  });

  static Future<void> show(
    BuildContext context, {
    required SharedPayload payload,
    required VaultService vaultService,
    required JiyueThemeColors colors,
    required VoidCallback onImported,
  }) {
    return showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (_) => ShareImportSheet(
        payload: payload,
        vaultService: vaultService,
        colors: colors,
        onImported: onImported,
      ),
    );
  }

  @override
  State<ShareImportSheet> createState() => _ShareImportSheetState();
}

class _ShareImportSheetState extends State<ShareImportSheet> {
  late final TextEditingController _textCtrl;
  _ShareTarget _target = _ShareTarget.inbox;
  bool _saving = false;

  bool get _hasFiles => widget.payload.filePaths.isNotEmpty;

  @override
  void initState() {
    super.initState();
    _textCtrl = TextEditingController(text: widget.payload.text);
  }

  @override
  void dispose() {
    _textCtrl.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    if (_saving) return;
    setState(() => _saving = true);

    try {
      final noteText = _textCtrl.text.trim();
      if (_hasFiles) {
        var successCount = 0;
        String? lastError;
        for (var i = 0; i < widget.payload.filePaths.length; i++) {
          final path = widget.payload.filePaths[i];
          final res = await widget.vaultService.importFileToInbox(
            path,
            noteText: i == 0 ? noteText : '',
          );
          if (res.success) {
            successCount++;
          } else {
            lastError = res.message;
          }
        }
        if (mounted) {
          Navigator.of(context).pop();
          widget.onImported();
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text(
                successCount > 0
                    ? '已将 $successCount 个分享文件存入随手记与附件目录'
                    : (lastError ?? '导入分享文件失败'),
              ),
            ),
          );
        }
        return;
      }

      if (noteText.isEmpty) {
        if (mounted) Navigator.of(context).pop();
        return;
      }

      switch (_target) {
        case _ShareTarget.inbox:
          await widget.vaultService.appendToInbox(noteText, asTodo: false);
          break;
        case _ShareTarget.todo:
          await widget.vaultService.appendToInbox(noteText, asTodo: true);
          break;
        case _ShareTarget.daily:
          await widget.vaultService.appendToDaily(noteText);
          break;
      }

      if (mounted) {
        Navigator.of(context).pop();
        widget.onImported();
        final label = switch (_target) {
          _ShareTarget.inbox => '随手记',
          _ShareTarget.todo => '笔记待办',
          _ShareTarget.daily => '今日日记',
        };
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('已将分享内容存入$label')),
        );
      }
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.colors;
    return JiyueModalSheet(
      colors: colors,
      icon: _hasFiles ? 'photo' : 'bolt',
      title: _hasFiles ? '导入分享的文件到随手记' : '收集分享内容到霁月',
      subtitle: _hasFiles
          ? '共 ${widget.payload.filePaths.length} 个文件，将保存至笔记库附件目录并写入随手记'
          : '可编辑内容后存入随手记、笔记待办或今日日记',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (_hasFiles) ...[
            Container(
              padding: const EdgeInsets.all(10),
              decoration: BoxDecoration(
                color: colors.elevated,
                borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
                border: Border.all(color: colors.borderSubtle, width: 1),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: widget.payload.filePaths.map((p) {
                  final name = p.split(RegExp(r'[/\\]')).last;
                  return Padding(
                    padding: const EdgeInsets.symmetric(vertical: 2),
                    child: Row(
                      children: [
                        JiyueIcon('file-text', color: colors.accent, size: 14),
                        const SizedBox(width: 6),
                        Expanded(
                          child: Text(
                            name,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontSize: JiyueTokens.fontSm,
                              color: colors.textPrimary,
                            ),
                          ),
                        ),
                      ],
                    ),
                  );
                }).toList(),
              ),
            ),
            const SizedBox(height: 10),
          ],
          JiyueTextField(
            colors: colors,
            controller: _textCtrl,
            maxLines: 4,
            label: _hasFiles ? '附带说明（可选）' : '内容',
            hintText: _hasFiles ? '为导入的附件添加一条随手记说明...' : '分享文本内容...',
          ),
          if (!_hasFiles) ...[
            const SizedBox(height: 10),
            Row(
              children: [
                JiyueChoiceChip(
                  colors: colors,
                  label: '随手记',
                  icon: 'bulb',
                  selected: _target == _ShareTarget.inbox,
                  onTap: () => setState(() => _target = _ShareTarget.inbox),
                ),
                const SizedBox(width: 6),
                JiyueChoiceChip(
                  colors: colors,
                  label: '待办',
                  icon: 'checkbox',
                  selected: _target == _ShareTarget.todo,
                  onTap: () => setState(() => _target = _ShareTarget.todo),
                ),
                const SizedBox(width: 6),
                JiyueChoiceChip(
                  colors: colors,
                  label: '今日日记',
                  icon: 'calendar',
                  selected: _target == _ShareTarget.daily,
                  onTap: () => setState(() => _target = _ShareTarget.daily),
                ),
              ],
            ),
          ],
          const SizedBox(height: 14),
          JiyueButton(
            colors: colors,
            label: _hasFiles ? '导入并记录' : '存入笔记库',
            icon: 'check',
            kind: JiyueButtonKind.primary,
            size: JiyueControlSize.lg,
            loading: _saving,
            expand: true,
            onPressed: _save,
          ),
        ],
      ),
    );
  }
}
