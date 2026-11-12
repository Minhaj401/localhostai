import 'dart:io';
import 'dart:math';
import 'package:flutter/foundation.dart';

/// What this phone can give to a split model.
///
/// The laptop uses these numbers to decide how many layers the phone holds.
/// Reporting optimistic values would make the laptop promise memory that does
/// not exist, so the budget subtracts a real reserve for Android and the app.
class DeviceProfile {
  final String id;
  final String name;
  final String kind;
  final int ramMb;
  final int availableMb;
  final int budgetMb;
  final int cores;
  final int threads;
  final String arch;

  const DeviceProfile({
    required this.id,
    required this.name,
    required this.kind,
    required this.ramMb,
    required this.availableMb,
    required this.budgetMb,
    required this.cores,
    required this.threads,
    required this.arch,
  });

  /// Reserve for Android itself plus this app. Matches the laptop's rule for
  /// phones in `core/src/hardware.rs` so both sides agree on the budget.
  static const int _phoneReserveMb = 900;
  static final String _sessionId = createId();

  static String createId() {
    final random = Random.secure();
    return 'phone-${List.generate(16, (_) => random.nextInt(256).toRadixString(16).padLeft(2, '0')).join()}';
  }

  /// Thread tiers, mirroring `Hardware::tier_threads` on the laptop.
  static int tierThreads(int ramMb, int cores) {
    final byRam = ramMb >= 12000
        ? 6
        : ramMb >= 8000
            ? 4
            : ramMb >= 4000
                ? 3
                : 2;
    return byRam < cores ? byRam : (cores < 1 ? 1 : cores);
  }

  static int shareBudgetMb(int ramMb, int availableMb) {
    final totalCapacity = ramMb > _phoneReserveMb ? ramMb - _phoneReserveMb : 0;
    final freeCapacity =
        availableMb > _phoneReserveMb ? availableMb - _phoneReserveMb : 0;
    return totalCapacity < freeCapacity ? totalCapacity : freeCapacity;
  }

  /// Android exposes total and currently available memory through /proc.
  static ({int total, int available}) readMemoryMb() {
    try {
      final txt = File('/proc/meminfo').readAsStringSync();
      int? total;
      int? available;
      for (final line in txt.split('\n')) {
        final fields =
            line.split(RegExp(r'\s+')).where((s) => s.isNotEmpty).toList();
        if (fields.length < 2) continue;
        if (fields.first == 'MemTotal:') total = int.tryParse(fields[1]);
        if (fields.first == 'MemAvailable:') {
          available = int.tryParse(fields[1]);
        }
      }
      if (total != null) {
        final totalMb = total ~/ 1024;
        return (
          total: totalMb,
          available: ((available ?? 0) ~/ 1024).clamp(0, totalMb)
        );
      }
    } catch (_) {
      // Fall through to the conservative default below.
    }
    return (total: 4096, available: 0);
  }

  static int readTotalRamMb() => readMemoryMb().total;

  static String archName() {
    if (kIsWeb) return 'web';
    final a = Platform.version.toLowerCase();
    if (a.contains('arm64') || a.contains('aarch64')) return 'arm64';
    if (a.contains('arm')) return 'arm';
    return 'x86_64';
  }

  static int coreCount() {
    try {
      return Platform.numberOfProcessors;
    } catch (_) {
      return 4;
    }
  }

  /// Build the profile this device will report.
  factory DeviceProfile.detect({required String name, String? id}) {
    final memory = readMemoryMb();
    final ram = memory.total;
    final cores = coreCount();
    final budget = shareBudgetMb(ram, memory.available);
    return DeviceProfile(
      id: id ?? _sessionId,
      name: name,
      kind: 'phone',
      ramMb: ram,
      availableMb: memory.available,
      budgetMb: budget,
      cores: cores,
      threads: tierThreads(ram, cores),
      arch: archName(),
    );
  }

  bool get canHostLayers => budgetMb > 0;

  Map<String, dynamic> toJson({String? rpc}) => {
        'id': id,
        'name': name,
        'kind': kind,
        'ram_mb': ramMb,
        'available_mb': availableMb,
        'cores': cores,
        'arch': arch,
        if (rpc != null && rpc.isNotEmpty) 'rpc': rpc,
      };
}
