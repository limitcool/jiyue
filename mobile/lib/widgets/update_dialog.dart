import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown/flutter_markdown.dart';
import 'package:url_launcher/url_launcher.dart';
import '../services/update_service.dart';
import '../theme/theme_provider.dart';
import 'jiyue_icon.dart';

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
      builder: (_) => UpdateDialog(result: result, themeProvider: themeProvider),
    );
  }

  Future<void> _launchDownload(BuildContext context) async {
    final url = result.apkAsset?.downloadUrl ?? result.releaseUrl;
    final uri = Uri.parse(url);
    try {
      final launched = await launchUrl(uri, mode: LaunchMode.externalApplication);
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
      return AlertDialog(
        backgroundColor: colors.sheet,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(20)),
        title: Row(
          children: [
            Container(
              padding: const EdgeInsets.all(6),
              decoration: BoxDecoration(
                color: colors.accent.withAlpha(25),
                borderRadius: BorderRadius.circular(10),
              ),
              child: JiyueIcon('check-circle', color: colors.accent, size: 20),
            ),
            const SizedBox(width: 10),
            Text(
              '当前已是最新版本',
              style: TextStyle(fontSize: 16, fontWeight: FontWeight.bold, color: colors.textPrimary),
            ),
          ],
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              '霁月 Jiyue Mobile (v${result.currentVersion})',
              style: TextStyle(fontSize: 14, fontWeight: FontWeight.bold, color: colors.textPrimary),
            ),
            const SizedBox(height: 8),
            Text(
              '暂无可用更新，您已享受最新特性与安全优化。',
              style: TextStyle(fontSize: 12, color: colors.textMuted, height: 1.4),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: Text('好', style: TextStyle(color: colors.accent, fontWeight: FontWeight.bold)),
          ),
        ],
      );
    }

    final apk = result.apkAsset;
    final sizeStr = apk != null ? UpdateService.formatFileSize(apk.size) : null;

    return AlertDialog(
      backgroundColor: colors.sheet,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(24)),
      contentPadding: const EdgeInsets.fromLTRB(20, 16, 20, 16),
      titlePadding: const EdgeInsets.fromLTRB(20, 20, 20, 0),
      title: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Row(
            children: [
              Container(
                padding: const EdgeInsets.all(6),
                decoration: BoxDecoration(
                  color: colors.accent.withAlpha(30),
                  borderRadius: BorderRadius.circular(10),
                ),
                child: JiyueIcon('sparkles', color: colors.accent, size: 20),
              ),
              const SizedBox(width: 10),
              Text(
                '发现新版本',
                style: TextStyle(fontSize: 16, fontWeight: FontWeight.bold, color: colors.textPrimary),
              ),
            ],
          ),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
            decoration: BoxDecoration(
              color: colors.accent,
              borderRadius: BorderRadius.circular(12),
            ),
            child: Text(
              'v${result.latestVersion}',
              style: TextStyle(fontSize: 11, fontWeight: FontWeight.bold, color: colors.onAccent),
            ),
          ),
        ],
      ),
      content: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.of(context).size.height * 0.5,
          maxWidth: 400,
        ),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // 版本升级标签条
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(10),
                  border: Border.all(color: colors.border),
                ),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    Text('当前版本: v${result.currentVersion}', style: TextStyle(fontSize: 11, color: colors.textMuted)),
                    if (sizeStr != null)
                      Text('安装包: $sizeStr', style: TextStyle(fontSize: 11, fontWeight: FontWeight.bold, color: colors.textPrimary)),
                  ],
                ),
              ),
              const SizedBox(height: 12),

              // 更新日志标题
              Text(
                '更新日志 (Release Notes)',
                style: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
              ),
              const SizedBox(height: 6),

              // Markdown 日志正文
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: colors.raised,
                  borderRadius: BorderRadius.circular(12),
                  border: Border.all(color: colors.border),
                ),
                child: result.releaseNotes.trim().isNotEmpty
                    ? MarkdownBody(
                        data: result.releaseNotes,
                        styleSheet: MarkdownStyleSheet.fromTheme(Theme.of(context)).copyWith(
                          p: TextStyle(fontSize: 12, height: 1.5, color: colors.textPrimary),
                          h1: TextStyle(fontSize: 14, fontWeight: FontWeight.bold, color: colors.textPrimary),
                          h2: TextStyle(fontSize: 13, fontWeight: FontWeight.bold, color: colors.textPrimary),
                          listBullet: TextStyle(fontSize: 12, color: colors.accent),
                        ),
                      )
                    : Text(
                        '本次更新包含了性能改进与功能优化。',
                        style: TextStyle(fontSize: 12, color: colors.textMuted),
                      ),
              ),
            ],
          ),
        ),
      ),
      actionsPadding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
      actions: [
        Row(
          children: [
            IconButton(
              icon: JiyueIcon('copy', color: colors.textMuted, size: 18),
              tooltip: '复制发布页链接',
              onPressed: () => _copyReleaseUrl(context),
            ),
            const SizedBox(width: 4),
            Expanded(
              child: OutlinedButton(
                style: OutlinedButton.styleFrom(
                  foregroundColor: colors.textPrimary,
                  side: BorderSide(color: colors.border),
                  padding: const EdgeInsets.symmetric(vertical: 10),
                  shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
                ),
                onPressed: () => Navigator.of(context).pop(),
                child: const Text('稍后', style: TextStyle(fontSize: 12)),
              ),
            ),
            const SizedBox(width: 8),
            Expanded(
              flex: 2,
              child: ElevatedButton.icon(
                style: ElevatedButton.styleFrom(
                  backgroundColor: colors.accent,
                  foregroundColor: colors.onAccent,
                  padding: const EdgeInsets.symmetric(vertical: 10),
                  shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
                ),
                icon: JiyueIcon('download', color: colors.onAccent, size: 16),
                label: Text(
                  apk != null ? '下载更新 (${sizeStr ?? ""})' : '查看发布页',
                  style: const TextStyle(fontSize: 12, fontWeight: FontWeight.bold),
                ),
                onPressed: () => _launchDownload(context),
              ),
            ),
          ],
        ),
      ],
    );
  }
}
