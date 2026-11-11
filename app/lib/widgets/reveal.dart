import 'package:flutter/material.dart';
import 'package:flutter_animate/flutter_animate.dart';

import '../theme/tokens.dart';

class Reveal extends StatelessWidget {
  const Reveal({super.key, required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) => MediaQuery.disableAnimationsOf(context)
      ? child
      : child
          .animate()
          .fadeIn(duration: Motion.reveal, curve: Motion.curve)
          .slideY(
              begin: 0.04,
              end: 0,
              duration: Motion.reveal,
              curve: Motion.curve);
}
