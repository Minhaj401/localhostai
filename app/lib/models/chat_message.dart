class ChatMessage {
  ChatMessage({
    required this.role,
    required this.content,
    this.backend,
    this.model,
    this.interrupted = false,
  });

  final String role; // 'user' | 'assistant' | 'system'
  final String content;
  final String? backend; // which engine produced it (for the UI badge)
  final String? model;
  final bool interrupted;

  Map<String, dynamic> toJson() => {
        'role': role,
        'content': content,
        if (backend != null) 'backend': backend,
        if (model != null) 'model': model,
        if (interrupted) 'interrupted': true,
      };

  factory ChatMessage.fromJson(Map<String, dynamic> json) => ChatMessage(
        role: json['role'] as String,
        content: json['content'] as String,
        backend: json['backend'] as String?,
        model: json['model'] as String?,
        interrupted: json['interrupted'] == true,
      );
}
