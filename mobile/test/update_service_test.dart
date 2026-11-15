import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:jiyue_mobile/services/update_service.dart';

class _MockUpdateHttpClient implements HttpClient {
  final Map<String, dynamic> responseJson;
  final int statusCode;

  _MockUpdateHttpClient({required this.responseJson, required this.statusCode});

  @override
  Future<HttpClientRequest> getUrl(Uri url) async {
    return _MockUpdateHttpRequest(responseJson: responseJson, statusCode: statusCode);
  }

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockUpdateHttpRequest implements HttpClientRequest {
  final Map<String, dynamic> responseJson;
  final int statusCode;
  @override
  final HttpHeaders headers = _MockUpdateHttpHeaders();

  _MockUpdateHttpRequest({required this.responseJson, required this.statusCode});

  @override
  Future<HttpClientResponse> close() async {
    return _MockUpdateHttpResponse(responseJson: responseJson, statusCode: statusCode);
  }

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockUpdateHttpHeaders implements HttpHeaders {
  @override
  void set(String name, Object value, {bool preserveHeaderCase = false}) {}

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockUpdateHttpResponse extends Stream<List<int>> implements HttpClientResponse {
  final Map<String, dynamic> responseJson;
  @override
  final int statusCode;

  _MockUpdateHttpResponse({required this.responseJson, required this.statusCode});

  @override
  StreamSubscription<List<int>> listen(void Function(List<int> event)? onData,
      {Function? onError, void Function()? onDone, bool? cancelOnError}) {
    final bytes = utf8.encode(jsonEncode(responseJson));
    return Stream.value(bytes).listen(
      onData,
      onError: onError,
      onDone: onDone,
      cancelOnError: cancelOnError,
    );
  }

  @override
  void noSuchMethod(Invocation invocation) {}
}

void main() {
  group('UpdateService Version Parsing & Logic', () {
    test('parseSemver handles clean and prefixed versions', () {
      expect(UpdateService.parseSemver('0.3.1'), equals([0, 3, 1]));
      expect(UpdateService.parseSemver('v0.3.2'), equals([0, 3, 2]));
      expect(UpdateService.parseSemver('V1.0.0-rc1'), equals([1, 0, 0]));
      expect(UpdateService.parseSemver('invalid'), isNull);
    });

    test('isNewer returns true only for newer semvers', () {
      expect(UpdateService.isNewer('v0.3.3', '0.3.2'), isTrue);
      expect(UpdateService.isNewer('v1.0.0', '0.9.9'), isTrue);
      expect(UpdateService.isNewer('v0.3.2', '0.3.2'), isFalse);
      expect(UpdateService.isNewer('v0.3.1', '0.3.2'), isFalse);
    });

    test('pickMobileAsset selects correct Android APK', () {
      final assets = [
        ReleaseAsset(name: 'jiyue-windows-x64.msi', downloadUrl: 'https://...', size: 100),
        ReleaseAsset(name: 'jiyue-macos-arm64.dmg', downloadUrl: 'https://...', size: 100),
        ReleaseAsset(name: 'jiyue-mobile-universal.apk', downloadUrl: 'https://.../universal.apk', size: 200),
        ReleaseAsset(name: 'jiyue-android-arm64-v8a.apk', downloadUrl: 'https://.../arm64.apk', size: 300),
      ];

      final picked = UpdateService.pickMobileAsset(assets);
      expect(picked, isNotNull);
      expect(picked!.name, equals('jiyue-android-arm64-v8a.apk'));
      expect(picked.downloadUrl, equals('https://.../arm64.apk'));
    });

    test('formatFileSize formats bytes to MB/KB correctly', () {
      expect(UpdateService.formatFileSize(500), equals('500 B'));
      expect(UpdateService.formatFileSize(2048), equals('2.0 KB'));
      expect(UpdateService.formatFileSize(15 * 1024 * 1024), equals('15.0 MB'));
    });
  });

  group('UpdateService GitHub API Mock', () {
    test('parses release info and detects available update', () async {
      final mockData = {
        'tag_name': 'v0.3.3',
        'name': 'Jiyue v0.3.3 Release',
        'body': '## What\'s new\n- AI polish support\n- Better sync',
        'published_at': '2026-11-15T03:00:00Z',
        'html_url': 'https://github.com/limitcool/jiyue/releases/tag/v0.3.3',
        'assets': [
          {
            'name': 'jiyue-android-arm64.apk',
            'browser_download_url': 'https://github.com/limitcool/jiyue/releases/download/v0.3.3/jiyue-android-arm64.apk',
            'size': 25000000,
          }
        ]
      };

      final client = _MockUpdateHttpClient(responseJson: mockData, statusCode: 200);
      final result = await UpdateService.checkUpdate(
        currentVersion: '0.3.2',
        client: client,
      );

      expect(result.hasUpdate, isTrue);
      expect(result.latestVersion, equals('0.3.3'));
      expect(result.releaseNotes, contains('AI polish support'));
      expect(result.apkAsset, isNotNull);
      expect(result.apkAsset!.name, equals('jiyue-android-arm64.apk'));
    });
  });
}
