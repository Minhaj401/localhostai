import 'dart:convert';
import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import '../models/conversation.dart';
import 'node_client.dart';
import 'storage_service.dart';

/// Coordinator-backed library; no full-history uploads or last-writer-wins edits.
class ConversationLibrary extends ChangeNotifier {
  ConversationLibrary(this.storage);
  final StorageService storage;
  NodeClient? client;
  Conversation? current;
  List<ConversationSummary> conversations = [];
  String query = '';
  String? error;
  String? storeId;
  bool ready = false;
  bool online = false;
  bool busy = false;
  bool _disposed = false;
  int _epoch = 0;
  Future<void>? _refreshing;
  String? _identity;
  String? _cachedId;
  int? _cachedRevision;
  bool get canImport => storage.loadMessages().isNotEmpty;

  void attach(NodeClient next) {
    final identity = sha256
        .convert(utf8.encode('${next.baseUrl}\n${next.pair.token}'))
        .toString();
    client = next;
    if (_identity == identity) return;
    _identity = identity;
    _epoch++;
    current = null;
    _cachedId = null;
    _cachedRevision = null;
    conversations = [];
    query = '';
    storeId = null;
    ready = online = busy = false;
    _refreshing = null;
    error = null;
    try {
      final raw = storage.getString('shared_conversation_cache');
      final cached = raw == null || raw.isEmpty ? null : jsonDecode(raw) as Map;
      if (cached?['identity'] == identity) {
        current = Conversation.fromJson(
            Map<String, dynamic>.from(cached!['conversation'] as Map));
        storeId = cached['store_id'] as String;
        ready = true;
        _cachedId = current!.id;
        _cachedRevision = current!.revision;
        conversations = [
          ConversationSummary.fromJson({
            ...current!.json,
            'message_count': current!.messages.length,
            'preview': current!.messages.lastOrNull?.content ?? '',
            'generating': false
          })
        ];
      }
    } catch (_) {/* A corrupt cache cannot replace coordinator data. */}
    notifyListeners();
  }

  void detach() {
    _epoch++;
    client = null;
    _identity = null;
    ready = online = false;
    current = null;
    conversations = [];
    _refreshing = null;
    notifyListeners();
  }

  Future<void> refresh() {
    if (_disposed || client == null || busy) return Future.value();
    if (_refreshing != null) return _refreshing!;
    late final Future<void> pending;
    pending = _refresh().whenComplete(() {
      if (identical(_refreshing, pending)) _refreshing = null;
    });
    _refreshing = pending;
    return pending;
  }

  Future<void> _refresh() async {
    final epoch = _epoch;
    final node = client!;
    final identity = _identity;
    try {
      final value = await node
          .document('/conversations?query=${Uri.encodeQueryComponent(query)}');
      if (_disposed || epoch != _epoch) return;
      final list = (value['conversations'] as List)
          .map((c) =>
              ConversationSummary.fromJson(Map<String, dynamic>.from(c as Map)))
          .toList();
      final id = value['store_id'] as String;
      final saved = storage.getString('conversation_selected:$id');
      final selected = current?.id ??
          (saved == null || saved.isEmpty ? list.firstOrNull?.id : saved);
      Conversation? next;
      if (selected != null && selected.isNotEmpty) {
        try {
          next = await _get(node, selected);
        } on NodeHttpException catch (failure) {
          if (failure.status != 404) rethrow;
          if (list.isNotEmpty) next = await _get(node, list.first.id);
        }
      }
      if (_disposed || epoch != _epoch) return;
      storeId = id;
      conversations = list;
      current = next;
      ready = online = true;
      error = null;
      if (next != null &&
          (_cachedId != next.id || _cachedRevision != next.revision)) {
        await storage.setString('conversation_selected:$id', next.id);
        await storage.setString(
            'shared_conversation_cache',
            jsonEncode({
              'identity': identity,
              'store_id': id,
              'conversation': next.json
            }));
        if (!_disposed && epoch == _epoch) {
          _cachedId = next.id;
          _cachedRevision = next.revision;
        }
      } else if (next == null) {
        await storage.setString('shared_conversation_cache', '');
        _cachedId = null;
        _cachedRevision = null;
      }
    } catch (failure) {
      if (_disposed || epoch != _epoch) return;
      online = false;
      error = 'Conversation sync unavailable: $failure';
    }
    if (!_disposed && epoch == _epoch) notifyListeners();
  }

  Future<Conversation> _get(NodeClient node, String id) async =>
      Conversation.fromJson(Map<String, dynamic>.from(
          (await node.document('/conversations/$id'))['conversation'] as Map));

  Future<bool> _change(
      Future<Conversation?> Function(NodeClient) operation) async {
    if (busy || client == null || _disposed) return false;
    final epoch = ++_epoch;
    busy = true;
    error = null;
    notifyListeners();
    try {
      final next = await operation(client!);
      if (_disposed || epoch != _epoch) return false;
      current = next;
      ready = online = true;
      if (storeId != null) {
        await storage.setString(
            'conversation_selected:$storeId', next?.id ?? '');
      }
      return true;
    } catch (failure) {
      if (!_disposed && epoch == _epoch) error = '$failure';
      return false;
    } finally {
      if (!_disposed && epoch == _epoch) {
        busy = false;
        notifyListeners();
      }
    }
  }

  Future<bool> create() =>
      _change((node) async => Conversation.fromJson(Map<String, dynamic>.from(
          (await node.document('/conversations', body: {}))['conversation']
              as Map)));
  Future<bool> open(String id) => !online && current?.id == id
      ? Future.value(true)
      : _change((node) => _get(node, id));
  Future<bool> importPhoneHistory() =>
      _change((node) async => Conversation.fromJson(Map<String, dynamic>.from(
              (await node.document('/conversations', body: {
            'title': 'Imported phone chat',
            'messages': storage.loadMessages()
          }))['conversation'] as Map)));
  Future<bool> rename(String title) async {
    final chat = current;
    if (chat == null) return false;
    return _change((node) async => Conversation.fromJson(
            Map<String, dynamic>.from((await node
                .document('/conversations/${chat.id}', body: {
          'revision': chat.revision,
          'title': title
        }))['conversation'] as Map)));
  }

  Future<bool> delete(ConversationSummary chat) => _change((node) async {
        await node.document('/conversations/${chat.id}/delete',
            body: {'revision': chat.revision});
        return current?.id == chat.id ? null : current;
      });
  Future<Map<String, dynamic>> export(String format) async {
    final chat = current;
    if (chat == null || client == null) {
      throw StateError('Open a shared conversation first.');
    }
    return client!.document('/conversations/${chat.id}/export?format=$format');
  }

  Future<void> cancel() async {
    final chat = current;
    if (chat?.activeRequest == null || client == null) return;
    await client!.document('/conversations/${chat!.id}/cancel',
        body: {'request_id': chat.activeRequest});
    await refresh();
  }

  void search(String value) {
    query = value;
    if (busy) return;
    _epoch++;
    _refreshing = null;
  }

  void invalidate() {
    _epoch++;
    _refreshing = null;
  }

  @override
  void dispose() {
    _disposed = true;
    _epoch++;
    super.dispose();
  }
}
