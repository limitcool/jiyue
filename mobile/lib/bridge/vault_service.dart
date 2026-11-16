import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:isolate';
import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../models/exported_config.dart';
import '../models/note.dart';
import '../models/sync_config.dart';

// FFI Typedefs
typedef COpen = ffi.Pointer<ffi.Void> Function(ffi.Pointer<Utf8> rootPath);
typedef DartOpen = ffi.Pointer<ffi.Void> Function(ffi.Pointer<Utf8> rootPath);

typedef CClose = ffi.Void Function(ffi.Pointer<ffi.Void> bridge);
typedef DartClose = void Function(ffi.Pointer<ffi.Void> bridge);

typedef CReload = ffi.Void Function(ffi.Pointer<ffi.Void> bridge);
typedef DartReload = void Function(ffi.Pointer<ffi.Void> bridge);

typedef CList = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge);
typedef DartList = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge);

typedef CGet =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> path,
    );
typedef DartGet =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> path,
    );

typedef CSave =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> path,
      ffi.Pointer<Utf8> content,
    );
typedef DartSave =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> path,
      ffi.Pointer<Utf8> content,
    );

typedef CCreate =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> title,
      ffi.Pointer<Utf8> content,
    );
typedef DartCreate =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> title,
      ffi.Pointer<Utf8> content,
    );

typedef CDelete =
    ffi.Bool Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> path);
typedef DartDelete =
    bool Function(ffi.Pointer<ffi.Void> bridge, ffi.Pointer<Utf8> path);

typedef CRename =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> path,
      ffi.Pointer<Utf8> newTitle,
    );
typedef DartRename =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> path,
      ffi.Pointer<Utf8> newTitle,
    );

typedef CResolveLink =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> fromPath,
      ffi.Pointer<Utf8> target,
    );
typedef DartResolveLink =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> fromPath,
      ffi.Pointer<Utf8> target,
    );

typedef CSearch =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> query,
    );
typedef DartSearch =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> query,
    );

typedef CDailyGet = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge);
typedef DartDailyGet = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> bridge);

typedef CDailyAppend =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> text,
    );
typedef DartDailyAppend =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<ffi.Void> bridge,
      ffi.Pointer<Utf8> text,
    );

typedef CSyncTest = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8> configJson);
typedef DartSyncTest = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8> configJson);

typedef CSyncRunPath =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8> vaultPath,
      ffi.Pointer<Utf8> configJson,
      ffi.Pointer<Utf8> deviceName,
    );
typedef DartSyncRunPath =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8> vaultPath,
      ffi.Pointer<Utf8> configJson,
      ffi.Pointer<Utf8> deviceName,
    );

typedef CFree = ffi.Void Function(ffi.Pointer<Utf8> ptr);
typedef DartFree = void Function(ffi.Pointer<Utf8> ptr);

ffi.DynamicLibrary? _tryOpenFfiLibrary() {
  try {
    if (Platform.isAndroid) {
      return ffi.DynamicLibrary.open('libjiyue_ffi.so');
    }
    if (Platform.isIOS) {
      return ffi.DynamicLibrary.process();
    }
    if (Platform.isWindows) {
      for (final candidate in const [
        'jiyue_ffi.dll',
        r'target\release\jiyue_ffi.dll',
        r'..\target\release\jiyue_ffi.dll',
        r'target\debug\jiyue_ffi.dll',
        r'..\target\debug\jiyue_ffi.dll',
      ]) {
        if (File(candidate).existsSync()) {
          return ffi.DynamicLibrary.open(candidate);
        }
      }
      return ffi.DynamicLibrary.open('jiyue_ffi.dll');
    }
    if (Platform.isLinux) {
      for (final candidate in const [
        'libjiyue_ffi.so',
        'target/release/libjiyue_ffi.so',
        '../target/release/libjiyue_ffi.so',
      ]) {
        if (File(candidate).existsSync()) {
          return ffi.DynamicLibrary.open(candidate);
        }
      }
      return ffi.DynamicLibrary.open('libjiyue_ffi.so');
    }
    if (Platform.isMacOS) {
      for (final candidate in const [
        'libjiyue_ffi.dylib',
        'target/release/libjiyue_ffi.dylib',
        '../target/release/libjiyue_ffi.dylib',
      ]) {
        if (File(candidate).existsSync()) {
          return ffi.DynamicLibrary.open(candidate);
        }
      }
      return ffi.DynamicLibrary.open('libjiyue_ffi.dylib');
    }
  } catch (_) {}
  return null;
}

ConnectionTestResult _runSyncTestInIsolate(String configJson) {
  final lib = _tryOpenFfiLibrary();
  if (lib == null) {
    return const ConnectionTestResult(
      success: false,
      reachable: false,
      canList: false,
      canWrite: false,
      message: '原生同步引擎未加载，无法测试存储桶读写权限',
    );
  }
  try {
    final fnTest = lib.lookupFunction<CSyncTest, DartSyncTest>(
      'jiyue_sync_test',
    );
    final fnFree = lib.lookupFunction<CFree, DartFree>('jiyue_string_free');
    final cfgPtr = configJson.toNativeUtf8();
    final resPtr = fnTest(cfgPtr);
    calloc.free(cfgPtr);
    final resStr = resPtr.toDartString();
    fnFree(resPtr);
    final map = jsonDecode(resStr) as Map<String, dynamic>;
    return ConnectionTestResult.fromJson(map);
  } catch (e) {
    return ConnectionTestResult(
      success: false,
      reachable: false,
      canList: false,
      canWrite: false,
      message: '原生同步检测调用失败: $e',
    );
  }
}

