import 'chat_message.dart';

class ConversationSummary {
  ConversationSummary.fromJson(Map<String, dynamic> value)
      : id = value['id'] as String,
        title = value['title'] as String,
        revision = (value['revision'] as num).toInt(),
        messageCount = (value['message_count'] as num?)?.toInt() ?? 0,
        generating = value['generating'] == true,
        preview = value['preview'] as String? ?? '';
  final String id, title, preview;
  final int revision, messageCount;
  final bool generating;
}

class Conversation {
  Conversation.fromJson(Map<String, dynamic> value)
      : id = value['id'] as String,
        title = value['title'] as String,
        revision = (value['revision'] as num).toInt(),
        model = value['model'] as String?,
        contextTokens = (value['ctx'] as num).toInt(),
        systemPrompt = value['system_prompt'] as String? ?? '',
        activeRequest = value['active_request'] as String?,
        messages = (value['messages'] as List)
            .map((m) => ChatMessage.fromJson({
                  ...Map<String, dynamic>.from(m as Map),
                  'generating': value['active_request'] != null &&
                      m['id'] == '${value['active_request']}-assistant'
                }))
            .toList(),
        json = value;
  final String id, title, systemPrompt;
  final String? model, activeRequest;
  final int revision, contextTokens;
  final List<ChatMessage> messages;
  final Map<String, dynamic> json;
}
