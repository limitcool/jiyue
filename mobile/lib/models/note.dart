class BacklinkItem {
  final String path;
  final String title;
  final String snippet;

  const BacklinkItem({
    required this.path,
    required this.title,
    required this.snippet,
  });

  factory BacklinkItem.fromJson(Map<String, dynamic> json) {
    return BacklinkItem(
      path: json['path'] as String? ?? '',
      title: json['title'] as String? ?? '',
      snippet: json['snippet'] as String? ?? '',
    );
  }
}

class NoteSummary {
  final String id;
  final String title;
  final String path;
  final String preview;
  final List<String> tags;
  final int modifiedMs;

  NoteSummary({
    required this.id,
    required this.title,
    required this.path,
    required this.preview,
    required this.tags,
    required this.modifiedMs,
  });

  factory NoteSummary.fromJson(Map<String, dynamic> json) {
    return NoteSummary(
      id: json['id'] as String? ?? '',
      title: json['title'] as String? ?? '',
      path: json['path'] as String? ?? '',
      preview: json['preview'] as String? ?? '',
      tags:
          (json['tags'] as List<dynamic>?)?.map((e) => e.toString()).toList() ??
              [],
      modifiedMs: json['modified_ms'] as int? ?? 0,
    );
  }
}

class NoteDetail {
  final String id;
  final String title;
  final String path;
  final String content;
  final List<String> tags;
  final List<String> wikilinks;
  final List<BacklinkItem> backlinks;
  final int modifiedMs;

  NoteDetail({
    required this.id,
    required this.title,
    required this.path,
    required this.content,
    required this.tags,
    required this.wikilinks,
    this.backlinks = const [],
    required this.modifiedMs,
  });

  factory NoteDetail.fromJson(Map<String, dynamic> json) {
    final rawBacklinks = json['backlinks'] as List<dynamic>? ?? const [];
    return NoteDetail(
      id: json['id'] as String? ?? '',
      title: json['title'] as String? ?? '',
      path: json['path'] as String? ?? '',
      content: json['content'] as String? ?? '',
      tags:
          (json['tags'] as List<dynamic>?)?.map((e) => e.toString()).toList() ??
              [],
      wikilinks:
          (json['wikilinks'] as List<dynamic>?)
              ?.map((e) => e.toString())
              .toList() ??
              [],
      backlinks:
          rawBacklinks
              .whereType<Map<String, dynamic>>()
              .map(BacklinkItem.fromJson)
              .toList(),
      modifiedMs: json['modified_ms'] as int? ?? 0,
    );
  }
}

class SearchResultItem {
  final String path;
  final String title;
  final String snippet;
  final String matchType;

  SearchResultItem({
    required this.path,
    required this.title,
    required this.snippet,
    required this.matchType,
  });

  factory SearchResultItem.fromJson(Map<String, dynamic> json) {
    return SearchResultItem(
      path: json['path'] as String? ?? '',
      title: json['title'] as String? ?? '',
      snippet: json['snippet'] as String? ?? '',
      matchType: json['match_type'] as String? ?? 'content',
    );
  }
}

class InboxCapture {
  final String notePath;
  final String noteTitle;
  final int? lineIndex;
  final String dateLabel;
  final String? timeLabel;
  final String content;
  final List<String> tags;

  const InboxCapture({
    required this.notePath,
    required this.noteTitle,
    required this.lineIndex,
    required this.dateLabel,
    required this.timeLabel,
    required this.content,
    required this.tags,
  });

  factory InboxCapture.fromJson(Map<String, dynamic> json) {
    return InboxCapture(
      notePath: json['note_path'] as String? ?? '',
      noteTitle: json['note_title'] as String? ?? '',
      lineIndex: json['line_index'] as int?,
      dateLabel: json['date_label'] as String? ?? '',
      timeLabel: json['time_label'] as String?,
      content: json['content'] as String? ?? '',
      tags:
          (json['tags'] as List<dynamic>?)?.map((e) => e.toString()).toList() ??
              const [],
    );
  }
}

class InboxTodo {
  final String notePath;
  final String noteTitle;
  final String group;
  final int lineIndex;
  final String text;
  final bool done;
  final List<String> tags;

  const InboxTodo({
    required this.notePath,
    required this.noteTitle,
    required this.group,
    required this.lineIndex,
    required this.text,
    required this.done,
    required this.tags,
  });

  factory InboxTodo.fromJson(Map<String, dynamic> json) {
    return InboxTodo(
      notePath: json['note_path'] as String? ?? '',
      noteTitle: json['note_title'] as String? ?? '',
      group: json['group'] as String? ?? 'core',
      lineIndex: json['line_index'] as int? ?? 0,
      text: json['text'] as String? ?? '',
      done: json['done'] as bool? ?? false,
      tags:
          (json['tags'] as List<dynamic>?)?.map((e) => e.toString()).toList() ??
              const [],
    );
  }
}

class InboxSummary {
  final List<InboxCapture> captures;
  final List<InboxTodo> todos;
  final String streamPath;
  final String dailyPath;

  const InboxSummary({
    required this.captures,
    required this.todos,
    required this.streamPath,
    required this.dailyPath,
  });

  static const empty = InboxSummary(
    captures: [],
    todos: [],
    streamPath: 'Inbox/随手记.md',
    dailyPath: '',
  );

  factory InboxSummary.fromJson(Map<String, dynamic> json) {
    final rawCaps = json['captures'] as List<dynamic>? ?? const [];
    final rawTodos = json['todos'] as List<dynamic>? ?? const [];
    return InboxSummary(
      captures:
          rawCaps
              .whereType<Map<String, dynamic>>()
              .map(InboxCapture.fromJson)
              .toList(),
      todos:
          rawTodos
              .whereType<Map<String, dynamic>>()
              .map(InboxTodo.fromJson)
              .toList(),
      streamPath: json['stream_path'] as String? ?? 'Inbox/随手记.md',
      dailyPath: json['daily_path'] as String? ?? '',
    );
  }
}

class ImportFileResult {
  final bool success;
  final String destRel;
  final String linkText;
  final String notePath;
  final String message;

  const ImportFileResult({
    required this.success,
    required this.destRel,
    required this.linkText,
    required this.notePath,
    required this.message,
  });

  factory ImportFileResult.fromJson(Map<String, dynamic> json) {
    return ImportFileResult(
      success: json['success'] as bool? ?? false,
      destRel: json['dest_rel'] as String? ?? '',
      linkText: json['link_text'] as String? ?? '',
      notePath: json['note_path'] as String? ?? '',
      message: json['message'] as String? ?? '',
    );
  }
}

class SharedPayload {
  final String text;
  final List<String> filePaths;

  const SharedPayload({
    required this.text,
    required this.filePaths,
  });

  bool get isEmpty => text.trim().isEmpty && filePaths.isEmpty;

  factory SharedPayload.fromMap(Map<dynamic, dynamic> map) {
    final rawPaths = map['filePaths'] as List<dynamic>? ?? const [];
    return SharedPayload(
      text: (map['text'] as String? ?? '').trim(),
      filePaths:
          rawPaths
              .map((e) => e.toString().trim())
              .where((p) => p.isNotEmpty)
              .toList(),
    );
  }
}
