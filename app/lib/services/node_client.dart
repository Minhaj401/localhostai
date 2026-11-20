import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import '../models/device_profile.dart';
import '../models/pairing.dart';
import '../models/remote_model.dart';

class NodeHttpException implements Exception {
  const NodeHttpException(this.status, this.message);
  final int status;
  final String message;
  @override
  String toString() => message;
}

/// A node with its health, for the picker.
class NodeInfo {
  final String baseUrl;
  final String name;
  final String backend;
  final List<String> models;
  final bool pairing;
  final String version;

  const NodeInfo({
    required this.baseUrl,
    required this.name,
    required this.backend,
    required this.models,
    required this.pairing,
    required this.version,
  });

  bool get ready => backend.isNotEmpty && backend != 'none';
}

/// One device in the compute pool, as the node reports it.
class PoolDevice {
  final String id;
  final String name;
  final String kind;
  final int ramMb;
  final int availableMb;
  final int budgetMb;
  final int cores;
  final int threads;
  final bool isSelf;
  final bool enabled;
  final bool stale;
  final bool reachable;
  final int heartbeatSeconds;
  final int assignedLayers;
  final int assignedMemoryMb;
  final bool? reportedUsable;
  final bool inUse;
  final String? reason;
  final int swapUsedMb;
  final double pressureFullAvg10;

  const PoolDevice({
    this.id = '',
    required this.name,
    required this.kind,
    this.ramMb = 0,
    this.availableMb = 0,
    required this.budgetMb,
    this.cores = 0,
    required this.threads,
    required this.isSelf,
    this.enabled = true,
    required this.stale,
    required this.reachable,
    this.heartbeatSeconds = 0,
    this.assignedLayers = 0,
    this.assignedMemoryMb = 0,
    this.reportedUsable,
    this.inUse = false,
    this.reason,
    this.swapUsedMb = 0,
    this.pressureFullAvg10 = 0,
  });

  /// Layers can only be placed where a worker answers.
  bool get usable =>
      enabled &&
      (budgetMb > 0 || inUse) &&
      (reportedUsable ?? (isSelf || reachable));

  factory PoolDevice.fromJson(Map<String, dynamic> j) => PoolDevice(
        id: (j['id'] as String?) ?? '',
        name: (j['name'] as String?) ?? '?',
        kind: (j['kind'] as String?) ?? '?',
        ramMb: (j['ram_mb'] as num?)?.toInt() ?? 0,
        availableMb: (j['available_mb'] as num?)?.toInt() ?? 0,
        budgetMb: (j['budget_mb'] as num?)?.toInt() ?? 0,
        cores: (j['cores'] as num?)?.toInt() ?? 0,
        threads: (j['threads'] as num?)?.toInt() ?? 0,
        isSelf: j['self'] == true,
        enabled: j['enabled'] != false,
        stale: j['stale'] == true,
        reachable: j['reachable'] == true,
        heartbeatSeconds: (j['heartbeat_seconds'] as num?)?.toInt() ?? 0,
        assignedLayers: (j['assigned_layers'] as num?)?.toInt() ?? 0,
        assignedMemoryMb: (j['assigned_memory_mb'] as num?)?.toInt() ?? 0,
        reportedUsable: j['usable'] as bool?,
        inUse: j['in_use'] == true,
        reason: j['reason'] as String?,
        swapUsedMb:
            ((j['memory'] as Map?)?['swap_used_mb'] as num?)?.toInt() ?? 0,
        pressureFullAvg10:
            ((j['memory'] as Map?)?['pressure_full_avg10'] as num?)
                    ?.toDouble() ??
                0,
      );
}

/// Laptop memory tuning as the coordinator reports it.
///
/// `reserveMb` is what the laptop keeps for its own OS and apps (`null` when
/// the kind-based default applies); `remoteHeadroomMb` is what every remote
/// device keeps back for RPC runtime buffers.
class LaptopMemory {
  final int? reserveMb;
  final int effectiveReserveMb;
  final int remoteHeadroomMb;
  final int defaultReserveMb;
  final int totalMb;
  final int availableMb;
  final int budgetMb;

