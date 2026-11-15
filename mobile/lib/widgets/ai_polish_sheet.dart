import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../bridge/vault_service.dart';
import '../models/exported_config.dart';
import '../services/ai_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import 'jiyue_icon.dart';

class AiPolishSheet extends StatefulWidget {
  final String initialText;
  final VaultService vaultService;
  final ThemeProvider themeProvider;
  final void Function(String text, bool isReplace) onApply;

  const AiPolishSheet({
    super.key,
    required this.initialText,
    required this.vaultService,
    required this.themeProvider,
    required this.onApply,
  });

  @override
  State<AiPolishSheet> createState() => _AiPolishSheetState();
}

class _AiPolishSheetState extends State<AiPolishSheet> {
  PolishStyle _currentStyle = PolishStyle.smart;
  ExportedAiProvider? _selectedProvider;
  String? _resultText;
  bool _isLoading = false;
  String? _errorMessage;

  @override
  void initState() {
    super.initState();
    _initProvider();
    if (_selectedProvider != null && widget.initialText.trim().isNotEmpty) {
      _startPolishing();
    }
  }

  void _initProvider() {
    final aiConfig = widget.vaultService.aiConfig;
    if (aiConfig != null && aiConfig.providers.isNotEmpty) {
      if (aiConfig.activeChat != null) {
        _selectedProvider = aiConfig.providers.firstWhere(
          (p) => p.id == aiConfig.activeChat,
          orElse: () => aiConfig.providers.first,
        );
      } else {
        _selectedProvider = aiConfig.providers.first;
      }
    }
  }

  Future<void> _startPolishing() async {
    if (_selectedProvider == null) return;
    final text = widget.initialText.trim();
    if (text.isEmpty) {
      setState(() {
        _errorMessage = '请先提供需要润色或总结的文本';
      });
      return;
    }

    setState(() {
      _isLoading = true;
      _errorMessage = null;
      _resultText = null;
    });

    try {
      final res = await AiService.execute(
        text: text,
        style: _currentStyle,
        provider: _selectedProvider!,
      );
      if (mounted) {
        setState(() {
          _resultText = res;
          _isLoading = false;
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _errorMessage = e.toString().replaceFirst('Exception: ', '');
          _isLoading = false;
        });
      }
    }
  }

  void _copyResult() {
    if (_resultText == null) return;
    Clipboard.setData(ClipboardData(text: _resultText!));
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('已复制润色内容到剪贴板'), duration: Duration(seconds: 1)),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;
    final aiConfig = widget.vaultService.aiConfig;

