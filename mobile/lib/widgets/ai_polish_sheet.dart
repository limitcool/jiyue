import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../bridge/vault_service.dart';
import '../models/exported_config.dart';
import '../services/ai_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import 'jiyue_controls.dart';
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
      const SnackBar(
        content: Text('已复制润色内容到剪贴板'),
        duration: Duration(seconds: 1),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;
    final aiConfig = widget.vaultService.aiConfig;

    final Widget? providerPicker =
        (aiConfig != null && aiConfig.providers.isNotEmpty)
            ? Container(
                height: JiyueTokens.controlSm,
                padding: const EdgeInsets.symmetric(horizontal: 8),
                decoration: BoxDecoration(
                  color: colors.elevated,
                  borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                  border: Border.all(color: colors.border, width: 1),
                ),
                child: DropdownButtonHideUnderline(
                  child: DropdownButton<ExportedAiProvider>(
                    value: _selectedProvider,
                    isDense: true,
                    icon: JiyueIcon(
                      'chevron-down',
                      color: colors.textMuted,
                      size: 13,
                    ),
                    dropdownColor: colors.elevated,
                    borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
                    style: TextStyle(
                      fontSize: JiyueTokens.fontXs,
                      color: colors.textPrimary,
                    ),
                    items: aiConfig.providers.map((p) {
                      return DropdownMenuItem(
                        value: p,
                        child: Text(
                          '${p.name} (${p.chatModel})',
                          style: TextStyle(
                            fontSize: JiyueTokens.fontXs,
                            color: colors.textPrimary,
                          ),
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
              )
            : null;

    return JiyueModalSheet(
      colors: colors,
      icon: 'sparkles',
      title: 'AI 智能润色',
      headerTrailing: providerPicker,
      scrollable: false,
      maxHeightFactor: 0.85,
      child: _selectedProvider == null
          ? Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                JiyueSectionCard(
                  colors: colors,
                  child: JiyueEmptyState(
                    colors: colors,
                    icon: 'info-circle',
                    title: '未检测到已配置的 AI 服务商',
                    subtitle: '请在设置中配置服务商，或使用「扫码导入」同步 PC 端配置',
                  ),
                ),
                const SizedBox(height: 12),
                JiyueButton(
                  colors: colors,
                  label: '关闭',
                  kind: JiyueButtonKind.secondary,
                  expand: true,
                  onPressed: () => Navigator.of(context).pop(),
                ),
              ],
            )
          : Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: PolishStyle.values.map((style) {
                      final isSelected = style == _currentStyle;
                      return Padding(
                        padding: const EdgeInsets.only(right: 8),
                        child: JiyueChoiceChip(
                          colors: colors,
                          icon: style.icon,
                          label: style.label,
                          selected: isSelected,
                          onTap: () {
                            setState(() => _currentStyle = style);
                            _startPolishing();
                          },
                        ),
                      );
                    }).toList(),
                  ),
                ),
                const SizedBox(height: 12),
                Flexible(
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(minHeight: 160),
                    child: JiyueSectionCard(
                      colors: colors,
                      padding: const EdgeInsets.all(12),
                      child: _buildContentArea(colors),
                    ),
                  ),
                ),
                const SizedBox(height: 12),
                if (_resultText != null && !_isLoading)
                  Row(
                    children: [
                      Expanded(
                        child: JiyueButton(
                          colors: colors,
                          label: '插入下方',
                          icon: 'arrow-down',
                          kind: JiyueButtonKind.secondary,
                          expand: true,
                          onPressed: () {
                            widget.onApply(_resultText!, false);
                            Navigator.of(context).pop();
                          },
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: JiyueButton(
                          colors: colors,
                          label: '替换原内容',
                          icon: 'check',
                          kind: JiyueButtonKind.primary,
                          expand: true,
                          onPressed: () {
                            widget.onApply(_resultText!, true);
                            Navigator.of(context).pop();
                          },
                        ),
                      ),
                    ],
                  )
                else if (_isLoading)
                  Center(
                    child: Text(
                      '正在使用 ${_currentStyle.label} 模式生成，请稍候...',
                      style: TextStyle(
                        fontSize: JiyueTokens.fontSm,
                        color: colors.textMuted,
                      ),
                    ),
                  ),
              ],
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
              width: 22,
              height: 22,
              child: CircularProgressIndicator(
                strokeWidth: 2,
                color: colors.accent,
              ),
            ),
            const SizedBox(height: 10),
            Text(
              '正在以「${_currentStyle.label}」风格重塑文本...',
              style: TextStyle(
                fontSize: JiyueTokens.fontBase,
                color: colors.textMuted,
              ),
            ),
          ],
        ),
      );
    }

    if (_errorMessage != null) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            JiyueIcon('alert-triangle', color: colors.danger, size: 26),
            const SizedBox(height: 8),
            Text(
              '润色请求失败',
              style: TextStyle(
                fontSize: JiyueTokens.fontBase,
                fontWeight: FontWeight.w600,
                color: colors.danger,
              ),
            ),
            const SizedBox(height: 4),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: Text(
                _errorMessage!,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontSize: JiyueTokens.fontXs,
                  color: colors.textMuted,
                ),
              ),
            ),
            const SizedBox(height: 12),
            JiyueButton(
              colors: colors,
              label: '重试',
              icon: 'refresh',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
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
            padding: const EdgeInsets.only(right: 32),
            child: SelectableText(
              _resultText!,
              style: TextStyle(
                fontSize: JiyueTokens.fontMd,
                height: 1.6,
                color: colors.textPrimary,
                fontFamily: 'monospace',
              ),
            ),
          ),
          Positioned(
            top: 0,
            right: 0,
            child: JiyueIconButton(
              colors: colors,
              icon: 'copy',
              size: 28,
              iconSize: 15,
              tooltip: '复制内容',
              onPressed: _copyResult,
            ),
          ),
        ],
      );
    }

    return Center(
      child: Text(
        '选择上方风格后开始润色',
        style: TextStyle(
          fontSize: JiyueTokens.fontSm,
          color: colors.textMuted,
        ),
      ),
    );
  }
}
