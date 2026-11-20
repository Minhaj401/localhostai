import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:localhostai/models/device_profile.dart';
import 'package:localhostai/models/pairing.dart';
import 'package:localhostai/services/node_client.dart';
import 'package:localhostai/ui/pool_screen.dart';

void main() {
  testWidgets(
      'refreshing a changed model inventory repairs the picker selection',
      (tester) async {
    var models = ['old'];
    final node = NodeClient(
      const PairPayload(
          host: 'laptop', port: 8765, token: 'test', name: 'laptop'),
      client: MockClient((request) async {
        if (request.url.path == '/models') {
          return http.Response(
              jsonEncode({
                'models': models.map((m) => {'name': m}).toList()
              }),
              200);
        }
        if (request.url.path == '/peers') {
          return http.Response('{"devices":[]}', 200);
        }
        return http.Response('{"state":"stopped"}', 200);
      }),
    );
    addTearDown(node.dispose);
    await tester.pumpWidget(MaterialApp(
        home: PoolScreen(
            client: node, profile: DeviceProfile.detect(name: 'phone'))));
    await tester.pumpAndSettle();
    expect(find.text('old'), findsOneWidget);
    models = ['new', 'new'];
    await tester.tap(find.byTooltip('Refresh'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.text('new'), findsOneWidget);
  });

  testWidgets('compute pool uses the selected context for plan and start',
      (tester) async {
    int? plannedContext;
    int? startedContext;
    final node = NodeClient(
      const PairPayload(
          host: 'laptop', port: 8765, token: 'test', name: 'laptop'),
      client: MockClient((request) async {
        if (request.url.path == '/models') {
          return http.Response('{"models":[{"name":"model"}]}', 200);
        }
        if (request.url.path == '/peers') {
          return http.Response('{"devices":[]}', 200);
        }
        if (request.url.path == '/cluster/plan') {
          plannedContext = (jsonDecode(request.body) as Map)['ctx'] as int;
          return http.Response(
              jsonEncode({
                'plan': {
                  'model': 'model',
                  'ctx': plannedContext,
                  'total_layers': 24,
                  'over_budget': false,
                  'shortfall_mb': 0,
                  'summary': 'Fits',
                  'assignments': [],
                }
              }),
              200);
        }
        if (request.url.path == '/cluster/start') {
          startedContext = (jsonDecode(request.body) as Map)['ctx'] as int;
          return http.Response(
              jsonEncode({
                'ok': true,
                'plan': {
                  'model': 'model',
                  'ctx': startedContext,
                  'total_layers': 24,
                  'over_budget': false,
                  'shortfall_mb': 0,
                  'summary': 'Fits',
                  'assignments': [],
                }
              }),
              200);
        }
        return http.Response('{"state":"stopped"}', 200);
      }),
    );
    addTearDown(node.dispose);
    await tester.pumpWidget(MaterialApp(
      home: PoolScreen(
        client: node,
        profile: DeviceProfile.detect(name: 'phone'),
        contextTokens: 8192,
      ),
    ));
    await tester.pumpAndSettle();
    final previewButton = tester.widget<OutlinedButton>(find.ancestor(
      of: find.text('Preview split'),
      matching: find.byType(OutlinedButton),
    ));
    previewButton.onPressed!();
    await tester.pumpAndSettle();
    final loadButton = tester.widget<FilledButton>(find.ancestor(
      of: find.text('Load model'),
      matching: find.byType(FilledButton),
    ));
    loadButton.onPressed!();
    await tester.pumpAndSettle();
    expect(plannedContext, 8192);
    expect(startedContext, 8192);
    expect(tester.takeException(), isNull);
  });

  testWidgets(
      'memory reserves adjust the laptop over the API and the phone locally',
      (tester) async {
    int? postedReserve;
    int? phoneReserve;
    const laptop = {
      'reserve_mb': 1400,
      'effective_reserve_mb': 1400,
      'remote_headroom_mb': 256,
      'default_reserve_mb': 1400,
      'total_mb': 7720,
      'available_mb': 3000,
      'budget_mb': 6320,
    };
    final node = NodeClient(
      const PairPayload(
          host: 'laptop', port: 8765, token: 'test', name: 'laptop'),
      client: MockClient((request) async {
        if (request.url.path == '/config/memory') {
          if (request.method == 'POST') {
            postedReserve =
                (jsonDecode(request.body) as Map)['reserve_mb'] as int?;
            final reserve = postedReserve ?? 1400;
            return http.Response(
                jsonEncode({
                  ...laptop,
                  'reserve_mb': reserve,
                  'effective_reserve_mb': reserve,
                  'budget_mb': 7720 - reserve,
                }),
                200);
          }
          return http.Response(jsonEncode(laptop), 200);
        }
        if (request.url.path == '/models') {
          return http.Response('{"models":[]}', 200);
        }
        return http.Response('{"state":"stopped"}', 200);
      }),
    );
    addTearDown(node.dispose);
    await tester.pumpWidget(MaterialApp(
      home: PoolScreen(
        client: node,
        profile: DeviceProfile.detect(name: 'phone'),
        phoneReserveMb: 900,
        onPhoneReserveChanged: (mb) => phoneReserve = mb,
      ),
    ));
    await tester.pumpAndSettle();
    expect(find.text('Memory reserves'), findsOneWidget);
    expect(find.text('Laptop reserve'), findsOneWidget);
    expect(find.text('Phone reserve'), findsOneWidget);
    expect(find.textContaining('offers 1.6 GB of 7.5 GB to the pool'),
        findsOneWidget);

    // Raising the laptop reserve posts the new value to the coordinator.
    await tester.drag(find.byType(Slider).first, const Offset(160, 0));
    await tester.pumpAndSettle();
    expect(postedReserve, isNotNull);
    expect(postedReserve, greaterThan(1400));

    // Lowering the phone reserve persists locally through the controller.
    await tester.drag(find.byType(Slider).last, const Offset(-160, 0));
    await tester.pumpAndSettle();
    expect(phoneReserve, isNotNull);
    expect(phoneReserve, lessThan(900));
    expect(tester.takeException(), isNull);
  });

  testWidgets(
      'an older laptop without the memory endpoint still offers the phone reserve',
      (tester) async {
    final node = NodeClient(
      const PairPayload(
          host: 'laptop', port: 8765, token: 'test', name: 'laptop'),
      client: MockClient((request) async {
        if (request.url.path == '/config/memory') {
          return http.Response('{"error":"unknown endpoint"}', 404);
        }
        if (request.url.path == '/models') {
          return http.Response('{"models":[]}', 200);
        }
        return http.Response('{"state":"stopped"}', 200);
      }),
    );
    addTearDown(node.dispose);
    await tester.pumpWidget(MaterialApp(
      home: PoolScreen(
        client: node,
        profile: DeviceProfile.detect(name: 'phone'),
        phoneReserveMb: 900,
        onPhoneReserveChanged: (mb) {},
      ),
    ));
    await tester.pumpAndSettle();
    expect(find.text('Laptop reserve'), findsNothing);
    expect(find.text('Phone reserve'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
