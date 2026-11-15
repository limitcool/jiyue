import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
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
  final MobileScannerController _controller = MobileScannerController(
    detectionSpeed: DetectionSpeed.normal,
    facing: CameraFacing.back,
  );

  bool _isScanned = false;
  bool _torchOn = false;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _onDetect(BarcodeCapture capture) {
    if (_isScanned) return;
    for (final barcode in capture.barcodes) {
      final val = barcode.rawValue;
      if (val != null && val.trim().isNotEmpty) {
        _isScanned = true;
        HapticFeedback.mediumImpact();
        Navigator.of(context).pop(val.trim());
        break;
      }
    }
  }

  Future<void> _pasteFromClipboard() async {
    final data = await Clipboard.getData(Clipboard.kTextPlain);
    final text = data?.text?.trim();
    if (text != null && text.isNotEmpty) {
      if (mounted) {
        Navigator.of(context).pop(text);
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
            onPressed: () async {
              await _controller.toggleTorch();
              setState(() => _torchOn = !_torchOn);
            },
          ),
          Padding(
            padding: const EdgeInsets.only(right: 8),
            child: JiyueIconButton(
              colors: cam,
              icon: 'camera-rotate',
              size: 32,
              iconSize: 17,
              tooltip: '切换摄像头',
              onPressed: () => _controller.switchCamera(),
            ),
          ),
        ],
      ),
      body: Stack(
        children: [
          MobileScanner(
            controller: _controller,
            onDetect: _onDetect,
          ),
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
            bottom: 36,
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
                const SizedBox(height: 16),
                JiyueButton(
                  colors: cam,
                  label: '从剪贴板粘贴配置代码',
                  icon: 'copy',
                  kind: JiyueButtonKind.secondary,
                  size: JiyueControlSize.md,
                  onPressed: _pasteFromClipboard,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
