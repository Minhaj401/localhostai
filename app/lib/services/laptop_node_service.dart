import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import 'llm_backend.dart';
import '../models/chat_message.dart';

/// Talks to a PrivateLM-LAN node discovered on the local network
/// (or entered manually). The heavy inference runs on the laptop's
/// CPU/GPU; the phone only relays the stream.
class LaptopNodeBackend extends LlmBackend {
  LaptopNodeBackend({required String baseUrl, this.token, http.Client? client})
      : baseUrl = baseUrl.trim().replaceFirst(RegExp(r'/+$'), ''),
        _http = client ?? http.Client();

  final String baseUrl; // e.g. http://192.168.1.20:8765
  final http.Client _http;

  /// Pair token from the scanned QR. Required when the node enforces pairing,
  /// otherwise every request is rejected with 401.
  final String? token;

  Map<String, String> get _authHeaders => {
        'Content-Type': 'application/json',
        if (token != null && token!.isNotEmpty)
          'Authorization': 'Bearer $token',
      };
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
      final r = await _http
          .get(Uri.parse('$baseUrl/health'), headers: _authHeaders)
          .timeout(const Duration(seconds: 4));
      if (r.statusCode == 200) {
        _health = jsonDecode(r.body) as Map<String, dynamic>;
        return _health;
      }
    } catch (_) {}
    _health = null;
    return null;
  }

  List<String> get models => ((_health?['models'] as List?) ?? const [])
      .map((m) => m is String ? m : (m is Map ? m['id'] : null))
      .whereType<String>()
      .toList();

  @override
  Future<void> ensureReady() async {
    final h = await health();
    if (h == null) {
      throw StateError(
          'Cannot reach the laptop. Run privateml on it, then pair from the home screen.');
    }
    if (h['ok'] != true) {
      throw StateError('The laptop is connected but has no model loaded. '
          'Open its terminal model picker, select a model, and press Enter.');
    }
    if (_model != null && !models.contains(_model)) _model = null;
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
    )
      ..headers['Content-Type'] = 'application/json'
      ..headers.addAll({
        if (token != null && token!.isNotEmpty)
          'Authorization': 'Bearer $token',
      });
    request.body = jsonEncode(body);

    final response =
        await _http.send(request).timeout(const Duration(seconds: 300));
    if (response.statusCode >= 400) {
      final detail = await response.stream.bytesToString();
      throw StateError('Node error ${response.statusCode}: $detail');
    }
    yield* parseOpenAiStream(response);
  }

  @override
  Future<void> dispose() async => _http.close();
}
