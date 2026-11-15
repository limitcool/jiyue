import 'package:flutter/material.dart';
import '../theme/jiyue_theme.dart';
import 'jiyue_icon.dart';

enum JiyueButtonKind { primary, secondary, ghost, danger }

enum JiyueControlSize { lg, md, sm, xs }

enum JiyueBadgeTone { neutral, accent, ok, warn, danger }

(Color fg, Color bg, Color border) _resolveToneColors(
  JiyueThemeColors colors,
  JiyueBadgeTone tone,
) {
  switch (tone) {
    case JiyueBadgeTone.neutral:
      return (colors.textSecondary, colors.hover, colors.borderSubtle);
    case JiyueBadgeTone.accent:
      return (colors.accent, colors.accentSoft, colors.accent);
    case JiyueBadgeTone.ok:
      return (colors.ok, colors.okSoft, colors.ok);
    case JiyueBadgeTone.warn:
      return (colors.warn, colors.warnSoft, colors.warn);
    case JiyueBadgeTone.danger:
      return (colors.danger, colors.dangerSoft, colors.danger);
  }
}

/// SSOT push button matching desktop Linear/Zed control geometry.
class JiyueButton extends StatelessWidget {
  final JiyueThemeColors colors;
  final String label;
  final VoidCallback? onPressed;
  final JiyueButtonKind kind;
  final JiyueControlSize size;
  final String? icon;
  final bool loading;
  final bool expand;

  const JiyueButton({
    super.key,
    required this.colors,
    required this.label,
    required this.onPressed,
    this.kind = JiyueButtonKind.secondary,
    this.size = JiyueControlSize.md,
    this.icon,
    this.loading = false,
    this.expand = false,
  });

  @override
  Widget build(BuildContext context) {
    final (double height, double px, double fontSize, double radius, double iconSize) =
        switch (size) {
          JiyueControlSize.lg => (
            40.0,
            14.0,
            JiyueTokens.fontMd,
            JiyueTokens.radiusMd,
            16.0,
          ),
          JiyueControlSize.md => (
            JiyueTokens.controlLg,
            12.0,
            JiyueTokens.fontBase,
            JiyueTokens.radiusMd,
            15.0,
          ),
          JiyueControlSize.sm => (
            JiyueTokens.controlMd,
            10.0,
            JiyueTokens.fontSm,
            JiyueTokens.radiusSm,
            14.0,
          ),
          JiyueControlSize.xs => (
            JiyueTokens.controlSm,
            8.0,
            JiyueTokens.fontXs,
            JiyueTokens.radiusSm,
            12.0,
          ),
        };

    final (Color bg, Color fg, Color border) = switch (kind) {
      JiyueButtonKind.primary => (colors.accent, colors.onAccent, colors.accent),
      JiyueButtonKind.secondary => (colors.elevated, colors.textPrimary, colors.border),
      JiyueButtonKind.ghost => (Colors.transparent, colors.textSecondary, Colors.transparent),
      JiyueButtonKind.danger => (colors.dangerSoft, colors.danger, colors.danger),
    };

    final bool disabled = onPressed == null && !loading;
    final VoidCallback? effectiveTap = (loading || onPressed == null) ? null : onPressed;

    final Widget content = Container(
      height: height,
      padding: EdgeInsets.symmetric(horizontal: px),
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(radius),
        border: Border.all(color: border, width: 1),
      ),
      child: Row(
        mainAxisSize: expand ? MainAxisSize.max : MainAxisSize.min,
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          if (loading) ...[
            SizedBox(
              width: iconSize - 2,
              height: iconSize - 2,
              child: CircularProgressIndicator(strokeWidth: 1.8, color: fg),
            ),
            const SizedBox(width: 6),
          ] else if (icon != null) ...[
            JiyueIcon(icon!, color: fg, size: iconSize),
            const SizedBox(width: 6),
          ],
          Flexible(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: fontSize,
                fontWeight: kind == JiyueButtonKind.primary
                    ? FontWeight.w600
                    : FontWeight.w500,
                color: fg,
              ),
            ),
          ),
        ],
      ),
    );

    final Widget interactive = Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: effectiveTap,
        borderRadius: BorderRadius.circular(radius),
        splashColor: colors.active,
        highlightColor: colors.hover,
        child: content,
      ),
    );

    final Widget sized = expand
        ? SizedBox(width: double.infinity, child: interactive)
        : interactive;

    if (disabled) {
      return Opacity(opacity: 0.5, child: sized);
    }
    return sized;
  }
}

