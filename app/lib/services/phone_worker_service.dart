import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:get/get.dart';

import '../models/device_profile.dart';
import '../models/pairing.dart';
import 'node_client.dart';

/// Owns the bundled RPC process and its node registration. Sharing continues
/// across screens and connection refreshes, and ends when the worker exits.
class PhoneWorkerService extends GetxService with WidgetsBindingObserver {
  PhoneWorkerService({
    Future<Process> Function(String, List<String>, String, Map<String, String>)?
        launch,
    Future<bool> Function(int)? probe,
    NodeClient Function(PairPayload)? nodeClient,
    bool? androidProtection,
  })  : _launch = launch ??
            ((bin, args, workingDirectory, environment) => Process.start(
                  bin,
                  args,
                  workingDirectory: workingDirectory,
                  environment: environment,
                )),
        _probe = probe ?? _workerReachable,
        _nodeClient = nodeClient ?? ((pair) => NodeClient(pair)),
        _androidProtection = androidProtection ?? Platform.isAndroid;

  static const _channel = MethodChannel('localhostai/native');
  static const defaultPort = 50052;
  static const _lastFallbackPort = 50062;
  final Future<Process> Function(
      String, List<String>, String, Map<String, String>) _launch;
  final Future<bool> Function(int) _probe;
  final NodeClient Function(PairPayload) _nodeClient;
  final bool _androidProtection;
  bool _protected = false;
  Future<void> _protectionClosing = Future<void>.value();
  Future<void> _restoringFailure = Future<void>.value();
  Future<void>? _exiting;
  String? _workerSession;
  String? lastFailure;
  int? _lastExitCode;
  String nativeOutput = '';
  String _stopReason = 'Sharing stopped by the user';
  Process? _proc;
  int? _activePort;
  Future<void>? _starting;
  Future<void>? _stopping;
  Future<void> _reportingStopped = Future<void>.value();
  Timer? _resourceMonitor;
  int _epoch = 0;
  NodeClient? _reporter;
  DeviceProfile? _profile;
  final state = ''.obs;
  final sharing = false.obs;
  final joining = false.obs;

  bool get isRunning => state.value == 'running' && _proc != null;
  int get activePort => _activePort ?? defaultPort;
  bool get isBusy =>
      state.value == 'starting' || state.value == 'stopping' || joining.value;

  Future<String?> localIp() => _channel
      .invokeMethod<String>('localIp')
      .timeout(const Duration(seconds: 3));

  Future<void> start({String? allowedPeer}) async {
    final requestedEpoch = _epoch;
    if (_stopping != null) await _stopping;
    if (_exiting != null) await _exiting;
    await _restoringFailure;
    if (_starting != null) return _starting!;
    if (requestedEpoch != _epoch) throw StateError('worker start cancelled');
    if (isRunning) return;
    final pending = _start(++_epoch, allowedPeer);
    _starting = pending;
    try {
      await pending;
    } finally {
      if (identical(_starting, pending)) _starting = null;
    }
  }

