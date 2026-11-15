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
import 'jiyue_controls.dart';
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
        UpdateDialog.show(
          context,
          result: res,
          themeProvider: widget.themeProvider,
        );
      }
    } catch (e) {
      if (mounted) {
        setState(() => _checkingUpdate = false);
        final msg = e.toString().replaceFirst('Exception: ', '');
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('检查更新: $msg'),
            action: SnackBarAction(
              label: '网页查看',
              onPressed: () {
                launchUrl(
                  Uri.parse(
                    'https://github.com/$defaultRepoOwner/$defaultRepoName/releases',
                  ),
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
        return JiyueModalSheet(
          colors: colors,
          icon: 'check',
          title: '确认导入 PC 端配置',
          subtitle: '请核对即将覆盖生效的同步与服务配置',
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (config.s3 != null) ...[
                JiyueSectionHeader(
                  colors: colors,
                  title: 'S3 云端存储配置',
                  padding: const EdgeInsets.only(bottom: 6),
                ),
                JiyueSectionCard(
                  colors: colors,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        '存储桶: ${config.s3!.bucket}',
                        style: TextStyle(
                          fontSize: JiyueTokens.fontSm,
                          fontWeight: FontWeight.w600,
                          color: colors.textPrimary,
                        ),
                      ),
                      const SizedBox(height: 3),
                      Text(
                        '端点: ${config.s3!.endpoint}',
                        style: TextStyle(
                          fontSize: JiyueTokens.fontXs,
                          color: colors.textSecondary,
                        ),
                      ),
                      if (config.s3!.rootPrefix.isNotEmpty) ...[
                        const SizedBox(height: 3),
                        Text(
                          '路径前缀: ${config.s3!.rootPrefix}',
                          style: TextStyle(
                            fontSize: JiyueTokens.fontXs,
                            color: colors.textMuted,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                const SizedBox(height: 12),
              ],
              if (config.ai != null) ...[
                JiyueSectionHeader(
                  colors: colors,
                  title: 'AI 助手配置 (${config.ai!.providers.length} 个服务商)',
                  padding: const EdgeInsets.only(bottom: 6),
                ),
                JiyueSectionCard(
                  colors: colors,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: config.ai!.providers.map((p) {
                      return Padding(
                        padding: const EdgeInsets.only(bottom: 4),
                        child: Text(
                          '${p.name} (${p.chatModel})',
                          style: TextStyle(
                            fontSize: JiyueTokens.fontSm,
                            color: colors.textPrimary,
                          ),
                        ),
                      );
                    }).toList(),
                  ),
                ),
                const SizedBox(height: 12),
              ],
              if (config.memos != null) ...[
                JiyueSectionHeader(
                  colors: colors,
                  title: 'Memos 备忘录配置',
                  padding: const EdgeInsets.only(bottom: 6),
                ),
                JiyueSectionCard(
                  colors: colors,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        '服务端点: ${config.memos!.endpoint}',
                        style: TextStyle(
                          fontSize: JiyueTokens.fontSm,
                          color: colors.textPrimary,
                        ),
                      ),
                      if (config.memos!.token.isNotEmpty) ...[
                        const SizedBox(height: 3),
                        Text(
                          '访问密钥: 已包含凭据',
                          style: TextStyle(
                            fontSize: JiyueTokens.fontXs,
                            color: colors.textMuted,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                const SizedBox(height: 14),
              ],
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
                      label: '确认导入并应用',
                      icon: 'check',
                      kind: JiyueButtonKind.primary,
                      expand: true,
                      onPressed: () async {
                        if (config.s3 != null) {
                          await widget.vaultService.saveS3Config(config.s3!);
                        }
                        if (config.ai != null) {
                          await widget.vaultService.saveAiConfig(config.ai!);
                        }
                        if (config.memos != null) {
                          await widget.vaultService
                              .saveMemosConfig(config.memos!);
                        }
                        if (ctx.mounted) {
                          Navigator.of(ctx).pop();
                        }
                        if (mounted) {
                          setState(() {});
                          ScaffoldMessenger.of(context).showSnackBar(
                            const SnackBar(
                              content: Text(
                                '已成功导入 PC 端配置 (S3/AI/Memos)，已保存生效',
                              ),
                            ),
                          );
                        }
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
  }

  String _syncSummaryText() {
    final parts = <String>[];
    if (widget.vaultService.s3Config != null) {
      parts.add('S3 (${widget.vaultService.s3Config!.bucket})');
    }
    if (widget.vaultService.aiConfig != null) {
      parts.add('AI (${widget.vaultService.aiConfig!.providers.length} 服务商)');
    }
    if (widget.vaultService.memosConfig != null) {
      parts.add('Memos (已绑定)');
    }
    return '已同步: ${parts.join(' · ')}';
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;
    final aiConfig = widget.vaultService.aiConfig;
    final activeProvider =
        (aiConfig != null && aiConfig.providers.isNotEmpty)
            ? aiConfig.providers.firstWhere(
                (p) => p.id == aiConfig.activeChat,
                orElse: () => aiConfig.providers.first,
              )
            : null;
    final s3Config = widget.vaultService.s3Config;
    final memosConfig = widget.vaultService.memosConfig;

    return JiyueModalSheet(
      colors: colors,
      icon: 'settings',
      title: '外观与笔记库设置',
      subtitle: '与桌面端共享同一套 SSOT 主题与存储配置',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // 一、主题外观切换
          JiyueSectionHeader(
            colors: colors,
            title: '主题外观 (Theme)',
          ),
          ...JiyueThemeColors.allThemes.map((theme) {
            final isSelected = theme.id == widget.themeProvider.themeId;
            return JiyueSectionCard(
              colors: colors,
              active: isSelected,
              margin: const EdgeInsets.only(bottom: 8),
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
              onTap: () {
                widget.themeProvider.setTheme(theme.id);
                setState(() {});
              },
              child: Row(
                children: [
                  // 三色微缩主题预览块（canvas + surface + accent）
                  Container(
                    width: 40,
                    height: 32,
                    padding: const EdgeInsets.all(4),
                    decoration: BoxDecoration(
                      color: theme.canvas,
                      borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                      border: Border.all(color: theme.border, width: 1),
                    ),
                    child: Row(
                      children: [
                        Container(
                          width: 4,
                          height: double.infinity,
                          decoration: BoxDecoration(
                            color: theme.accent,
                            borderRadius: BorderRadius.circular(
                              JiyueTokens.radiusPill,
                            ),
                          ),
                        ),
                        const SizedBox(width: 3),
                        Expanded(
                          child: Container(
                            decoration: BoxDecoration(
                              color: theme.surface,
                              borderRadius: BorderRadius.circular(
                                JiyueTokens.radiusSm,
                              ),
                              border: Border.all(
                                color: theme.borderSubtle,
                                width: 1,
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          theme.name,
                          style: TextStyle(
                            fontSize: JiyueTokens.fontBase,
                            fontWeight: FontWeight.w600,
                            color: colors.textPrimary,
                          ),
                        ),
                        const SizedBox(height: 2),
                        Text(
                          theme.description,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: JiyueTokens.fontXs,
                            color: colors.textMuted,
                          ),
                        ),
                      ],
                    ),
                  ),
                  if (isSelected) ...[
                    const SizedBox(width: 8),
                    JiyueBadge(
                      colors: colors,
                      label: '使用中',
                      tone: JiyueBadgeTone.accent,
                      icon: 'check',
                    ),
                  ],
                ],
              ),
            );
          }),

          const SizedBox(height: 12),

          // 二、笔记库管理 (Vaults)
          JiyueSectionHeader(
            colors: colors,
            title: '当前笔记库 (Vaults)',
          ),
          JiyueSectionCard(
            colors: colors,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    JiyueIcon('folder', color: colors.accent, size: 17),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        widget.vaultService.vaultName,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: JiyueTokens.fontMd,
                          fontWeight: FontWeight.w600,
                          color: colors.textPrimary,
                        ),
                      ),
                    ),
                    const SizedBox(width: 8),
                    JiyueBadge(
                      colors: colors,
                      label: '当前 · ${widget.noteCount} 篇',
                      tone: JiyueBadgeTone.accent,
                    ),
                  ],
                ),
                const SizedBox(height: 6),
                Text(
                  widget.vaultService.vaultPath,
                  style: TextStyle(
                    fontSize: JiyueTokens.fontXs,
                    color: colors.textMuted,
                  ),
                ),
                const SizedBox(height: 10),
                Row(
                  children: [
                    Expanded(
                      child: JiyueButton(
                        colors: colors,
                        label: '复制路径',
                        icon: 'copy',
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.xs,
                        expand: true,
                        onPressed: () {
                          Clipboard.setData(
                            ClipboardData(text: widget.vaultService.vaultPath),
                          );
                          ScaffoldMessenger.of(context).showSnackBar(
                            const SnackBar(content: Text('笔记库路径已复制到剪贴板')),
                          );
                        },
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      child: JiyueButton(
                        colors: colors,
                        label: '其他库',
                        icon: 'folder-open',
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.xs,
                        expand: true,
                        onPressed: () {
                          ScaffoldMessenger.of(context).showSnackBar(
                            const SnackBar(content: Text('已准备支持多库切换')),
                          );
                        },
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      child: JiyueButton(
                        colors: colors,
                        label: '新建库',
                        icon: 'folder-plus',
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.xs,
                        expand: true,
                        onPressed: () {
                          ScaffoldMessenger.of(context).showSnackBar(
                            const SnackBar(content: Text('新建笔记库功能就绪')),
                          );
                        },
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 三、跨端快捷配置导入
          JiyueSectionHeader(
            colors: colors,
            title: '跨端配置导入 (PC 导入)',
            trailing: JiyueBadge(
              colors: colors,
              label: '免手动输入',
              tone: JiyueBadgeTone.accent,
            ),
          ),
          JiyueSectionCard(
            colors: colors,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '扫描 PC 端设置弹窗中「手机扫码同步」展示的二维码，一键导入 S3 存储桶凭据、Memos 与 AI 助手配置。',
                  style: TextStyle(
                    fontSize: JiyueTokens.fontSm,
                    color: colors.textSecondary,
                    height: 1.4,
                  ),
                ),
                const SizedBox(height: 10),
                Row(
                  children: [
                    Expanded(
                      child: JiyueButton(
                        colors: colors,
                        label: '扫码导入',
                        icon: 'qrcode',
                        kind: JiyueButtonKind.primary,
                        size: JiyueControlSize.sm,
                        expand: true,
                        onPressed: _handleScanQr,
                      ),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: JiyueButton(
                        colors: colors,
                        label: '粘贴代码',
                        icon: 'copy',
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.sm,
                        expand: true,
                        onPressed: _handlePasteJson,
                      ),
                    ),
                  ],
                ),
                if (s3Config != null ||
                    aiConfig != null ||
                    memosConfig != null) ...[
                  JiyueDivider(
                    colors: colors,
                    margin: const EdgeInsets.symmetric(vertical: 10),
                  ),
                  JiyueBanner(
                    colors: colors,
                    tone: JiyueBadgeTone.ok,
                    icon: 'check',
                    message: _syncSummaryText(),
                  ),
                ],
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 四、AI 助手与服务商管理
          JiyueSectionHeader(
            colors: colors,
            title: 'AI 助手配置 (AI Assistant)',
            trailing: JiyueButton(
              colors: colors,
              label: '管理服务商',
              icon: 'settings',
              kind: JiyueButtonKind.ghost,
              size: JiyueControlSize.xs,
              onPressed: () async {
                await AiConfigSheet.show(
                  context,
                  vaultService: widget.vaultService,
                  colors: colors,
                );
                setState(() {});
              },
            ),
          ),
          JiyueSectionCard(
            colors: colors,
            child: Row(
              children: [
                Container(
                  width: 32,
                  height: 32,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: colors.accentSoft,
                    borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                  ),
                  child: JiyueIcon('sparkles', color: colors.accent, size: 17),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        activeProvider != null
                            ? '默认服务: ${activeProvider.name}'
                            : '未配置 AI 服务商',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: JiyueTokens.fontBase,
                          fontWeight: FontWeight.w600,
                          color: colors.textPrimary,
                        ),
                      ),
                      const SizedBox(height: 2),
                      Text(
                        activeProvider != null
                            ? '模型: ${activeProvider.chatModel}'
                            : '点击右侧以添加 OpenAI 或 Anthropic 服务',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: JiyueTokens.fontXs,
                          color: colors.textMuted,
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                JiyueButton(
                  colors: colors,
                  label: '管理',
                  kind: JiyueButtonKind.secondary,
                  size: JiyueControlSize.sm,
                  onPressed: () async {
                    await AiConfigSheet.show(
                      context,
                      vaultService: widget.vaultService,
                      colors: colors,
                    );
                    setState(() {});
                  },
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 五、云端同步配置 (Sync)
          JiyueSectionHeader(
            colors: colors,
            title: '云端同步配置 (Sync)',
            trailing: JiyueButton(
              colors: colors,
              label: '修改配置',
              icon: 'settings',
              kind: JiyueButtonKind.ghost,
              size: JiyueControlSize.xs,
              onPressed: () async {
                await S3ConfigSheet.show(
                  context,
                  vaultService: widget.vaultService,
                  colors: colors,
                );
                setState(() {});
              },
            ),
          ),
          JiyueSectionCard(
            colors: colors,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                JiyueSwitchRow(
                  colors: colors,
                  icon: 'link',
                  title: '启用 S3 / WebDAV 自动同步',
                  value: _s3Enabled,
                  onChanged: (v) => setState(() => _s3Enabled = v),
                ),
                JiyueDivider(
                  colors: colors,
                  margin: const EdgeInsets.symmetric(vertical: 8),
                ),
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        s3Config != null
                            ? '存储桶: ${s3Config.bucket} (${s3Config.endpoint})'
                            : '协议: S3 (Cloudflare R2 / RustFS / MinIO)',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: JiyueTokens.fontXs,
                          color: colors.textSecondary,
                        ),
                      ),
                    ),
                    const SizedBox(width: 8),
                    JiyueButton(
                      colors: colors,
                      label: s3Config != null ? '已配置' : '配置',
                      kind: JiyueButtonKind.secondary,
                      size: JiyueControlSize.xs,
                      onPressed: () async {
                        await S3ConfigSheet.show(
                          context,
                          vaultService: widget.vaultService,
                          colors: colors,
                        );
                        setState(() {});
                      },
                    ),
                  ],
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 六、Memos 备忘录服务
          JiyueSectionHeader(
            colors: colors,
            title: 'Memos 备忘录服务 (Memos)',
            trailing: JiyueButton(
              colors: colors,
              label: '配置 Memos',
              icon: 'settings',
              kind: JiyueButtonKind.ghost,
              size: JiyueControlSize.xs,
              onPressed: () async {
                await MemosConfigSheet.show(
                  context,
                  vaultService: widget.vaultService,
                  colors: colors,
                );
                setState(() {});
              },
            ),
          ),
          JiyueSectionCard(
            colors: colors,
            child: Row(
              children: [
                Container(
                  width: 32,
                  height: 32,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: colors.accentSoft,
                    borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                  ),
                  child: JiyueIcon('calendar', color: colors.accent, size: 17),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        memosConfig != null
                            ? '端点: ${memosConfig.endpoint}'
                            : '未配置 Memos 服务',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: JiyueTokens.fontBase,
                          fontWeight: FontWeight.w600,
                          color: colors.textPrimary,
                        ),
                      ),
                      const SizedBox(height: 2),
                      Text(
                        memosConfig != null
                            ? 'Token 已保存，点击右侧可测试连接与修改'
                            : '点击右侧按钮以绑定私有或自建 Memos 服务',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: JiyueTokens.fontXs,
                          color: colors.textMuted,
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                JiyueButton(
                  colors: colors,
                  label: memosConfig != null ? '管理' : '配置',
                  kind: JiyueButtonKind.secondary,
                  size: JiyueControlSize.sm,
                  onPressed: () async {
                    await MemosConfigSheet.show(
                      context,
                      vaultService: widget.vaultService,
                      colors: colors,
                    );
                    setState(() {});
                  },
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 七、软件版本与自更新
          JiyueSectionHeader(
            colors: colors,
            title: '软件版本与自更新 (Updates)',
          ),
          JiyueSectionCard(
            colors: colors,
            child: Row(
              children: [
                Container(
                  width: 32,
                  height: 32,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: colors.accentSoft,
                    borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                  ),
                  child: JiyueIcon('download', color: colors.accent, size: 17),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        '霁月 (Jiyue Mobile)',
                        style: TextStyle(
                          fontSize: JiyueTokens.fontBase,
                          fontWeight: FontWeight.w600,
                          color: colors.textPrimary,
                        ),
                      ),
                      const SizedBox(height: 2),
                      Text(
                        '当前版本: v$currentAppVersion',
                        style: TextStyle(
                          fontSize: JiyueTokens.fontXs,
                          color: colors.textMuted,
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                JiyueButton(
                  colors: colors,
                  label: _checkingUpdate ? '检查中...' : '检查更新',
                  icon: 'refresh',
                  loading: _checkingUpdate,
                  kind: JiyueButtonKind.primary,
                  size: JiyueControlSize.sm,
                  onPressed: _checkingUpdate ? null : _checkUpdate,
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),
          JiyueButton(
            colors: colors,
            label: '完成',
            icon: 'check',
            kind: JiyueButtonKind.primary,
            size: JiyueControlSize.lg,
            expand: true,
            onPressed: () => Navigator.of(context).pop(),
          ),
        ],
      ),
    );
  }
}