/// Square icon-only button with optional selected or danger tone.
class JiyueIconButton extends StatelessWidget {
  final JiyueThemeColors colors;
  final String icon;
  final VoidCallback? onPressed;
  final double size;
  final double iconSize;
  final bool selected;
  final bool danger;
  final String? tooltip;

  const JiyueIconButton({
    super.key,
    required this.colors,
    required this.icon,
    required this.onPressed,
    this.size = 32.0,
    this.iconSize = 16.0,
    this.selected = false,
    this.danger = false,
    this.tooltip,
  });

  @override
  Widget build(BuildContext context) {
    final Color fg = danger
        ? colors.danger
        : (selected ? colors.accent : colors.textSecondary);
    final Color bg = selected ? colors.selected : Colors.transparent;
    final Color border = selected ? colors.accent : Colors.transparent;

    Widget btn = Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: onPressed,
        borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
        splashColor: colors.active,
        highlightColor: colors.hover,
        child: Container(
          width: size,
          height: size,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: bg,
            borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
            border: Border.all(color: border, width: 1),
          ),
          child: JiyueIcon(icon, color: fg, size: iconSize),
        ),
      ),
    );

    if (onPressed == null) {
      btn = Opacity(opacity: 0.45, child: btn);
    }

    if (tooltip != null && tooltip!.isNotEmpty) {
      return Tooltip(message: tooltip!, child: btn);
    }
    return btn;
  }
}

/// Compact semantic status badge.
class JiyueBadge extends StatelessWidget {
  final JiyueThemeColors colors;
  final String label;
  final JiyueBadgeTone tone;
  final String? icon;

  const JiyueBadge({
    super.key,
    required this.colors,
    required this.label,
    this.tone = JiyueBadgeTone.neutral,
    this.icon,
  });

  @override
  Widget build(BuildContext context) {
    final (fg, bg, border) = _resolveToneColors(colors, tone);
    final Color borderColor =
        tone == JiyueBadgeTone.neutral ? colors.borderSubtle : border.withAlpha(90);

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
        border: Border.all(color: borderColor, width: 1),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (icon != null) ...[
            JiyueIcon(icon!, color: fg, size: 11),
            const SizedBox(width: 4),
          ],
          Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontSize: JiyueTokens.fontXs,
              fontWeight: FontWeight.w600,
              color: fg,
            ),
          ),
        ],
      ),
    );
  }
}

/// Pill choice chip for filter rails and option selectors.
class JiyueChoiceChip extends StatelessWidget {
  final JiyueThemeColors colors;
  final String label;
  final bool selected;
  final VoidCallback onTap;
  final String? icon;

  const JiyueChoiceChip({
    super.key,
    required this.colors,
    required this.label,
    required this.selected,
    required this.onTap,
    this.icon,
  });

  @override
  Widget build(BuildContext context) {
    final Color bg = selected ? colors.accentSoft : colors.surface;
    final Color border = selected ? colors.accent : colors.borderSubtle;
    final Color fg = selected ? colors.accent : colors.textSecondary;

    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(JiyueTokens.radiusPill),
        splashColor: colors.active,
        highlightColor: colors.hover,
        child: Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: 10),
          decoration: BoxDecoration(
            color: bg,
            borderRadius: BorderRadius.circular(JiyueTokens.radiusPill),
            border: Border.all(color: border, width: 1),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (icon != null) ...[
                JiyueIcon(icon!, color: fg, size: 13),
                const SizedBox(width: 5),
              ],
              Text(
                label,
                style: TextStyle(
                  fontSize: JiyueTokens.fontSm,
                  fontWeight: selected ? FontWeight.w600 : FontWeight.w400,
                  color: fg,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Compact rectangular chip for note #tags and [[wikilinks]].
class JiyueTagChip extends StatelessWidget {
  final JiyueThemeColors colors;
  final String label;
  final bool accent;
  final String? icon;
  final VoidCallback? onTap;

  const JiyueTagChip({
    super.key,
    required this.colors,
    required this.label,
    this.accent = false,
    this.icon,
    this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final Color bg = accent ? colors.accentSoft : colors.elevated;
    final Color border = accent ? colors.accent.withAlpha(80) : colors.borderSubtle;
    final Color fg = accent ? colors.accent : colors.textSecondary;

    final Widget chip = Container(
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 3),
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
        border: Border.all(color: border, width: 1),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (icon != null) ...[
            JiyueIcon(icon!, color: fg, size: 12),
            const SizedBox(width: 4),
          ],
          Text(
            label,
            style: TextStyle(
              fontSize: JiyueTokens.fontXs,
              fontWeight: accent ? FontWeight.w600 : FontWeight.w400,
              color: fg,
            ),
          ),
        ],
      ),
    );

    if (onTap == null) return chip;
    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
        splashColor: colors.active,
        highlightColor: colors.hover,
        child: chip,
      ),
    );
  }
}