  Future<void> _start(int epoch, String? allowedPeer) async {
    state.value = 'starting';
    lastFailure = null;
    _lastExitCode = null;
    nativeOutput = '';
    _stopReason = 'Sharing stopped by the user';
    try {
      await _checkResources(starting: true);
      await _beginProtection();
      final libDir = await _channel
          .invokeMethod<String>('nativeLibDir')
          .timeout(const Duration(seconds: 3));
      if (libDir == null) throw StateError('nativeLibDir unavailable');
      final cacheDir = await _channel
          .invokeMethod<String>('cacheDir')
          .timeout(const Duration(seconds: 3));
      if (cacheDir == null) throw StateError('cacheDir unavailable');
      if (epoch != _epoch) throw StateError('worker start cancelled');
      final profile = DeviceProfile.detect(name: 'this phone');
      final failures = <String>[];

      for (var candidate = defaultPort;
          candidate <= _lastFallbackPort;
          candidate++) {
        if (epoch != _epoch) throw StateError('worker start cancelled');
        final occupied = await _probe(candidate);
        if (epoch != _epoch) throw StateError('worker start cancelled');
        if (occupied) {
          failures.add('port $candidate is already occupied');
          continue;
        }

        var lastOutput = '';
        final process = await _launch(
          '$libDir/libggmlrpc.so',
          [
            '--host',
            '0.0.0.0',
            '--port',
            '$candidate',
            if (allowedPeer != null) ...[
              '--allow-peer',
              allowedPeer,
              '--upstream-port',
              '${candidate + 10000}',
            ],
            '-t',
            '${profile.threads}',
            '--device',
            'CPU',
            '--cache',
          ],
          libDir,
          {'LLAMA_CACHE': cacheDir},
        );
        if (epoch != _epoch) {
          await _terminate(process);
          throw StateError('worker start cancelled');
        }

        _proc = process;
        _activePort = candidate;
        _workerSession = DeviceProfile.createId();
        if (_androidProtection) {
          await _channel
              .invokeMethod<void>('trackWorker', {'pid': process.pid});
        }
        void capture(String text) {
          lastOutput += text;
          if (lastOutput.length > 4096) {
            lastOutput = lastOutput.substring(lastOutput.length - 4096);
          }
          if (identical(_proc, process)) nativeOutput = lastOutput;
        }

        final stdoutDone = Completer<void>();
        process.stdout
            .transform(const Utf8Decoder(allowMalformed: true))
            .listen(capture,
                onError: (_) {}, onDone: () => stdoutDone.complete());
        final stderrDone = Completer<void>();
        process.stderr
            .transform(const Utf8Decoder(allowMalformed: true))
            .listen(capture,
                onError: (_) {}, onDone: () => stderrDone.complete());

        int? exitCode;
        final exit = process.exitCode;
        unawaited(exit.then((code) => exitCode = code));
        final startupTime = Stopwatch()..start();
        while (startupTime.elapsed < const Duration(seconds: 10) &&
            exitCode == null) {
          if (epoch != _epoch) {
            await _terminate(process);
            throw StateError('worker start cancelled');
          }
          final ready = await _probe(candidate);
          if (epoch != _epoch) throw StateError('worker start cancelled');
          if (exitCode != null || !identical(_proc, process)) break;
          if (ready) {
            state.value = 'running';
            await _clearSavedFailure();
            unawaited(exit.then((code) async {
              if (identical(_proc, process)) {
                final pending = _workerExited(
                    process,
                    code,
                    Future.wait([stdoutDone.future, stderrDone.future]),
                    () => lastOutput);
                _exiting = pending;
                try {
                  await pending;
                } finally {
                  if (identical(_exiting, pending)) _exiting = null;
                }
              }
            }));
            return;
          }
          await Future<void>.delayed(const Duration(milliseconds: 50));
        }

        if (identical(_proc, process)) {
          _proc = null;
          _activePort = null;
        }
        if (exitCode == null) {
          await _terminate(process);
          await _clearTrackedProcess(process.pid);
          failures.add('port $candidate did not become ready');
          continue;
        }

        try {
          await stderrDone.future.timeout(const Duration(milliseconds: 250));
        } catch (_) {
          // The exit code is authoritative; output is best-effort diagnostics.
        }
        final detail = lastOutput.trim();
        await _clearTrackedProcess(process.pid);
        if (detail.contains('Failed to create server socket')) {
          failures.add('port $candidate could not be bound');
          continue;
        }
        throw StateError(
          'worker exited ($exitCode)${detail.isEmpty ? '' : ': $detail'}',
        );
      }

      throw StateError(
        'No usable RPC port in $defaultPort-$_lastFallbackPort. '
        '${failures.join('; ')}',
      );
    } catch (e) {
      final process = _proc;
      _proc = null;
      _activePort = null;
      if (process != null) await _terminate(process);
      await _endProtection(process?.pid);
      if (epoch == _epoch) {
        lastFailure = e.toString();
        state.value = 'error: $e';
        await _saveFailure();
      }
      rethrow;
    }
  }

  Future<void> startSharing(PairPayload pair, DeviceProfile profile) async {
    if (joining.value) return;
    if (sharing.value && _reporter?.pair.toUri() == pair.toUri()) return;
    joining.value = true;
    var epoch = _epoch;
    try {
      String? allowedPeer;
      if (_androidProtection) {
        final addresses = await InternetAddress.lookup(pair.host,
            type: InternetAddressType.IPv4);
        allowedPeer = addresses.firstOrNull?.address;
        if (allowedPeer == null) {
          throw StateError('Could not resolve the paired laptop to IPv4');
        }
      }
      await start(allowedPeer: allowedPeer);
      epoch = _epoch;
      final ip = await localIp();
      if (ip == null || ip.isEmpty) {
        throw StateError('Could not determine the phone LAN address');
      }
      if (epoch != _epoch || !isRunning) {
        throw StateError('Worker stopped before joining');
      }
      await _stopReporting();
      if (epoch != _epoch || !isRunning) {
        throw StateError('Worker stopped before joining');
      }
      final reporter = _nodeClient(pair);
      _reporter = reporter;
      final runtimeProfile = DeviceProfile.detect(
        name: profile.name,
        id: profile.id,
      );
      _profile = runtimeProfile;
      _workerSession = DeviceProfile.createId();
      var rpc = '$ip:$activePort';
      final accepted = await reporter.reportSelf(runtimeProfile,
          rpc: rpc, worker: _workerReport('running'));
      if (epoch != _epoch || !isRunning) {
        throw StateError('Worker stopped before joining');
      }
      if (!accepted) {
        throw StateError(reporter.lastPeerError ??
            'The laptop did not accept this device. Check pairing.');
      }
      reporter.startHeartbeat(runtimeProfile,
          rpcProvider: () async {
            if (epoch != _epoch || !isRunning) return null;
            final address = await localIp();
            if (epoch != _epoch || !isRunning) return null;
            if (address != null && address.isNotEmpty) {
              rpc = '$address:$activePort';
            }
            return rpc;
          },
          workerReport: () => _workerReport('running'));
      sharing.value = true;
      _resourceMonitor?.cancel();
      _resourceMonitor = Timer.periodic(const Duration(seconds: 15), (_) {
        unawaited(_checkResources().catchError((_) {}));
      });
    } catch (_) {
      if (epoch == _epoch) await stop();
      rethrow;
    } finally {
      joining.value = false;
    }
  }

