import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:get/get.dart';

import '../../controllers/chat_controller.dart';
import '../../models/device_profile.dart';
import '../../models/managed_model.dart';
import '../../models/pairing.dart';
import '../../models/remote_model.dart';
import '../../services/node_client.dart';
import '../../services/phone_worker_service.dart';
import '../../services/storage_service.dart';

final storageProvider =
    Provider<StorageService>((ref) => Get.find<StorageService>());
final phoneWorkerProvider =
    Provider<PhoneWorkerService>((ref) => Get.find<PhoneWorkerService>());
final nodeClientFactoryProvider = Provider<NodeClient Function(PairPayload)>(
    (ref) => (pair) => NodeClient(pair));
final connectionProvider = ChangeNotifierProvider<ConnectionController>((ref) {
  final controller = ConnectionController(
    ref.watch(storageProvider),
    ref.watch(phoneWorkerProvider),
    clientFactory: ref.watch(nodeClientFactoryProvider),
  );
  Future.microtask(controller.restore);
  return controller;
});

/// Owns the paired coordinator snapshot across every navigation destination.
class ConnectionController extends ChangeNotifier with WidgetsBindingObserver {
  ConnectionController(this.storage, this.worker,
      {NodeClient Function(PairPayload)? clientFactory, bool polling = true})
      : _clientFactory = clientFactory ?? ((pair) => NodeClient(pair)),
        profile = DeviceProfile.detect(
            name: 'this phone', id: storage.getString('device_id')) {
    WidgetsBinding.instance.addObserver(this);
    _workerWatch = ever<String>(worker.state, (state) {
      if (_disposed) return;
      profile = DeviceProfile.detect(name: profile.name, id: profile.id);
      if (state.startsWith('error:')) error = state.replaceFirst('error: ', '');
      _record('phone worker: ${state.isEmpty ? 'off' : state}');
      notifyListeners();
    });
    if (polling) {
      _timer = Timer.periodic(const Duration(seconds: 4), (_) {
        if (_active && _client != null && !checking) unawaited(refresh());
      });
    }
  }

  final StorageService storage;
  final PhoneWorkerService worker;
  final NodeClient Function(PairPayload) _clientFactory;
  Timer? _timer;
  Worker? _workerWatch;
  NodeClient? _client;
  PairPayload? pair;
  NodeInfo? info;
  List<PoolDevice> devices = const [];
  List<ManagedModel> models = const [];
  Map<String, dynamic>? cluster;
  Map<String, dynamic>? managed;
  DeviceProfile profile;
  String? error;
  String? operationMessage;
  String? operationError;
  final List<String> _events = [];
  bool checking = false;
  bool refreshing = false;
  bool operationBusy = false;
  bool sharingBusy = false;
  bool connected = false;
  String? activeOperationModel;
  String? selectedModel;
  int contextTokens = 4096;
  SplitPlan? lastPlan;
  Future<void>? _refreshing;
  bool _disposed = false;
  int _epoch = 0;
  int _operationId = 0;
  int _previewId = 0;
  int _revision = 0;
  bool _active = true;

  NodeClient? get client => _client;
  bool get paired => storage.loadPair() != null;
  bool get ready => connected && info?.ready == true;
  String get endpoint => _client?.baseUrl ?? pair?.baseUrl ?? '';
  String get clusterState => cluster?['state'] as String? ?? 'stopped';
  String? get runningModel => cluster?['model'] as String?;
  String get installPhase =>
      (managed?['status'] as Map?)?['phase'] as String? ?? 'idle';
  bool get installing => [
        'downloading_runtime',
        'installing_runtime',
        'downloading_model',
        'verifying_model'
      ].contains(installPhase);
  int get downloadedBytes =>
      ((managed?['status'] as Map?)?['downloaded_bytes'] as num?)?.toInt() ?? 0;
  int get totalBytes =>
      ((managed?['status'] as Map?)?['total_bytes'] as num?)?.toInt() ?? 0;
  String? get downloadingModel =>
      (managed?['status'] as Map?)?['model'] as String? ?? activeOperationModel;
  String get eventLog => [
        ..._events,
        if (worker.lastFailure != null && worker.nativeOutput.isNotEmpty)
          'phone worker output:\n${worker.nativeOutput}',
      ].join('\n');

