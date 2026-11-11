import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:privatelm_lan/models/chat_message.dart';
import 'package:privatelm_lan/models/pairing.dart';
import 'package:privatelm_lan/services/cloud_service.dart';
import 'package:privatelm_lan/services/laptop_node_service.dart';
import 'package:privatelm_lan/services/llm_backend.dart';
import 'package:privatelm_lan/services/node_client.dart';

void main() {
  test('a disconnected chat stream is not a completed answer', () async {
    final response = http.StreamedResponse(
      Stream.value(utf8.encode(
          'data: {"choices":[{"delta":{"content":"unfinished"}}]}\n\n')),
      200,
    );
    await expectLater(parseOpenAiStream(response).toList(), throwsStateError);
  });
  test('SSE preserves split Unicode, multiline data, CRLF, and the last event',
      () async {
    final body = utf8.encode(
        'data: {"choices":\r\ndata: [{"delta":{"content":"你好🙂"}}]}\r\n\r\ndata: {"choices":[{"delta":{"content":"last"},"finish_reason":"stop"}]}');
    final response =
        http.StreamedResponse(Stream.fromIterable(body.map((b) => [b])), 200);
    final chunks = await parseOpenAiStream(response).toList();
    expect(chunks.map((c) => c.delta).join(), '你好🙂last');
    expect(chunks.where((c) => c.done).length, 1);
  });

  test(
      'stream errors fail the request instead of becoming saved assistant text',
      () async {
    final response = http.StreamedResponse(
        Stream.value(
            utf8.encode('data: {"error":{"message":"model crashed"}}\n\n')),
        200);
    await expectLater(parseOpenAiStream(response).toList(), throwsStateError);
    final backend = AnthropicBackend(
        apiKey: 'test',
        client: MockClient((_) async => http.Response(
            'data: {"type":"error","error":{"message":"overloaded"}}\n\n',
            200)));
    await expectLater(backend.chat([]).toList(), throwsStateError);
    await backend.dispose();
  });

  test('laptop requests authenticate and follow a changed active model',
      () async {
    var model = 'old';
    final client = MockClient((request) async {
      expect(request.headers['Authorization'], 'Bearer SecretCase');
      if (request.url.path == '/health') {
        return http.Response(
            jsonEncode({
              'ok': true,
              'models': [model]
            }),
            200);
      }
      expect(request.url.path, '/v1/chat/completions');
      expect((jsonDecode(request.body) as Map)['model'], 'new');
      return http.Response(
          'data: {"choices":[{"delta":{"content":"ok"}}]}\n\ndata: [DONE]\n\n',
          200);
    });
    final backend = LaptopNodeBackend(
        baseUrl: 'http://laptop:8765/', token: 'SecretCase', client: client);
    await backend.ensureReady();
    expect(await backend.currentModel(), 'old');
    model = 'new';
    await backend.ensureReady();
    expect(await backend.currentModel(), 'new');
    final response =
        await backend.chat([ChatMessage(role: 'user', content: 'hi')]).toList();
    expect(response.map((c) => c.delta).join(), 'ok');
    await backend.dispose();
  });

  test('node client verifies pairing and respects failed stop responses',
      () async {
    const pair = PairPayload(
        host: 'laptop', port: 8765, token: 'SecretCase', name: 'laptop');
    final node = NodeClient(pair, client: MockClient((request) async {
      expect(request.headers['Authorization'], 'Bearer SecretCase');
      if (request.url.path == '/pair/verify') {
        return http.Response('{"ok":true}', 200);
      }
      if (request.url.path == '/cluster/stop') {
        return http.Response('{"ok":false}', 200);
      }
      return http.Response('{"error":{"message":"pair token required"}}', 401);
    }));
    expect(await node.verifyPairing(), isTrue);
    expect(await node.stopCluster(), isFalse);
    expect((await node.startCluster('test')).error, 'pair token required');
    node.dispose();
  });

  test('missing plan fields and unusable devices cannot promise memory', () {
    expect(SplitPlan.fromJson({}).fits, isFalse);
    expect(PoolDevice.fromJson({'self': true, 'budget_mb': 0}).usable, isFalse);
    expect(
        PoolDevice.fromJson(
            {'reachable': true, 'budget_mb': 8000, 'usable': false}).usable,
        isFalse);
    expect(
        PoolDevice.fromJson({
          'reachable': true,
          'in_use': true,
          'usable': true,
          'budget_mb': 0,
        }).usable,
        isTrue,
        reason:
            'a busy device is not offline just because its memory is allocated');
    expect(
        PoolDevice.fromJson({
          'reachable': true,
          'in_use': true,
          'usable': true,
          'enabled': false,
          'budget_mb': 0,
        }).usable,
        isFalse);
  });
}
