import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.dart';
import 'status_dot.dart';

class TerminalWindow extends StatelessWidget {
  const TerminalWindow({
    super.key,
    required this.title,
    required this.child,
    this.glow = false,
    this.running = false,
  });

  final String title;
  final Widget child;
  final bool glow;
  final bool running;

  @override
  Widget build(BuildContext context) => Container(
        decoration: BoxDecoration(
          color: C.termBg,
          borderRadius: BorderRadius.circular(R.md),
          border: Border.all(color: C.border),
          boxShadow: glow
              ? const [
                  BoxShadow(
                      color: C.accentGlow, blurRadius: 60, spreadRadius: -20)
                ]
              : const [],
        ),
        clipBehavior: Clip.antiAlias,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              constraints: const BoxConstraints(minHeight: 36),
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
              decoration: const BoxDecoration(
                border: Border(bottom: BorderSide(color: C.border)),
              ),
              child: Row(
                children: [
                  StatusDot(
                      pulse: running,
                      label: running ? 'Server running' : 'Terminal'),
                  const SizedBox(width: 12),
                  Expanded(
                      child: Text(title,
                          textAlign: TextAlign.center, style: mono(12))),
                  const SizedBox(width: 20),
                ],
              ),
            ),
            Padding(
              padding: const EdgeInsets.all(16),
              child: DefaultTextStyle(style: mono(13), child: child),
            ),
          ],
        ),
      );
}

class BlinkingCursor extends StatefulWidget {
  const BlinkingCursor({super.key});

  @override
  State<BlinkingCursor> createState() => _BlinkingCursorState();
}

class _BlinkingCursorState extends State<BlinkingCursor>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(seconds: 1),
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context) || !TickerMode.of(context)) {
      _controller.stop();
      _controller.value = 0;
    } else {
      _controller.repeat();
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ExcludeSemantics(
        child: AnimatedBuilder(
          animation: _controller,
          builder: (context, _) => Opacity(
            opacity: _controller.value < 0.5 ? 1 : 0,
            child: Text('_', style: mono(16, color: C.accent)),
          ),
        ),
      );
}
