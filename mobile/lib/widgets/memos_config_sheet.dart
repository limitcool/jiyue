import 'dart:io';
import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/exported_config.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_icon.dart';

class MemosConfigSheet extends StatefulWidget {
  final VaultService vaultService;
  final JiyueThemeColors colors;

  const MemosConfigSheet({
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
      builder: (_) => MemosConfigSheet(
        vaultService: vaultService,
        colors: colors,
      ),
    );
  }

  @override
  State<MemosConfigSheet> createState() => _MemosConfigSheetState();
}

class _MemosConfigSheetState extends State<MemosConfigSheet> {
  late final TextEditingController _endpointCtrl;
  late final TextEditingController _tokenCtrl;

  bool _testing = false;
  String? _testResult;

  @override
  void initState() {
    super.initState();
    final cfg = widget.vaultService.memosConfig;
    _endpointCtrl = TextEditingController(text: cfg?.endpoint ?? '');
    _tokenCtrl = TextEditingController(text: cfg?.token ?? '');
  }

  @override
  void dispose() {
    _endpointCtrl.dispose();
    _tokenCtrl.dispose();
    super.dispose();
  }

  Future<void> _testMemosConnection() async {
    final endpoint = _endpointCtrl.text.trim().replaceAll(RegExp(r'/+$'), '');
    final token = _tokenCtrl.text.trim();
    if (endpoint.isEmpty) {
      setState(() => _testResult = '✕ 请先填写 Memos 服务地址');
      return;
    }

    setState(() {
      _testing = true;
      _testResult = '正在连接 Memos 服务...';
    });

    final stopwatch = Stopwatch()..start();
    final client = HttpClient()..connectionTimeout = const Duration(seconds: 8);
    try {
      final uri = Uri.parse('$endpoint/api/v1/ping');
      final req = await client.getUrl(uri);
      if (token.isNotEmpty) {
        req.headers.set('Authorization', 'Bearer $token');
      }
      final resp = await req.close();
      stopwatch.stop();

      setState(() {
        _testing = false;
        _testResult = '✓ Memos 服务响应正常 (${stopwatch.elapsedMilliseconds}ms, HTTP ${resp.statusCode})';
      });
    } catch (e) {
      setState(() {
        _testing = false;
        _testResult = '✕ 连接 Memos 失败: $e';
      });
    } finally {
      client.close();
    }
  }

  Future<void> _saveConfig() async {
    final endpoint = _endpointCtrl.text.trim();
    final token = _tokenCtrl.text.trim();

    if (endpoint.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('服务地址不可为空')),
      );
      return;
    }

    final newCfg = ExportedMemosConfig(
      endpoint: endpoint,
      token: token,
    );

    await widget.vaultService.saveMemosConfig(newCfg);
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Memos 备忘录配置已保存')),
      );
      Navigator.of(context).pop();
    }
  }

  @override
  Widget build(BuildContext context) {
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
        MediaQuery.of(context).viewInsets.bottom + 24,
      ),
      child: SafeArea(
        top: false,
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
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Row(
                    children: [
                      JiyueIcon('calendar', color: colors.accent, size: 20),
                      const SizedBox(width: 8),
                      Text(
                        'Memos 备忘录服务配置',
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
              const SizedBox(height: 14),

              // 端点 URL
              TextField(
                controller: _endpointCtrl,
                style: TextStyle(color: colors.textPrimary, fontSize: 13),
                decoration: InputDecoration(
                  labelText: 'Memos 服务端点 (Endpoint URL)',
                  hintText: '例如: https://memo.init.cool',
                  labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                  hintStyle: TextStyle(color: colors.textMuted, fontSize: 12),
                  isDense: true,
                  border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                ),
              ),
              const SizedBox(height: 12),

              // Access Token
              TextField(
                controller: _tokenCtrl,
                obscureText: true,
                style: TextStyle(color: colors.textPrimary, fontSize: 13),
                decoration: InputDecoration(
                  labelText: 'Access Token (个人访问令牌)',
                  hintText: '例如: memos_pat_xxx',
                  labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                  isDense: true,
                  border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                ),
              ),

              const SizedBox(height: 16),
              // 测试端点按钮与反馈
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Expanded(
                    child: Text(
                      _testResult ?? '建议保存前测试 Memos 服务响应',
                      style: TextStyle(
                        fontSize: 11,
                        color: _testResult?.startsWith('✓') == true
                            ? const Color(0xFF2E7D32)
                            : (_testResult?.startsWith('✕') == true ? Colors.red : colors.textMuted),
                      ),
                    ),
                  ),
                  OutlinedButton.icon(
                    style: OutlinedButton.styleFrom(
                      foregroundColor: colors.accent,
                      side: BorderSide(color: colors.border),
                      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
                    ),
                    icon: _testing
                        ? SizedBox(
                            width: 12,
                            height: 12,
                            child: CircularProgressIndicator(strokeWidth: 2, color: colors.accent),
                          )
                        : JiyueIcon('zap', color: colors.accent, size: 12),
                    label: const Text('测试连接', style: TextStyle(fontSize: 11)),
                    onPressed: _testing ? null : _testMemosConnection,
                  ),
                ],
              ),

              const SizedBox(height: 18),
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
                  label: const Text('保存 Memos 配置', style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold)),
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
