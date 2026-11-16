import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';
import '../bridge/vault_service.dart';
import '../services/update_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import 'ai_config_sheet.dart';
import 'jiyue_controls.dart';
import 'jiyue_icon.dart';
import 'memos_config_sheet.dart';
import 's3_config_sheet.dart';
import 'update_dialog.dart';
import 'vault_actions.dart';
import 'vault_switcher_sheet.dart';

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
  bool _checkingUpdate = false;
  String _installedVersion = currentAppVersion;
  String? _updateMessage;
  bool _updateIsError = false;

  @override
  void initState() {
    super.initState();
    _loadInstalledVersion();
  }

  Future<void> _loadInstalledVersion() async {
    final version = await UpdateService.getInstalledVersion();
    if (mounted && version != _installedVersion) {
      setState(() => _installedVersion = version);
    }
  }

  Future<void> _openReleasesPage() async {
    final uri = Uri.parse(
      'https://github.com/$defaultRepoOwner/$defaultRepoName/releases/latest',
    );
    try {
      final launched = await launchUrl(
        uri,
        mode: LaunchMode.externalApplication,
      );
      if (!launched && mounted) {
        Clipboard.setData(ClipboardData(text: uri.toString()));
        setState(() {
          _updateMessage = '已复制发布页链接至剪贴板，请在浏览器中打开';
          _updateIsError = false;
        });
      }
    } catch (_) {
      if (mounted) {
        Clipboard.setData(ClipboardData(text: uri.toString()));
        setState(() {
          _updateMessage = '已复制发布页链接至剪贴板，请在浏览器中打开';
          _updateIsError = false;
        });
      }
    }
  }

  Future<void> _checkUpdate() async {
    setState(() {
      _checkingUpdate = true;
      _updateMessage = null;
      _updateIsError = false;
    });
    try {
      final res =
          await UpdateService.checkUpdate(currentVersion: _installedVersion);
      if (!mounted) return;
      setState(() => _checkingUpdate = false);
      if (!res.hasUpdate) {
        setState(() {
          _updateMessage = '当前已是最新版本 v${res.currentVersion}';
          _updateIsError = false;
        });
        return;
      }
      UpdateDialog.show(
        context,
        result: res,
        themeProvider: widget.themeProvider,
      );
    } catch (e) {
      if (!mounted) return;
      final msg = e.toString().replaceFirst('Exception: ', '');
      setState(() {
        _checkingUpdate = false;
        _updateMessage = '检查更新失败: $msg';
        _updateIsError = true;
      });
    }
  }

  String _syncSummaryText() {
    final parts = <String>[];
    if (widget.vaultService.s3Config != null) {
      parts.add('S3 (${widget.vaultService.s3Config!.bucket})');
    } else if (widget.vaultService.webdavConfig != null) {
      parts.add('WebDAV');
    }
    if (widget.vaultService.aiConfig != null) {
      parts.add('AI (${widget.vaultService.aiConfig!.providers.length} 服务商)');
    }
    if (widget.vaultService.memosConfig != null) {
      parts.add('Memos (已绑定)');
    }
    return parts.isEmpty ? '已导入配置' : '当前已配置: ${parts.join(' · ')}';
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
    final activeVault = widget.vaultService.activeVault;
    final s3Config = widget.vaultService.s3Config;
    final davConfig = widget.vaultService.webdavConfig;
    final memosConfig = widget.vaultService.memosConfig;

    return JiyueModalSheet(
      colors: colors,
      icon: 'settings',
      title: '外观与笔记库设置',
      subtitle: '与桌面端共享同一套 SSOT 主题与多库存储配置',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // 一、主题外观切换
          JiyueSectionHeader(
            colors: colors,
            title: '主题外观',
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

          // 二、当前笔记库（多库管理入口）
          JiyueSectionHeader(
            colors: colors,
            title: '笔记库管理 (${widget.vaultService.vaults.length} 个)',
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
                      label: activeVault != null
                          ? '${activeVault.syncBadgeLabel} · ${widget.noteCount} 篇'
                          : '未打开',
                      tone: (activeVault?.hasRemoteSync ?? false)
                          ? JiyueBadgeTone.ok
                          : JiyueBadgeTone.neutral,
                    ),
                  ],
                ),
                if (widget.vaultService.vaultPath.isNotEmpty) ...[
                  const SizedBox(height: 8),
                  Text(
                    widget.vaultService.vaultPath,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: JiyueTokens.fontXs,
                      color: colors.textMuted,
                    ),
                  ),
                ],
                const SizedBox(height: 10),
                Row(
                  children: [
                    Expanded(
                      child: JiyueButton(
                        colors: colors,
                        label: '切换 / 管理',
                        icon: 'selector',
                        kind: JiyueButtonKind.primary,
                        size: JiyueControlSize.xs,
                        expand: true,
                        onPressed: () async {
                          await VaultSwitcherSheet.show(
                            context,
                            vaultService: widget.vaultService,
                            colors: colors,
                          );
                          if (mounted) setState(() {});
                        },
                      ),
                    ),
                    if (widget.vaultService.vaultPath.isNotEmpty) ...[
                      const SizedBox(width: 8),
                      JiyueButton(
                        colors: colors,
                        label: '复制路径',
                        icon: 'copy',
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.xs,
                        onPressed: () {
                          Clipboard.setData(
                            ClipboardData(text: widget.vaultService.vaultPath),
                          );
                          ScaffoldMessenger.of(context).showSnackBar(
                            const SnackBar(content: Text('笔记库路径已复制到剪贴板')),
                          );
                        },
                      ),
                    ],
                  ],
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 三、跨端快捷配置导入
          JiyueSectionHeader(
            colors: colors,
            title: '跨端配置导入',
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
                  '扫描 PC 端设置弹窗中「手机扫码同步」展示的二维码，自动建立同步笔记库并导入 S3 / WebDAV、Memos 与 AI 配置。',
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
                        onPressed: () async {
                          final ok = await VaultActions.importFromQrOrJsonFlow(
                            context,
                            vaultService: widget.vaultService,
                            colors: colors,
                            fromScanner: true,
                            allowUpdateCurrent: true,
                          );
                          if (ok && mounted) setState(() {});
                        },
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
                        onPressed: () async {
                          final ok = await VaultActions.importFromQrOrJsonFlow(
                            context,
                            vaultService: widget.vaultService,
                            colors: colors,
                            fromScanner: false,
                            allowUpdateCurrent: true,
                          );
                          if (ok && mounted) setState(() {});
                        },
                      ),
                    ),
                  ],
                ),
                if (s3Config != null ||
                    davConfig != null ||
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

          // 四、AI 助手
          JiyueSectionHeader(
            colors: colors,
            title: 'AI 助手',
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
                  label: activeProvider != null ? '管理' : '配置',
                  kind: JiyueButtonKind.secondary,
                  size: JiyueControlSize.sm,
                  onPressed: () async {
                    await AiConfigSheet.show(
                      context,
                      vaultService: widget.vaultService,
                      colors: colors,
                    );
                    if (mounted) setState(() {});
                  },
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 五、当前库云端同步
          JiyueSectionHeader(
            colors: colors,
            title: '当前库云端同步',
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
                  child: JiyueIcon('link', color: colors.accent, size: 17),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        s3Config != null
                            ? 'S3 存储桶: ${s3Config.bucket}'
                            : (davConfig != null
                                ? 'WebDAV: ${davConfig.endpoint}'
                                : '当前为纯本地笔记库'),
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
                        s3Config != null
                            ? '前缀: ${s3Config.rootPrefix.isEmpty ? '(桶根目录)' : s3Config.rootPrefix}'
                            : (davConfig != null
                                ? '远端目录: ${davConfig.root.isEmpty ? '/' : davConfig.root}'
                                : '可随时绑定 S3 或 WebDAV 实现跨端漫游'),
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
                  label: (s3Config != null || davConfig != null) ? '管理' : '配置',
                  kind: JiyueButtonKind.secondary,
                  size: JiyueControlSize.sm,
                  onPressed: widget.vaultService.hasActiveVault
                      ? () async {
                          await S3ConfigSheet.show(
                            context,
                            vaultService: widget.vaultService,
                            colors: colors,
                          );
                          if (mounted) setState(() {});
                        }
                      : null,
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 六、Memos 速记
          JiyueSectionHeader(
            colors: colors,
            title: 'Memos 速记',
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
                    if (mounted) setState(() {});
                  },
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // 七、版本与更新
          JiyueSectionHeader(
            colors: colors,
            title: kIsFossBuild ? '关于应用' : '版本与更新',
          ),
          JiyueSectionCard(
            colors: colors,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Container(
                      width: 32,
                      height: 32,
                      alignment: Alignment.center,
                      decoration: BoxDecoration(
                        color: colors.accentSoft,
                        borderRadius: BorderRadius.circular(
                          JiyueTokens.radiusSm,
                        ),
                      ),
                      child: JiyueIcon(
                        kIsFossBuild ? 'info-circle' : 'download',
                        color: colors.accent,
                        size: 17,
                      ),
                    ),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            '霁月 Mobile',
                            style: TextStyle(
                              fontSize: JiyueTokens.fontBase,
                              fontWeight: FontWeight.w600,
                              color: colors.textPrimary,
                            ),
                          ),
                          const SizedBox(height: 2),
                          Text(
                            kIsFossBuild
                                ? '当前版本: v$_installedVersion (FOSS)'
                                : '当前版本: v$_installedVersion',
                            style: TextStyle(
                              fontSize: JiyueTokens.fontXs,
                              color: colors.textMuted,
                            ),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(width: 8),
                    if (kIsFossBuild)
                      JiyueButton(
                        colors: colors,
                        label: '开源发布页',
                        icon: 'link',
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.sm,
                        onPressed: _openReleasesPage,
                      )
                    else
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
                if (_updateMessage != null) ...[
                  JiyueDivider(
                    colors: colors,
                    margin: const EdgeInsets.symmetric(vertical: 10),
                  ),
                  JiyueBanner(
                    colors: colors,
                    tone: _updateIsError
                        ? JiyueBadgeTone.danger
                        : JiyueBadgeTone.ok,
                    icon: _updateIsError ? 'alert-triangle' : 'check',
                    message: _updateMessage!,
                  ),
                  if (_updateIsError) ...[
                    const SizedBox(height: 8),
                    Align(
                      alignment: Alignment.centerRight,
                      child: JiyueButton(
                        colors: colors,
                        label: '前往网页查看',
                        icon: 'link',
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.xs,
                        onPressed: _openReleasesPage,
                      ),
                    ),
                  ],
                ],
              ],
            ),
          ),

          const SizedBox(height: 8),
        ],
      ),
    );
  }
}
