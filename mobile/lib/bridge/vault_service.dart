import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'package:ffi/ffi.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../models/note.dart';
import '../models/sync_config.dart';
import '../models/exported_config.dart';

// FFI Typedefs
typedef COpen = ffi.Pointer<ffi.Void> Function(ffi.Pointer<Utf8> rootPath);
typedef DartOpen = ffi.Pointer<ffi.Void> Function(ffi.Pointer<Utf8> rootPath);

typedef CClose = ffi.Void Function(ffi.Pointer<ffi.Void> bridge);
typedef DartClose = void Function(ffi.Pointer<ffi.Void> bridge);

typedef CList = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge);
typedef DartList = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge);

typedef CGet = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> path);
typedef DartGet = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> path);

typedef CSave = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> path, ffi.Pointer<Utf8> content);
typedef DartSave = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> path, ffi.Pointer<Utf8> content);

typedef CCreate = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> title, ffi.Pointer<Utf8> content);
typedef DartCreate = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> title, ffi.Pointer<Utf8> content);

typedef CDelete = ffi.Bool Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> path);
typedef DartDelete = bool Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> path);

typedef CDailyGet = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge);
typedef DartDailyGet = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge);

typedef CDailyAppend = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> text);
typedef DartDailyAppend = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> text);

typedef CFree = ffi.Void Function(ffi.Pointer<Utf8> ptr);
typedef DartFree = void Function(ffi.Pointer<Utf8> ptr);

class VaultService {
  static final VaultService instance = VaultService._internal();
  VaultService._internal();

  Directory? _vaultDir;
  ffi.Pointer<ffi.Void>? _ffiBridge;
  bool _useFfi = false;

  DartOpen? _fnOpen;
  DartClose? _fnClose;
  DartList? _fnList;
  DartGet? _fnGet;
  DartSave? _fnSave;
  DartCreate? _fnCreate;
  DartDelete? _fnDelete;
  DartDailyGet? _fnDailyGet;
  DartDailyAppend? _fnDailyAppend;
  DartFree? _fnFree;

  S3Config? _s3Config;
  ExportedAiConfig? _aiConfig;
  ExportedMemosConfig? _memosConfig;

  S3Config? get s3Config => _s3Config;
  ExportedAiConfig? get aiConfig => _aiConfig;
  ExportedMemosConfig? get memosConfig => _memosConfig;

  String get vaultPath => _vaultDir?.path ?? '';
  String get vaultName => _vaultDir?.uri.pathSegments.where((s) => s.isNotEmpty).lastOrNull ?? '霁月';

  Future<void> init([String? customPath]) async {
    if (customPath != null && customPath.isNotEmpty) {
      _vaultDir = Directory(customPath);
    } else {
      final docDir = await getApplicationDocumentsDirectory();
      _vaultDir = Directory('${docDir.path}/Jiyue');
    }

    if (!await _vaultDir!.exists()) {
      await _vaultDir!.create(recursive: true);
    }
    final entities = await _vaultDir!.list().toList();
    if (entities.isEmpty) {
      await _initSampleVault();
    }

    // Attempt to link Rust Dynamic Library
    _tryInitFfi();

    await loadConfig();
  }

