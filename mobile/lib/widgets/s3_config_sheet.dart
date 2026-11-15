import 'dart:io';
import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/sync_config.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_controls.dart';

class S3ConfigSheet extends StatefulWidget {
  final VaultService vaultService;
  final JiyueThemeColors colors;

  const S3ConfigSheet({
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
      builder: (_) => S3ConfigSheet(
        vaultService: vaultService,
        colors: colors,
      ),
    );
  }

  @override
  State<S3ConfigSheet> createState() => _S3ConfigSheetState();
}

class _S3ConfigSheetState extends State<S3ConfigSheet> {
  late final TextEditingController _endpointCtrl;
  late final TextEditingController _bucketCtrl;
  late final TextEditingController _regionCtrl;
  late final TextEditingController _accessKeyCtrl;
  late final TextEditingController _secretKeyCtrl;
  late final TextEditingController _prefixCtrl;
  late bool _forcePathStyle;

  bool _testing = false;
  JiyueBadgeTone? _testTone;
  String? _testMessage;

  @override
  void initState() {
    super.initState();
    final cfg = widget.vaultService.s3Config;
    _endpointCtrl = TextEditingController(text: cfg?.endpoint ?? '');
    _bucketCtrl = TextEditingController(text: cfg?.bucket ?? '');
    _regionCtrl = TextEditingController(text: cfg?.region ?? 'us-east-1');
    _accessKeyCtrl = TextEditingController(text: cfg?.accessKeyId ?? '');
    _secretKeyCtrl = TextEditingController(text: cfg?.secretAccessKey ?? '');
    _prefixCtrl = TextEditingController(text: cfg?.rootPrefix ?? '');
    _forcePathStyle = cfg?.forcePathStyle ?? true;
  }

  @override
  void dispose() {
    _endpointCtrl.dispose();
    _bucketCtrl.dispose();
    _regionCtrl.dispose();
    _accessKeyCtrl.dispose();
    _secretKeyCtrl.dispose();
    _prefixCtrl.dispose();
    super.dispose();
  }

  Future<void> _testS3Connection() async {
    final endpoint = _endpointCtrl.text.trim();
    final bucket = _bucketCtrl.text.trim();
    if (endpoint.isEmpty || bucket.isEmpty) {
      setState(() {
        _testTone = JiyueBadgeTone.warn;
        _testMessage = '请先填写端点 URL 与存储桶名称';
      });
      return;
    }

    setState(() {
      _testing = true;
      _testTone = JiyueBadgeTone.neutral;
      _testMessage = '正在探测端点连通性...';
    });

    final stopwatch = Stopwatch()..start();
    final client = HttpClient()..connectionTimeout = const Duration(seconds: 8);
    try {
      final uri = Uri.parse(endpoint);
      final req = await client.getUrl(uri);
      final resp = await req.close();
      stopwatch.stop();

      if (mounted) {
        setState(() {
          _testing = false;
          _testTone = JiyueBadgeTone.ok;
          _testMessage =
              '端点响应正常 (${stopwatch.elapsedMilliseconds}ms, HTTP ${resp.statusCode})';
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _testing = false;
          _testTone = JiyueBadgeTone.danger;
          _testMessage = '连接端点失败: $e';
        });
      }
    } finally {
      client.close();
    }
  }

  Future<void> _saveConfig() async {
    final endpoint = _endpointCtrl.text.trim();
    final bucket = _bucketCtrl.text.trim();
    final accessKey = _accessKeyCtrl.text.trim();
    final secretKey = _secretKeyCtrl.text.trim();

    if (endpoint.isEmpty || bucket.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('存储桶端点与名称不可为空')),
      );
      return;
    }

    final newCfg = S3Config(
      endpoint: endpoint,
      region:
          _regionCtrl.text.trim().isEmpty ? 'us-east-1' : _regionCtrl.text.trim(),
      bucket: bucket,
      accessKeyId: accessKey,
      secretAccessKey: secretKey,
      rootPrefix: _prefixCtrl.text.trim(),
      forcePathStyle: _forcePathStyle,
    );

    await widget.vaultService.saveS3Config(newCfg);
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('S3 云端同步配置已保存')),
      );
      Navigator.of(context).pop();
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.colors;

    return JiyueModalSheet(
      colors: colors,
      icon: 'link',
      title: 'S3 云端同步配置',
      subtitle: '支持 Cloudflare R2、MinIO、RustFS 与标准 S3 对象存储',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          JiyueTextField(
            colors: colors,
            controller: _endpointCtrl,
            label: 'S3 服务端点 (Endpoint URL)',
            hintText: '例如: https://s3.init.cool 或 R2 端点',
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              Expanded(
                flex: 2,
                child: JiyueTextField(
                  colors: colors,
                  controller: _bucketCtrl,
                  label: '存储桶 (Bucket)',
                  hintText: '例如: jiyue',
                ),
              ),
              const SizedBox(width: 10),
              Expanded(
                flex: 1,
                child: JiyueTextField(
                  colors: colors,
                  controller: _regionCtrl,
                  label: '区域 (Region)',
                  hintText: 'us-east-1',
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
          JiyueTextField(
            colors: colors,
            controller: _accessKeyCtrl,
            label: 'Access Key ID',
            hintText: '输入 S3 访问密钥 ID',
          ),
          const SizedBox(height: 12),
          JiyueTextField(
            colors: colors,
            controller: _secretKeyCtrl,
            obscureText: true,
            label: 'Secret Access Key',
            hintText: '输入 S3 私有访问密钥',
          ),
          const SizedBox(height: 12),
          JiyueTextField(
            colors: colors,
            controller: _prefixCtrl,
            label: '路径前缀 (Root Prefix, 可选)',
            hintText: '例如: jiyue-documents/ (留空为桶根目录)',
          ),
          const SizedBox(height: 12),
          JiyueSectionCard(
            colors: colors,
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: JiyueSwitchRow(
              colors: colors,
              title: '路径寻址模式 (Path-Style)',
              subtitle: '适用于 MinIO、RustFS 与私有 S3 服务',
              value: _forcePathStyle,
              onChanged: (v) => setState(() => _forcePathStyle = v),
            ),
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
                  label: _testing ? '探测中...' : '测试连通性',
                  icon: 'bolt',
                  loading: _testing,
                  kind: JiyueButtonKind.secondary,
                  expand: true,
                  onPressed: _testing ? null : _testS3Connection,
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