/// Segmented control aligned with desktop `segmented` / `segmented_full`.
class JiyueSegmented extends StatelessWidget {
  final JiyueThemeColors colors;
  final List<(String label, String? icon)> items;
  final int selectedIndex;
  final ValueChanged<int> onChanged;
  final bool fullWidth;

  const JiyueSegmented({
    super.key,
    required this.colors,
    required this.items,
    required this.selectedIndex,
    required this.onChanged,
    this.fullWidth = false,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: colors.panel,
        borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
        border: Border.all(color: colors.borderSubtle, width: 1),
      ),
      child: Row(
        mainAxisSize: fullWidth ? MainAxisSize.max : MainAxisSize.min,
        children: List.generate(items.length, (i) {
          final (label, iconName) = items[i];
          final bool active = i == selectedIndex;
          final Color fg = active ? colors.textPrimary : colors.textMuted;

          final Widget seg = Material(
            color: Colors.transparent,
            child: InkWell(
              onTap: () => onChanged(i),
              borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
              splashColor: colors.active,
              highlightColor: colors.hover,
              child: Container(
                height: JiyueTokens.controlSm,
                padding: const EdgeInsets.symmetric(horizontal: 10),
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  color: active ? colors.elevated : Colors.transparent,
                  borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                  border: Border.all(
                    color: active ? colors.border : Colors.transparent,
                    width: 1,
                  ),
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    if (iconName != null) ...[
                      JiyueIcon(
                        iconName,
                        color: active ? colors.accent : colors.textMuted,
                        size: 13,
                      ),
                      const SizedBox(width: 5),
                    ],
                    Text(
                      label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: JiyueTokens.fontSm,
                        fontWeight: active ? FontWeight.w600 : FontWeight.w400,
                        color: fg,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          );

          return fullWidth ? Expanded(child: seg) : seg;
        }),
      ),
    );
  }
}

/// Restraint 36x20 pill toggle matching desktop `controls::toggle_view`.
class JiyueSwitch extends StatelessWidget {
  final JiyueThemeColors colors;
  final bool value;
  final ValueChanged<bool> onChanged;

  const JiyueSwitch({
    super.key,
    required this.colors,
    required this.value,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final Color trackBg = value ? colors.accent : colors.active;
    final Color trackBorder = value ? colors.accent : colors.border;
    final Color knobColor = value ? colors.onAccent : colors.textSecondary;

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () => onChanged(!value),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 150),
        width: 36,
        height: 20,
        padding: const EdgeInsets.all(2),
        decoration: BoxDecoration(
          color: trackBg,
          borderRadius: BorderRadius.circular(JiyueTokens.radiusPill),
          border: Border.all(color: trackBorder, width: 1),
        ),
        child: AnimatedAlign(
          duration: const Duration(milliseconds: 150),
          alignment: value ? Alignment.centerRight : Alignment.centerLeft,
          child: Container(
            width: 14,
            height: 14,
            decoration: BoxDecoration(
              color: knobColor,
              borderRadius: BorderRadius.circular(JiyueTokens.radiusPill),
            ),
          ),
        ),
      ),
    );
  }
}

/// Interactive settings row combining icon, label, subtitle and `JiyueSwitch`.
class JiyueSwitchRow extends StatelessWidget {
  final JiyueThemeColors colors;
  final String title;
  final String? subtitle;
  final String? icon;
  final bool value;
  final ValueChanged<bool> onChanged;

  const JiyueSwitchRow({
    super.key,
    required this.colors,
    required this.title,
    this.subtitle,
    this.icon,
    required this.value,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: () => onChanged(!value),
        borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
        splashColor: colors.active,
        highlightColor: colors.hover,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 4),
          child: Row(
            children: [
              if (icon != null) ...[
                JiyueIcon(icon!, color: colors.accent, size: 17),
                const SizedBox(width: 10),
              ],
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      title,
                      style: TextStyle(
                        fontSize: JiyueTokens.fontBase,
                        fontWeight: FontWeight.w500,
                        color: colors.textPrimary,
                      ),
                    ),
                    if (subtitle != null && subtitle!.isNotEmpty) ...[
                      const SizedBox(height: 2),
                      Text(
                        subtitle!,
                        style: TextStyle(
                          fontSize: JiyueTokens.fontXs,
                          color: colors.textMuted,
                        ),
                      ),
                    ],
                  ],
                ),
              ),
              const SizedBox(width: 12),
              JiyueSwitch(colors: colors, value: value, onChanged: onChanged),
            ],
          ),
        ),
      ),
    );
  }
}

