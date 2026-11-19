import 'dart:io';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

class ConversationExport {
  static Future<String> save(Map<String, dynamic> export) async {
    final filename = export['filename'] as String;
    final content = export['content'] as String;
    if (!RegExp(r'^[a-zA-Z0-9_.-]+$').hasMatch(filename)) {
      throw const FormatException('Invalid export filename');
    }
    if (Platform.isAndroid) {
      final saved = await const MethodChannel('localhostai/native')
          .invokeMethod<bool>('exportConversation', {
        'filename': filename,
        'content': content,
        'mime': export['mime'],
      });
      return saved == true ? 'Conversation exported.' : 'Export cancelled.';
    }
    final directory = await getApplicationDocumentsDirectory();
    final file = File(
        '${directory.path}/${DateTime.now().millisecondsSinceEpoch}-$filename');
    await file.writeAsString(content, flush: true);
    return 'Exported to ${file.path}';
  }
}
