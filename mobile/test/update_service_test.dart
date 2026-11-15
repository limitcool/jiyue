import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:jiyue_mobile/services/update_service.dart';

class _MockRouteResponse {
  final int statusCode;
  final List<int> bodyBytes;
  final Map<String, String> headers;
  final int chunkSize;

  _MockRouteResponse({
    required this.statusCode,
    required this.bodyBytes,
    this.headers = const {},
    this.chunkSize = 0,
  });

  factory _MockRouteResponse.json(int statusCode, Map<String, dynamic> data) {
    return _MockRouteResponse(
      statusCode: statusCode,
      bodyBytes: utf8.encode(jsonEncode(data)),
    );
  }

  factory _MockRouteResponse.text(
    int statusCode,
    String text, {
    Map<String, String> headers = const {},
    int chunkSize = 0,
  }) {
    return _MockRouteResponse(
      statusCode: statusCode,
      bodyBytes: utf8.encode(text),
      headers: headers,
      chunkSize: chunkSize,
    );
  }
}

class _MockHttpClient implements HttpClient {
  final _MockRouteResponse Function(Uri url) handler;

  _MockHttpClient(this.handler);

  @override
  Duration? connectionTimeout;

  @override
  Future<HttpClientRequest> getUrl(Uri url) async {
    return _MockHttpRequest(handler(url));
  }

  @override
  void close({bool force = false}) {}

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockHttpRequest implements HttpClientRequest {
  final _MockRouteResponse route;
  @override
  bool followRedirects = true;
  @override
  final HttpHeaders headers = _MockHttpHeaders(const {});

  _MockHttpRequest(this.route);

  @override
  Future<HttpClientResponse> close() async {
    return _MockHttpResponse(route);
  }

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockHttpHeaders implements HttpHeaders {
  final Map<String, String> _map;

  _MockHttpHeaders(this._map);

  @override
  void set(String name, Object value, {bool preserveHeaderCase = false}) {}

  @override
  String? value(String name) {
    final target = name.toLowerCase();
    for (final entry in _map.entries) {
      if (entry.key.toLowerCase() == target) return entry.value;
    }
    return null;
  }

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockHttpResponse extends Stream<List<int>> implements HttpClientResponse {
  final _MockRouteResponse route;

  _MockHttpResponse(this.route);

  @override
  int get statusCode => route.statusCode;

  @override
  int get contentLength => route.bodyBytes.length;

  @override
  HttpHeaders get headers => _MockHttpHeaders(route.headers);

  @override
  StreamSubscription<List<int>> listen(
    void Function(List<int> event)? onData, {
    Function? onError,
    void Function()? onDone,
    bool? cancelOnError,
  }) {
    if (route.chunkSize <= 0 || route.bodyBytes.isEmpty) {
      return Stream.value(route.bodyBytes).listen(
        onData,
        onError: onError,
        onDone: onDone,
        cancelOnError: cancelOnError,
      );
    }
    final chunks = <List<int>>[];
    for (var i = 0; i < route.bodyBytes.length; i += route.chunkSize) {
      final end = (i + route.chunkSize).clamp(0, route.bodyBytes.length);
      chunks.add(route.bodyBytes.sublist(i, end));
    }
    return Stream.fromIterable(chunks).listen(
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

    test('parseExpandedAssetsHtml extracts unique release assets', () {
      const html = '''
        <a href="/limitcool/jiyue/releases/download/v0.3.5/jiyue-android-arm64.apk">arm64</a>
        <a href="/limitcool/jiyue/releases/download/v0.3.5/jiyue-android-arm64.apk">dup</a>
        <a href="/limitcool/jiyue/releases/download/v0.3.5/jiyue-windows-x64.zip?download=1">win</a>
      ''';
      final assets = UpdateService.parseExpandedAssetsHtml(
        html,
        'limitcool',
        'jiyue',
        'v0.3.5',
      );
      expect(assets.length, equals(2));
      expect(assets.first.name, equals('jiyue-android-arm64.apk'));
      expect(
        assets.first.downloadUrl,
        equals('https://github.com/limitcool/jiyue/releases/download/v0.3.5/jiyue-android-arm64.apk'),
      );
    });
  });

  group('UpdateService GitHub API & Fallback', () {
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
            'browser_download_url':
                'https://github.com/limitcool/jiyue/releases/download/v0.3.3/jiyue-android-arm64.apk',
            'size': 25000000,
          }
        ]
      };

      final client = _MockHttpClient(
        (_) => _MockRouteResponse.json(200, mockData),
      );
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

    test('falls back to web release endpoints when REST API returns 403', () async {
      final client = _MockHttpClient((uri) {
        if (uri.host == 'api.github.com') {
          return _MockRouteResponse.json(403, {'message': 'API rate limit exceeded'});
        }
        if (uri.path.endsWith('/releases/latest')) {
          return _MockRouteResponse.text(
            302,
            '',
            headers: {
              'location': 'https://github.com/limitcool/jiyue/releases/tag/v0.3.5',
            },
          );
        }
        if (uri.path.contains('/releases/expanded_assets/v0.3.5')) {
          return _MockRouteResponse.text(
            200,
            '<a href="/limitcool/jiyue/releases/download/v0.3.5/jiyue-android-arm64.apk">apk</a>',
          );
        }
        return _MockRouteResponse.text(404, 'not found');
      });

      final result = await UpdateService.checkUpdate(
        currentVersion: '0.3.4',
        client: client,
      );

      expect(result.hasUpdate, isTrue);
      expect(result.latestVersion, equals('0.3.5'));
      expect(result.apkAsset, isNotNull);
      expect(result.apkAsset!.name, equals('jiyue-android-arm64.apk'));
      expect(
        result.releaseUrl,
        equals('https://github.com/limitcool/jiyue/releases/tag/v0.3.5'),
      );
    });
  });

