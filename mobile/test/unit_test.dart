import 'dart:io';
import 'dart:typed_data';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:jiyue_mobile/bridge/vault_service.dart';
import 'package:jiyue_mobile/models/sync_config.dart';
import 'package:jiyue_mobile/services/qr_decoder.dart';
import 'package:jiyue_mobile/theme/jiyue_theme.dart';
import 'package:jiyue_mobile/theme/theme_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:zxing2/qrcode.dart';

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

  group('Jiyue Mobile Multi-Vault & Note Engine Tests', () {
    test('Empty vault starts clean without hardcoded sample notes and supports CRUD + links + backlinks', () async {
      SharedPreferences.setMockInitialValues({});
      final tempDir = Directory.systemTemp.createTempSync('jiyue_unit_test');
      final service = VaultService.testable();
      try {
        await service.init(tempDir.path);

        // Should start completely empty (no auto-generated sample notes)
        final initialNotes = await service.listNotes();
        expect(initialNotes, isEmpty);

        // Prepare nested Obsidian-like directory & hidden ignored files
        final subDir = Directory('${tempDir.path}${Platform.pathSeparator}backend');
        subDir.createSync(recursive: true);
        File('${subDir.path}${Platform.pathSeparator}README.md')
            .writeAsStringSync('# 后端目录索引\n\n参见 [[Rust 笔记]]。\n');
        File('${subDir.path}${Platform.pathSeparator}Rust 笔记.md')
            .writeAsStringSync('# Rust 笔记\n\n#Rust 返回 [[README]]。\n');

        final obsHidden = Directory('${tempDir.path}${Platform.pathSeparator}.obsidian');
        obsHidden.createSync(recursive: true);
        File('${obsHidden.path}${Platform.pathSeparator}workspace.json')
            .writeAsStringSync('{}');
        File('${obsHidden.path}${Platform.pathSeparator}ignored.md')
            .writeAsStringSync('# Should be ignored');

        // Re-init to index existing nested files via FFI / fallback
        await service.init(tempDir.path);

        final listed = await service.listNotes();
        expect(listed.length, 2);
        expect(listed.any((n) => n.path.contains('.obsidian')), isFalse);
        expect(listed.any((n) => n.path == 'backend/README.md'), isTrue);
        expect(listed.any((n) => n.path == 'backend/Rust 笔记.md'), isTrue);

        // Resolve link to folder index & subdirectory note
        final resolvedFolder = await service.resolveLink('backend/Rust 笔记.md', 'backend');
        expect(resolvedFolder, equals('backend/README.md'));
        final resolvedNote = await service.resolveLink('backend/README.md', 'Rust 笔记');
        expect(resolvedNote, equals('backend/Rust 笔记.md'));

        // Verify backlinks for backend/Rust 笔记.md
        final detail = await service.getNote('backend/Rust 笔记.md');
        expect(detail, isNotNull);
        expect(detail!.backlinks.any((b) => b.path == 'backend/README.md'), isTrue);

        // Bookmark persistence & subdirectory rename preserving folder
        final bookmarked = await service.toggleBookmark('backend/Rust 笔记.md');
        expect(bookmarked, isTrue);
        expect(service.isBookmarked('backend/Rust 笔记.md'), isTrue);

        final renamed = await service.renameNote('backend/Rust 笔记.md', 'Rust 异步');
        expect(renamed, equals('backend/Rust 异步.md'));
        expect(service.isBookmarked('backend/Rust 异步.md'), isTrue);
        expect(service.isBookmarked('backend/Rust 笔记.md'), isFalse);

        // Create & delete root note
        final created = await service.createNote(
          '单元测试笔记',
          '# 单元测试\n\n正文内容 [[Rust 异步]]',
        );
        expect(created, isNotNull);
        final saved = await service.saveNote(created!.path, '# 单元测试更新\n\n新内容');
        expect(saved, isTrue);
        final deleted = await service.deleteNote(created.path);
        expect(deleted, isTrue);
        expect(await service.getNote(created.path), isNull);
      } finally {
        service.dispose();
        try {
          tempDir.deleteSync(recursive: true);
        } catch (_) {}
      }
    });

    test('Supports multiple vaults with isolated sync configs and clean onboarding start', () async {
      SharedPreferences.setMockInitialValues({});
      final tempRoot = Directory.systemTemp.createTempSync('jiyue_multi_vault');
      final dirA = Directory('${tempRoot.path}${Platform.pathSeparator}VaultA')..createSync();
      final dirB = Directory('${tempRoot.path}${Platform.pathSeparator}VaultB')..createSync();
      File('${dirA.path}${Platform.pathSeparator}a.md').writeAsStringSync('# Note A');
      File('${dirB.path}${Platform.pathSeparator}b.md').writeAsStringSync('# Note B');
      final service = VaultService.testable();

      try {
        // Fresh startup with no saved profiles enters onboarding mode
        await service.init();
        expect(service.hasActiveVault, isFalse);
        expect(service.vaults, isEmpty);

        // Mount VaultA (pure local) and VaultB (S3 synced)
        final profA = await service.openExternalVault(dirA.path, name: 'LocalObsidian');
        expect(service.hasActiveVault, isTrue);
        expect(profA.hasRemoteSync, isFalse);

        final profB = await service.openExternalVault(dirB.path, name: 'WorkVault');
        await service.saveS3Config(
          S3Config(
            endpoint: 'https://s3.example.com',
            region: 'us-east-1',
            bucket: 'jiyue-bucket',
            accessKeyId: 'ak',
            secretAccessKey: 'sk',
            rootPrefix: 'jiyue/WorkVault/',
          ),
        );
        expect(service.activeVault!.id, equals(profB.id));
        expect(service.activeVault!.hasRemoteSync, isTrue);
        expect(service.s3Config!.rootPrefix, equals('jiyue/WorkVault/'));

        var notes = await service.listNotes();
        expect(notes.map((n) => n.path), equals(['b.md']));

        // Switch back to VaultA -> purely local again, S3 is isolated to VaultB
        await service.switchVault(profA.id);
        expect(service.activeVault!.id, equals(profA.id));
        expect(service.s3Config, isNull);
        expect(service.activeVault!.hasRemoteSync, isFalse);

        notes = await service.listNotes();
        expect(notes.map((n) => n.path), equals(['a.md']));

        // Switch back to VaultB -> unbind sync
        await service.switchVault(profB.id);
        expect(service.s3Config!.bucket, equals('jiyue-bucket'));
        await service.clearSyncConfig();
        expect(service.activeVault!.hasRemoteSync, isFalse);

        // Remove vaults cleanly back to onboarding state
        await service.removeVault(profA.id);
        await service.removeVault(profB.id);
        expect(service.hasActiveVault, isFalse);
      } finally {
        service.dispose();
        try {
          tempRoot.deleteSync(recursive: true);
        } catch (_) {}
      }
    });
  });

  group('Open-Source QR Decoder Tests', () {
    test('QrDecoder decodes PNG image bytes and YUV luma planes cleanly', () {
      const payload = '{"jiyue":"config_export","version":1,"sync":{"vault_name":"MyObsidian"}}';
      final hints = EncodeHints()
        ..put(EncodeHintType.characterSet, CharacterSetECI.UTF8);
      final qr = Encoder.encode(payload, ErrorCorrectionLevel.m, hints: hints);
      final matrix = qr.matrix!;

      const scale = 6;
      const margin = 12;
      final imgSize = matrix.width * scale + margin * 2;
      final canvas = img.Image(width: imgSize, height: imgSize, numChannels: 3);
      img.fill(canvas, color: img.ColorRgb8(255, 255, 255));

      final yPlane = Uint8List(imgSize * imgSize)..fillRange(0, imgSize * imgSize, 255);
      for (var my = 0; my < matrix.height; my++) {
        for (var mx = 0; mx < matrix.width; mx++) {
          if (matrix.get(mx, my) == 1) {
            final startX = margin + mx * scale;
            final startY = margin + my * scale;
            for (var dy = 0; dy < scale; dy++) {
              for (var dx = 0; dx < scale; dx++) {
                canvas.setPixelRgb(startX + dx, startY + dy, 0, 0, 0);
                yPlane[(startY + dy) * imgSize + (startX + dx)] = 0;
              }
            }
          }
        }
      }

      final pngBytes = Uint8List.fromList(img.encodePng(canvas));
      final decodedFromPng = QrDecoder.decodeFromImageBytes(pngBytes);
      expect(decodedFromPng, equals(payload));

      final decodedFromYuv = QrDecoder.decodeFromYuvPlane(
        yBytes: yPlane,
        width: imgSize,
        height: imgSize,
        bytesPerRow: imgSize,
      );
      expect(decodedFromYuv, equals(payload));
    });
  });

  group('Jiyue Mobile Inbox & Share Import Tests', () {
    test('Supports inbox stream capture, todo toggle, delete, and file import', () async {
      SharedPreferences.setMockInitialValues({});
      final tempDir = Directory.systemTemp.createTempSync('jiyue_inbox_test');
      final service = VaultService.testable();

      try {
        await service.openExternalVault(tempDir.path, name: 'InboxTest');

        expect(await service.appendToInbox('记录一条手机灵感 #随手记'), isTrue);
        expect(await service.appendToInbox('购买咖啡豆', asTodo: true), isTrue);

        var summary = await service.getInboxSummary();
        expect(summary.captures.length, equals(1));
        expect(summary.captures.first.content, contains('记录一条手机灵感'));
        expect(summary.todos.length, equals(1));
        expect(summary.todos.first.done, isFalse);
        expect(summary.todos.first.text, equals('购买咖啡豆'));

        final todo = summary.todos.first;
        expect(await service.toggleInboxTodo(todo.notePath, todo.lineIndex), isTrue);
        summary = await service.getInboxSummary();
        expect(summary.todos.first.done, isTrue);

        final cap = summary.captures.first;
        expect(await service.deleteInboxEntry(cap.notePath, cap.lineIndex), isTrue);
        summary = await service.getInboxSummary();
        expect(summary.captures, isEmpty);

        // Import shared image file
        final srcFile = File('${tempDir.path}${Platform.pathSeparator}share_demo.png')
          ..writeAsBytesSync([0x89, 0x50, 0x4E, 0x47]);
        final importRes = await service.importFileToInbox(
          srcFile.path,
          noteText: '来自系统的分享截图',
        );
        expect(importRes.success, isTrue);
        expect(
          File('${tempDir.path}${Platform.pathSeparator}${importRes.destRel.replaceAll('/', Platform.pathSeparator)}').existsSync(),
          isTrue,
        );
        summary = await service.getInboxSummary();
        expect(
          summary.captures.any((c) => c.content.contains('来自系统的分享截图')),
          isTrue,
        );
      } finally {
        service.dispose();
        try {
          tempDir.deleteSync(recursive: true);
        } catch (_) {}
      }
    });
  });
}
