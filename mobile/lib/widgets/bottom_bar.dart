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
    const tabs = [
      ('folder', '笔记库'),
      ('calendar', '速记'),
      ('search', '检索'),
      ('settings', '设置'),
    ];

    return Container(
      decoration: BoxDecoration(
        color: colors.panel,
        border: Border(
          top: BorderSide(color: colors.borderSubtle, width: 1),
        ),
      ),
      child: SafeArea(
        top: false,
        child: SizedBox(
          height: 54,
          child: Row(
            children: List.generate(tabs.length, (index) {
              final bool isSelected = currentIndex == index;
              final (iconName, label) = tabs[index];
              final Color fg = isSelected ? colors.accent : colors.textMuted;

              return Expanded(
                child: Material(
                  color: Colors.transparent,
                  child: InkWell(
                    onTap: () => onTabSelected(index),
                    borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
                    splashColor: colors.active,
                    highlightColor: colors.hover,
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Container(
                          width: 24,
                          height: 2,
                          decoration: BoxDecoration(
                            color: isSelected ? colors.accent : Colors.transparent,
                            borderRadius: BorderRadius.circular(
                              JiyueTokens.radiusPill,
                            ),
                          ),
                        ),
                        const SizedBox(height: 5),
                        JiyueIcon(iconName, color: fg, size: 19),
                        const SizedBox(height: 3),
                        Text(
                          label,
                          style: TextStyle(
                            fontSize: JiyueTokens.fontXs,
                            fontWeight:
                                isSelected ? FontWeight.w600 : FontWeight.w400,
                            color: fg,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              );
            }),
          ),
        ),
      ),
    );
  }
}
