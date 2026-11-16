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
    return showModalBottomSheet<void>(
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
  // 0 = S3, 1 = WebDAV
  late int _backendIndex;

  // S3 fields
  late final TextEditingController _endpointCtrl;
  late final TextEditingController _bucketCtrl;
  late final TextEditingController _regionCtrl;
  late final TextEditingController _accessKeyCtrl;
  late final TextEditingController _secretKeyCtrl;
  late final TextEditingController _prefixCtrl;
  late bool _forcePathStyle;

  // WebDAV fields
  late final TextEditingController _davEndpointCtrl;
  late final TextEditingController _davUserCtrl;
  late final TextEditingController _davPassCtrl;
  late final TextEditingController _davRootCtrl;

  bool _testing = false;
  JiyueBadgeTone? _testTone;
  String? _testMessage;

  @override
  void initState() {
    super.initState();
    final s3 = widget.vaultService.s3Config;
    final dav = widget.vaultService.webdavConfig;
    _backendIndex = (s3 == null && dav != null) ? 1 : 0;

    _endpointCtrl = TextEditingController(text: s3?.endpoint ?? '');
    _bucketCtrl = TextEditingController(text: s3?.bucket ?? '');
    _regionCtrl = TextEditingController(text: s3?.region ?? 'us-east-1');
    _accessKeyCtrl = TextEditingController(text: s3?.accessKeyId ?? '');
    _secretKeyCtrl = TextEditingController(text: s3?.secretAccessKey ?? '');
    _prefixCtrl = TextEditingController(
      text: s3?.rootPrefix ?? 'jiyue/${widget.vaultService.vaultName}/',
    );
    _forcePathStyle = s3?.forcePathStyle ?? true;

    _davEndpointCtrl = TextEditingController(text: dav?.endpoint ?? '');
    _davUserCtrl = TextEditingController(text: dav?.username ?? '');
    _davPassCtrl = TextEditingController(text: dav?.password ?? '');
    _davRootCtrl = TextEditingController(
      text: dav?.root ?? 'jiyue/${widget.vaultService.vaultName}/',
    );
  }

  @override
  void dispose() {
    _endpointCtrl.dispose();
    _bucketCtrl.dispose();
    _regionCtrl.dispose();
    _accessKeyCtrl.dispose();
    _secretKeyCtrl.dispose();
    _prefixCtrl.dispose();
    _davEndpointCtrl.dispose();
    _davUserCtrl.dispose();
    _davPassCtrl.dispose();
    _davRootCtrl.dispose();
    super.dispose();
  }

  S3Config? _buildS3Config() {
    final endpoint = _endpointCtrl.text.trim();
    final bucket = _bucketCtrl.text.trim();
    if (endpoint.isEmpty || bucket.isEmpty) return null;
    return S3Config(
      endpoint: endpoint,
      region: _regionCtrl.text.trim().isEmpty
          ? 'us-east-1'
          : _regionCtrl.text.trim(),
      bucket: bucket,
      accessKeyId: _accessKeyCtrl.text.trim(),
      secretAccessKey: _secretKeyCtrl.text.trim(),
      rootPrefix: _prefixCtrl.text.trim(),
      forcePathStyle: _forcePathStyle,
    );
  }

  WebDavConfig? _buildWebDavConfig() {
    final endpoint = _davEndpointCtrl.text.trim();
    if (endpoint.isEmpty) return null;
    return WebDavConfig(
      endpoint: endpoint,
      username: _davUserCtrl.text.trim(),
      password: _davPassCtrl.text,
      root: _davRootCtrl.text.trim(),
    );
  }

  Future<void> _testConnection() async {
    final Map<String, dynamic>? storeJson;
    if (_backendIndex == 0) {
      storeJson = _buildS3Config()?.toJson();
      if (storeJson == null) {
        setState(() {
          _testTone = JiyueBadgeTone.warn;
          _testMessage = '请先填写 S3 端点 URL 与存储桶名称';
        });
        return;
      }
    } else {
      storeJson = _buildWebDavConfig()?.toJson();
      if (storeJson == null) {
        setState(() {
          _testTone = JiyueBadgeTone.warn;
          _testMessage = '请先填写 WebDAV 服务端点 URL';
        });
        return;
      }
    }

    setState(() {
      _testing = true;
      _testTone = JiyueBadgeTone.neutral;
      _testMessage = '正在通过 Rust 引擎校验可达性、列举与写入权限...';
    });

    final result = await widget.vaultService.testSyncConnection(storeJson);
    if (!mounted) return;
    setState(() {
      _testing = false;
      _testTone = result.success ? JiyueBadgeTone.ok : JiyueBadgeTone.danger;
      _testMessage = result.message.isNotEmpty
          ? result.message
          : (result.success ? '连接校验通过（可读可写）' : '连接校验失败');
    });
  }

  Future<void> _saveConfig() async {
    if (_backendIndex == 0) {
      final cfg = _buildS3Config();
      if (cfg == null) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('S3 存储桶端点与名称不可为空')),
        );
        return;
      }
      await widget.vaultService.saveS3Config(cfg);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('已为当前笔记库保存 S3 同步配置')),
        );
        Navigator.of(context).pop();
      }
    } else {
      final cfg = _buildWebDavConfig();
      if (cfg == null) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('WebDAV 服务端点不可为空')),
        );
        return;
      }
      await widget.vaultService.saveWebDavConfig(cfg);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('已为当前笔记库保存 WebDAV 同步配置')),
        );
        Navigator.of(context).pop();
      }
    }
  }

  Future<void> _unbindSync() async {
    await widget.vaultService.clearSyncConfig();
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('已解绑远端同步，笔记库已转为纯本地模式')),
      );
      Navigator.of(context).pop();
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.colors;
    final hasRemoteSync = widget.vaultService.activeVault?.hasRemoteSync ?? false;

    return JiyueModalSheet(
      colors: colors,
      icon: 'link',
      title: '笔记库同步设置',
      subtitle: '当前库：${widget.vaultService.vaultName}（多库按前缀独立隔离）',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          JiyueSegmented(
            colors: colors,
            fullWidth: true,
            selectedIndex: _backendIndex,
            items: const [
              ('S3 对象存储', 'link'),
              ('WebDAV', 'folder'),
            ],
            onChanged: (idx) {
              setState(() {
                _backendIndex = idx;
                _testTone = null;
                _testMessage = null;
              });
            },
          ),
          const SizedBox(height: 14),

          if (_backendIndex == 0) ...[
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
              label: '路径前缀 (Root Prefix, 用于多库隔离)',
              hintText: '例如: jiyue/MyObsidian/ (需与 PC 端一致)',
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
          ] else ...[
            JiyueTextField(
              colors: colors,
              controller: _davEndpointCtrl,
              label: 'WebDAV 服务端点 (Endpoint URL)',
              hintText: '例如: https://dav.jianguoyun.com/dav/',
            ),
            const SizedBox(height: 12),
            JiyueTextField(
              colors: colors,
              controller: _davUserCtrl,
              label: '用户名',
              hintText: '输入 WebDAV 登录账号',
            ),
            const SizedBox(height: 12),
            JiyueTextField(
              colors: colors,
              controller: _davPassCtrl,
              obscureText: true,
              label: '应用密码 / Token',
              hintText: '输入 WebDAV 授权密码',
            ),
            const SizedBox(height: 12),
            JiyueTextField(
              colors: colors,
              controller: _davRootCtrl,
              label: '远端目录路径 (Root)',
              hintText: '例如: jiyue/MyObsidian/',
            ),
          ],

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
                  label: _testing ? '校验中...' : '测试连通性',
                  icon: 'bolt',
                  loading: _testing,
                  kind: JiyueButtonKind.secondary,
                  expand: true,
                  onPressed: _testing ? null : _testConnection,
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
          if (hasRemoteSync) ...[
            const SizedBox(height: 10),
            JiyueButton(
              colors: colors,
              label: '解除云端同步绑定（保留本地笔记）',
              icon: 'trash',
              kind: JiyueButtonKind.danger,
              size: JiyueControlSize.sm,
              expand: true,
              onPressed: _unbindSync,
            ),
          ],
        ],
      ),
    );
  }
}
