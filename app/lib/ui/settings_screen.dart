import 'package:flutter/material.dart';
import 'package:get/get.dart';

import '../controllers/chat_controller.dart';
import '../services/storage_service.dart';

class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final chat = Get.find<ChatController>();
    final storage = Get.find<StorageService>();

    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: ListView(
        padding: const EdgeInsets.all(12),
        children: [
          _section(context, 'Backend'),
          Obx(() => RadioListTile<String>(
                title: const Text('Laptop node (LAN offload)'),
                subtitle: const Text('Inference runs on your laptop\'s CPU/GPU'),
                value: 'laptop',
                groupValue: chat.activeBackendId.value,
                onChanged: (v) => v != null ? chat.switchBackend(v) : null,
              )),
          Obx(() => RadioListTile<String>(
                title: const Text('Cloud (OpenAI-compatible)'),
                subtitle: const Text('OpenAI, Kimi, Groq, OpenRouter…'),
                value: 'cloud',
                groupValue: chat.activeBackendId.value,
                onChanged: (v) => v != null ? chat.switchBackend(v) : null,
              )),
          Obx(() => RadioListTile<String>(
                title: const Text('Anthropic'),
                value: 'anthropic',
                groupValue: chat.activeBackendId.value,
                onChanged: (v) => v != null ? chat.switchBackend(v) : null,
              )),
          Obx(() => RadioListTile<String>(
                title: const Text('On-device'),
                subtitle: const Text('Not wired in this build — see local_inference_stub.dart'),
                value: 'local',
                groupValue: chat.activeBackendId.value,
                onChanged: (v) => v != null ? chat.switchBackend(v) : null,
              )),
          const SizedBox(height: 16),
          _section(context, 'Laptop node'),
          _field(
            label: 'Node URL (fallback when discovery fails)',
            hint: 'http://192.168.1.20:8765',
            initial: storage.getString('laptop_node_url') ?? '',
            onSave: (v) => storage.setString('laptop_node_url', v),
          ),
          const SizedBox(height: 16),
          _section(context, 'Cloud API'),
          _field(
            label: 'Base URL',
            hint: 'https://api.openai.com/v1',
            initial: storage.getString('cloud_base_url') ?? '',
            onSave: (v) => storage.setString('cloud_base_url', v),
          ),
          _field(
            label: 'API key',
            hint: 'sk-…',
            obscure: true,
            initial: storage.getString('cloud_key') ?? '',
            onSave: (v) => storage.setString('cloud_key', v),
          ),
          _field(
            label: 'Default model',
            hint: 'gpt-4o-mini',
            initial: storage.getString('cloud_model') ?? '',
            onSave: (v) => storage.setString('cloud_model', v),
          ),
          const SizedBox(height: 16),
          _section(context, 'Anthropic'),
          _field(
            label: 'API key',
            hint: 'sk-ant-…',
            obscure: true,
            initial: storage.getString('anthropic_key') ?? '',
            onSave: (v) => storage.setString('anthropic_key', v),
          ),
          const SizedBox(height: 16),
          _section(context, 'System prompt'),
          _field(
            label: 'Sent to every backend',
            hint: 'You are a helpful assistant…',
            initial: storage.getString('system_prompt') ?? '',
            maxLines: 4,
            onSave: (v) => storage.setString('system_prompt', v),
          ),
        ],
      ),
    );
  }

  Widget _section(BuildContext context, String title) => Padding(
        padding: const EdgeInsets.only(bottom: 4),
        child: Text(title,
            style: TextStyle(
                fontSize: 13,
                fontWeight: FontWeight.w600,
                color: Theme.of(context).colorScheme.primary)),
      );

  Widget _field({
    required String label,
    required String hint,
    required String initial,
    required ValueChanged<String> onSave,
    bool obscure = false,
    int maxLines = 1,
  }) {
    final controller = TextEditingController(text: initial);
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: TextField(
        controller: controller,
        obscureText: obscure,
        maxLines: maxLines,
        decoration: InputDecoration(
          labelText: label,
          hintText: hint,
          border: const OutlineInputBorder(),
          isDense: true,
        ),
        onSubmitted: onSave,
        onTapOutside: (_) => onSave(controller.text),
      ),
    );
  }
}
