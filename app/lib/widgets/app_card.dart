import 'package:flutter/material.dart';

import '../theme/tokens.dart';

class AppCard extends StatefulWidget {
  const AppCard({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.all(20),
    this.featured = false,
    this.onTap,
    this.selected = false,
  });

  final Widget child;
  final EdgeInsets padding;
  final bool featured;
  final VoidCallback? onTap;
  final bool selected;

  @override
  State<AppCard> createState() => _AppCardState();
}

class _AppCardState extends State<AppCard> {
  bool _focused = false;
  bool _down = false;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      container: true,
      selected: widget.selected,
      child: AnimatedContainer(
        duration: MediaQuery.disableAnimationsOf(context)
            ? Duration.zero
            : Motion.fast,
        decoration: BoxDecoration(
          color: _down ? C.surface2 : C.surface,
          borderRadius: BorderRadius.circular(R.lg),
          border: Border.all(
            color: _focused || widget.selected
                ? C.accent
                : _down
                    ? C.borderStrong
                    : C.border,
            width: _focused ? 2 : 1,
          ),
          boxShadow: widget.featured
              ? const [
                  BoxShadow(
                      color: C.accentGlow, blurRadius: 40, spreadRadius: -16)
                ]
              : const [],
        ),
        child: Material(
          color: Colors.transparent,
          child: InkWell(
            onTap: widget.onTap,
            borderRadius: BorderRadius.circular(R.lg),
            canRequestFocus: widget.onTap != null,
            onFocusChange: (value) => setState(() => _focused = value),
            onTapDown: widget.onTap == null
                ? null
                : (_) => setState(() => _down = true),
            onTapUp: (_) => setState(() => _down = false),
            onTapCancel: () => setState(() => _down = false),
            child: Padding(padding: widget.padding, child: widget.child),
          ),
        ),
      ),
    );
  }
}
