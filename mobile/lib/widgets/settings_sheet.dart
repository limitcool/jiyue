import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';
import '../bridge/vault_service.dart';
import '../models/exported_config.dart';
import '../services/update_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import '../views/qr_scan_page.dart';
import 'ai_config_sheet.dart';
import 'jiyue_icon.dart';
import 'memos_config_sheet.dart';
import 's3_config_sheet.dart';
import 'update_dialog.dart';

class SettingsModalSheet extends StatefulWidget {
  final ThemeProvider themeProvider;
  final VaultService vaultService;
  final int noteCount;

  const SettingsModalSheet({
    super.key,
    required this.themeProvider,
    required this.vaultService,
    required this.noteCount,
  });

  @override
  State<SettingsModalSheet> createState() => _SettingsModalSheetState();
}

class _SettingsModalSheetState extends State<SettingsModalSheet> {
  bool _s3Enabled = true;
  bool _checkingUpdate = false;

  Future<void> _checkUpdate() async {
    setState(() => _checkingUpdate = true);
    try {
      final res = await UpdateService.checkUpdate();
      if (mounted) {
        setState(() => _checkingUpdate = false);
        UpdateDialog.show(context, result: res, themeProvider: widget.themeProvider);
      }
    } catch (e) {
      if (mounted) {
        setState(() => _checkingUpdate = false);
        final msg = e.toString().replaceFirst("Exception: ", "");
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('检查更新: $msg'),
            action: SnackBarAction(
              label: '网页查看',
              onPressed: () {
                launchUrl(
                  Uri.parse('https://github.com/$defaultRepoOwner/$defaultRepoName/releases'),
                  mode: LaunchMode.externalApplication,
                );
              },
            ),
          ),
        );
      }
    }
  }

  Future<void> _handleScanQr() async {
    final result = await Navigator.of(context).push<String>(
      MaterialPageRoute(
        builder: (_) => QrScanPage(colors: widget.themeProvider.colors),
      ),
    );
    if (result != null && mounted) {
      _processConfigPayload(result);
    }
  }

  Future<void> _handlePasteJson() async {
    final clip = await Clipboard.getData(Clipboard.kTextPlain);
    final text = clip?.text?.trim();
    if (text == null || text.isEmpty) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('剪贴板中没有文本内容')),
        );
      }
      return;
    }
    _processConfigPayload(text);
  }

  void _processConfigPayload(String raw) {
    try {
      final json = jsonDecode(raw) as Map<String, dynamic>;
      if (json['jiyue'] != 'config_export') {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('未识别到有效的霁月配置代码')),
        );
        return;
      }
      final config = ExportedConfig.fromJson(json);
      _showImportPreview(config);
    } catch (e) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('配置代码解析失败: $e')),
      );
    }
  }

  void _showImportPreview(ExportedConfig config) {
    final colors = widget.themeProvider.colors;
    showModalBottomSheet(
      context: context,
      backgroundColor: Colors.transparent,
      isScrollControlled: true,
      builder: (ctx) {
        return Container(
          decoration: BoxDecoration(
            color: colors.sheet,
            borderRadius: const BorderRadius.vertical(top: Radius.circular(24)),
          ),
          padding: const EdgeInsets.fromLTRB(20, 16, 20, 24),
          child: SafeArea(
            top: false,
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
                  children: [
                    JiyueIcon('check', color: colors.accent, size: 20),
                    const SizedBox(width: 8),
                    Text(
                      '确认导入 PC 端配置',
                      style: TextStyle(
                        fontSize: 16,
                        fontWeight: FontWeight.bold,
                        color: colors.textPrimary,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 12),
                if (config.s3 != null) ...[
                  Text('☁️ S3 云端存储配置', style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary)),
                  const SizedBox(height: 6),
                  Container(
                    width: double.infinity,
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      color: colors.raised,
                      borderRadius: BorderRadius.circular(10),
                      border: Border.all(color: colors.border),
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text('存储桶: ${config.s3!.bucket}', style: TextStyle(fontSize: 12, color: colors.textPrimary)),
                        const SizedBox(height: 3),
                        Text('端点: ${config.s3!.endpoint}', style: TextStyle(fontSize: 11, color: colors.textSecondary)),
                        if (config.s3!.rootPrefix.isNotEmpty) ...[
                          const SizedBox(height: 3),
                          Text('路径前缀: ${config.s3!.rootPrefix}', style: TextStyle(fontSize: 11, color: colors.textMuted)),
                        ],
                      ],
                    ),
                  ),
                  const SizedBox(height: 12),
                ],
                if (config.ai != null) ...[
                  Text('🤖 AI 助手配置 (${config.ai!.providers.length} 个服务商)', style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary)),
                  const SizedBox(height: 6),
                  Container(
                    width: double.infinity,
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      color: colors.raised,
                      borderRadius: BorderRadius.circular(10),
                      border: Border.all(color: colors.border),
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: config.ai!.providers.map((p) {
                        return Padding(
                          padding: const EdgeInsets.only(bottom: 4),
                          child: Text('• ${p.name} (${p.chatModel})', style: TextStyle(fontSize: 12, color: colors.textPrimary)),
                        );
                      }).toList(),
                    ),
                  ),
                  const SizedBox(height: 12),
                ],
                if (config.memos != null) ...[
                  Text('📝 Memos 备忘录配置', style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary)),
                  const SizedBox(height: 6),
                  Container(
                    width: double.infinity,
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      color: colors.raised,
                      borderRadius: BorderRadius.circular(10),
                      border: Border.all(color: colors.border),
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text('服务端点: ${config.memos!.endpoint}', style: TextStyle(fontSize: 12, color: colors.textPrimary)),
                        if (config.memos!.token.isNotEmpty) ...[
                          const SizedBox(height: 3),
                          Text('访问密钥: •••••••••••• (已包含)', style: TextStyle(fontSize: 11, color: colors.textMuted)),
                        ],
                      ],
                    ),
                  ),
                  const SizedBox(height: 16),
                ],
                Row(
                  children: [
                    Expanded(
                      child: OutlinedButton(
                        style: OutlinedButton.styleFrom(
                          foregroundColor: colors.textSecondary,
                          side: BorderSide(color: colors.border),
                          padding: const EdgeInsets.symmetric(vertical: 12),
                          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
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
                          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
                        ),
                        onPressed: () async {
                          if (config.s3 != null) {
                            await widget.vaultService.saveS3Config(config.s3!);
                          }
                          if (config.ai != null) {
                            await widget.vaultService.saveAiConfig(config.ai!);
                          }
                          if (config.memos != null) {
                            await widget.vaultService.saveMemosConfig(config.memos!);
                          }
                          if (ctx.mounted) {
                            Navigator.of(ctx).pop();
                          }
                          if (mounted) {
                            setState(() {});
                            ScaffoldMessenger.of(context).showSnackBar(
                              const SnackBar(
                                content: Text('🎉 已成功导入 PC 端配置 (S3/AI/Memos)，已保存生效！'),
                                backgroundColor: Color(0xFF2E7D32),
                              ),
                            );
                          }
                        },
                        child: const Text('确认导入并应用', style: TextStyle(fontWeight: FontWeight.bold)),
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
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;

    return Container(
      decoration: BoxDecoration(
        color: colors.sheet,
        borderRadius: const BorderRadius.vertical(top: Radius.circular(24)),
      ),
      child: SafeArea(
        top: false,
        child: SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(20, 12, 20, 24),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
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

              // 标题栏与关闭按钮
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        '外观与笔记库设置',
                        style: TextStyle(
                          fontSize: 18,
                          fontWeight: FontWeight.bold,
                          color: colors.textPrimary,
                        ),
                      ),
                      const SizedBox(height: 3),
                      Text(
                        '移动端与 PC 端无缝同步外观偏好与存储配置',
                        style: TextStyle(fontSize: 11, color: colors.textMuted),
                      ),
                    ],
                  ),
                  IconButton(
                    icon: JiyueIcon('x', color: colors.textSecondary, size: 20),
                    onPressed: () => Navigator.of(context).pop(),
                  ),
                ],
              ),
              const SizedBox(height: 18),

              // 一、主题外观切换
              Text(
                '主题外观 (Theme)',
                style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
              ),
              const SizedBox(height: 10),
              ...JiyueThemeColors.allThemes.map((theme) {
                final isSelected = theme.id == widget.themeProvider.themeId;
                return Container(
                  margin: const EdgeInsets.only(bottom: 8),
                  decoration: BoxDecoration(
                    color: theme.chrome,
                    borderRadius: BorderRadius.circular(14),
                    border: Border.all(
                      color: isSelected ? theme.accent : colors.border,
                      width: isSelected ? 2 : 1,
                    ),
                  ),
                  child: ListTile(
                    contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 4),
                    leading: Container(
                      width: 36,
                      height: 36,
                      decoration: BoxDecoration(
                        color: theme.sheet,
                        borderRadius: BorderRadius.circular(8),
                        border: Border.all(color: theme.accent.withAlpha(80)),
                      ),
                      child: Center(
                        child: JiyueIcon(
                          theme.isDark ? 'bulb' : 'layout-columns',
                          color: theme.accent,
                          size: 18,
                        ),
                      ),
                    ),
                    title: Text(
                      theme.name,
                      style: TextStyle(
                        fontSize: 14,
                        fontWeight: FontWeight.bold,
                        color: theme.textPrimary,
                      ),
                    ),
                    subtitle: Text(
                      theme.description,
                      style: TextStyle(fontSize: 11, color: theme.textSecondary),
                    ),
                    trailing: isSelected
                        ? JiyueIcon('check', color: theme.accent, size: 20)
                        : null,
                    onTap: () {
                      widget.themeProvider.setTheme(theme.id);
                      setState(() {});
                    },
                  ),
                );
              }),

              const SizedBox(height: 18),

              // 二、笔记库管理 (Vaults) - 严格对齐 PC 端截图
              Text(
                '当前笔记库管理 (Vaults)',
                style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
              ),
              const SizedBox(height: 10),
              Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(14),
                  border: Border.all(color: colors.border),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        JiyueIcon('folder', color: colors.accent, size: 20),
                        const SizedBox(width: 8),
                        Text(
                          widget.vaultService.vaultName,
                          style: TextStyle(
                            fontSize: 15,
                            fontWeight: FontWeight.bold,
                            color: colors.textPrimary,
                          ),
                        ),
                        const SizedBox(width: 8),
                        Container(
                          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                          decoration: BoxDecoration(
                            color: colors.accent.withAlpha(40),
                            borderRadius: BorderRadius.circular(6),
                          ),
                          child: Text(
                            '当前',
                            style: TextStyle(
                              fontSize: 10,
                              fontWeight: FontWeight.bold,
                              color: colors.accent,
                            ),
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 8),
                    Text(
                      '${widget.noteCount} 篇 Markdown 笔记',
                      style: TextStyle(fontSize: 11, color: colors.textSecondary),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      widget.vaultService.vaultPath,
                      style: TextStyle(fontSize: 10, color: colors.textMuted),
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 12),

              // 操作按钮行：[复制路径] [打开其他笔记库] [新建笔记库] (与 PC 端截图完全一致)
              Row(
                children: [
                  Expanded(
                    child: OutlinedButton.icon(
                      style: OutlinedButton.styleFrom(
                        foregroundColor: colors.textPrimary,
                        side: BorderSide(color: colors.border),
                        padding: const EdgeInsets.symmetric(vertical: 10),
                        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                      icon: JiyueIcon('copy', color: colors.textSecondary, size: 14),
                      label: const Text('复制路径', style: TextStyle(fontSize: 11)),
                      onPressed: () {
                        Clipboard.setData(ClipboardData(text: widget.vaultService.vaultPath));
                        ScaffoldMessenger.of(context).showSnackBar(
                          const SnackBar(content: Text('笔记库路径已复制到剪贴板')),
                        );
                      },
                    ),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: OutlinedButton.icon(
                      style: OutlinedButton.styleFrom(
                        foregroundColor: colors.textPrimary,
                        side: BorderSide(color: colors.border),
                        padding: const EdgeInsets.symmetric(vertical: 10),
                        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                      icon: JiyueIcon('folder-open', color: colors.textSecondary, size: 14),
                      label: const Text('其他库...', style: TextStyle(fontSize: 11)),
                      onPressed: () {
                        ScaffoldMessenger.of(context).showSnackBar(
                          const SnackBar(content: Text('已准备支持多库切换')),
                        );
                      },
                    ),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: OutlinedButton.icon(
                      style: OutlinedButton.styleFrom(
                        foregroundColor: colors.textPrimary,
                        side: BorderSide(color: colors.border),
                        padding: const EdgeInsets.symmetric(vertical: 10),
                        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                      icon: JiyueIcon('folder-plus', color: colors.textSecondary, size: 14),
                      label: const Text('新建库...', style: TextStyle(fontSize: 11)),
                      onPressed: () {
                        ScaffoldMessenger.of(context).showSnackBar(
                          const SnackBar(content: Text('新建笔记库功能就绪')),
                        );
                      },
                    ),
                  ),
                ],
              ),

              const SizedBox(height: 18),

              // 三、跨端快捷配置导入 (从 PC 扫码或粘贴)
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Text(
                    '跨端配置导入 (PC 导入)',
                    style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
                  ),
                  Container(
                    padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                    decoration: BoxDecoration(
                      color: colors.accent.withAlpha(35),
                      borderRadius: BorderRadius.circular(6),
                    ),
                    child: Text(
                      '免手动输入',
                      style: TextStyle(fontSize: 10, fontWeight: FontWeight.bold, color: colors.accent),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 10),
              Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(14),
                  border: Border.all(color: colors.border),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '扫描 PC 端设置弹窗中「📱 手机扫码同步」展示的二维码，一键导入 S3 存储桶凭据与 AI 助手配置。',
                      style: TextStyle(fontSize: 11, color: colors.textSecondary, height: 1.4),
                    ),
                    const SizedBox(height: 12),
                    Row(
                      children: [
                        Expanded(
                          child: ElevatedButton.icon(
                            style: ElevatedButton.styleFrom(
                              backgroundColor: colors.accent,
                              foregroundColor: colors.onAccent,
                              elevation: 0,
                              padding: const EdgeInsets.symmetric(vertical: 10),
                              shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                            ),
                            icon: const Icon(Icons.qr_code_scanner, size: 16),
                            label: const Text('扫码导入', style: TextStyle(fontSize: 12, fontWeight: FontWeight.bold)),
                            onPressed: _handleScanQr,
                          ),
                        ),
                        const SizedBox(width: 8),
                        Expanded(
                          child: OutlinedButton.icon(
                            style: OutlinedButton.styleFrom(
                              foregroundColor: colors.textPrimary,
                              side: BorderSide(color: colors.border),
                              padding: const EdgeInsets.symmetric(vertical: 10),
                              shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                            ),
                            icon: JiyueIcon('copy', color: colors.textSecondary, size: 14),
                            label: const Text('粘贴代码', style: TextStyle(fontSize: 12)),
                            onPressed: _handlePasteJson,
                          ),
                        ),
                      ],
                    ),
                    if (widget.vaultService.s3Config != null ||
                        widget.vaultService.aiConfig != null ||
                        widget.vaultService.memosConfig != null) ...[
                      const Divider(height: 20),
                      Row(
                        children: [
                          Icon(Icons.check_circle_outline, color: colors.accent, size: 14),
                          const SizedBox(width: 6),
                          Expanded(
                            child: Text(
                              '已同步: ${widget.vaultService.s3Config != null ? "S3[${widget.vaultService.s3Config!.bucket}] " : ""}${widget.vaultService.aiConfig != null ? "AI[${widget.vaultService.aiConfig!.providers.length}服务商] " : ""}${widget.vaultService.memosConfig != null ? "Memos[已绑定]" : ""}',
                              style: TextStyle(fontSize: 11, color: colors.accent),
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                        ],
                      ),
                    ],
                  ],
                ),
              ),

              const SizedBox(height: 18),

              // 四、AI 助手与服务商管理 (AI Assistant)
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Text(
                    'AI 助手配置 (AI Assistant)',
                    style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
                  ),
                  TextButton.icon(
                    onPressed: () async {
                      await AiConfigSheet.show(
                        context,
                        vaultService: widget.vaultService,
                        colors: colors,
                      );
                      setState(() {});
                    },
                    icon: JiyueIcon('settings', color: colors.accent, size: 12),
                    label: Text('管理/测试服务商', style: TextStyle(fontSize: 11, color: colors.accent)),
                  ),
                ],
              ),
              const SizedBox(height: 8),
              Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(14),
                  border: Border.all(color: colors.border),
                ),
                child: Column(
                  children: [
                    Row(
                      children: [
                        Container(
                          padding: const EdgeInsets.all(6),
                          decoration: BoxDecoration(
                            color: colors.accent.withAlpha(25),
                            borderRadius: BorderRadius.circular(8),
                          ),
                          child: JiyueIcon('sparkles', color: colors.accent, size: 18),
                        ),
                        const SizedBox(width: 10),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                widget.vaultService.aiConfig != null &&
                                        widget.vaultService.aiConfig!.providers.isNotEmpty
                                    ? '默认服务: ${widget.vaultService.aiConfig!.providers.firstWhere((p) => p.id == widget.vaultService.aiConfig!.activeChat, orElse: () => widget.vaultService.aiConfig!.providers.first).name}'
                                    : '未配置 AI 服务商',
                                style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
                              ),
                              const SizedBox(height: 2),
                              Text(
                                widget.vaultService.aiConfig != null &&
                                        widget.vaultService.aiConfig!.providers.isNotEmpty
                                    ? '模型: ${widget.vaultService.aiConfig!.providers.firstWhere((p) => p.id == widget.vaultService.aiConfig!.activeChat, orElse: () => widget.vaultService.aiConfig!.providers.first).chatModel} · 支持指定模型测试'
                                    : '点击右上角管理以添加 DeepSeek/OpenAI/Anthropic 服务',
                                style: TextStyle(fontSize: 11, color: colors.textMuted),
                              ),
                            ],
                          ),
                        ),
                        OutlinedButton(
                          style: OutlinedButton.styleFrom(
                            foregroundColor: colors.accent,
                            side: BorderSide(color: colors.border),
                            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                            shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
                          ),
                          onPressed: () async {
                            await AiConfigSheet.show(
                              context,
                              vaultService: widget.vaultService,
                              colors: colors,
                            );
                            setState(() {});
                          },
                          child: const Text('管理/测试', style: TextStyle(fontSize: 11)),
                        ),
                      ],
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 18),

              // 五、S3 / WebDAV 云同步设置 (Sync)
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Text(
                    '云端同步配置 (Sync)',
                    style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
                  ),
                  TextButton.icon(
                    onPressed: () async {
                      await S3ConfigSheet.show(
                        context,
                        vaultService: widget.vaultService,
                        colors: colors,
                      );
                      setState(() {});
                    },
                    icon: JiyueIcon('settings', color: colors.accent, size: 12),
                    label: Text('修改/测试配置', style: TextStyle(fontSize: 11, color: colors.accent)),
                  ),
                ],
              ),
              const SizedBox(height: 8),
              Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(14),
                  border: Border.all(color: colors.border),
                ),
                child: Column(
                  children: [
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Row(
                          children: [
                            JiyueIcon('link', color: colors.accent, size: 18),
                            const SizedBox(width: 8),
                            Text(
                              '启用 S3 / WebDAV 自动同步',
                              style: TextStyle(fontSize: 13, color: colors.textPrimary),
                            ),
                          ],
                        ),
                        Switch(
                          value: _s3Enabled,
                          activeColor: colors.accent,
                          onChanged: (v) => setState(() => _s3Enabled = v),
                        ),
                      ],
                    ),
                    const Divider(),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Expanded(
                          child: Text(
                            widget.vaultService.s3Config != null
                                ? '存储桶: ${widget.vaultService.s3Config!.bucket} (${widget.vaultService.s3Config!.endpoint})'
                                : '协议类型: S3 (Cloudflare R2 / RustFS / MinIO)',
                            style: TextStyle(fontSize: 11, color: colors.textSecondary),
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                        TextButton(
                          onPressed: () async {
                            await S3ConfigSheet.show(
                              context,
                              vaultService: widget.vaultService,
                              colors: colors,
                            );
                            setState(() {});
                          },
                          child: Text(
                            widget.vaultService.s3Config != null ? '已配置 (点击编辑)' : '手动配置',
                            style: TextStyle(fontSize: 11, color: colors.accent),
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 18),

              // 六、Memos 备忘录服务
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Text(
                    'Memos 备忘录服务 (Memos)',
                    style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
                  ),
                  TextButton.icon(
                    onPressed: () async {
                      await MemosConfigSheet.show(
                        context,
                        vaultService: widget.vaultService,
                        colors: colors,
                      );
                      setState(() {});
                    },
                    icon: JiyueIcon('settings', color: colors.accent, size: 12),
                    label: Text('配置 Memos', style: TextStyle(fontSize: 11, color: colors.accent)),
                  ),
                ],
              ),
              const SizedBox(height: 8),
              Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(14),
                  border: Border.all(color: colors.border),
                ),
                child: Row(
                  children: [
                    Container(
                      padding: const EdgeInsets.all(6),
                      decoration: BoxDecoration(
                        color: colors.accent.withAlpha(25),
                        borderRadius: BorderRadius.circular(8),
                      ),
                      child: JiyueIcon('calendar', color: colors.accent, size: 18),
                    ),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            widget.vaultService.memosConfig != null
                                ? '服务端点: ${widget.vaultService.memosConfig!.endpoint}'
                                : '未配置 Memos 服务',
                            style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
                            overflow: TextOverflow.ellipsis,
                          ),
                          const SizedBox(height: 2),
                          Text(
                            widget.vaultService.memosConfig != null
                                ? 'Token: •••••••••••• (点击可测试连接与修改)'
                                : '点击右侧配置以绑定私有或自建 Memos 服务',
                            style: TextStyle(fontSize: 11, color: colors.textMuted),
                          ),
                        ],
                      ),
                    ),
                    OutlinedButton(
                      style: OutlinedButton.styleFrom(
                        foregroundColor: colors.accent,
                        side: BorderSide(color: colors.border),
                        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
                      ),
                      onPressed: () async {
                        await MemosConfigSheet.show(
                          context,
                          vaultService: widget.vaultService,
                          colors: colors,
                        );
                        setState(() {});
                      },
                      child: Text(
                        widget.vaultService.memosConfig != null ? '管理/测试' : '配置',
                        style: const TextStyle(fontSize: 11),
                      ),
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 18),

              // 七、软件版本与自更新 (Updates & About)
              Text(
                '软件版本与自更新 (Updates & About)',
                style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
              ),
              const SizedBox(height: 10),
              Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(14),
                  border: Border.all(color: colors.border),
                ),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    Row(
                      children: [
                        Container(
                          padding: const EdgeInsets.all(6),
                          decoration: BoxDecoration(
                            color: colors.accent.withAlpha(25),
                            borderRadius: BorderRadius.circular(8),
                          ),
                          child: JiyueIcon('download-cloud', color: colors.accent, size: 18),
                        ),
                        const SizedBox(width: 10),
                        Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              '霁月 (Jiyue Mobile)',
                              style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
                            ),
                            const SizedBox(height: 2),
                            Text(
                              '当前版本: v$currentAppVersion',
                              style: TextStyle(fontSize: 11, color: colors.textMuted),
                            ),
                          ],
                        ),
                      ],
                    ),
                    ElevatedButton.icon(
                      style: ElevatedButton.styleFrom(
                        backgroundColor: colors.accent,
                        foregroundColor: colors.onAccent,
                        elevation: 0,
                        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
                        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
                      ),
                      icon: _checkingUpdate
                          ? SizedBox(
                              width: 12,
                              height: 12,
                              child: CircularProgressIndicator(strokeWidth: 2, color: colors.onAccent),
                            )
                          : JiyueIcon('refresh-cw', color: colors.onAccent, size: 14),
                      label: Text(_checkingUpdate ? '检查中...' : '检查更新', style: const TextStyle(fontSize: 12, fontWeight: FontWeight.bold)),
                      onPressed: _checkingUpdate ? null : _checkUpdate,
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 20),

              // 保存并关闭按钮
              SizedBox(
                width: double.infinity,
                height: 46,
                child: ElevatedButton.icon(
                  style: ElevatedButton.styleFrom(
                    backgroundColor: colors.accent,
                    foregroundColor: colors.onAccent,
                    shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(23)),
                  ),
                  icon: JiyueIcon('check', color: colors.onAccent, size: 18),
                  label: const Text('保存并应用当前设置', style: TextStyle(fontSize: 14, fontWeight: FontWeight.bold)),
                  onPressed: () => Navigator.of(context).pop(),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
