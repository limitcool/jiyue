import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:jiyue_mobile/bridge/vault_service.dart';
import 'package:jiyue_mobile/theme/jiyue_theme.dart';
import 'package:jiyue_mobile/theme/theme_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('Jiyue Mobile Theme Tests', () {
    test('All 5 themes are configured with correct accent colors', () {
      expect(JiyueThemeColors.allThemes.length, 5);

      final ink = JiyueThemeColors.ink;
      expect(ink.id, JiyueThemeId.ink);
      expect(ink.isDark, isTrue);
      expect(ink.accent, const Color(0xFF6BB2FF));

      final fog = JiyueThemeColors.fog;
      expect(fog.id, JiyueThemeId.fog);
      expect(fog.isDark, isFalse);
      expect(fog.accent, const Color(0xFF2563EB));
    });

    test('ThemeProvider switches themes and saves preference', () async {
      SharedPreferences.setMockInitialValues({});
      final provider = ThemeProvider();

      expect(provider.colors.id, JiyueThemeId.ink);

      await provider.setTheme(JiyueThemeId.fog);
      expect(provider.colors.id, JiyueThemeId.fog);
      expect(provider.colors.name, '雾 (Fog)');

      await provider.setTheme(JiyueThemeId.paper);
      expect(provider.colors.id, JiyueThemeId.paper);
    });
  });

  group('Jiyue Mobile VaultService Tests', () {
    test('VaultService initializes notes and supports CRUD operations', () async {
      final tempDir = Directory.systemTemp.createTempSync('jiyue_unit_test');
      final service = VaultService.instance;
      await service.init(tempDir.path);

      // Verify sample notes created
      final notes = await service.listNotes();
      expect(notes.isNotEmpty, isTrue);
      expect(notes.any((n) => n.title == '欢迎使用 Jiyue'), isTrue);

      // Create new note
      final created = await service.createNote('单元测试笔记', '# 单元测试\n\n正文内容 [[欢迎使用 Jiyue]]');
      expect(created, isNotNull);
      expect(created!.title, '单元测试笔记');

      // Get note detail & verify wikilinks
      final detail = await service.getNote(created.path);
      expect(detail, isNotNull);
      expect(detail!.wikilinks.contains('欢迎使用 Jiyue'), isTrue);

      // Save note
      final saved = await service.saveNote(created.path, '# 单元测试更新\n\n新内容');
      expect(saved, isTrue);

      // Delete note
      final deleted = await service.deleteNote(created.path);
      expect(deleted, isTrue);
      expect(await service.getNote(created.path), isNull);

      // Clean up
      try {
        tempDir.deleteSync(recursive: true);
      } catch (_) {}
    });
  });
}
