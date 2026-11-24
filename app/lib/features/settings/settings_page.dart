import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:get/get.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../../controllers/chat_controller.dart';
import '../../models/pairing.dart';
import '../../services/phone_worker_service.dart';
import '../../services/storage_service.dart';
import '../../theme/theme.dart';
import '../../theme/tokens.dart';
import '../../widgets/app_card.dart';
import '../../widgets/brand.dart';
import '../../widgets/common.dart';
import '../connection/connection_controller.dart';
import 'memory_settings.dart';

class SettingsScreen extends ConsumerWidget {
  const SettingsScreen({super.key, this.onNodeChanged, this.onReplaySetup});
  final Future<bool> Function(String)? onNodeChanged;
  final VoidCallback? onReplaySetup;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final chat = Get.find<ChatController>();
    final storage = Get.find<StorageService>();
    final connection = ref.watch(connectionProvider);
    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: SafeArea(
          top: false,
          child: ListView(
            keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
            children: [
              AdaptiveContent(
                  child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const SectionHeader('Make it yours',
                      eyebrow: 'Settings',
                      subtitle:
                          'Your laptop runs the models. Tune memory and behavior here.'),
                  const SizedBox(height: 24),
                  _LaptopSettings(
                      key: ValueKey(
                          '${connection.pair?.baseUrl}-${connection.info?.pairing == true}'),
                      chat: chat,
                      storage: storage,
                      pair: connection.pair,
                      pairingRequired: connection.info?.pairing == true,
                      onNodeChanged: onNodeChanged),
                  const SizedBox(height: 24),
                  const MemorySettings(),
                  const SizedBox(height: 24),
                  Text('Assistant behavior',
                      style: Theme.of(context).textTheme.titleLarge),
                  const SizedBox(height: 12),
                  AppCard(
                      child: _SettingField(
                    label: 'System prompt',
                    hint: 'You are a helpful assistant...',
                    initial: storage.getString('system_prompt') ?? '',
                    maxLines: 5,
                    onChanged: (value) =>
                        storage.setString('system_prompt', value),
                  )),
                  const SizedBox(height: 24),
                  AppCard(
                      child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                        const Brand(compact: true, size: 20),
                        const SizedBox(height: 12),
                        Text('0.3.2 / build 11', style: mono(12)),
                        if (onReplaySetup != null)
                          TextButton(
                              onPressed: onReplaySetup,
                              child: const Text('Replay setup')),
                      ])),
                ],
              ))
            ],
          )),
    );
  }
}

class _LaptopSettings extends StatefulWidget {
  const _LaptopSettings(
      {super.key,
      required this.chat,
      required this.storage,
      required this.pair,
      required this.pairingRequired,
      this.onNodeChanged});
  final ChatController chat;
  final StorageService storage;
  final PairPayload? pair;
  final bool pairingRequired;
  final Future<bool> Function(String)? onNodeChanged;
  @override
  State<_LaptopSettings> createState() => _LaptopSettingsState();
}

class _LaptopSettingsState extends State<_LaptopSettings> {
  String? _error;
  bool _applying = false;

  Future<void> _apply(String value) async {
    if (_applying) return;
    final uri = Uri.tryParse(value.trim());
    if (uri == null ||
        !['http', 'https'].contains(uri.scheme) ||
        uri.host.isEmpty) {
      setState(() => _error = 'Enter a complete HTTP(S) node address');
      return;
    }
    setState(() {
      _applying = true;
      _error = null;
    });
    try {
      if (widget.onNodeChanged != null) {
        if (!await widget.onNodeChanged!(uri.toString())) {
          throw StateError(
              'Could not connect. Check the address or use QR pairing.');
        }
      } else {
        final current = widget.storage
            .getString('laptop_node_url')
            ?.replaceFirst(RegExp(r'/+$'), '');
        final normalized = uri.toString().replaceFirst(RegExp(r'/+$'), '');
        if (normalized != current) {
          if (Get.isRegistered<PhoneWorkerService>()) {
            final worker = Get.find<PhoneWorkerService>();
            if (worker.sharing.value) await worker.stop();
          }
          await widget.storage.selectNode(normalized);
          await widget.chat.rebuildBackend();
        }
      }
    } catch (error) {
      if (mounted) {
        setState(
            () => _error = error.toString().replaceFirst('Bad state: ', ''));
      }
    } finally {
      if (mounted) setState(() => _applying = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final pair = widget.pair;
    return AppCard(
        child:
            Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Text('Coordinator', style: mono(12, color: C.text, w: FontWeight.w500)),
      if (pair != null) ...[
        const SizedBox(height: 12),
        Row(children: [
          Icon(
              widget.pairingRequired
                  ? LucideIcons.shieldCheck
                  : LucideIcons.link,
              color: widget.pairingRequired ? C.accent : C.text2,
              size: 20),
          const SizedBox(width: 10),
          Expanded(
              child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                Text(pair.name, style: Theme.of(context).textTheme.titleMedium),
                Text(pair.baseUrl, style: mono(12)),
                Text(
                    widget.pairingRequired
                        ? 'Pairing required / verified'
                        : 'Saved node / pairing optional',
                    style: mono(12,
                        color: widget.pairingRequired ? C.accent : C.text2)),
              ])),
        ]),
      ],
      const SizedBox(height: 16),
      _SettingField(
          label: 'Advanced node URL',
          hint: 'http://192.168.1.20:8765',
          initial: widget.storage.getString('laptop_node_url') ?? '',
          keyboardType: TextInputType.url,
          helperText: _applying
              ? 'Checking coordinator...'
              : 'Press done to validate and apply.',
          errorText: _error,
          onSubmitted: _apply),
    ]));
  }
}

class _SettingField extends StatefulWidget {
  const _SettingField(
      {required this.label,
      required this.hint,
      required this.initial,
      this.onChanged,
      this.onSubmitted,
      this.maxLines = 1,
      this.keyboardType,
      this.helperText,
      this.errorText});
  final String label;
  final String hint;
  final String initial;
  final Future<void> Function(String)? onChanged;
  final Future<void> Function(String)? onSubmitted;
  final int maxLines;
  final TextInputType? keyboardType;
  final String? helperText;
  final String? errorText;
  @override
  State<_SettingField> createState() => _SettingFieldState();
}

class _SettingFieldState extends State<_SettingField> {
  late final TextEditingController _controller =
      TextEditingController(text: widget.initial);

  @override
  void didUpdateWidget(_SettingField oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.initial != widget.initial &&
        _controller.text != widget.initial) {
      _controller.value = TextEditingValue(
        text: widget.initial,
        selection: TextSelection.collapsed(offset: widget.initial.length),
      );
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => TextField(
        controller: _controller,
        maxLines: widget.maxLines,
        keyboardType: widget.keyboardType,
        autocorrect: widget.keyboardType != TextInputType.url,
        enableSuggestions: widget.keyboardType != TextInputType.url,
        textInputAction: widget.maxLines > 1
            ? TextInputAction.newline
            : TextInputAction.done,
        decoration: InputDecoration(
          labelText: widget.label,
          hintText: widget.hint,
          helperText: widget.helperText,
          errorText: widget.errorText,
        ),
        onChanged: widget.onChanged,
        onSubmitted: widget.onSubmitted,
      );
}
