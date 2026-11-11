import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:http/http.dart' as http;
import 'package:privatelm_lan/features/connection/connection_controller.dart';
import 'package:privatelm_lan/features/models/model_browser_page.dart';
import 'package:privatelm_lan/features/models/models_page.dart';
import 'package:privatelm_lan/theme/theme.dart';

import 'app_fixture.dart';

Future<void> mount(WidgetTester tester, ConnectionController node,
    {String location = '/models'}) async {
  final router = GoRouter(initialLocation: location, routes: [
    GoRoute(path: '/models', builder: (_, __) => const ModelsPage()),
    GoRoute(
        path: '/models/browse', builder: (_, __) => const ModelBrowserPage()),
  ]);
  addTearDown(router.dispose);
  await tester.pumpWidget(ProviderScope(
    overrides: [connectionProvider.overrideWith((ref) => node)],
    child: MaterialApp.router(
      theme: buildTheme(),
      routerConfig: router,
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(context).copyWith(
            disableAnimations: true, textScaler: const TextScaler.linear(1.3)),
        child: child!,
      ),
    ),
  ));
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(() async {
    GoogleFonts.config.allowRuntimeFetching = false;
    buildTheme();
    await GoogleFonts.pendingFonts();
  });

  for (final size in const [
    Size(360, 800),
    Size(412, 915),
    Size(768, 1024),
    Size(1280, 800)
  ]) {
    testWidgets('discover and download a custom GGUF at $size with scaled text',
        (tester) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final fixture = AppFixture();
      final node = (await tester.runAsync(fixture.createController))!;
      await mount(tester, node);
      await tester.tap(find.text('Find and download models'));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('model-hub-query')), 'community');
      await tester.ensureVisible(find.text('Search models'));
      await tester.tap(find.text('Search models'));
      await tester.pumpAndSettle();
      final browserScroll = find
          .descendant(
              of: find.byType(ModelBrowserPage),
              matching: find.byType(Scrollable))
          .first;
      await tester.scrollUntilVisible(find.text('Choose GGUF'), 200,
          scrollable: browserScroll);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Choose GGUF'));
      await tester.pumpAndSettle();
      await tester.scrollUntilVisible(find.text('Download GGUF'), 200,
          scrollable: browserScroll);
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      await tester.tap(find.text('Download GGUF'));
      await tester.pumpAndSettle();
      expect(fixture.customInstalled, isTrue);
      expect(node.selectedModel, AppFixture.customId);
      expect(find.byType(ModelBrowserPage), findsNothing);
      expect(
          fixture.requests.any((r) => r.url.path == '/cluster/start'), isFalse);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    });
  }

  testWidgets('editing a query ignores delayed results for the old search',
      (tester) async {
    final fixture = AppFixture();
    final node = (await tester.runAsync(fixture.createController))!;
    final delayed = Completer<http.Response>();
    fixture.intercept = (request) async => request.url.path == '/models/browse'
        ? delayed.future
        : fixture.respond(request);
    await mount(tester, node, location: '/models/browse');
    final input = find.byKey(const ValueKey('model-hub-query'));
    await tester.enterText(input, 'old query');
    await tester.tap(find.text('Search models'));
    await tester.pump();
    await tester.enterText(input, 'new query');
    delayed.complete(http.Response(
        jsonEncode({
          'models': [AppFixture.customModel],
          'repositories': []
        }),
        200));
    await tester.pumpAndSettle();
    expect(find.text('Community model Q4_K_M'), findsNothing);
    expect(find.text('new query'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
  });
}