  Future<void> loadConfig() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final s3Json = prefs.getString('jiyue_s3_config');
      if (s3Json != null) {
        final map = jsonDecode(s3Json) as Map<String, dynamic>;
        _s3Config = S3Config(
          endpoint: map['endpoint'] as String? ?? '',
          region: map['region'] as String? ?? 'us-east-1',
          bucket: map['bucket'] as String? ?? '',
          accessKeyId: map['access_key_id'] as String? ?? '',
          secretAccessKey: map['secret_access_key'] as String? ?? '',
          sessionToken: map['session_token'] as String?,
          rootPrefix: map['root_prefix'] as String? ?? '',
          forcePathStyle: map['force_path_style'] as bool? ?? true,
        );
      }
      final aiJson = prefs.getString('jiyue_ai_config');
      if (aiJson != null) {
        _aiConfig = ExportedAiConfig.fromJson(jsonDecode(aiJson) as Map<String, dynamic>);
      }
      final memosJson = prefs.getString('jiyue_memos_config');
      if (memosJson != null) {
        _memosConfig = ExportedMemosConfig.fromJson(jsonDecode(memosJson) as Map<String, dynamic>);
      }
    } catch (_) {}
  }

  Future<void> saveS3Config(S3Config config) async {
    _s3Config = config;
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('jiyue_s3_config', jsonEncode(config.toJson()));
  }

  Future<void> saveAiConfig(ExportedAiConfig config) async {
    _aiConfig = config;
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('jiyue_ai_config', jsonEncode(config.toJson()));
  }

  Future<void> saveMemosConfig(ExportedMemosConfig config) async {
    _memosConfig = config;
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('jiyue_memos_config', jsonEncode(config.toJson()));
  }

  void _tryInitFfi() {
    try {
      ffi.DynamicLibrary lib;
      if (Platform.isAndroid) {
        lib = ffi.DynamicLibrary.open('libjiyue_ffi.so');
      } else if (Platform.isIOS) {
        lib = ffi.DynamicLibrary.process();
      } else if (Platform.isWindows) {
        lib = ffi.DynamicLibrary.open('jiyue_ffi.dll');
      } else {
        return;
      }

      _fnOpen = lib.lookupFunction<COpen, DartOpen>('jiyue_vault_open');
      _fnClose = lib.lookupFunction<CClose, DartClose>('jiyue_vault_close');
      _fnList = lib.lookupFunction<CList, DartList>('jiyue_vault_list_notes');
      _fnGet = lib.lookupFunction<CGet, DartGet>('jiyue_vault_get_note');
      _fnSave = lib.lookupFunction<CSave, DartSave>('jiyue_vault_save_note');
      _fnCreate = lib.lookupFunction<CCreate, DartCreate>('jiyue_vault_create_note');
      _fnDelete = lib.lookupFunction<CDelete, DartDelete>('jiyue_vault_delete_note');
      _fnDailyGet = lib.lookupFunction<CDailyGet, DartDailyGet>('jiyue_daily_get_or_create');
      _fnDailyAppend = lib.lookupFunction<CDailyAppend, DartDailyAppend>('jiyue_daily_append');
      _fnFree = lib.lookupFunction<CFree, DartFree>('jiyue_string_free');

      final pathPtr = _vaultDir!.path.toNativeUtf8();
      _ffiBridge = _fnOpen!(pathPtr);
      calloc.free(pathPtr);
      _useFfi = _ffiBridge != null && _ffiBridge != ffi.nullptr;
    } catch (_) {
      _useFfi = false;
    }
  }

  Future<void> _initSampleVault() async {
    final welcome = File('${_vaultDir!.path}/欢迎使用 Jiyue.md');
    await welcome.writeAsString('''# 欢迎使用 霁月 (Jiyue)

霁月 是一款基于纯 Rust 核心构建的高性能极客双向链接笔记系统。

## 核心特性
- [[极客双向链接与图谱]]：使用双括号无缝互联知识节点
- [[S3 与 WebDAV 跨端同步]]：与 PC 端零冲突实时漫游
- **五套经典主题**：墨 (Ink) · 雾 (Fog) · 纸 (Paper) · 苔 (Moss) · 石墨 (Graphite)
- **今日速记**：随时随地单手捕获灵感，支持按时间戳追加日志

> 先记录，再整理。
''');

    final linkSample = File('${_vaultDir!.path}/极客双向链接与图谱.md');
    await linkSample.writeAsString('''# 极客双向链接与图谱

通过双括号标记，可自动抽取为拓扑图谱，轻松建立网状关联：
- 返回 [[欢迎使用 Jiyue]]
- 阅读 [[S3 与 WebDAV 跨端同步]]

## 待办清单
- [x] 完成 Rust FFI C-ABI 跨平台集成
- [x] 移植官方原生 SVG 图标库
- [ ] 体验全屏极夜沉浸式编辑
''');

    final syncSample = File('${_vaultDir!.path}/S3 与 WebDAV 跨端同步.md');
    await syncSample.writeAsString('''# S3 与 WebDAV 跨端同步

移动端与 PC 端共享同一套 `jiyue-sync` Rust 核心，支持标准 S3 协议（AWS, MinIO, Cloudflare R2）以及 WebDAV（坚果云，群晖 Synology NAS）。

- 具备安全三向合并与本地冲突副本机制
- 连接测试通过后方可启动同步任务
''');
  }

  Future<List<NoteSummary>> listNotes() async {
    if (_useFfi && _fnList != null && _ffiBridge != null) {
      final jsonPtr = _fnList!(_ffiBridge!);
      final jsonStr = jsonPtr.toDartString();
      _fnFree!(jsonPtr);
      final List<dynamic> list = jsonDecode(jsonStr);
      return list.map((e) => NoteSummary.fromJson(e as Map<String, dynamic>)).toList();
    }

    // Local file fallback
    final notes = <NoteSummary>[];
    final entities = await _vaultDir!.list().toList();
    for (final ent in entities) {
      if (ent is File && ent.path.endsWith('.md')) {
        final content = await ent.readAsString();
        final stat = await ent.stat();
        final name = ent.uri.pathSegments.last;
        final title = name.endsWith('.md') ? name.substring(0, name.length - 3) : name;
        final preview = content.split('\n').where((l) => l.trim().isNotEmpty && !l.startsWith('#')).take(2).join(' ');
        final tags = RegExp(r'#([^\s#]+)').allMatches(content).map((m) => '#${m.group(1)}').toSet().toList();

        notes.add(NoteSummary(
          id: ent.path,
          title: title,
          path: name,
          preview: preview.length > 80 ? '${preview.substring(0, 80)}...' : preview,
          tags: tags,
          modifiedMs: stat.modified.millisecondsSinceEpoch,
        ));
      }
    }
    notes.sort((a, b) => b.modifiedMs.compareTo(a.modifiedMs));
    return notes;
  }

  Future<NoteDetail?> getNote(String notePath) async {
    if (_useFfi && _fnGet != null && _ffiBridge != null) {
      final pathPtr = notePath.toNativeUtf8();
      final jsonPtr = _fnGet!(_ffiBridge!, pathPtr);
      calloc.free(pathPtr);
      final jsonStr = jsonPtr.toDartString();
      _fnFree!(jsonPtr);
      if (jsonStr == 'null') return null;
      return NoteDetail.fromJson(jsonDecode(jsonStr) as Map<String, dynamic>);
    }

    final file = File('${_vaultDir!.path}/$notePath');
    if (!await file.exists()) return null;
    final content = await file.readAsString();
    final stat = await file.stat();
    final name = file.uri.pathSegments.last;
    final title = name.endsWith('.md') ? name.substring(0, name.length - 3) : name;
    final wikilinks = RegExp(r'\[\[([^\]|]+)(?:\|[^\]]+)?\]\]')
        .allMatches(content)
        .map((m) => m.group(1)!)
        .toSet()
        .toList();
    final tags = RegExp(r'#([^\s#]+)').allMatches(content).map((m) => '#${m.group(1)}').toSet().toList();

    return NoteDetail(
      id: file.path,
      title: title,
      path: notePath,
      content: content,
      tags: tags,
      wikilinks: wikilinks,
      modifiedMs: stat.modified.millisecondsSinceEpoch,
    );
  }

  Future<bool> saveNote(String notePath, String content) async {
    if (_useFfi && _fnSave != null && _ffiBridge != null) {
      final pathPtr = notePath.toNativeUtf8();
      final contentPtr = content.toNativeUtf8();
      final errPtr = _fnSave!(_ffiBridge!, pathPtr, contentPtr);
      calloc.free(pathPtr);
      calloc.free(contentPtr);
      if (errPtr == ffi.nullptr) return true;
      _fnFree!(errPtr);
      return false;
    }

    final file = File('${_vaultDir!.path}/$notePath');
    await file.writeAsString(content);
    return true;
  }

  Future<NoteSummary?> createNote(String title, String content) async {
    if (_useFfi && _fnCreate != null && _ffiBridge != null) {
      final titlePtr = title.toNativeUtf8();
      final contentPtr = content.toNativeUtf8();
      final jsonPtr = _fnCreate!(_ffiBridge!, titlePtr, contentPtr);
      calloc.free(titlePtr);
      calloc.free(contentPtr);
      final jsonStr = jsonPtr.toDartString();
      _fnFree!(jsonPtr);
      if (jsonStr == 'null') return null;
      return NoteSummary.fromJson(jsonDecode(jsonStr) as Map<String, dynamic>);
    }

    final safeTitle = title.trim().replaceAll(RegExp(r'[\\/:*?"<>|#]'), '_');
    final filename = '$safeTitle.md';
    final file = File('${_vaultDir!.path}/$filename');
    await file.writeAsString(content);

    return NoteSummary(
      id: file.path,
      title: safeTitle,
      path: filename,
      preview: content.length > 80 ? '${content.substring(0, 80)}...' : content,
      tags: [],
      modifiedMs: DateTime.now().millisecondsSinceEpoch,
    );
  }

  Future<bool> deleteNote(String notePath) async {
    if (_useFfi && _fnDelete != null && _ffiBridge != null) {
      final pathPtr = notePath.toNativeUtf8();
      final ok = _fnDelete!(_ffiBridge!, pathPtr);
      calloc.free(pathPtr);
      return ok;
    }

    final file = File('${_vaultDir!.path}/$notePath');
    if (await file.exists()) {
      await file.delete();
      return true;
    }
    return false;
  }

  Future<NoteDetail> getOrCreateDailyNote() async {
    if (_useFfi && _fnDailyGet != null && _ffiBridge != null) {
      final jsonPtr = _fnDailyGet!(_ffiBridge!);
      final jsonStr = jsonPtr.toDartString();
      _fnFree!(jsonPtr);
      if (jsonStr != 'null') {
        return NoteDetail.fromJson(jsonDecode(jsonStr) as Map<String, dynamic>);
      }
    }

    final now = DateTime.now();
    final dateStr = '${now.year}-${now.month.toString().padLeft(2, '0')}-${now.day.toString().padLeft(2, '0')}';
    final filename = '$dateStr.md';
    final file = File('${_vaultDir!.path}/$filename');

    if (!await file.exists()) {
      await file.writeAsString('''# $dateStr 每日速记

## 今日待办
- [ ] 梳理移动端核心交互细节

## 速记随想
''');
    }
    return (await getNote(filename))!;
  }

  Future<void> appendToDaily(String text) async {
    if (_useFfi && _fnDailyAppend != null && _ffiBridge != null) {
      final textPtr = text.toNativeUtf8();
      final errPtr = _fnDailyAppend!(_ffiBridge!, textPtr);
      calloc.free(textPtr);
      if (errPtr == ffi.nullptr) return;
      _fnFree!(errPtr);
    }

    final daily = await getOrCreateDailyNote();
    final now = DateTime.now();
    final timeStr = '${now.hour.toString().padLeft(2, '0')}:${now.minute.toString().padLeft(2, '0')}';
    final updated = '${daily.content.trimRight()}\n- [$timeStr] ${text.trim()}\n';
    await saveNote(daily.path, updated);
  }

  void dispose() {
    if (_useFfi && _fnClose != null && _ffiBridge != null) {
      _fnClose!(_ffiBridge!);
      _ffiBridge = null;
    }
  }
}