  Future<void> stop() {
    return _stopping ??= _stop().whenComplete(() => _stopping = null);
  }

  Future<void> _stop() async {
    _resourceMonitor?.cancel();
    _resourceMonitor = null;
    final epoch = ++_epoch;
    final process = _proc;
    _proc = null;
    _activePort = null;
    final reporting = _stopReporting();
    state.value = 'stopping';
    if (process != null) await _terminate(process);
    try {
      await _starting;
    } catch (_) {/* A cancelled startup is expected. */}
    await reporting;
    await _endProtection(process?.pid);
    if (epoch == _epoch) state.value = 'stopped';
  }

  Future<void> _stopReporting() {
    final reporter = _reporter;
    final profile = _profile;
    _reporter = null;
    _profile = null;
    sharing.value = false;
    final heartbeatStopped = reporter?.stopHeartbeat();
    if (reporter == null) return _reportingStopped;
    final previous = _reportingStopped;
    final report = _workerSession == null
        ? null
        : _workerReport(lastFailure == null ? 'stopped' : 'error',
            reason: lastFailure ?? _stopReason);
    // An old withdrawal must finish before a new registration. Otherwise a
    // quick off/on toggle can erase the new worker address on the laptop.
    _reportingStopped = () async {
      await previous;
      await heartbeatStopped;
      try {
        if (profile != null) await reporter.reportSelf(profile, worker: report);
      } finally {
        reporter.dispose();
      }
    }();
    return _reportingStopped;
  }

  Map<String, dynamic> _workerReport(String workerState, {String? reason}) => {
        'session': _workerSession!,
        'state': workerState,
        if (_lastExitCode != null) 'exit_code': _lastExitCode,
        if (reason != null) 'error': _reportText(reason),
        if (nativeOutput.isNotEmpty) 'log_tail': _reportText(nativeOutput),
        if (_proc != null) 'releasable_mb': _processAnonymousMb(_proc!.pid),
        if (_proc != null) 'rss_mb': _processRssMb(_proc!.pid),
      };

  static int _processAnonymousMb(int pid) {
    try {
      final status = File('/proc/$pid/status').readAsStringSync();
      final line =
          status.split('\n').firstWhere((line) => line.startsWith('RssAnon:'));
      return (int.tryParse(line.split(RegExp(r'\s+'))[1]) ?? 0) ~/ 1024;
    } catch (_) {
      return 0;
    }
  }

  static int? _processRssMb(int pid) {
    try {
      final status = File('/proc/$pid/status').readAsStringSync();
      final line =
          status.split('\n').firstWhere((line) => line.startsWith('VmRSS:'));
      return int.parse(line.split(RegExp(r'\s+'))[1]) ~/ 1024;
    } catch (_) {
      return null;
    }
  }

  Future<void> _checkResources({bool starting = false}) async {
    if (!_androidProtection) return;
    final value = await _channel
        .invokeMapMethod<String, dynamic>('powerState')
        .timeout(const Duration(seconds: 3));
    if (value == null) return;
    final thermal = value['thermal_status'] as int? ?? 0;
    final battery = value['battery_percent'] as int? ?? -1;
    final powered = value['powered'] == true;
    final reason = thermal >= 3
        ? 'Phone sharing stopped because Android reported severe thermal pressure.'
        : (!powered && battery >= 0 && battery <= (starting ? 15 : 10))
            ? 'Phone sharing stopped to protect a low battery. Connect power and try again.'
            : null;
    if (reason == null) return;
    lastFailure = reason;
    _stopReason = reason;
    if (!starting && isRunning) await stop();
    throw StateError(reason);
  }

