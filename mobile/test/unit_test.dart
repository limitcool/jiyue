import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:jiyue_mobile/bridge/vault_service.dart';
import 'package:jiyue_mobile/theme/jiyue_theme.dart';
import 'package:jiyue_mobile/theme/theme_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('Jiyue Mobile Theme & SSOT Token Tests', () {
    test('All 5 themes and SSOT semantic slots are correctly configured', () {
      expect(JiyueThemeColors.allThemes.length, 5);

      const ink = JiyueThemeColors.ink;
      expect(ink.id, JiyueThemeId.ink);
      expect(ink.isDark, isTrue);
      expect(ink.accent, const Color(0xFF5AA9FF));
      expect(ink.canvas, const Color(0xFF070A10));
      expect(ink.panel, const Color(0xFF0B0F17));
      expect(ink.surface, const Color(0xFF0F141E));
      expect(ink.elevated, const Color(0xFF161D2A));

      const fog = JiyueThemeColors.fog;
      expect(fog.id, JiyueThemeId.fog);
      expect(fog.isDark, isFalse);
      expect(fog.accent, const Color(0xFF2563EB));
      expect(fog.surface, const Color(0xFFFFFFFF));

      for (final theme in JiyueThemeColors.allThemes) {
        expect(theme.name.isNotEmpty, isTrue);
        expect(theme.canvas.a, greaterThan(0));
        expect(theme.panel.a, greaterThan(0));
        expect(theme.surface.a, greaterThan(0));
        expect(theme.elevated.a, greaterThan(0));
        expect(theme.ok.a, greaterThan(0));
        expect(theme.warn.a, greaterThan(0));
        expect(theme.danger.a, greaterThan(0));
      }

      // Verify shared Linear/Zed geometry & typography tokens
      expect(JiyueTokens.radiusSm, 4.0);
      expect(JiyueTokens.radiusMd, 6.0);
      expect(JiyueTokens.radiusLg, 10.0);
      expect(JiyueTokens.radiusXl, 12.0);
      expect(JiyueTokens.radiusPill, 999.0);
      expect(JiyueTokens.controlMd, 30.0);
      expect(JiyueTokens.controlLg, 36.0);
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
      final created = await service.createNote(
        '单元测试笔记',
        '# 单元测试\n\n正文内容 [[欢迎使用 Jiyue]]',
      );
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
