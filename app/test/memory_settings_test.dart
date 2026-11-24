import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import 'package:localhostai/features/connection/connection_controller.dart';
import 'package:localhostai/features/settings/memory_settings.dart';

import 'app_fixture.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  Future<ConnectionController> pumpMemorySettings(WidgetTester tester,
      {required AppFixture fixture}) async {
    final controller = await fixture.createController();
    addTearDown(controller.dispose);
    await tester.pumpWidget(ProviderScope(
      overrides: [
        connectionProvider.overrideWith((ref) => controller),
      ],
      child: const MaterialApp(home: Scaffold(body: MemorySettings())),
    ));
    await tester.pumpAndSettle();
    return controller;
  }

  testWidgets(
      'memory reserves adjust the laptop over the API and the phone locally',
      (tester) async {
    int? postedReserve;
    const laptop = {
      'reserve_mb': 1400,
      'effective_reserve_mb': 1400,
      'remote_headroom_mb': 256,
      'default_reserve_mb': 1400,
      'total_mb': 7720,
      'available_mb': 3000,
      'budget_mb': 6320,
    };
    final fixture = AppFixture()
      ..intercept = (request) async {
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
        return http.Response('{"state":"stopped"}', 200);
      };

    final controller = await pumpMemorySettings(tester, fixture: fixture);
    expect(find.text('Laptop reserve'), findsOneWidget);
    expect(find.text('Phone reserve'), findsOneWidget);
    expect(find.textContaining('offers 1.6 GB of 7.5 GB to the pool'),
        findsOneWidget);

    // Raising the laptop reserve posts the new value to the coordinator.
    // drags start at the slider's center, so push far enough to clamp.
    await tester.drag(find.byType(Slider).first, const Offset(400, 0));
    await tester.pumpAndSettle();
    expect(postedReserve, isNotNull);
    expect(postedReserve, greaterThan(1400));

    // Lowering the phone reserve persists locally through the controller.
    await tester.drag(find.byType(Slider).last, const Offset(-400, 0));
    await tester.pumpAndSettle();
    expect(controller.phoneReserveMb, lessThan(900));
    expect(tester.takeException(), isNull);
  });

  testWidgets(
      'an older laptop without the memory endpoint still offers the phone reserve',
      (tester) async {
    final fixture = AppFixture()
      ..intercept = (request) async {
        if (request.url.path == '/config/memory') {
          return http.Response('{"error":"unknown endpoint"}', 404);
        }
        return http.Response('{"state":"stopped"}', 200);
      };

    await pumpMemorySettings(tester, fixture: fixture);
    expect(find.text('Laptop reserve'), findsNothing);
    expect(find.text('Phone reserve'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
