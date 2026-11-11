import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_native_splash/flutter_native_splash.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:get/get.dart';
import 'package:google_fonts/google_fonts.dart';

import 'controllers/chat_controller.dart';
import 'features/connection/connection_controller.dart';
import 'features/shell/app_router.dart';
import 'services/phone_worker_service.dart';
import 'services/storage_service.dart';
import 'theme/theme.dart';
import 'theme/tokens.dart';

Future<void> main() async {
  final binding = WidgetsFlutterBinding.ensureInitialized();
  FlutterNativeSplash.preserve(widgetsBinding: binding);
  SystemChrome.setSystemUIOverlayStyle(const SystemUiOverlayStyle(
    statusBarColor: C.bg,
    statusBarIconBrightness: Brightness.light,
    systemNavigationBarColor: C.bg,
    systemNavigationBarIconBrightness: Brightness.light,
  ));
  GoogleFonts.config.allowRuntimeFetching = false;
  buildTheme();
  final storage = StorageService();
  await storage.init();
  final worker = PhoneWorkerService();
  Get.put(storage, permanent: true);
  Get.put(worker, permanent: true);
  Get.put(ChatController(storage), permanent: true);
  await GoogleFonts.pendingFonts();
  runApp(ProviderScope(overrides: [
    storageProvider.overrideWithValue(storage),
    phoneWorkerProvider.overrideWithValue(worker),
  ], child: const LocalhostAiApp()));
  binding.addPostFrameCallback((_) => FlutterNativeSplash.remove());
}

class LocalhostAiApp extends ConsumerWidget {
  const LocalhostAiApp({super.key, this.builder});
  final TransitionBuilder? builder;

  @override
  Widget build(BuildContext context, WidgetRef ref) => MaterialApp.router(
        title: 'localhost.ai',
        debugShowCheckedModeBanner: false,
        theme: buildTheme(),
        darkTheme: buildTheme(),
        themeMode: ThemeMode.dark,
        builder: builder,
        routerConfig: ref.watch(routerProvider),
      );
}
