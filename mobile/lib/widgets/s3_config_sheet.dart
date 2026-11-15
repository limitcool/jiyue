import 'dart:io';
import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../models/sync_config.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_icon.dart';

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
  String? _testResult;

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
      setState(() => _testResult = '✕ 请先填写端点 URL 与存储桶名称');
      return;
    }

    setState(() {
      _testing = true;
      _testResult = '正在探测端点连通性...';
    });

    final stopwatch = Stopwatch()..start();
    final client = HttpClient()..connectionTimeout = const Duration(seconds: 8);
    try {
      final uri = Uri.parse(endpoint);
      final req = await client.getUrl(uri);
      final resp = await req.close();
      stopwatch.stop();

      setState(() {
        _testing = false;
        _testResult = '✓ 端点响应正常 (${stopwatch.elapsedMilliseconds}ms, HTTP ${resp.statusCode})';
      });
    } catch (e) {
      setState(() {
        _testing = false;
        _testResult = '✕ 连接端点失败: $e';
      });
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
      region: _regionCtrl.text.trim().isEmpty ? 'us-east-1' : _regionCtrl.text.trim(),
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
                      JiyueIcon('link', color: colors.accent, size: 20),
                      const SizedBox(width: 8),
                      Text(
                        'S3 云端同步配置',
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
                  labelText: 'S3 服务端点 (Endpoint URL)',
                  hintText: '例如: https://s3.init.cool 或 https://<id>.r2.cloudflarestorage.com',
                  labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                  hintStyle: TextStyle(color: colors.textMuted, fontSize: 12),
                  isDense: true,
                  border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                ),
              ),
              const SizedBox(height: 12),

              // 存储桶与区域
              Row(
                children: [
                  Expanded(
                    flex: 2,
                    child: TextField(
                      controller: _bucketCtrl,
                      style: TextStyle(color: colors.textPrimary, fontSize: 13),
                      decoration: InputDecoration(
                        labelText: '存储桶 (Bucket)',
                        hintText: '例如: jiyue',
                        labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                        isDense: true,
                        border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                    ),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    flex: 1,
                    child: TextField(
                      controller: _regionCtrl,
                      style: TextStyle(color: colors.textPrimary, fontSize: 13),
                      decoration: InputDecoration(
                        labelText: '区域 (Region)',
                        hintText: 'us-east-1',
                        labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                        isDense: true,
                        border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),

              // Access Key ID
              TextField(
                controller: _accessKeyCtrl,
                style: TextStyle(color: colors.textPrimary, fontSize: 13),
                decoration: InputDecoration(
                  labelText: 'Access Key ID',
                  hintText: 'S3 访问密钥 ID',
                  labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                  isDense: true,
                  border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                ),
              ),
              const SizedBox(height: 12),

              // Secret Access Key
              TextField(
                controller: _secretKeyCtrl,
                obscureText: true,
                style: TextStyle(color: colors.textPrimary, fontSize: 13),
                decoration: InputDecoration(
                  labelText: 'Secret Access Key',
                  hintText: 'S3 私有访问密钥',
                  labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                  isDense: true,
                  border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                ),
              ),
              const SizedBox(height: 12),

              // 路径前缀
              TextField(
                controller: _prefixCtrl,
                style: TextStyle(color: colors.textPrimary, fontSize: 13),
                decoration: InputDecoration(
                  labelText: '路径前缀 (Root Prefix, 可选)',
                  hintText: '例如: jiyue-documents/ (留空为桶根目录)',
                  labelStyle: TextStyle(color: colors.textSecondary, fontSize: 12),
                  isDense: true,
                  border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                ),
              ),
              const SizedBox(height: 12),

              // Force Path Style 开关 (MinIO / RustFS 特别需要)
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(10),
                  border: Border.all(color: colors.border),
                ),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          '路径寻址模式 (Path-Style)',
                          style: TextStyle(fontSize: 12, fontWeight: FontWeight.bold, color: colors.textPrimary),
                        ),
                        Text(
                          '适用于 MinIO、RustFS 与私有 S3 服务',
                          style: TextStyle(fontSize: 10, color: colors.textSecondary),
                        ),
                      ],
                    ),
                    Switch(
                      value: _forcePathStyle,
                      activeColor: colors.accent,
                      onChanged: (v) => setState(() => _forcePathStyle = v),
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 16),
              // 测试端点按钮与反馈
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Expanded(
                    child: Text(
                      _testResult ?? '建议保存前测试端点网络连通性',
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
                    label: const Text('测试连通性', style: TextStyle(fontSize: 11)),
                    onPressed: _testing ? null : _testS3Connection,
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
                  label: const Text('保存 S3 配置', style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold)),
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
