import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import 'llm_backend.dart';
import '../models/chat_message.dart';

/// Talks to a PrivateLM-LAN node discovered on the local network
/// (or entered manually). The heavy inference runs on the laptop's
/// CPU/GPU; the phone only relays the stream.
class LaptopNodeBackend extends LlmBackend {
  LaptopNodeBackend({required this.baseUrl});

  final String baseUrl; // e.g. http://192.168.1.20:8765
  String? _model;
  Map<String, dynamic>? _health;

  @override
  String get id => 'laptop';

  @override
  String get displayName => 'Laptop node';

  @override
  bool get isReady => _health?['ok'] == true;

  Future<Map<String, dynamic>?> health() async {
    try {
      final r = await http.get(Uri.parse('$baseUrl/health')).timeout(const Duration(seconds: 4));
      if (r.statusCode == 200) {
        _health = jsonDecode(r.body) as Map<String, dynamic>;
        return _health;
      }
    } catch (_) {}
    _health = null;
    return null;
  }

  List<String> get models =>
      (_health?['models'] as List?)?.cast<String>() ?? const [];

  @override
  Future<void> ensureReady() async {
    final h = await health();
    if (h == null || h['ok'] != true) {
      throw StateError(
          'Laptop node unreachable — start it on the laptop: '
          'cd laptop-node && ./run.sh, then tap the laptop icon to pick a node');
    }
    _model ??= models.isNotEmpty ? models.first : null;
  }

  @override
  Future<String?> currentModel() async => _model;

  @override
  Future<void> setModel(String? model) async => _model = model;

  @override
  Stream<StreamChunk> chat(
    List<ChatMessage> messages, {
    String? systemPrompt,
  }) async* {
    final body = {
      'model': _model,
      'stream': true,
      'messages': [
        if (systemPrompt != null && systemPrompt.isNotEmpty)
          {'role': 'system', 'content': systemPrompt},
        ...messages.map((m) => {'role': m.role, 'content': m.content}),
      ],
    };
    final request = http.Request(
      'POST',
      Uri.parse('$baseUrl/v1/chat/completions'),
    )..headers['Content-Type'] = 'application/json';
    request.body = jsonEncode(body);

    final response = await request.send().timeout(const Duration(seconds: 300));
    if (response.statusCode >= 400) {
      final detail = await response.stream.bytesToString();
      throw StateError('Node error ${response.statusCode}: $detail');
    }
    yield* parseOpenAiStream(response);
  }

  @override
  Future<void> dispose() async {}
}
