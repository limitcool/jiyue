import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../bridge/vault_service.dart';
import '../models/exported_config.dart';
import '../services/platform_bridge.dart';
import '../theme/jiyue_theme.dart';
import '../views/qr_scan_page.dart';
import 'jiyue_controls.dart';
import 'vault_switcher_sheet.dart';

/// Shared user flows for creating, mounting, and importing vaults.
class VaultActions {
  static Future<bool> createLocalVaultFlow(
    BuildContext context, {
    required VaultService vaultService,
    required JiyueThemeColors colors,
  }) async {
    final ctrl = TextEditingController(text: '我的笔记');
    final name = await showDialog<String>(
      context: context,
      builder: (ctx) {
        return JiyueDialog(
          colors: colors,
          icon: 'folder-plus',
          title: '新建纯本地笔记库',
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              JiyueTextField(
                colors: colors,
                controller: ctrl,
                autofocus: true,
                label: '笔记库名称',
                hintText: '例如：个人知识库、工作随记...',
                onSubmitted: (v) => Navigator.of(ctx).pop(v.trim()),
              ),
              const SizedBox(height: 10),
              Text(
                '将创建空白纯本地目录，不写入任何示例笔记，后续随时可在设置中绑定 S3 / WebDAV 同步。',
                style: TextStyle(
                  fontSize: JiyueTokens.fontXs,
                  color: colors.textMuted,
                  height: 1.4,
                ),
              ),
            ],
          ),
          actions: [
            JiyueButton(
              colors: colors,
              label: '取消',
              kind: JiyueButtonKind.secondary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(null),
            ),
            const SizedBox(width: 8),
            JiyueButton(
              colors: colors,
              label: '创建',
              icon: 'check',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(ctrl.text.trim()),
            ),
          ],
        );
      },
    );

    if (name == null || name.isEmpty) return false;
    await vaultService.createLocalVault(name);
    if (context.mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('已创建纯本地笔记库「$name」')),
      );
    }
    return true;
  }

  static Future<bool> openExternalFolderFlow(
    BuildContext context, {
    required VaultService vaultService,
    required JiyueThemeColors colors,
  }) async {
    final hasPerm = await PlatformBridge.hasStoragePermission();
    if (!hasPerm && context.mounted) {
      final grant = await showDialog<bool>(
        context: context,
        builder: (ctx) {
          return JiyueDialog(
            colors: colors,
            icon: 'folder-open',
            title: '需要文件访问权限',
            content: Text(
              '打开手机本地的 Obsidian 文件夹需要授予「所有文件访问」权限，授权后才能直接读写目录中的 Markdown 与附件。',
              style: TextStyle(
                fontSize: JiyueTokens.fontBase,
                color: colors.textSecondary,
                height: 1.45,
              ),
            ),
            actions: [
              JiyueButton(
                colors: colors,
                label: '取消',
                kind: JiyueButtonKind.secondary,
                size: JiyueControlSize.sm,
                onPressed: () => Navigator.of(ctx).pop(false),
              ),
              const SizedBox(width: 8),
              JiyueButton(
                colors: colors,
                label: '前往授权',
                icon: 'check',
                kind: JiyueButtonKind.primary,
                size: JiyueControlSize.sm,
                onPressed: () => Navigator.of(ctx).pop(true),
              ),
            ],
          );
        },
      );
      if (grant != true) return false;
      final granted = await PlatformBridge.requestStoragePermission();
      if (!granted && context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('未获得存储访问权限，无法打开外部文件夹')),
        );
        return false;
      }
    }

    String? pickedPath;
    String? pickerError;
    try {
      pickedPath = await PlatformBridge.pickDirectory();
    } on PlatformException catch (e) {
      pickerError = e.message;
    } catch (e) {
      pickerError = e.toString();
    }

    if ((pickedPath == null || pickedPath.isEmpty) && context.mounted) {
      pickedPath = await _promptManualFolderPath(
        context,
        colors: colors,
        warningText: pickerError,
      );
    }

    if (pickedPath == null || pickedPath.trim().isEmpty) return false;
    final profile = await vaultService.openExternalVault(pickedPath.trim());
    final notes = await vaultService.listNotes();
    if (context.mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text('已打开目录「${profile.name}」，共索引 ${notes.length} 篇笔记'),
        ),
      );
    }
    return true;
  }

  static Future<String?> _promptManualFolderPath(
    BuildContext context, {
    required JiyueThemeColors colors,
    String? warningText,
  }) {
    final pathCtrl = TextEditingController();
    return showDialog<String>(
      context: context,
      builder: (ctx) {
        return JiyueDialog(
          colors: colors,
          icon: 'folder-open',
          title: '指定笔记库文件夹路径',
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (warningText != null && warningText.isNotEmpty) ...[
                JiyueBanner(
                  colors: colors,
                  tone: JiyueBadgeTone.warn,
                  message: warningText,
                ),
                const SizedBox(height: 10),
              ],
              JiyueTextField(
                colors: colors,
                controller: pathCtrl,
                autofocus: true,
                label: '文件夹绝对路径',
                hintText: '例如: /storage/emulated/0/Documents/Obsidian',
                onSubmitted: (v) => Navigator.of(ctx).pop(v.trim()),
              ),
            ],
          ),
          actions: [
            JiyueButton(
              colors: colors,
              label: '取消',
              kind: JiyueButtonKind.secondary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(null),
            ),
            const SizedBox(width: 8),
            JiyueButton(
              colors: colors,
              label: '打开目录',
              icon: 'check',
              kind: JiyueButtonKind.primary,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(pathCtrl.text.trim()),
            ),
          ],
        );
      },
    );
  }

  static Future<bool> importFromQrOrJsonFlow(
    BuildContext context, {
    required VaultService vaultService,
    required JiyueThemeColors colors,
    bool fromScanner = true,
    bool allowUpdateCurrent = false,
  }) async {
    String? rawPayload;
    if (fromScanner) {
      rawPayload = await Navigator.of(context).push<String>(
        MaterialPageRoute(
          builder: (_) => QrScanPage(colors: colors),
        ),
      );
    } else {
      final clip = await Clipboard.getData(Clipboard.kTextPlain);
      rawPayload = clip?.text?.trim();
      if ((rawPayload == null || rawPayload.isEmpty) && context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('剪贴板中没有文本内容')),
        );
        return false;
      }
    }

    if (rawPayload == null || rawPayload.trim().isEmpty || !context.mounted) {
      return false;
    }

    ExportedConfig config;
    try {
      final json = jsonDecode(rawPayload.trim()) as Map<String, dynamic>;
      if (json['jiyue'] != 'config_export') {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('未识别到有效的霁月配置代码')),
          );
        }
        return false;
      }
      config = ExportedConfig.fromJson(json);
    } catch (e) {
      if (context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('配置代码解析失败: $e')),
        );
      }
      return false;
    }

    return showImportPreviewSheet(
      context,
      config: config,
      vaultService: vaultService,
      colors: colors,
      allowUpdateCurrent: allowUpdateCurrent && vaultService.hasActiveVault,
    );
  }

  static Future<bool> showImportPreviewSheet(
    BuildContext context, {
    required ExportedConfig config,
    required VaultService vaultService,
    required JiyueThemeColors colors,
    bool allowUpdateCurrent = false,
  }) async {
    final hasSync = config.s3 != null || config.webdav != null;
    final nameCtrl = TextEditingController(text: config.suggestedVaultName);
    var createNewVault = !allowUpdateCurrent;
    var pullNow = hasSync;
    var importing = false;

    final applied = await showModalBottomSheet<bool>(
      context: context,
      backgroundColor: Colors.transparent,
      isScrollControlled: true,
      builder: (ctx) {
        return StatefulBuilder(
          builder: (ctx, setSheetState) {
            return JiyueModalSheet(
              colors: colors,
              icon: 'check',
              title: '确认导入 PC 端配置',
              subtitle:
                  hasSync ? '包含存储同步配置，可一键建库并同步 PC 笔记' : '包含跨端服务配置',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (hasSync) ...[
                    if (allowUpdateCurrent) ...[
                      JiyueSegmented(
                        colors: colors,
                        fullWidth: true,
                        selectedIndex: createNewVault ? 0 : 1,
                        items: const [
                          ('新建同步笔记库', 'folder-plus'),
                          ('应用到当前库', 'settings'),
                        ],
                        onChanged:
                            (i) =>
                                setSheetState(() => createNewVault = i == 0),
                      ),
                      const SizedBox(height: 12),
                    ],
                    if (createNewVault) ...[
                      JiyueTextField(
                        colors: colors,
                        controller: nameCtrl,
                        label: '新建笔记库名称',
                        hintText: '默认与 PC 端笔记库同名',
                      ),
                      const SizedBox(height: 12),
                    ],
                  ],
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
                  if (config.webdav != null) ...[
                    JiyueSectionHeader(
                      colors: colors,
                      title: 'WebDAV 同步配置',
                      padding: const EdgeInsets.only(bottom: 6),
                    ),
                    JiyueSectionCard(
                      colors: colors,
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            '端点: ${config.webdav!.endpoint}',
                            style: TextStyle(
                              fontSize: JiyueTokens.fontSm,
                              fontWeight: FontWeight.w600,
                              color: colors.textPrimary,
                            ),
                          ),
                          if (config.webdav!.root.isNotEmpty) ...[
                            const SizedBox(height: 3),
                            Text(
                              '远端路径: ${config.webdav!.root}',
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
                  if (hasSync) ...[
                    JiyueSectionCard(
                      colors: colors,
                      padding: const EdgeInsets.symmetric(
                        horizontal: 12,
                        vertical: 8,
                      ),
                      child: JiyueSwitchRow(
                        colors: colors,
                        title: '导入后立即从云端同步',
                        subtitle: '自动拉取 PC 端笔记目录与 Markdown 文件',
                        value: pullNow,
                        onChanged: (v) => setSheetState(() => pullNow = v),
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
                        children:
                            config.ai!.providers.map((p) {
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
                          onPressed:
                              importing ? null : () => Navigator.of(ctx).pop(false),
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: JiyueButton(
                          colors: colors,
                          label: importing ? '正在应用...' : '确认导入',
                          icon: 'check',
                          loading: importing,
                          kind: JiyueButtonKind.primary,
                          expand: true,
                          onPressed:
                              importing
                                  ? null
                                  : () async {
                                    setSheetState(() => importing = true);
                                    if (config.ai != null) {
                                      await vaultService.saveAiConfig(
                                        config.ai!,
                                      );
                                    }
                                    if (config.memos != null) {
                                      await vaultService.saveMemosConfig(
                                        config.memos!,
                                      );
                                    }
                                    if (hasSync) {
                                      if (createNewVault) {
                                        await vaultService.importCloudVault(
                                          name: nameCtrl.text.trim(),
                                          s3: config.s3,
                                          webdav: config.webdav,
                                        );
                                      } else if (config.s3 != null) {
                                        await vaultService.saveS3Config(
                                          config.s3!,
                                        );
                                      } else if (config.webdav != null) {
                                        await vaultService.saveWebDavConfig(
                                          config.webdav!,
                                        );
                                      }
                                    }

                                    String syncFeedback = 'PC 端配置已导入并生效';
                                    if (hasSync && pullNow) {
                                      final report =
                                          await vaultService.executeSync();
                                      syncFeedback =
                                          report.success
                                              ? report.message
                                              : '库已配置，但首次拉取未成功：${report.message}';
                                    }

                                    if (ctx.mounted) {
                                      Navigator.of(ctx).pop(true);
                                    }
                                    if (context.mounted) {
                                      ScaffoldMessenger.of(context)
                                          .showSnackBar(
                                            SnackBar(
                                              content: Text(syncFeedback),
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
      },
    );

    return applied == true;
  }

  /// Displays a clean switcher & manager modal for all local/synced vaults.
  static Future<void> showVaultSwitcherSheet(
    BuildContext context, {
    required VaultService vaultService,
    required JiyueThemeColors colors,
  }) {
    return VaultSwitcherSheet.show(
      context,
      vaultService: vaultService,
      colors: colors,
    );
  }
}
