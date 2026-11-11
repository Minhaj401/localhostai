import 'dart:io';
import 'dart:ui' show ImageByteFormat, SemanticsAction;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:privatelm_lan/controllers/chat_controller.dart';
import 'package:privatelm_lan/features/connection/connection_controller.dart';
import 'package:privatelm_lan/features/onboarding/onboarding_page.dart';
import 'package:privatelm_lan/main.dart';
import 'package:privatelm_lan/services/phone_worker_service.dart';
import 'package:privatelm_lan/services/storage_service.dart';
import 'package:privatelm_lan/theme/theme.dart';
import 'package:privatelm_lan/theme/tokens.dart';
import 'package:privatelm_lan/ui/pair_screen.dart';
import 'package:privatelm_lan/widgets/buttons.dart';
import 'package:privatelm_lan/widgets/status_dot.dart';

import 'app_fixture.dart';
import 'support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async {
    GoogleFonts.config.allowRuntimeFetching = false;
    buildTheme();
    GoogleFonts.outfit(fontWeight: FontWeight.w600);
    await GoogleFonts.pendingFonts();
    final icons = FontLoader('packages/lucide_icons/Lucide');
    icons.addFont(rootBundle.load('packages/lucide_icons/assets/lucide.ttf'));
    await icons.load();
  });
  tearDown(Get.reset);

  test('dark palette uses Outfit UI and offline JetBrains Mono labels', () {
    final theme = buildTheme();
    expect(theme.brightness, Brightness.dark);
    expect(theme.scaffoldBackgroundColor, Colors.black);
    expect(theme.colorScheme.primary, C.accent);
    expect(theme.textTheme.bodyLarge?.fontFamily, contains('Outfit'));
    expect(theme.textTheme.bodyLarge?.color, C.text2);
    expect(mono(13).fontFamily, contains('JetBrainsMono'));
    expect(GoogleFonts.config.allowRuntimeFetching, isFalse);
    expect(theme.cardTheme.elevation, 0);
  });

  for (final size in const [
    Size(360, 800),
    Size(412, 915),
    Size(768, 1024),
    Size(1280, 800)
  ]) {
    testWidgets(
        'four-tab shell at ${size.width.toInt()}x${size.height.toInt()} with 1.3x text',
        (tester) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final fixture = AppFixture();
      final node = await tester.runAsync(fixture.createController);
      Get.put<StorageService>(fixture.storage);
      Get.put<PhoneWorkerService>(fixture.worker);
      Get.put(ChatController(fixture.storage,
          backendFactory: (id) => FakeBackend(id: id)));
      final handle = tester.ensureSemantics();
      final previewKey = GlobalKey();
      try {
        await tester.pumpWidget(ProviderScope(
            overrides: [
              storageProvider.overrideWithValue(fixture.storage),
              phoneWorkerProvider.overrideWithValue(fixture.worker),
              connectionProvider.overrideWith((ref) => node!),
            ],
            child: LocalhostAiApp(
                builder: (context, child) => RepaintBoundary(
                    key: previewKey,
                    child: MediaQuery(
                      data: MediaQuery.of(context).copyWith(
                          disableAnimations: true,
                          textScaler: const TextScaler.linear(1.3)),
                      child: child!,
                    )))));
        await tester.pumpAndSettle();
        expect(find.byType(size.width >= 840 ? NavigationRail : NavigationBar),
            findsOneWidget);
        final nav =
            find.byType(size.width >= 840 ? NavigationRail : NavigationBar);
        for (final label in ['Models', 'Server', 'Settings', 'Chat']) {
          await tester
              .tap(find.descendant(of: nav, matching: find.text(label)));
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull, reason: '$label at $size');
          await _savePreview(tester, previewKey,
              '${label.toLowerCase()}-${size.width.toInt()}x${size.height.toInt()}');
        }
        expect(find.byTooltip('Send message'), findsOneWidget);
        expect(find.bySemanticsLabel(RegExp('localhost.ai')), findsWidgets);
        await tester.pumpWidget(const SizedBox.shrink());
      } finally {
        handle.dispose();
        node?.dispose();
      }
    });
  }

  testWidgets(
      'tab changes preserve the chat draft and do not stop phone sharing',
      (tester) async {
    final fixture = AppFixture();
    final node = await tester.runAsync(fixture.createController);
    fixture.worker.sharing.value = true;
    Get.put<StorageService>(fixture.storage);
    Get.put<PhoneWorkerService>(fixture.worker);
    Get.put(ChatController(fixture.storage,
        backendFactory: (id) => FakeBackend(id: id)));
    await tester.pumpWidget(ProviderScope(
        overrides: [
          storageProvider.overrideWithValue(fixture.storage),
          phoneWorkerProvider.overrideWithValue(fixture.worker),
          connectionProvider.overrideWith((ref) => node!),
        ],
        child: LocalhostAiApp(
            builder: (context, child) => MediaQuery(
                  data:
                      MediaQuery.of(context).copyWith(disableAnimations: true),
                  child: child!,
                ))));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).first, 'Keep this draft');
    final nav = find.byType(NavigationBar);
    await tester.tap(find.descendant(of: nav, matching: find.text('Server')));
    await tester.pumpAndSettle();
    await tester.tap(find.descendant(of: nav, matching: find.text('Chat')));
    await tester.pumpAndSettle();
    expect(find.text('Keep this draft'), findsOneWidget);
    expect(fixture.worker.sharing.value, isTrue);
    expect(fixture.worker.stops, 0);
    await tester.pumpWidget(const SizedBox.shrink());
    node?.dispose();
  });

  testWidgets('reduced-motion status dot has no repeating frame callbacks',
      (tester) async {
    await tester.pumpWidget(MaterialApp(
        theme: buildTheme(),
        home: const MediaQuery(
          data: MediaQueryData(disableAnimations: true),
          child: Scaffold(body: StatusDot()),
        )));
    await tester.pump(const Duration(seconds: 3));
    expect(tester.binding.transientCallbackCount, 0);
    expect(tester.takeException(), isNull);
  });

  testWidgets(
      'disabled primary action is labelled, at least 48px, and cannot fire',
      (tester) async {
    final handle = tester.ensureSemantics();
    try {
      await tester.pumpWidget(MaterialApp(
          theme: buildTheme(),
          home: const Scaffold(
            body:
                SizedBox(width: 240, child: PrimaryButton(label: 'Run model')),
          )));
      await tester.pump();
      expect(tester.getSize(find.byType(PrimaryButton)).height,
          greaterThanOrEqualTo(48));
      final semantics = tester.getSemantics(find.byType(PrimaryButton));
      expect(semantics.label, 'Run model');
      expect(
          semantics.getSemanticsData().hasAction(SemanticsAction.tap), isFalse);
      expect(tester.takeException(), isNull);
    } finally {
      handle.dispose();
    }
  });

  testWidgets('manual pairing validates inputs and obscures the token',
      (tester) async {
    await tester.pumpWidget(MaterialApp(
        theme: buildTheme(), home: const PairScreen(initialManual: true)));
    await tester.pumpAndSettle();
    final connect = find.byType(PrimaryButton);
    await tester.ensureVisible(connect);
    await tester.tap(connect);
    await tester.pump();
    expect(
        find.text('Enter the laptop IP address or hostname'), findsOneWidget);
    expect(find.text('Enter the token shown by the laptop'), findsOneWidget);
    expect(
        find.byWidgetPredicate(
            (widget) => widget is TextField && widget.obscureText),
        findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets('onboarding is scrollable on a small landscape viewport',
      (tester) async {
    tester.view.physicalSize = const Size(800, 360);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final storage = MemoryStorage();
    await tester.pumpWidget(ProviderScope(
      overrides: [storageProvider.overrideWithValue(storage)],
      child: MaterialApp(
          theme: buildTheme(),
          home: const MediaQuery(
            data: MediaQueryData(
                size: Size(800, 360),
                disableAnimations: true,
                textScaler: TextScaler.linear(1.3)),
            child: OnboardingPage(),
          )),
    ));
    await tester.pumpAndSettle();
    expect(find.text('Continue'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
  });
}

Future<void> _savePreview(
    WidgetTester tester, GlobalKey key, String name) async {
  final output = Platform.environment['LOCALHOST_UI_PREVIEW_DIR'];
  if (output == null) return;
  await tester.runAsync(() async {
    final boundary =
        key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
    final image = await boundary.toImage();
    try {
      final data = await image.toByteData(format: ImageByteFormat.png);
      final directory = Directory(output);
      await directory.create(recursive: true);
      await File('$output/$name.png').writeAsBytes(data!.buffer.asUint8List());
    } finally {
      image.dispose();
    }
  });
}
