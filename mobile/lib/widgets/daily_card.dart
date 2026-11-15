import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_controls.dart';
import 'jiyue_icon.dart';

class DailyHeroCard extends StatelessWidget {
  final JiyueThemeColors colors;
  final VoidCallback onTapRecord;

  const DailyHeroCard({
    super.key,
    required this.colors,
    required this.onTapRecord,
  });

  @override
  Widget build(BuildContext context) {
    final now = DateTime.now();
    final dateStr = DateFormat('yyyy-MM-dd').format(now);
    const weekdayNames = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];
    final weekdayStr = weekdayNames[now.weekday - 1];

    return JiyueSectionCard(
      colors: colors,
      margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      child: Row(
        children: [
          Container(
            width: 3,
            height: 36,
            decoration: BoxDecoration(
              color: colors.accent,
              borderRadius: BorderRadius.circular(JiyueTokens.radiusPill),
            ),
          ),
          const SizedBox(width: 10),
          Container(
            width: 30,
            height: 30,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: colors.accentSoft,
              borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
            ),
            child: JiyueIcon('calendar-event', color: colors.accent, size: 16),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '今日速记 · $dateStr $weekdayStr',
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
                  '点击右侧按钮立即追加灵感便签',
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
            label: '速记',
            icon: 'pencil',
            kind: JiyueButtonKind.secondary,
            size: JiyueControlSize.sm,
            onPressed: onTapRecord,
          ),
        ],
      ),
    );
  }
}
