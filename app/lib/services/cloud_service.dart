import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import 'llm_backend.dart';
import '../models/chat_message.dart';

/// Cloud providers with OpenAI-shaped APIs (OpenAI, Kimi, Groq, OpenRouter,
/// or any self-hosted OpenAI-compatible endpoint). Anthropic/Gemini shape
/// differences are handled by [AnthropicBackend] below.
class OpenAiCompatibleBackend extends LlmBackend {
  OpenAiCompatibleBackend({
    required this.id,
    required this.displayName,
    required String baseUrl,
    required this.apiKey,
    this.defaultModel,
    http.Client? client,
  })  : baseUrl = baseUrl.trim().replaceFirst(RegExp(r'/+$'), ''),
        _http = client ?? http.Client();

  @override
  final String id;
  @override
  final String displayName;
  final String baseUrl;
  final http.Client _http;
  final String apiKey;
  final String? defaultModel;
  String? _model;

  @override
  bool get isReady =>
      apiKey.isNotEmpty && (_model ?? defaultModel)?.isNotEmpty == true;

  @override
  Future<void> ensureReady() async {
    if (apiKey.isEmpty) throw StateError('$displayName: no API key set');
    _model ??= defaultModel;
    if (_model == null || _model!.trim().isEmpty) {
      throw StateError('$displayName: choose a model in Settings');
    }
  }

  @override
  Future<String?> currentModel() async => _model ?? defaultModel;

  @override
  Future<void> setModel(String? model) async => _model = model;

  @override
  Stream<StreamChunk> chat(
    List<ChatMessage> messages, {
    String? systemPrompt,
  }) async* {
    final request = http.Request('POST', Uri.parse('$baseUrl/chat/completions'))
      ..headers['Content-Type'] = 'application/json'
      ..headers['Authorization'] = 'Bearer $apiKey';
    request.body = jsonEncode({
      'model': _model ?? defaultModel,
      'stream': true,
      'messages': [
        if (systemPrompt != null && systemPrompt.isNotEmpty)
          {'role': 'system', 'content': systemPrompt},
        ...messages.map((m) => {'role': m.role, 'content': m.content}),
      ],
    });
    final response =
        await _http.send(request).timeout(const Duration(seconds: 120));
    if (response.statusCode >= 400) {
      final detail = await response.stream.bytesToString();
      throw StateError('$displayName error ${response.statusCode}: $detail');
    }
    yield* parseOpenAiStream(response);
  }

  @override
  Future<void> dispose() async => _http.close();
}

/// Anthropic Messages API (system prompt is a top-level param, not a message).
class AnthropicBackend extends LlmBackend {
  AnthropicBackend(
      {required this.apiKey,
      this.defaultModel = 'claude-sonnet-4-5',
      http.Client? client})
      : _http = client ?? http.Client();

  @override
  final String id = 'anthropic';
  @override
  final String displayName = 'Anthropic';
  final String apiKey;
  final http.Client _http;
  final String? defaultModel;
  String? _model;

  @override
  bool get isReady => apiKey.isNotEmpty;

  @override
  Future<void> ensureReady() async {
    if (apiKey.isEmpty) throw StateError('Anthropic: no API key set');
    _model ??= defaultModel;
  }

  @override
  Future<String?> currentModel() async => _model ?? defaultModel;

  @override
  Future<void> setModel(String? model) async => _model = model;

  @override
  Stream<StreamChunk> chat(
    List<ChatMessage> messages, {
    String? systemPrompt,
  }) async* {
    final request =
        http.Request('POST', Uri.parse('https://api.anthropic.com/v1/messages'))
          ..headers['Content-Type'] = 'application/json'
          ..headers['x-api-key'] = apiKey
          ..headers['anthropic-version'] = '2023-06-01';
    request.body = jsonEncode({
      'model': _model ?? defaultModel,
      'max_tokens': 4096,
      'stream': true,
      if (systemPrompt != null && systemPrompt.isNotEmpty)
        'system': systemPrompt,
      'messages': messages
          .where((m) => m.role != 'system')
          .map((m) => {'role': m.role, 'content': m.content})
          .toList(),
    });
    final response =
        await _http.send(request).timeout(const Duration(seconds: 120));
    if (response.statusCode >= 400) {
      final detail = await response.stream.bytesToString();
      throw StateError('Anthropic error ${response.statusCode}: $detail');
    }
    // Anthropic SSE: content_block_delta events carry text deltas.
    await for (final data in sseData(response)) {
      if (data.trim().isEmpty) continue;
      final event = jsonDecode(data) as Map<String, dynamic>;
      if (event['type'] == 'error' || event['error'] != null) {
        throw StateError(streamError(event['error'] ?? event));
      }
      if (event['type'] == 'content_block_delta') {
        final text = (event['delta']?['text'] as String?) ?? '';
        if (text.isNotEmpty) yield StreamChunk(text);
      } else if (event['type'] == 'message_stop') {
        yield const StreamChunk('', done: true);
        return;
      }
    }
    throw StateError(
        'Anthropic reply stream ended before completion. Please retry.');
  }

  @override
  Future<void> dispose() async => _http.close();
}
