import 'package:flutter/material.dart';

abstract final class C {
  static const bg = Color(0xFF000000);
  static const elevated = Color(0xFF0A0A0B);
  static const surface = Color(0xFF101012);
  static const surface2 = Color(0xFF16161A);
  static const border = Color(0xFF1F1F24);
  static const borderStrong = Color(0xFF2E2E35);
  static const text = Color(0xFFFFFFFF);
  static const text2 = Color(0xFFA1A1AA);
  static const text3 = Color(0xFF6B6B73);
  static const accent = Color(0xFF5CF26A);
  static const accentHi = Color(0xFF8BFF95);
  static const accentLo = Color(0xFF3BC94B);
  static const accentDim = Color(0x1F5CF26A);
  static const accentGlow = Color(0x595CF26A);
  static const warn = Color(0xFFFACC15);
  static const danger = Color(0xFFF87171);
  static const termBg = Color(0xFF050506);
}

abstract final class S {
  static const x1 = 4.0;
  static const x2 = 8.0;
  static const x3 = 12.0;
  static const x4 = 16.0;
  static const x6 = 24.0;
  static const x8 = 32.0;
  static const x12 = 48.0;
  static const x16 = 64.0;
}

abstract final class R {
  static const sm = 8.0;
  static const md = 12.0;
  static const lg = 16.0;
  static const xl = 24.0;
  static const pill = 999.0;
}

abstract final class Motion {
  static const fast = Duration(milliseconds: 150);
  static const base = Duration(milliseconds: 250);
  static const reveal = Duration(milliseconds: 600);
  static const curve = Cubic(0.22, 1, 0.36, 1);
}
