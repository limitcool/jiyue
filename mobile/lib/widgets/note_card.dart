import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../models/note.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_icon.dart';

class NoteCard extends StatelessWidget {
  final NoteSummary note;
  final JiyueThemeColors colors;
  final VoidCallback onTap;

  const NoteCard({
    super.key,
    required this.note,
    required this.colors,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final modified = DateTime.fromMillisecondsSinceEpoch(note.modifiedMs);
    final isToday = DateTime.now().difference(modified).inDays == 0;
    final timeStr = isToday ? DateFormat('HH:mm').format(modified) : DateFormat('MM-dd').format(modified);

    // 确定前缀图标
    String iconName = 'file-text';
    if (note.title.contains('日记') || RegExp(r'^\d{4}-\d{2}-\d{2}').hasMatch(note.title)) {
      iconName = 'calendar';
    } else if (note.title.contains('同步') || note.title.contains('S3') || note.title.contains('WebDAV')) {
      iconName = 'link';
    } else if (note.title.contains('设计') || note.title.contains('规范') || note.title.contains('主题')) {
      iconName = 'palette';
    }

    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 20, vertical: 5),
      decoration: BoxDecoration(
        color: colors.sheet,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: colors.border, width: 1),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withAlpha(colors.isDark ? 30 : 10),
            blurRadius: 4,
            offset: const Offset(0, 1),
          ),
        ],
      ),
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          borderRadius: BorderRadius.circular(14),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.all(14.0),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                // 顶部：图标 + 标题 + 时间
                Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    JiyueIcon(iconName, color: colors.accent, size: 16),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        note.title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 14,
                          fontWeight: FontWeight.bold,
                          color: colors.textPrimary,
                        ),
                      ),
                    ),
                    const SizedBox(width: 8),
                    Text(
                      timeStr,
                      style: TextStyle(
                        fontSize: 11,
                        color: colors.textMuted,
                      ),
                    ),
                  ],
                ),
                // 中部：摘要
                if (note.preview.isNotEmpty) ...[
                  const SizedBox(height: 8),
                  Text(
                    note.preview,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 12,
                      height: 1.35,
                      color: colors.textSecondary,
                    ),
                  ),
                ],
                // 底部：标签 Chips
                if (note.tags.isNotEmpty) ...[
                  const SizedBox(height: 10),
                  Wrap(
                    spacing: 6,
                    children: note.tags.take(3).map((tag) {
                      return Container(
                        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                        decoration: BoxDecoration(
                          color: colors.raised,
                          borderRadius: BorderRadius.circular(6),
                        ),
                        child: Text(
                          tag,
                          style: TextStyle(
                            fontSize: 10,
                            color: colors.textMuted,
                          ),
                        ),
                      );
                    }).toList(),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
