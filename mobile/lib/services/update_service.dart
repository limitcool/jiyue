import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/services.dart';

const String defaultRepoOwner = 'limitcool';
const String defaultRepoName = 'jiyue';
const String currentAppVersion = '0.3.7';
const bool kIsFossBuild = bool.fromEnvironment('FOSS_BUILD', defaultValue: false);
const Duration _networkTimeout = Duration(seconds: 12);
const MethodChannel _updaterChannel =
    MethodChannel('cool.jiyue.jiyue_mobile/app_updater');

class UpdateCancelledException implements Exception {
  @override
  String toString() => '已取消下载';
}

class UpdateDownloadToken {
  bool _cancelled = false;

  void cancel() {
    _cancelled = true;
  }

  bool get isCancelled => _cancelled;
}

enum ApkInstallResult {
  launched,
  permissionRequired,
}

class ReleaseAsset {
  final String name;
  final String downloadUrl;
  final int size;

  ReleaseAsset({
    required this.name,
    required this.downloadUrl,
    required this.size,
  });

  factory ReleaseAsset.fromJson(Map<String, dynamic> json) {
    return ReleaseAsset(
      name: json['name'] as String? ?? '',
      downloadUrl: json['browser_download_url'] as String? ?? '',
      size: (json['size'] as num?)?.toInt() ?? 0,
    );
  }
}

class UpdateCheckResult {
  final bool hasUpdate;
  final String currentVersion;
  final String latestVersion;
  final String releaseName;
  final String releaseNotes;
  final String? publishedAt;
  final ReleaseAsset? apkAsset;
  final String releaseUrl;

  UpdateCheckResult({
    required this.hasUpdate,
    required this.currentVersion,
    required this.latestVersion,
    required this.releaseName,
    required this.releaseNotes,
    this.publishedAt,
    this.apkAsset,
    required this.releaseUrl,
  });
}

class UpdateService {
  /// Reads the actual installed app version on Android via MethodChannel, falling back to `currentAppVersion`.
  static Future<String> getInstalledVersion() async {
    try {
      final version =
          await _updaterChannel.invokeMethod<String>('getAppVersion');
      if (version != null && version.trim().isNotEmpty) {
        return version.trim();
      }
    } catch (_) {
      // Test environment or non-Android platform
    }
    return currentAppVersion;
  }

  /// Resolves the app-private cache/updates directory exposed by Android FileProvider.
  static Future<Directory> getUpdatesDirectory() async {
    try {
      final path =
          await _updaterChannel.invokeMethod<String>('getUpdatesDir');
      if (path != null && path.trim().isNotEmpty) {
        final dir = Directory(path.trim());
        if (!dir.existsSync()) {
          dir.createSync(recursive: true);
        }
        return dir;
      }
    } catch (_) {
      // Fallback for unit tests
    }
    final fallback =
        Directory('${Directory.systemTemp.path}${Platform.pathSeparator}updates');
    if (!fallback.existsSync()) {
      fallback.createSync(recursive: true);
    }
    return fallback;
  }

  static Future<bool> canInstallPackages() async {
    try {
      final allowed =
          await _updaterChannel.invokeMethod<bool>('canInstallPackages');
      return allowed ?? true;
    } catch (_) {
      return true;
    }
  }

  static Future<bool> openInstallPermissionSettings() async {
    try {
      final opened = await _updaterChannel
          .invokeMethod<bool>('openInstallPermissionSettings');
      return opened ?? false;
    } catch (_) {
      return false;
    }
  }

  static Future<ApkInstallResult> installApk(String filePath) async {
    final res = await _updaterChannel.invokeMethod<String>(
      'installApk',
      <String, dynamic>{'filePath': filePath},
    );
    if (res == 'PERMISSION_REQUIRED') {
      return ApkInstallResult.permissionRequired;
    }
    return ApkInstallResult.launched;
  }

