import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'jiyue_theme.dart';

class ThemeProvider extends ChangeNotifier {
  static const _prefKey = 'jiyue_theme_id';
  JiyueThemeColors _current = JiyueThemeColors.ink; // 默认采用用户喜爱的「墨 (Ink)」

  ThemeProvider() {
    _loadTheme();
  }

  JiyueThemeColors get colors => _current;
  JiyueThemeId get themeId => _current.id;

  Future<void> _loadTheme() async {
    final prefs = await SharedPreferences.getInstance();
    final savedId = prefs.getString(_prefKey);
    if (savedId != null) {
      for (final t in JiyueThemeColors.allThemes) {
        if (t.id.name == savedId) {
          _current = t;
          notifyListeners();
          break;
        }
      }
    }
  }

  Future<void> setTheme(JiyueThemeId id) async {
    _current = JiyueThemeColors.fromId(id);
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_prefKey, id.name);
  }
}
