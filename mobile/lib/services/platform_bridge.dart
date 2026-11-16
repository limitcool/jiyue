import 'dart:io';
import 'package:flutter/services.dart';

const MethodChannel _platformChannel =
    MethodChannel('cool.jiyue.jiyue_mobile/platform');

/// Thin cross-platform wrapper around lightweight Android platform intents.
class PlatformBridge {
  static Future<bool> hasStoragePermission() async {
    if (!Platform.isAndroid) return true;
    try {
      return await _platformChannel.invokeMethod<bool>('hasStoragePermission') ??
          false;
    } catch (_) {
      return true;
    }
  }

  static Future<bool> requestStoragePermission() async {
    if (!Platform.isAndroid) return true;
    try {
      return await _platformChannel
              .invokeMethod<bool>('requestStoragePermission') ??
          false;
    } catch (_) {
      return false;
    }
  }

  static Future<String?> pickDirectory() async {
    if (!Platform.isAndroid) return null;
    try {
      final res = await _platformChannel.invokeMethod<String>('pickDirectory');
      if (res == null || res.trim().isEmpty) return null;
      return res.trim();
    } on MissingPluginException {
      return null;
    }
  }

  static Future<Uint8List?> pickImageBytes() async {
    if (!Platform.isAndroid) return null;
    try {
      return await _platformChannel.invokeMethod<Uint8List>('pickImageBytes');
    } on MissingPluginException {
      return null;
    }
  }

  static Future<bool> shareFile(String filePath, {String title = '导出笔记'}) async {
    if (!Platform.isAndroid) return false;
    try {
      return await _platformChannel.invokeMethod<bool>('shareFile', <String, dynamic>{
            'filePath': filePath,
            'title': title,
          }) ??
          false;
    } on MissingPluginException {
      return false;
    }
  }
}
