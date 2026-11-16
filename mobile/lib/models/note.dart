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
