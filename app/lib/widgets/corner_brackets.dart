import 'package:flutter/material.dart';

import '../theme/tokens.dart';

class CornerBrackets extends StatelessWidget {
  const CornerBrackets(
      {super.key, required this.child, this.inset = 10, this.len = 28});

  final Widget child;
  final double inset;
  final double len;

  @override
  Widget build(BuildContext context) => CustomPaint(
        foregroundPainter: _BracketPainter(inset, len),
        child: Padding(padding: EdgeInsets.all(inset + 6), child: child),
      );
}

class _BracketPainter extends CustomPainter {
  _BracketPainter(this.inset, this.len);
  final double inset;
  final double len;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = C.accent
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2.5
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    canvas.drawPath(
      Path()
        ..moveTo(size.width - inset - len, inset)
        ..lineTo(size.width - inset, inset)
        ..lineTo(size.width - inset, inset + len),
      paint,
    );
    canvas.drawPath(
      Path()
        ..moveTo(inset, size.height - inset - len)
        ..lineTo(inset, size.height - inset)
        ..lineTo(inset + len, size.height - inset),
      paint,
    );
  }

  @override
  bool shouldRepaint(_BracketPainter oldDelegate) =>
      oldDelegate.inset != inset || oldDelegate.len != len;
}
