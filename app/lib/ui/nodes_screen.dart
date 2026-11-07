import 'package:flutter/material.dart';
import 'package:get/get.dart';

import '../controllers/chat_controller.dart';
import '../services/node_discovery.dart';
import '../services/phone_worker_service.dart';
import '../services/storage_service.dart';

/// Discovers PrivateLM-LAN nodes on the network and lets the user pick one
/// as the active backend. Manual entry is always available (web, or when
/// broadcast is blocked by the AP).
class NodesScreen extends StatefulWidget {
  const NodesScreen({super.key});

  @override
  State<NodesScreen> createState() => _NodesScreenState();
}

class _NodesScreenState extends State<NodesScreen> {
  final discovery = NodeDiscovery();
  final worker = Get.put(PhoneWorkerService());
  bool scanning = false;
  List<Map<String, dynamic>> nodes = [];

  @override
  void initState() {
    super.initState();
    _scan();
  }

  /// Offer this phone's RAM/CPU to the laptop as an RPC worker.
  Widget _workerCard(BuildContext context) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Icon(Icons.memory),
                const SizedBox(width: 8),
                const Expanded(
                  child: Text('Share this phone\'s compute',
                      style: TextStyle(fontWeight: FontWeight.w600)),
                ),
                Obx(() => Switch(
                      value: worker.isRunning,
                      onChanged: (on) async {
                        if (on) {
                          try {
                            await worker.start();
                          } catch (_) {
                            Get.snackbar('Worker', 'Failed to start RPC worker');
                          }
                        } else {
                          worker.stop();
                        }
                      },
                    )),
              ],
            ),
            const Text(
              'Runs a llama.cpp RPC worker on :50052. The laptop splits model '
              'layers onto this phone\'s memory while the switch is on '
              '(keep the app open).',
              style: TextStyle(fontSize: 12),
            ),
            const SizedBox(height: 4),
            Obx(() => Text(
                  'status: ${worker.state.value.isEmpty ? "off" : worker.state.value}',
                  style: TextStyle(
                      fontSize: 12,
                      color: Theme.of(context).colorScheme.primary),
                )),
          ],
        ),
      ),
    );
  }

  Future<void> _scan() async {
    setState(() => scanning = true);
    nodes = await discovery.discover();
    if (mounted) setState(() => scanning = false);
  }

  Future<void> _select(String url) async {
    final chat = Get.find<ChatController>();
    await Get.find<StorageService>().setString('laptop_node_url', url);
    if (chat.activeBackendId.value == 'laptop') {
      await chat.rebuildBackend();
    } else {
      await chat.switchBackend('laptop');
    }
    if (mounted) {
      Get.back();
      Get.snackbar('Laptop node', 'Using $url', duration: const Duration(seconds: 2));
    }
  }

  Future<void> _manualEntry(BuildContext context) async {
    final controller = TextEditingController(
      text: Get.find<StorageService>().getString('laptop_node_url'),
    );
    final url = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Node address'),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: const InputDecoration(hintText: 'http://192.168.1.20:8765'),
        ),
        actions: [
          TextButton(onPressed: () => Get.back(), child: const Text('Cancel')),
          FilledButton(
            onPressed: () => Get.back(result: controller.text.trim()),
            child: const Text('Save'),
          ),
        ],
      ),
    );
    if (url != null && url.isNotEmpty) await _select(url);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Laptop nodes'),
        actions: [
          IconButton(
            icon: scanning
                ? const SizedBox(
                    width: 18, height: 18,
                    child: CircularProgressIndicator(strokeWidth: 2))
                : const Icon(Icons.radar),
            onPressed: scanning ? null : _scan,
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.all(12),
        children: [
          _workerCard(context),
          const SizedBox(height: 8),
          if (nodes.isEmpty && !scanning)
            const Padding(
              padding: EdgeInsets.all(24),
              child: Text(
                'No nodes found.\n\nStart the node on your laptop:\n'
                '  cd laptop-node && python3 server.py\n\n'
                'If broadcast discovery is blocked (some Wi-Fi APs do this), '
                'add the node manually below.',
              ),
            ),
          for (final node in nodes)
            Card(
              child: ListTile(
                leading: const Icon(Icons.laptop_mac),
                title: Text('${node['name']}'),
                subtitle: Text(
                  '${node['address']}:${node['port']} · '
                  'backend: ${node['backend']} · '
                  'models: ${(node['models'] as List?)?.join(', ') ?? '—'}',
                ),
                trailing: const Icon(Icons.chevron_right),
                onTap: () => _select('http://${node['address']}:${node['port']}'),
              ),
            ),
          const SizedBox(height: 8),
          OutlinedButton.icon(
            icon: const Icon(Icons.edit),
            label: const Text('Add node manually'),
            onPressed: () => _manualEntry(context),
          ),
        ],
      ),
    );
  }
}
