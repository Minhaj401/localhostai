import 'dart:async';
import 'dart:io';
import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:localhostai/models/device_profile.dart';
import 'package:localhostai/models/pairing.dart';
import 'package:localhostai/services/node_client.dart';
import 'package:localhostai/services/phone_worker_service.dart';

class FakeProcess implements Process {
  FakeProcess({this.stderrText = ''});

  final String stderrText;
  final exited = Completer<int>();
  bool killed = false;
  @override
  Future<int> get exitCode => exited.future;
  @override
  int get pid => 123;
  @override
  Stream<List<int>> get stdout => const Stream.empty();
  @override
  Stream<List<int>> get stderr => stderrText.isEmpty
      ? const Stream.empty()
      : Stream.value(utf8.encode(stderrText));
  @override
  IOSink get stdin => throw UnimplementedError();
  @override
  bool kill([ProcessSignal signal = ProcessSignal.sigterm]) {
    killed = true;
    if (!exited.isCompleted) exited.complete(0);
    return true;
  }
}

class LateOutputProcess extends FakeProcess {
  final output = StreamController<List<int>>();
  @override
  Stream<List<int>> get stderr => output.stream;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel('localhostai/native');
  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
            channel,
            (call) async => switch (call.method) {
                  'nativeLibDir' => '/native',
                  'cacheDir' => '/cache',
                  _ => '10.0.0.2',
                });
  });
  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, null);
  });

  test('concurrent starts launch one process and worker death is observed',
      () async {
    final launching = Completer<Process>();
    var launches = 0;
    var probes = 0;
    final service = PhoneWorkerService(
      launch: (_, args, __, ___) {
        launches++;
        expect(args, contains('CPU'));
        expect(args, contains('--cache'));
        return launching.future;
      },
      probe: (_) async => probes++ > 0,
    );
    final first = service.start();
    final second = service.start();
    await Future<void>.delayed(Duration.zero);
    final process = FakeProcess();
    launching.complete(process);
    await Future.wait([first, second]);
    expect(launches, 1);
    expect(service.isRunning, isTrue);
    process.exited.complete(1);
    await Future<void>.delayed(Duration.zero);
    expect(service.isRunning, isFalse);
    expect(service.state.value, contains('exited (1)'));
    await service.stop();
  });

  test('stopping while process launch is pending cannot leave an orphan',
      () async {
    final launching = Completer<Process>();
    final service = PhoneWorkerService(
        launch: (_, __, ___, ____) => launching.future,
        probe: (_) async => false);
    final starting = service.start();
    final rejected = expectLater(starting, throwsStateError);
    await Future<void>.delayed(Duration.zero);
    final stopping = service.stop();
    final process = FakeProcess();
    launching.complete(process);
    await rejected;
    await stopping;
    expect(process.killed, isTrue);
    expect(service.isRunning, isFalse);
    expect(service.state.value, 'stopped');
  });

  test('a successful readiness probe cannot resurrect a stopped worker',
      () async {
    final probing = Completer<void>();
    final ready = Completer<bool>();
    final process = FakeProcess();
    var probes = 0;
    final service = PhoneWorkerService(
      launch: (_, __, ___, ____) async => process,
      probe: (_) async {
        if (probes++ == 0) return false;
        probing.complete();
        return ready.future;
      },
    );
    final states = <String>[];
    final subscription = service.state.listen(states.add);
    final starting = service.start();
    final rejected = expectLater(starting, throwsStateError);
    await probing.future;
    final stopping = service.stop();
    ready.complete(true);
    await rejected;
    await stopping;
    expect(process.killed, isTrue);
    expect(service.isRunning, isFalse);
    expect(states, isNot(contains('running')));
    await subscription.cancel();
  });

  test('stop during port preflight prevents a later process launch', () async {
    final probing = Completer<void>();
    final occupied = Completer<bool>();
    var launches = 0;
    final service = PhoneWorkerService(
      launch: (_, __, ___, ____) async {
        launches++;
        return FakeProcess();
      },
      probe: (_) {
        probing.complete();
        return occupied.future;
      },
    );
    final starting = service.start();
    final rejected = expectLater(starting, throwsStateError);
    await probing.future;
    final stopping = service.stop();
    occupied.complete(false);
    await rejected;
    await stopping;
    expect(launches, 0);
  });

  test('an occupied port is not mistaken for a successfully launched worker',
      () async {
    var launches = 0;
    final service = PhoneWorkerService(
      launch: (_, __, ___, ____) async {
        launches++;
        return FakeProcess();
      },
      probe: (_) async => true,
    );
    await expectLater(service.start(), throwsStateError);
    expect(launches, 0);
    expect(service.isRunning, isFalse);
  });

  test('rapid stop/restart cannot withdraw a newer worker registration',
      () async {
    final withdrawal = Completer<void>();
    FakeProcess? process;
    var registrations = 0;
    final service = PhoneWorkerService(
      launch: (_, __, ___, ____) async => process = FakeProcess(),
      probe: (_) async => process != null && !process!.exited.isCompleted,
      nodeClient: (pair) =>
          NodeClient(pair, client: MockClient((request) async {
        final body = jsonDecode(request.body) as Map;
        if (body['rpc'] != null) {
          registrations++;
        } else {
          await withdrawal.future;
        }
        return http.Response('{"ok":true}', 200);
      })),
    );
    const pair =
        PairPayload(host: 'laptop', port: 8765, token: 't', name: 'laptop');
    final profile = DeviceProfile.detect(name: 'phone', id: 'stable-id');
    await service.startSharing(pair, profile);
    final stopping = service.stop();
    final restarting = service.startSharing(pair, profile);
    await Future<void>.delayed(Duration.zero);
    expect(registrations, 1);
    withdrawal.complete();
    await stopping;
    await restarting;
    expect(registrations, 2);
    expect(service.sharing.value, isTrue);
    await service.stop();
  });

  test('occupied default port falls back and launches from nativeLibDir',
      () async {
    FakeProcess? process;
    List<String>? launchedArgs;
    String? launchedDirectory;
    Map<String, String>? launchedEnvironment;
    final service = PhoneWorkerService(
      launch: (_, args, workingDirectory, environment) async {
        launchedArgs = args;
        launchedDirectory = workingDirectory;
        launchedEnvironment = environment;
        return process = FakeProcess();
      },
      probe: (port) async =>
          port == PhoneWorkerService.defaultPort ||
          (port == PhoneWorkerService.defaultPort + 1 &&
              process != null &&
              !process!.exited.isCompleted),
    );
    await service.start();
    expect(service.activePort, PhoneWorkerService.defaultPort + 1);
    expect(launchedArgs, contains('${PhoneWorkerService.defaultPort + 1}'));
    expect(launchedArgs, contains('--cache'));
    expect(launchedDirectory, '/native');
    expect(launchedEnvironment, {'LLAMA_CACHE': '/cache'});
    expect(service.isRunning, isTrue);
    await service.stop();
  });

  test('bind race retries the next port even when rpc-server exits zero',
      () async {
    var launches = 0;
    FakeProcess? running;
    final service = PhoneWorkerService(
      launch: (_, __, ___, ____) async {
        launches++;
        if (launches == 1) {
          final failed =
              FakeProcess(stderrText: 'Failed to create server socket\n');
          failed.exited.complete(0);
          return failed;
        }
        return running = FakeProcess();
      },
      probe: (port) async =>
          port == PhoneWorkerService.defaultPort + 1 &&
          running != null &&
          !running!.exited.isCompleted,
    );
    await service.start();
    expect(launches, 2);
    expect(service.activePort, PhoneWorkerService.defaultPort + 1);
    expect(service.isRunning, isTrue);
    await service.stop();
  });

  test(
      'foreground protection covers the worker lifetime and is released on stop',
      () async {
    final calls = <String>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
      calls.add(call.method);
      return switch (call.method) {
        'nativeLibDir' => '/native',
        'cacheDir' => '/cache',
        'localIp' => '10.0.0.2',
        _ => null,
      };
    });
    FakeProcess? process;
    final service = PhoneWorkerService(
      androidProtection: true,
      launch: (_, __, ___, ____) async {
        expect(calls, contains('startWorkerProtection'));
        return process = FakeProcess();
      },
      probe: (_) async => process != null && !process!.exited.isCompleted,
    );
    await service.start();
    expect(calls, contains('trackWorker'));
    expect(calls, isNot(contains('stopWorkerProtection')));
    await service.stop();
    expect(process!.killed, isTrue);
    expect(calls.indexOf('clearWorker'),
        lessThan(calls.indexOf('stopWorkerProtection')));
    expect(calls.where((call) => call == 'stopWorkerProtection'), hasLength(1));
  });

  test(
      'cancel while Android protection starts releases it without launching a worker',
      () async {
    final protecting = Completer<void>();
    final protected = Completer<void>();
    var released = false;
    var launches = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
      if (call.method == 'startWorkerProtection') {
        protecting.complete();
        await protected.future;
      }
      if (call.method == 'stopWorkerProtection') released = true;
      return switch (call.method) {
        'nativeLibDir' => '/native',
        'cacheDir' => '/cache',
        _ => null,
      };
    });
    final service = PhoneWorkerService(
        androidProtection: true,
        launch: (_, __, ___, ____) async {
          launches++;
          return FakeProcess();
        });
    final start = service.start();
    final failure = expectLater(start, throwsStateError);
    await protecting.future;
    final stop = service.stop();
    protected.complete();
    await failure;
    await stop;
    expect(launches, 0);
    expect(released, isTrue);
  });

  test('Android protection denial never leaves a native worker behind',
      () async {
    var released = false;
    var launches = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
      if (call.method == 'startWorkerProtection') {
        throw PlatformException(
            code: 'not_foreground', message: 'Open the app to share');
      }
      if (call.method == 'stopWorkerProtection') released = true;
      return null;
    });
    final service = PhoneWorkerService(
        androidProtection: true,
        launch: (_, __, ___, ____) async {
          launches++;
          return FakeProcess();
        });
    await expectLater(service.start(), throwsA(isA<PlatformException>()));
    expect(launches, 0);
    expect(released, isTrue);
    expect(service.lastFailure, contains('Open the app'));
  });

  test(
      'worker death reports the exit signal and final pipe output in the same session',
      () async {
    LateOutputProcess? process;
    final reports = <Map<String, dynamic>>[];
    final withdrawn = Completer<void>();
    final service = PhoneWorkerService(
      launch: (_, __, ___, ____) async => process = LateOutputProcess(),
      probe: (_) async => process != null && !process!.exited.isCompleted,
      nodeClient: (pair) =>
          NodeClient(pair, client: MockClient((request) async {
        final value = jsonDecode(request.body) as Map<String, dynamic>;
        reports.add(value);
        if (value['rpc'] == null) withdrawn.complete();
        return http.Response('{"ok":true}', 200);
      })),
    );
    const pair =
        PairPayload(host: 'laptop', port: 8765, token: 't', name: 'laptop');
    await service.startSharing(pair, DeviceProfile.detect(name: 'phone'));
    process!.exited.complete(-9);
    await Future<void>.delayed(Duration.zero);
    process!.output.add(utf8.encode('final native allocation diagnostic'));
    await process!.output.close();
    await withdrawn.future;
    expect(service.lastFailure, contains('SIGKILL'));
    expect(service.sharing.value, isFalse);
    final first = reports.first['worker'] as Map;
    final last = reports.last['worker'] as Map;
    expect(last['session'], first['session']);
    expect(last['sequence'], greaterThan(first['sequence'] as int));
    expect(last['exit_code'], -9);
    expect(last['state'], 'error');
    expect(last['log_tail'], contains('final native allocation diagnostic'));
    await service.stop();
  });
}
