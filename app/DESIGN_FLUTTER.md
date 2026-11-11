# DESIGN_FLUTTER.md — localhost.ai (Flutter app)

> Build all UI in this Flutter app to match this file.
> Brand: **localhost.ai** — *Run AI Locally.* Dark, terminal-native, minimal.
> Visual reference: Basedash — dark surfaces, hairline borders, featured glow,
> and big quiet headings. Brand artwork comes from the supplied Downloads logo.

This specification supersedes the earlier cream/ASCII design in `design.md`.

## 1. Setup

- Dark theme only: `ThemeMode.dark`, Material 3.
- Outfit for UI; JetBrains Mono for code, labels, and the tagline, through
  `google_fonts`. Bundle fonts and licenses; disable runtime font fetching.
- Dependencies: `google_fonts ^6.2.1`, `flutter_animate ^4.5.0`,
  `lucide_icons ^0.257.0`, `go_router ^14.0.0`, `flutter_riverpod ^2.5.1`,
  and `flutter_svg` for `assets/logo/mark.svg`.
- Structure: `lib/theme/{tokens,theme}.dart`, `lib/widgets/`, and
  `lib/features/<feature>/`.
- Keep working LAN pairing, streaming, model verification, and RPC lifecycle
  behavior behind the new UI. Riverpod owns the connection/shell view state;
  tested backend services may be retained during the UI migration.

## 2. Tokens — lib/theme/tokens.dart

```dart
class C {
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
  static final accentDim = accent.withOpacity(0.12);
  static final accentGlow = accent.withOpacity(0.35);
  static const warn = Color(0xFFFACC15);
  static const danger = Color(0xFFF87171);
  static const termBg = Color(0xFF050506);
}
class S {
  static const x1 = 4.0, x2 = 8.0, x3 = 12.0, x4 = 16.0,
      x6 = 24.0, x8 = 32.0, x12 = 48.0, x16 = 64.0;
}
class R {
  static const sm = 8.0, md = 12.0, lg = 16.0, xl = 24.0, pill = 999.0;
}
class Motion {
  static const fast = Duration(milliseconds: 150);
  static const base = Duration(milliseconds: 250);
  static const reveal = Duration(milliseconds: 600);
  static const curve = Cubic(0.22, 1, 0.36, 1);
}
```

- Pure black background. No light mode, purple, or blue brand colors.
- Green is the sole brand accent: one primary CTA per screen, status dots,
  active navigation, and important numbers.
- Status: green = running, yellow = loading, red = error/stopped. Pair every
  color with an explicit label.
- 1px borders. No elevation shadows; low-alpha green glow only on featured
  elements. Body text is never dimmer than `C.text2`.

## 3. Theme — lib/theme/theme.dart

- Outfit headings: 40px/500/1.05/-1.2 display, 30px/500/1.1/-0.75 H2,
  20px/500 titles; white.
- Body: Outfit 16px/1.6 and 14px/1.5, `C.text2`.
- Labels/code/tagline: JetBrains Mono, 1.6 line height. Uppercase eyebrow
  tracking 1.5. Use `C.text3` only for nonessential text at least 12px.
- ColorScheme dark: primary and secondary `C.accent`, onPrimary black,
  surface `C.surface`, onSurface white, error `C.danger`, outline `C.border`.
- Transparent app bar, zero elevation/scrolled-under elevation.
- Cards: surface, radius 16, 1px border, no elevation.
- Inputs: surface, radius 10, 14x12 padding, border; focused accent border.
- Navigation: elevated surface, green-tinted selection indicator, mono labels.
- `NoSplash`, transparent default highlight, visible keyboard focus.

## 4. Core widgets — lib/widgets/

### StatusDot

A single green dot matching the logo. Default 8px, 2-second outward pulse.
No macOS red/yellow/green trio. Label it semantically (for example, “Server
running”). Skip pulse under reduced motion and on inactive tabs.

### PrimaryButton / SecondaryButton / Ghost

- Primary: green, black Outfit 16/500, radius 10, minimum height 48. Pressed
  accentLo. Glow only while enabled and focused/hovered. Light haptic on tap.
- Secondary: surface, 1px border, white. Pressed surface2/borderStrong.
- Ghost: transparent, text2. Disabled: 40% opacity, no glow.
- Use AnimatedContainer at 150ms, native button/focus/keyboard semantics.
- Do not impose a fixed height that clips scaled or wrapped text.

### TerminalWindow

- termBg, 1px border, radius 12, clipped.
- Title bar: single status dot at left, centered mono 12 title, hairline below.
- Body: mono 13/1.6, padding 16. Prompt `>_` green, command white, output text2,
  comments text3, success green.
- Optional featured glow: accentGlow, blur 60, spread -20.
- Optional stacked gray window at 40% opacity offset (12, -12).
- Cursor blink: one-second steps. Real commands/output only.

### CornerBrackets

Two green L corners, top-right and bottom-left, stroke 2.5px with rounded joins.
At most one bracket treatment per screen. CustomPaint over featured visuals.

### AppCard

Surface, border, radius 16, padding 20. Pressed surface2/borderStrong. A featured
card may have one subtle green radial/glow (blur 40, spread -16).

### Badge / Pill

