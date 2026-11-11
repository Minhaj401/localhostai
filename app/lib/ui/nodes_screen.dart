import 'package:flutter/material.dart';
import 'package:get/get.dart';

import '../controllers/chat_controller.dart';
import '../services/node_discovery.dart';
import '../services/phone_worker_service.dart';
import '../services/storage_service.dart';
import 'design_system.dart';

class NodesScreen extends StatefulWidget {
  const NodesScreen({super.key, this.onSelected, this.discovery});
  final Future<bool> Function(String)? onSelected;
  final NodeDiscovery? discovery;

  @override
  State<NodesScreen> createState() => _NodesScreenState();
}

class _NodesScreenState extends State<NodesScreen> {
  late final discovery = widget.discovery ?? NodeDiscovery();
  bool scanning = false;
  List<Map<String, dynamic>> nodes = const [];

  @override
  void initState() {
    super.initState();
    _scan();
  }

  Future<void> _scan() async {
    if (scanning) return;
    setState(() => scanning = true);
    final found = await discovery.discover();
    found.sort((a, b) => '${a['name']}'.compareTo('${b['name']}'));
    if (mounted) {
      setState(() {
        nodes = found;
        scanning = false;
      });
    }
  }

  Future<void> _select(String url) async {
    final uri = Uri.tryParse(url.trim());
    if (uri == null ||
        !['http', 'https'].contains(uri.scheme) ||
        uri.host.isEmpty) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('[x] Enter a complete HTTP(S) node address'),
          ),
        );
      }
      return;
    }
    if (widget.onSelected != null) {
      final accepted = await widget.onSelected!(url);
      if (mounted) {
        if (accepted) {
          Navigator.of(context).pop();
        } else {
          ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
            content: Text(
                'Could not connect. Use QR pairing if the node requires a token.'),
          ));
        }
      }
      return;
    }
    final storage = Get.find<StorageService>();
    final normalized = uri.toString().replaceFirst(RegExp(r'/+$'), '');
    final current =
        storage.getString('laptop_node_url')?.replaceFirst(RegExp(r'/+$'), '');
    if (current == normalized) {
      final chat = Get.find<ChatController>();
      if (chat.activeBackendId.value != 'laptop') {
        await chat.switchBackend('laptop');
      }
      if (mounted) Navigator.of(context).pop();
      return;
    }
    final worker = Get.find<PhoneWorkerService>();
    if (worker.sharing.value) await worker.stop();
    await storage.selectNode(uri.toString());

    final chat = Get.find<ChatController>();
    if (chat.activeBackendId.value == 'laptop') {
      await chat.rebuildBackend();
    } else {
      await chat.switchBackend('laptop');
    }
    if (mounted) {
      Navigator.of(context).pop();
    }
  }

  Future<void> _manualEntry(BuildContext context) async {
    final controller = TextEditingController(
      text: Get.find<StorageService>().getString('laptop_node_url') ?? '',
    );
    String? error;
    final url = await showDialog<String>(
      context: context,
      builder: (context) => StatefulBuilder(
        builder: (context, setDialogState) => AlertDialog(
          title: const Text('Enter node address'),
          content: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 430),
            child: TextField(
              controller: controller,
              autofocus: true,
              autocorrect: false,
              enableSuggestions: false,
              keyboardType: TextInputType.url,
              textInputAction: TextInputAction.done,
              decoration: InputDecoration(
                labelText: 'HTTP(S) address',
                hintText: 'http://192.168.1.20:8765',
                errorText: error,
              ),
              onSubmitted: (_) {
                final uri = Uri.tryParse(controller.text.trim());
                if (uri == null ||
                    !['http', 'https'].contains(uri.scheme) ||
                    uri.host.isEmpty) {
                  setDialogState(
                    () => error = 'Enter a complete HTTP(S) address',
                  );
                } else {
                  Navigator.pop(context, uri.toString());
                }
              },
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () {
                final uri = Uri.tryParse(controller.text.trim());
                if (uri == null ||
                    !['http', 'https'].contains(uri.scheme) ||
                    uri.host.isEmpty) {
                  setDialogState(
                    () => error = 'Enter a complete HTTP(S) address',
                  );
                } else {
                  Navigator.pop(context, uri.toString());
                }
              },
              child: const Text('Use node'),
            ),
          ],
        ),
      ),
    );
    controller.dispose();
    if (mounted && url != null) await _select(url);
  }

  @override
  Widget build(BuildContext context) {
    final storage = Get.find<StorageService>();
    final active = storage.getString('laptop_node_url');
    return Scaffold(
      appBar: AppBar(
        title: const Text('Laptop nodes'),
        actions: [
          IconButton(
            tooltip: scanning ? 'Scanning for laptops' : 'Scan again',
            icon: scanning
                ? const SizedBox.square(
                    dimension: 20,
                    child: CircularProgressIndicator(
                      strokeWidth: 2,
                      semanticsLabel: 'Scanning for laptop nodes',
                    ),
                  )
                : const AsciiGlyph('[R]', semanticLabel: 'Scan again'),
            onPressed: scanning ? null : _scan,
          ),
        ],
      ),
      body: AppBackdrop(
        child: RefreshIndicator(
          onRefresh: _scan,
          child: ListView(
            physics: const AlwaysScrollableScrollPhysics(),
            children: [
              AdaptiveContent(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    const SectionHeader(
                      'Nearby coordinators',
                      subtitle:
                          'Discovery stays on your current Wi-Fi network.',
                    ),
                    const SizedBox(height: 14),
                    AnimatedSwitcher(
                      duration: MediaQuery.disableAnimationsOf(context)
                          ? Duration.zero
                          : Motion.base,
                      child: scanning && nodes.isEmpty
                          ? const AppPanel(
                              key: ValueKey('scanning'),
                              child: Padding(
                                padding: EdgeInsets.symmetric(vertical: 34),
                                child: Column(
                                  children: [
                                    CircularProgressIndicator(),
                                    SizedBox(height: 14),
                                    Text('Looking across the local network...'),
                                  ],
                                ),
                              ),
                            )
                          : nodes.isEmpty
                              ? const AppPanel(
                                  key: ValueKey('empty'),
                                  child: Column(
                                    children: [
                                      AsciiGlyph(
                                        '[?]',
                                        semanticLabel: 'No laptop found',
                                      ),
                                      SizedBox(height: 14),
                                      Text(
                                        'No coordinator found',
                                        style: TextStyle(
                                          fontWeight: FontWeight.w700,
                                        ),
                                      ),
                                      SizedBox(height: 7),
                                      Text(
                                        'Start the coordinator on the laptop. If your router blocks discovery, enter its address manually.',
                                        textAlign: TextAlign.center,
                                      ),
                                    ],
                                  ),
                                )
                              : Column(
                                  key: const ValueKey('results'),
                                  children: nodes
                                      .map(
                                        (node) => Padding(
                                          padding:
                                              const EdgeInsets.only(bottom: 10),
                                          child: _nodeCard(
                                            context,
                                            node,
                                            active,
                                          ),
                                        ),
                                      )
                                      .toList(),
                                ),
                    ),
                    const SizedBox(height: 16),
                    OutlinedButton(
                      onPressed: () => _manualEntry(context),
                      child: const Text('Enter address manually'),
                    ),
                    const SizedBox(height: 12),
                    const NoticeBanner(
                      message:
                          'Selecting a different node stops phone sharing and clears credentials that belong to the old laptop.',
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _nodeCard(
    BuildContext context,
    Map<String, dynamic> node,
    String? active,
  ) {
    final theme = Theme.of(context);
    final address = '${node['address']}:${node['port']}';
    final url = 'http://$address';
    final models = node['models'] is List ? node['models'] as List : const [];
    final selected = active == url || active == '$url/';
    final backend = '${node['backend'] ?? 'none'}';
    return AppPanel(
      child: InkWell(
        borderRadius: BorderRadius.zero,
        onTap: () => _select(url),
        child: Padding(
          padding: const EdgeInsets.all(2),
          child: Row(
            children: [
              const AsciiGlyph(
                '[PC]',
                semanticLabel: 'Laptop node',
              ),
              const SizedBox(width: 13),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            '${node['name'] ?? address}',
                            style: theme.textTheme.titleMedium,
                          ),
                        ),
                        if (selected)
                          const StatusPill(
                            'Active',
                            tone: StatusTone.positive,
                          ),
                      ],
                    ),
                    const SizedBox(height: 4),
                    Text(
                      address,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                    const SizedBox(height: 7),
                    Wrap(
                      spacing: 6,
                      runSpacing: 6,
                      children: [
                        StatusPill(
                          backend,
                          tone: backend == 'none'
                              ? StatusTone.warning
                              : StatusTone.accent,
                        ),
                        StatusPill(
                          '${models.length} ${models.length == 1 ? "model" : "models"}',
                        ),
                      ],
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 6),
              const AsciiGlyph('[>]', semanticLabel: 'Select node'),
            ],
          ),
        ),
      ),
    );
  }
}
