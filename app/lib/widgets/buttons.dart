import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../theme/tokens.dart';

class PrimaryButton extends StatefulWidget {
  const PrimaryButton(
      {super.key, required this.label, this.onPressed, this.icon});

  final String label;
  final VoidCallback? onPressed;
  final IconData? icon;

  @override
  State<PrimaryButton> createState() => _PrimaryButtonState();
}

class _PrimaryButtonState extends State<PrimaryButton> {
  bool _down = false;
  bool _focused = false;
  bool _hovered = false;

  void _tap() {
    HapticFeedback.lightImpact();
    widget.onPressed?.call();
  }

  @override
  Widget build(BuildContext context) {
    final enabled = widget.onPressed != null;
    return Semantics(
      button: true,
      enabled: enabled,
      label: widget.label,
      onTap: enabled ? _tap : null,
      excludeSemantics: true,
      child: Opacity(
        opacity: enabled ? 1 : 0.4,
        child: AnimatedContainer(
          duration: MediaQuery.disableAnimationsOf(context)
              ? Duration.zero
              : Motion.fast,
          constraints: const BoxConstraints(minHeight: 48),
          decoration: BoxDecoration(
            color: enabled && _down ? C.accentLo : C.accent,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(
              color: enabled && _focused ? C.accent : Colors.transparent,
              width: 2,
            ),
            boxShadow: enabled && (_focused || _hovered)
                ? const [BoxShadow(color: C.accentGlow, blurRadius: 24)]
                : const [],
          ),
          child: Material(
            color: Colors.transparent,
            child: InkWell(
              borderRadius: BorderRadius.circular(10),
              canRequestFocus: enabled,
              onFocusChange: (value) => setState(() => _focused = value),
              onHover: (value) => setState(() => _hovered = value),
              onTapDown: enabled ? (_) => setState(() => _down = true) : null,
              onTapCancel: () => setState(() => _down = false),
              onTapUp: enabled ? (_) => setState(() => _down = false) : null,
              onTap: enabled ? _tap : null,
              child: Padding(
                padding:
                    const EdgeInsets.symmetric(horizontal: 20, vertical: 10),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.center,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (widget.icon != null) ...[
                      Icon(widget.icon, size: 18, color: C.bg),
                      const SizedBox(width: 8),
                    ],
                    Flexible(
                      child: Text(
                        widget.label,
                        textAlign: TextAlign.center,
                        style: Theme.of(context)
                            .textTheme
                            .labelLarge
                            ?.copyWith(color: C.bg),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class SecondaryButton extends StatelessWidget {
  const SecondaryButton(
      {super.key, required this.label, this.onPressed, this.icon});

  final String label;
  final VoidCallback? onPressed;
  final IconData? icon;

  @override
  Widget build(BuildContext context) => icon == null
      ? OutlinedButton(onPressed: onPressed, child: Text(label))
      : OutlinedButton.icon(
          onPressed: onPressed, icon: Icon(icon, size: 18), label: Text(label));
}