  const LaptopMemory({
    required this.reserveMb,
    required this.effectiveReserveMb,
    required this.remoteHeadroomMb,
    required this.defaultReserveMb,
    required this.totalMb,
    required this.availableMb,
    required this.budgetMb,
  });

  factory LaptopMemory.fromJson(Map<String, dynamic> j) => LaptopMemory(
        reserveMb: (j['reserve_mb'] as num?)?.toInt(),
        effectiveReserveMb:
            (j['effective_reserve_mb'] as num?)?.toInt() ?? 0,
        remoteHeadroomMb:
            (j['remote_headroom_mb'] as num?)?.toInt() ?? 0,
        defaultReserveMb:
            (j['default_reserve_mb'] as num?)?.toInt() ?? 0,
        totalMb: (j['total_mb'] as num?)?.toInt() ?? 0,
        availableMb: (j['available_mb'] as num?)?.toInt() ?? 0,
        budgetMb: (j['budget_mb'] as num?)?.toInt() ?? 0,
      );

  /// Budget the laptop would offer to the pool with [reserve] kept free —
  /// the same `min(total, available) - reserve` rule the coordinator uses.
  int budgetForReserveMb(int reserve) {
    final totalCapacity = totalMb > reserve ? totalMb - reserve : 0;
    final freeCapacity = availableMb > reserve ? availableMb - reserve : 0;
    return totalCapacity < freeCapacity ? totalCapacity : freeCapacity;
  }
}

/// How a model is divided across the pool.
class SplitPlan {
  final String model;
  final int totalLayers;
  final bool fits;
  final int shortfallMb;
  final String summary;
  final List<PlanRow> rows;

  const SplitPlan({
    required this.model,
    required this.totalLayers,
    required this.fits,
    required this.shortfallMb,
    required this.summary,
    required this.rows,
  });

  factory SplitPlan.fromJson(Map<String, dynamic> j) => SplitPlan(
        model: (j['model'] as String?) ?? '',
        totalLayers: (j['total_layers'] as num?)?.toInt() ?? 0,
        fits: j['over_budget'] == false,
        shortfallMb: (j['shortfall_mb'] as num?)?.toInt() ?? 0,
        summary: (j['summary'] as String?) ?? '',
        rows: ((j['assignments'] as List?) ?? [])
            .map((e) => PlanRow.fromJson(e as Map<String, dynamic>))
            .toList(),
      );
}

class PlanRow {
  final String deviceName;
  final int layers;
  final int memoryMb;
  final int budgetMb;

  const PlanRow({
    required this.deviceName,
    required this.layers,
    required this.memoryMb,
    required this.budgetMb,
  });

  factory PlanRow.fromJson(Map<String, dynamic> j) => PlanRow(
        deviceName: (j['device_name'] as String?) ?? '?',
        layers: (j['layers'] as num?)?.toInt() ?? 0,
        memoryMb: (j['memory_mb'] as num?)?.toInt() ?? 0,
        budgetMb: (j['budget_mb'] as num?)?.toInt() ?? 0,
      );
}

/// Result of a cluster start/stop request.
class ClusterOutcome {
  final bool ok;
  final String? error;
  final SplitPlan? plan;

  const ClusterOutcome({required this.ok, this.error, this.plan});
}

/// Talks to the laptop node: health, models, pairing, peers, cluster.
///
/// Every call carries the pair token when one is held, because a node with
/// `require_pairing` rejects anything unauthorised.
class NodeClient {
  final PairPayload pair;
  final http.Client _http;
  final String? _endpoint;
  final DeviceProfile Function(DeviceProfile) _refreshProfile;
  Timer? _heartbeat;
  Future<void>? _heartbeatPending;
  int _heartbeatEpoch = 0;
  int _reportSequence = 0;
  String? lastPeerError;
  String? activeInstallJob;
  double? _lastReportRttMs;

