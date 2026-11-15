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

class ExportedConfig {
  final String jiyue;
  final int version;
  final int timestamp;
  final S3Config? s3;
  final WebDavConfig? webdav;
  final ExportedAiConfig? ai;

  ExportedConfig({
    required this.jiyue,
    required this.version,
    required this.timestamp,
    this.s3,
    this.webdav,
    this.ai,
  });

  factory ExportedConfig.fromJson(Map<String, dynamic> json) {
    final syncMap = json['sync'] as Map<String, dynamic>?;
    S3Config? s3;
    WebDavConfig? webdav;

    if (syncMap != null) {
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

    return ExportedConfig(
      jiyue: json['jiyue'] as String? ?? '',
      version: json['version'] as int? ?? 1,
      timestamp: json['timestamp'] as int? ?? 0,
      s3: s3,
      webdav: webdav,
      ai: ai,
    );
  }
}
