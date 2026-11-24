import 'package:flutter/foundation.dart' show kIsWeb;
import 'dart:math';
import 'package:get/get.dart';

import '../models/chat_message.dart';
import '../services/llm_backend.dart';
import '../services/laptop_node_service.dart';
import '../services/node_discovery.dart';
import '../services/storage_service.dart';
import '../services/conversation_library.dart';

/// Owns chat state and builds the laptop backend from stored settings.
/// localhost.ai runs on your own hardware: the paired laptop is the only
/// inference backend.
class ChatController extends GetxController {
  ChatController(this._storage, {LlmBackend Function()? backendFactory})
      : _backendFactory = backendFactory,
        shared = ConversationLibrary(_storage);

  final StorageService _storage;
  final LlmBackend Function()? _backendFactory;
  int _generation = 0;
  final ConversationLibrary shared;
  String? _sharedId;
  int _sharedRevision = -1;
  final retryDraft = RxnString();

  final messages = <ChatMessage>[].obs;
  final isGenerating = false.obs;
  final error = RxnString(null);
  final streamingText = ''.obs;

  LlmBackend? _backend;

  LlmBackend get backend => _backend ??= _backendFactory?.call() ?? _buildBackend();

  @override
  void onInit() {
    super.onInit();
    shared.addListener(_sharedChanged);
    for (final raw in _storage.loadMessages()) {
      try {
        messages.add(ChatMessage.fromJson(raw));
      } catch (_) {
        // A malformed old record must not prevent the rest from loading.
      }
    }
  }

  LlmBackend _buildBackend() {
    // Prefer the paired node: the scanned QR carries host, port and token.
    final pair = _storage.loadPair();
    if (pair != null) {
      return LaptopNodeBackend(baseUrl: pair.baseUrl, token: pair.token);
    }
    final url = _storage.getString('laptop_node_url') ?? '';
    return LaptopNodeBackend(
      baseUrl: url,
      token: _storage.getString('pair_token'),
    );
  }

  /// Disposes the current backend so the next access rebuilds it from settings.
  Future<void> rebuildBackend() async {
    _generation++;
    isGenerating.value = false;
    streamingText.value = '';
    final old = _backend;
    _backend = null;
    await old?.dispose();
  }

  /// Finds a working laptop node before the user's first message fails.
  /// Tries, in order:
  ///   1. the saved URL (last known good node)
  ///   2. web: the node that served this page, then host:8765, then loopback
  ///   3. native: LAN discovery (UDP broadcast + subnet scan)
  Future<void> autoDiscoverNode() async {
    if (_storage.loadPair() != null) return;
    final generation = _generation;
    final saved = _storage.getString('laptop_node_url');
    if (saved != null && saved.isNotEmpty) {
      final test = LaptopNodeBackend(baseUrl: saved);
      final h = await test.health();
      await test.dispose();
      if (h != null) {
        return; // The node may be reachable before loading a model.
      }
    }
    String? url;

    if (kIsWeb) {
      // Browsers can't broadcast or scan. But when the node serves this app
      // itself, our own origin IS the node address.
      final candidates = <String>[
        if (Uri.base.port == NodeDiscovery.nodePort) Uri.base.origin
      ];
      final host = Uri.base.host.isEmpty ? '127.0.0.1' : Uri.base.host;
      candidates.add('${Uri.base.scheme}://$host:${NodeDiscovery.nodePort}');
      candidates.add('http://127.0.0.1:${NodeDiscovery.nodePort}');
      for (final c in candidates) {
        final candidate = LaptopNodeBackend(baseUrl: c);
        final h = await candidate.health();
        await candidate.dispose();
        if (h != null) {
          url = c;
          break;
        }
      }
    } else {
      final nodes = await NodeDiscovery().discover();
      if (nodes.isNotEmpty) {
        url = 'http://${nodes.first['address']}:${nodes.first['port']}';
      }
    }

    if (url != null && generation == _generation && !isClosed) {
      await _storage.selectNode(url);
      await rebuildBackend();
    }
  }

  Future<void> send(String text) async {
    if (text.trim().isEmpty || isGenerating.value) return;
    if (backend is LaptopNodeBackend) {
      await _sendShared(text.trim());
      return;
    }
    final content = text.trim();
    final context =
        int.tryParse(_storage.getString('context_tokens') ?? '') ?? 4096;
    final maximum = (context * 3 ~/ 2).clamp(1024, 1024 * 1024);
    if (content.length > maximum) {
      error.value =
          'This message is too large for the selected context. Shorten it or choose a larger context.';
      return;
    }
    error.value = null;
    messages.add(ChatMessage(role: 'user', content: content));
    final generation = ++_generation;
    final current = backend;
    isGenerating.value = true;
    streamingText.value = '';
    final buffer = StringBuffer();
    try {
      await _persist();
      await current.ensureReady();
      final model = await current.currentModel();
      if (generation != _generation || isClosed) return;
      await for (final chunk in current.chat(
        _boundedMessages(),
        systemPrompt: _storage.getString('system_prompt'),
      )) {
        if (generation != _generation || isClosed) return;
        buffer.write(chunk.delta);
        streamingText.value = buffer.toString();
        if (chunk.done) break;
      }
      if (generation != _generation || isClosed) return;
      messages.add(ChatMessage(
        role: 'assistant',
        content: buffer.toString(),
        backend: current.displayName,
        model: model,
      ));
      await _persist();
    } catch (e) {
      if (generation == _generation && !isClosed) {
        if (buffer.isNotEmpty) {
          messages.add(ChatMessage(
            role: 'assistant',
            content: buffer.toString(),
            backend: current.displayName,
            interrupted: true,
          ));
          await _persist();
        }
        error.value = '$e';
      }
    } finally {
      if (generation == _generation && !isClosed) {
        isGenerating.value = false;
        streamingText.value = '';
      }
    }
  }