  Future<void> restore() async {
    if (_disposed) return;
    final saved = storage.loadPair();
    if (saved != null) {
      pair = saved;
      _client = _clientFactory(saved);
      await refresh();
      return;
    }
    final url = storage.getString('laptop_node_url');
    final uri = url == null ? null : Uri.tryParse(url);
    if (uri != null &&
        ['http', 'https'].contains(uri.scheme) &&
        uri.host.isNotEmpty) {
      pair = PairPayload(
          host: uri.host,
          port: uri.hasPort ? uri.port : 8765,
          token: storage.getString('pair_token') ?? '',
          name: uri.host);
      _client = NodeClient(pair!, endpoint: url);
      await refresh();
    }
  }

  Future<bool> connect(PairPayload candidate) async {
    if (!candidate.isValid) {
      error = 'Invalid pairing details.';
      notifyListeners();
      return false;
    }
    return _connect(candidate);
  }

  Future<bool> connectAddress(String address) async {
    final uri = Uri.tryParse(address.trim());
    if (uri == null ||
        !['http', 'https'].contains(uri.scheme) ||
        uri.host.isEmpty) {
      error = 'Enter a complete HTTP(S) coordinator address.';
      notifyListeners();
      return false;
    }
    final saved = storage.loadPair();
    if (saved != null && saved.baseUrl == uri.origin) return connect(saved);
    return _connect(
      PairPayload(host: uri.host, port: uri.port, token: '', name: uri.host),
      endpointOverride: uri.toString(),
      savePair: false,
    );
  }

  Future<bool> _connect(PairPayload candidate,
      {String? endpointOverride, bool savePair = true}) async {
    if (checking || operationBusy) return false;
    _revision++;
    checking = true;
    error = null;
    notifyListeners();
    final client = endpointOverride == null
        ? _clientFactory(candidate)
        : NodeClient(candidate, endpoint: endpointOverride);
    try {
      final health = await client.health();
      if (_disposed) return false;
      if (health == null) {
        throw StateError(
            'Cannot reach this laptop. Connect both devices to the same LAN.');
      }
      if (health.pairing && !await client.verifyPairing()) {
        throw StateError(
            'Pairing token rejected. Scan the current code from the laptop.');
      }
      if (_disposed) return false;
      final changed =
          endpoint != client.baseUrl || pair?.token != candidate.token;
      if (changed && worker.sharing.value) await worker.stop();
      if (savePair) {
        await storage.savePair(candidate);
      } else {
        await storage.selectNode(client.baseUrl);
      }
      if (_disposed) return false;
      _epoch++;
      _refreshing = null;
      _client?.dispose();
      _client = client;
      pair = candidate;
      info = health;
      connected = true;
      devices = const [];
      models = const [];
      cluster = null;
      managed = null;
      _record('connected to ${client.baseUrl}');
      if (changed && Get.isRegistered<ChatController>()) {
        await Get.find<ChatController>().rebuildBackend();
      }
      await refresh();
      return true;
    } catch (failure) {
      if (!_disposed) error = cleanError(failure);
      return false;
    } finally {
      if (!identical(_client, client)) client.dispose();
      if (!_disposed) {
        checking = false;
        notifyListeners();
      }
    }
  }

  Future<void> refresh() {
    if (_disposed || _client == null) return Future.value();
    if (_refreshing != null) return _refreshing!;
    late final Future<void> pending;
    pending = _refresh().whenComplete(() {
      if (identical(_refreshing, pending)) _refreshing = null;
    });
    _refreshing = pending;
    return pending;
  }

