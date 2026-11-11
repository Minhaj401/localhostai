import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:google_fonts/google_fonts.dart';

import '../theme/theme.dart';
import '../theme/tokens.dart';

class Brand extends StatelessWidget {
  const Brand(
      {super.key, this.compact = false, this.size = 24, this.showMark = true});

  final bool compact;
  final double size;
  final bool showMark;

  @override
  Widget build(BuildContext context) => Semantics(
        label: 'localhost.ai. Run AI Locally.',
        excludeSemantics: true,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (showMark) ...[
              SvgPicture.asset('assets/logo/mark.svg', width: 48, height: 48),
              const SizedBox(width: 12),
            ],
            Flexible(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  FittedBox(
                    fit: BoxFit.scaleDown,
                    alignment: Alignment.centerLeft,
                    child: Text.rich(
                      const TextSpan(children: [
                        TextSpan(
                            text: 'localhost', style: TextStyle(color: C.text)),
                        TextSpan(
                            text: '.ai', style: TextStyle(color: C.accent)),
                      ]),
                      maxLines: 1,
                      softWrap: false,
                      style: GoogleFonts.outfit(
                        fontSize: size,
                        fontWeight: FontWeight.w600,
                        letterSpacing: -0.6,
                      ),
                    ),
                  ),
                  if (!compact)
                    Text('Run AI Locally', style: mono(12, ls: 1.2)),
                ],
              ),
            ),
          ],
        ),
      );
}