/// Unified single-line or multi-line input field using `colors.elevated`.
class JiyueTextField extends StatelessWidget {
  final JiyueThemeColors colors;
  final TextEditingController? controller;
  final String? label;
  final String? hintText;
  final String? prefixIcon;
  final Widget? suffix;
  final bool obscureText;
  final int maxLines;
  final bool autofocus;
  final FocusNode? focusNode;
  final ValueChanged<String>? onChanged;
  final ValueChanged<String>? onSubmitted;

  const JiyueTextField({
    super.key,
    required this.colors,
    this.controller,
    this.label,
    this.hintText,
    this.prefixIcon,
    this.suffix,
    this.obscureText = false,
    this.maxLines = 1,
    this.autofocus = false,
    this.focusNode,
    this.onChanged,
    this.onSubmitted,
  });

  @override
  Widget build(BuildContext context) {
    final borderRadius = BorderRadius.circular(JiyueTokens.radiusMd);
    final defaultBorder = OutlineInputBorder(
      borderRadius: borderRadius,
      borderSide: BorderSide(color: colors.border, width: 1),
    );
    final focusedBorder = OutlineInputBorder(
      borderRadius: borderRadius,
      borderSide: BorderSide(color: colors.borderFocus, width: 1.2),
    );

    final Widget field = TextField(
      controller: controller,
      focusNode: focusNode,
      autofocus: autofocus,
      obscureText: obscureText,
      maxLines: obscureText ? 1 : maxLines,
      onChanged: onChanged,
      onSubmitted: onSubmitted,
      cursorColor: colors.accent,
      style: TextStyle(
        fontSize: JiyueTokens.fontBase,
        color: colors.textPrimary,
      ),
      decoration: InputDecoration(
        isDense: true,
        filled: true,
        fillColor: colors.elevated,
        hintText: hintText,
        hintStyle: TextStyle(
          fontSize: JiyueTokens.fontSm,
          color: colors.textMuted,
        ),
        contentPadding: EdgeInsets.symmetric(
          horizontal: 10,
          vertical: maxLines == 1 ? 9 : 10,
        ),
        prefixIcon: prefixIcon == null
            ? null
            : Padding(
                padding: const EdgeInsets.only(left: 10, right: 8),
                child: JiyueIcon(prefixIcon!, color: colors.textMuted, size: 16),
              ),
        prefixIconConstraints: prefixIcon == null
            ? null
            : const BoxConstraints(minWidth: 34, minHeight: 32),
        suffixIcon: suffix,
        suffixIconConstraints: suffix == null
            ? null
            : const BoxConstraints(minWidth: 32, minHeight: 32),
        border: defaultBorder,
        enabledBorder: defaultBorder,
        focusedBorder: focusedBorder,
      ),
    );

    if (label == null || label!.isEmpty) return field;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          label!,
          style: TextStyle(
            fontSize: JiyueTokens.fontSm,
            fontWeight: FontWeight.w500,
            color: colors.textSecondary,
          ),
        ),
        const SizedBox(height: 6),
        field,
      ],
    );
  }
}

/// Section caption header with optional trailing action.
class JiyueSectionHeader extends StatelessWidget {
  final JiyueThemeColors colors;
  final String title;
  final Widget? trailing;
  final EdgeInsetsGeometry padding;

  const JiyueSectionHeader({
    super.key,
    required this.colors,
    required this.title,
    this.trailing,
    this.padding = const EdgeInsets.only(bottom: 8),
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: padding,
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(
            title,
            style: TextStyle(
              fontSize: JiyueTokens.fontSm,
              fontWeight: FontWeight.w600,
              color: colors.textMuted,
            ),
          ),
          if (trailing != null) trailing!,
        ],
      ),
    );
  }
}

/// Flat 1px-bordered card container without drop shadow.
class JiyueSectionCard extends StatelessWidget {
  final JiyueThemeColors colors;
  final Widget child;
  final bool active;
  final EdgeInsetsGeometry padding;
  final EdgeInsetsGeometry? margin;
  final VoidCallback? onTap;

