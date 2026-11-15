import 'dart:io';
import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/exported_config.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_controls.dart';

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
  JiyueBadgeTone? _testTone;
  String? _testMessage;

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
      setState(() {
        _testTone = JiyueBadgeTone.warn;
        _testMessage = '请先填写 Memos 服务地址';
      });
      return;
    }

    setState(() {
      _testing = true;
      _testTone = JiyueBadgeTone.neutral;
      _testMessage = '正在连接 Memos 服务...';
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

      if (mounted) {
        setState(() {
          _testing = false;
          _testTone = JiyueBadgeTone.ok;
          _testMessage =
              'Memos 服务响应正常 (${stopwatch.elapsedMilliseconds}ms, HTTP ${resp.statusCode})';
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _testing = false;
          _testTone = JiyueBadgeTone.danger;
          _testMessage = '连接 Memos 失败: $e';
        });
      }
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

    return JiyueModalSheet(
      colors: colors,
      icon: 'calendar',
      title: 'Memos 备忘录服务配置',
      subtitle: '绑定私有部署或云端 Memos 实例',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          JiyueTextField(
            colors: colors,
            controller: _endpointCtrl,
            label: 'Memos 服务端点 (Endpoint URL)',
            hintText: '例如: https://memo.init.cool',
          ),
          const SizedBox(height: 12),
          JiyueTextField(
            colors: colors,
            controller: _tokenCtrl,
            obscureText: true,
            label: 'Access Token (个人访问令牌)',
            hintText: '例如: memos_pat_xxx',
          ),
          const SizedBox(height: 12),
          if (_testMessage != null && _testTone != null) ...[
            JiyueBanner(
              colors: colors,
              tone: _testTone!,
              message: _testMessage!,
            ),
            const SizedBox(height: 10),
          ],
          Row(
            children: [
              Expanded(
                child: JiyueButton(
                  colors: colors,
                  label: _testing ? '连接中...' : '测试连接',
                  icon: 'bolt',
                  loading: _testing,
                  kind: JiyueButtonKind.secondary,
                  expand: true,
                  onPressed: _testing ? null : _testMemosConnection,
                ),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: JiyueButton(
                  colors: colors,
                  label: '保存配置',
                  icon: 'check',
                  kind: JiyueButtonKind.primary,
                  expand: true,
                  onPressed: _saveConfig,
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
