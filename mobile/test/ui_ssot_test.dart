import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:jiyue_mobile/models/note.dart';
import 'package:jiyue_mobile/services/ai_service.dart';
import 'package:jiyue_mobile/theme/jiyue_theme.dart';
import 'package:jiyue_mobile/widgets/bottom_bar.dart';
import 'package:jiyue_mobile/widgets/daily_card.dart';
import 'package:jiyue_mobile/widgets/jiyue_controls.dart';
import 'package:jiyue_mobile/widgets/note_card.dart';

Directory _resolveMobileRoot() {
  final cwd = Directory.current;
  if (File('${cwd.path}${Platform.pathSeparator}pubspec.yaml').existsSync()) {
    return cwd;
  }
  final nested = Directory('${cwd.path}${Platform.pathSeparator}mobile');
  if (File('${nested.path}${Platform.pathSeparator}pubspec.yaml')
      .existsSync()) {
    return nested;
  }
  throw StateError('Cannot locate mobile root from ${cwd.path}');
}

String _stem(FileSystemEntity entity) {
  final name = entity.uri.pathSegments.where((s) => s.isNotEmpty).last;
  return name.endsWith('.svg') ? name.substring(0, name.length - 4) : name;
}

void main() {
  final mobileRoot = _resolveMobileRoot();
  final libDir = Directory('${mobileRoot.path}${Platform.pathSeparator}lib');
  final iconsDir = Directory(
    '${mobileRoot.path}${Platform.pathSeparator}assets${Platform.pathSeparator}icons',
  );

  group('Mobile SSOT Static Audit & Asset Integrity', () {
    test('All mobile SVG icons are valid and synced with desktop', () {
      expect(iconsDir.existsSync(), isTrue);
      final svgFiles = iconsDir
          .listSync()
          .whereType<File>()
          .where((f) => f.path.endsWith('.svg'))
          .toList();
      expect(svgFiles, isNotEmpty);

      final mobileNames = <String>{};
      for (final file in svgFiles) {
        final content = file.readAsStringSync();
        expect(content.trim().isNotEmpty, isTrue, reason: '${file.path} empty');
        expect(
          content.contains('<svg'),
          isTrue,
          reason: '${file.path} missing <svg',
        );
        mobileNames.add(_stem(file));
      }

      // Cross-check parity with desktop icon assets when running inside monorepo
      final desktopIconsDir = Directory(
        '${mobileRoot.parent.path}${Platform.pathSeparator}crates${Platform.pathSeparator}jiyue-app${Platform.pathSeparator}assets${Platform.pathSeparator}icons',
      );
      if (desktopIconsDir.existsSync()) {
        final desktopNames = desktopIconsDir
            .listSync()
            .whereType<File>()
            .where((f) => f.path.endsWith('.svg'))
            .map(_stem)
            .toSet();
        expect(
          mobileNames,
          equals(desktopNames),
          reason: 'Mobile and desktop SVG icon sets diverged',
        );
      }
    });

    test('Zero missing SVG icon references across mobile/lib', () {
      final availableIcons = iconsDir
          .listSync()
          .whereType<File>()
          .where((f) => f.path.endsWith('.svg'))
          .map(_stem)
          .toSet();

      final referenced = <String, List<String>>{};
      final jiyueIconPattern = RegExp(
        r'''JiyueIcon\(\s*['"]([a-z0-9-]+)['"]''',
      );
      final namedIconPattern = RegExp(
        r'''(?:icon|prefixIcon)\s*:\s*['"]([a-z0-9-]+)['"]''',
      );

      for (final file
          in libDir.listSync(recursive: true).whereType<File>()) {
        if (!file.path.endsWith('.dart')) continue;
        final text = file.readAsStringSync();
        for (final m in jiyueIconPattern.allMatches(text)) {
          referenced
              .putIfAbsent(m.group(1)!, () => [])
              .add(file.path);
        }
        for (final m in namedIconPattern.allMatches(text)) {
          referenced
              .putIfAbsent(m.group(1)!, () => [])
              .add(file.path);
        }
      }

      for (final style in PolishStyle.values) {
        referenced.putIfAbsent(style.icon, () => []).add('PolishStyle.${style.name}');
      }

      final missing = referenced.keys
          .where((name) => !availableIcons.contains(name))
          .toList()
        ..sort();
      expect(
        missing,
        isEmpty,
        reason:
            'Missing SVG assets for icons: $missing (${missing.map((k) => '$k -> ${referenced[k]}').join('; ')})',
      );
    });

    test(
      'Zero hardcoded Color(0x..), Colors.*, literal radii, Material Icons, or UI emojis in non-generated lib files',
      () {
        final colorHexPattern = RegExp(r'\bColor\(\s*0x[0-9a-fA-F]+\s*\)');
        final colorsClassPattern = RegExp(
          r'\bColors\.(?!transparent\b)[a-zA-Z0-9_]+',
        );
        final literalRadiusPattern = RegExp(
          r'\b(?:BorderRadius|Radius)\.circular\(\s*\d',
        );
        final materialIconPattern = RegExp(r'\bIcons\.[a-zA-Z0-9_]+');
        final emojiPattern = RegExp(
          r'[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]',
          unicode: true,
        );

        final violations = <String>[];

        for (final file
            in libDir.listSync(recursive: true).whereType<File>()) {
          if (!file.path.endsWith('.dart')) continue;
          final normalized = file.path.replaceAll('\\', '/');
          // Skip generated token definitions file
          if (normalized.endsWith('/lib/theme/jiyue_theme.dart')) continue;

          final lines = file.readAsLinesSync();
          for (var i = 0; i < lines.length; i++) {
            final line = lines[i];
            final loc = '$normalized:${i + 1}';
            if (colorHexPattern.hasMatch(line)) {
              violations.add('$loc: hardcoded Color(0x...) -> ${line.trim()}');
            }
            if (colorsClassPattern.hasMatch(line)) {
              violations.add('$loc: hardcoded Colors.* -> ${line.trim()}');
            }
            if (literalRadiusPattern.hasMatch(line)) {
              violations.add('$loc: hardcoded circular radius -> ${line.trim()}');
            }
            if (materialIconPattern.hasMatch(line)) {
              violations.add('$loc: Material Icons.* -> ${line.trim()}');
            }
            if (emojiPattern.hasMatch(line)) {
              violations.add('$loc: UI Emoji -> ${line.trim()}');
            }
          }
        }

        expect(
          violations,
          isEmpty,
          reason: 'SSOT static audit violations:\n${violations.join('\n')}',
        );
      },
    );
  });

  group('Mobile SSOT Controls Widget Tests', () {
    Widget wrapWithMaterial(Widget child, JiyueThemeColors colors) {
      return MaterialApp(
        home: Scaffold(
          backgroundColor: colors.canvas,
          body: Center(child: child),
        ),
      );
    }

    testWidgets(
      'Core SSOT controls render and handle interactions across Ink and Fog themes',
      (tester) async {
        for (final colors in [JiyueThemeColors.ink, JiyueThemeColors.fog]) {
          var buttonTapped = false;
          var switchValue = false;
          var selectedSeg = 0;
          var chipTapped = false;

          await tester.pumpWidget(
            wrapWithMaterial(
              StatefulBuilder(
                builder: (context, setState) {
                  return SingleChildScrollView(
                    padding: const EdgeInsets.all(16),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        JiyueButton(
                          colors: colors,
                          label: '操作按钮',
                          icon: 'check',
                          kind: JiyueButtonKind.primary,
                          onPressed: () => setState(() => buttonTapped = true),
                        ),
                        const SizedBox(height: 8),
                        JiyueChoiceChip(
                          colors: colors,
                          label: '全部',
                          icon: 'hash',
                          selected: chipTapped,
                          onTap: () => setState(() => chipTapped = true),
                        ),
                        const SizedBox(height: 8),
                        JiyueSegmented(
                          colors: colors,
                          items: const [('阅读', 'eye'), ('编辑', 'pencil')],
                          selectedIndex: selectedSeg,
                          onChanged: (i) => setState(() => selectedSeg = i),
                        ),
                        const SizedBox(height: 8),
                        JiyueSwitchRow(
                          colors: colors,
                          title: '自动同步',
                          value: switchValue,
                          onChanged: (v) => setState(() => switchValue = v),
                        ),
                        const SizedBox(height: 8),
                        JiyueTextField(
                          colors: colors,
                          label: '端点地址',
                          hintText: 'https://example.com',
                          prefixIcon: 'search',
                        ),
                        const SizedBox(height: 8),
                        JiyueBanner(
                          colors: colors,
                          tone: JiyueBadgeTone.ok,
                          message: '连接测试通过',
                        ),
                        const SizedBox(height: 8),
                        JiyueEmptyState(
                          colors: colors,
                          icon: 'file-text',
                          title: '暂无笔记',
                        ),
                      ],
                    ),
                  );
                },
              ),
              colors,
            ),
          );
          await tester.pumpAndSettle();

          await tester.tap(find.text('操作按钮'));
          await tester.tap(find.text('全部'));
          await tester.tap(find.text('编辑'));
          await tester.tap(find.text('自动同步'));
          await tester.pump();

          expect(buttonTapped, isTrue);
          expect(chipTapped, isTrue);
          expect(selectedSeg, 1);
          expect(switchValue, isTrue);
        }
      },
    );

    testWidgets(
      'DailyHeroCard, NoteCard, and JiyueBottomBar render cleanly and fire callbacks',
      (tester) async {
        const colors = JiyueThemeColors.ink;
        var quickLogTapped = false;
        var noteTapped = false;
        var tabIndex = 0;

        final sampleNote = NoteSummary(
          id: '1',
          title: '架构设计记录',
          path: '架构设计记录.md',
          preview: '统一使用 JiyueTokens 与语义色板驱动双端界面。',
          tags: ['#架构', '#设计'],
          modifiedMs: DateTime.now().millisecondsSinceEpoch,
        );

        await tester.pumpWidget(
          MaterialApp(
            home: Scaffold(
              backgroundColor: colors.canvas,
              body: Column(
                children: [
                  DailyHeroCard(
                    colors: colors,
                    onTapRecord: () => quickLogTapped = true,
                  ),
                  NoteCard(
                    note: sampleNote,
                    colors: colors,
                    onTap: () => noteTapped = true,
                  ),
                ],
              ),
              bottomNavigationBar: JiyueBottomBar(
                currentIndex: tabIndex,
                colors: colors,
                onTabSelected: (i) => tabIndex = i,
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.text('速记').first);
        await tester.tap(find.text('架构设计记录'));
        await tester.tap(find.text('设置'));
        await tester.pump();

        expect(quickLogTapped, isTrue);
        expect(noteTapped, isTrue);
        expect(tabIndex, 3);
      },
    );
  });
}
