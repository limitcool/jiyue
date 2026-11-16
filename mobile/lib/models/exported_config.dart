import 'sync_config.dart';

class ExportedAiProvider {
  final String id;
  final String name;
  final String kind;
  final String baseUrl;
  final String chatModel;
  final String? embeddingModel;
  final String apiKey;

  ExportedAiProvider({
    required this.id,
    required this.name,
    required this.kind,
    required this.baseUrl,
    required this.chatModel,
    this.embeddingModel,
    required this.apiKey,
  });

  factory ExportedAiProvider.fromJson(Map<String, dynamic> json) {
    return ExportedAiProvider(
      id: json['id'] as String? ?? '',
      name: json['name'] as String? ?? '',
      kind: json['kind'] as String? ?? '',
      baseUrl: json['base_url'] as String? ?? '',
      chatModel: json['chat_model'] as String? ?? '',
      embeddingModel: json['embedding_model'] as String?,
      apiKey: json['api_key'] as String? ?? '',
    );
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'kind': kind,
    'base_url': baseUrl,
    'chat_model': chatModel,
    'embedding_model': embeddingModel,
    'api_key': apiKey,
  };

  ExportedAiProvider copyWith({
    String? id,
    String? name,
    String? kind,
    String? baseUrl,
    String? chatModel,
    String? embeddingModel,
    String? apiKey,
  }) {
    return ExportedAiProvider(
      id: id ?? this.id,
      name: name ?? this.name,
      kind: kind ?? this.kind,
      baseUrl: baseUrl ?? this.baseUrl,
      chatModel: chatModel ?? this.chatModel,
      embeddingModel: embeddingModel ?? this.embeddingModel,
      apiKey: apiKey ?? this.apiKey,
    );
  }
}

class ExportedAiConfig {
  final bool enabled;
  final String? activeChat;
  final List<ExportedAiProvider> providers;

  ExportedAiConfig({
    required this.enabled,
    this.activeChat,
    required this.providers,
  });

  ExportedAiConfig copyWith({
    bool? enabled,
    String? activeChat,
    List<ExportedAiProvider>? providers,
  }) {
    return ExportedAiConfig(
      enabled: enabled ?? this.enabled,
      activeChat: activeChat ?? this.activeChat,
      providers: providers ?? this.providers,
    );
  }

  factory ExportedAiConfig.fromJson(Map<String, dynamic> json) {
    final rawProviders = json['providers'] as List<dynamic>? ?? [];
    final list = rawProviders
        .map((e) => ExportedAiProvider.fromJson(e as Map<String, dynamic>))
        .toList();

    return ExportedAiConfig(
      enabled: json['enabled'] as bool? ?? false,
      activeChat: json['active_chat'] as String?,
      providers: list,
    );
  }

  Map<String, dynamic> toJson() => {
    'enabled': enabled,
    'active_chat': activeChat,
    'providers': providers.map((e) => e.toJson()).toList(),
  };
}

class ExportedMemosConfig {
  final String endpoint;
  final String token;

  ExportedMemosConfig({
    required this.endpoint,
    required this.token,
  });

  factory ExportedMemosConfig.fromJson(Map<String, dynamic> json) {
    return ExportedMemosConfig(
      endpoint: json['endpoint'] as String? ?? '',
      token: json['token'] as String? ?? '',
    );
  }

  Map<String, dynamic> toJson() => {
    'endpoint': endpoint,
    'token': token,
  };
}

class ExportedConfig {
  final String jiyue;
  final int version;
  final int timestamp;
  final String? vaultName;
  final S3Config? s3;
  final WebDavConfig? webdav;
  final ExportedAiConfig? ai;
  final ExportedMemosConfig? memos;

  ExportedConfig({
    required this.jiyue,
    required this.version,
    required this.timestamp,
    this.vaultName,
    this.s3,
    this.webdav,
    this.ai,
    this.memos,
  });

  /// Returns an explicit vault name from PC export, or infers one from `root_prefix` (`jiyue/<name>/`).
  String get suggestedVaultName {
    final explicit = vaultName?.trim();
    if (explicit != null && explicit.isNotEmpty) {
      return explicit;
    }
    final prefix = (s3?.rootPrefix ?? webdav?.root ?? '').trim();
    if (prefix.isNotEmpty) {
      final parts = prefix
          .replaceAll('\\', '/')
          .split('/')
          .where((s) => s.trim().isNotEmpty)
          .toList();
      if (parts.isNotEmpty) {
        if (parts.length >= 2 && parts.first.toLowerCase() == 'jiyue') {
          return parts[1];
        }
        return parts.last;
      }
    }
    final bucket = s3?.bucket.trim();
    if (bucket != null && bucket.isNotEmpty) {
      return bucket;
    }
    return '云端笔记库';
  }

  factory ExportedConfig.fromJson(Map<String, dynamic> json) {
    final syncMap = json['sync'] as Map<String, dynamic>?;
    S3Config? s3;
    WebDavConfig? webdav;
    String? vaultName;

    if (syncMap != null) {
      final rawName = (syncMap['vault_name'] as String?)?.trim();
      if (rawName != null && rawName.isNotEmpty) {
        vaultName = rawName;
      }
      final backend = syncMap['backend'] as String? ?? 's3';
      if (backend == 's3') {
        s3 = S3Config(
          endpoint: syncMap['endpoint'] as String? ?? '',
          region: syncMap['region'] as String? ?? 'us-east-1',
          bucket: syncMap['bucket'] as String? ?? '',
          accessKeyId: syncMap['access_key_id'] as String? ?? '',
          secretAccessKey: syncMap['secret_access_key'] as String? ?? '',
          rootPrefix: syncMap['root_prefix'] as String? ?? '',
          forcePathStyle: syncMap['force_path_style'] as bool? ?? true,
        );
      } else if (backend == 'webdav') {
        webdav = WebDavConfig(
          endpoint: syncMap['endpoint'] as String? ?? '',
          username: syncMap['username'] as String? ?? '',
          password: syncMap['password'] as String? ?? '',
          root: syncMap['root'] as String? ?? '',
        );
      }
    }

    final aiMap = json['ai'] as Map<String, dynamic>?;
    final ai = aiMap != null ? ExportedAiConfig.fromJson(aiMap) : null;

    final memosMap = json['memos'] as Map<String, dynamic>?;
    final memos = memosMap != null ? ExportedMemosConfig.fromJson(memosMap) : null;

    return ExportedConfig(
      jiyue: json['jiyue'] as String? ?? '',
      version: json['version'] as int? ?? 1,
      timestamp: json['timestamp'] as int? ?? 0,
      vaultName: vaultName,
      s3: s3,
      webdav: webdav,
      ai: ai,
      memos: memos,
    );
  }
}
