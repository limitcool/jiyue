import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../models/note.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_controls.dart';
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
    final timeStr = isToday
        ? DateFormat('HH:mm').format(modified)
        : DateFormat('MM-dd').format(modified);

    String iconName = 'file-text';
    if (note.title.contains('日记') ||
        RegExp(r'^\d{4}-\d{2}-\d{2}').hasMatch(note.title)) {
      iconName = 'calendar';
    } else if (note.title.contains('同步') ||
        note.title.contains('S3') ||
        note.title.contains('WebDAV')) {
      iconName = 'link';
    } else if (note.title.contains('设计') ||
        note.title.contains('规范') ||
        note.title.contains('主题')) {
      iconName = 'palette';
    }

    return JiyueSectionCard(
      colors: colors,
      margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
      padding: const EdgeInsets.all(12),
      onTap: onTap,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              JiyueIcon(iconName, color: colors.accent, size: 15),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  note.title,
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
              Text(
                timeStr,
                style: TextStyle(
                  fontSize: JiyueTokens.fontXs,
                  color: colors.textMuted,
                ),
              ),
            ],
          ),
          if (note.preview.isNotEmpty) ...[
            const SizedBox(height: 6),
            Text(
              note.preview,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: JiyueTokens.fontSm,
                height: 1.4,
                color: colors.textSecondary,
              ),
            ),
          ],
          if (note.tags.isNotEmpty) ...[
            const SizedBox(height: 8),
            Wrap(
              spacing: 6,
              runSpacing: 4,
              children: note.tags.take(3).map((tag) {
                return JiyueTagChip(colors: colors, label: tag);
              }).toList(),
            ),
          ],
        ],
      ),
    );
  }
}
