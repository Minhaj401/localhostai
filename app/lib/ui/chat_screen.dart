import 'package:flutter/material.dart';
import 'package:get/get.dart';

import '../controllers/chat_controller.dart';
import '../models/chat_message.dart';
import 'nodes_screen.dart';
import 'settings_screen.dart';

class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final controller = Get.put(ChatController(Get.find()));
  final inputController = TextEditingController();
  final scrollController = ScrollController();

  void _scrollToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (scrollController.hasClients) {
        scrollController.animateTo(
          scrollController.position.maxScrollExtent,
          duration: const Duration(milliseconds: 150),
          curve: Curves.easeOut,
        );
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Obx(() => Text(
              'PrivateLM-LAN · ${controller.backend.displayName}',
              style: const TextStyle(fontSize: 16),
            )),
        actions: [
          IconButton(
            tooltip: 'Laptop nodes',
            icon: const Icon(Icons.laptop_mac),
            onPressed: () => Get.to(() => const NodesScreen()),
          ),
          IconButton(
            tooltip: 'Settings',
            icon: const Icon(Icons.settings),
            onPressed: () => Get.to(() => const SettingsScreen()),
          ),
          IconButton(
            tooltip: 'New chat',
            icon: const Icon(Icons.refresh),
            onPressed: controller.clearChat,
          ),
        ],
      ),
      body: Column(
        children: [
          Obx(() {
            if (controller.error.value == null) return const SizedBox.shrink();
            return Material(
              color: Theme.of(context).colorScheme.errorContainer,
              child: ListTile(
                dense: true,
                leading: const Icon(Icons.error_outline),
                title: Text(controller.error.value!,
                    style: const TextStyle(fontSize: 13)),
                trailing: IconButton(
                  icon: const Icon(Icons.close, size: 18),
                  onPressed: () => controller.error.value = null,
                ),
              ),
            );
          }),
          Expanded(
            child: Obx(() {
              final msgs = controller.messages;
              final streaming =
                  controller.isGenerating.value && controller.streamingText.isNotEmpty;
              final children = <Widget>[
                for (final m in msgs) _bubble(m),
                if (streaming) _bubble(ChatMessage(
                  role: 'assistant',
                  content: controller.streamingText.value,
                  backend: 'streaming…',
                )),
              ];
              _scrollToBottom();
              if (children.isEmpty) {
                return _emptyState(context);
              }
              return ListView(
                controller: scrollController,
                padding: const EdgeInsets.all(12),
                children: children,
              );
            }),
          ),
          _inputBar(context),
        ],
      ),
    );
  }

  Widget _emptyState(BuildContext context) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(Icons.laptop_mac, size: 56,
              color: Theme.of(context).colorScheme.primary),
          const SizedBox(height: 12),
          const Text('Chat using your laptop\'s compute',
              style: TextStyle(fontSize: 16, fontWeight: FontWeight.w600)),
          const SizedBox(height: 6),
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 32),
            child: Text(
              'Start the laptop node, tap the laptop icon above to discover '
              'it, then send a message. Cloud and on-device backends are in Settings.',
              textAlign: TextAlign.center,
            ),
          ),
        ],
      ),
    );
  }

  Widget _bubble(ChatMessage m) {
    final isUser = m.role == 'user';
    return Align(
      alignment: isUser ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 4),
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
        constraints: BoxConstraints(
            maxWidth: MediaQuery.of(context).size.width * 0.8),
        decoration: BoxDecoration(
          color: isUser
              ? Theme.of(context).colorScheme.primaryContainer
              : Theme.of(context).colorScheme.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(14),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SelectableText(m.content),
            if (!isUser && (m.backend != null || m.model != null))
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Text(
                  [m.backend, m.model].whereType<String>().join(' · '),
                  style: TextStyle(
                      fontSize: 10,
                      color: Theme.of(context).colorScheme.outline),
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _inputBar(BuildContext context) {
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(8),
        child: Row(
          children: [
            Expanded(
              child: TextField(
                controller: inputController,
                minLines: 1,
                maxLines: 5,
                decoration: const InputDecoration(
                  hintText: 'Message…',
                  border: OutlineInputBorder(),
                  isDense: true,
                ),
                onSubmitted: (_) => _send(),
              ),
            ),
            const SizedBox(width: 8),
            Obx(() => controller.isGenerating.value
                ? const Padding(
                    padding: EdgeInsets.all(12),
                    child: SizedBox(
                        width: 22, height: 22,
                        child: CircularProgressIndicator(strokeWidth: 2)),
                  )
                : IconButton.filled(
                    icon: const Icon(Icons.send),
                    onPressed: _send,
                  )),
          ],
        ),
      ),
    );
  }

  void _send() {
    final text = inputController.text;
    inputController.clear();
    controller.send(text);
  }
}
