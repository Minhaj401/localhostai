import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:get/get.dart';

/// Runs the bundled llama.cpp RPC worker (libggmlrpc.so) so the laptop can
/// split model layers onto this phone's RAM/CPU. The laptop discovers this
/// worker by scanning the LAN on port 50052.
class PhoneWorkerService extends GetxService {
  static const _channel = MethodChannel('privatelm/native');
  static const port = 50052;

  Process? _proc;
  final state = ''.obs; // '', 'running', 'stopped', 'error: …'

  bool get isRunning => _proc != null && state.value == 'running';

  Future<String?> localIp() async => _channel.invokeMethod<String>('localIp');

  Future<void> start() async {
    if (isRunning) return;
    try {
      final libDir = await _channel.invokeMethod<String>('nativeLibDir');
      if (libDir == null) throw StateError('nativeLibDir unavailable');
      final bin = '$libDir/libggmlrpc.so';

      // Same device-tier logic as phone-rpc/setup-termux.sh: scale threads
      // to the phone instead of hardcoding.
      final totalMb = await _totalRamMb() ?? 4096;
      final threads = totalMb >= 12000 ? 6 : totalMb >= 8000 ? 4 : totalMb >= 4000 ? 3 : 2;

      _proc = await Process.start(
        bin,
        ['--host', '0.0.0.0', '--port', '$port', '-t', '$threads'],
        mode: ProcessStartMode.detached,
      );
      state.value = 'running';
    } catch (e) {
      state.value = 'error: $e';
      rethrow;
    }
  }

  void stop() {
    _proc?.kill();
    _proc = null;
    state.value = 'stopped';
  }

  Future<int?> _totalRamMb() async {
    try {
      final meminfo = File('/proc/meminfo').readAsStringSync();
      final match = RegExp(r'MemTotal:\s+(\d+) kB').firstMatch(meminfo);
      return match != null ? int.parse(match.group(1)!) ~/ 1024 : null;
    } catch (_) {
      return null;
    }
  }
}
