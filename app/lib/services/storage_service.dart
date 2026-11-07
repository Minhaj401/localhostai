import 'package:hive_flutter/hive_flutter.dart';

/// Persists settings and chat history locally (nothing leaves the device
/// except what the chosen backend needs).
class StorageService {
  static const _settingsBox = 'settings';
  static const _chatBox = 'chat';

  Future<void> init() async {
    await Hive.initFlutter();
    await Hive.openBox(_settingsBox);
    await Hive.openBox(_chatBox);
  }

  String? getString(String key) => Hive.box(_settingsBox).get(key) as String?;
  Future<void> setString(String key, String value) =>
      Hive.box(_settingsBox).put(key, value);

  List<Map<String, dynamic>> loadMessages() {
    final raw = Hive.box(_chatBox).get('messages') as List? ?? const [];
    return raw.cast<Map>().map((m) => Map<String, dynamic>.from(m)).toList();
  }

  Future<void> saveMessages(List<Map<String, dynamic>> messages) =>
      Hive.box(_chatBox).put('messages', messages);

  Future<void> clearMessages() => Hive.box(_chatBox).delete('messages');
}