  NodeClient(this.pair,
      {http.Client? client,
      String? endpoint,
      DeviceProfile Function(DeviceProfile)? refreshProfile})
      : _http = client ?? http.Client(),
        _endpoint = endpoint?.replaceFirst(RegExp(r'/+$'), ''),
        _refreshProfile = refreshProfile ??
            ((profile) =>
                DeviceProfile.detect(name: profile.name, id: profile.id));

  String get baseUrl => _endpoint ?? pair.baseUrl;

  Map<String, String> get _headers => {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer ${pair.token}',
      };

  Future<NodeInfo?> health() async {
    try {
      final r = await _http
          .get(Uri.parse('$baseUrl/health'), headers: _headers)
          .timeout(const Duration(seconds: 4));
      if (r.statusCode != 200) return null;
      final j = jsonDecode(r.body) as Map<String, dynamic>;
      return NodeInfo(
        baseUrl: baseUrl,
        name: (j['name'] as String?) ?? pair.name,
        backend: (j['backend'] as String?) ?? '',
        models: ((j['models'] as List?) ?? [])
            .map((e) => e is String ? e : (e is Map ? e['id'] : null))
            .whereType<String>()
            .toList(),
        pairing: j['pairing'] == true,
        version: (j['version'] as String?) ?? '',
      );
    } catch (_) {
      return null;
    }
  }

  Future<bool> verifyPairing() async {
    try {
      final r = await _http
          .post(Uri.parse('$baseUrl/pair/verify'), headers: _headers)
          .timeout(const Duration(seconds: 5));
      return r.statusCode == 200 && (jsonDecode(r.body) as Map)['ok'] == true;
    } catch (_) {
      return false;
    }
  }

  /// Models that can actually be split across devices.
  ///
  /// This is deliberately not `/health`: that advertises whatever the backend
  /// serves (e.g. Ollama tags), which cannot be layer-split. Planning needs a
  /// real GGUF path, so the picker must show registered files only — otherwise
  /// the UI offers models that will always fail to plan.
  Future<List<String>> plannableModels() async {
    try {
      final r = await _http
          .get(Uri.parse('$baseUrl/models'), headers: _headers)
          .timeout(const Duration(seconds: 5));
      if (r.statusCode != 200) return const [];
      final j = jsonDecode(r.body) as Map<String, dynamic>;
      final list = (j['models'] as List?) ?? const [];
      return list
          .map((m) => (m is Map ? m['name'] : m)?.toString())
          .whereType<String>()
          .toList();
    } catch (_) {
      return const [];
    }
  }

  /// Report this phone's hardware so the laptop can plan a split with it.
  ///
  /// [rpc] is the address of the worker running on this phone; without it the
  /// laptop will refuse to count this device's memory.
  Future<bool> reportSelf(DeviceProfile profile,
      {String? rpc, Map<String, dynamic>? worker}) async {
    final started = Stopwatch()..start();
    try {
      final r = await _http
          .post(
            Uri.parse('$baseUrl/peers'),
            headers: _headers,
            body: jsonEncode({
              ...profile.toJson(rpc: rpc),
              if (worker != null)
                'worker': {
                  ...worker,
                  'sequence': ++_reportSequence,
                  if (_lastReportRttMs != null)
                    'coordinator_rtt_ms': _lastReportRttMs
                },
            }),
          )
          .timeout(const Duration(seconds: 5));
      final value = jsonDecode(r.body) as Map;
      _lastReportRttMs = started.elapsedMicroseconds / 1000;
      final accepted = r.statusCode == 200 &&
          value['ok'] == true &&
          value['ignored'] != true;
      lastPeerError = accepted
          ? null
          : _errorMessage(value['error'] ??
              'Device report was rejected (HTTP ${r.statusCode}).');
      return accepted;
    } catch (error) {
      lastPeerError = 'Device report to $baseUrl/peers failed: $error';
      return false;
    }
  }

