import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/exported_config.dart';
import '../services/ai_service.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_controls.dart';
import 'jiyue_icon.dart';

class AiConfigSheet extends StatefulWidget {
  final VaultService vaultService;
  final JiyueThemeColors colors;

  const AiConfigSheet({
    super.key,
    required this.vaultService,
    required this.colors,
  });

  static Future<void> show(
    BuildContext context, {
    required VaultService vaultService,
    required JiyueThemeColors colors,
  }) {
    return showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (_) => AiConfigSheet(
        vaultService: vaultService,
        colors: colors,
      ),
    );
  }

  @override
  State<AiConfigSheet> createState() => _AiConfigSheetState();
}

class _AiConfigSheetState extends State<AiConfigSheet> {
  late bool _enabled;
  String? _activeChat;
  late List<ExportedAiProvider> _providers;

  String? _testingProviderId;
  JiyueBadgeTone? _testTone;
  String? _testMessage;

  @override
  void initState() {
    super.initState();
    final cfg = widget.vaultService.aiConfig;
    _enabled = cfg?.enabled ?? true;
    _activeChat = cfg?.activeChat;
    _providers = List.from(cfg?.providers ?? []);
    if (_activeChat == null && _providers.isNotEmpty) {
      _activeChat = _providers.first.id;
    }
  }

