import 'package:flutter/material.dart';

import '../theme/tokens.dart';

class StatusDot extends StatefulWidget {
  const StatusDot({
    super.key,
    this.color = C.accent,
    this.pulse = true,
    this.size = 8,
    this.label = 'Server running',
  });

  final Color color;
  final bool pulse;
  final double size;
  final String label;

  @override
  State<StatusDot> createState() => _StatusDotState();
}

class _StatusDotState extends State<StatusDot>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(seconds: 2),
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _syncAnimation();
  }

  @override
  void didUpdateWidget(StatusDot oldWidget) {
    super.didUpdateWidget(oldWidget);
    _syncAnimation();
  }

  void _syncAnimation() {
    final animate = widget.pulse &&
        !MediaQuery.disableAnimationsOf(context) &&
        TickerMode.of(context);
    if (animate && !_controller.isAnimating) {
      _controller.repeat();
    } else if (!animate) {
      _controller.stop();
      _controller.value = 0;
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: widget.label,
      child: AnimatedBuilder(
        animation: _controller,
        child: SizedBox.square(dimension: widget.size),
        builder: (context, child) => DecoratedBox(
          decoration: BoxDecoration(
            color: widget.color,
            shape: BoxShape.circle,
            boxShadow: _controller.isAnimating
                ? [
                    BoxShadow(
                      color: widget.color
                          .withOpacity(0.45 * (1 - _controller.value)),
                      spreadRadius: 6 * _controller.value,
                    ),
                  ]
                : const [],
          ),
          child: child,
        ),
      ),
    );
  }
}
