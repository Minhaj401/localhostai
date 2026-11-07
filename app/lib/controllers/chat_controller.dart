import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:get/get.dart';

import '../models/chat_message.dart';
import '../services/cloud_service.dart';
import '../services/llm_backend.dart';
import '../services/laptop_node_service.dart';
import '../services/local_inference_stub.dart';
import '../services/node_discovery.dart';
import '../services/storage_service.dart';

/// Builds the active backend from stored settings and owns chat state.
/// Mirrors PrivateLM's controller layer, with 'laptop' added as a first-class
/// backend alongside 'cloud' and 'local'.
class ChatController extends GetxController {
  ChatController(this._storage);

  final StorageService _storage;

  final messages = <ChatMessage>[].obs;
  final isGenerating = false.obs;
  final activeBackendId = 'laptop'.obs;
  final error = RxnString(null);
  final streamingText = ''.obs;

  LlmBackend? _backend;

  LlmBackend get backend {
    if (_backend != null) return _backend!;
    _backend = _buildBackend(activeBackendId.value);
    return _backend!;
  }

  LlmBackend _buildBackend(String id) {
    switch (id) {
      case 'laptop':
        final url = _storage.getString('laptop_node_url') ?? '';
        return LaptopNodeBackend(baseUrl: url);
      case 'local':
        return LocalInferenceBackend();
      case 'anthropic':
        return AnthropicBackend(
          apiKey: _storage.getString('anthropic_key') ?? '',
        );
      case 'cloud':
      default:
        return OpenAiCompatibleBackend(
          id: 'cloud',
          displayName: 'OpenAI-compatible',
          baseUrl:
              _storage.getString('cloud_base_url') ?? 'https://api.openai.com/v1',
          apiKey: _storage.getString('cloud_key') ?? '',
          defaultModel: _storage.getString('cloud_model'),
        );
    }
  }

  /// Disposes and rebuilds the backend for the current [activeBackendId].
  Future<void> rebuildBackend() async {
    await _backend?.dispose();
    _backend = _buildBackend(activeBackendId.value);
  }

  /// Finds a working laptop node before the user's first message fails.
  /// Tries, in order:
  ///   1. the saved URL (last known good node)
  ///   2. web: the node that served this page, then host:8765, then loopback
  ///   3. native: LAN discovery (UDP broadcast + subnet scan)
  ///   4. native fallback: http://127.0.0.1:8765 (adb reverse over USB)
  Future<void> autoDiscoverNode() async {
    if (activeBackendId.value != 'laptop') return;
    final saved = _storage.getString('laptop_node_url');
    if (saved != null && saved.isNotEmpty) {
      final test = LaptopNodeBackend(baseUrl: saved);
      final h = await test.health();
      if (h != null && h['ok'] == true) return; // saved URL still works
    }
    String? url;

    if (kIsWeb) {
      // Browsers can't broadcast or scan. But when the node serves this app
      // itself, our own origin IS the node address.
      final candidates = <String>[if (Uri.base.port == NodeDiscovery.nodePort) Uri.base.origin];
      final host = Uri.base.host.isEmpty ? '127.0.0.1' : Uri.base.host;
      candidates.add('${Uri.base.scheme}://$host:${NodeDiscovery.nodePort}');
      candidates.add('http://127.0.0.1:${NodeDiscovery.nodePort}');
      for (final c in candidates) {
        final h = await LaptopNodeBackend(baseUrl: c).health();
        if (h != null && h['ok'] == true) {
          url = c;
          break;
        }
      }
    } else {
      final nodes = await NodeDiscovery().discover();
      if (nodes.isNotEmpty) {
        url = 'http://${nodes.first['address']}:${nodes.first['port']}';
      } else {
        const usb = 'http://127.0.0.1:8765';
        final test = LaptopNodeBackend(baseUrl: usb);
        final h = await test.health();
        if (h != null && h['ok'] == true) url = usb;
      }
    }

    if (url != null) {
      await _storage.setString('laptop_node_url', url);
      await rebuildBackend();
    }
  }

  Future<void> switchBackend(String id) async {
    if (id == activeBackendId.value) return;
    await _backend?.dispose();
    _backend = null;
    activeBackendId.value = id;
    await _storage.setString('active_backend', id);
    error.value = null;
  }

  Future<void> send(String text) async {
    if (text.trim().isEmpty || isGenerating.value) return;
    error.value = null;
    messages.add(ChatMessage(role: 'user', content: text.trim()));
    _persist();

    isGenerating.value = true;
    streamingText.value = '';
    final buffer = StringBuffer();
    try {
      await backend.ensureReady();
      final model = await backend.currentModel();
      await for (final chunk in backend.chat(
        messages.toList(),
        systemPrompt: _storage.getString('system_prompt'),
      )) {
        buffer.write(chunk.delta);
        streamingText.value = buffer.toString();
        if (chunk.done) break;
      }
      messages.add(ChatMessage(
        role: 'assistant',
        content: buffer.toString(),
        backend: backend.displayName,
        model: model,
      ));
      _persist();
    } catch (e) {
      error.value = '$e';
    } finally {
      isGenerating.value = false;
      streamingText.value = '';
    }
  }

  Future<void> clearChat() async {
    messages.clear();
    await _storage.clearMessages();
  }

  void _persist() =>
      _storage.saveMessages(messages.map((m) => m.toJson()).toList());

  @override
  void onClose() {
    _backend?.dispose();
    super.onClose();
  }
}