  Future<void> _refresh() async {
    final client = _client!;
    final epoch = _epoch;
    final revision = _revision;
    refreshing = true;
    notifyListeners();
    try {
      final values = await Future.wait<Object?>([
        client.health(),
        client.clusterStatus(),
        client.peers(),
        client.managedStatus(),
        client.registeredModels(),
      ]);
      if (_disposed || epoch != _epoch || revision != _revision) return;
      final health = values[0] as NodeInfo?;
      if (health == null) {
        throw StateError(
            'Laptop offline. Last known model and device information is retained.');
      }
      final status = values[1] as Map<String, dynamic>?;
      if (status == null) {
        throw StateError(
            'Cannot read coordinator controls. Check the pairing token.');
      }
      final before = clusterState;
      info = health;
      connected = true;
      cluster = status;
      devices = values[2] as List<PoolDevice>;
      managed = values[3] as Map<String, dynamic>;
      final catalog = [
        ...(managed?['catalog'] as List? ?? const []),
        ...(managed?['custom'] as List? ?? const []),
      ]
          .whereType<Map>()
          .map((value) =>
              ManagedModel.fromCatalog(Map<String, dynamic>.from(value)))
          .toList();
      for (final value in values[4] as List<Map<String, dynamic>>) {
        final name = value['name'];
        if (name is String && !catalog.any((model) => model.id == name)) {
          catalog.add(ManagedModel.fromRegistered(value));
        }
      }
      models = catalog;
      if (!models.any((model) => model.id == selectedModel)) {
        selectedModel = runningModel ??
            models
                .where((model) => model.installed)
                .map((model) => model.id)
                .firstOrNull ??
            models
                .where((model) => model.recommended)
                .map((model) => model.id)
                .firstOrNull ??
            models.firstOrNull?.id;
      }
      error = worker.lastFailure;
      if (before != clusterState) {
        _record(
            'model ${clusterState.toLowerCase()}${runningModel == null ? '' : ': $runningModel'}');
      }
      profile = DeviceProfile.detect(name: profile.name, id: profile.id);
    } catch (failure) {
      if (!_disposed && epoch == _epoch && revision == _revision) {
        connected = false;
        error = cleanError(failure);
        if (_events.lastOrNull != error) _record(error!);
      }
    } finally {
      if (!_disposed && epoch == _epoch && revision == _revision) {
        refreshing = false;
        notifyListeners();
      }
    }
  }

  void selectModel(String id) {
    _previewId++;
    selectedModel = id;
    lastPlan = null;
    notifyListeners();
  }

  void selectContext(int value) {
    _previewId++;
    contextTokens = value;
    lastPlan = null;
    notifyListeners();
  }

  Future<SplitPlan?> preview(String id) async {
    final requestId = ++_previewId;
    final epoch = _epoch;
    final ctx = contextTokens;
    if (models.any((model) => model.id == id && !model.canPreview)) {
      lastPlan = null;
      return null;
    }
    final plan = await _client?.plan(id, ctx: ctx);
    if (_disposed ||
        requestId != _previewId ||
        epoch != _epoch ||
        selectedModel != id ||
        ctx != contextTokens) return null;
    lastPlan = plan;
    notifyListeners();
    return plan;
  }

  Future<void> install(ManagedModel model, {bool load = false}) async {
    if (operationBusy || installing || !connected || _client == null) return;
    final id = ++_operationId;
    _revision++;
    final client = _client!;
    final ctx = contextTokens;
    operationBusy = true;
    activeOperationModel = model.id;
    error = null;
    operationError = null;
    operationMessage =
        load ? 'Checking placement...' : 'Downloading ${model.name}...';
    notifyListeners();
    try {
      if (load && !model.canPreview) {
        throw StateError('Download this GGUF before planning and loading it.');
      }
      if (load) {
        final plan = await client.plan(model.id, ctx: ctx);
        if (_disposed || id != _operationId) return;
        if (plan == null) {
          throw StateError('The coordinator did not return a placement plan.');
        }
        lastPlan = plan;
        if (!plan.fits) throw StateError(plan.summary);
      }
      if (model.catalog) {
        operationMessage = 'Installing and verifying ${model.name}...';
        notifyListeners();
        await client.installModel(model.id);
        if (_disposed || id != _operationId) return;
      }
      if (load) {
        operationMessage = 'Loading ${model.name}...';
        notifyListeners();
        final result = await client.startCluster(model.id, ctx: ctx);
        if (_disposed || id != _operationId) return;
        if (!result.ok) {
          throw StateError(result.error ?? 'Model did not start.');
        }
        lastPlan = result.plan ?? lastPlan;
      }
      operationMessage = load
          ? '${model.name} is ready for chat.'
          : '${model.name} is installed.';
      _record(operationMessage!);
      operationError = null;
      if (_refreshing != null) await _refreshing;
      await refresh();
    } catch (failure) {
      if (!_disposed && id == _operationId) {
        error = cleanError(failure);
        operationError = error;
        operationMessage = error;
        _record(error!);
      }
    } finally {
      if (!_disposed && id == _operationId) {
        operationBusy = false;
        activeOperationModel = null;
        notifyListeners();
      }
    }
  }

