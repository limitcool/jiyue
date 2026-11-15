import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import '../theme/jiyue_theme.dart';

/// Renders a native Jiyue Lucide/Tabler SVG icon with exact color and size.
class JiyueIcon extends StatelessWidget {
  final String name;
  final Color? color;
  final double size;

  const JiyueIcon(
    this.name, {
    super.key,
    this.color,
    this.size = 20.0,
  });

  @override
  Widget build(BuildContext context) {
    final effectiveColor =
        color ??
        IconTheme.of(context).color ??
        DefaultTextStyle.of(context).style.color ??
        JiyueThemeColors.ink.textPrimary;
    return SvgPicture.asset(
      'assets/icons/$name.svg',
      width: size,
      height: size,
      colorFilter: ColorFilter.mode(effectiveColor, BlendMode.srcIn),
    );
  }
}
