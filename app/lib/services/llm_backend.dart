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
  String get displayName;
  Future<void> ensureReady();
  Future<String?> currentModel();

  /// Streams an assistant reply for [messages]. Yields deltas.
  Stream<StreamChunk> chat(
    List<ChatMessage> messages, {
    String? systemPrompt,
  });

  Future<void> dispose();
}

/// Reassemble UTF-8, line endings, and multiline SSE data fields before parsing.
Stream<String> sseData(http.StreamedResponse response) async* {
  final data = <String>[];
  await for (final line in response.stream
      .timeout(const Duration(seconds: 300))
      .transform(utf8.decoder)
      .transform(const LineSplitter())) {
    if (line.isEmpty) {
      if (data.isNotEmpty) {
        yield data.join('\n');
        data.clear();
      }
    } else if (line.startsWith('data:')) {
      final value = line.substring(5);
      data.add(value.startsWith(' ') ? value.substring(1) : value);
    }
  }
  if (data.isNotEmpty) yield data.join('\n');
}

String streamError(Object error) {
  if (error is Map && error['message'] != null) return '${error['message']}';
  return '$error';
}

Stream<StreamChunk> parseOpenAiStream(http.StreamedResponse response) async* {
  await for (final data in sseData(response)) {
    final payload = data.trim();
    if (payload.isEmpty) continue;
    if (payload == '[DONE]') {
      yield const StreamChunk('', done: true);
      return;
    }
    final json = jsonDecode(payload) as Map<String, dynamic>;
    if (json['error'] != null) throw StateError(streamError(json['error']));
    final choices = json['choices'] as List?;
    final delta = choices != null && choices.isNotEmpty
        ? (choices.first['delta']?['content'] as String?) ?? ''
        : '';
    if (delta.isNotEmpty) yield StreamChunk(delta);
    if (choices != null &&
        choices.isNotEmpty &&
        choices.first['finish_reason'] != null) {
      yield const StreamChunk('', done: true);
      return;
    }
  }
  throw StateError(
      'The reply stream ended before completion. Check the connection and retry.');
}
