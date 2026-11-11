import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:privatelm_lan/models/device_profile.dart';
import 'package:privatelm_lan/models/pairing.dart';
import 'package:privatelm_lan/services/node_client.dart';
import 'package:privatelm_lan/ui/pool_screen.dart';

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
}