  static String _reportText(String text) =>
      text.length <= 2048 ? text : text.substring(text.length - 2048);

  Future<void> _workerExited(Process process, int code, Future<void> outputDone,
      String Function() output) async {
    _proc = null;
    _activePort = null;
    _lastExitCode = code;
    try {
      await outputDone.timeout(const Duration(milliseconds: 250));
    } catch (_) {
      /* Exit status remains authoritative if a pipe fails to close. */
    }
    nativeOutput = output().trim();
    final signal = code < 0
        ? -code
        : code >= 128
            ? code - 128
            : null;
    final hint = switch (signal) {
      9 =>
        'SIGKILL: Android memory pressure, background restrictions, or a force-stop may have killed it.',
      6 => 'SIGABRT: the native worker aborted; inspect its saved output.',
      11 => 'SIGSEGV: the native worker crashed; inspect its saved output.',
      13 => 'SIGPIPE: the RPC transport closed unexpectedly.',
      _ => 'The loaded model must be reloaded after restarting sharing.',
    };
    lastFailure = 'Phone worker exited ($code). $hint';
    state.value = 'error: $lastFailure';
    final reporting = _stopReporting();
    await _saveFailure();
    await reporting;
    await _endProtection(process.pid);
  }

  Future<void> _beginProtection() async {
    if (!_androidProtection || _protected) return;
    await _protectionClosing;
    _protected = true;
    await _channel
        .invokeMethod<void>('startWorkerProtection')
        .timeout(const Duration(seconds: 5));
  }

  Future<void> _clearTrackedProcess(int pid) async {
    if (!_androidProtection) return;
    await _channel.invokeMethod<void>('clearWorker', {'pid': pid});
  }

  Future<void> _endProtection(int? pid) {
    if (!_androidProtection || !_protected) return _protectionClosing;
    _protected = false;
    return _protectionClosing = () async {
      if (pid != null) await _clearTrackedProcess(pid);
      await _channel.invokeMethod<void>('stopWorkerProtection');
    }();
  }

  Future<File?> _failureFile() async {
    if (!_androidProtection) return null;
    final directory = await _channel.invokeMethod<String>('workerLogDir');
    return directory == null
        ? null
        : File('$directory/worker-last-failure.json');
  }

  Future<void> _saveFailure() async {
    try {
      await (await _failureFile())?.writeAsString(jsonEncode({
        'error': lastFailure,
        'exit_code': _lastExitCode,
        'output': nativeOutput,
        'time': DateTime.now().toUtc().toIso8601String(),
      }));
    } catch (_) {
      /* Coordinator reporting still works if local storage is full. */
    }
  }

  Future<void> _restoreFailure() async {
    try {
      final file = await _failureFile();
      if (file == null || !await file.exists()) return;
      final saved = jsonDecode(await file.readAsString()) as Map;
      lastFailure = saved['error'] as String?;
      nativeOutput = saved['output'] as String? ?? '';
      _lastExitCode = saved['exit_code'] as int?;
      if (lastFailure != null) state.value = 'error: $lastFailure';
    } catch (_) {
      /* A missing or incomplete old report must not block startup. */
    }
  }

  Future<void> _clearSavedFailure() async {
    try {
      final file = await _failureFile();
      if (file != null && await file.exists()) await file.delete();
    } catch (_) {}
  }

  static Future<void> _terminate(Process process) async {
    process.kill();
    await process.exitCode.timeout(const Duration(seconds: 2), onTimeout: () {
      process.kill(ProcessSignal.sigkill);
      return process.exitCode;
    });
  }

  static Future<bool> _workerReachable(int port) async {
    try {
      final socket = await Socket.connect('127.0.0.1', port,
          timeout: const Duration(milliseconds: 200));
      socket.destroy();
      return true;
    } catch (_) {
      return false;
    }
  }

  @override
  void onInit() {
    super.onInit();
    WidgetsBinding.instance.addObserver(this);
    _restoringFailure = _restoreFailure();
    if (_androidProtection) {
      _channel.setMethodCallHandler((call) async {
        if (call.method == 'stopSharing') {
          _stopReason =
              'Sharing stopped from Android notification or app task removal';
          await stop();
        }
      });
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.detached) unawaited(stop());
  }

  @override
  void onClose() {
    _resourceMonitor?.cancel();
    WidgetsBinding.instance.removeObserver(this);
    if (_androidProtection) _channel.setMethodCallHandler(null);
    unawaited(stop());
    super.onClose();
  }
}