  /// Keep reporting so the node does not mark this phone stale.
  ///
  /// Reachability decides whether memory is usable; heartbeats keep device
  /// metadata current independently of that reachability check.
  void startHeartbeat(DeviceProfile profile,
      {String? rpc,
      Future<String?> Function()? rpcProvider,
      Map<String, dynamic> Function()? workerReport}) {
    stopHeartbeat();
    final epoch = _heartbeatEpoch;
    _heartbeat = Timer.periodic(const Duration(seconds: 10), (_) {
      if (_heartbeatPending != null) return;
      final pending =
          _sendHeartbeat(profile, epoch, rpc, rpcProvider, workerReport);
      _heartbeatPending = pending;
      unawaited(pending.whenComplete(() {
        if (identical(_heartbeatPending, pending)) _heartbeatPending = null;
      }));
    });
  }

  Future<void> _sendHeartbeat(
      DeviceProfile profile,
      int epoch,
      String? rpc,
      Future<String?> Function()? rpcProvider,
      Map<String, dynamic> Function()? workerReport) async {
    try {
      final address = rpcProvider == null ? rpc : await rpcProvider();
      if (epoch != _heartbeatEpoch) return;
      // Never alternate a fresh sample with the stale startup profile.
      final fresh = _refreshProfile(profile);
      await reportSelf(fresh, rpc: address, worker: workerReport?.call());
    } catch (_) {
      // The next heartbeat retries a temporary address or network failure.
    }
  }

  Future<void> stopHeartbeat() {
    _heartbeatEpoch++;
    _heartbeat?.cancel();
    _heartbeat = null;
    return _heartbeatPending ?? Future<void>.value();
  }

  Future<List<PoolDevice>> peers() async {
    try {
      final r = await _http
          .get(Uri.parse('$baseUrl/peers'), headers: _headers)
          .timeout(const Duration(seconds: 5));
      if (r.statusCode != 200) return const [];
      final j = jsonDecode(r.body) as Map<String, dynamic>;
      return ((j['devices'] as List?) ?? [])
          .map((e) => PoolDevice.fromJson(e as Map<String, dynamic>))
          .toList();
    } catch (_) {
      return const [];
    }
  }

  /// Current memory tuning, or null when the laptop runs an older build
  /// without `/config/memory`.
  Future<LaptopMemory?> memoryConfig() async {
    try {
      final r = await _http
          .get(Uri.parse('$baseUrl/config/memory'), headers: _headers)
          .timeout(const Duration(seconds: 5));
      if (r.statusCode != 200) return null;
      return LaptopMemory.fromJson(jsonDecode(r.body) as Map<String, dynamic>);
    } catch (_) {
      return null;
    }
  }

  /// Change what the laptop keeps for itself ([reserveMb], or clear an
  /// explicit reserve with [useDefaultReserve]) or what every remote device
  /// keeps back for RPC buffers ([remoteHeadroomMb]). Fields that are not
  /// sent stay unchanged. Returns the new state, or null when the laptop
  /// rejected the change or is unreachable.
  Future<LaptopMemory?> setMemoryConfig({
    int? reserveMb,
    bool useDefaultReserve = false,
    int? remoteHeadroomMb,
  }) async {
    try {
      final r = await _http
          .post(
            Uri.parse('$baseUrl/config/memory'),
            headers: _headers,
            body: jsonEncode({
              if (useDefaultReserve)
                'reserve_mb': null
              else if (reserveMb != null)
                'reserve_mb': reserveMb,
              if (remoteHeadroomMb != null)
                'remote_headroom_mb': remoteHeadroomMb,
            }),
          )
          .timeout(const Duration(seconds: 10));
      if (r.statusCode != 200) return null;
      return LaptopMemory.fromJson(jsonDecode(r.body) as Map<String, dynamic>);
    } catch (_) {
      return null;
    }
  }