Mono 12, uppercase, tracking 1.2, 4x10 padding, pill radius. Default surface,
border, text2. Accent variant green-tinted background/green label. Running badge
includes a labeled StatusDot. Wrap at large text or constrained width.

### CommandPill

Mono command with dim `$` prefix and copy button. Copy via Clipboard.setData;
show green check for 1.5 seconds. Dispose feedback timer.

### SectionHeader / Tabs / Code

- Mono eyebrow + sentence-case white heading + text2 description. Center for
  onboarding/empty states; left-align operational screens.
- Tabs: dim labels, active white label with 1px green underline; 200ms crossfade.
- Code block: termBg/border/radius12, mono13, horizontal scroll without wrapping.
  Keywords green, strings `#A5F3FC`, numbers `#FDE68A`, comments text3.
- Optional hero gradient wordmark: white to white at 15%, top-to-bottom.

## 5. Logo and branding

- Use the supplied “Localhost AI Terminal Logo.png” from Downloads. Preserve
  its foreground artwork and green `.ai`; never stretch/recolor or add shadows.
- Mark: outlined terminal, one green dot, `>_`, gray stacked window, TR/BL corners.
  `assets/logo/mark.svg` contains the lossless extracted mark; provenance records
  the supplied source and crop. Keep the original lockup in-app as appropriate.
- Wordmark: `localhost` white and `.ai` accent, Outfit 600; tight tracking.
- Tagline: `Run AI Locally` in mono, wide tracking.
- All branding on black. Clear space at least the height of the “l”.
- Launcher: mark on black, approximately 66% scale, no wordmark text.
- Splash: black with centered supplied lockup; 400ms fade in-app.
- Native splash/status/navigation: black, light system icons.

## 6. Screens and navigation

Bottom NavigationBar on mobile; NavigationRail from 840px. Four destinations:

1. **Chat**: lazy message list; bottom-pinned composer; user right-aligned
   surface2 bubble with radius16 and tail4. Assistant left-aligned without a
   bubble, white text, mono code, green streaming cursor. Composer surface pill,
   border, green circular send button/black arrow. Model/status pill above it.
2. **Models**: searchable model cards, white names, mono size/quant, Installed/
   Download badges, 2px green download progress. Download/plan/load are real
   coordinator operations. Show capacity shortfall before downloading to load.
3. **Server**: truthful coordinator status card, copyable endpoint, load/start
   and stop controls, live coordinator events in TerminalWindow, pairing and phone
   resource-sharing controls. Display the real endpoint from pairing; never
   pretend the phone runs Ollama at `localhost:11434`.
4. **Settings**: grouped cards, hairline dividers, mono values, green switches;
   preserve backend choice, API keys, system prompt, and paired-node management.

First-launch onboarding: at most three pages. H1 “Run AI locally.”, terminal
demo, one “Get started” CTA. Do not claim encrypted or authenticated RPC; show
a trusted-LAN warning in Server. Active progress indicator 16px green pill; inactive
6px borderStrong dot. Do not block cloud chat behind LAN pairing.

Layout: 20px mobile / 32px tablet padding, max width720 chat /1120 elsewhere,
24px sections /12px rows. SafeArea, keyboard avoidance, scale-aware heights.
Connection, pairing, compute pool, and node discovery remain available from
Server and relevant contextual actions.

## 7. Backgrounds and polish

Pure black base. Hero/empty-state visual may have a green radial at 14% opacity,
center (0,-1), radius0.9. Optional static/slow sparse particles on onboarding
only. Faded UI fragments are optional, not required for operational screens.
Hairline dividers, no elevation shadows.

## 8. Motion

Press150ms, UI250ms, reveal600ms, Motion.curve. Enter once with fade/16px upward
translation; optional60ms stagger. Fade-through routes250ms, no Material
slide-from-right. Terminal typing30ms/character; cursor one-second steps;
dot pulse2s. Animate opacity/transforms, isolate repaints, dispose controllers.
Respect `MediaQuery.disableAnimationsOf(context)` and inactive-tab TickerMode.

## 9. Content

Always `localhost.ai`, lowercase. Short, direct, sentence-case headings. No hype,
emoji, lorem ipsum, fabricated stats, or unsupported privacy/performance claims.
Clearly distinguish LAN chat, optional external cloud, and phone RPC sharing.

## 10. Accessibility

48x48 minimum target, 2px accent focus outline, icon/status semantics, AA text
contrast, font scaling at least1.3x, no fixed-height clipping. Color always paired
with a status label. Essential/body text uses text/text2. Reduced-motion path
has no typing/pulsing. Disabled actions must not accept taps or haptics.

## 11. Do / Don't

**Do:** black/quiet, one green brand accent, mono labels/code, hairline borders,
featured-only glow. **Don't:** default purple/blue, elevation shadows, light
mode, multiple green CTAs, RGB terminal dots, emoji, or recolored logos.

## 12. Acceptance checklist

1. Install compatible dependencies, create tokens/theme, force dark theme.
2. Build StatusDot, buttons, TerminalWindow, brackets, AppCard, Badge, CommandPill.
3. Implement onboarding and Chat / Models / Server / Settings adaptive shell.
4. Apply motion and verify reduced-motion/inactive-tab behavior.
5. Test360x800,412x915,768x1024,1280x800 and1.3x text; no overflow.
6. Generate icons/splash from the supplied mark on black, build signed ARM64 APK.
