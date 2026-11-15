import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:jiyue_mobile/models/exported_config.dart';
import 'package:jiyue_mobile/services/ai_service.dart';

class _MockHttpClient implements HttpClient {
  final Map<String, dynamic> responseJson;
  final int statusCode;

  _MockHttpClient({required this.responseJson, required this.statusCode});

  @override
  Future<HttpClientRequest> postUrl(Uri url) async {
    return _MockHttpRequest(responseJson: responseJson, statusCode: statusCode);
  }

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockHttpRequest implements HttpClientRequest {
  final Map<String, dynamic> responseJson;
  final int statusCode;
  @override
  final HttpHeaders headers = _MockHttpHeaders();

  _MockHttpRequest({required this.responseJson, required this.statusCode});

  @override
  void write(Object? obj) {}

  @override
  Future<HttpClientResponse> close() async {
    return _MockHttpResponse(responseJson: responseJson, statusCode: statusCode);
  }

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockHttpHeaders implements HttpHeaders {
  @override
  void set(String name, Object value, {bool preserveHeaderCase = false}) {}

  @override
  void noSuchMethod(Invocation invocation) {}
}

class _MockHttpResponse extends Stream<List<int>> implements HttpClientResponse {
  final Map<String, dynamic> responseJson;
  @override
  final int statusCode;

  _MockHttpResponse({required this.responseJson, required this.statusCode});

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
  group('AiService Prompts', () {
    test('all styles have distinctive prompts', () {
      final smart = AiService.getSystemPrompt(PolishStyle.smart);
      final prof = AiService.getSystemPrompt(PolishStyle.professional);
      final casual = AiService.getSystemPrompt(PolishStyle.casual);
      final sum = AiService.getSystemPrompt(PolishStyle.summarize);

      expect(smart, contains('语言润色'));
      expect(prof, contains('学术写作'));
      expect(casual, contains('生动有趣'));
      expect(sum, contains('速记总结'));
    });
  });

  group('AiService Execution', () {
    test('parses OpenAI compatible response', () async {
      final mockData = {
        'choices': [
          {
            'message': {
              'role': 'assistant',
              'content': '这是润色后的优雅文本。',
            }
          }
        ]
      };

      await HttpOverrides.runZoned(() async {
        final provider = ExportedAiProvider(
          id: 'openai',
          name: 'OpenAI',
          kind: 'open_ai_compatible',
          baseUrl: 'https://api.openai.com/v1',
          chatModel: 'gpt-4o-mini',
          apiKey: 'mock-key',
        );

        final result = await AiService.execute(
          text: '这是粗糙草稿',
          style: PolishStyle.smart,
          provider: provider,
        );

        expect(result, equals('这是润色后的优雅文本。'));
      }, createHttpClient: (ctx) => _MockHttpClient(responseJson: mockData, statusCode: 200));
    });

    test('parses Anthropic response', () async {
      final mockData = {
        'content': [
          {
            'type': 'text',
            'text': 'Anthropic 润色完成。',
          }
        ]
      };

      await HttpOverrides.runZoned(() async {
        final provider = ExportedAiProvider(
          id: 'claude',
          name: 'Claude',
          kind: 'anthropic',
          baseUrl: 'https://api.anthropic.com',
          chatModel: 'claude-3-5-sonnet',
          apiKey: 'mock-key',
        );

        final result = await AiService.execute(
          text: '原文',
          style: PolishStyle.professional,
          provider: provider,
        );

        expect(result, equals('Anthropic 润色完成。'));
      }, createHttpClient: (ctx) => _MockHttpClient(responseJson: mockData, statusCode: 200));
    });

    test('testConnection supports custom model and measures latency', () async {
      final mockData = {
        'choices': [
          {
            'message': {
              'role': 'assistant',
              'content': 'pong',
            }
          }
        ]
      };

      await HttpOverrides.runZoned(() async {
        final provider = ExportedAiProvider(
          id: 'deepseek',
          name: 'DeepSeek',
          kind: 'openaicompatible',
          baseUrl: 'https://api.ai.init.cool',
          chatModel: 'deepseek-chat',
          apiKey: 'mock-key',
        );

        final result = await AiService.testConnection(
          provider: provider,
          customModel: 'deepseek-flash-eco',
        );

        expect(result['success'], isTrue);
        expect(result['model'], equals('deepseek-flash-eco'));
        expect(result['latency_ms'], isA<int>());
      }, createHttpClient: (ctx) => _MockHttpClient(responseJson: mockData, statusCode: 200));
    });
  });
}
