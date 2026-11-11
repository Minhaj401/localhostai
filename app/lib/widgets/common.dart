import 'package:flutter/material.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../theme/theme.dart';
import '../theme/tokens.dart';
import 'app_card.dart';
import 'brand.dart';
import 'status_dot.dart';

enum StatusTone { neutral, accent, positive, warning, danger }

class AdaptiveContent extends StatelessWidget {
  const AdaptiveContent(
      {super.key, required this.child, this.maxWidth = 1120, this.padding});
  final Widget child;
  final double maxWidth;
  final EdgeInsets? padding;

  @override
  Widget build(BuildContext context) => Align(
        alignment: Alignment.topCenter,
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: maxWidth),
          child: Padding(
            padding: padding ??
                EdgeInsets.all(
                    MediaQuery.sizeOf(context).width < 840 ? 20 : 32),
            child: child,
          ),
        ),
      );
}

class AppBackdrop extends StatelessWidget {
  const AppBackdrop({super.key, required this.child, this.hero = false});
  final Widget child;
  final bool hero;

  @override
  Widget build(BuildContext context) => DecoratedBox(
        decoration: BoxDecoration(
          color: C.bg,
          gradient: hero
              ? RadialGradient(
                  center: const Alignment(0, -1),
                  radius: 0.9,
                  colors: [C.accent.withOpacity(0.14), Colors.transparent],
                )
              : null,
        ),
        child: child,
      );
}

class SectionHeader extends StatelessWidget {
  const SectionHeader(this.title,
      {super.key,
      this.subtitle,
      this.action,
      this.eyebrow,
      this.centered = false});
  final String title;
  final String? subtitle;
  final String? eyebrow;
  final Widget? action;
  final bool centered;

  @override
  Widget build(BuildContext context) {
    final heading = Semantics(
      header: true,
      child: Column(
        crossAxisAlignment:
            centered ? CrossAxisAlignment.center : CrossAxisAlignment.start,
        children: [
          if (eyebrow != null) ...[
            Text(eyebrow!.toUpperCase(),
                style: mono(12, color: C.text3, w: FontWeight.w500, ls: 1.5)),
            const SizedBox(height: 8),
          ],
          Text(title,
              style: Theme.of(context).textTheme.headlineMedium,
              textAlign: centered ? TextAlign.center : TextAlign.start),
          if (subtitle != null) ...[
            const SizedBox(height: 8),
            Text(subtitle!,
                textAlign: centered ? TextAlign.center : TextAlign.start),
          ],
        ],
      ),
    );
    if (action == null) return heading;
    return LayoutBuilder(builder: (context, constraints) {
      if (constraints.maxWidth < 500 ||
          MediaQuery.textScalerOf(context).scale(1) > 1.3) {
        return Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [heading, const SizedBox(height: 12), action!]);
      }
      return Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Expanded(child: heading),
        const SizedBox(width: 16),
        action!
      ]);
    });
  }
}

class StatusPill extends StatelessWidget {
  const StatusPill(this.label,
      {super.key, this.tone = StatusTone.neutral, this.pulse = false});
  final String label;
  final StatusTone tone;
  final bool pulse;

  @override
  Widget build(BuildContext context) {
    final color = toneColor(tone);
    return Semantics(
      label: 'Status: $label',
      excludeSemantics: true,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
        decoration: BoxDecoration(
          color: tone == StatusTone.positive || tone == StatusTone.accent
              ? C.accentDim
              : C.surface,
          border: Border.all(
              color: tone == StatusTone.neutral
                  ? C.borderStrong
                  : color.withOpacity(0.25)),
          borderRadius: BorderRadius.circular(R.pill),
        ),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          if (tone != StatusTone.neutral) ...[
            StatusDot(color: color, pulse: pulse, size: 6, label: label),
            const SizedBox(width: 8),
          ],
          Flexible(
              child: Text(label.toUpperCase(),
                  style: mono(12, color: color, w: FontWeight.w500, ls: 1.2))),
        ]),
      ),
    );
  }
}

class NoticeBanner extends StatelessWidget {
  const NoticeBanner(
      {super.key,
      required this.message,
      this.tone = StatusTone.neutral,
      this.action});
  final String message;
  final StatusTone tone;
  final Widget? action;

  @override
  Widget build(BuildContext context) => Semantics(
        liveRegion: true,
        child: Container(
          padding: const EdgeInsets.all(14),
          decoration: BoxDecoration(
            color: C.surface2,
            borderRadius: BorderRadius.circular(R.md),
            border: Border.all(color: toneColor(tone).withOpacity(0.25)),
          ),
          child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Icon(
                tone == StatusTone.danger
                    ? LucideIcons.alertCircle
                    : LucideIcons.info,
                size: 18,
                color: toneColor(tone)),
            const SizedBox(width: 10),
            Expanded(
                child: Text(message,
                    style: Theme.of(context).textTheme.bodyMedium)),
            if (action != null) action!,
          ]),
        ),
      );
}

class AppPanel extends StatelessWidget {
  const AppPanel(
      {super.key,
      required this.child,
      this.padding = const EdgeInsets.all(20),
      this.color});
  final Widget child;
  final EdgeInsets padding;
  final Color? color;

  @override
  Widget build(BuildContext context) => AppCard(padding: padding, child: child);
}

class MetricTile extends StatelessWidget {
  const MetricTile(
      {super.key,
      required this.label,
      required this.value,
      this.icon,
      this.marker = ''});
  final String label;
  final String value;
  final IconData? icon;
  final String marker;

  @override
  Widget build(BuildContext context) => Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(value,
              style: Theme.of(context)
                  .textTheme
                  .titleLarge
                  ?.copyWith(color: C.accent)),
          const SizedBox(height: 4),
          Text(label, style: mono(12)),
        ],
      );
}

class ResponsiveActions extends StatelessWidget {
  const ResponsiveActions(
      {super.key, required this.children, this.breakpoint = 600});
  final List<Widget> children;
  final double breakpoint;

  @override
  Widget build(BuildContext context) =>
      LayoutBuilder(builder: (context, constraints) {
        final vertical = constraints.maxWidth < breakpoint ||
            MediaQuery.textScalerOf(context).scale(1) > 1.3;
        if (vertical) {
          return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (var i = 0; i < children.length; i++) ...[
                  children[i],
                  if (i + 1 < children.length) const SizedBox(height: 12),
                ],
              ]);
        }
        return Row(children: [
          for (var i = 0; i < children.length; i++) ...[
            Expanded(child: children[i]),
            if (i + 1 < children.length) const SizedBox(width: 12),
          ],
        ]);
      });
}

class SectionGap extends StatelessWidget {
  const SectionGap({super.key});
  @override
  Widget build(BuildContext context) => const SizedBox(height: 24);
}

class BrandHeader extends StatelessWidget {
  const BrandHeader({super.key, this.trailing});
  final Widget? trailing;
  @override
  Widget build(BuildContext context) => Wrap(
        spacing: 16,
        runSpacing: 12,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [const Brand(), if (trailing != null) trailing!],
      );
}

Color toneColor(StatusTone tone) => switch (tone) {
      StatusTone.positive || StatusTone.accent => C.accent,
      StatusTone.warning => C.warn,
      StatusTone.danger => C.danger,
      StatusTone.neutral => C.text2,
    };
