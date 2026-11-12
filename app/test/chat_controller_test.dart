import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:get/get.dart';
import 'package:localhostai/controllers/chat_controller.dart';
import 'package:localhostai/features/connection/connection_controller.dart';
import 'package:localhostai/models/pairing.dart';
import 'package:localhostai/services/phone_worker_service.dart';
import 'package:localhostai/services/storage_service.dart';
import 'package:localhostai/ui/settings_screen.dart';

import 'app_fixture.dart';
import 'support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    Get.testMode = true;
  });
  tearDown(() {
    Get.reset();
  });

  test('restores conversation and the last selected backend', () {
    final storage = MemoryStorage()
      ..settings['active_backend'] = 'anthropic'
      ..history = [
        {'role': 'user', 'content': 'Remember this'}
      ];
    final controller = Get.put(
        ChatController(storage, backendFactory: (id) => FakeBackend(id: id)));
    expect(controller.activeBackendId.value, 'anthropic');
    expect(controller.messages.single.content, 'Remember this');
  });

  test('clearing a chat cancels an in-flight reply without repopulating it',
      () async {
    final ready = Completer<void>();
    final old = FakeBackend(ready: ready.future);
    final storage = MemoryStorage();
    final controller =
        Get.put(ChatController(storage, backendFactory: (_) => old));
    final sending = controller.send('old prompt');
    await Future<void>.delayed(Duration.zero);
    await controller.clearChat();
    ready.complete();
    await sending;
    expect(old.disposed, isTrue);
    expect(controller.messages, isEmpty);
    expect(storage.history, isEmpty);
    expect(controller.error.value, isNull);
  });

  test('a cancelled request cannot clear a newer generation state', () async {
    final firstReady = Completer<void>();
    final secondReady = Completer<void>();
    var builds = 0;
    final controller = Get.put(ChatController(MemoryStorage(),
        backendFactory: (_) => ++builds == 1
            ? FakeBackend(ready: firstReady.future)
            : FakeBackend(ready: secondReady.future, reply: 'new reply')));
    final first = controller.send('first');
    await controller.clearChat();
    final second = controller.send('second');
    firstReady.complete();
    await first;
    expect(controller.isGenerating.value, isTrue);
    secondReady.complete();
    await second;
    expect(controller.messages.last.content, 'new reply');
    expect(controller.isGenerating.value, isFalse);
  });

  test('interrupted streams preserve visible partial assistant text', () async {
    final backend = FakeBackend(reply: 'partial answer', failAfterReply: true);
    final storage = MemoryStorage();
    final controller =
        Get.put(ChatController(storage, backendFactory: (_) => backend));
    await controller.send('question');
    expect(controller.messages.last.content, 'partial answer');
    expect(controller.messages.last.interrupted, isTrue);
    expect(storage.history.last['interrupted'], isTrue);
    expect(controller.error.value, contains('transport interrupted'));
  });

  test('laptop history is conservatively bounded by selected context',
      () async {
    final backend = FakeBackend(id: 'laptop');
    final storage = MemoryStorage()
      ..settings['active_backend'] = 'laptop'
      ..settings['context_tokens'] = '1024'
      ..history = List.generate(
          20,
          (index) => {
                'role': index.isEven ? 'user' : 'assistant',
                'content': 'x' * 600
              });
    final controller =
        Get.put(ChatController(storage, backendFactory: (_) => backend));
    await controller.send('latest');
    expect(backend.received.last.content, 'latest');
    expect(
        backend.received
            .fold<int>(0, (sum, message) => sum + message.content.length),
        lessThanOrEqualTo(2304));
  });

  test('a single message cannot bypass the selected context budget', () async {
    final storage = MemoryStorage()
      ..settings['active_backend'] = 'laptop'
      ..settings['context_tokens'] = '1024';
    final controller = Get.put(ChatController(storage,
        backendFactory: (_) => FakeBackend(id: 'laptop')));
    await controller.send('x' * 2000);
    expect(controller.messages, isEmpty);
    expect(controller.error.value, contains('too large'));
  });

  test('selecting another node clears the old pairing credentials', () async {
    final storage = MemoryStorage();
    await storage.savePair(const PairPayload(
        host: '10.0.0.1', port: 8765, token: 'secret', name: 'one'));
    await storage.selectNode('http://10.0.0.2:8765/');
    expect(storage.loadPair(), isNull);
    expect(storage.getString('pair_token'), isEmpty);
    expect(storage.getString('laptop_node_url'), 'http://10.0.0.2:8765');
  });

  testWidgets('settings persist while typing and invalidate the cached backend',
      (tester) async {
    final storage = MemoryStorage()..settings['active_backend'] = 'cloud';
    Get.put<StorageService>(storage);
    final worker = Get.put(PhoneWorkerService());
    final controller = Get.put(
        ChatController(storage, backendFactory: (id) => FakeBackend(id: id)));
    final old = controller.backend as FakeBackend;
    await tester.pumpWidget(ProviderScope(
      overrides: [
        storageProvider.overrideWithValue(storage),
        phoneWorkerProvider.overrideWithValue(worker),
      ],
      child: const GetMaterialApp(home: SettingsScreen()),
    ));
    final field = find
        .byWidgetPredicate(
            (w) => w is TextField && w.decoration?.labelText == 'API key')
        .first;
    await tester.ensureVisible(field);
    await tester.enterText(field, 'updated-key');
    await tester.pump();
    expect(storage.getString('cloud_key'), 'updated-key');
    expect(old.disposed, isTrue);
    expect(identical(controller.backend, old), isFalse);
  });

  testWidgets('settings refresh paired coordinator details after re-pairing',
      (tester) async {
    final fixture = AppFixture();
    final node = await fixture.createController();
    final storage = fixture.storage;
    Get.put<StorageService>(storage);
    Get.put<PhoneWorkerService>(fixture.worker);
    Get.put(
        ChatController(storage, backendFactory: (id) => FakeBackend(id: id)));
    await tester.pumpWidget(ProviderScope(
      overrides: [
        storageProvider.overrideWithValue(storage),
        phoneWorkerProvider.overrideWithValue(fixture.worker),
        connectionProvider.overrideWith((ref) => node),
      ],
      child: const GetMaterialApp(home: SettingsScreen()),
    ));
    await tester.pumpAndSettle();
    expect(find.text('http://10.0.0.1:8765'), findsWidgets);
    await node.connect(const PairPayload(
      host: '10.0.0.2',
      port: 8765,
      token: 'NewToken',
      name: 'New laptop',
    ));
    await tester.pumpAndSettle();
    expect(find.text('http://10.0.0.2:8765'), findsWidgets);
    expect(find.text('New laptop'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
    node.dispose();
  });
}