  Future<bool> download(RemoteModel model) async {
    if (operationBusy || installing || !connected || _client == null) {
      return false;
    }
    final id = ++_operationId;
    _revision++;
    final client = _client!;
    operationBusy = true;
    activeOperationModel = model.id;
    operationError = null;
    operationMessage = 'Downloading ${model.name} to the laptop...';
    notifyListeners();
    try {
      final installedId = await client.downloadModel(model);
      if (_disposed || id != _operationId) return false;
      selectedModel = installedId;
      lastPlan = null;
      operationMessage =
          '${model.name} is installed. Choose a context and load it from your library.';
      _record(operationMessage!);
      // An older poll may have been discarded by _revision. Wait for it before
      // requesting the new library, rather than reusing its stale future.
      if (_refreshing != null) await _refreshing;
      await refresh();
      return !_disposed && id == _operationId;
    } catch (failure) {
      if (!_disposed && id == _operationId) {
        operationError = cleanError(failure);
        operationMessage = operationError;
        _record(operationError!);
        if (_refreshing != null) await _refreshing;
        await refresh(); // Show a retained partial download for resume.
      }
      return false;
    } finally {
      if (!_disposed && id == _operationId) {
        operationBusy = false;
        activeOperationModel = null;
        notifyListeners();
      }
    }
  }

  Future<void> stopModel() async {
    if (_client == null || installing) return;
    final id = ++_operationId;
    _revision++;
    operationBusy = true;
    operationError = null;
    operationMessage = 'Stopping the model...';
    notifyListeners();
    try {
      if (!await _client!.stopCluster()) {
        throw StateError('The laptop did not confirm model shutdown.');
      }
      if (_disposed || id != _operationId) return;
      operationMessage = 'Model stopped.';
      operationError = null;
      _record(operationMessage!);
      await refresh();
    } catch (failure) {
      if (!_disposed) {
        error = cleanError(failure);
        operationError = error;
      }
    } finally {
      if (!_disposed && id == _operationId) {
        operationBusy = false;
        activeOperationModel = null;
        notifyListeners();
      }
    }
  }

  Future<void> sharePhone(bool enabled) async {
    if (sharingBusy) return;
    _revision++;
    sharingBusy = true;
    error = null;
    notifyListeners();
    try {
      if (enabled) {
        final saved = storage.loadPair();
        if (saved == null) {
          throw StateError('Pair the laptop before sharing this phone.');
        }
        await worker.startSharing(saved, profile);
      } else {
        await worker.stop();
      }
      await refresh();
    } catch (failure) {
      if (!_disposed) error = cleanError(failure);
    } finally {
      if (!_disposed) {
        sharingBusy = false;
        notifyListeners();
      }
    }
  }

  Future<void> disconnect() async {
    _revision++;
    _operationId++;
    operationBusy = false;
    activeOperationModel = null;
    operationMessage = null;
    operationError = null;
    await worker.stop();
    _epoch++;
    _client?.dispose();
    _client = null;
    await storage.setNodeUrl('');
    pair = null;
    info = null;
    connected = false;
    devices = const [];
    models = const [];
    cluster = null;
    managed = null;
    if (Get.isRegistered<ChatController>()) {
      await Get.find<ChatController>().rebuildBackend();
    }
    if (!_disposed) notifyListeners();
  }

  void _record(String message) {
    final now = DateTime.now();
    final time =
        '${now.hour.toString().padLeft(2, '0')}:${now.minute.toString().padLeft(2, '0')}:${now.second.toString().padLeft(2, '0')}';
    _events.add('$time  $message');
    if (_events.length > 80) _events.removeAt(0);
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _active = state == AppLifecycleState.resumed;
    if (_active && _client != null && !checking) unawaited(refresh());
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _epoch++;
    _timer?.cancel();
    _workerWatch?.dispose();
    _client?.dispose();
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }
}

String cleanError(Object error) => error
    .toString()
    .replaceFirst('Bad state: ', '')
    .replaceFirst('Exception: ', '');