SyncResultReport _runSyncPathInIsolate(
  String vaultPath,
  String configJson,
  String deviceName,
) {
  final lib = _tryOpenFfiLibrary();
  if (lib == null) {
    return SyncResultReport(
      success: false,
      uploaded: 0,
      downloaded: 0,
      deletedRemote: 0,
      deletedLocal: 0,
      conflicts: 0,
      message: '原生同步引擎未加载，请确保 APK 已打包 libjiyue_ffi.so',
    );
  }
  try {
    final fnSync = lib.lookupFunction<CSyncRunPath, DartSyncRunPath>(
      'jiyue_sync_run_path',
    );
    final fnFree = lib.lookupFunction<CFree, DartFree>('jiyue_string_free');
    final pathPtr = vaultPath.toNativeUtf8();
    final cfgPtr = configJson.toNativeUtf8();
    final devPtr = deviceName.toNativeUtf8();
    final resPtr = fnSync(pathPtr, cfgPtr, devPtr);
    calloc.free(pathPtr);
    calloc.free(cfgPtr);
    calloc.free(devPtr);
    final resStr = resPtr.toDartString();
    fnFree(resPtr);
    final map = jsonDecode(resStr) as Map<String, dynamic>;
    return SyncResultReport.fromJson(map);
  } catch (e) {
    return SyncResultReport(
      success: false,
      uploaded: 0,
      downloaded: 0,
      deletedRemote: 0,
      deletedLocal: 0,
      conflicts: 0,
      message: '原生同步执行失败: $e',
    );
  }
}

class VaultService extends ChangeNotifier {
  static final VaultService instance = VaultService._internal();
  VaultService._internal();

  /// Creates a standalone instance for isolated unit testing.
  @visibleForTesting
  factory VaultService.testable() => VaultService._internal();

  static const String _kVaultProfilesKey = 'jiyue_vault_profiles_v1';
  static const String _kActiveVaultIdKey = 'jiyue_active_vault_id';
  static const String _kLegacyS3ConfigKey = 'jiyue_s3_config';
  static const String _kAiConfigKey = 'jiyue_ai_config';
  static const String _kMemosConfigKey = 'jiyue_memos_config';

  Directory? _vaultDir;
  ffi.Pointer<ffi.Void>? _ffiBridge;
  bool _useFfi = false;

  DartOpen? _fnOpen;
  DartClose? _fnClose;
  DartReload? _fnReload;
  DartList? _fnList;
  DartGet? _fnGet;
  DartSave? _fnSave;
  DartCreate? _fnCreate;
  DartDelete? _fnDelete;
  DartRename? _fnRename;
  DartResolveLink? _fnResolveLink;
  DartSearch? _fnSearch;
  DartDailyGet? _fnDailyGet;
  DartDailyAppend? _fnDailyAppend;
  DartFree? _fnFree;

  List<VaultProfile> _vaults = [];
  String? _activeVaultId;
  ExportedAiConfig? _aiConfig;
  ExportedMemosConfig? _memosConfig;

  List<VaultProfile> get vaults => List.unmodifiable(_vaults);
  bool get hasActiveVault => _vaultDir != null && activeVault != null;

  VaultProfile? get activeVault {
    if (_vaults.isEmpty) return null;
    if (_activeVaultId != null) {
      for (final v in _vaults) {
        if (v.id == _activeVaultId) return v;
      }
    }
    return _vaults.first;
  }

  S3Config? get s3Config => activeVault?.s3Config;
  WebDavConfig? get webdavConfig => activeVault?.webdavConfig;
  ExportedAiConfig? get aiConfig => _aiConfig;
  ExportedMemosConfig? get memosConfig => _memosConfig;
  List<String> get bookmarks => activeVault?.bookmarks ?? const [];
  bool get ffiAvailable => _useFfi;

  String get vaultPath => _vaultDir?.path ?? '';
  String get vaultName =>
      activeVault?.name ??
      _vaultDir?.uri.pathSegments.where((s) => s.isNotEmpty).lastOrNull ??
      '未选择笔记库';

  String _generateId() =>
      'v_${DateTime.now().microsecondsSinceEpoch}_${_vaults.length}';

  String _normalizeRel(String raw) {
    var rel = raw.replaceAll('\\', '/').trim();
    while (rel.startsWith('/')) {
      rel = rel.substring(1);
    }
    return rel;
  }

  File _vaultFile(String relPath) {
    final norm = _normalizeRel(relPath);
    final sep = Platform.pathSeparator;
    final joined = norm.split('/').join(sep);
    return File('${_vaultDir!.path}$sep$joined');
  }

  String _sanitizeTitle(String raw) {
    final clean = raw
        .trim()
        .replaceAll(RegExp(r'[\\/:*?"<>|#\r\n\t]'), '_')
        .replaceAll(RegExp(r'\s+'), ' ')
        .trim();
    return clean;
  }

  String _dirBasename(String path) {
    final clean = path.replaceAll('\\', '/').replaceAll(RegExp(r'/+$'), '');
    if (clean.isEmpty) return '笔记库';
    return clean.split('/').last;
  }

