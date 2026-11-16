import 'dart:math' as math;
import 'dart:typed_data';
import 'package:image/image.dart' as img;
import 'package:zxing2/qrcode.dart';

/// Open-source QR code decoder powered by `zxing2` and `package:image`.
class QrDecoder {
  /// Decodes a QR code from encoded image bytes (PNG, JPEG, WebP, etc.).
  static String? decodeFromImageBytes(Uint8List bytes) {
    if (bytes.isEmpty) return null;
    final decoded = img.decodeImage(bytes);
    if (decoded == null) return null;

    var work = decoded;
    final maxDim = math.max(work.width, work.height);
    if (maxDim > 1200) {
      if (work.width >= work.height) {
        work = img.copyResize(work, width: 1200);
      } else {
        work = img.copyResize(work, height: 1200);
      }
    }

    final w = work.width;
    final h = work.height;
    final luminances = Int8List(w * h);
    final maxVal = work.maxChannelValue > 0 ? work.maxChannelValue.toInt() : 255;

    var idx = 0;
    for (var y = 0; y < h; y++) {
      for (var x = 0; x < w; x++) {
        final pixel = work.getPixel(x, y);
        final r = maxVal == 255 ? pixel.r.toInt() : (pixel.r * 255 ~/ maxVal);
        final g = maxVal == 255 ? pixel.g.toInt() : (pixel.g * 255 ~/ maxVal);
        final b = maxVal == 255 ? pixel.b.toInt() : (pixel.b * 255 ~/ maxVal);
        luminances[idx++] = ((r + (g << 1) + b) >> 2);
      }
    }

    final source = RGBLuminanceSource.crop(luminances, w, h, 0, 0, w, h);
    return _decodeLuminanceSource(source, tryInverted: true);
  }

  /// Decodes a QR code directly from a camera YUV420 / NV21 luma (Y) plane.
  static String? decodeFromYuvPlane({
    required Uint8List yBytes,
    required int width,
    required int height,
    required int bytesPerRow,
  }) {
    if (width <= 0 || height <= 0 || yBytes.isEmpty) return null;
    final luminances = Int8List(width * height);

    for (var y = 0; y < height; y++) {
      final rowStart = y * bytesPerRow;
      if (rowStart + width > yBytes.length) break;
      final dstStart = y * width;
      for (var x = 0; x < width; x++) {
        luminances[dstStart + x] = yBytes[rowStart + x];
      }
    }

    final source = RGBLuminanceSource.crop(luminances, width, height, 0, 0, width, height);
    return _decodeLuminanceSource(source, tryInverted: false);
  }

  static String? _decodeLuminanceSource(
    LuminanceSource source, {
    required bool tryInverted,
  }) {
    final reader = QRCodeReader();
    final hints = DecodeHints()..put(DecodeHintType.tryHarder);

    String? tryDecode(LuminanceSource src) {
      try {
        final bitmap = BinaryBitmap(HybridBinarizer(src));
        final text = reader.decode(bitmap, hints: hints).text.trim();
        if (text.isNotEmpty) return text;
      } catch (_) {}

      try {
        final bitmap = BinaryBitmap(GlobalHistogramBinarizer(src));
        final text = reader.decode(bitmap, hints: hints).text.trim();
        if (text.isNotEmpty) return text;
      } catch (_) {}
      return null;
    }

    final direct = tryDecode(source);
    if (direct != null) return direct;
    if (tryInverted) {
      return tryDecode(source.invert());
    }
    return null;
  }
}
