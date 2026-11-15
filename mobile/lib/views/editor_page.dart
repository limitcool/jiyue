import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import '../widgets/ai_polish_sheet.dart';
import '../widgets/jiyue_icon.dart';

class EditorPage extends StatefulWidget {
  final String notePath;
  final String initialContent;
  final ThemeProvider themeProvider;
  final VaultService vaultService;

  const EditorPage({
    super.key,
    required this.notePath,
    required this.initialContent,
    required this.themeProvider,
    required this.vaultService,
  });

  @override
  State<EditorPage> createState() => _EditorPageState();
}

class _EditorPageState extends State<EditorPage> {
  late TextEditingController _controller;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _controller = TextEditingController(text: widget.initialContent);
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    setState(() => _saving = true);
    final ok = await widget.vaultService.saveNote(widget.notePath, _controller.text);
    if (mounted) {
      setState(() => _saving = false);
      if (ok) {
        Navigator.of(context).pop(true);
      } else {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('保存失败，请检查写入权限')),
        );
      }
    }
  }

  void _insertMarkdown(String prefix, [String suffix = '']) {
    final text = _controller.text;
    final selection = _controller.selection;
    if (selection.start < 0) {
      _controller.text = '$text$prefix$suffix';
      return;
    }
    final selectedText = text.substring(selection.start, selection.end);
    final newText = text.replaceRange(selection.start, selection.end, '$prefix$selectedText$suffix');
    _controller.text = newText;
    _controller.selection = TextSelection.collapsed(offset: selection.start + prefix.length + selectedText.length);
  }

  void _openAiPolish([String? customText]) {
    final text = _controller.text;
    final selection = _controller.selection;
    final bool hasSelection = selection.isValid && !selection.isCollapsed && selection.start >= 0 && selection.end <= text.length;
    final targetText = customText ?? (hasSelection ? text.substring(selection.start, selection.end) : text);

    if (targetText.trim().isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('笔记内容为空，无法润色')),
      );
      return;
    }

    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (_) => AiPolishSheet(
        initialText: targetText,
        vaultService: widget.vaultService,
        themeProvider: widget.themeProvider,
        onApply: (polished, isReplace) {
          if (isReplace) {
            if (hasSelection && customText == null) {
              final newText = text.replaceRange(selection.start, selection.end, polished);
              _controller.text = newText;
              _controller.selection = TextSelection.collapsed(offset: selection.start + polished.length);
            } else {
              _controller.text = polished;
              _controller.selection = TextSelection.collapsed(offset: polished.length);
            }
          } else {
            if (hasSelection && customText == null) {
              final insertOffset = selection.end;
              final newText = '${text.substring(0, insertOffset)}\n\n$polished\n${text.substring(insertOffset)}';
              _controller.text = newText;
              _controller.selection = TextSelection.collapsed(offset: insertOffset + polished.length + 3);
            } else {
              final newText = '${_controller.text}\n\n$polished';
              _controller.text = newText;
              _controller.selection = TextSelection.collapsed(offset: newText.length);
            }
          }
        },
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;

    return Scaffold(
      backgroundColor: colors.chrome,
      appBar: AppBar(
        backgroundColor: colors.chrome,
        elevation: 0,
        leading: IconButton(
          icon: JiyueIcon('chevron-left', color: colors.textPrimary, size: 24),
          onPressed: () => Navigator.of(context).pop(false),
        ),
        title: Text(
          widget.notePath,
          style: TextStyle(fontSize: 14, fontWeight: FontWeight.bold, color: colors.textPrimary),
        ),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 12),
            child: TextButton.icon(
              style: TextButton.styleFrom(
                foregroundColor: colors.accent,
                backgroundColor: colors.accent.withAlpha(25),
                shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
              ),
              icon: _saving
                  ? SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(strokeWidth: 2, color: colors.accent),
                    )
                  : JiyueIcon('check', color: colors.accent, size: 16),
              label: Text(_saving ? '保存中' : '完成', style: const TextStyle(fontWeight: FontWeight.bold)),
              onPressed: _saving ? null : _save,
            ),
          ),
        ],
      ),
      body: Column(
        children: [
          // 快捷编辑栏
          Container(
            height: 42,
            padding: const EdgeInsets.symmetric(horizontal: 12),
            decoration: BoxDecoration(
              color: colors.sheet,
              border: Border(bottom: BorderSide(color: colors.border)),
            ),
            child: ListView(
              scrollDirection: Axis.horizontal,
              children: [
                _aiToolButton('✨ AI 润色', () => _openAiPolish(), colors),
                _toolButton('[[ ]]', '双链', () => _insertMarkdown('[[', ']]'), colors),
                _toolButton('#', '大标题', () => _insertMarkdown('# '), colors),
                _toolButton('##', '二级标题', () => _insertMarkdown('## '), colors),
                _toolButton('- [ ]', '待办', () => _insertMarkdown('- [ ] '), colors),
                _toolButton('**B**', '粗体', () => _insertMarkdown('**', '**'), colors),
                _toolButton('`code`', '代码', () => _insertMarkdown('`', '`'), colors),
                _toolButton('> 引用', '引用块', () => _insertMarkdown('> '), colors),
              ],
            ),
          ),
          // 正文多行输入框
          Expanded(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
              child: TextField(
                controller: _controller,
                maxLines: null,
                expands: true,
                keyboardType: TextInputType.multiline,
                contextMenuBuilder: (context, editableTextState) {
                  final List<ContextMenuButtonItem> buttonItems = editableTextState.contextMenuButtonItems;
                  buttonItems.insert(
                    0,
                    ContextMenuButtonItem(
                      label: '✨ AI 润色',
                      onPressed: () {
                        editableTextState.hideToolbar();
                        _openAiPolish();
                      },
                    ),
                  );
                  return AdaptiveTextSelectionToolbar.buttonItems(
                    anchors: editableTextState.contextMenuAnchors,
                    buttonItems: buttonItems,
                  );
                },
                style: TextStyle(
                  fontSize: 15,
                  height: 1.6,
                  color: colors.textPrimary,
                  fontFamily: 'monospace',
                ),
                decoration: InputDecoration(
                  border: InputBorder.none,
                  hintText: '开始书写笔记正文 (支持标准 Markdown 及 [[双链]])...',
                  hintStyle: TextStyle(color: colors.textMuted),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _aiToolButton(String label, VoidCallback onTap, JiyueThemeColors colors) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(8),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10),
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: colors.accent.withAlpha(30),
            border: Border.all(color: colors.accent.withAlpha(80)),
            borderRadius: BorderRadius.circular(8),
          ),
          child: Text(
            label,
            style: TextStyle(
              fontSize: 12,
              fontWeight: FontWeight.bold,
              color: colors.accent,
            ),
          ),
        ),
      ),
    );
  }

  Widget _toolButton(String label, String tooltip, VoidCallback onTap, JiyueThemeColors colors) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(6),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10),
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: colors.raised,
            borderRadius: BorderRadius.circular(6),
          ),
          child: Text(
            label,
            style: TextStyle(fontSize: 12, fontWeight: FontWeight.bold, color: colors.textPrimary),
          ),
        ),
      ),
    );
  }
}
