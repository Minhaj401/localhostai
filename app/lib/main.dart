import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'controllers/chat_controller.dart';
import 'services/storage_service.dart';
import 'ui/chat_screen.dart';

Future<void> main() async {
  final storage = StorageService();
  await storage.init();
  Get.put(storage);

  // Restore last-used backend (default: laptop offload).
  final lastBackend = storage.getString('active_backend') ?? 'laptop';

  runApp(PrivateLanApp(initialBackend: lastBackend));
}

class PrivateLanApp extends StatelessWidget {
  const PrivateLanApp({super.key, required this.initialBackend});

  final String initialBackend;

  @override
  Widget build(BuildContext context) {
    final controller = Get.put(ChatController(Get.find<StorageService>()));
    if (controller.activeBackendId.value != initialBackend) {
      controller.switchBackend(initialBackend);
    }
    // Find the laptop node on the LAN before the first message fails.
    controller.autoDiscoverNode();
    return GetMaterialApp(
      title: 'PrivateLM-LAN',
      theme: ThemeData(colorSchemeSeed: Colors.indigo, useMaterial3: true),
      darkTheme: ThemeData(
          colorSchemeSeed: Colors.indigo,
          useMaterial3: true,
          brightness: Brightness.dark),
      home: const ChatScreen(),
    );
  }
}