    return Container(
      constraints: BoxConstraints(
        maxHeight: MediaQuery.of(context).size.height * 0.85,
      ),
      decoration: BoxDecoration(
        color: colors.sheet,
        borderRadius: const BorderRadius.vertical(top: Radius.circular(24)),
      ),
      child: SafeArea(
        top: false,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(20, 12, 20, 16),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              // 顶部拖拽手柄
              Center(
                child: Container(
                  width: 36,
                  height: 4,
                  margin: const EdgeInsets.only(bottom: 12),
                  decoration: BoxDecoration(
                    color: colors.border,
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),

              // 标题与模型选择器
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Row(
                    children: [
                      Container(
                        padding: const EdgeInsets.all(6),
                        decoration: BoxDecoration(
                          color: colors.accent.withAlpha(30),
                          borderRadius: BorderRadius.circular(8),
                        ),
                        child: JiyueIcon('sparkles', color: colors.accent, size: 18),
                      ),
                      const SizedBox(width: 8),
                      Text(
                        'AI 智能润色 / 助手',
                        style: TextStyle(
                          fontSize: 16,
                          fontWeight: FontWeight.bold,
                          color: colors.textPrimary,
                        ),
                      ),
                    ],
                  ),
                  if (aiConfig != null && aiConfig.providers.isNotEmpty)
                    DropdownButtonHideUnderline(
                      child: DropdownButton<ExportedAiProvider>(
                        value: _selectedProvider,
                        icon: JiyueIcon('chevron-down', color: colors.textMuted, size: 14),
                        dropdownColor: colors.raised,
                        borderRadius: BorderRadius.circular(12),
                        style: TextStyle(fontSize: 12, color: colors.textPrimary),
                        items: aiConfig.providers.map((p) {
                          return DropdownMenuItem(
                            value: p,
                            child: Text(
                              '${p.name} (${p.chatModel})',
                              style: TextStyle(fontSize: 12, color: colors.textPrimary),
                            ),
                          );
                        }).toList(),
                        onChanged: (p) {
                          if (p != null) {
                            setState(() => _selectedProvider = p);
                            _startPolishing();
                          }
                        },
                      ),
                    ),
                ],
              ),
              const SizedBox(height: 12),

              // 未配置 AI 提示
              if (_selectedProvider == null) ...[
                Container(
                  padding: const EdgeInsets.all(16),
                  margin: const EdgeInsets.symmetric(vertical: 16),
                  decoration: BoxDecoration(
                    color: colors.raised,
                    borderRadius: BorderRadius.circular(14),
                    border: Border.all(color: colors.border),
                  ),
                  child: Column(
                    children: [
                      JiyueIcon('info', color: colors.accent, size: 28),
                      const SizedBox(height: 8),
                      Text(
                        '未检测到已配置的 AI 服务商',
                        style: TextStyle(
                          fontSize: 14,
                          fontWeight: FontWeight.bold,
                          color: colors.textPrimary,
                        ),
                      ),
                      const SizedBox(height: 6),
                      Text(
                        '请在右侧边栏设置中配置，或使用「手机扫码同步」导入 PC 端的配置。',
                        textAlign: TextAlign.center,
                        style: TextStyle(fontSize: 12, color: colors.textMuted, height: 1.4),
                      ),
                    ],
                  ),
                ),
                TextButton(
                  onPressed: () => Navigator.of(context).pop(),
                  child: Text('关闭', style: TextStyle(color: colors.textMuted)),
                ),
              ] else ...[
                // 风格选择选项卡
                SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: PolishStyle.values.map((style) {
                      final isSelected = style == _currentStyle;
                      return Padding(
                        padding: const EdgeInsets.only(right: 8),
                        child: ChoiceChip(
                          showCheckmark: false,
                          avatar: Text(style.icon, style: const TextStyle(fontSize: 13)),
                          label: Text(
                            style.label,
                            style: TextStyle(
                              fontSize: 12,
                              fontWeight: isSelected ? FontWeight.bold : FontWeight.normal,
                              color: isSelected ? colors.onAccent : colors.textPrimary,
                            ),
                          ),
                          selected: isSelected,
                          selectedColor: colors.accent,
                          backgroundColor: colors.raised,
                          side: BorderSide(
                            color: isSelected ? colors.accent : colors.border,
                          ),
                          shape: RoundedRectangleBorder(
                            borderRadius: BorderRadius.circular(18),
                          ),
                          onSelected: (val) {
                            if (val) {
                              setState(() => _currentStyle = style);
                              _startPolishing();
                            }
                          },
                        ),
                      );
                    }).toList(),
                  ),
                ),
                const SizedBox(height: 12),

                // 结果区 / 加载中 / 错误展示
                Flexible(
                  child: Container(
                    width: double.infinity,
                    padding: const EdgeInsets.all(14),
                    decoration: BoxDecoration(
                      color: colors.raised,
                      borderRadius: BorderRadius.circular(14),
                      border: Border.all(color: colors.border),
                    ),
                    child: _buildContentArea(colors),
                  ),
                ),
                const SizedBox(height: 14),

                // 底部操作按钮栏
                if (_resultText != null && !_isLoading) ...[
                  Row(
                    children: [
                      Expanded(
                        child: OutlinedButton(
                          style: OutlinedButton.styleFrom(
                            foregroundColor: colors.textPrimary,
                            side: BorderSide(color: colors.border),
                            padding: const EdgeInsets.symmetric(vertical: 12),
                            shape: RoundedRectangleBorder(
                              borderRadius: BorderRadius.circular(12),
                            ),
                          ),
                          onPressed: () {
                            widget.onApply(_resultText!, false);
                            Navigator.of(context).pop();
                          },
                          child: const Text('⬇️ 插入下方', style: TextStyle(fontSize: 13)),
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: ElevatedButton(
                          style: ElevatedButton.styleFrom(
                            backgroundColor: colors.accent,
                            foregroundColor: colors.onAccent,
                            padding: const EdgeInsets.symmetric(vertical: 12),
                            shape: RoundedRectangleBorder(
                              borderRadius: BorderRadius.circular(12),
                            ),
                          ),
                          onPressed: () {
                            widget.onApply(_resultText!, true);
                            Navigator.of(context).pop();
                          },
                          child: const Text('✨ 替换原内容', style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold)),
                        ),
                      ),
                    ],
                  ),
                ] else if (_isLoading) ...[
                  Center(
                    child: Text(
                      'AI 正在进行 ${_currentStyle.label}，请稍候...',
                      style: TextStyle(fontSize: 12, color: colors.textMuted),
                    ),
                  ),
                ],
              ],
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildContentArea(JiyueThemeColors colors) {
    if (_isLoading) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              width: 24,
              height: 24,
              child: CircularProgressIndicator(strokeWidth: 2.5, color: colors.accent),
            ),
            const SizedBox(height: 12),
            Text(
              '${_currentStyle.icon} 正在分析并重塑语言结构...',
              style: TextStyle(fontSize: 13, color: colors.textMuted),
            ),
          ],
        ),
      );
    }

    if (_errorMessage != null) {
      const dangerColor = Color(0xFFEF4444);
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            JiyueIcon('alert-triangle', color: dangerColor, size: 28),
            const SizedBox(height: 8),
            Text(
              '润色请求失败',
              style: const TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: dangerColor),
            ),
            const SizedBox(height: 4),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16),
              child: Text(
                _errorMessage!,
                textAlign: TextAlign.center,
                style: TextStyle(fontSize: 11, color: colors.textMuted),
              ),
            ),
            const SizedBox(height: 10),
            ElevatedButton.icon(
              style: ElevatedButton.styleFrom(
                backgroundColor: colors.accent,
                foregroundColor: colors.onAccent,
                padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
                shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
              ),
              icon: JiyueIcon('refresh-cw', color: colors.onAccent, size: 14),
              label: const Text('重试', style: TextStyle(fontSize: 12)),
              onPressed: _startPolishing,
            ),
          ],
        ),
      );
    }

    if (_resultText != null) {
      return Stack(
        children: [
          SingleChildScrollView(
            child: SelectableText(
              _resultText!,
              style: TextStyle(
                fontSize: 14,
                height: 1.6,
                color: colors.textPrimary,
                fontFamily: 'monospace',
              ),
            ),
          ),
          Positioned(
            top: 0,
            right: 0,
            child: IconButton(
              icon: JiyueIcon('copy', color: colors.textMuted, size: 16),
              tooltip: '复制内容',
              onPressed: _copyResult,
            ),
          ),
        ],
      );
    }

    return Center(
      child: Text('准备就绪，选择风格开始润色', style: TextStyle(color: colors.textMuted)),
    );
  }
}