  Future<Directory> _resolveAppDocsDir() async {
    try {
      return await getApplicationDocumentsDirectory();
    } catch (_) {
      return Directory.systemTemp;
    }
  }

  Future<Directory> _allocateVaultDir(String name) async {
    final docsDir = await _resolveAppDocsDir();
    final vaultsRoot = Directory(
      '${docsDir.path}${Platform.pathSeparator}vaults',
    );
    if (!await vaultsRoot.exists()) {
      await vaultsRoot.create(recursive: true);
    }
    final sanitized = _sanitizeTitle(name);
    final baseName = sanitized.isEmpty ? 'vault' : sanitized;
    var candidate = Directory(
      '${vaultsRoot.path}${Platform.pathSeparator}$baseName',
    );
    var suffix = 2;
    while (_vaults.any(
      (v) => _normalizeFsPath(v.path) == _normalizeFsPath(candidate.path),
    )) {
      candidate = Directory(
        '${vaultsRoot.path}${Platform.pathSeparator}${baseName}_$suffix',
      );
      suffix++;
    }
    if (!await candidate.exists()) {
      await candidate.create(recursive: true);
    }
    return candidate;
  }

  String _normalizeFsPath(String raw) {
    var norm = raw.replaceAll('\\', '/').replaceAll(RegExp(r'/+$'), '');
    if (Platform.isWindows) {
      norm = norm.toLowerCase();
    }
    return norm;
  }

  Future<void> init([String? customPath]) async {
    _closeBridge();
    _vaultDir = null;
    _vaults = [];
    _activeVaultId = null;
    _aiConfig = null;
    _memosConfig = null;

    await _loadGlobalConfigs();

    if (customPath != null && customPath.trim().isNotEmpty) {
      final dir = Directory(customPath.trim());
      if (!await dir.exists()) {
        await dir.create(recursive: true);
      }
      final normTarget = _normalizeFsPath(dir.path);
      final existing =
          _vaults
              .where((v) => _normalizeFsPath(v.path) == normTarget)
              .firstOrNull;
      final profile =
          existing ??
          VaultProfile(
            id: _generateId(),
            name: _dirBasename(dir.path),
            path: dir.path,
            isExternal: true,
          );
      if (existing == null) {
        _vaults.add(profile);
        await _persistProfiles();
      }
      await _activateProfile(profile, persist: existing != null);
      return;
    }

    if (_vaults.isEmpty) {
      await _migrateLegacyVaultIfPresent();
    }

    final target = activeVault;
    if (target != null) {
      await _activateProfile(target, persist: false);
    } else {
      notifyListeners();
    }
  }

  Future<void> _loadGlobalConfigs() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final profilesJson = prefs.getString(_kVaultProfilesKey);
      if (profilesJson != null && profilesJson.trim().isNotEmpty) {
        final rawList = jsonDecode(profilesJson) as List<dynamic>;
        _vaults =
            rawList
                .whereType<Map<String, dynamic>>()
                .map(VaultProfile.fromJson)
                .where((v) => v.path.trim().isNotEmpty)
                .toList();
      }
      _activeVaultId = prefs.getString(_kActiveVaultIdKey);