  Future<void> _saveConfig() async {
    final updated = ExportedAiConfig(
      enabled: _enabled,
      activeChat: _activeChat,
      providers: _providers,
    );
    await widget.vaultService.saveAiConfig(updated);
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('AI 配置已成功保存')),
      );
      Navigator.of(context).pop();
    }
  }

  Future<void> _testProvider(
    ExportedAiProvider p, [
    String? customModel,
  ]) async {
    setState(() {
      _testingProviderId = p.id;
      _testTone = JiyueBadgeTone.neutral;
      _testMessage = '正在探测 ${p.name} 连接...';
    });

    try {
      final res = await AiService.testConnection(
        provider: p,
        customModel: customModel,
      );
      if (mounted) {
        setState(() {
          _testingProviderId = null;
          _testTone = JiyueBadgeTone.ok;
          _testMessage =
              '连接成功 (${res["latency_ms"]}ms) · 模型 ${res["model"]} 响应正常';
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _testingProviderId = null;
          _testTone = JiyueBadgeTone.danger;
          _testMessage =
              '连接测试失败: ${e.toString().replaceAll("Exception: ", "")}';
        });
      }
    }
  }

  void _openEditDialog([ExportedAiProvider? existing]) {
    final isNew = existing == null;
    final nameCtrl = TextEditingController(text: existing?.name ?? '');
    final urlCtrl = TextEditingController(text: existing?.baseUrl ?? '');
    final modelCtrl = TextEditingController(text: existing?.chatModel ?? '');
    final keyCtrl = TextEditingController(text: existing?.apiKey ?? '');
    final testModelCtrl = TextEditingController();
    String kind =
        existing?.kind.isNotEmpty == true ? existing!.kind : 'openaicompatible';

    JiyueBadgeTone? dialogTone;
    String? dialogTestResult;
    bool dialogTesting = false;

    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder: (ctx) {
        return StatefulBuilder(
          builder: (dialogCtx, setDialogState) {
            final colors = widget.colors;
            final bool isAnthropic = kind.toLowerCase() == 'anthropic';

            return JiyueModalSheet(
              colors: colors,
              icon: 'sparkles',
              title: isNew ? '添加 AI 服务商' : '编辑服务商「${existing.name}」',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  JiyueTextField(
                    colors: colors,
                    controller: nameCtrl,
                    label: '服务商名称',
                    hintText: '例如: Init AI / DeepSeek / 本地 Ollama',
                  ),
                  const SizedBox(height: 12),
                  Text(
                    '接口协议',
                    style: TextStyle(
                      fontSize: JiyueTokens.fontSm,
                      fontWeight: FontWeight.w500,
                      color: colors.textSecondary,
                    ),
                  ),
                  const SizedBox(height: 6),
                  JiyueSegmented(
                    colors: colors,
                    fullWidth: true,
                    items: const [
                      ('OpenAI 兼容', null),
                      ('Anthropic (Claude)', null),
                    ],
                    selectedIndex: isAnthropic ? 1 : 0,
                    onChanged: (idx) {
                      setDialogState(() {
                        kind = idx == 1 ? 'anthropic' : 'openaicompatible';
                      });
                    },
                  ),
                  const SizedBox(height: 12),
                  JiyueTextField(
                    colors: colors,
                    controller: urlCtrl,
                    label: '接口端点 (Base URL)',
                    hintText: '例如: https://api.ai.init.cool',
                  ),
                  const SizedBox(height: 12),
                  JiyueTextField(
                    colors: colors,
                    controller: modelCtrl,
                    label: '默认对话模型',
                    hintText: '例如: deepseek-flash-eco / deepseek-chat',
                  ),
                  const SizedBox(height: 12),
                  JiyueTextField(
                    colors: colors,
                    controller: keyCtrl,
                    obscureText: true,
                    label: 'API 密钥 (API Key)',
                    hintText: '输入该服务商的 API Key',
                  ),
                  const SizedBox(height: 14),
                  JiyueSectionCard(
                    colors: colors,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          children: [
                            JiyueIcon('bolt', color: colors.accent, size: 14),
                            const SizedBox(width: 6),
                            Text(
                              '指定测试模型与连通性',
                              style: TextStyle(
                                fontSize: JiyueTokens.fontSm,
                                fontWeight: FontWeight.w600,
                                color: colors.textPrimary,
                              ),
                            ),
                          ],
                        ),
                        const SizedBox(height: 8),
                        JiyueTextField(
                          colors: colors,
                          controller: testModelCtrl,
                          hintText: '测试用指定模型 (留空默认使用上方对话模型)',
                        ),
                        const SizedBox(height: 10),
                        if (dialogTestResult != null && dialogTone != null) ...[
                          JiyueBanner(
                            colors: colors,
                            tone: dialogTone!,
                            message: dialogTestResult!,
                          ),
                          const SizedBox(height: 8),
                        ],
                        Align(
                          alignment: Alignment.centerRight,
                          child: JiyueButton(
                            colors: colors,
                            label: dialogTesting ? '探测中...' : '测试连接',
                            icon: 'bolt',
                            loading: dialogTesting,
                            kind: JiyueButtonKind.secondary,
                            size: JiyueControlSize.sm,
                            onPressed: dialogTesting
                                ? null
                                : () async {
                                    if (urlCtrl.text.trim().isEmpty) {
                                      setDialogState(() {
                                        dialogTone = JiyueBadgeTone.warn;
                                        dialogTestResult = '请先填写接口端点';
                                      });
                                      return;
                                    }
                                    setDialogState(() {
                                      dialogTesting = true;
                                      dialogTone = JiyueBadgeTone.neutral;
                                      dialogTestResult = '正在测试连接中...';
                                    });

                                    final tempProvider = ExportedAiProvider(
                                      id: existing?.id ??
                                          'temp_${DateTime.now().millisecondsSinceEpoch}',
                                      name: nameCtrl.text.trim(),
                                      kind: kind,
                                      baseUrl: urlCtrl.text.trim(),
                                      chatModel: modelCtrl.text.trim(),
                                      apiKey: keyCtrl.text.trim(),
                                    );

                                    try {
                                      final res =
                                          await AiService.testConnection(
                                        provider: tempProvider,
                                        customModel:
                                            testModelCtrl.text.trim(),
                                      );
                                      setDialogState(() {
                                        dialogTesting = false;
                                        dialogTone = JiyueBadgeTone.ok;
                                        dialogTestResult =
                                            '连接成功 (${res["latency_ms"]}ms) · 模型 ${res["model"]}';
                                      });
                                    } catch (e) {
                                      setDialogState(() {
                                        dialogTesting = false;
                                        dialogTone = JiyueBadgeTone.danger;
                                        dialogTestResult =
                                            '失败: ${e.toString().replaceAll("Exception: ", "")}';
                                      });
                                    }
                                  },
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 16),
                  Row(
                    children: [
                      Expanded(
                        child: JiyueButton(
                          colors: colors,
                          label: '取消',
                          kind: JiyueButtonKind.secondary,
                          expand: true,
                          onPressed: () => Navigator.of(ctx).pop(),
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: JiyueButton(
                          colors: colors,
                          label: '保存服务商',
                          icon: 'check',
                          kind: JiyueButtonKind.primary,
                          expand: true,
                          onPressed: () {
                            final name = nameCtrl.text.trim();
                            final url = urlCtrl.text.trim();
                            final model = modelCtrl.text.trim();
                            final key = keyCtrl.text.trim();
                            if (name.isEmpty ||
                                url.isEmpty ||
                                model.isEmpty) {
                              ScaffoldMessenger.of(ctx).showSnackBar(
                                const SnackBar(
                                  content: Text('请填写完整的服务商名称、端点及默认模型'),
                                ),
                              );
                              return;
                            }

                            final providerId = existing?.id ??
                                'provider_${DateTime.now().millisecondsSinceEpoch}';
                            final newProvider = ExportedAiProvider(
                              id: providerId,
                              name: name,
                              kind: kind,
                              baseUrl: url,
                              chatModel: model,
                              apiKey: key,
                            );

                            setState(() {
                              if (isNew) {
                                _providers.add(newProvider);
                                _activeChat ??= providerId;
                              } else {
                                final idx = _providers
                                    .indexWhere((p) => p.id == existing.id);
                                if (idx >= 0) {
                                  _providers[idx] = newProvider;
                                }
                              }
                            });

                            Navigator.of(ctx).pop();
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
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.colors;

    return JiyueModalSheet(
      colors: colors,
      icon: 'sparkles',
      title: 'AI 助手与服务商管理',
      subtitle: '配置 OpenAI 兼容或 Anthropic 模型服务并测试连通性',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          JiyueSectionCard(
            colors: colors,
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: JiyueSwitchRow(
              colors: colors,
              title: '开启 AI 智能润色与提炼',
              subtitle: '在阅读器与编辑器中提供多风格润色及摘要提炼',
              value: _enabled,
              onChanged: (v) => setState(() => _enabled = v),
            ),
          ),
          const SizedBox(height: 12),
          if (_testMessage != null && _testTone != null) ...[
            JiyueBanner(
              colors: colors,
              tone: _testTone!,
              message: _testMessage!,
            ),
            const SizedBox(height: 12),
          ],
          JiyueSectionHeader(
            colors: colors,
            title: '已配置服务商 (${_providers.length})',
            trailing: JiyueButton(
              colors: colors,
              label: '新增服务商',
              icon: 'plus',
              kind: JiyueButtonKind.ghost,
              size: JiyueControlSize.xs,
              onPressed: () => _openEditDialog(),
            ),
          ),
          if (_providers.isEmpty)
            JiyueSectionCard(
              colors: colors,
              child: JiyueEmptyState(
                colors: colors,
                icon: 'bulb',
                title: '尚未配置 AI 服务商',
                subtitle: '点击右上角「新增服务商」或通过扫描 PC 端二维码一键导入',
                action: JiyueButton(
                  colors: colors,
                  label: '新增服务商',
                  icon: 'plus',
                  kind: JiyueButtonKind.primary,
                  size: JiyueControlSize.sm,
                  onPressed: () => _openEditDialog(),
                ),
              ),
            )
          else
            ..._providers.map((p) {
              final isActive = _activeChat == p.id;
              final isTesting = _testingProviderId == p.id;
              final isAnthropic = p.kind.toLowerCase() == 'anthropic';

              return JiyueSectionCard(
                colors: colors,
                active: isActive,
                margin: const EdgeInsets.only(bottom: 10),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Expanded(
                          child: Row(
                            children: [
                              Flexible(
                                child: Text(
                                  p.name,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: TextStyle(
                                    fontSize: JiyueTokens.fontMd,
                                    fontWeight: FontWeight.w600,
                                    color: colors.textPrimary,
                                  ),
                                ),
                              ),
                              const SizedBox(width: 6),
                              JiyueBadge(
                                colors: colors,
                                label: isAnthropic ? 'Anthropic' : 'OpenAI',
                                tone: isAnthropic
                                    ? JiyueBadgeTone.warn
                                    : JiyueBadgeTone.ok,
                              ),
                              if (isActive) ...[
                                const SizedBox(width: 6),
                                JiyueBadge(
                                  colors: colors,
                                  label: '当前默认',
                                  tone: JiyueBadgeTone.accent,
                                ),
                              ],
                            ],
                          ),
                        ),
                        const SizedBox(width: 6),
                        JiyueIconButton(
                          colors: colors,
                          icon: 'pencil',
                          size: 28,
                          iconSize: 14,
                          tooltip: '编辑服务商',
                          onPressed: () => _openEditDialog(p),
                        ),
                        const SizedBox(width: 2),
                        JiyueIconButton(
                          colors: colors,
                          icon: 'trash',
                          size: 28,
                          iconSize: 14,
                          danger: true,
                          tooltip: '删除服务商',
                          onPressed: () {
                            setState(() {
                              _providers
                                  .removeWhere((item) => item.id == p.id);
                              if (_activeChat == p.id) {
                                _activeChat = _providers.isNotEmpty
                                    ? _providers.first.id
                                    : null;
                              }
                            });
                          },
                        ),
                      ],
                    ),
                    const SizedBox(height: 6),
                    Text(
                      '${p.baseUrl} · ${p.chatModel}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: JiyueTokens.fontXs,
                        color: colors.textSecondary,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      p.apiKey.isNotEmpty ? '密钥状态: 已配置' : '密钥状态: 未配置 API Key',
                      style: TextStyle(
                        fontSize: JiyueTokens.fontXs,
                        color: p.apiKey.isNotEmpty ? colors.ok : colors.warn,
                      ),
                    ),
                    JiyueDivider(
                      colors: colors,
                      margin: const EdgeInsets.symmetric(vertical: 10),
                    ),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        if (!isActive)
                          JiyueButton(
                            colors: colors,
                            label: '设为默认模型',
                            kind: JiyueButtonKind.ghost,
                            size: JiyueControlSize.xs,
                            onPressed: () =>
                                setState(() => _activeChat = p.id),
                          )
                        else
                          JiyueBadge(
                            colors: colors,
                            label: '默认对话模型',
                            tone: JiyueBadgeTone.accent,
                            icon: 'check',
                          ),
                        JiyueButton(
                          colors: colors,
                          label: isTesting ? '测试中...' : '测试连接',
                          icon: 'bolt',
                          loading: isTesting,
                          kind: JiyueButtonKind.secondary,
                          size: JiyueControlSize.xs,
                          onPressed: isTesting ? null : () => _testProvider(p),
                        ),
                      ],
                    ),
                  ],
                ),
              );
            }),
          const SizedBox(height: 10),
          JiyueButton(
            colors: colors,
            label: '保存并应用 AI 设置',
            icon: 'check',
            kind: JiyueButtonKind.primary,
            size: JiyueControlSize.lg,
            expand: true,
            onPressed: _saveConfig,
          ),
        ],
      ),
    );
  }
}
