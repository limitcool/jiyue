import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown/flutter_markdown.dart';
import 'package:url_launcher/url_launcher.dart';
import '../services/update_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import 'jiyue_controls.dart';

class UpdateDialog extends StatelessWidget {
  final UpdateCheckResult result;
  final ThemeProvider themeProvider;

  const UpdateDialog({
    super.key,
    required this.result,
    required this.themeProvider,
  });

  static Future<void> show(
    BuildContext context, {
    required UpdateCheckResult result,
    required ThemeProvider themeProvider,
  }) {
    return showDialog(
      context: context,
      builder: (_) =>
          UpdateDialog(result: result, themeProvider: themeProvider),
    );
  }

  Future<void> _launchDownload(BuildContext context) async {
    final url = result.apkAsset?.downloadUrl ?? result.releaseUrl;
    final uri = Uri.parse(url);
    try {
      final launched =
          await launchUrl(uri, mode: LaunchMode.externalApplication);
      if (!launched && context.mounted) {
        Clipboard.setData(ClipboardData(text: url));
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('已复制下载链接至剪贴板，请在浏览器中打开')),
        );
      }
    } catch (_) {
      if (context.mounted) {
        Clipboard.setData(ClipboardData(text: url));
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('已复制下载链接至剪贴板，请在浏览器中打开')),
        );
      }
    }
  }

  void _copyReleaseUrl(BuildContext context) {
    Clipboard.setData(ClipboardData(text: result.releaseUrl));
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('已复制 GitHub 发布页链接至剪贴板')),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = themeProvider.colors;

    if (!result.hasUpdate) {
      return JiyueDialog(
        colors: colors,
        icon: 'check',
        title: '当前已是最新版本',
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              '霁月 Jiyue Mobile (v${result.currentVersion})',
              style: TextStyle(
                fontSize: JiyueTokens.fontMd,
                fontWeight: FontWeight.w600,
                color: colors.textPrimary,
              ),
            ),
            const SizedBox(height: 6),
            Text(
              '暂无可用更新，您已享受最新特性与安全优化。',
              style: TextStyle(
                fontSize: JiyueTokens.fontSm,
                color: colors.textMuted,
                height: 1.4,
              ),
            ),
          ],
        ),
        actions: [
          JiyueButton(
            colors: colors,
            label: '确定',
            kind: JiyueButtonKind.primary,
            size: JiyueControlSize.sm,
            onPressed: () => Navigator.of(context).pop(),
          ),
        ],
      );
    }

    final apk = result.apkAsset;
    final sizeStr = apk != null ? UpdateService.formatFileSize(apk.size) : null;

    return JiyueDialog(
      colors: colors,
      icon: 'sparkles',
      title: '发现新版本',
      badge: JiyueBadge(
        colors: colors,
        label: 'v${result.latestVersion}',
        tone: JiyueBadgeTone.accent,
      ),
      content: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.of(context).size.height * 0.48,
        ),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              JiyueSectionCard(
                colors: colors,
                padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    Text(
                      '当前版本: v${result.currentVersion}',
                      style: TextStyle(
                        fontSize: JiyueTokens.fontXs,
                        color: colors.textMuted,
                      ),
                    ),
                    if (sizeStr != null)
                      Text(
                        '安装包: $sizeStr',
                        style: TextStyle(
                          fontSize: JiyueTokens.fontXs,
                          fontWeight: FontWeight.w600,
                          color: colors.textPrimary,
                        ),
                      ),
                  ],
                ),
              ),
              const SizedBox(height: 12),
              JiyueSectionHeader(
                colors: colors,
                title: '更新日志 (Release Notes)',
                padding: const EdgeInsets.only(bottom: 6),
              ),
              JiyueSectionCard(
                colors: colors,
                padding: const EdgeInsets.all(10),
                child: result.releaseNotes.trim().isNotEmpty
                    ? MarkdownBody(
                        data: result.releaseNotes,
                        styleSheet:
                            MarkdownStyleSheet.fromTheme(Theme.of(context))
                                .copyWith(
                          p: TextStyle(
                            fontSize: JiyueTokens.fontSm,
                            height: 1.45,
                            color: colors.textPrimary,
                          ),
                          h1: TextStyle(
                            fontSize: JiyueTokens.fontMd,
                            fontWeight: FontWeight.w600,
                            color: colors.textPrimary,
                          ),
                          h2: TextStyle(
                            fontSize: JiyueTokens.fontBase,
                            fontWeight: FontWeight.w600,
                            color: colors.textPrimary,
                          ),
                          listBullet: TextStyle(
                            fontSize: JiyueTokens.fontSm,
                            color: colors.accent,
                          ),
                        ),
                      )
                    : Text(
                        '本次更新包含了性能改进与功能优化。',
                        style: TextStyle(
                          fontSize: JiyueTokens.fontSm,
                          color: colors.textMuted,
                        ),
                      ),
              ),
            ],
          ),
        ),
      ),
      actions: [
        JiyueIconButton(
          colors: colors,
          icon: 'copy',
          tooltip: '复制发布页链接',
          onPressed: () => _copyReleaseUrl(context),
        ),
        const SizedBox(width: 8),
        JiyueButton(
          colors: colors,
          label: '稍后',
          kind: JiyueButtonKind.secondary,
          size: JiyueControlSize.sm,
          onPressed: () => Navigator.of(context).pop(),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: JiyueButton(
            colors: colors,
            label: apk != null ? '下载更新 ($sizeStr)' : '查看发布页',
            icon: 'download',
            kind: JiyueButtonKind.primary,
            size: JiyueControlSize.sm,
            expand: true,
            onPressed: () => _launchDownload(context),
          ),
        ),
      ],
    );
  }
}
