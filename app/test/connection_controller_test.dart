import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import 'app_fixture.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
      'users can discover and download a model outside the starter catalog, then load it',
      () async {
    final fixture = AppFixture();
    final node = await fixture.createController();
    addTearDown(node.dispose);
    final search = await node.client!.browseModels('a model I choose');
    expect(search.repositories, ['example/new-model']);
    final files = await node.client!.browseModels(search.repositories.single);
    fixture.requests.clear();
    expect(await node.download(files.models.single), isTrue);
    final request = fixture.requests.singleWhere((r) => r.method == 'POST');
    expect(request.url.path, '/managed/download');
    expect(request.headers['Authorization'], 'Bearer MixedCaseToken');
    expect((jsonDecode(request.body) as Map)['source'],
        AppFixture.customModel['source']);
    final custom = node.models.singleWhere((m) => m.id == AppFixture.customId);
    expect(custom.installed, isTrue);
    expect(custom.custom, isTrue);
    expect(custom.quantization, 'Q4_K_M');
    expect(node.selectedModel, custom.id);
    fixture.requests.clear();
    await node.install(custom, load: true);
    expect(node.operationError, isNull);
    expect(
        fixture.requests
            .where((r) => r.method == 'POST')
            .map((r) => r.url.path),
        ['/cluster/plan', '/managed/install', '/cluster/start']);
  });

  test(
      'partial custom downloads can resume but cannot be planned as installed models',
      () async {
    final fixture = AppFixture()..customAvailable = true;
    final node = await fixture.createController();
    addTearDown(node.dispose);
    final custom = node.models.singleWhere((m) => m.id == AppFixture.customId);
    expect(custom.canPreview, isFalse);
    fixture.requests.clear();
    expect(await node.preview(custom.id), isNull);
    expect(fixture.requests, isEmpty);
    await node.install(custom);
    expect(
        fixture.requests
            .where((r) => r.method == 'POST')
            .map((r) => r.url.path),
        ['/managed/install']);
    expect(node.models.singleWhere((m) => m.id == custom.id).installed, isTrue);
  });

  test('custom download failure retains the library and an actionable error',
      () async {
    final fixture = AppFixture();
    final node = await fixture.createController();
    addTearDown(node.dispose);
    final files = await node.client!.browseModels('example/new-model');
    fixture.intercept = (request) async =>
        request.url.path == '/managed/download'
            ? http.Response(
                '{"ok":false,"error":"download checksum mismatch"}', 400)
            : fixture.respond(request);
    expect(await node.download(files.models.single), isFalse);
    expect(node.operationError, contains('checksum mismatch'));
    expect(node.operationBusy, isFalse);
    expect(node.models.any((m) => m.id == AppFixture.customId), isFalse);
  });

  test('loading previews before downloading and preserves context in start',
      () async {
    final fixture = AppFixture()..installed = false;
    final node = await fixture.createController();
    addTearDown(node.dispose);
    node.selectContext(2048);
    fixture.requests.clear();
    await node.install(node.models.first, load: true);
    final controls =
        fixture.requests.where((request) => request.method == 'POST').toList();
    expect(controls.map((request) => request.url.path).toList(),
        ['/cluster/plan', '/managed/install', '/cluster/start']);
    expect((jsonDecode(controls.first.body) as Map)['ctx'], 2048);
    expect((jsonDecode(controls.last.body) as Map)['ctx'], 2048);
    expect(controls.first.headers['Authorization'], 'Bearer MixedCaseToken');
    expect(node.operationBusy, isFalse);
    expect(node.error, isNull);
    expect(
        fixture.requests.any((request) => request.url.path == '/cluster/log'),
        isFalse);
  });

  test('an oversized preview never triggers a download or start', () async {
    final fixture = AppFixture()..fits = false;
    final node = await fixture.createController();
    addTearDown(node.dispose);
    fixture.requests.clear();
    await node.install(node.models.last, load: true);
    expect(node.error, contains('500 MB short'));
    expect(
        fixture.requests
            .where((request) => request.method == 'POST')
            .map((request) => request.url.path)
            .toList(),
        ['/cluster/plan']);
    await node.refresh();
    expect(node.operationError, contains('500 MB short'));
  });

  test('download-only uses the existing installer without starting inference',
      () async {
    final fixture = AppFixture()..installed = false;
    final node = await fixture.createController();
    addTearDown(node.dispose);
    fixture.requests.clear();
    await node.install(node.models.first);
    expect(
        fixture.requests
            .where((request) => request.method == 'POST')
            .map((request) => request.url.path)
            .toList(),
        ['/managed/install']);
    expect(node.models.first.installed, isTrue);
  });

  test('offline refresh keeps the last known device and model snapshot',
      () async {
    final fixture = AppFixture();
    final node = await fixture.createController();
    addTearDown(node.dispose);
    final models = node.models;
    final devices = node.devices;
    fixture.online = false;
    await node.refresh();
    expect(node.connected, isFalse);
    expect(node.error, isNotNull);
    expect(node.models, same(models));
    expect(node.devices, same(devices));
  });

  test('a delayed preview cannot replace a newer context selection', () async {
    final fixture = AppFixture();
    final node = await fixture.createController();
    addTearDown(node.dispose);
    final delayed = Completer<http.Response>();
    http.Request? oldRequest;
    fixture.intercept = (request) async {
      if (request.url.path == '/cluster/plan') {
        oldRequest = request;
        return delayed.future;
      }
      return fixture.respond(request);
    };
    node.selectModel(AppFixture.modelId);
    final preview = node.preview(AppFixture.modelId);
    await Future<void>.delayed(Duration.zero);
    node.selectContext(8192);
    delayed.complete(
        http.Response(jsonEncode({'plan': fixture.plan(oldRequest!)}), 200));
    await preview;
    expect(node.lastPlan, isNull);
  });

  test(
      'phone worker failure remains visible across successful coordinator polling',
      () async {
    final fixture = AppFixture();
    final node = await fixture.createController();
    addTearDown(node.dispose);
    fixture.worker.lastFailure = 'Phone worker exited (-9). SIGKILL';
    await node.refresh();
    expect(node.connected, isTrue);
    expect(node.error, contains('SIGKILL'));
    await node.refresh();
    expect(node.error, contains('SIGKILL'));
  });

  test('a stopped load cannot overwrite the newer stop operation', () async {
    final fixture = AppFixture();
    final node = await fixture.createController();
    addTearDown(node.dispose);
    final delayed = Completer<http.Response>();
    final started = Completer<void>();
    fixture.intercept = (request) async {
      if (request.url.path == '/cluster/start') {
        started.complete();
        return delayed.future;
      }
      return fixture.respond(request);
    };
    final load = node.install(node.models.first, load: true);
    await started.future;
    await node.stopModel();
    delayed.complete(http.Response('{"ok":true}', 200));
    await load;
    expect(node.operationMessage, 'Model stopped.');
    expect(node.clusterState, 'stopped');
    expect(node.operationBusy, isFalse);
  });

  test('disconnect during a download clears busy state and permits reconnect',
      () async {
    final fixture = AppFixture();
    final node = await fixture.createController();
    addTearDown(node.dispose);
    final files = await node.client!.browseModels('example/new-model');
    final delayed = Completer<http.Response>();
    fixture.intercept = (request) async =>
        request.url.path == '/managed/download'
            ? delayed.future
            : fixture.respond(request);
    final download = node.download(files.models.single);
    await Future<void>.delayed(Duration.zero);
    expect(node.operationBusy, isTrue);
    await node.disconnect();
    expect(node.operationBusy, isFalse);
    expect(node.activeOperationModel, isNull);
    fixture.intercept = null;
    expect(await node.connect(AppFixture.pair), isTrue);
    delayed.complete(http.Response(
        '{"ok":true,"model":{"name":"${AppFixture.customId}"}}', 200));
    expect(await download, isFalse);
    expect(node.operationBusy, isFalse);
  });
}
