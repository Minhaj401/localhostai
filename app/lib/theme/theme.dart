import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

import 'tokens.dart';

TextStyle mono(
  double size, {
  Color color = C.text2,
  FontWeight w = FontWeight.w400,
  double ls = 0,
}) =>
    GoogleFonts.jetBrainsMono(
      fontSize: size,
      color: color,
      fontWeight: w,
      letterSpacing: ls,
      height: 1.6,
    );

ThemeData buildTheme() {
  GoogleFonts.config.allowRuntimeFetching = false;
  final base = ThemeData(brightness: Brightness.dark, useMaterial3: true);
  final sans = GoogleFonts.outfitTextTheme(base.textTheme)
      .apply(bodyColor: C.text2, displayColor: C.text);
  final text = sans.copyWith(
    displayLarge: sans.displayLarge?.copyWith(
      fontSize: 40,
      fontWeight: FontWeight.w500,
      height: 1.05,
      letterSpacing: -1.2,
      color: C.text,
    ),
    headlineMedium: sans.headlineMedium?.copyWith(
      fontSize: 30,
      fontWeight: FontWeight.w500,
      height: 1.1,
      letterSpacing: -0.75,
      color: C.text,
    ),
    headlineSmall: sans.headlineSmall?.copyWith(
      fontSize: 24,
      fontWeight: FontWeight.w500,
      height: 1.2,
      color: C.text,
    ),
    titleLarge: sans.titleLarge?.copyWith(
      fontSize: 20,
      fontWeight: FontWeight.w500,
      color: C.text,
    ),
    titleMedium: sans.titleMedium?.copyWith(
      fontSize: 16,
      fontWeight: FontWeight.w500,
      color: C.text,
    ),
    bodyLarge:
        sans.bodyLarge?.copyWith(fontSize: 16, height: 1.6, color: C.text2),
    bodyMedium:
        sans.bodyMedium?.copyWith(fontSize: 14, height: 1.5, color: C.text2),
    bodySmall:
        sans.bodySmall?.copyWith(fontSize: 12, height: 1.5, color: C.text2),
    labelLarge: sans.labelLarge?.copyWith(
      fontSize: 16,
      fontWeight: FontWeight.w500,
      color: C.text,
    ),
    labelMedium: mono(12, w: FontWeight.w500),
    labelSmall: mono(12, color: C.text3, w: FontWeight.w500, ls: 1.5),
  );
  final shape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(10));
  final border = OutlineInputBorder(
    borderRadius: BorderRadius.circular(10),
    borderSide: const BorderSide(color: C.border),
  );

  return base.copyWith(
    scaffoldBackgroundColor: C.bg,
    canvasColor: C.bg,
    colorScheme: const ColorScheme.dark(
      primary: C.accent,
      onPrimary: Colors.black,
      primaryContainer: C.accentDim,
      onPrimaryContainer: C.accent,
      secondary: C.accent,
      onSecondary: Colors.black,
      secondaryContainer: C.accentDim,
      onSecondaryContainer: C.accent,
      tertiary: C.warn,
      onTertiary: C.bg,
      tertiaryContainer: Color(0x1FFACC15),
      onTertiaryContainer: C.warn,
      surface: C.surface,
      onSurface: C.text,
      surfaceContainerHighest: C.surface2,
      onSurfaceVariant: C.text2,
      error: C.danger,
      onError: C.bg,
      errorContainer: Color(0x1FF87171),
      onErrorContainer: C.danger,
      outline: C.border,
      outlineVariant: C.borderStrong,
      shadow: Colors.transparent,
      surfaceTint: Colors.transparent,
    ),
    textTheme: text,
    dividerTheme:
        const DividerThemeData(color: C.border, thickness: 1, space: 1),
    appBarTheme: AppBarTheme(
      backgroundColor: C.bg,
      elevation: 0,
      scrolledUnderElevation: 0,
      centerTitle: false,
      foregroundColor: C.text,
      surfaceTintColor: Colors.transparent,
      titleTextStyle: text.titleLarge,
    ),
    cardTheme: CardTheme(
      color: C.surface,
      elevation: 0,
      margin: EdgeInsets.zero,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(R.lg),
        side: const BorderSide(color: C.border),
      ),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: C.surface,
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      hintStyle: text.bodyMedium?.copyWith(color: C.text3),
      labelStyle: text.bodyMedium,
      helperStyle: text.bodySmall,
      errorStyle: text.bodySmall?.copyWith(color: C.danger),
      errorMaxLines: 3,
      helperMaxLines: 3,
      border: border,
      enabledBorder: border,
      focusedBorder: border.copyWith(
        borderSide: const BorderSide(color: C.accent, width: 2),
      ),
      errorBorder: border.copyWith(
        borderSide: const BorderSide(color: C.danger),
      ),
      focusedErrorBorder: border.copyWith(
        borderSide: const BorderSide(color: C.danger, width: 2),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        minimumSize: const Size(48, 48),
        padding: const EdgeInsets.symmetric(horizontal: 22, vertical: 12),
        shape: shape,
        backgroundColor: C.accent,
        foregroundColor: C.bg,
        disabledBackgroundColor: C.accent.withOpacity(0.4),
        disabledForegroundColor: C.bg.withOpacity(0.7),
        textStyle: text.labelLarge,
        elevation: 0,
      ).copyWith(
        backgroundColor: WidgetStateProperty.resolveWith(
            (states) => states.contains(WidgetState.disabled)
                ? C.accent.withOpacity(0.4)
                : states.contains(WidgetState.pressed)
                    ? C.accentLo
                    : C.accent),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        minimumSize: const Size(48, 48),
        foregroundColor: C.text,
        backgroundColor: C.surface,
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
        side: const BorderSide(color: C.borderStrong),
        shape: shape,
        textStyle: text.labelLarge,
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: C.text2,
        minimumSize: const Size(48, 48),
        shape: shape,
        textStyle: text.bodyMedium,
      ),
    ),
    iconButtonTheme: IconButtonThemeData(
      style: IconButton.styleFrom(
        foregroundColor: C.text2,
        minimumSize: const Size.square(48),
        shape: shape,
      ),
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: C.elevated,
      indicatorColor: C.accentDim,
      height: 72,
      labelTextStyle: WidgetStateProperty.resolveWith((states) => mono(
            12,
            color: states.contains(WidgetState.selected) ? C.accent : C.text2,
          )),
      iconTheme: WidgetStateProperty.resolveWith((states) => IconThemeData(
            size: 22,
            color: states.contains(WidgetState.selected) ? C.accent : C.text2,
          )),
    ),
    navigationRailTheme: NavigationRailThemeData(
      backgroundColor: C.elevated,
      indicatorColor: C.accentDim,
      selectedIconTheme: const IconThemeData(color: C.accent, size: 22),
      unselectedIconTheme: const IconThemeData(color: C.text2, size: 22),
      selectedLabelTextStyle: mono(12, color: C.accent),
      unselectedLabelTextStyle: mono(12),
    ),
    switchTheme: SwitchThemeData(
      thumbColor: WidgetStateProperty.resolveWith(
          (states) => states.contains(WidgetState.disabled)
              ? C.text3
              : states.contains(WidgetState.selected)
                  ? C.accent
                  : C.text2),
      trackColor: WidgetStateProperty.resolveWith((states) =>
          states.contains(WidgetState.selected) ? C.accentDim : C.surface2),
      trackOutlineColor: const WidgetStatePropertyAll(C.borderStrong),
    ),
    radioTheme: RadioThemeData(
      fillColor: WidgetStateProperty.resolveWith((states) =>
          states.contains(WidgetState.disabled) ? C.text3 : C.accent),
    ),
    dialogTheme: DialogTheme(
      backgroundColor: C.surface,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(R.lg),
        side: const BorderSide(color: C.borderStrong),
      ),
    ),
    popupMenuTheme: PopupMenuThemeData(
      color: C.surface,
      elevation: 0,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(R.md),
        side: const BorderSide(color: C.borderStrong),
      ),
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: C.surface2,
      contentTextStyle: text.bodyMedium,
      actionTextColor: C.accent,
      behavior: SnackBarBehavior.floating,
      elevation: 0,
      shape: shape,
    ),
    progressIndicatorTheme: const ProgressIndicatorThemeData(
      color: C.accent,
      linearTrackColor: C.border,
      circularTrackColor: C.border,
    ),
    splashFactory: NoSplash.splashFactory,
    highlightColor: Colors.transparent,
  );
}