  const JiyueSectionCard({
    super.key,
    required this.colors,
    required this.child,
    this.active = false,
    this.padding = const EdgeInsets.all(12),
    this.margin,
    this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final borderRadius = BorderRadius.circular(JiyueTokens.radiusLg);
    final Widget body = Container(
      width: double.infinity,
      padding: padding,
      child: child,
    );

    return Container(
      margin: margin,
      decoration: BoxDecoration(
        color: colors.surface,
        borderRadius: borderRadius,
        border: Border.all(
          color: active ? colors.accent : colors.borderSubtle,
          width: active ? 1.5 : 1,
        ),
      ),
      child: onTap == null
          ? body
          : Material(
              color: Colors.transparent,
              child: InkWell(
                onTap: onTap,
                borderRadius: borderRadius,
                splashColor: colors.active,
                highlightColor: colors.hover,
                child: body,
              ),
            ),
    );
  }
}

/// Crisp 1px hairline separator.
class JiyueDivider extends StatelessWidget {
  final JiyueThemeColors colors;
  final EdgeInsetsGeometry? margin;

  const JiyueDivider({
    super.key,
    required this.colors,
    this.margin,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 1,
      margin: margin,
      color: colors.borderSubtle,
    );
  }
}

/// Inline status banner consuming soft semantic tokens.
class JiyueBanner extends StatelessWidget {
  final JiyueThemeColors colors;
  final JiyueBadgeTone tone;
  final String message;
  final String? icon;

  const JiyueBanner({
    super.key,
    required this.colors,
    required this.tone,
    required this.message,
    this.icon,
  });

  @override
  Widget build(BuildContext context) {
    final (fg, bg, border) = _resolveToneColors(colors, tone);
    final String effectiveIcon = icon ??
        switch (tone) {
          JiyueBadgeTone.ok => 'check',
          JiyueBadgeTone.warn => 'alert-triangle',
          JiyueBadgeTone.danger => 'alert-triangle',
          JiyueBadgeTone.accent || JiyueBadgeTone.neutral => 'info-circle',
        };
    final Color borderColor =
        tone == JiyueBadgeTone.neutral ? colors.border : border.withAlpha(110);

    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(JiyueTokens.radiusMd),
        border: Border.all(color: borderColor, width: 1),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(top: 1),
            child: JiyueIcon(effectiveIcon, color: fg, size: 14),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              message,
              style: TextStyle(
                fontSize: JiyueTokens.fontSm,
                fontWeight: FontWeight.w500,
                height: 1.35,
                color: fg,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Centered empty-state placeholder matching desktop `empty_state`.
class JiyueEmptyState extends StatelessWidget {
  final JiyueThemeColors colors;
  final String icon;
  final String title;
  final String? subtitle;
  final Widget? action;

  const JiyueEmptyState({
    super.key,
    required this.colors,
    required this.icon,
    required this.title,
    this.subtitle,
    this.action,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 28),
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            JiyueIcon(icon, color: colors.textMuted, size: 28),
            const SizedBox(height: 10),
            Text(
              title,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: JiyueTokens.fontMd,
                fontWeight: FontWeight.w600,
                color: colors.textSecondary,
              ),
            ),
            if (subtitle != null && subtitle!.isNotEmpty) ...[
              const SizedBox(height: 4),
              Text(
                subtitle!,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontSize: JiyueTokens.fontSm,
                  height: 1.4,
                  color: colors.textMuted,
                ),
              ),
            ],
            if (action != null) ...[
              const SizedBox(height: 14),
              action!,
            ],
          ],
        ),
      ),
    );
  }
}

/// Unified bottom modal sheet chrome using `colors.panel` + `1px` top border.
class JiyueModalSheet extends StatelessWidget {
  final JiyueThemeColors colors;
  final String title;
  final String? subtitle;
  final String? icon;
  final Widget? headerTrailing;
  final VoidCallback? onClose;
  final bool scrollable;
  final double maxHeightFactor;
  final Widget child;

