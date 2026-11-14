import 'package:flutter/material.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_icon.dart';

class JiyueBottomBar extends StatelessWidget {
  final int currentIndex;
  final JiyueThemeColors colors;
  final ValueChanged<int> onTabSelected;

  const JiyueBottomBar({
    super.key,
    required this.currentIndex,
    required this.colors,
    required this.onTabSelected,
  });

  @override
  Widget build(BuildContext context) {
    final tabs = [
      ('folder', '笔记库'),
      ('calendar', '速记'),
      ('search', '检索'),
      ('settings', '设置'),
    ];

    return Container(
      height: 64,
      decoration: BoxDecoration(
        color: colors.chrome,
        border: Border(
          top: BorderSide(color: colors.border, width: 1),
        ),
      ),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceAround,
        children: List.generate(tabs.length, (index) {
          final isSelected = currentIndex == index;
          final (iconName, label) = tabs[index];
          final color = isSelected ? colors.accent : colors.textMuted;

          return InkWell(
            onTap: () => onTabSelected(index),
            borderRadius: BorderRadius.circular(12),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  JiyueIcon(iconName, color: color, size: 22),
                  const SizedBox(height: 3),
                  Text(
                    label,
                    style: TextStyle(
                      fontSize: 10,
                      fontWeight: isSelected ? FontWeight.bold : FontWeight.normal,
                      color: color,
                    ),
                  ),
                ],
              ),
            ),
          );
        }),
      ),
    );
  }
}