  group('UpdateService APK Download', () {
    test('streams APK with progress and reuses verified cache', () async {
      final tmpDir = await Directory.systemTemp.createTemp('jiyue_apk_dl_');
      try {
        final payload = List<int>.generate(1024, (i) => i % 256);
        var requests = 0;
        final client = _MockHttpClient((_) {
          requests++;
          return _MockRouteResponse(
            statusCode: 200,
            bodyBytes: payload,
            chunkSize: 256,
          );
        });

        final check = UpdateCheckResult(
          hasUpdate: true,
          currentVersion: '0.3.4',
          latestVersion: '0.3.5',
          releaseName: 'v0.3.5',
          releaseNotes: '',
          releaseUrl: 'https://github.com/limitcool/jiyue/releases/tag/v0.3.5',
          apkAsset: ReleaseAsset(
            name: 'jiyue-android-arm64.apk',
            downloadUrl: 'https://example.invalid/jiyue-android-arm64.apk',
            size: payload.length,
          ),
        );

        final progressCalls = <(int, int)>[];
        final file = await UpdateService.downloadApk(
          check,
          client: client,
          targetDir: tmpDir,
          onProgress: (r, t) => progressCalls.add((r, t)),
        );

        expect(requests, equals(1));
        expect(file.existsSync(), isTrue);
        expect(file.readAsBytesSync(), equals(payload));
        expect(progressCalls.length, equals(4));
        expect(progressCalls.last, equals((payload.length, payload.length)));

        // Reusing verified cached APK avoids duplicate network request
        final reused = await UpdateService.downloadApk(
          check,
          client: client,
          targetDir: tmpDir,
          onProgress: (_, __) {},
        );
        expect(requests, equals(1));
        expect(reused.path, equals(file.path));
      } finally {
        tmpDir.deleteSync(recursive: true);
      }
    });

    test('cancellation removes partial download file', () async {
      final tmpDir = await Directory.systemTemp.createTemp('jiyue_apk_cancel_');
      try {
        final payload = List<int>.generate(1024, (i) => i % 256);
        final token = UpdateDownloadToken();
        final client = _MockHttpClient(
          (_) => _MockRouteResponse(
            statusCode: 200,
            bodyBytes: payload,
            chunkSize: 256,
          ),
        );
        final check = UpdateCheckResult(
          hasUpdate: true,
          currentVersion: '0.3.4',
          latestVersion: '0.3.5',
          releaseName: 'v0.3.5',
          releaseNotes: '',
          releaseUrl: 'https://github.com/limitcool/jiyue/releases/tag/v0.3.5',
          apkAsset: ReleaseAsset(
            name: 'jiyue-android-arm64.apk',
            downloadUrl: 'https://example.invalid/jiyue-android-arm64.apk',
            size: payload.length,
          ),
        );

        await expectLater(
          UpdateService.downloadApk(
            check,
            cancelToken: token,
            client: client,
            targetDir: tmpDir,
            onProgress: (r, _) {
              if (r >= 256) token.cancel();
            },
          ),
          throwsA(isA<UpdateCancelledException>()),
        );

        expect(tmpDir.listSync(), isEmpty);
      } finally {
        tmpDir.deleteSync(recursive: true);
      }
    });
  });
}
