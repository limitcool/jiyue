import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:jiyue_mobile/models/exported_config.dart';

void main() {
  group('ExportedConfig Tests', () {
    test('successfully parses PC exported JSON with S3 and AI configurations', () {
      const jsonStr = '''
      {
        "jiyue": "config_export",
        "version": 1,
        "timestamp": 1740000000,
        "sync": {
          "backend": "s3",
          "endpoint": "https://s3.us-east-1.amazonaws.com",
          "region": "us-east-1",
          "bucket": "my-vault-bucket",
          "access_key_id": "AKIA_TEST_KEY",
          "secret_access_key": "SK_TEST_SECRET",
          "root_prefix": "jiyue/notes/",
          "force_path_style": true
        },
        "ai": {
          "enabled": true,
          "active_chat": "openai",
          "providers": [
            {
              "id": "openai",
              "name": "OpenAI",
              "kind": "openai",
              "base_url": "https://api.openai.com/v1",
              "chat_model": "gpt-4o",
              "embedding_model": "text-embedding-3-small",
              "api_key": "sk-proj-testkey"
            }
          ]
        }
      }
      ''';

      final Map<String, dynamic> map = jsonDecode(jsonStr);
      final config = ExportedConfig.fromJson(map);

      expect(config.jiyue, 'config_export');
      expect(config.version, 1);
      expect(config.timestamp, 1740000000);

      // Verify S3
      expect(config.s3, isNotNull);
      expect(config.s3!.bucket, 'my-vault-bucket');
      expect(config.s3!.endpoint, 'https://s3.us-east-1.amazonaws.com');
      expect(config.s3!.accessKeyId, 'AKIA_TEST_KEY');
      expect(config.s3!.secretAccessKey, 'SK_TEST_SECRET');
      expect(config.s3!.rootPrefix, 'jiyue/notes/');
      expect(config.s3!.forcePathStyle, isTrue);

      // Verify AI
      expect(config.ai, isNotNull);
      expect(config.ai!.enabled, isTrue);
      expect(config.ai!.activeChat, 'openai');
      expect(config.ai!.providers.length, 1);
      expect(config.ai!.providers[0].name, 'OpenAI');
      expect(config.ai!.providers[0].chatModel, 'gpt-4o');
      expect(config.ai!.providers[0].apiKey, 'sk-proj-testkey');
    });

    test('handles partial export without AI', () {
      const jsonStr = '''
      {
        "jiyue": "config_export",
        "version": 1,
        "timestamp": 1740000000,
        "sync": {
          "backend": "s3",
          "endpoint": "https://r2.cloudflarestorage.com",
          "region": "auto",
          "bucket": "r2-bucket",
          "access_key_id": "r2-id",
          "secret_access_key": "r2-secret",
          "root_prefix": "",
          "force_path_style": true
        }
      }
      ''';

      final Map<String, dynamic> map = jsonDecode(jsonStr);
      final config = ExportedConfig.fromJson(map);

      expect(config.s3, isNotNull);
      expect(config.s3!.bucket, 'r2-bucket');
      expect(config.ai, isNull);
    });
  });
}
