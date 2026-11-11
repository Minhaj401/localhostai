import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../models/pairing.dart';
import 'design_system.dart';

/// Scan the QR the laptop shows, or type an address as a fallback.
class PairScreen extends StatefulWidget {
  const PairScreen({super.key, this.initialManual = false});
  final bool initialManual;

  @override
  State<PairScreen> createState() => _PairScreenState();
}

class _PairScreenState extends State<PairScreen> with WidgetsBindingObserver {
  final _formKey = GlobalKey<FormState>();
  final _hostCtrl = TextEditingController();
  final _portCtrl = TextEditingController(text: '8765');
  final _tokenCtrl = TextEditingController();

  MobileScannerController? _scanner;
  bool _handled = false;
  bool _manual = false;
  bool _scannerStarting = false;
  bool _obscureToken = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _manual = widget.initialManual;
    WidgetsBinding.instance.addObserver(this);
    _scanner = MobileScannerController(
      autoStart: false,
      detectionSpeed: DetectionSpeed.noDuplicates,
      facing: CameraFacing.back,
    );
    WidgetsBinding.instance.addPostFrameCallback((_) => _startScanner());
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (_manual || _scanner == null) return;
    if (state == AppLifecycleState.resumed) {
      unawaited(_startScanner());
    } else if (state == AppLifecycleState.inactive ||
        state == AppLifecycleState.paused) {
      unawaited(_scanner!.stop());
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _scanner?.dispose();
    _hostCtrl.dispose();
    _portCtrl.dispose();
    _tokenCtrl.dispose();
    super.dispose();
  }

  void _onDetect(BarcodeCapture capture) {
    if (_handled || !mounted || _manual) return;
    for (final barcode in capture.barcodes) {
      final raw = barcode.rawValue;
      if (raw == null) continue;
      final payload = PairPayload.tryParse(raw);
      if (payload != null) {
        _handled = true;
        Navigator.of(context).pop(payload);
        return;
      }
      setState(() => _error = 'That QR is not a localhost.ai pairing code.');
    }
  }

  Future<void> _setManual(bool manual, {bool clearError = true}) async {
    if (_manual == manual) return;
    if (manual) {
      await _scanner?.stop();
    }
    if (!mounted) return;
    setState(() {
      _manual = manual;
      if (clearError) _error = null;
    });
    if (!manual) await _startScanner();
  }

  Future<void> _startScanner() async {
    if (!mounted || _manual || _scanner == null || _scannerStarting) return;
    _scannerStarting = true;
    try {
      await _scanner!.start();
      if (mounted && _manual) await _scanner!.stop();
    } finally {
      _scannerStarting = false;
    }
  }