      final aiJson = prefs.getString(_kAiConfigKey);
      if (aiJson != null) {
        _aiConfig = ExportedAiConfig.fromJson(
          jsonDecode(aiJson) as Map<String, dynamic>,
        );
      }
      final memosJson = prefs.getString(_kMemosConfigKey);
      if (memosJson != null) {
        _memosConfig = ExportedMemosConfig.fromJson(
          jsonDecode(memosJson) as Map<String, dynamic>,
        );
      }
    } catch (_) {}
  }

  Future<void> _migrateLegacyVaultIfPresent() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final legacyS3Json = prefs.getString(_kLegacyS3ConfigKey);
      S3Config? legacyS3;
      if (legacyS3Json != null && legacyS3Json.trim().isNotEmpty) {
        legacyS3 = S3Config.fromJson(
          jsonDecode(legacyS3Json) as Map<String, dynamic>,
        );
      }

      final docDir = await _resolveAppDocsDir();
      final legacyDir = Directory(
        '${docDir.path}${Platform.pathSeparator}Jiyue',
      );
      var hasLegacyNotes = false;
      if (await legacyDir.exists()) {
        await for (final ent in legacyDir.list(recursive: false)) {
          final base = _dirBasename(ent.path);
          if (!base.startsWith('.')) {
            hasLegacyNotes = true;
            break;
          }
        }
      }

      if (legacyS3 != null || hasLegacyNotes) {
        if (!await legacyDir.exists()) {
          await legacyDir.create(recursive: true);
        }
        final migrated = VaultProfile(
          id: _generateId(),
          name: 'Jiyue',
          path: legacyDir.path,
          isExternal: false,
          s3Config: legacyS3,
        );
        _vaults = [migrated];
        _activeVaultId = migrated.id;
        await _persistProfiles();
      }
    } catch (_) {}
  }

  Future<void> _persistProfiles() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final encoded = jsonEncode(_vaults.map((v) => v.toJson()).toList());
      await prefs.setString(_kVaultProfilesKey, encoded);
      if (_activeVaultId != null) {
        await prefs.setString(_kActiveVaultIdKey, _activeVaultId!);
      } else {
        await prefs.remove(_kActiveVaultIdKey);
      }
    } catch (_) {}
  }

  Future<void> _activateProfile(
    VaultProfile profile, {
    bool persist = true,
  }) async {
    _closeBridge();
    final dir = Directory(profile.path);
    if (!await dir.exists()) {
      await dir.create(recursive: true);
    }
    _vaultDir = dir;
    _activeVaultId = profile.id;
    _tryInitFfi();
    if (persist) {
      await _persistProfiles();
    }
    notifyListeners();
  }

  Future<VaultProfile> createLocalVault(String name) async {
    final cleanName =
        name.trim().isEmpty ? '我的笔记库' : name.trim();
    final dir = await _allocateVaultDir(cleanName);
    final profile = VaultProfile(
      id: _generateId(),
      name: cleanName,
      path: dir.path,
      isExternal: false,
    );
    _vaults.add(profile);
    await _activateProfile(profile);
    return profile;
  }

  Future<VaultProfile> openExternalVault(
    String directoryPath, {
    String? name,
  }) async {
    final trimmed = directoryPath.trim();
    final dir = Directory(trimmed);
    if (!await dir.exists()) {
      await dir.create(recursive: true);
    }
    final normTarget = _normalizeFsPath(dir.path);
    final existingIdx = _vaults.indexWhere(
      (v) => _normalizeFsPath(v.path) == normTarget,
    );
    if (existingIdx >= 0) {
      final existing = _vaults[existingIdx];
      final updated =
          (name != null && name.trim().isNotEmpty)
              ? existing.copyWith(name: name.trim())
              : existing;
      _vaults[existingIdx] = updated;
      await _activateProfile(updated);
      return updated;
    }

    final displayName =
        (name != null && name.trim().isNotEmpty)
            ? name.trim()
            : _dirBasename(dir.path);
    final profile = VaultProfile(
      id: _generateId(),
      name: displayName,
      path: dir.path,
      isExternal: true,
    );
    _vaults.add(profile);
    await _activateProfile(profile);
    return profile;
  }

  Future<VaultProfile> importCloudVault({
    required String name,
    S3Config? s3,
    WebDavConfig? webdav,
  }) async {
    final cleanName = name.trim().isEmpty ? '云端笔记库' : name.trim();
    final dir = await _allocateVaultDir(cleanName);
    final profile = VaultProfile(
      id: _generateId(),
      name: cleanName,
      path: dir.path,
      isExternal: false,
      s3Config: s3,
      webdavConfig: s3 == null ? webdav : null,
    );
    _vaults.add(profile);
    await _activateProfile(profile);
    return profile;
  }

  Future<void> switchVault(String vaultId) async {
    final idx = _vaults.indexWhere((v) => v.id == vaultId);
    if (idx < 0) return;
    await _activateProfile(_vaults[idx]);
  }

  Future<void> removeVault(
    String vaultId, {
    bool deleteLocalFiles = false,
  }) async {
    final idx = _vaults.indexWhere((v) => v.id == vaultId);
    if (idx < 0) return;
    final removed = _vaults.removeAt(idx);
    final wasActive = _activeVaultId == vaultId;

    if (deleteLocalFiles && !removed.isExternal) {
      try {
        final dir = Directory(removed.path);
        if (await dir.exists()) {
          await dir.delete(recursive: true);
        }
      } catch (_) {}
    }

    if (wasActive) {
      _closeBridge();
      _vaultDir = null;
      _activeVaultId = _vaults.isNotEmpty ? _vaults.first.id : null;
      if (_vaults.isNotEmpty) {
        await _activateProfile(_vaults.first);
        return;
      }
    }
    await _persistProfiles();
    notifyListeners();
  }

  void _updateActiveProfile(VaultProfile Function(VaultProfile current) transform) {
    final cur = activeVault;
    if (cur == null) return;
    final idx = _vaults.indexWhere((v) => v.id == cur.id);
    if (idx < 0) return;
    _vaults[idx] = transform(_vaults[idx]);
  }

  Future<void> saveS3Config(S3Config config) async {
    if (activeVault == null) {
      await importCloudVault(name: '云端笔记库', s3: config);
      return;
    }
    _updateActiveProfile(
      (v) => v.copyWith(s3Config: config, clearWebdavConfig: true),
    );
    await _persistProfiles();
    notifyListeners();
  }

  Future<void> saveWebDavConfig(WebDavConfig config) async {
    if (activeVault == null) {
      await importCloudVault(name: 'WebDAV 笔记库', webdav: config);
      return;
    }
    _updateActiveProfile(
      (v) => v.copyWith(webdavConfig: config, clearS3Config: true),
    );
    await _persistProfiles();
    notifyListeners();
  }

  Future<void> clearSyncConfig() async {
    if (activeVault == null) return;
    _updateActiveProfile(
      (v) => v.copyWith(
        clearS3Config: true,
        clearWebdavConfig: true,
        clearLastSyncMs: true,
      ),
    );
    await _persistProfiles();
    notifyListeners();
  }

  Future<void> saveAiConfig(ExportedAiConfig config) async {
    _aiConfig = config;
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_kAiConfigKey, jsonEncode(config.toJson()));
    notifyListeners();
  }

  Future<void> saveMemosConfig(ExportedMemosConfig config) async {
    _memosConfig = config;
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_kMemosConfigKey, jsonEncode(config.toJson()));
    notifyListeners();
  }

  bool isBookmarked(String notePath) {
    final norm = _normalizeRel(notePath);
    return bookmarks.any((b) => _normalizeRel(b) == norm);
  }

  Future<bool> toggleBookmark(String notePath) async {
    final norm = _normalizeRel(notePath);
    final list = List<String>.from(bookmarks);
    final existingIdx = list.indexWhere((b) => _normalizeRel(b) == norm);
    bool added;
    if (existingIdx >= 0) {
      list.removeAt(existingIdx);
      added = false;
    } else {
      list.add(norm);
      added = true;
    }
    _updateActiveProfile((v) => v.copyWith(bookmarks: list));
    await _persistProfiles();
    notifyListeners();
    return added;
  }

  void _tryInitFfi() {
    if (_vaultDir == null) {
      _useFfi = false;
      return;
    }
    final lib = _tryOpenFfiLibrary();
    if (lib == null) {
      _useFfi = false;
      return;
    }
    try {
      _fnOpen = lib.lookupFunction<COpen, DartOpen>('jiyue_vault_open');
      _fnClose = lib.lookupFunction<CClose, DartClose>('jiyue_vault_close');
      _fnReload = lib.lookupFunction<CReload, DartReload>('jiyue_vault_reload');
      _fnList = lib.lookupFunction<CList, DartList>('jiyue_vault_list_notes');
      _fnGet = lib.lookupFunction<CGet, DartGet>('jiyue_vault_get_note');
      _fnSave = lib.lookupFunction<CSave, DartSave>('jiyue_vault_save_note');
      _fnCreate = lib.lookupFunction<CCreate, DartCreate>(
        'jiyue_vault_create_note',
      );
      _fnDelete = lib.lookupFunction<CDelete, DartDelete>(
        'jiyue_vault_delete_note',
      );
      _fnRename = lib.lookupFunction<CRename, DartRename>(
        'jiyue_vault_rename_note',
      );
      _fnResolveLink = lib.lookupFunction<CResolveLink, DartResolveLink>(
        'jiyue_vault_resolve_link',
      );
      _fnSearch = lib.lookupFunction<CSearch, DartSearch>('jiyue_vault_search');
      _fnDailyGet = lib.lookupFunction<CDailyGet, DartDailyGet>(
        'jiyue_daily_get_or_create',
      );
      _fnDailyAppend = lib.lookupFunction<CDailyAppend, DartDailyAppend>(
        'jiyue_daily_append',
      );
      _fnFree = lib.lookupFunction<CFree, DartFree>('jiyue_string_free');

      final pathPtr = _vaultDir!.path.toNativeUtf8();
      _ffiBridge = _fnOpen!(pathPtr);
      calloc.free(pathPtr);
      _useFfi = _ffiBridge != null && _ffiBridge != ffi.nullptr;
    } catch (_) {
      _useFfi = false;
    }
  }

  void _closeBridge() {
    if (_useFfi && _fnClose != null && _ffiBridge != null) {
      _fnClose!(_ffiBridge!);
    }
    _ffiBridge = null;
    _useFfi = false;
  }

  Future<ConnectionTestResult> testSyncConnection(
    Map<String, dynamic> storeConfigJson,
  ) {
    final encoded = jsonEncode(storeConfigJson);
    return Isolate.run(() => _runSyncTestInIsolate(encoded));
  }

  Future<SyncResultReport> executeSync({String deviceName = 'Android'}) async {
    if (_vaultDir == null || activeVault == null) {
      return SyncResultReport(
        success: false,
        uploaded: 0,
        downloaded: 0,
        deletedRemote: 0,
        deletedLocal: 0,
        conflicts: 0,
        message: '当前未打开任何笔记库',
      );
    }
    final storeMap = activeVault!.activeStoreConfigJson;
    if (storeMap == null) {
      return SyncResultReport(
        success: false,
        uploaded: 0,
        downloaded: 0,
        deletedRemote: 0,
        deletedLocal: 0,
        conflicts: 0,
        message: '当前笔记库尚未配置 S3 或 WebDAV 同步参数',
      );
    }

    final rootPath = _vaultDir!.path;
    final configJson = jsonEncode(storeMap);
    final report = await Isolate.run(
      () => _runSyncPathInIsolate(rootPath, configJson, deviceName),
    );

    if (report.success) {
      if (_useFfi && _fnReload != null && _ffiBridge != null) {
        _fnReload!(_ffiBridge!);
      }
      _updateActiveProfile(
        (v) => v.copyWith(lastSyncMs: DateTime.now().millisecondsSinceEpoch),
      );
      await _persistProfiles();
      notifyListeners();
    }
    return report;
  }

  bool _isIgnoredRelPath(String relPath) {
    final parts = relPath.split('/');
    for (final part in parts) {
      if (part.isEmpty || part.startsWith('.')) return true;
    }
    return false;
  }

  Future<List<(String relPath, File file)>> _scanMarkdownFiles() async {
    if (_vaultDir == null || !await _vaultDir!.exists()) return [];
    final rootPrefix = _normalizeFsPath(_vaultDir!.path);
    final out = <(String, File)>[];

    await for (final ent in _vaultDir!.list(
      recursive: true,
      followLinks: false,
    )) {
      if (ent is! File) continue;
      if (!ent.path.toLowerCase().endsWith('.md')) continue;
      final fullNorm = _normalizeFsPath(ent.path);
      if (!fullNorm.startsWith('$rootPrefix/')) continue;
      final relRaw =
          ent.path.replaceAll('\\', '/').substring(rootPrefix.length + 1);
      final rel = _normalizeRel(relRaw);
      if (_isIgnoredRelPath(rel)) continue;
      out.add((rel, ent));
    }
    return out;
  }

  String _titleFromRelPath(String relPath) {
    final base = _normalizeRel(relPath).split('/').last;
    return base.toLowerCase().endsWith('.md')
        ? base.substring(0, base.length - 3)
        : base;
  }

  List<String> _extractTagsDart(String content) {
    return RegExp(r'(?:^|\s)#([^\s#\[\]]+)')
        .allMatches(content)
        .map((m) => '#${m.group(1)}')
        .toSet()
        .toList();
  }

  List<String> _extractWikilinksDart(String content) {
    return RegExp(r'\[\[([^\]|]+)(?:\|[^\]]+)?\]\]')
        .allMatches(content)
        .map((m) => m.group(1)!)
        .toSet()
        .toList();
  }

  Future<List<NoteSummary>> listNotes() async {
    if (_vaultDir == null) return [];
    if (_useFfi && _fnList != null && _ffiBridge != null) {
      final jsonPtr = _fnList!(_ffiBridge!);
      final jsonStr = jsonPtr.toDartString();
      _fnFree!(jsonPtr);
      final List<dynamic> list = jsonDecode(jsonStr);
      return list
          .map((e) => NoteSummary.fromJson(e as Map<String, dynamic>))
          .toList();
    }

    final files = await _scanMarkdownFiles();
    final notes = <NoteSummary>[];
    for (final (rel, file) in files) {
      try {
        final content = await file.readAsString();
        final stat = await file.stat();
        final title = _titleFromRelPath(rel);
        final preview = content
            .split('\n')
            .where((l) => l.trim().isNotEmpty && !l.trim().startsWith('#'))
            .take(3)
            .join(' ');
        final tags = _extractTagsDart(content);

        notes.add(
          NoteSummary(
            id: rel,
            title: title,
            path: rel,
            preview:
                preview.length > 100
                    ? '${preview.substring(0, 100)}...'
                    : preview,
            tags: tags,
            modifiedMs: stat.modified.millisecondsSinceEpoch,
          ),
        );
      } catch (_) {}
    }
    notes.sort((a, b) => b.modifiedMs.compareTo(a.modifiedMs));
    return notes;
  }

  Future<NoteDetail?> getNote(String notePath) async {
    if (_vaultDir == null) return null;
    final norm = _normalizeRel(notePath);
    if (_useFfi && _fnGet != null && _ffiBridge != null) {
      final pathPtr = norm.toNativeUtf8();
      final jsonPtr = _fnGet!(_ffiBridge!, pathPtr);
      calloc.free(pathPtr);
      final jsonStr = jsonPtr.toDartString();
      _fnFree!(jsonPtr);
      if (jsonStr == 'null') return null;
      return NoteDetail.fromJson(jsonDecode(jsonStr) as Map<String, dynamic>);
    }

    final file = _vaultFile(norm);
    if (!await file.exists()) return null;
    final content = await file.readAsString();
    final stat = await file.stat();
    final title = _titleFromRelPath(norm);
    final wikilinks = _extractWikilinksDart(content);
    final tags = _extractTagsDart(content);
    final backlinks = await _collectBacklinksDart(norm);

    return NoteDetail(
      id: norm,
      title: title,
      path: norm,
      content: content,
      tags: tags,
      wikilinks: wikilinks,
      backlinks: backlinks,
      modifiedMs: stat.modified.millisecondsSinceEpoch,
    );
  }

  Future<List<BacklinkItem>> _collectBacklinksDart(String targetNorm) async {
    final files = await _scanMarkdownFiles();
    final targetLower = targetNorm.toLowerCase();
    final allPaths = files.map((e) => e.$1).toList();
    final backlinks = <BacklinkItem>[];

    for (final (srcRel, srcFile) in files) {
      if (srcRel.toLowerCase() == targetLower) continue;
      try {
        final content = await srcFile.readAsString();
        String? matchedLine;
        for (final line in content.split('\n')) {
          final wikiMatches = RegExp(
            r'\[\[([^\]|]+)(?:\|[^\]]+)?\]\]',
          ).allMatches(line);
          for (final m in wikiMatches) {
            final resolved = _resolveTargetDart(allPaths, srcRel, m.group(1)!);
            if (resolved != null && resolved.toLowerCase() == targetLower) {
              matchedLine = line.trim();
              break;
            }
          }
          if (matchedLine != null) break;

          final mdMatches = RegExp(
            r'\[[^\]]*\]\(([^)]+)\)',
          ).allMatches(line);
          for (final m in mdMatches) {
            final target = m.group(1)!.trim();
            if (target.startsWith('http://') || target.startsWith('https://')) {
              continue;
            }
            final resolved = _resolveTargetDart(allPaths, srcRel, target);
            if (resolved != null && resolved.toLowerCase() == targetLower) {
              matchedLine = line.trim();
              break;
            }
          }
          if (matchedLine != null) break;
        }
        if (matchedLine != null) {
          backlinks.add(
            BacklinkItem(
              path: srcRel,
              title: _titleFromRelPath(srcRel),
              snippet:
                  matchedLine.length > 140
                      ? '${matchedLine.substring(0, 140)}...'
                      : matchedLine,
            ),
          );
        }
      } catch (_) {}
    }
    return backlinks;
  }

  Future<bool> saveNote(String notePath, String content) async {
    if (_vaultDir == null) return false;
    final norm = _normalizeRel(notePath);
    if (_useFfi && _fnSave != null && _ffiBridge != null) {
      final pathPtr = norm.toNativeUtf8();
      final contentPtr = content.toNativeUtf8();
      final errPtr = _fnSave!(_ffiBridge!, pathPtr, contentPtr);
      calloc.free(pathPtr);
      calloc.free(contentPtr);
      if (errPtr == ffi.nullptr) return true;
      _fnFree!(errPtr);
      return false;
    }

    final file = _vaultFile(norm);
    if (!await file.parent.exists()) {
      await file.parent.create(recursive: true);
    }
    await file.writeAsString(content);
    return true;
  }

  Future<NoteSummary?> createNote(String title, String content) async {
    if (_vaultDir == null) return null;
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

    var safeTitle = _sanitizeTitle(title);
    if (safeTitle.isEmpty) {
      safeTitle = '未命名';
    }
    final filename = '$safeTitle.md';
    final file = _vaultFile(filename);
    await file.writeAsString(content);

    return NoteSummary(
      id: filename,
      title: safeTitle,
      path: filename,
      preview: content.length > 100 ? '${content.substring(0, 100)}...' : content,
      tags: _extractTagsDart(content),
      modifiedMs: DateTime.now().millisecondsSinceEpoch,
    );
  }

  Future<bool> deleteNote(String notePath) async {
    if (_vaultDir == null) return false;
    final norm = _normalizeRel(notePath);
    bool deleted = false;
    if (_useFfi && _fnDelete != null && _ffiBridge != null) {
      final pathPtr = norm.toNativeUtf8();
      deleted = _fnDelete!(_ffiBridge!, pathPtr);
      calloc.free(pathPtr);
    } else {
      final file = _vaultFile(norm);
      if (await file.exists()) {
        await file.delete();
        deleted = true;
      }
    }

    if (deleted && isBookmarked(norm)) {
      final updated =
          bookmarks.where((b) => _normalizeRel(b) != norm).toList();
      _updateActiveProfile((v) => v.copyWith(bookmarks: updated));
      await _persistProfiles();
      notifyListeners();
    }
    return deleted;
  }

  Future<String?> renameNote(String notePath, String newTitle) async {
    if (_vaultDir == null) return null;
    final norm = _normalizeRel(notePath);
    String? renamedPath;

    if (_useFfi && _fnRename != null && _ffiBridge != null) {
      final pathPtr = norm.toNativeUtf8();
      final titlePtr = newTitle.toNativeUtf8();
      final resPtr = _fnRename!(_ffiBridge!, pathPtr, titlePtr);
      calloc.free(pathPtr);
      calloc.free(titlePtr);
      if (resPtr != ffi.nullptr) {
        renamedPath = _normalizeRel(resPtr.toDartString());
        _fnFree!(resPtr);
      }
    } else {
      final safeTitle = _sanitizeTitle(newTitle);
      if (safeTitle.isEmpty) return null;
      final oldFile = _vaultFile(norm);
      if (!await oldFile.exists()) return null;

      final slash = norm.lastIndexOf('/');
      final newRel =
          slash >= 0 ? '${norm.substring(0, slash)}/$safeTitle.md' : '$safeTitle.md';
      if (newRel == norm) return newRel;

      final newFile = _vaultFile(newRel);
      if (newRel.toLowerCase() != norm.toLowerCase() && await newFile.exists()) {
        return null;
      }
      await oldFile.rename(newFile.path);
      renamedPath = newRel;
    }

    if (renamedPath != null && isBookmarked(norm)) {
      final updated =
          bookmarks
              .map((b) => _normalizeRel(b) == norm ? renamedPath! : b)
              .toList();
      _updateActiveProfile((v) => v.copyWith(bookmarks: updated));
      await _persistProfiles();
      notifyListeners();
    }
    return renamedPath;
  }

  Future<String?> resolveLink(String fromPath, String target) async {
    if (_vaultDir == null) return null;
    final cleanTarget = target.trim();
    if (cleanTarget.isEmpty) return null;
    final normFrom = _normalizeRel(fromPath);

    if (_useFfi && _fnResolveLink != null && _ffiBridge != null) {
      final fromPtr = normFrom.toNativeUtf8();
      final tgtPtr = cleanTarget.toNativeUtf8();
      final resPtr = _fnResolveLink!(_ffiBridge!, fromPtr, tgtPtr);
      calloc.free(fromPtr);
      calloc.free(tgtPtr);
      final jsonStr = resPtr.toDartString();
      _fnFree!(resPtr);
      final decoded = jsonDecode(jsonStr);
      return decoded is String ? _normalizeRel(decoded) : null;
    }

    final files = await _scanMarkdownFiles();
    final allPaths = files.map((e) => e.$1).toList();
    return _resolveTargetDart(allPaths, normFrom, cleanTarget);
  }

  String? _resolveTargetDart(
    List<String> allPaths,
    String fromRel,
    String rawTarget,
  ) {
    final beforePipe = rawTarget.split('|').first;
    final beforeHash = beforePipe.split('#').first.trim();
    if (beforeHash.isEmpty) return fromRel;

    final decoded = Uri.decodeFull(beforeHash).replaceAll('\\', '/');
    var clean = decoded.trim();
    if (clean.startsWith('./')) clean = clean.substring(2);
    while (clean.startsWith('/')) {
      clean = clean.substring(1);
    }
    if (clean.isEmpty) return null;

    String? findCi(String candidate) {
      final low = candidate.toLowerCase();
      for (final p in allPaths) {
        if (p.toLowerCase() == low) return p;
      }
      return null;
    }

    // 1. Resolve relative `../` or relative to current folder
    final slash = fromRel.lastIndexOf('/');
    final fromDir = slash >= 0 ? fromRel.substring(0, slash) : '';
    if (clean.startsWith('../') || fromDir.isNotEmpty) {
      final segs = fromDir.isEmpty ? <String>[] : fromDir.split('/');
      for (final part in clean.split('/')) {
        if (part.isEmpty || part == '.') continue;
        if (part == '..') {
          if (segs.isNotEmpty) segs.removeLast();
        } else {
          segs.add(part);
        }
      }
      final relJoined = segs.join('/');
      final hit =
          findCi(relJoined) ??
          (relJoined.toLowerCase().endsWith('.md')
              ? null
              : findCi('$relJoined.md'));
      if (hit != null) return hit;
    }

    // 2. Exact vault-relative path
    final directHit =
        findCi(clean) ??
        (clean.toLowerCase().endsWith('.md') ? null : findCi('$clean.md'));
    if (directHit != null) return directHit;

    // 3. Suffix or basename match
    final targetWithMd =
        clean.toLowerCase().endsWith('.md') ? clean : '$clean.md';
    final targetWithMdLow = targetWithMd.toLowerCase();
    for (final p in allPaths) {
      final pLow = p.toLowerCase();
      if (pLow.endsWith('/$targetWithMdLow') ||
          _titleFromRelPath(p).toLowerCase() == clean.toLowerCase()) {
        return p;
      }
    }

    // 4. Folder README / index / _indexes fallback
    final base = clean.split('/').last.trim();
    for (final folder in _extractFolders(allPaths)) {
      final folderBase = folder.split('/').last;
      if (folder.toLowerCase() == clean.toLowerCase() ||
          folderBase.toLowerCase() == clean.toLowerCase()) {
        for (final cand in [
          '$folder/$folderBase.md',
          '$folder/README.md',
          '$folder/index.md',
        ]) {
          final hit = findCi(cand);
          if (hit != null) return hit;
        }
      }
    }
    return findCi('_indexes/$base.md');
  }

  List<String> _extractFolders(List<String> paths) {
    final folders = <String>{};
    for (final p in paths) {
      var cur = p;
      while (cur.contains('/')) {
        cur = cur.substring(0, cur.lastIndexOf('/'));
        if (cur.isNotEmpty) folders.add(cur);
      }
    }
    final sorted = folders.toList()..sort((a, b) => a.length.compareTo(b.length));
    return sorted;
  }

  Future<List<SearchResultItem>> searchNotes(String query) async {
    if (_vaultDir == null || query.trim().isEmpty) return [];
    if (_useFfi && _fnSearch != null && _ffiBridge != null) {
      final qPtr = query.toNativeUtf8();
      final jsonPtr = _fnSearch!(_ffiBridge!, qPtr);
      calloc.free(qPtr);
      final jsonStr = jsonPtr.toDartString();
      _fnFree!(jsonPtr);
      final List<dynamic> list = jsonDecode(jsonStr);
      return list
          .map((e) => SearchResultItem.fromJson(e as Map<String, dynamic>))
          .toList();
    }

    final q = query.trim().toLowerCase();
    final files = await _scanMarkdownFiles();
    final results = <SearchResultItem>[];
    for (final (rel, file) in files) {
      final title = _titleFromRelPath(rel);
      if (title.toLowerCase().contains(q)) {
        results.add(
          SearchResultItem(
            path: rel,
            title: title,
            snippet: '标题匹配: $title',
            matchType: 'title',
          ),
        );
        continue;
      }
      try {
        final content = await file.readAsString();
        final idx = content.toLowerCase().indexOf(q);
        if (idx >= 0) {
          final start = idx > 24 ? idx - 24 : 0;
          final end =
              (idx + q.length + 36) < content.length
                  ? idx + q.length + 36
                  : content.length;
          final snippet = content.substring(start, end).replaceAll('\n', ' ');
          results.add(
            SearchResultItem(
              path: rel,
              title: title,
              snippet: '...$snippet...',
              matchType: 'content',
            ),
          );
        }
      } catch (_) {}
    }
    return results;
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
    final dateStr =
        '${now.year}-${now.month.toString().padLeft(2, '0')}-${now.day.toString().padLeft(2, '0')}';
    final rootLegacy = _vaultFile('$dateStr.md');
    final relPath =
        await rootLegacy.exists() ? '$dateStr.md' : 'Daily/$dateStr.md';
    final file = _vaultFile(relPath);

    if (!await file.exists()) {
      if (!await file.parent.exists()) {
        await file.parent.create(recursive: true);
      }
      await file.writeAsString('# $dateStr\n\n');
    }
    return (await getNote(relPath))!;
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
    final timeStr =
        '${now.hour.toString().padLeft(2, '0')}:${now.minute.toString().padLeft(2, '0')}';
    final updated = '${daily.content.trimRight()}\n- [$timeStr] ${text.trim()}\n';
    await saveNote(daily.path, updated);
  }

  @override
  void dispose() {
    _closeBridge();
    super.dispose();
  }
}