  /// Parses semver triple `(major, minor, patch)` from a version string (e.g. "0.3.1", "v0.3.2").
  static List<int>? parseSemver(String s) {
    var clean = s.trim();
    if (clean.startsWith('v') || clean.startsWith('V')) {
      clean = clean.substring(1);
    }
    final parts = clean.split('.');
    if (parts.length < 3) return null;
    final major = int.tryParse(parts[0]);
    final minor = int.tryParse(parts[1]);
    final patchRaw = parts[2].split(RegExp(r'[-+]')).first;
    final patch = int.tryParse(patchRaw);
    if (major == null || minor == null || patch == null) return null;
    return [major, minor, patch];
  }

  /// Returns true if candidateTag is newer than currentVersion.
  static bool isNewer(String candidateTag, String currentVersion) {
    final candidate = parseSemver(candidateTag);
    final current = parseSemver(currentVersion);
    if (candidate == null || current == null) return false;
    for (int i = 0; i < 3; i++) {
      if (candidate[i] > current[i]) return true;
      if (candidate[i] < current[i]) return false;
    }
    return false;
  }

  /// Selects the best APK asset for standard (GitHub) self-updating Android devices.
  static ReleaseAsset? pickMobileAsset(List<ReleaseAsset> assets) {
    final apks = assets.where((a) {
      final lower = a.name.toLowerCase();
      return lower.endsWith('.apk') && !lower.contains('-foss');
    }).toList();
    if (apks.isEmpty) return null;

    // 优先选择 arm64-v8a 适配移动端常见架构
    final arm64 =
        apks.where((a) => a.name.toLowerCase().contains('arm64')).firstOrNull;
    if (arm64 != null) return arm64;

    // 其次选择 universal 或包含 android 的
    final universal = apks.where((a) {
      final l = a.name.toLowerCase();
      return l.contains('universal') ||
          l.contains('android') ||
          l.contains('mobile');
    }).firstOrNull;
    if (universal != null) return universal;

    // 默认返回首个 APK
    return apks.first;
  }

  /// Parses raw HTML from GitHub `expanded_assets` endpoint into [ReleaseAsset] items.
  static List<ReleaseAsset> parseExpandedAssetsHtml(
    String html,
    String owner,
    String repo,
    String tag,
  ) {
    final assets = <ReleaseAsset>[];
    final prefix = '/$owner/$repo/releases/download/$tag/';
    final seen = <String>{};
    for (final part in html.split('"')) {
      if (part.startsWith(prefix)) {
        final rest = part.substring(prefix.length);
        final fileName = rest.split(RegExp(r'[?#&/]')).first.trim();
        if (fileName.isNotEmpty && seen.add(fileName)) {
          assets.add(
            ReleaseAsset(
              name: fileName,
              downloadUrl: 'https://github.com$prefix$fileName',
              size: 0,
            ),
          );
        }
      }
    }
    return assets;
  }

  /// Formats byte sizes into readable string (e.g. 15.4 MB).
  static String formatFileSize(int bytes) {
    if (bytes <= 0) return '0 B';
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
    return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
  }