  Future<SplitPlan?> plan(String model, {int ctx = 4096}) async {
    try {
      final r = await _http
          .post(
            Uri.parse('$baseUrl/cluster/plan'),
            headers: _headers,
            body: jsonEncode({'model': model, 'ctx': ctx}),
          )
          .timeout(const Duration(seconds: 15));
      if (r.statusCode != 200) return null;
      final j = jsonDecode(r.body) as Map<String, dynamic>;
      return SplitPlan.fromJson(j['plan'] as Map<String, dynamic>);
    } catch (_) {
      return null;
    }
  }

  Future<ClusterOutcome> startCluster(String model, {int ctx = 4096}) async {
    try {
      final r = await _http
          .post(
            Uri.parse('$baseUrl/cluster/start'),
            headers: _headers,
            body: jsonEncode({'model': model, 'ctx': ctx}),
          )
          .timeout(const Duration(seconds: 240));
      final j = jsonDecode(r.body) as Map<String, dynamic>;
      final ok = j['ok'] == true;
      final planJson = j['plan'];
      return ClusterOutcome(
        ok: ok,
        error: ok
            ? null
            : _errorMessage(j['error'] ?? (j['status'] as Map?)?['error']),
        plan: planJson is Map<String, dynamic>
            ? SplitPlan.fromJson(planJson)
            : null,
      );
    } catch (e) {
      return ClusterOutcome(ok: false, error: e.toString());
    }
  }

  Future<bool> stopCluster() async {
    try {
      final r = await _http
          .post(Uri.parse('$baseUrl/cluster/stop'), headers: _headers)
          .timeout(const Duration(seconds: 20));
      return r.statusCode == 200 && (jsonDecode(r.body) as Map)['ok'] == true;
    } catch (_) {
      return false;
    }
  }

  Future<Map<String, dynamic>?> clusterStatus() async {
    try {
      final r = await _http
          .get(Uri.parse('$baseUrl/cluster'), headers: _headers)
          .timeout(const Duration(seconds: 5));
      if (r.statusCode != 200) return null;
      return jsonDecode(r.body) as Map<String, dynamic>;
    } catch (_) {
      return null;
    }
  }

  Future<Map<String, dynamic>> managedStatus() => _json('/managed');
  Future<Map<String, dynamic>> document(String path,
          {Map<String, dynamic>? body}) =>
      _json(path, body: body);
  Future<double> ping() async {
    final watch = Stopwatch()..start();
    await _json('/ping', timeout: const Duration(seconds: 3));
    return watch.elapsedMicroseconds / 1000;
  }

  Future<List<Map<String, dynamic>>> registeredModels() async {
    final value = await _json('/models');
    return (value['models'] as List? ?? const [])
        .whereType<Map>()
        .map((item) => Map<String, dynamic>.from(item))
        .toList();
  }

  Future<void> installModel(String id) async {
    final value = await _json('/managed/install',
        body: {'model': id, 'async': true},
        timeout: const Duration(seconds: 30));
    await _waitInstall(value);
  }

  Future<HubResults> browseModels(String query) async => HubResults.fromJson(
        await _json('/models/browse',
            body: {'query': query}, timeout: const Duration(seconds: 45)),
      );

  Future<String> downloadModel(RemoteModel model) async {
    final value = await _json('/managed/download',
        body: {'source': model.source, 'async': true},
        timeout: const Duration(seconds: 30));
    final result = await _waitInstall(value);
    final name = (result['result'] as Map?)?['name'];
    if (name is! String || name.isEmpty) {
      throw const FormatException(
          'The coordinator did not return the downloaded model');
    }
    return name;
  }

