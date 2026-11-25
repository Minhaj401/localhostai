import 'dart:async';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:localhostai/controllers/chat_controller.dart';
import 'package:localhostai/features/chat/conversations_page.dart';
import 'package:localhostai/models/pairing.dart';
import 'package:localhostai/services/conversation_library.dart';
import 'package:localhostai/services/laptop_node_service.dart';
import 'package:localhostai/services/node_client.dart';
import 'package:localhostai/theme/theme.dart';
import 'support.dart';

class WorkspaceServer {
  static const pair = PairPayload(
      host: '10.0.0.1', port: 8765, token: 'secret', name: 'laptop');
  final records = <String, Map<String, dynamic>>{};
  bool online = true;
  Future<http.Response>? delayed;
  final received = <Map<String, dynamic>>[];
  Map<String, dynamic> add(String id, String title) => records[id] = {
        'id': id,
        'title': title,
        'revision': 1,
        'ctx': 2048,
        'system_prompt': '',
        'model': 'model',
        'active_request': null,
        'messages': [
          {
            'id': 'old-user',
            'role': 'user',
            'content': 'Keep 日本語 on both devices',
            'interrupted': false
          }
        ]
      };
  late final httpClient = MockClient((request) async {
    expect(request.headers['Authorization'], 'Bearer secret');
    if (!online) return http.Response('{"error":"offline"}', 503);
    final body = request.body.isEmpty
        ? <String, dynamic>{}
        : jsonDecode(request.body) as Map<String, dynamic>;
    final parts =
        request.url.path.split('/').where((p) => p.isNotEmpty).toList();
    Object value;
    if (request.url.path == '/health') {
      return http.Response(
          jsonEncode({
            'ok': true,
            'backend': 'managed',
            'models': ['model']
          }),
          200);
    }
    if (request.url.path == '/v1/chat/completions') {
      received.add(body);
      final record = records[body['conversation_id']]!;
      expect(body['revision'], record['revision']);
      expect(body.containsKey('messages'), false);
      record['messages'] = [
        ...record['messages'] as List,
        {'role': 'user', 'content': body['content']},
        {'role': 'assistant', 'content': 'Shared reply', 'model': 'model'}
      ];
      record['revision'] = (record['revision'] as int) + 1;
      return http.Response(
          'data: {"choices":[{"delta":{"content":"Shared reply"}}]}\n\ndata: [DONE]\n\n',
          200);
    }
    if (parts.length == 1 && parts[0] == 'conversations') {
      if (request.method == 'POST') {
        value = {
          'conversation': add(
              'new-${records.length}', body['title'] as String? ?? 'New chat')
        };
      } else {
        final query = request.url.queryParameters['query'] ?? '';
        value = {
          'store_id': 'store',
          'conversations': records.values
              .where((r) =>
                  query.isEmpty ||
                  '${r['title']} ${r['messages']}'.contains(query))
              .map((r) => {
                    'id': r['id'],
                    'title': r['title'],
                    'revision': r['revision'],
                    'message_count': (r['messages'] as List).length,
                    'preview': 'Keep 日本語 on both devices',
                    'generating': false
                  })
              .toList()
        };
      }
    } else {
      final record = records[parts[1]];
      if (record == null) return http.Response('{"error":"not found"}', 404);
      if (parts.last == 'export') {
        value = {
          'filename': 'chat.md',
          'mime': 'text/markdown',
          'content': '# ${record['title']}\n\nKeep 日本語'
        };
      } else if (request.method == 'POST') {
        if (body['revision'] != record['revision']) {
          return http.Response('{"error":"changed on another device"}', 409);
        }
        record['title'] = body['title'];
        record['revision'] = (record['revision'] as int) + 1;
        value = {'conversation': record};
      } else {
        if (delayed != null && parts[1] == 'one') return delayed!;
        value = {'conversation': record};
      }
    }
    return http.Response(jsonEncode(value), 200,
        headers: {'content-type': 'application/json; charset=utf-8'});
  });
  NodeClient client() => NodeClient(pair, client: httpClient);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    GoogleFonts.config.allowRuntimeFetching = false;
    buildTheme();
    await GoogleFonts.pendingFonts();
  });
  tearDown(Get.reset);

  test(
      'two phone clients preserve revisions, search/export and offline cache without lost edits',
      () async {
    final server = WorkspaceServer()..add('one', 'Research');
    final storage = MemoryStorage();
    final a = ConversationLibrary(storage)..attach(server.client());
    final b = ConversationLibrary(MemoryStorage())..attach(server.client());
    addTearDown(a.dispose);
    addTearDown(b.dispose);
    await a.refresh();
    await b.refresh();
    expect(a.error, isNull);
    expect(b.error, isNull);
    expect(await a.rename('Named notes'), true);
    expect(await b.rename('Stale title'), false);
    expect(b.error, contains('changed on another device'));
    await a.refresh();
    await b.refresh();
    expect(b.current!.title, 'Named notes');
    b.search('日本語');
    await b.refresh();
    expect(b.conversations.single.id, 'one');
    expect((await b.export('markdown'))['content'], contains('日本語'));
    server.online = false;
    await b.refresh();
    expect(b.online, false);
    expect(b.current!.title, 'Named notes');
    final restored = ConversationLibrary(storage)..attach(server.client());
    addTearDown(restored.dispose);
    expect(restored.current!.title, 'Named notes');
    expect(restored.online, false);
    final different = NodeClient(
        const PairPayload(
            host: '10.0.0.2', port: 8765, token: 'other', name: 'other'),
        client: server.httpClient);
    restored.attach(different);
    expect(restored.current, isNull);
  });

  test('late refresh cannot switch back after selecting another conversation',
      () async {
    final server = WorkspaceServer()
      ..add('one', 'First')
      ..add('two', 'Second');
    final library = ConversationLibrary(MemoryStorage())
      ..attach(server.client());
    addTearDown(library.dispose);
    await library.refresh();
    final gate = Completer<http.Response>();
    server.delayed = gate.future;
    final pending = library.refresh();
    await Future<void>.delayed(Duration.zero);
    expect(await library.open('two'), true);
    gate.complete(http.Response(
        jsonEncode({'conversation': server.records['one']}), 200,
        headers: {'content-type': 'application/json; charset=utf-8'}));
    await pending;
    expect(library.current!.id, 'two');
  });

  test(
      'chat uses coordinator conversation id and revision and restores completed messages',
      () async {
    final server = WorkspaceServer()..add('one', 'Shared');
    final storage = MemoryStorage();
    final chat = Get.put(ChatController(storage,
        backendFactory: () => LaptopNodeBackend(
            baseUrl: WorkspaceServer.pair.baseUrl,
            token: 'secret',
            client: server.httpClient)));
    chat.shared.attach(server.client());
    await chat.shared.refresh();
    await chat.send('Continue from the laptop');
    expect(server.received.single['conversation_id'], 'one');
    expect(server.received.single['request_id'], hasLength(32));
    expect(chat.messages.last.content, 'Shared reply');
    expect(chat.isGenerating.value, false);
    await chat.clearChat();
    expect(server.records['one']!['messages'], hasLength(3));
    expect(chat.shared.current!.id, isNot('one'));
  });

  for (final size in const [
    Size(360, 800),
    Size(412, 915),
    Size(768, 1024),
    Size(1280, 800)
  ]) {
    testWidgets(
        'conversation library at $size with scaled text',
        (tester) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final server = WorkspaceServer()
        ..add('one',
            'A long conversation title with 日本語 and notes to continue on the phone');
      final chat = Get.put(ChatController(MemoryStorage(),
          backendFactory: () => FakeBackend()));
      chat.shared.attach(server.client());
      await chat.shared.refresh();
      Widget host(Widget child) => MaterialApp(
          theme: buildTheme(),
          home: MediaQuery(
              data: MediaQueryData(
                  size: size,
                  textScaler: const TextScaler.linear(1.3),
                  disableAnimations: true),
              child: child));
      await tester.pumpWidget(host(ConversationsPage(chat: chat)));
      await tester.pumpAndSettle();
      expect(find.text('Search names and messages'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.enterText(find.byType(TextField), '日本語');
      await tester.pump(const Duration(milliseconds: 400));
      await tester.pumpAndSettle();
      expect(chat.shared.conversations, hasLength(1));
    });
  }
}
