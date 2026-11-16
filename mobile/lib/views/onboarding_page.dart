import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import '../widgets/jiyue_controls.dart';
import '../widgets/jiyue_icon.dart';
import '../widgets/settings_sheet.dart';
import '../widgets/vault_actions.dart';

class OnboardingPage extends StatefulWidget {
  final ThemeProvider themeProvider;
  final VaultService vaultService;

  const OnboardingPage({
    super.key,
    required this.themeProvider,
    required this.vaultService,
  });

  @override
  State<OnboardingPage> createState() => _OnboardingPageState();
}

class _OnboardingPageState extends State<OnboardingPage> {
  void _openSettings() {
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: Colors.transparent,
      builder:
          (_) => SettingsModalSheet(
            themeProvider: widget.themeProvider,
            vaultService: widget.vaultService,
            noteCount: 0,
          ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;

    return Scaffold(
      backgroundColor: colors.canvas,
      appBar: AppBar(
        backgroundColor: colors.panel,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(1),
          child: JiyueDivider(colors: colors),
        ),
        title: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Image.asset('assets/logo/icon-256.png', width: 20, height: 20),
            const SizedBox(width: 8),
            Text(
              '霁月 Jiyue',
              style: TextStyle(
                fontSize: JiyueTokens.fontLg,
                fontWeight: FontWeight.w600,
                color: colors.textPrimary,
              ),
            ),
          ],
        ),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 12),
            child: JiyueIconButton(
              colors: colors,
              icon: 'settings',
              tooltip: '外观与设置',
              onPressed: _openSettings,
            ),
          ),
        ],
      ),
      body: SafeArea(
        child: SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(20, 28, 20, 28),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  Container(
                    width: 44,
                    height: 44,
                    padding: const EdgeInsets.all(6),
                    decoration: BoxDecoration(
                      color: colors.surface,
                      borderRadius: BorderRadius.circular(JiyueTokens.radiusLg),
                      border: Border.all(color: colors.border, width: 1),
                    ),
                    child: Image.asset('assets/logo/icon-256.png'),
                  ),
                  const SizedBox(width: 14),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          '选择或建立笔记库',
                          style: TextStyle(
                            fontSize: JiyueTokens.fontXl,
                            fontWeight: FontWeight.w700,
                            color: colors.textPrimary,
                          ),
                        ),
                        const SizedBox(height: 3),
                        Text(
                          '兼容 Obsidian 目录结构与 Markdown 纯文本，完全由你掌控。',
                          style: TextStyle(
                            fontSize: JiyueTokens.fontSm,
                            color: colors.textMuted,
                            height: 1.4,
                          ),
                        ),
                      ],
                    ),
                  ),
                ],
              ),

              const SizedBox(height: 24),

              // 1. Scan PC QR or paste JSON to clone synced vault
              JiyueSectionCard(
                colors: colors,
                active: true,
                padding: const EdgeInsets.all(14),
                margin: const EdgeInsets.only(bottom: 12),
                onTap:
                    () => VaultActions.importFromQrOrJsonFlow(
                      context,
                      vaultService: widget.vaultService,
                      colors: colors,
                      fromScanner: true,
                    ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Container(
                          width: 34,
                          height: 34,
                          alignment: Alignment.center,
                          decoration: BoxDecoration(
                            color: colors.accentSoft,
                            borderRadius: BorderRadius.circular(
                              JiyueTokens.radiusMd,
                            ),
                          ),
                          child: JiyueIcon(
                            'qrcode',
                            color: colors.accent,
                            size: 18,
                          ),
                        ),
                        const SizedBox(width: 12),
                        Expanded(
                          child: Text(
                            '扫码导入 PC 笔记库',
                            style: TextStyle(
                              fontSize: JiyueTokens.fontMd,
                              fontWeight: FontWeight.w600,
                              color: colors.textPrimary,
                            ),
                          ),
                        ),
                        JiyueBadge(
                          colors: colors,
                          label: '跨端同步推荐',
                          tone: JiyueBadgeTone.accent,
                        ),
                      ],
                    ),
                    const SizedBox(height: 10),
                    Text(
                      '扫描桌面端「设置 -> 手机扫码同步」二维码，自动配置 S3 / WebDAV 前缀并拉取 PC 端的 Obsidian 笔记库。',
                      style: TextStyle(
                        fontSize: JiyueTokens.fontSm,
                        color: colors.textSecondary,
                        height: 1.45,
                      ),
                    ),
                    const SizedBox(height: 12),
                    Row(
                      children: [
                        Expanded(
                          child: JiyueButton(
                            colors: colors,
                            label: '立即扫码',
                            icon: 'qrcode',
                            kind: JiyueButtonKind.primary,
                            size: JiyueControlSize.sm,
                            expand: true,
                            onPressed:
                                () => VaultActions.importFromQrOrJsonFlow(
                                  context,
                                  vaultService: widget.vaultService,
                                  colors: colors,
                                  fromScanner: true,
                                ),
                          ),
                        ),
                        const SizedBox(width: 8),
                        Expanded(
                          child: JiyueButton(
                            colors: colors,
                            label: '粘贴 JSON 代码',
                            icon: 'copy',
                            kind: JiyueButtonKind.secondary,
                            size: JiyueControlSize.sm,
                            expand: true,
                            onPressed:
                                () => VaultActions.importFromQrOrJsonFlow(
                                  context,
                                  vaultService: widget.vaultService,
                                  colors: colors,
                                  fromScanner: false,
                                ),
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
              ),

              // 2. Open existing local folder (e.g. Obsidian vault synced by Syncthing/FolderSync)
              _entryCard(
                colors: colors,
                icon: 'folder-open',
                title: '打开手机现有文件夹',
                badge: '适配 Obsidian 目录',
                description:
                    '直接挂载手机内部存储中的现有 Markdown 笔记目录（如已通过 Syncthing 或其他工具同步的库）。',
                onTap:
                    () => VaultActions.openExternalFolderFlow(
                      context,
                      vaultService: widget.vaultService,
                      colors: colors,
                    ),
              ),

              // 3. Create clean local empty vault
              _entryCard(
                colors: colors,
                icon: 'folder-plus',
                title: '新建空白本地笔记库',
                badge: '纯本地离线',
                description:
                    '创建一个干净空白的纯本地笔记库，不写任何示例文档，后续可随时在设置中绑定多库同步。',
                onTap:
                    () => VaultActions.createLocalVaultFlow(
                      context,
                      vaultService: widget.vaultService,
                      colors: colors,
                    ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _entryCard({
    required JiyueThemeColors colors,
    required String icon,
    required String title,
    required String badge,
    required String description,
    required VoidCallback onTap,
  }) {
    return JiyueSectionCard(
      colors: colors,
      padding: const EdgeInsets.all(14),
      margin: const EdgeInsets.only(bottom: 12),
      onTap: onTap,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Container(
                width: 34,
                height: 34,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  color: colors.elevated,
                  borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
                ),
                child: JiyueIcon(icon, color: colors.textSecondary, size: 18),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  title,
                  style: TextStyle(
                    fontSize: JiyueTokens.fontMd,
                    fontWeight: FontWeight.w600,
                    color: colors.textPrimary,
                  ),
                ),
              ),
              JiyueBadge(
                colors: colors,
                label: badge,
                tone: JiyueBadgeTone.neutral,
              ),
            ],
          ),
          const SizedBox(height: 8),
          Text(
            description,
            style: TextStyle(
              fontSize: JiyueTokens.fontSm,
              color: colors.textSecondary,
              height: 1.45,
            ),
          ),
        ],
      ),
    );
  }
}