  /// Queries GitHub Releases API (with web redirect fallback when rate-limited).
  static Future<UpdateCheckResult> checkUpdate({
    String? currentVersion,
    String owner = defaultRepoOwner,
    String repo = defaultRepoName,
    HttpClient? client,
  }) async {
    final resolvedCurrent = currentVersion ?? await getInstalledVersion();
    final uri =
        Uri.parse('https://api.github.com/repos/$owner/$repo/releases/latest');
    final ioClient = client ?? (HttpClient()..connectionTimeout = _networkTimeout);

    try {
      final req = await ioClient.getUrl(uri).timeout(_networkTimeout);
      req.headers.set('Accept', 'application/vnd.github.v3+json');
      req.headers.set('User-Agent', 'Jiyue-Mobile/$resolvedCurrent');

      final resp = await req.close().timeout(_networkTimeout);
      if (resp.statusCode == 403 || resp.statusCode == 429) {
        await resp.drain<void>().catchError((_) {});
        return await _checkUpdateViaWeb(
          ioClient: ioClient,
          currentVersion: resolvedCurrent,
          owner: owner,
          repo: repo,
        );
      }

      final body = await resp.transform(utf8.decoder).join().timeout(_networkTimeout);
      if (resp.statusCode != 200) {
        throw Exception('检查更新失败 (HTTP ${resp.statusCode})');
      }

      final json = jsonDecode(body) as Map<String, dynamic>;
      final tagName = json['tag_name'] as String? ?? '';
      final releaseName = json['name'] as String? ?? tagName;
      final releaseNotes = json['body'] as String? ?? '';
      final publishedAt = json['published_at'] as String?;
      final releaseUrl = json['html_url'] as String? ??
          'https://github.com/$owner/$repo/releases/latest';

      final rawAssets = json['assets'] as List<dynamic>? ?? [];
      final assets = rawAssets
          .map((e) => ReleaseAsset.fromJson(e as Map<String, dynamic>))
          .toList();

      final apk = pickMobileAsset(assets);
      final hasUpdate = isNewer(tagName, resolvedCurrent);

      return UpdateCheckResult(
        hasUpdate: hasUpdate,
        currentVersion: resolvedCurrent,
        latestVersion: tagName.replaceAll(RegExp(r'^[vV]'), ''),
        releaseName: releaseName,
        releaseNotes: releaseNotes,
        publishedAt: publishedAt,
        apkAsset: apk,
        releaseUrl: releaseUrl,
      );
    } on TimeoutException {
      throw Exception('检查更新超时，请检查网络连接后重试');
    } finally {
      if (client == null) {
        ioClient.close();
      }
    }
  }

  /// Fallback via GitHub web endpoints bypassing REST API unauthenticated rate limits.
  static Future<UpdateCheckResult> _checkUpdateViaWeb({
    required HttpClient ioClient,
    required String currentVersion,
    required String owner,
    required String repo,
  }) async {
    final latestUri =
        Uri.parse('https://github.com/$owner/$repo/releases/latest');
    final req = await ioClient.getUrl(latestUri).timeout(_networkTimeout);
    req.followRedirects = false;
    req.headers.set('User-Agent', 'Jiyue-Mobile/$currentVersion');
    final resp = await req.close().timeout(_networkTimeout);

    String? tag;
    if (resp.statusCode >= 300 && resp.statusCode < 400) {
      await resp.drain<void>().catchError((_) {});
      final location = resp.headers.value(HttpHeaders.locationHeader) ?? '';
      final idx = location.indexOf('/tag/');
      if (idx != -1) {
        tag = location.substring(idx + 5).split(RegExp(r'[?#/]')).first.trim();
      }
    } else if (resp.statusCode == 200) {
      final html =
          await resp.transform(utf8.decoder).join().timeout(_networkTimeout);
      final match = RegExp(
        '/$owner/$repo/releases/tag/([^"/?#\\s]+)',
      ).firstMatch(html);
      tag = match?.group(1)?.trim();
    } else {
      await resp.drain<void>().catchError((_) {});
      throw Exception('GitHub 访问受限 (HTTP ${resp.statusCode})');
    }

    if (tag == null || tag.isEmpty) {
      throw Exception('GitHub 访问频次受限，无法解析最新版本信息');
    }

    final hasUpdate = isNewer(tag, currentVersion);
    final cleanLatest = tag.replaceAll(RegExp(r'^[vV]'), '');
    final releaseUrl = 'https://github.com/$owner/$repo/releases/tag/$tag';
    ReleaseAsset? apk;

    if (hasUpdate) {
      try {
        final expandedUri = Uri.parse(
          'https://github.com/$owner/$repo/releases/expanded_assets/$tag',
        );
        final expReq =
            await ioClient.getUrl(expandedUri).timeout(_networkTimeout);
        expReq.headers.set('User-Agent', 'Jiyue-Mobile/$currentVersion');
        final expResp = await expReq.close().timeout(_networkTimeout);
        if (expResp.statusCode == 200) {
          final html = await expResp
              .transform(utf8.decoder)
              .join()
              .timeout(_networkTimeout);
          final assets = parseExpandedAssetsHtml(html, owner, repo, tag);
          apk = pickMobileAsset(assets);
        } else {
          await expResp.drain<void>().catchError((_) {});
        }
      } catch (_) {
        // Fallback still allows user to open releaseUrl in browser
      }
    }

    return UpdateCheckResult(
      hasUpdate: hasUpdate,
      currentVersion: currentVersion,
      latestVersion: cleanLatest,
      releaseName: 'Jiyue v$cleanLatest',
      releaseNotes: '当前通过 GitHub 发布页检测到新版本，更新详情请前往发布页查看。',
      apkAsset: apk,
      releaseUrl: releaseUrl,
    );
  }

