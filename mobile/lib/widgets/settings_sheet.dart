import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../bridge/vault_service.dart';
import '../theme/jiyue_theme.dart';
import '../theme/theme_provider.dart';
import 'jiyue_icon.dart';

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

              // 三、S3 / WebDAV 云同步设置 (Sync)
              Text(
                '云端同步配置 (Sync)',
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
                        Text('协议类型: S3 (Cloudflare R2 / MinIO)',
                            style: TextStyle(fontSize: 11, color: colors.textSecondary)),
                        TextButton(
                          onPressed: () {
                            ScaffoldMessenger.of(context).showSnackBar(
                              const SnackBar(content: Text('与 PC 端共享相同存储桶配置')),
                            );
                          },
                          child: Text('修改配置', style: TextStyle(fontSize: 11, color: colors.accent)),
                        ),
                      ],
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
