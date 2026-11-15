import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/exported_config.dart';
import '../services/ai_service.dart';
import '../theme/jiyue_theme.dart';
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

  // 测试状态
  String? _testingProviderId;
  String? _testSuccessMessage;
  String? _testErrorMessage;

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

  Future<void> _testProvider(ExportedAiProvider p, [String? customModel]) async {
    setState(() {
      _testingProviderId = p.id;
      _testSuccessMessage = null;
      _testErrorMessage = null;
    });

    try {
      final res = await AiService.testConnection(
        provider: p,
        customModel: customModel,
      );
      if (mounted) {
        setState(() {
          _testingProviderId = null;
          _testSuccessMessage =
              '✓ 连接成功 (${res["latency_ms"]}ms) · 模型 ${res["model"]} 响应正常';
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _testingProviderId = null;
          _testErrorMessage = '✕ 连接测试失败: ${e.toString().replaceAll("Exception: ", "")}';
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
    String kind = existing?.kind.isNotEmpty == true ? existing!.kind : 'openaicompatible';

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
            return Container(
              decoration: BoxDecoration(
                color: colors.sheet,
                borderRadius: const BorderRadius.vertical(top: Radius.circular(24)),
              ),
              padding: EdgeInsets.fromLTRB(
                20,
                16,
                20,
                MediaQuery.of(ctx).viewInsets.bottom + 24,
              ),
              child: SingleChildScrollView(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Center(
                      child: Container(
                        width: 36,
                        height: 4,
                        decoration: BoxDecoration(
                          color: colors.border,
                          borderRadius: BorderRadius.circular(2),
                        ),
                      ),
                    ),
                    const SizedBox(height: 14),
                    Text(
                      isNew ? '添加 AI 服务商' : '编辑服务商「${existing.name}」',
                      style: TextStyle(
                        fontSize: 16,
                        fontWeight: FontWeight.bold,
                        color: colors.textPrimary,
                      ),
                    ),
                    const SizedBox(height: 14),
                    // 服务商名称
                    TextField(
                      controller: nameCtrl,
                      style: TextStyle(color: colors.textPrimary, fontSize: 13),
                      decoration: InputDecoration(
                        labelText: '服务商名称',
                        hintText: '例如: Init AI / DeepSeek / 本地 Ollama',
                        labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                        hintStyle: TextStyle(color: colors.textMuted, fontSize: 12),
                        isDense: true,
                        border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                    ),
                    const SizedBox(height: 12),
                    // 协议类型
                    Text('接口协议', style: TextStyle(fontSize: 12, color: colors.textSecondary)),
                    const SizedBox(height: 6),
                    Row(
                      children: [
                        ChoiceChip(
                          label: const Text('OpenAI 兼容'),
                          selected: kind.toLowerCase() != 'anthropic',
                          selectedColor: colors.accent.withAlpha(40),
                          labelStyle: TextStyle(
                            fontSize: 12,
                            color: kind.toLowerCase() != 'anthropic' ? colors.accent : colors.textSecondary,
                            fontWeight: kind.toLowerCase() != 'anthropic' ? FontWeight.bold : FontWeight.normal,
                          ),
                          onSelected: (s) {
                            if (s) setDialogState(() => kind = 'openaicompatible');
                          },
                        ),
                        const SizedBox(width: 8),
                        ChoiceChip(
                          label: const Text('Anthropic (Claude)'),
                          selected: kind.toLowerCase() == 'anthropic',
                          selectedColor: colors.accent.withAlpha(40),
                          labelStyle: TextStyle(
                            fontSize: 12,
                            color: kind.toLowerCase() == 'anthropic' ? colors.accent : colors.textSecondary,
                            fontWeight: kind.toLowerCase() == 'anthropic' ? FontWeight.bold : FontWeight.normal,
                          ),
                          onSelected: (s) {
                            if (s) setDialogState(() => kind = 'anthropic');
                          },
                        ),
                      ],
                    ),
                    const SizedBox(height: 12),
                    // 接口地址
                    TextField(
                      controller: urlCtrl,
                      style: TextStyle(color: colors.textPrimary, fontSize: 13),
                      decoration: InputDecoration(
                        labelText: '接口端点 (Base URL)',
                        hintText: '例如: https://api.ai.init.cool',
                        labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                        hintStyle: TextStyle(color: colors.textMuted, fontSize: 12),
                        isDense: true,
                        border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                    ),
                    const SizedBox(height: 12),
                    // 聊天模型
                    TextField(
                      controller: modelCtrl,
                      style: TextStyle(color: colors.textPrimary, fontSize: 13),
                      decoration: InputDecoration(
                        labelText: '默认对话模型',
                        hintText: '例如: deepseek-flash-eco / deepseek-chat',
                        labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                        hintStyle: TextStyle(color: colors.textMuted, fontSize: 12),
                        isDense: true,
                        border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                    ),
                    const SizedBox(height: 12),
                    // API Key
                    TextField(
                      controller: keyCtrl,
                      obscureText: true,
                      style: TextStyle(color: colors.textPrimary, fontSize: 13),
                      decoration: InputDecoration(
                        labelText: 'API 密钥 (API Key)',
                        hintText: '输入该服务商的 API Key',
                        labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                        hintStyle: TextStyle(color: colors.textMuted, fontSize: 12),
                        isDense: true,
                        border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                    ),
                    const SizedBox(height: 14),

                    // 指定测试模型与连通性测试
                    Container(
                      padding: const EdgeInsets.all(12),
                      decoration: BoxDecoration(
                        color: colors.raised,
                        borderRadius: BorderRadius.circular(12),
                        border: Border.all(color: colors.border),
                      ),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Row(
                            children: [
                              JiyueIcon('zap', color: colors.accent, size: 14),
                              const SizedBox(width: 6),
                              Text(
                                '指定测试模型与连通性',
                                style: TextStyle(
                                  fontSize: 12,
                                  fontWeight: FontWeight.bold,
                                  color: colors.textPrimary,
                                ),
                              ),
                            ],
                          ),
                          const SizedBox(height: 8),
                          TextField(
                            controller: testModelCtrl,
                            style: TextStyle(color: colors.textPrimary, fontSize: 12),
                            decoration: InputDecoration(
                              hintText: '测试用指定模型 (留空则默认使用上方对话模型)',
                              hintStyle: TextStyle(color: colors.textMuted, fontSize: 11),
                              isDense: true,
                              contentPadding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                              border: OutlineInputBorder(borderRadius: BorderRadius.circular(8)),
                            ),
                          ),
                          const SizedBox(height: 10),
                          Row(
                            mainAxisAlignment: MainAxisAlignment.spaceBetween,
                            children: [
                              Expanded(
                                child: Text(
                                  dialogTestResult ?? '点击右侧按钮发起连接探测',
                                  style: TextStyle(
                                    fontSize: 11,
                                    color: dialogTestResult?.startsWith('✓') == true
                                        ? const Color(0xFF2E7D32)
                                        : (dialogTestResult?.startsWith('✕') == true
                                            ? Colors.red
                                            : colors.textMuted),
                                  ),
                                ),
                              ),
                              ElevatedButton.icon(
                                style: ElevatedButton.styleFrom(
                                  backgroundColor: colors.accent,
                                  foregroundColor: colors.onAccent,
                                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                                  shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
                                ),
                                icon: dialogTesting
                                    ? SizedBox(
                                        width: 12,
                                        height: 12,
                                        child: CircularProgressIndicator(strokeWidth: 2, color: colors.onAccent),
                                      )
                                    : JiyueIcon('zap', color: colors.onAccent, size: 12),
                                label: const Text('测试', style: TextStyle(fontSize: 11)),
                                onPressed: dialogTesting
                                    ? null
                                    : () async {
                                        if (urlCtrl.text.trim().isEmpty) {
                                          setDialogState(() => dialogTestResult = '✕ 请先填写接口端点');
                                          return;
                                        }
                                        setDialogState(() {
                                          dialogTesting = true;
                                          dialogTestResult = '正在测试连接中...';
                                        });

                                        final tempProvider = ExportedAiProvider(
                                          id: existing?.id ?? 'temp_${DateTime.now().millisecondsSinceEpoch}',
                                          name: nameCtrl.text.trim(),
                                          kind: kind,
                                          baseUrl: urlCtrl.text.trim(),
                                          chatModel: modelCtrl.text.trim(),
                                          apiKey: keyCtrl.text.trim(),
                                        );

                                        try {
                                          final res = await AiService.testConnection(
                                            provider: tempProvider,
                                            customModel: testModelCtrl.text.trim(),
                                          );
                                          setDialogState(() {
                                            dialogTesting = false;
                                            dialogTestResult =
                                                '✓ 成功 (${res["latency_ms"]}ms) · 模型 ${res["model"]}';
                                          });
                                        } catch (e) {
                                          setDialogState(() {
                                            dialogTesting = false;
                                            dialogTestResult = '✕ 失败: ${e.toString().replaceAll("Exception: ", "")}';
                                          });
                                        }
                                      },
                              ),
                            ],
                          ),
                        ],
                      ),
                    ),

                    const SizedBox(height: 18),
                    Row(
                      children: [
                        Expanded(
                          child: OutlinedButton(
                            style: OutlinedButton.styleFrom(
                              foregroundColor: colors.textSecondary,
                              side: BorderSide(color: colors.border),
                              padding: const EdgeInsets.symmetric(vertical: 12),
                              shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                            ),
                            onPressed: () => Navigator.of(ctx).pop(),
                            child: const Text('取消'),
                          ),
                        ),
                        const SizedBox(width: 12),
                        Expanded(
                          child: ElevatedButton(
                            style: ElevatedButton.styleFrom(
                              backgroundColor: colors.accent,
                              foregroundColor: colors.onAccent,
                              padding: const EdgeInsets.symmetric(vertical: 12),
                              shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                            ),
                            onPressed: () {
                              final name = nameCtrl.text.trim();
                              final url = urlCtrl.text.trim();
                              final model = modelCtrl.text.trim();
                              final key = keyCtrl.text.trim();
                              if (name.isEmpty || url.isEmpty || model.isEmpty) {
                                ScaffoldMessenger.of(ctx).showSnackBar(
                                  const SnackBar(content: Text('请填写完整的服务商名称、端点及默认模型')),
                                );
                                return;
                              }

                              final providerId = existing?.id ?? 'provider_${DateTime.now().millisecondsSinceEpoch}';
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
                                  final idx = _providers.indexWhere((p) => p.id == existing.id);
                                  if (idx >= 0) {
                                    _providers[idx] = newProvider;
                                  }
                                }
                              });

                              Navigator.of(ctx).pop();
                            },
                            child: const Text('保存服务商', style: TextStyle(fontWeight: FontWeight.bold)),
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
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

    return Container(
      decoration: BoxDecoration(
        color: colors.sheet,
        borderRadius: const BorderRadius.vertical(top: Radius.circular(24)),
      ),
      padding: const EdgeInsets.fromLTRB(20, 16, 20, 24),
      child: SafeArea(
        top: false,
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              // 拖动把手
              Center(
                child: Container(
                  width: 36,
                  height: 4,
                  decoration: BoxDecoration(
                    color: colors.border,
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),
              const SizedBox(height: 14),
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Row(
                    children: [
                      JiyueIcon('sparkles', color: colors.accent, size: 20),
                      const SizedBox(width: 8),
                      Text(
                        'AI 助手与服务商管理',
                        style: TextStyle(
                          fontSize: 17,
                          fontWeight: FontWeight.bold,
                          color: colors.textPrimary,
                        ),
                      ),
                    ],
                  ),
                  IconButton(
                    icon: JiyueIcon('x', color: colors.textSecondary, size: 20),
                    onPressed: () => Navigator.of(context).pop(),
                  ),
                ],
              ),
              const SizedBox(height: 10),

              // 全局开关
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(12),
                  border: Border.all(color: colors.border),
                ),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          '开启 AI 智能润色与提炼',
                          style: TextStyle(
                            fontSize: 13,
                            fontWeight: FontWeight.bold,
                            color: colors.textPrimary,
                          ),
                        ),
                        const SizedBox(height: 2),
                        Text(
                          '在笔记详情中使用多风格润色及摘要提炼',
                          style: TextStyle(fontSize: 11, color: colors.textSecondary),
                        ),
                      ],
                    ),
                    Switch(
                      value: _enabled,
                      activeColor: colors.accent,
                      onChanged: (v) => setState(() => _enabled = v),
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 16),
              // 连通性测试全局提示
              if (_testSuccessMessage != null) ...[
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(10),
                  decoration: BoxDecoration(
                    color: const Color(0xFF2E7D32).withAlpha(30),
                    borderRadius: BorderRadius.circular(8),
                    border: Border.all(color: const Color(0xFF2E7D32)),
                  ),
                  child: Text(
                    _testSuccessMessage!,
                    style: const TextStyle(fontSize: 11, color: Color(0xFF2E7D32), fontWeight: FontWeight.bold),
                  ),
                ),
                const SizedBox(height: 12),
              ],
              if (_testErrorMessage != null) ...[
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(10),
                  decoration: BoxDecoration(
                    color: Colors.red.withAlpha(30),
                    borderRadius: BorderRadius.circular(8),
                    border: Border.all(color: Colors.red),
                  ),
                  child: Text(
                    _testErrorMessage!,
                    style: const TextStyle(fontSize: 11, color: Colors.red, fontWeight: FontWeight.bold),
                  ),
                ),
                const SizedBox(height: 12),
              ],

              // 服务商列表
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Text(
                    '已配置服务商 (${_providers.length})',
                    style: TextStyle(
                      fontSize: 13,
                      fontWeight: FontWeight.bold,
                      color: colors.textPrimary,
                    ),
                  ),
                  TextButton.icon(
                    onPressed: () => _openEditDialog(),
                    icon: JiyueIcon('plus', color: colors.accent, size: 14),
                    label: Text('新增服务商', style: TextStyle(fontSize: 12, color: colors.accent)),
                  ),
                ],
              ),
              const SizedBox(height: 8),

              if (_providers.isEmpty)
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(24),
                  decoration: BoxDecoration(
                    color: colors.raised,
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(color: colors.border),
                  ),
                  child: Column(
                    children: [
                      JiyueIcon('alert-circle', color: colors.textMuted, size: 28),
                      const SizedBox(height: 8),
                      Text('尚未配置 AI 服务商', style: TextStyle(fontSize: 12, color: colors.textSecondary)),
                      const SizedBox(height: 4),
                      Text('可点击上方「新增服务商」或通过扫描 PC 端二维码一键导入', style: TextStyle(fontSize: 11, color: colors.textMuted)),
                    ],
                  ),
                )
              else
                ..._providers.map((p) {
                  final isActive = _activeChat == p.id;
                  final isTesting = _testingProviderId == p.id;
                  final isAnthropic = p.kind.toLowerCase() == 'anthropic';

                  return Container(
                    margin: const EdgeInsets.only(bottom: 10),
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      color: colors.raised,
                      borderRadius: BorderRadius.circular(12),
                      border: Border.all(
                        color: isActive ? colors.accent : colors.border,
                        width: isActive ? 1.5 : 1,
                      ),
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          children: [
                            Text(
                              p.name,
                              style: TextStyle(
                                fontSize: 14,
                                fontWeight: FontWeight.bold,
                                color: colors.textPrimary,
                              ),
                            ),
                            const SizedBox(width: 8),
                            Container(
                              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                              decoration: BoxDecoration(
                                color: isAnthropic ? Colors.purple.withAlpha(35) : Colors.green.withAlpha(35),
                                borderRadius: BorderRadius.circular(6),
                              ),
                              child: Text(
                                isAnthropic ? 'Anthropic' : 'OpenAI',
                                style: TextStyle(
                                  fontSize: 10,
                                  fontWeight: FontWeight.bold,
                                  color: isAnthropic ? Colors.purple : Colors.green,
                                ),
                              ),
                            ),
                            if (isActive) ...[
                              const SizedBox(width: 6),
                              Container(
                                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                                decoration: BoxDecoration(
                                  color: colors.accent.withAlpha(35),
                                  borderRadius: BorderRadius.circular(6),
                                ),
                                child: Text(
                                  '当前默认',
                                  style: TextStyle(
                                    fontSize: 10,
                                    fontWeight: FontWeight.bold,
                                    color: colors.accent,
                                  ),
                                ),
                              ),
                            ],
                            const Spacer(),
                            // 操作按钮
                            IconButton(
                              icon: JiyueIcon('edit-2', color: colors.textSecondary, size: 14),
                              padding: EdgeInsets.zero,
                              constraints: const BoxConstraints(),
                              onPressed: () => _openEditDialog(p),
                            ),
                            const SizedBox(width: 10),
                            IconButton(
                              icon: JiyueIcon('trash-2', color: Colors.redAccent, size: 14),
                              padding: EdgeInsets.zero,
                              constraints: const BoxConstraints(),
                              onPressed: () {
                                setState(() {
                                  _providers.removeWhere((item) => item.id == p.id);
                                  if (_activeChat == p.id) {
                                    _activeChat = _providers.isNotEmpty ? _providers.first.id : null;
                                  }
                                });
                              },
                            ),
                          ],
                        ),
                        const SizedBox(height: 6),
                        Text(
                          '${p.baseUrl} · ${p.chatModel}',
                          style: TextStyle(fontSize: 11, color: colors.textSecondary),
                        ),
                        const SizedBox(height: 4),
                        Text(
                          p.apiKey.isNotEmpty ? '密钥: •••••••••••• (已配置)' : '未配置密钥',
                          style: TextStyle(
                            fontSize: 10,
                            color: p.apiKey.isNotEmpty ? const Color(0xFF2E7D32) : Colors.orange,
                          ),
                        ),
                        const Divider(height: 16),
                        Row(
                          mainAxisAlignment: MainAxisAlignment.spaceBetween,
                          children: [
                            if (!isActive)
                              TextButton(
                                onPressed: () => setState(() => _activeChat = p.id),
                                child: Text('设为默认模型', style: TextStyle(fontSize: 11, color: colors.accent)),
                              )
                            else
                              Text('✓ 当前默认对话模型', style: TextStyle(fontSize: 11, color: colors.accent, fontWeight: FontWeight.bold)),
                            ElevatedButton.icon(
                              style: ElevatedButton.styleFrom(
                                backgroundColor: colors.sheet,
                                foregroundColor: colors.textPrimary,
                                elevation: 0,
                                side: BorderSide(color: colors.border),
                                padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                                shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
                              ),
                              icon: isTesting
                                  ? SizedBox(
                                      width: 12,
                                      height: 12,
                                      child: CircularProgressIndicator(strokeWidth: 2, color: colors.accent),
                                    )
                                  : JiyueIcon('zap', color: colors.accent, size: 12),
                              label: Text(isTesting ? '测试中...' : '测试连接', style: const TextStyle(fontSize: 11)),
                              onPressed: isTesting ? null : () => _testProvider(p),
                            ),
                          ],
                        ),
                      ],
                    ),
                  );
                }),

              const SizedBox(height: 20),
              // 保存按钮
              SizedBox(
                width: double.infinity,
                height: 44,
                child: ElevatedButton.icon(
                  style: ElevatedButton.styleFrom(
                    backgroundColor: colors.accent,
                    foregroundColor: colors.onAccent,
                    shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(22)),
                  ),
                  icon: JiyueIcon('check', color: colors.onAccent, size: 16),
                  label: const Text('保存并应用 AI 设置', style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold)),
                  onPressed: _saveConfig,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
