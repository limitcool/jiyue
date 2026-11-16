import 'package:flutter/material.dart';
import '../bridge/vault_service.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_controls.dart';
import 'jiyue_icon.dart';
import 'vault_actions.dart';

/// Bottom sheet displaying all local and synced vaults for quick switching and management.
class VaultSwitcherSheet extends StatefulWidget {
  final VaultService vaultService;
  final JiyueThemeColors colors;

  const VaultSwitcherSheet({
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
      builder:
          (_) =>
              VaultSwitcherSheet(vaultService: vaultService, colors: colors),
    );
  }

  @override
  State<VaultSwitcherSheet> createState() => _VaultSwitcherSheetState();
}

class _VaultSwitcherSheetState extends State<VaultSwitcherSheet> {
  Future<void> _confirmRemoveVault(String vaultId, String vaultName) async {
    final colors = widget.colors;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) {
        return JiyueDialog(
          colors: colors,
          icon: 'trash',
          title: '移除笔记库',
          content: Text(
            '确认从列表移除笔记库「$vaultName」？手机硬盘中的原有文件夹与 Markdown 文件不会被删除。',
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
              label: '仅从列表移除',
              icon: 'trash',
              kind: JiyueButtonKind.danger,
              size: JiyueControlSize.sm,
              onPressed: () => Navigator.of(ctx).pop(true),
            ),
          ],
        );
      },
    );

    if (confirmed == true) {
      await widget.vaultService.removeVault(vaultId, deleteLocalFiles: false);
      if (mounted) {
        if (!widget.vaultService.hasActiveVault) {
          Navigator.of(context).pop();
        } else {
          setState(() {});
        }
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.colors;
    final vaults = widget.vaultService.vaults;
    final activeId = widget.vaultService.activeVault?.id;

    return JiyueModalSheet(
      colors: colors,
      icon: 'folder',
      title: '笔记库切换与管理',
      subtitle: '支持纯本地目录、外部 Obsidian 文件夹与多 S3 前缀隔离',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (vaults.isEmpty)
            JiyueEmptyState(
              colors: colors,
              icon: 'folder',
              title: '尚未添加笔记库',
              subtitle: '请使用下方入口新建、打开或扫码导入',
            )
          else
            ...vaults.map((v) {
              final isActive = v.id == activeId;
              return JiyueSectionCard(
                colors: colors,
                active: isActive,
                margin: const EdgeInsets.only(bottom: 8),
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 10,
                ),
                onTap: () async {
                  if (!isActive) {
                    await widget.vaultService.switchVault(v.id);
                  }
                  if (context.mounted) Navigator.of(context).pop();
                },
                child: Row(
                  children: [
                    Container(
                      width: 32,
                      height: 32,
                      alignment: Alignment.center,
                      decoration: BoxDecoration(
                        color: isActive ? colors.accentSoft : colors.elevated,
                        borderRadius: BorderRadius.circular(
                          JiyueTokens.radiusSm,
                        ),
                      ),
                      child: JiyueIcon(
                        v.hasRemoteSync
                            ? 'refresh'
                            : (v.isExternal ? 'folder-open' : 'folder'),
                        color: isActive ? colors.accent : colors.textSecondary,
                        size: 16,
                      ),
                    ),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Row(
                            children: [
                              Flexible(
                                child: Text(
                                  v.name,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: TextStyle(
                                    fontSize: JiyueTokens.fontBase,
                                    fontWeight: FontWeight.w600,
                                    color: colors.textPrimary,
                                  ),
                                ),
                              ),
                              const SizedBox(width: 6),
                              JiyueBadge(
                                colors: colors,
                                label: v.syncBadgeLabel,
                                tone:
                                    v.hasRemoteSync
                                        ? JiyueBadgeTone.ok
                                        : JiyueBadgeTone.neutral,
                              ),
                            ],
                          ),
                          const SizedBox(height: 3),
                          Text(
                            v.path,
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
                    const SizedBox(width: 4),
                    JiyueIconButton(
                      colors: colors,
                      icon: 'trash',
                      size: 30,
                      iconSize: 15,
                      danger: true,
                      tooltip: '从列表移除',
                      onPressed: () => _confirmRemoveVault(v.id, v.name),
                    ),
                  ],
                ),
              );
            }),

          const SizedBox(height: 8),
          JiyueSectionHeader(
            colors: colors,
            title: '添加笔记库',
            padding: const EdgeInsets.only(bottom: 8),
          ),
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
                      allowUpdateCurrent: false,
                    );
                    if (ok && context.mounted) {
                      Navigator.of(context).pop();
                    }
                  },
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: JiyueButton(
                  colors: colors,
                  label: '打开文件夹',
                  icon: 'folder-open',
                  kind: JiyueButtonKind.secondary,
                  size: JiyueControlSize.sm,
                  expand: true,
                  onPressed: () async {
                    final ok = await VaultActions.openExternalFolderFlow(
                      context,
                      vaultService: widget.vaultService,
                      colors: colors,
                    );
                    if (ok && context.mounted) {
                      Navigator.of(context).pop();
                    }
                  },
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: JiyueButton(
                  colors: colors,
                  label: '新建空白库',
                  icon: 'folder-plus',
                  kind: JiyueButtonKind.secondary,
                  size: JiyueControlSize.sm,
                  expand: true,
                  onPressed: () async {
                    final ok = await VaultActions.createLocalVaultFlow(
                      context,
                      vaultService: widget.vaultService,
                      colors: colors,
                    );
                    if (ok && context.mounted) {
                      Navigator.of(context).pop();
                    }
                  },
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
