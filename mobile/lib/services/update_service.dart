import 'dart:convert';
import 'dart:io';

const String defaultRepoOwner = 'limitcool';
const String defaultRepoName = 'jiyue';
const String currentAppVersion = '0.3.1';

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

  /// Selects the best APK asset for Android devices.
  static ReleaseAsset? pickMobileAsset(List<ReleaseAsset> assets) {
    final apks = assets.where((a) => a.name.toLowerCase().endsWith('.apk')).toList();
    if (apks.isEmpty) return null;

    // 优先选择 arm64-v8a 适配移动端常见架构
    final arm64 = apks.where((a) => a.name.toLowerCase().contains('arm64')).firstOrNull;
    if (arm64 != null) return arm64;

    // 其次选择 universal 或包含 android 的
    final universal = apks.where((a) {
      final l = a.name.toLowerCase();
      return l.contains('universal') || l.contains('android') || l.contains('mobile');
    }).firstOrNull;
    if (universal != null) return universal;

    // 默认返回首个 APK
    return apks.first;
  }

  /// Formats byte sizes into readable string (e.g. 15.4 MB).
  static String formatFileSize(int bytes) {
    if (bytes <= 0) return '0 B';
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
    return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
  }

  /// Queries GitHub Releases API for latest release info.
  static Future<UpdateCheckResult> checkUpdate({
    String currentVersion = currentAppVersion,
    String owner = defaultRepoOwner,
    String repo = defaultRepoName,
    HttpClient? client,
  }) async {
    final uri = Uri.parse('https://api.github.com/repos/$owner/$repo/releases/latest');
    final ioClient = client ?? HttpClient();

    try {
      final req = await ioClient.getUrl(uri);
      req.headers.set('Accept', 'application/vnd.github.v3+json');
      req.headers.set('User-Agent', 'Jiyue-Mobile');

      final resp = await req.close();
      final body = await resp.transform(utf8.decoder).join();

      if (resp.statusCode == 403) {
        throw Exception('GitHub 访问频次受限，请稍后重试或直接查看发布页');
      }
      if (resp.statusCode != 200) {
        throw Exception('检查更新失败 (HTTP ${resp.statusCode})');
      }

      final json = jsonDecode(body) as Map<String, dynamic>;
      final tagName = json['tag_name'] as String? ?? '';
      final releaseName = json['name'] as String? ?? tagName;
      final releaseNotes = json['body'] as String? ?? '';
      final publishedAt = json['published_at'] as String?;
      final releaseUrl = json['html_url'] as String? ?? 'https://github.com/$owner/$repo/releases/latest';

      final rawAssets = json['assets'] as List<dynamic>? ?? [];
      final assets = rawAssets
          .map((e) => ReleaseAsset.fromJson(e as Map<String, dynamic>))
          .toList();

      final apk = pickMobileAsset(assets);
      final hasUpdate = isNewer(tagName, currentVersion);

      return UpdateCheckResult(
        hasUpdate: hasUpdate,
        currentVersion: currentVersion,
        latestVersion: tagName.replaceAll(RegExp(r'^[vV]'), ''),
        releaseName: releaseName,
        releaseNotes: releaseNotes,
        publishedAt: publishedAt,
        apkAsset: apk,
        releaseUrl: releaseUrl,
      );
    } finally {
      if (client == null) {
        ioClient.close();
      }
    }
  }
}
