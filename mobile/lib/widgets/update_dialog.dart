import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown/flutter_markdown.dart';
import 'package:url_launcher/url_launcher.dart';
import '../services/update_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import 'jiyue_controls.dart';

enum _UpdateStage {
  idle,
  downloading,
  readyToInstall,
  waitingPermission,
  error,
}

class UpdateDialog extends StatefulWidget {
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

  @override
  State<UpdateDialog> createState() => _UpdateDialogState();
}

class _UpdateDialogState extends State<UpdateDialog> {
  _UpdateStage _stage = _UpdateStage.idle;
  int _receivedBytes = 0;
  int _totalBytes = 0;
  String? _downloadedApkPath;
  String? _errorMessage;
  UpdateDownloadToken? _downloadToken;

  @override
  void dispose() {
    _downloadToken?.cancel();
    super.dispose();
  }

  Future<void> _startInAppDownload() async {
    final token = UpdateDownloadToken();
    setState(() {
      _downloadToken = token;
      _stage = _UpdateStage.downloading;
      _receivedBytes = 0;
      _totalBytes = widget.result.apkAsset?.size ?? 0;
      _errorMessage = null;
    });

    try {
      final file = await UpdateService.downloadApk(
        widget.result,
        cancelToken: token,
        onProgress: (received, total) {
          if (!mounted) return;
          setState(() {
            _receivedBytes = received;
            if (total > 0) _totalBytes = total;
          });
        },
      );
      if (!mounted) return;
      _downloadedApkPath = file.path;
      await _triggerInstall(file.path);
    } on UpdateCancelledException {
      if (!mounted) return;
      setState(() {
        _stage = _UpdateStage.idle;
        _downloadToken = null;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _stage = _UpdateStage.error;
        _downloadToken = null;
        _errorMessage = e.toString().replaceFirst('Exception: ', '');
      });
    }
  }

  Future<void> _triggerInstall(String filePath) async {
    try {
      final installResult = await UpdateService.installApk(filePath);
      if (!mounted) return;
      setState(() {
        _stage = installResult == ApkInstallResult.permissionRequired
            ? _UpdateStage.waitingPermission
            : _UpdateStage.readyToInstall;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _stage = _UpdateStage.error;
        _errorMessage = e.toString().replaceFirst('Exception: ', '');
      });
    }
  }

  Future<void> _launchExternalBrowser() async {
    final url = widget.result.apkAsset?.downloadUrl ?? widget.result.releaseUrl;
    final uri = Uri.parse(url);
    try {
      final launched =
          await launchUrl(uri, mode: LaunchMode.externalApplication);
      if (!launched && mounted) {
        _copyUrlWithNotice(url);
      }
    } catch (_) {
      if (mounted) {
        _copyUrlWithNotice(url);
      }
    }
  }

  void _copyUrlWithNotice(String url) {
    Clipboard.setData(ClipboardData(text: url));
    setState(() {
      _errorMessage = '未能直接打开浏览器，已将下载链接复制到剪贴板。';
      if (_stage == _UpdateStage.idle) {
        _stage = _UpdateStage.error;
      }
    });
  }

  void _copyReleaseUrl() {
    Clipboard.setData(ClipboardData(text: widget.result.releaseUrl));
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('已复制 GitHub 发布页链接至剪贴板')),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;
    final result = widget.result;

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
    final sizeStr = apk != null && apk.size > 0
        ? UpdateService.formatFileSize(apk.size)
        : null;
    final progressValue =
        _totalBytes > 0 ? (_receivedBytes / _totalBytes).clamp(0.0, 1.0) : null;
    final percentText = progressValue != null
        ? '${(progressValue * 100).round()}%'
        : '下载中';

