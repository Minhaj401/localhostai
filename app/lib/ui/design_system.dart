/// Shared imports for auxiliary pairing and device screens.
library;

import 'package:flutter/material.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../theme/theme.dart';
import '../theme/tokens.dart';

export '../theme/theme.dart';
export '../theme/tokens.dart';
export '../widgets/app_card.dart';
export '../widgets/brand.dart';
export '../widgets/buttons.dart';
export '../widgets/command_pill.dart';
export '../widgets/common.dart';
export '../widgets/corner_brackets.dart';
export '../widgets/reveal.dart';
export '../widgets/status_dot.dart';
export '../widgets/terminal_window.dart';

abstract final class LocalhostAiTheme {
  static ThemeData light() => buildTheme();
  static ThemeData dark() => buildTheme();
}

abstract final class AppColors {
  static const canvas = C.bg;
  static const ink = C.text;
  static const inkDeep = C.accentLo;
  static const surfaceSoft = C.surface2;
  static const surfaceCard = C.surface;
  static const surfaceDark = C.termBg;
  static const surfaceDarkElevated = C.surface2;
  static const charcoal = C.text2;
  static const body = C.text2;
  static const mute = C.text2;
  static const stone = C.text3;
  static const ash = C.text2;
  static const hairline = C.border;
  static const hairlineStrong = C.borderStrong;
  static const accent = C.accent;
  static const accentActive = C.accentHi;
  static const danger = C.danger;
  static const dangerActive = C.danger;
  static const warning = C.warn;
  static const warningActive = C.warn;
  static const success = C.accent;
  static const successInk = C.accent;
}

class AsciiGlyph extends StatelessWidget {
  const AsciiGlyph(this.text,
      {super.key, this.color, this.semanticLabel, this.bold = true});
  final String text;
  final Color? color;
  final String? semanticLabel;
  final bool bold;

  @override
  Widget build(BuildContext context) {
    final icon = switch (text) {
      '[PC]' || '[::]' => LucideIcons.laptop,
      '[PH]' => LucideIcons.smartphone,
      '[QR]' => LucideIcons.qrCode,
      '[R]' => LucideIcons.refreshCw,
      '[>]' || '[v]' => LucideIcons.arrowRight,
      '[x]' => LucideIcons.x,
      '[+]' => LucideIcons.check,
      '[*]' => LucideIcons.flashlight,
      '[?]' => LucideIcons.helpCircle,
      _ => LucideIcons.terminal,
    };
    return Icon(icon,
        color: color ?? C.text2, size: 20, semanticLabel: semanticLabel);
  }
}
