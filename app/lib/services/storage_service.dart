import 'dart:convert';

import 'package:hive_flutter/hive_flutter.dart';
import '../models/device_profile.dart';
import '../models/pairing.dart';

/// Persists settings and chat history locally (nothing leaves the device
/// except what the chosen backend needs).
class StorageService {
  static const _settingsBox = 'settings';
  static const _chatBox = 'chat';

  Future<void> init() async {
    await Hive.initFlutter();
    await Hive.openBox(_settingsBox);
    await Hive.openBox(_chatBox);
    if (getString('device_id') == null) {
      await setString('device_id', DeviceProfile.createId());
    }
  }

  String? getString(String key) => Hive.box(_settingsBox).get(key) as String?;
  Future<void> setString(String key, String value) =>
      Hive.box(_settingsBox).put(key, value);

  PairPayload? loadPair() {
    final raw = getString('pair_payload');
    if (raw == null || raw.isEmpty) return null;
    try {
      return PairPayload.fromJson(jsonDecode(raw) as Map<String, dynamic>);
    } catch (_) {
      return null;
    }
  }

  Future<void> savePair(PairPayload pair) async {
    await setString('pair_payload', jsonEncode(pair.toJson()));
    await setString('pair_token', pair.token);
    await setString('laptop_node_url', pair.baseUrl);
  }

  Future<void> selectNode(String url) async {
    final uri = Uri.tryParse(url.trim());
    if (uri == null ||
        !['http', 'https'].contains(uri.scheme) ||
        uri.host.isEmpty) {
      throw const FormatException('Enter a complete HTTP(S) node address');
    }
    await setNodeUrl(uri.toString());
  }

  Future<void> setNodeUrl(String url) async {
    final normalized = url.trim().replaceFirst(RegExp(r'/+$'), '');
    final pair = loadPair();
    if (pair == null || pair.baseUrl != normalized) {
      await setString('pair_payload', '');
      await setString('pair_token', '');
    }
    await setString('laptop_node_url', normalized);
  }

  List<Map<String, dynamic>> loadMessages() {
    final raw = Hive.box(_chatBox).get('messages') as List? ?? const [];
    return raw.cast<Map>().map((m) => Map<String, dynamic>.from(m)).toList();
  }

  Future<void> saveMessages(List<Map<String, dynamic>> messages) =>
      Hive.box(_chatBox).put('messages', messages);

  Future<void> clearMessages() => Hive.box(_chatBox).delete('messages');
}
