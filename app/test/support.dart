import 'package:privatelm_lan/models/chat_message.dart';
import 'package:privatelm_lan/services/llm_backend.dart';
import 'package:privatelm_lan/services/storage_service.dart';

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
}

class FakeBackend extends LlmBackend {
  FakeBackend({this.ready, this.reply = 'reply', this.id = 'cloud'});
  final Future<void>? ready;
  final String reply;
  bool disposed = false;
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
    yield StreamChunk(reply);
    yield const StreamChunk('', done: true);
  }

  @override
  Future<void> dispose() async {
    disposed = true;
  }
}