  void _submitManual() {
    if (_formKey.currentState?.validate() != true) return;
    final host = _hostCtrl.text.trim();
    final port = int.parse(_portCtrl.text.trim());
    final payload = PairPayload(
      host: host,
      port: port,
      token: _tokenCtrl.text.trim(),
      name: host,
    );
    if (!payload.isValid) {
      setState(() => _error = 'These pairing details are not valid.');
      return;
    }
    _handled = true;
    Navigator.of(context).pop(payload);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Connect laptop')),
      body: AppBackdrop(
        child: SafeArea(
          top: false,
          child: Column(
            children: [
              AdaptiveContent(
                padding: const EdgeInsets.fromLTRB(18, 8, 18, 14),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      'One code connects chat, models, and compute sharing.',
                      style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                            color:
                                Theme.of(context).colorScheme.onSurfaceVariant,
                          ),
                    ),
                    const SizedBox(height: 14),
                    Row(
                      children: [
                        Expanded(
                          child: _modeTab(
                            context,
                            label: 'Scan QR',
                            selected: !_manual,
                            onPressed: () => _setManual(false),
                          ),
                        ),
                        Expanded(
                          child: _modeTab(
                            context,
                            label: 'Enter details',
                            selected: _manual,
                            onPressed: () => _setManual(true),
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
              Expanded(
                child: AnimatedSwitcher(
                  duration: MediaQuery.disableAnimationsOf(context)
                      ? Duration.zero
                      : Motion.base,
                  child:
                      _manual ? _manualEntry(context) : _scannerView(context),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _scannerView(BuildContext context) {
    return Stack(
      key: const ValueKey('scanner'),
      fit: StackFit.expand,
      children: [
        if (_scanner != null)
          MobileScanner(
            controller: _scanner,
            onDetect: _onDetect,
            errorBuilder: (context, error, child) {
              WidgetsBinding.instance.addPostFrameCallback((_) {
                if (!mounted || _manual) return;
                setState(() {
                  _error =
                      'Camera access is unavailable. Enter the pairing details instead.';
                });
                unawaited(_setManual(true, clearError: false));
              });
              return const SizedBox.shrink();
            },
          ),
        const Align(
          alignment: Alignment.topCenter,
          child: FractionallySizedBox(
            widthFactor: 1,
            heightFactor: 0.18,
            child: IgnorePointer(child: ColoredBox(color: Color(0x66000000))),
          ),
        ),
        const Align(
          alignment: Alignment.bottomCenter,
          child: FractionallySizedBox(
            widthFactor: 1,
            heightFactor: 0.38,
            child: IgnorePointer(child: ColoredBox(color: Color(0x99000000))),
          ),
        ),
        LayoutBuilder(
          builder: (context, constraints) {
            final size =
                (constraints.biggest.shortestSide * 0.68).clamp(120.0, 300.0);
            return Center(
              child: Semantics(
                label: 'Place the localhost.ai QR code inside this frame',
                child: Container(
                  width: size,
                  height: size,
                  decoration: BoxDecoration(
                    border: Border.all(color: C.accent, width: 2),
                    borderRadius: BorderRadius.circular(R.lg),
                  ),
                ),
              ),
            );
          },
        ),
        Positioned(
          top: 12,
          right: 16,
          child: IconButton.filledTonal(
            tooltip: 'Toggle flashlight',
            onPressed: () => _scanner?.toggleTorch(),
            icon: const AsciiGlyph(
              '[*]',
              semanticLabel: 'Toggle flashlight',
            ),
          ),
        ),
        Positioned(
          left: 18,
          right: 18,
          bottom: 18,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 8,
                ),
                decoration: const BoxDecoration(
                  color: AppColors.canvas,
                  borderRadius: BorderRadius.all(Radius.circular(R.md)),
                ),
                child: const Row(
                  children: [
                    AsciiGlyph('[>]', semanticLabel: 'Terminal command'),
                    SizedBox(width: 12),
                    Expanded(
                      child: Text(
                        'On the laptop, press Ctrl+P and point this camera at the code.',
                      ),
                    ),
                  ],
                ),
              ),
              if (_error != null) ...[
                const SizedBox(height: 10),
                NoticeBanner(message: _error!, tone: StatusTone.danger),
              ],
            ],
          ),
        ),
      ],
    );
  }

  Widget _modeTab(
    BuildContext context, {
    required String label,
    required bool selected,
    required VoidCallback onPressed,
  }) {
    return Semantics(
      selected: selected,
      button: true,
      child: DecoratedBox(
        decoration: BoxDecoration(
          border: Border(
            bottom: BorderSide(
              color: selected ? C.accent : C.border,
              width: 1,
            ),
          ),
        ),
        child: TextButton(
          onPressed: onPressed,
          style: TextButton.styleFrom(
            shape:
                const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
          ),
          child: Text(
            label,
            style: TextStyle(
              color: selected ? AppColors.ink : AppColors.mute,
            ),
          ),
        ),
      ),
    );
  }

  Widget _manualEntry(BuildContext context) {
    final theme = Theme.of(context);
    return ListView(
      key: const ValueKey('manual'),
      keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
      padding: EdgeInsets.zero,
      children: [
        AdaptiveContent(
          child: Form(
            key: _formKey,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const SectionHeader(
                  'Enter pairing details',
                  subtitle: 'Use the values shown beside the laptop QR code.',
                ),
                const SizedBox(height: 16),
                AppPanel(
                  child: Column(
                    children: [
                      TextFormField(
                        controller: _hostCtrl,
                        decoration: const InputDecoration(
                          labelText: 'Laptop address',
                          hintText: '192.168.1.20',
                        ),
                        keyboardType: TextInputType.url,
                        textInputAction: TextInputAction.next,
                        autocorrect: false,
                        enableSuggestions: false,
                        validator: (value) =>
                            value == null || value.trim().isEmpty
                                ? 'Enter the laptop IP address or hostname'
                                : null,
                      ),
                      const SizedBox(height: 12),
                      TextFormField(
                        controller: _portCtrl,
                        decoration: const InputDecoration(
                          labelText: 'Port',
                        ),
                        keyboardType: TextInputType.number,
                        textInputAction: TextInputAction.next,
                        validator: (value) {
                          final port = int.tryParse(value?.trim() ?? '');
                          return port == null || port < 1 || port > 65535
                              ? 'Enter a port from 1 to 65535'
                              : null;
                        },
                      ),
                      const SizedBox(height: 12),
                      TextFormField(
                        controller: _tokenCtrl,
                        obscureText: _obscureToken,
                        decoration: InputDecoration(
                          labelText: 'Pairing token',
                          suffixIcon: IconButton(
                            tooltip: _obscureToken
                                ? 'Show pairing token'
                                : 'Hide pairing token',
                            onPressed: () =>
                                setState(() => _obscureToken = !_obscureToken),
                            icon: Icon(
                                _obscureToken
                                    ? LucideIcons.eye
                                    : LucideIcons.eyeOff,
                                size: 18),
                          ),
                        ),
                        autocorrect: false,
                        enableSuggestions: false,
                        textInputAction: TextInputAction.done,
                        onFieldSubmitted: (_) => _submitManual(),
                        validator: (value) =>
                            value == null || value.trim().isEmpty
                                ? 'Enter the token shown by the laptop'
                                : null,
                      ),
                    ],
                  ),
                ),
                if (_error != null) ...[
                  const SizedBox(height: 12),
                  NoticeBanner(message: _error!, tone: StatusTone.danger),
                ],
                const SizedBox(height: 16),
                PrimaryButton(
                  onPressed: _submitManual,
                  label: 'Connect laptop',
                ),
                const SizedBox(height: 12),
                Text(
                  'The token stays on this phone and is sent only to your laptop.',
                  textAlign: TextAlign.center,
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}
