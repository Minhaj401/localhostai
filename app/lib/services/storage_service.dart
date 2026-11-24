import 'dart:convert';

import 'package:hive_flutter/hive_flutter.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import '../models/device_profile.dart';
import '../models/pairing.dart';

/// Persists settings and chat history locally (nothing leaves the device
/// except what the chosen backend needs).
class StorageService {
  static const _settingsBox = 'settings';
  static const _chatBox = 'chat';
  static const _secretKeys = {
    'pair_payload',
    'pair_token'
  };
  final FlutterSecureStorage _secure = const FlutterSecureStorage(
    aOptions: AndroidOptions(encryptedSharedPreferences: true),
  );
  final Map<String, String> _secrets = {};

  Future<void> init() async {
    await Hive.initFlutter();
    await Hive.openBox(_settingsBox);
    await Hive.openBox(_chatBox);
    final settings = Hive.box(_settingsBox);
    for (final key in _secretKeys) {
      var value = await _secure.read(key: key);
      final legacy = settings.get(key) as String?;
      if ((value == null || value.isEmpty) &&
          legacy != null &&
          legacy.isNotEmpty) {
        value = legacy;
        await _secure.write(key: key, value: value);
      }
      if (value != null && value.isNotEmpty) _secrets[key] = value;
      if (settings.containsKey(key)) await settings.delete(key);
    }
    if (getString('device_id') == null) {
      await setString('device_id', DeviceProfile.createId());
    }
    final reserve = getReserveMb();
    if (reserve != null && reserve >= 0 && reserve <= 65536) {
      DeviceProfile.configuredReserveMb = reserve;
    }
  }

  String? getString(String key) => _secretKeys.contains(key)
      ? _secrets[key]
      : Hive.box(_settingsBox).get(key) as String?;
  Future<void> setString(String key, String value) async {
    if (_secretKeys.contains(key)) {
      if (value.isEmpty) {
        _secrets.remove(key);
        await _secure.delete(key: key);
      } else {
        _secrets[key] = value;
        await _secure.write(key: key, value: value);
      }
      return;
    }
    await Hive.box(_settingsBox).put(key, value);
  }

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
    final normalized = url.trim().replaceFirst(RegExp(r'/$'), '');
    final pair = loadPair();
    if (pair == null || pair.baseUrl != normalized) {
      await setString('pair_payload', '');
      await setString('pair_token', '');
    }
    await setString('laptop_node_url', normalized);
  }

  /// MiB this phone keeps free for Android and the app, overriding the 900
  /// default. Null restores the default.
  static const _reserveKey = 'phone_reserve_mb';

  int? getReserveMb() => Hive.box(_settingsBox).get(_reserveKey) as int?;

  Future<void> setReserveMb(int? mb) async {
    if (mb == null) {
      await Hive.box(_settingsBox).delete(_reserveKey);
    } else {
      await Hive.box(_settingsBox).put(_reserveKey, mb);
    }
  }

  List<Map<String, dynamic>> loadMessages() {
    final raw = Hive.box(_chatBox).get('messages') as List? ?? const [];
    return raw.cast<Map>().map((m) => Map<String, dynamic>.from(m)).toList();
  }

  Future<void> saveMessages(List<Map<String, dynamic>> messages) {
    const maximumBytes = 2 * 1024 * 1024;
    final retained = <Map<String, dynamic>>[];
    var bytes = 0;
    for (final message in messages.reversed) {
      var stored = message;
      var size = utf8.encode(jsonEncode(stored)).length;
      if (size > maximumBytes && retained.isEmpty) {
        final content = '${stored['content'] ?? ''}';
        stored = {
          ...stored,
          'content':
              '${content.substring(0, content.length.clamp(0, 256 * 1024))}\n\n[message truncated for local history storage]'
        };
        size = utf8.encode(jsonEncode(stored)).length;
      }
      if (retained.length >= 200 || bytes + size > maximumBytes) break;
      retained.add(stored);
      bytes += size;
    }
    return Hive.box(_chatBox).put('messages', retained.reversed.toList());
  }

  Future<void> clearMessages() => Hive.box(_chatBox).delete('messages');
}
