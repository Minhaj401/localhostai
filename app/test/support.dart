import 'package:localhostai/models/chat_message.dart';
import 'package:localhostai/services/llm_backend.dart';
import 'package:localhostai/services/storage_service.dart';

class MemoryStorage extends StorageService {
  final settings = <String, String>{};
  List<Map<String, dynamic>> history = [];

  @override
  String? getString(String key) => settings[key];
  @override
  Future<void> setString(String key, String value) async {
    settings[key] = value;
  }

  @override
  List<Map<String, dynamic>> loadMessages() => history;
  @override
  Future<void> saveMessages(List<Map<String, dynamic>> messages) async {
    history = messages;
  }

  @override
  Future<void> clearMessages() async {
    history = [];
  }

  int? reserveMb;
  @override
  int? getReserveMb() => reserveMb;
  @override
  Future<void> setReserveMb(int? mb) async {
    reserveMb = mb;
  }
}

class FakeBackend extends LlmBackend {
  FakeBackend(
      {this.ready,
      this.reply = 'reply',
      this.id = 'cloud',
      this.failAfterReply = false});
  final Future<void>? ready;
  final String reply;
  bool disposed = false;
  final bool failAfterReply;
  List<ChatMessage> received = const [];
  @override
  final String id;
  @override
  String get displayName => id;
  @override
  bool get isReady => true;
  @override
  Future<void> ensureReady() async {
    await ready;
  }

  @override
  Future<String?> currentModel() async => 'test';
  @override
  Future<void> setModel(String? model) async {}
  @override
  Stream<StreamChunk> chat(List<ChatMessage> messages,
      {String? systemPrompt}) async* {
    received = messages;
    yield StreamChunk(reply);
    if (failAfterReply) throw StateError('transport interrupted');
    yield const StreamChunk('', done: true);
  }

  @override
  Future<void> dispose() async {
    disposed = true;
  }
}
