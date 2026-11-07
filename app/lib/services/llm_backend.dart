import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import '../models/chat_message.dart';

/// A single chunk of streamed assistant output.
class StreamChunk {
  const StreamChunk(this.delta, {this.done = false});
  final String delta;
  final bool done;
}

/// Common interface every inference backend implements.
/// The chat UI never knows (or cares) where tokens come from.
abstract class LlmBackend {
  String get id; // 'laptop' | 'cloud' | 'local'
  String get displayName;
  bool get isReady;
  Future<void> ensureReady();
  Future<String?> currentModel();
  Future<void> setModel(String? model);

  /// Streams an assistant reply for [messages]. Yields deltas.
  Stream<StreamChunk> chat(
    List<ChatMessage> messages, {
    String? systemPrompt,
  });

  Future<void> dispose();
}

/// Minimal SSE parser for OpenAI-style `data:` lines.
Stream<StreamChunk> parseOpenAiStream(http.StreamedResponse response) async* {
  String buffer = '';
  await for (final chunk in response.stream.transform(utf8.decoder)) {
    buffer += chunk;
    while (buffer.contains('\n')) {
      final idx = buffer.indexOf('\n');
      final line = buffer.substring(0, idx).trim();
      buffer = buffer.substring(idx + 1);
      if (!line.startsWith('data:')) continue;
      final payload = line.substring(5).trim();
      if (payload == '[DONE]') {
        yield StreamChunk('', done: true);
        return;
      }
      try {
        final json = jsonDecode(payload) as Map<String, dynamic>;
        if (json['error'] != null) {
          yield StreamChunk('\n[error] ${json['error']}', done: true);
          return;
        }
        final choices = json['choices'] as List?;
        final delta = choices != null && choices.isNotEmpty
            ? (choices.first['delta']?['content'] as String?) ?? ''
            : '';
        if (delta.isNotEmpty) yield StreamChunk(delta);
      } catch (_) {
        // keep-alive / comment line — ignore
      }
    }
  }
  yield const StreamChunk('', done: true);
}
