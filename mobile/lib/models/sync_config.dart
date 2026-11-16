class S3Config {
  final String endpoint;
  final String region;
  final String bucket;
  final String accessKeyId;
  final String secretAccessKey;
  final String? sessionToken;
  final String rootPrefix;
  final bool forcePathStyle;

  S3Config({
    required this.endpoint,
    required this.region,
    required this.bucket,
    required this.accessKeyId,
    required this.secretAccessKey,
    this.sessionToken,
    this.rootPrefix = '',
    this.forcePathStyle = true,
  });

  factory S3Config.fromJson(Map<String, dynamic> json) {
    return S3Config(
      endpoint: json['endpoint'] as String? ?? '',
      region: json['region'] as String? ?? 'us-east-1',
      bucket: json['bucket'] as String? ?? '',
      accessKeyId: json['access_key_id'] as String? ?? '',
      secretAccessKey: json['secret_access_key'] as String? ?? '',
      sessionToken: json['session_token'] as String?,
      rootPrefix: json['root_prefix'] as String? ?? '',
      forcePathStyle: json['force_path_style'] as bool? ?? true,
    );
  }

  S3Config copyWith({
    String? endpoint,
    String? region,
    String? bucket,
    String? accessKeyId,
    String? secretAccessKey,
    String? sessionToken,
    String? rootPrefix,
    bool? forcePathStyle,
  }) {
    return S3Config(
      endpoint: endpoint ?? this.endpoint,
      region: region ?? this.region,
      bucket: bucket ?? this.bucket,
      accessKeyId: accessKeyId ?? this.accessKeyId,
      secretAccessKey: secretAccessKey ?? this.secretAccessKey,
      sessionToken: sessionToken ?? this.sessionToken,
      rootPrefix: rootPrefix ?? this.rootPrefix,
      forcePathStyle: forcePathStyle ?? this.forcePathStyle,
    );
  }

  Map<String, dynamic> toJson() => {
    'backend': 's3',
    'endpoint': endpoint,
    'region': region,
    'bucket': bucket,
    'access_key_id': accessKeyId,
    'secret_access_key': secretAccessKey,
    'session_token': sessionToken,
    'root_prefix': rootPrefix,
    'force_path_style': forcePathStyle,
  };
}

class WebDavConfig {
  final String endpoint;
  final String username;
  final String password;
  final String root;

  WebDavConfig({
    required this.endpoint,
    required this.username,
    required this.password,
    this.root = '',
  });

  factory WebDavConfig.fromJson(Map<String, dynamic> json) {
    return WebDavConfig(
      endpoint: json['endpoint'] as String? ?? '',
      username: json['username'] as String? ?? '',
      password: json['password'] as String? ?? '',
      root: json['root'] as String? ?? '',
    );
  }

  WebDavConfig copyWith({
    String? endpoint,
    String? username,
    String? password,
    String? root,
  }) {
    return WebDavConfig(
      endpoint: endpoint ?? this.endpoint,
      username: username ?? this.username,
      password: password ?? this.password,
      root: root ?? this.root,
    );
  }

  Map<String, dynamic> toJson() => {
    'backend': 'webdav',
    'endpoint': endpoint,
    'username': username,
    'password': password,
    'root': root,
  };
}

class ConnectionTestResult {
  final bool success;
  final bool reachable;
  final bool canList;
  final bool canWrite;
  final String message;

  const ConnectionTestResult({
    required this.success,
    required this.reachable,
    required this.canList,
    required this.canWrite,
    required this.message,
  });

  factory ConnectionTestResult.fromJson(Map<String, dynamic> json) {
    return ConnectionTestResult(
      success: json['success'] as bool? ?? false,
      reachable: json['reachable'] as bool? ?? false,
      canList: json['can_list'] as bool? ?? false,
      canWrite: json['can_write'] as bool? ?? false,
      message: json['message'] as String? ?? '',
    );
  }
}

class SyncResultReport {
  final bool success;
  final int uploaded;
  final int downloaded;
  final int deletedRemote;
  final int deletedLocal;
  final int conflicts;
  final String message;