  Future<void> clearChat() async {
    if (shared.ready) {
      await stopGeneration();
      await shared.create();
      await shared.refresh();
      return;
    }
    final cancelling = rebuildBackend();
    messages.clear();
    error.value = null;
    await _storage.clearMessages();
    await cancelling;
  }

  Future<void> stopGeneration() async {
    if (shared.ready) {
      await rebuildBackend();
      try {
        await shared.cancel();
      } catch (failure) {
        error.value = '$failure';
      }
      shared.invalidate();
      await shared.refresh();
      return;
    }
    if (!isGenerating.value) return;
    final partial = streamingText.value;
    final current = _backend;
    await rebuildBackend();
    if (partial.isNotEmpty) {
      messages.add(ChatMessage(
        role: 'assistant',
        content: partial,
        backend: current?.displayName,
        interrupted: true,
      ));
      await _persist();
    }
  }

  List<ChatMessage> _boundedMessages() {
    final context =
        int.tryParse(_storage.getString('context_tokens') ?? '') ?? 4096;
    // Three UTF-16 characters/token is deliberately conservative for mixed
    // prose/code/Unicode and leaves roughly 25% for the answer and templates.
    final budget = (context * 3 * 3 ~/ 4).clamp(1024, 3 * 1024 * 1024);
    var used = 0;
    final selected = <ChatMessage>[];
    for (final message in messages.reversed) {
      final cost = message.content.length + 32;
      if (used + cost > budget) {
        if (selected.isEmpty) {
          selected.add(ChatMessage(
            role: message.role,
            content: message.content
                .substring(0, budget.clamp(0, message.content.length)),
            backend: message.backend,
            model: message.model,
            interrupted: message.interrupted,
          ));
        }
        break;
      }
      selected.add(message);
      used += cost;
    }
    return selected.reversed.toList();
  }

  Future<void> _persist() =>
      _storage.saveMessages(messages.map((m) => m.toJson()).toList());

  void _sharedChanged() {
    if (isClosed || isGenerating.value || !shared.ready) return;
    final chat = shared.current;
    if (chat?.id == _sharedId && (chat?.revision ?? -1) == _sharedRevision) {
      return;
    }
    final switched = chat?.id != _sharedId;
    _sharedId = chat?.id;
    _sharedRevision = chat?.revision ?? -1;
    messages.assignAll(chat?.messages ?? []);
    streamingText.value = '';
    if (switched && chat != null) {
      _storage.setString('context_tokens', '${chat.contextTokens}');
      _storage.setString('system_prompt', chat.systemPrompt);
    }
  }

  Future<void> _sendShared(String content) async {
    if (!shared.ready ||
        !shared.online ||
        shared.busy ||
        shared.current?.activeRequest != null) {
      error.value = shared.current?.activeRequest != null
          ? 'A reply is already running on the coordinator.'
          : 'Connect to the laptop and wait for conversation sync before sending.';
      retryDraft.value = content;
      return;
    }
    final generation = ++_generation;
    isGenerating.value = true;
    error.value = null;
    streamingText.value = '';
    final current = backend;
    try {
      await current.ensureReady();
      if (generation != _generation || isClosed) return;
      if (shared.current == null && !await shared.create()) {
        throw StateError(shared.error ?? 'Could not create a conversation.');
      }
      if (generation != _generation || isClosed) return;
      final chat = shared.current!;
      final context =
          int.tryParse(_storage.getString('context_tokens') ?? '') ??
              chat.contextTokens;
      final system = _storage.getString('system_prompt') ?? chat.systemPrompt;
      messages.assignAll(
          [...chat.messages, ChatMessage(role: 'user', content: content)]);
      if (current is! LaptopNodeBackend) {
        throw StateError('Shared conversations require the laptop backend.');
      }
      final random = Random.secure();
      final requestId = List.generate(
              16, (_) => random.nextInt(256).toRadixString(16).padLeft(2, '0'))
          .join();
      await for (final chunk in current.chatConversation(
          chat, content, requestId,
          contextTokens: context, systemPrompt: system)) {
        if (generation != _generation || isClosed) return;
        streamingText.value += chunk.delta;
        if (chunk.done) break;
      }
    } catch (failure) {
      if (generation == _generation && !isClosed) {
        error.value = '$failure';
        retryDraft.value = content;
      }
    } finally {
      if (generation == _generation && !isClosed) {
        isGenerating.value = false;
        shared.invalidate();
        await shared.refresh();
        _sharedRevision = -1;
        _sharedChanged();
      }
    }
  }

  @override
  void onClose() {
    _generation++;
    shared.removeListener(_sharedChanged);
    shared.dispose();
    _backend?.dispose();
    super.onClose();
  }
}