    return PopScope(
      canPop: _stage != _UpdateStage.downloading,
      child: JiyueDialog(
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
            maxHeight: MediaQuery.of(context).size.height * 0.52,
          ),
          child: SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                JiyueSectionCard(
                  colors: colors,
                  padding:
                      const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
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
                if (_stage == _UpdateStage.downloading) ...[
                  const SizedBox(height: 10),
                  JiyueSectionCard(
                    colors: colors,
                    padding: const EdgeInsets.all(10),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          mainAxisAlignment: MainAxisAlignment.spaceBetween,
                          children: [
                            Text(
                              _totalBytes > 0
                                  ? '正在下载安装包 (${UpdateService.formatFileSize(_receivedBytes)} / ${UpdateService.formatFileSize(_totalBytes)})'
                                  : '正在下载安装包 (${UpdateService.formatFileSize(_receivedBytes)})',
                              style: TextStyle(
                                fontSize: JiyueTokens.fontXs,
                                fontWeight: FontWeight.w600,
                                color: colors.textPrimary,
                              ),
                            ),
                            Text(
                              percentText,
                              style: TextStyle(
                                fontSize: JiyueTokens.fontXs,
                                fontWeight: FontWeight.w600,
                                color: colors.accent,
                              ),
                            ),
                          ],
                        ),
                        const SizedBox(height: 8),
                        ClipRRect(
                          borderRadius:
                              BorderRadius.circular(JiyueTokens.radiusPill),
                          child: LinearProgressIndicator(
                            value: progressValue,
                            minHeight: 6,
                            backgroundColor: colors.elevated,
                            color: colors.accent,
                          ),
                        ),
                      ],
                    ),
                  ),
                ] else if (_stage == _UpdateStage.waitingPermission) ...[
                  const SizedBox(height: 10),
                  JiyueBanner(
                    colors: colors,
                    tone: JiyueBadgeTone.warn,
                    icon: 'alert-triangle',
                    message:
                        '安装包已下载完成。请在系统设置中开启“允许安装来自此来源的应用”，返回后点击“继续安装”。',
                  ),
                ] else if (_stage == _UpdateStage.readyToInstall) ...[
                  const SizedBox(height: 10),
                  JiyueBanner(
                    colors: colors,
                    tone: JiyueBadgeTone.ok,
                    icon: 'check',
                    message: '已拉起系统安装界面；若未自动弹出，可点击下方“重新安装”。',
                  ),
                ] else if (_stage == _UpdateStage.error &&
                    _errorMessage != null) ...[
                  const SizedBox(height: 10),
                  JiyueBanner(
                    colors: colors,
                    tone: JiyueBadgeTone.danger,
                    icon: 'alert-triangle',
                    message: _errorMessage!,
                  ),
                ],
                const SizedBox(height: 12),
                JiyueSectionHeader(
                  colors: colors,
                  title: '更新日志',
                  padding: const EdgeInsets.only(bottom: 6),
                ),
                JiyueSectionCard(
                  colors: colors,
                  padding: const EdgeInsets.all(10),
                  child: result.releaseNotes.trim().isNotEmpty
                      ? MarkdownBody(
                          data: result.releaseNotes,
                          styleSheet: MarkdownStyleSheet.fromTheme(
                            Theme.of(context),
                          ).copyWith(
                            p: TextStyle(
                              fontSize: JiyueTokens.fontSm,
                              height: 1.5,
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
                            h3: TextStyle(
                              fontSize: JiyueTokens.fontBase,
                              fontWeight: FontWeight.w600,
                              color: colors.textSecondary,
                            ),
                            strong: TextStyle(
                              fontWeight: FontWeight.w600,
                              color: colors.textPrimary,
                            ),
                            em: TextStyle(
                              fontStyle: FontStyle.italic,
                              color: colors.textSecondary,
                            ),
                            listBullet: TextStyle(
                              fontSize: JiyueTokens.fontSm,
                              color: colors.accent,
                            ),
                            code: TextStyle(
                              fontSize: JiyueTokens.fontXs,
                              color: colors.accent,
                              backgroundColor: colors.panel,
                              fontFamily: 'monospace',
                            ),
                            codeblockPadding: const EdgeInsets.all(8),
                            codeblockDecoration: BoxDecoration(
                              color: colors.panel,
                              borderRadius: BorderRadius.circular(
                                JiyueTokens.radiusMd,
                              ),
                              border: Border.all(
                                color: colors.borderSubtle,
                                width: 1,
                              ),
                            ),
                            blockquote: TextStyle(
                              fontSize: JiyueTokens.fontSm,
                              color: colors.textSecondary,
                            ),
                            blockquoteDecoration: BoxDecoration(
                              color: colors.panel,
                              border: Border(
                                left: BorderSide(
                                  color: colors.accent,
                                  width: 3,
                                ),
                              ),
                            ),
                            horizontalRuleDecoration: BoxDecoration(
                              border: Border(
                                top: BorderSide(
                                  color: colors.borderSubtle,
                                  width: 1,
                                ),
                              ),
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
        actions: _buildActions(colors, apk, sizeStr),
      ),
    );
  }

  List<Widget> _buildActions(
    JiyueThemeColors colors,
    ReleaseAsset? apk,
    String? sizeStr,
  ) {
    switch (_stage) {
      case _UpdateStage.downloading:
        return [
          Expanded(
            child: JiyueButton(
              colors: colors,
              label: '取消下载',
              icon: 'x',
              kind: JiyueButtonKind.secondary,
              size: JiyueControlSize.sm,
              expand: true,
              onPressed: () => _downloadToken?.cancel(),
            ),
          ),
        ];

      case _UpdateStage.waitingPermission:
        return [
          JiyueButton(
            colors: colors,
            label: '权限设置',
            kind: JiyueButtonKind.secondary,
            size: JiyueControlSize.sm,
            onPressed: () => UpdateService.openInstallPermissionSettings(),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: JiyueButton(
              colors: colors,
              label: '继续安装',
              icon: 'download',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              expand: true,
              onPressed: _downloadedApkPath != null
                  ? () => _triggerInstall(_downloadedApkPath!)
                  : null,
            ),
          ),
        ];

      case _UpdateStage.readyToInstall:
        return [
          JiyueButton(
            colors: colors,
            label: '关闭',
            kind: JiyueButtonKind.secondary,
            size: JiyueControlSize.sm,
            onPressed: () => Navigator.of(context).pop(),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: JiyueButton(
              colors: colors,
              label: '重新安装',
              icon: 'download',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              expand: true,
              onPressed: _downloadedApkPath != null
                  ? () => _triggerInstall(_downloadedApkPath!)
                  : null,
            ),
          ),
        ];

      case _UpdateStage.error:
        return [
          JiyueButton(
            colors: colors,
            label: '浏览器',
            icon: 'link',
            kind: JiyueButtonKind.secondary,
            size: JiyueControlSize.sm,
            onPressed: _launchExternalBrowser,
          ),
          const SizedBox(width: 8),
          Expanded(
            child: JiyueButton(
              colors: colors,
              label: apk != null ? '重试下载' : '打开发布页',
              icon: 'refresh',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              expand: true,
              onPressed: apk != null ? _startInAppDownload : _launchExternalBrowser,
            ),
          ),
        ];

      case _UpdateStage.idle:
        return [
          JiyueIconButton(
            colors: colors,
            icon: 'copy',
            tooltip: '复制发布页链接',
            onPressed: _copyReleaseUrl,
          ),
          const SizedBox(width: 6),
          JiyueIconButton(
            colors: colors,
            icon: 'link',
            tooltip: '在浏览器中查看',
            onPressed: _launchExternalBrowser,
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
              label: apk != null
                  ? (sizeStr != null ? '立即更新 ($sizeStr)' : '立即下载更新')
                  : '前往发布页',
              icon: 'download',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              expand: true,
              onPressed: apk != null ? _startInAppDownload : _launchExternalBrowser,
            ),
          ),
        ];
    }
  }
}
