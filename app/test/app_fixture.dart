import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:privatelm_lan/features/connection/connection_controller.dart';
import 'package:privatelm_lan/models/device_profile.dart';
import 'package:privatelm_lan/models/pairing.dart';
import 'package:privatelm_lan/services/node_client.dart';
import 'package:privatelm_lan/services/phone_worker_service.dart';

import 'support.dart';

class UiWorker extends PhoneWorkerService {
  int stops = 0;
  int starts = 0;

  @override
  Future<void> stop() async {
    stops++;
    sharing.value = false;
    state.value = 'stopped';
  }

  @override
  Future<void> startSharing(PairPayload pair, DeviceProfile profile) async {
    starts++;
    sharing.value = true;
    state.value = 'running';
  }
}

class AppFixture {
  final storage = MemoryStorage();
  final worker = UiWorker();
  final requests = <http.Request>[];
  bool online = true;
  bool installed = true;
  bool fits = true;
  bool customAvailable = false;
  bool customInstalled = false;
  String state = 'running';
  Future<http.Response> Function(http.Request)? intercept;

  static const pair = PairPayload(
      host: '10.0.0.1', port: 8765, token: 'MixedCaseToken', name: 'Laptop');
  static const modelId = 'qwen2.5-0.5b-instruct-q4_k_m';
  static const customId = 'custom-aaaaaaaaaaaaaaaaaaaaaaaa';
  static const customModel = {
    'id': customId,
    'display_name': 'Community model Q4_K_M',
    'description': 'example/new-model',
    'bytes': 1234567890,
    'license': 'apache-2.0',
    'files': [
      {'filename': 'community.Q4_K_M.gguf'}
    ],
    'source': {
      'kind': 'hugging_face',
      'repository': 'example/new-model',
      'filename': 'community.Q4_K_M.gguf',
      'revision': 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    },
  };

  NodeClient client(PairPayload pair) =>
      NodeClient(pair, client: MockClient((request) async {
        requests.add(request);
        if (intercept != null) return intercept!(request);
        return respond(request);
      }));

  http.Response respond(http.Request request) {
    if (!online) return http.Response('{"error":"offline"}', 503);
    Object value;
    switch (request.url.path) {
      case '/health':
        value = {
          'ok': state == 'running',
          'name': 'Laptop',
          'backend': state == 'running' ? 'distributed' : 'none',
          'models': state == 'running' ? [modelId] : [],
          'pairing': true,
          'version': '0.2.1'
        };
      case '/pair/verify':
        value = {'ok': true};
      case '/managed':
        value = {
          'runtime_ready': true,
          'status': {'phase': 'ready', 'model': modelId},
          'catalog': [
            {
              'id': modelId,
              'display_name': 'Qwen 2.5 0.5B Instruct',
              'description': 'Small first model.',
              'bytes': 491400032,
              'layers': 24,
              'params_b': 0.5,
              'license': 'Apache-2.0',
              'installed': installed,
              'recommended': true
            },
            {
              'id': 'qwen2.5-coder-7b-instruct-q4_k_m',
              'display_name': 'Qwen 2.5 Coder 7B Instruct',
              'description': 'A larger coding assistant.',
              'bytes': 4683073536,
              'layers': 28,
              'params_b': 7.6,
              'license': 'Apache-2.0',
              'installed': false,
              'recommended': false
            },
          ],
          'custom': [
            if (customAvailable)
              {...customModel, 'custom': true, 'installed': customInstalled},
          ],
        };
      case '/models':
        value = {
          'models': [
            if (installed) {'name': modelId, 'size_mb': 469, 'layers': 24},
            if (customInstalled)
              {'name': customId, 'size_mb': 1178, 'layers': 28},
          ],
        };
      case '/models/browse':
        final query = (jsonDecode(request.body) as Map)['query'];
        value = query == 'example/new-model' || '$query'.startsWith('https://')
            ? {
                'repository': 'example/new-model',
                'models': [customModel],
                'repositories': []
              }
            : {
                'repositories': [
                  {'id': 'example/new-model', 'downloads': 1234}
                ],
                'models': []
              };
      case '/managed/download':
        customAvailable = true;
        customInstalled = true;
        value = {
          'ok': true,
          'model': {'name': customId}
        };
      case '/cluster':
        value = {
          'state': state,
          'model': state == 'stopped' ? null : modelId,
          'log': '/home/user/.privateml/logs/cluster.log',
          'error': null,
          'assignments': []
        };
      case '/peers':
        value = {
          'devices': [
            {
              'id': 'laptop',
              'name': 'Laptop',
              'kind': 'laptop',
              'ram_mb': 7720,
              'available_mb': 3000,
              'budget_mb': 1600,
              'cores': 8,
              'threads': 3,
              'self': true,
              'reachable': true,
              'usable': true,
              'assigned_layers': state == 'running' ? 21 : 0,
              'assigned_memory_mb': state == 'running' ? 420 : 0
            },
            {
              'id': 'phone',
              'name': 'A phone with a deliberately long device name',
              'kind': 'phone',
              'ram_mb': 7494,
              'available_mb': 3100,
              'budget_mb': 2200,
              'cores': 8,
              'threads': 3,
              'self': false,
              'reachable': true,
              'usable': true,
              'enabled': true,
              'heartbeat_seconds': 4,
              'assigned_layers': state == 'running' ? 3 : 0,
              'assigned_memory_mb': state == 'running' ? 127 : 0,
              'in_use': state == 'running'
            },
          ]
        };
      case '/cluster/plan':
        value = {'plan': plan(request)};
      case '/managed/install':
        if ((jsonDecode(request.body) as Map)['model'] == customId) {
          customInstalled = true;
        } else {
          installed = true;
        }
        value = {'ok': true};
      case '/cluster/start':
        state = 'running';
        value = {'ok': true, 'plan': plan(request)};
      case '/cluster/stop':
        state = 'stopped';
        value = {'ok': true};
      default:
        return http.Response('{"error":"unknown endpoint"}', 404);
    }
    return http.Response(jsonEncode(value), 200,
        headers: {'Content-Type': 'application/json'});
  }

  Map<String, dynamic> plan(http.Request request) {
    final body = jsonDecode(request.body) as Map;
    return {
      'model': body['model'],
      'ctx': body['ctx'],
      'total_layers': 24,
      'over_budget': !fits,
      'shortfall_mb': fits ? 0 : 500,
      'summary': fits
          ? 'Fits on Laptop alone.'
          : '500 MB short. Lower context or add a device.',
      'assignments': [
        {
          'device_name': 'Laptop',
          'layers': fits ? 24 : 10,
          'memory_mb': 500,
          'budget_mb': 1600
        }
      ]
    };
  }

  Future<ConnectionController> createController() async {
    await storage.savePair(pair);
    storage.settings['localhost_onboarding_complete'] = 'true';
    storage.settings['active_backend'] = 'laptop';
    final controller = ConnectionController(storage, worker,
        clientFactory: client, polling: false);
    await controller.restore();
    return controller;
  }
}
