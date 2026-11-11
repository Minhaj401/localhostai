import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../theme/theme.dart';
import '../theme/tokens.dart';

class CommandPill extends StatefulWidget {
  const CommandPill(
      {super.key, required this.command, this.label = 'Copy command'});

  final String command;
  final String label;

  @override
  State<CommandPill> createState() => _CommandPillState();
}

class _CommandPillState extends State<CommandPill> {
  bool _copied = false;
  Timer? _timer;

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.command));
    if (!mounted) return;
    setState(() => _copied = true);
    _timer?.cancel();
    _timer = Timer(const Duration(milliseconds: 1500), () {
      if (mounted) setState(() => _copied = false);
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.only(left: 14),
        decoration: BoxDecoration(
          color: C.surface2,
          border: Border.all(color: C.border),
          borderRadius: BorderRadius.circular(R.md),
        ),
        child: Row(
          children: [
            Text(r'$', style: mono(13, color: C.text3)),
            const SizedBox(width: 8),
            Expanded(
                child: Text(widget.command, style: mono(13, color: C.text))),
            IconButton(
              tooltip: _copied ? 'Copied' : widget.label,
              onPressed: _copy,
              icon: Icon(_copied ? LucideIcons.check : LucideIcons.copy,
                  color: _copied ? C.accent : C.text2, size: 18),
            ),
          ],
        ),
      );
}
