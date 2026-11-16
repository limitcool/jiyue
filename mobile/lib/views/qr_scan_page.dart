import 'dart:async';
import 'dart:isolate';
import 'package:camera/camera.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../services/platform_bridge.dart';
import '../services/qr_decoder.dart';
import '../theme/jiyue_theme.dart';
import '../widgets/jiyue_controls.dart';
import '../widgets/jiyue_icon.dart';

class QrScanPage extends StatefulWidget {
  final JiyueThemeColors colors;

  const QrScanPage({super.key, required this.colors});

  @override
  State<QrScanPage> createState() => _QrScanPageState();
}

class _QrScanPageState extends State<QrScanPage> {
  List<CameraDescription> _cameras = const [];
  int _cameraIdx = 0;
  CameraController? _controller;
  String? _cameraError;
  bool _initializing = true;

  bool _isScanned = false;
  bool _decodingFrame = false;
  bool _decodingFile = false;
  bool _torchOn = false;
  DateTime _lastDecodeAt = DateTime.fromMillisecondsSinceEpoch(0);

  @override
  void initState() {
    super.initState();
    _initCamera();
  }

  @override
  void dispose() {
    _disposeController();
    super.dispose();
  }

  Future<void> _disposeController() async {
    final ctrl = _controller;
    _controller = null;
    if (ctrl != null) {
      try {
        if (ctrl.value.isStreamingImages) {
          await ctrl.stopImageStream();
        }
      } catch (_) {}
      await ctrl.dispose();
    }
  }

  Future<void> _initCamera() async {
    setState(() {
      _initializing = true;
      _cameraError = null;
    });
    try {
      final cameras = await availableCameras();
      if (cameras.isEmpty) {
        if (mounted) {
          setState(() {
            _initializing = false;
            _cameraError = '未检测到可用摄像头，可改用相册图片或粘贴代码导入';
          });
        }
        return;
      }

      _cameras = cameras;
      final backIdx = cameras.indexWhere(
        (c) => c.lensDirection == CameraLensDirection.back,
      );
      _cameraIdx = backIdx >= 0 ? backIdx : 0;
      await _startCamera(_cameras[_cameraIdx]);
    } catch (e) {
      if (mounted) {
        setState(() {
          _initializing = false;
          _cameraError = '摄像头初始化失败：$e';
        });
      }
    }
  }

  Future<void> _startCamera(CameraDescription description) async {
    await _disposeController();
    final controller = CameraController(
      description,
      ResolutionPreset.medium,
      enableAudio: false,
      imageFormatGroup: ImageFormatGroup.yuv420,
    );
    try {
      await controller.initialize();
      await controller.startImageStream(_onCameraFrame);
      if (mounted) {
        setState(() {
          _controller = controller;
          _initializing = false;
          _torchOn = false;
        });
      } else {
        await controller.dispose();
      }
    } catch (e) {
      await controller.dispose();
      if (mounted) {
        setState(() {
          _initializing = false;
          _cameraError = '无法开启相机预览：$e';
        });
      }
    }
  }

  void _onCameraFrame(CameraImage image) {
    if (_isScanned || _decodingFrame || image.planes.isEmpty) return;
    final now = DateTime.now();
    if (now.difference(_lastDecodeAt).inMilliseconds < 320) return;

    _lastDecodeAt = now;
    _decodingFrame = true;

    final plane = image.planes.first;
    final yBytes = Uint8List.fromList(plane.bytes);
    final width = image.width;
    final height = image.height;
    final bytesPerRow = plane.bytesPerRow;

    Isolate.run(
      () => QrDecoder.decodeFromYuvPlane(
        yBytes: yBytes,
        width: width,
        height: height,
        bytesPerRow: bytesPerRow,
      ),
    ).then((decoded) {
      _decodingFrame = false;
      if (decoded != null && decoded.isNotEmpty && mounted && !_isScanned) {
        _finishWithPayload(decoded);
      }
    }).catchError((_) {
      _decodingFrame = false;
    });
  }

  void _finishWithPayload(String payload) {
    _isScanned = true;
    HapticFeedback.mediumImpact();
    Navigator.of(context).pop(payload.trim());
  }

  Future<void> _toggleTorch() async {
    final ctrl = _controller;
    if (ctrl == null || !ctrl.value.isInitialized) return;
    try {
      final next = !_torchOn;
      await ctrl.setFlashMode(next ? FlashMode.torch : FlashMode.off);
      if (mounted) {
        setState(() => _torchOn = next);
      }
    } catch (_) {}
  }

  Future<void> _switchCamera() async {
    if (_cameras.length < 2) return;
    final nextIdx = (_cameraIdx + 1) % _cameras.length;
    setState(() {
      _cameraIdx = nextIdx;
      _initializing = true;
    });
    await _startCamera(_cameras[nextIdx]);
  }