  /// Streams the release APK to app-private cache/updates/ with progress, cancellation, and reuse.
  static Future<File> downloadApk(
    UpdateCheckResult result, {
    required void Function(int received, int total) onProgress,
    UpdateDownloadToken? cancelToken,
    HttpClient? client,
    Directory? targetDir,
  }) async {
    final asset = result.apkAsset;
    if (asset == null || asset.downloadUrl.isEmpty) {
      throw Exception('当前版本未提供可直连下载的 Android 安装包');
    }

    final dir = targetDir ?? await getUpdatesDirectory();
    if (!dir.existsSync()) {
      dir.createSync(recursive: true);
    }

    final safeVersion =
        result.latestVersion.replaceAll(RegExp(r'[^a-zA-Z0-9._-]'), '_');
    final destFile =
        File('${dir.path}${Platform.pathSeparator}jiyue-$safeVersion.apk');

    // Reuse cached APK when expected size matches
    if (destFile.existsSync() &&
        asset.size > 0 &&
        destFile.lengthSync() == asset.size) {
      onProgress(asset.size, asset.size);
      return destFile;
    }

    // Prune stale APK or partial downloads to preserve device storage
    for (final entity in dir.listSync().whereType<File>()) {
      if (entity.path.endsWith('.apk') || entity.path.endsWith('.part')) {
        try {
          entity.deleteSync();
        } catch (_) {}
      }
    }

    final partFile = File('${destFile.path}.part');
    final ioClient =
        client ?? (HttpClient()..connectionTimeout = const Duration(seconds: 20));
    IOSink? sink;

    try {
      final req =
          await ioClient.getUrl(Uri.parse(asset.downloadUrl)).timeout(_networkTimeout);
      req.headers.set('User-Agent', 'Jiyue-Mobile/${result.currentVersion}');
      final resp = await req.close().timeout(const Duration(seconds: 30));

      if (resp.statusCode != 200) {
        await resp.drain<void>().catchError((_) {});
        throw Exception('安装包下载失败 (HTTP ${resp.statusCode})');
      }

      final total = resp.contentLength > 0 ? resp.contentLength : asset.size;
      sink = partFile.openWrite();
      int received = 0;

      await for (final chunk in resp) {
        if (cancelToken?.isCancelled == true) {
          throw UpdateCancelledException();
        }
        sink.add(chunk);
        received += chunk.length;
        onProgress(received, total);
      }

      if (cancelToken?.isCancelled == true) {
        throw UpdateCancelledException();
      }

      await sink.flush();
      await sink.close();
      sink = null;

      if (received == 0 || (total > 0 && received != total)) {
        throw Exception('下载的安装包大小异常 ($received / $total)，请重试');
      }

      if (destFile.existsSync()) {
        destFile.deleteSync();
      }
      await partFile.rename(destFile.path);
      return destFile;
    } catch (e) {
      try {
        await sink?.close();
      } catch (_) {}
      if (partFile.existsSync()) {
        try {
          partFile.deleteSync();
        } catch (_) {}
      }
      if (e is TimeoutException) {
        throw Exception('安装包下载超时，请重试或改用浏览器下载');
      }
      rethrow;
    } finally {
      if (client == null) {
        ioClient.close();
      }
    }
  }
}
