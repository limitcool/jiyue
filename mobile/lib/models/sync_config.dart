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

  Map<String, dynamic> toJson() => {
    'backend': 'webdav',
    'endpoint': endpoint,
    'username': username,
    'password': password,
    'root': root,
  };
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
