import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:privatelm_lan/models/device_profile.dart';
import 'package:privatelm_lan/models/pairing.dart';
import 'package:privatelm_lan/services/node_client.dart';

const pair =
    PairPayload(host: 'laptop', port: 8765, token: 't', name: 'laptop');

DeviceProfile profile(int available) => DeviceProfile(
      id: 'phone',
      name: 'phone',
      kind: 'phone',
      ramMb: 8000,
      availableMb: available,
      budgetMb: available - 900,
      cores: 8,
      threads: 4,
      arch: 'arm64',
    );

void main() {
  testWidgets('heartbeats use current RAM and the current network address',
      (tester) async {
    var available = 4000;
    String? address = '10.0.0.2:50053';
    final reports = <Map<String, dynamic>>[];
    final node = NodeClient(
      pair,
      refreshProfile: (_) => profile(available),
      client: MockClient((request) async {
        reports.add(jsonDecode(request.body) as Map<String, dynamic>);
        return http.Response('{"ok":true}', 200);
      }),
    );
    addTearDown(node.dispose);
    node.startHeartbeat(profile(7000), rpcProvider: () async => address);
    await tester.pump(const Duration(seconds: 10));
    available = 2500;
    address = '192.168.1.4:50053';
    await tester.pump(const Duration(seconds: 10));
    await tester.pump(const Duration(seconds: 10));
    address = null;
    await tester.pump(const Duration(seconds: 10));
    await node.stopHeartbeat();
    expect(reports.map((r) => r['available_mb']), [4000, 2500, 2500, 2500]);
    expect(reports[1]['rpc'], '192.168.1.4:50053');
    expect(reports.last.containsKey('rpc'), isFalse);
    expect(reports.every((r) => r['id'] == 'phone'), isTrue);
  });

  testWidgets(
      'stopped heartbeat cannot re-register after address lookup completes',
      (tester) async {
    final address = Completer<String?>();
    var requests = 0;
    final node = NodeClient(pair, client: MockClient((_) async {
      requests++;
      return http.Response('{"ok":true}', 200);
    }));
    addTearDown(node.dispose);
    node.startHeartbeat(profile(4000), rpcProvider: () => address.future);
    await tester.pump(const Duration(seconds: 10));
    final stopping = node.stopHeartbeat();
    address.complete('10.0.0.2:50052');
    await tester.pump();
    await stopping;
    expect(requests, 0);
  });

  testWidgets(
      'slow heartbeats do not overlap and stopping waits for the last request',
      (tester) async {
    final response = Completer<http.Response>();
    var requests = 0;
    final node = NodeClient(pair, client: MockClient((_) {
      requests++;
      return response.future;
    }));
    addTearDown(node.dispose);
    node.startHeartbeat(profile(4000), rpc: '10.0.0.2:50052');
    await tester.pump(const Duration(seconds: 10));
    var stopped = false;
    final stopping = node.stopHeartbeat().then((_) => stopped = true);
    await tester.pump(const Duration(seconds: 1));
    expect(requests, 1);
    expect(stopped, isFalse);
    response.complete(http.Response('{"ok":true}', 200));
    await tester.pump();
    await stopping;
    expect(stopped, isTrue);
    await tester.pump(const Duration(seconds: 30));
    expect(requests, 1);
  });

  testWidgets(
      'worker reports are sequenced through registration, heartbeat, and withdrawal',
      (tester) async {
    final reports = <Map>[];
    final node = NodeClient(pair, client: MockClient((request) async {
      reports.add(jsonDecode(request.body) as Map);
      return http.Response('{"ok":true}', 200);
    }));
    addTearDown(node.dispose);
    final worker = {'session': 'run-one', 'state': 'running'};
    await node.reportSelf(profile(4000), rpc: '10.0.0.2:50054', worker: worker);
    node.startHeartbeat(profile(4000),
        rpc: '10.0.0.2:50054', workerReport: () => worker);
    await tester.pump(const Duration(seconds: 10));
    await node.stopHeartbeat();
    await node.reportSelf(profile(4000),
        worker: {'session': 'run-one', 'state': 'stopped'});
    expect(reports.map((report) => report['worker']['sequence']), [1, 2, 3]);
    expect(reports.last.containsKey('rpc'), isFalse);
  });
}