  const JiyueModalSheet({
    super.key,
    required this.colors,
    required this.title,
    this.subtitle,
    this.icon,
    this.headerTrailing,
    this.onClose,
    this.scrollable = true,
    this.maxHeightFactor = 0.88,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final bottomInset = MediaQuery.of(context).viewInsets.bottom;

    return ConstrainedBox(
      constraints: BoxConstraints(
        maxHeight: MediaQuery.of(context).size.height * maxHeightFactor,
      ),
      child: Container(
        decoration: BoxDecoration(
          color: colors.panel,
          borderRadius: const BorderRadius.vertical(
            top: Radius.circular(JiyueTokens.radiusXl),
          ),
          border: Border(
            top: BorderSide(color: colors.border, width: 1),
          ),
        ),
        child: SafeArea(
          top: false,
          child: Padding(
            padding: EdgeInsets.only(bottom: bottomInset),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 10, 12, 12),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Container(
                        width: 32,
                        height: 3,
                        decoration: BoxDecoration(
                          color: colors.borderStrong,
                          borderRadius: BorderRadius.circular(JiyueTokens.radiusPill),
                        ),
                      ),
                      const SizedBox(height: 12),
                      Row(
                        children: [
                          if (icon != null) ...[
                            Container(
                              width: 28,
                              height: 28,
                              alignment: Alignment.center,
                              decoration: BoxDecoration(
                                color: colors.accentSoft,
                                borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                                border: Border.all(
                                  color: colors.accent.withAlpha(80),
                                  width: 1,
                                ),
                              ),
                              child: JiyueIcon(icon!, color: colors.accent, size: 15),
                            ),
                            const SizedBox(width: 10),
                          ],
                          Expanded(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  title,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: TextStyle(
                                    fontSize: JiyueTokens.fontLg,
                                    fontWeight: FontWeight.w600,
                                    color: colors.textPrimary,
                                  ),
                                ),
                                if (subtitle != null && subtitle!.isNotEmpty) ...[
                                  const SizedBox(height: 2),
                                  Text(
                                    subtitle!,
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: TextStyle(
                                      fontSize: JiyueTokens.fontXs,
                                      color: colors.textMuted,
                                    ),
                                  ),
                                ],
                              ],
                            ),
                          ),
                          if (headerTrailing != null) ...[
                            const SizedBox(width: 8),
                            headerTrailing!,
                          ],
                          const SizedBox(width: 4),
                          JiyueIconButton(
                            colors: colors,
                            icon: 'x',
                            tooltip: '关闭',
                            onPressed: onClose ?? () => Navigator.of(context).pop(),
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
                JiyueDivider(colors: colors),
                if (scrollable)
                  Flexible(
                    child: SingleChildScrollView(
                      padding: const EdgeInsets.fromLTRB(16, 14, 16, 18),
                      child: child,
                    ),
                  )
                else
                  Flexible(
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(16, 14, 16, 18),
                      child: child,
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Unified centered dialog container matching scheme 1 geometry.
class JiyueDialog extends StatelessWidget {
  final JiyueThemeColors colors;
  final String title;
  final String? icon;
  final Widget? badge;
  final Widget content;
  final List<Widget> actions;

  const JiyueDialog({
    super.key,
    required this.colors,
    required this.title,
    this.icon,
    this.badge,
    required this.content,
    this.actions = const [],
  });

  @override
  Widget build(BuildContext context) {
    return Dialog(
      backgroundColor: colors.panel,
      elevation: 0,
      insetPadding: const EdgeInsets.symmetric(horizontal: 20, vertical: 24),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(JiyueTokens.radiusXl),
        side: BorderSide(color: colors.border, width: 1),
      ),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 420),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 14, 14, 12),
              child: Row(
                children: [
                  if (icon != null) ...[
                    Container(
                      width: 28,
                      height: 28,
                      alignment: Alignment.center,
                      decoration: BoxDecoration(
                        color: colors.accentSoft,
                        borderRadius: BorderRadius.circular(JiyueTokens.radiusSm),
                        border: Border.all(
                          color: colors.accent.withAlpha(80),
                          width: 1,
                        ),
                      ),
                      child: JiyueIcon(icon!, color: colors.accent, size: 15),
                    ),
                    const SizedBox(width: 10),
                  ],
                  Expanded(
                    child: Text(
                      title,
                      style: TextStyle(
                        fontSize: JiyueTokens.fontLg,
                        fontWeight: FontWeight.w600,
                        color: colors.textPrimary,
                      ),
                    ),
                  ),
                  if (badge != null) ...[
                    const SizedBox(width: 8),
                    badge!,
                  ],
                ],
              ),
            ),
            JiyueDivider(colors: colors),
            Flexible(
              child: Padding(
                padding: const EdgeInsets.fromLTRB(16, 14, 16, 14),
                child: content,
              ),
            ),
            if (actions.isNotEmpty) ...[
              JiyueDivider(colors: colors),
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 10, 16, 12),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: actions,
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
