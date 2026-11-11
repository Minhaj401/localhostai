import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../features/connection/connection_controller.dart';
import '../features/server/server_page.dart';
import '../services/storage_service.dart';

/// Compatibility entry point for the coordinator dashboard.
class HomeScreen extends StatelessWidget {
  const HomeScreen({super.key, required this.storage});
  final StorageService storage;

  @override
  Widget build(BuildContext context) => ProviderScope(
        overrides: [storageProvider.overrideWithValue(storage)],
        child: const ServerPage(),
      );
}
