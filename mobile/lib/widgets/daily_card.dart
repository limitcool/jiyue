import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../theme/jiyue_theme.dart';
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
    final weekdayNames = ['星期一', '星期二', '星期三', '星期四', '星期五', '星期六', '星期日'];
    final weekdayStr = weekdayNames[now.weekday - 1];

    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 20, vertical: 8),
      padding: const EdgeInsets.fromLTRB(16, 14, 16, 14),
      decoration: BoxDecoration(
        color: colors.sheet,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(color: colors.accent.withAlpha(50), width: 1),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withAlpha(colors.isDark ? 40 : 15),
            blurRadius: 8,
            offset: const Offset(0, 2),
          ),
        ],
      ),
      child: Row(
        children: [
          // 左侧竖向强调色彩条
          Container(
            width: 4,
            height: 48,
            decoration: BoxDecoration(
              color: colors.accent,
              borderRadius: BorderRadius.circular(2),
            ),
          ),
          const SizedBox(width: 12),
          // 官方日历图标
          JiyueIcon('calendar-event', color: colors.accent, size: 22),
          const SizedBox(width: 12),
          // 日期与说明
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '今日速记 · $dateStr $weekdayStr',
                  style: TextStyle(
                    fontSize: 13,
                    fontWeight: FontWeight.bold,
                    color: colors.accent,
                  ),
                ),
                const SizedBox(height: 4),
                Text(
                  '点击即刻无感追加灵感便签...',
                  style: TextStyle(
                    fontSize: 11,
                    color: colors.textSecondary,
                  ),
                ),
              ],
            ),
          ),
          // 速记按钮胶囊
          InkWell(
            onTap: onTapRecord,
            borderRadius: BorderRadius.circular(16),
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              decoration: BoxDecoration(
                color: colors.accent.withAlpha(35),
                borderRadius: BorderRadius.circular(16),
              ),
              child: Row(
                children: [
                  JiyueIcon('pencil', color: colors.accent, size: 14),
                  const SizedBox(width: 4),
                  Text(
                    '速记',
                    style: TextStyle(
                      fontSize: 12,
                      fontWeight: FontWeight.bold,
                      color: colors.accent,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
