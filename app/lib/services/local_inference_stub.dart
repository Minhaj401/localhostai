import 'dart:async';

import 'llm_backend.dart';
import '../models/chat_message.dart';

/// On-device inference backend.
///
/// localhost.ai runs GGUF models via its custom llama.cpp plugin. This clone
/// keeps the same seam: platform-specific implementations are wired in per
/// target (Android/iOS use llama.cpp; web has no local engine), and this
/// stub is the fallback that throws a clear message.
///
/// To wire a real engine, replace this file per platform with
/// `local_inference_llama.dart` using conditional imports:
///
///   export 'local_inference_stub.dart'
///     if (dart.library.io) 'local_inference_llama.dart';
class LocalInferenceBackend extends LlmBackend {
  @override
  String get id => 'local';

  @override
  String get displayName => 'On-device';

  @override
  bool get isReady => false;

  @override
  Future<void> ensureReady() async {
    throw StateError(
      'On-device inference is not wired in this build — '
      'use the Laptop node or a Cloud backend. '
      'See lib/services/local_inference_stub.dart for the integration seam.',
    );
  }

  @override
  Future<String?> currentModel() async => null;

  @override
  Future<void> setModel(String? model) async {}

  @override
  Stream<StreamChunk> chat(
    List<ChatMessage> messages, {
    String? systemPrompt,
  }) async* {
    throw StateError('On-device inference not available in this build');
  }

  @override
  Future<void> dispose() async {}
}