  Future<void> _pickQrFromGallery() async {
    if (_decodingFile) return;
    setState(() => _decodingFile = true);
    try {
      final bytes = await PlatformBridge.pickImageBytes();
      if (bytes == null || bytes.isEmpty) {
        if (mounted) setState(() => _decodingFile = false);
        return;
      }
      final decoded = await Isolate.run(
        () => QrDecoder.decodeFromImageBytes(bytes),
      );
      if (!mounted) return;
      setState(() => _decodingFile = false);

      if (decoded != null && decoded.isNotEmpty) {
        _finishWithPayload(decoded);
      } else {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('未能在所选图片中识别到有效二维码')),
        );
      }
    } catch (e) {
      if (mounted) {
        setState(() => _decodingFile = false);
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('读取二维码图片失败：$e')),
        );
      }
    }
  }

  Future<void> _pasteFromClipboard() async {
    final data = await Clipboard.getData(Clipboard.kTextPlain);
    final text = data?.text?.trim();
    if (text != null && text.isNotEmpty) {
      if (mounted) {
        _finishWithPayload(text);
      }
    } else {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('剪贴板中没有文本内容')),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    const cam = JiyueThemeColors.ink;
    final colors = widget.colors;
    final ctrl = _controller;

    return Scaffold(
      backgroundColor: cam.canvas,
      appBar: AppBar(
        backgroundColor: cam.panel,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(1),
          child: JiyueDivider(colors: cam),
        ),
        leading: Center(
          child: JiyueIconButton(
            colors: cam,
            icon: 'chevron-left',
            size: 34,
            iconSize: 18,
            tooltip: '返回',
            onPressed: () => Navigator.of(context).pop(),
          ),
        ),
        title: Text(
          '扫描电脑配置二维码',
          style: TextStyle(
            color: cam.textPrimary,
            fontSize: JiyueTokens.fontLg,
            fontWeight: FontWeight.w600,
          ),
        ),
        actions: [
          JiyueIconButton(
            colors: cam,
            icon: 'bolt',
            size: 32,
            iconSize: 17,
            selected: _torchOn,
            tooltip: '补光灯',
            onPressed: ctrl != null && ctrl.value.isInitialized ? _toggleTorch : null,
          ),
          Padding(
            padding: const EdgeInsets.only(right: 8),
            child: JiyueIconButton(
              colors: cam,
              icon: 'camera-rotate',
              size: 32,
              iconSize: 17,
              tooltip: '切换摄像头',
              onPressed: _cameras.length > 1 ? _switchCamera : null,
            ),
          ),
        ],
      ),
      body: Stack(
        children: [
          if (_initializing)
            Center(
              child: CircularProgressIndicator(color: colors.accent),
            )
          else if (ctrl != null && ctrl.value.isInitialized)
            Positioned.fill(
              child: FittedBox(
                fit: BoxFit.cover,
                child: SizedBox(
                  width: ctrl.value.previewSize?.height ?? 720,
                  height: ctrl.value.previewSize?.width ?? 1280,
                  child: CameraPreview(ctrl),
                ),
              ),
            )
          else
            Center(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: JiyueEmptyState(
                  colors: cam,
                  icon: 'qrcode',
                  title: '相机不可用',
                  subtitle: _cameraError ?? '请通过下方从相册选图或剪贴板导入',
                ),
              ),
            ),
          if (ctrl != null && ctrl.value.isInitialized)
            Center(
              child: Container(
                width: 248,
                height: 248,
                decoration: BoxDecoration(
                  border: Border.all(color: colors.accent, width: 1.5),
                  borderRadius: BorderRadius.circular(JiyueTokens.radiusLg),
                  color: Colors.transparent,
                ),
              ),
            ),
          Positioned(
            left: 20,
            right: 20,
            bottom: 32,
            child: Column(
              children: [
                Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 14,
                    vertical: 8,
                  ),
                  decoration: BoxDecoration(
                    color: cam.panel.withAlpha(225),
                    borderRadius: BorderRadius.circular(JiyueTokens.radiusPill),
                    border: Border.all(color: cam.border, width: 1),
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      JiyueIcon('qrcode', color: colors.accent, size: 14),
                      const SizedBox(width: 8),
                      Flexible(
                        child: Text(
                          '将取景框对准 PC 端「设置 -> 手机扫码同步」二维码',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: cam.textPrimary,
                            fontSize: JiyueTokens.fontSm,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 14),
                Row(
                  children: [
                    Expanded(
                      child: JiyueButton(
                        colors: cam,
                        label: _decodingFile ? '识别中...' : '相册二维码',
                        icon: 'photo',
                        loading: _decodingFile,
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.md,
                        expand: true,
                        onPressed: _decodingFile ? null : _pickQrFromGallery,
                      ),
                    ),
                    const SizedBox(width: 10),
                    Expanded(
                      child: JiyueButton(
                        colors: cam,
                        label: '粘贴配置代码',
                        icon: 'copy',
                        kind: JiyueButtonKind.secondary,
                        size: JiyueControlSize.md,
                        expand: true,
                        onPressed: _pasteFromClipboard,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
