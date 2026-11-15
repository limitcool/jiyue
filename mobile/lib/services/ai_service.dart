import 'dart:convert';
import 'dart:io';
import '../models/exported_config.dart';

enum PolishStyle {
  smart('智能润色', '✨', '优化语法、标点与行文流畅度，保留 Markdown 标记'),
  professional('学术专业', '🎓', '逻辑严密、用词规范、客观理性，提升专业度'),
  casual('生动通俗', '💬', '亲切自然、生动活泼、通俗易懂，富于表达力'),
  summarize('提炼摘要', '📝', '提炼核心观点、关键事实与行动要点');

  final String label;
  final String icon;
  final String description;
  const PolishStyle(this.label, this.icon, this.description);
}

class AiService {
  static String getSystemPrompt(PolishStyle style) {
    switch (style) {
      case PolishStyle.smart:
        return '你是一位专业的写作与笔记编辑助手。请对用户提供的笔记文本进行语言润色。\n要求：\n1. 修正错别字、语病与不顺畅的表达。\n2. 保持原意不变，保留 Markdown 排版格式与代码块。\n3. 直接输出润色后的内容，不要添加任何开场白、解释或总结。';
      case PolishStyle.professional:
        return '你是一位专业的学术写作与技术文档助手。请对用户提供的笔记文本进行专业重写。\n要求：\n1. 用词严谨、规范，逻辑缜密，语气客观理性。\n2. 消除口语化表述与冗余词句，结构层次分明。\n3. 保留所有 Markdown 格式与代码块。\n4. 直接输出润色后的内容，不要添加任何开场白、解释或总结。';
      case PolishStyle.casual:
        return '你是一位生动有趣的创作者助手。请对用户提供的笔记文本进行通俗化润色。\n要求：\n1. 语言亲切接地气、生动形象、富于表达力。\n2. 把复杂晦涩的概念转为易于理解的日常表述。\n3. 保留所有 Markdown 格式与代码块。\n4. 直接输出润色后的内容，不要添加任何开场白、解释或总结。';
      case PolishStyle.summarize:
        return '你是一位精炼的速记总结助手。请为用户提供的笔记文本提炼一份精炼的核心摘要。\n要求：\n1. 提炼出核心观点与关键事实。\n2. 如有具体待办或行动项，单独列出。\n3. 使用清晰的 Markdown 列表。\n4. 直接输出摘要内容，不要添加任何无关寒暄。';
    }
  }

  static Future<String> execute({
    required String text,
    required PolishStyle style,
    required ExportedAiProvider provider,
    HttpClient? client,
  }) async {
    final sysPrompt = getSystemPrompt(style);
    final baseUrl = provider.baseUrl.trim().replaceAll(RegExp(r'/+$'), '');
    final isAnthropic = provider.kind.toLowerCase() == 'anthropic';

    final uri = isAnthropic
        ? Uri.parse('$baseUrl/v1/messages')
        : (baseUrl.endsWith('/v1')
            ? Uri.parse('$baseUrl/chat/completions')
            : Uri.parse('$baseUrl/v1/chat/completions'));

    final ioClient = client ?? HttpClient();
    try {
      final request = await ioClient.postUrl(uri);
      request.headers.set('content-type', 'application/json; charset=utf-8');

      if (isAnthropic) {
        request.headers.set('x-api-key', provider.apiKey);
        request.headers.set('anthropic-version', '2023-06-01');
        final payload = jsonEncode({
          'model': provider.chatModel,
          'max_tokens': 4096,
          'system': sysPrompt,
          'messages': [
            {'role': 'user', 'content': text}
          ],
        });
        request.write(payload);
      } else {
        request.headers.set('Authorization', 'Bearer ${provider.apiKey}');
        final payload = jsonEncode({
          'model': provider.chatModel,
          'messages': [
            {'role': 'system', 'content': sysPrompt},
            {'role': 'user', 'content': text}
          ],
          'temperature': 0.3,
        });
        request.write(payload);
      }

      final response = await request.close();
      final respBody = await response.transform(utf8.decoder).join();

      if (response.statusCode != 200) {
        throw Exception('AI 服务响应异常 (${response.statusCode}): $respBody');
      }

      final json = jsonDecode(respBody) as Map<String, dynamic>;
      if (isAnthropic) {
        final content = json['content'] as List<dynamic>?;
        if (content != null && content.isNotEmpty) {
          final first = content.first as Map<String, dynamic>;
          return (first['text'] as String? ?? '').trim();
        }
      } else {
        final choices = json['choices'] as List<dynamic>?;
        if (choices != null && choices.isNotEmpty) {
          final first = choices.first as Map<String, dynamic>;
          final msg = first['message'] as Map<String, dynamic>?;
          return (msg?['content'] as String? ?? '').trim();
        }
      }
      throw Exception('AI 未能返回有效内容');
    } finally {
      if (client == null) {
        ioClient.close();
      }
    }
  }
}