  SyncResultReport({
    required this.success,
    required this.uploaded,
    required this.downloaded,
    required this.deletedRemote,
    required this.deletedLocal,
    required this.conflicts,
    required this.message,
  });

  factory SyncResultReport.fromJson(Map<String, dynamic> json) {
    return SyncResultReport(
      success: json['success'] as bool? ?? false,
      uploaded: json['uploaded'] as int? ?? 0,
      downloaded: json['downloaded'] as int? ?? 0,
      deletedRemote: json['deleted_remote'] as int? ?? 0,
      deletedLocal: json['deleted_local'] as int? ?? 0,
      conflicts: json['conflicts'] as int? ?? 0,
      message: json['message'] as String? ?? '',
    );
  }
}

/// Persistent configuration profile for a single local-first vault on mobile.
class VaultProfile {
  final String id;
  final String name;
  final String path;
  final bool isExternal;
  final S3Config? s3Config;
  final WebDavConfig? webdavConfig;
  final List<String> bookmarks;
  final int? lastSyncMs;

  const VaultProfile({
    required this.id,
    required this.name,
    required this.path,
    this.isExternal = false,
    this.s3Config,
    this.webdavConfig,
    this.bookmarks = const [],
    this.lastSyncMs,
  });

  bool get hasRemoteSync => s3Config != null || webdavConfig != null;

  String get syncBadgeLabel {
    if (s3Config != null) {
      final bucket = s3Config!.bucket.trim();
      return bucket.isEmpty ? 'S3' : 'S3 · $bucket';
    }
    if (webdavConfig != null) {
      return 'WebDAV';
    }
    return isExternal ? '本地目录' : '纯本地';
  }

  Map<String, dynamic>? get activeStoreConfigJson {
    if (s3Config != null) return s3Config!.toJson();
    if (webdavConfig != null) return webdavConfig!.toJson();
    return null;
  }

  VaultProfile copyWith({
    String? id,
    String? name,
    String? path,
    bool? isExternal,
    S3Config? s3Config,
    bool clearS3Config = false,
    WebDavConfig? webdavConfig,
    bool clearWebdavConfig = false,
    List<String>? bookmarks,
    int? lastSyncMs,
    bool clearLastSyncMs = false,
  }) {
    return VaultProfile(
      id: id ?? this.id,
      name: name ?? this.name,
      path: path ?? this.path,
      isExternal: isExternal ?? this.isExternal,
      s3Config: clearS3Config ? null : (s3Config ?? this.s3Config),
      webdavConfig:
          clearWebdavConfig ? null : (webdavConfig ?? this.webdavConfig),
      bookmarks: bookmarks ?? this.bookmarks,
      lastSyncMs: clearLastSyncMs ? null : (lastSyncMs ?? this.lastSyncMs),
    );
  }

  factory VaultProfile.fromJson(Map<String, dynamic> json) {
    final s3Map = json['s3_config'] as Map<String, dynamic>?;
    final davMap = json['webdav_config'] as Map<String, dynamic>?;
    final rawBookmarks = json['bookmarks'] as List<dynamic>? ?? const [];
    return VaultProfile(
      id: json['id'] as String? ?? '',
      name: json['name'] as String? ?? '未命名笔记库',
      path: json['path'] as String? ?? '',
      isExternal: json['is_external'] as bool? ?? false,
      s3Config: s3Map != null ? S3Config.fromJson(s3Map) : null,
      webdavConfig: davMap != null ? WebDavConfig.fromJson(davMap) : null,
      bookmarks: rawBookmarks.map((e) => e.toString()).toList(),
      lastSyncMs: json['last_sync_ms'] as int?,
    );
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'path': path,
    'is_external': isExternal,
    if (s3Config != null) 's3_config': s3Config!.toJson(),
    if (webdavConfig != null) 'webdav_config': webdavConfig!.toJson(),
    'bookmarks': bookmarks,
    if (lastSyncMs != null) 'last_sync_ms': lastSyncMs,
  };
}
