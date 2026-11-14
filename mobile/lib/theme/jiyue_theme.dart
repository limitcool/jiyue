import 'package:flutter/material.dart';

enum JiyueThemeId {
  ink,      // 墨 (暗黑夜间，冰蓝强调)
  fog,      // 雾 (柔冷清透，电光蓝强调)
  paper,    // 纸 (温润宣纸，松石绿强调)
  moss,     // 苔 (林间暗苔，冷调青绿)
  graphite, // 石墨 (工业沉稳，暖橙焦点)
}

class JiyueThemeColors {
  final JiyueThemeId id;
  final String name;
  final String description;
  final bool isDark;

  final Color chrome;        // 窗口根底色
  final Color sheet;         // 笔记卡片/正文底色
  final Color raised;        // 凸起控件/输入框底色
  final Color border;        // 边框与分割线色
  final Color accent;        // 强调色 (链接、高亮按钮)
  final Color textPrimary;   // 标题与主正文字色
  final Color textSecondary; // 次要说明文字
  final Color textMuted;     // 占位/时间弱化文字
  final Color onAccent;      // 强调色上的反白/反黑文字

  const JiyueThemeColors({
    required this.id,
    required this.name,
    required this.description,
    required this.isDark,
    required this.chrome,
    required this.sheet,
    required this.raised,
    required this.border,
    required this.accent,
    required this.textPrimary,
    required this.textSecondary,
    required this.textMuted,
    required this.onAccent,
  });

  // 墨 (Ink · 极客夜间暗黑)
  static const ink = JiyueThemeColors(
    id: JiyueThemeId.ink,
    name: '墨 (Ink)',
    description: '深夜深黑与极客冰蓝 · OLED 纯粹夜间沉浸',
    isDark: true,
    chrome: Color(0xFF070A10),
    sheet: Color(0xFF0E131B),
    raised: Color(0xFF151B25),
    border: Color(0xFF222B3A),
    accent: Color(0xFF6BB2FF),
    textPrimary: Color(0xFFE2E8F1),
    textSecondary: Color(0xFF9BA8BB),
    textMuted: Color(0xFF7A889D),
    onAccent: Color(0xFF05162B),
  );

  // 雾 (Fog · 现代柔冷浅色)
  static const fog = JiyueThemeColors(
    id: JiyueThemeId.fog,
    name: '雾 (Fog)',
    description: '柔冷清透浅白与电光蓝 · 日间阅读清爽锐利',
    isDark: false,
    chrome: Color(0xFFF1F5F9),
    sheet: Color(0xFFFFFFFF),
    raised: Color(0xFFE2E8F0),
    border: Color(0xFFCBD5E1),
    accent: Color(0xFF2563EB),
    textPrimary: Color(0xFF172033),
    textSecondary: Color(0xFF4F5B73),
    textMuted: Color(0xFF667389),
    onAccent: Color(0xFFFFFFFF),
  );

  // 纸 (Paper · 暖调宣纸)
  static const paper = JiyueThemeColors(
    id: JiyueThemeId.paper,
    name: '纸 (Paper)',
    description: '温润宣纸墨韵与松石绿 · 拟物长篇专注阅读',
    isDark: false,
    chrome: Color(0xFFEFEAE1),
    sheet: Color(0xFFF8F6F0),
    raised: Color(0xFFEDE8DC),
    border: Color(0xFFDCD6C7),
    accent: Color(0xFF1B7566),
    textPrimary: Color(0xFF2B2823),
    textSecondary: Color(0xFF5F5A50),
    textMuted: Color(0xFF736D63),
    onAccent: Color(0xFFFFFFFF),
  );

  // 苔 (Moss · 林间青绿)
  static const moss = JiyueThemeColors(
    id: JiyueThemeId.moss,
    name: '苔 (Moss)',
    description: '林间暗苔与青绿微光 · 静谧平和护眼',
    isDark: true,
    chrome: Color(0xFF0C110F),
    sheet: Color(0xFF121916),
    raised: Color(0xFF1A2420),
    border: Color(0xFF26352F),
    accent: Color(0xFF7FD1A8),
    textPrimary: Color(0xFFE0EAE5),
    textSecondary: Color(0xFF96A8A0),
    textMuted: Color(0xFF6B7E76),
    onAccent: Color(0xFF0B1F17),
  );

  // 石墨 (Graphite · 工业石墨与暖橙)
  static const graphite = JiyueThemeColors(
    id: JiyueThemeId.graphite,
    name: '石墨 (Graphite)',
    description: '工业石墨纯黑与暖橙焦点 · 沉稳内敛',
    isDark: true,
    chrome: Color(0xFF0D0D0D),
    sheet: Color(0xFF161616),
    raised: Color(0xFF202020),
    border: Color(0xFF2D2D2D),
    accent: Color(0xFFFF7A59),
    textPrimary: Color(0xFFECECEC),
    textSecondary: Color(0xFFA6A6A6),
    textMuted: Color(0xFF767676),
    onAccent: Color(0xFF1C0A04),
  );

  static const List<JiyueThemeColors> allThemes = [ink, fog, paper, moss, graphite];

  static JiyueThemeColors fromId(JiyueThemeId id) {
    switch (id) {
      case JiyueThemeId.ink:
        return ink;
      case JiyueThemeId.fog:
        return fog;
      case JiyueThemeId.paper:
        return paper;
      case JiyueThemeId.moss:
        return moss;
      case JiyueThemeId.graphite:
        return graphite;
    }
  }
}