  Future<Map<String, dynamic>> _waitInstall(
      Map<String, dynamic> accepted) async {
    final job = accepted['job'];
    final id = job is Map ? job['id'] : null;
    if (id is! String || id.isEmpty) {
      throw StateError(_errorMessage(accepted['error'] ??
          'The coordinator did not create an installation job'));
    }
    activeInstallJob = id;
    final deadline = DateTime.now().add(const Duration(hours: 2));
    var terminal = false;
    try {
      while (DateTime.now().isBefore(deadline)) {
        Map<String, dynamic> value;
        try {
          value = await _json('/managed/jobs/${Uri.encodeComponent(id)}',
              timeout: const Duration(seconds: 12));
        } on NodeHttpException catch (error) {
          if (error.status == 404) {
            terminal = true;
            throw StateError(
                'The coordinator restarted and no longer knows this installation job. Retry the model to resume its saved partial download.');
          }
          if (error.status >= 400 && error.status < 500) {
            terminal = true;
            throw StateError(error.message);
          }
          await Future<void>.delayed(const Duration(seconds: 2));
          continue;
        } catch (_) {
          // A transient poll failure must not orphan a server-side job or make
          // it impossible for the user to cancel it.
          await Future<void>.delayed(const Duration(seconds: 2));
          continue;
        }
        final current = value['job'];
        if (current is! Map) {
          throw const FormatException('Invalid installation status');
        }
        final state = current['state'];
        if (state == 'succeeded') {
          terminal = true;
          return Map<String, dynamic>.from(current);
        }
        if (state == 'failed' || state == 'cancelled') {
          terminal = true;
          throw StateError(
              _errorMessage(current['error'] ?? 'Installation $state'));
        }
        await Future<void>.delayed(const Duration(seconds: 1));
      }
      throw StateError(
          'Installation exceeded its two-hour operation deadline. Retry to resume the saved partial download.');
    } finally {
      if (terminal && activeInstallJob == id) activeInstallJob = null;
    }
  }

  void adoptInstallJob(Map<String, dynamic>? managed) {
    final jobs = managed?['jobs'] as List? ?? const [];
    final active = jobs
        .whereType<Map>()
        .where((job) => job['state'] == 'queued' || job['state'] == 'running')
        .firstOrNull;
    if (active?['id'] is String) activeInstallJob = active!['id'] as String;
  }

  Future<bool> cancelInstall() async {
    final id = activeInstallJob;
    if (id == null) return false;
    try {
      final value = await _json('/managed/cancel',
          body: {'job_id': id}, timeout: const Duration(seconds: 8));
      return value['ok'] == true;
    } catch (_) {
      return false;
    }
  }

  Future<void> deleteModel(String id) async {
    await _json('/managed/delete', body: {'model': id});
  }

  Future<void> resetRuntime() async {
    await _json('/managed/runtime/reset', body: const {});
  }

  Future<Map<String, dynamic>> _json(String path,
      {Map<String, dynamic>? body,
      Duration timeout = const Duration(seconds: 8)}) async {
    final uri = Uri.parse('$baseUrl$path');
    final response = await (body == null
            ? _http.get(uri, headers: _headers)
            : _http.post(uri, headers: _headers, body: jsonEncode(body)))
        .timeout(timeout,
            onTimeout: () => throw StateError(
                '${body == null ? 'GET' : 'POST'} $baseUrl$path timed out after ${timeout.inSeconds}s. Check the coordinator address and connection.'));
    final value = jsonDecode(response.body);
    if (value is! Map<String, dynamic>) {
      throw const FormatException('Invalid node response');
    }
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw NodeHttpException(response.statusCode,
          _errorMessage(value['error'] ?? 'HTTP ${response.statusCode}'));
    }
    return value;
  }

  void dispose() {
    stopHeartbeat();
    _http.close();
  }

  static String _errorMessage(Object? error) {
    if (error is Map && error['message'] != null) return '${error['message']}';
    return error?.toString() ?? 'cluster did not start';
  }
}
