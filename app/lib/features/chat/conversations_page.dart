import 'dart:async';
import 'package:flutter/material.dart';
import 'package:lucide_icons/lucide_icons.dart';
import '../../controllers/chat_controller.dart';
import '../../models/conversation.dart';
import '../../services/conversation_export.dart';
import '../../services/conversation_library.dart';
import '../../theme/theme.dart';
import '../../theme/tokens.dart';
import '../../widgets/common.dart';

class ConversationsPage extends StatefulWidget {
  const ConversationsPage({super.key, required this.chat});
  final ChatController chat;
  @override
  State<ConversationsPage> createState() => _ConversationsPageState();
}

class _ConversationsPageState extends State<ConversationsPage> {
  ConversationLibrary get library => widget.chat.shared;
  final search = TextEditingController();
  Timer? debounce;
  @override
  void initState() {
    super.initState();
    library.search('');
    unawaited(library.refresh());
  }

  @override
  void dispose() {
    debounce?.cancel();
    search.dispose();
    library.search('');
    super.dispose();
  }

  Future<void> _action(ConversationSummary item, String action) async {
    try {
      if (action == 'delete') {
        final deleted = await library.delete(item);
        if (!deleted && mounted) {
          ScaffoldMessenger.of(context).showSnackBar(SnackBar(
              content: Text(library.error ?? 'Could not delete conversation')));
        }
        await library.refresh();
        return;
      }
      final opened = await library.open(item.id);
      if (!mounted || !opened) return;
      if (action == 'rename') {
        final title = await showDialog<String>(
            context: context,
            builder: (context) => _RenameDialog(title: item.title));
        if (title != null) {
          final saved = await library.rename(title);
          if (!saved && mounted) {
            ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                content:
                    Text(library.error ?? 'Could not rename conversation')));
          }
        }
        await library.refresh();
      } else {
        final message =
            await ConversationExport.save(await library.export(action));
        if (mounted) {
          ScaffoldMessenger.of(context)
              .showSnackBar(SnackBar(content: Text(message)));
        }
      }
    } catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('$failure')));
      }
    }
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
      animation: library,
      builder: (context, _) => Scaffold(
            appBar: AppBar(title: const Text('Conversations'), actions: [
              IconButton(
                  tooltip: 'Refresh conversations',
                  onPressed: library.busy ? null : library.refresh,
                  icon: const Icon(LucideIcons.refreshCw)),
              IconButton(
                  tooltip: 'New shared conversation',
                  onPressed: library.busy || widget.chat.isGenerating.value
                      ? null
                      : () async {
                          if (await library.create() && context.mounted) {
                            Navigator.pop(context);
                          }
                        },
                  icon: const Icon(LucideIcons.plus)),
            ]),
            body: SafeArea(
                child: AdaptiveContent(
                    child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                  const Text(
                      'Saved on your paired laptop. Open a chat here or in the TUI to continue.'),
                  const SizedBox(height: 16),
                  if (library.canImport)
                    TextButton.icon(
                        onPressed: library.busy
                            ? null
                            : () async {
                                await library.importPhoneHistory();
                                await library.refresh();
                              },
                        icon: const Icon(LucideIcons.download),
                        label: const Text('Import this phone’s saved chat')),
                  TextField(
                      controller: search,
                      decoration: const InputDecoration(
                          labelText: 'Search names and messages',
                          prefixIcon: Icon(LucideIcons.search)),
                      onChanged: (value) {
                        debounce?.cancel();
                        debounce = Timer(const Duration(milliseconds: 350), () {
                          library.search(value);
                          library.refresh();
                        });
                      }),
                  const SizedBox(height: 12),
                  Text(
                      library.online
                          ? 'Synced with coordinator'
                          : 'Offline · last known conversations',
                      style:
                          mono(12, color: library.online ? C.accent : C.warn)),
                  if (library.error != null)
                    Padding(
                        padding: const EdgeInsets.symmetric(vertical: 12),
                        child: NoticeBanner(
                            message: library.error!, tone: StatusTone.warning)),
                  if (library.busy) const LinearProgressIndicator(),
                  const SizedBox(height: 12),
                  Expanded(
                      child: library.conversations.isEmpty
                          ? const Center(
                              child: Text(
                                  'No conversations found. Start a new chat.'))
                          : ListView.separated(
                              itemCount: library.conversations.length,
                              separatorBuilder: (_, __) => const Divider(),
                              itemBuilder: (context, index) {
                                final item = library.conversations[index];
                                return ListTile(
                                  contentPadding: EdgeInsets.zero,
                                  title: Text(item.title,
                                      maxLines: 2,
                                      overflow: TextOverflow.ellipsis),
                                  subtitle: Text(
                                      '${item.messageCount} messages${item.generating ? ' · generating' : ''}\n${item.preview}',
                                      maxLines: 3,
                                      overflow: TextOverflow.ellipsis),
                                  leading: const Icon(LucideIcons.messageSquare,
                                      color: C.text2),
                                  onTap: library.busy ||
                                          widget.chat.isGenerating.value
                                      ? null
                                      : () async {
                                          if (await library.open(item.id) &&
                                              context.mounted) {
                                            Navigator.pop(context);
                                          }
                                        },
                                  trailing: PopupMenuButton<String>(
                                      tooltip: 'Conversation actions',
                                      enabled: !library.busy &&
                                          !widget.chat.isGenerating.value,
                                      onSelected: (action) =>
                                          _action(item, action),
                                      itemBuilder: (_) => const [
                                            PopupMenuItem(
                                                value: 'rename',
                                                child: Text('Rename')),
                                            PopupMenuItem(
                                                value: 'markdown',
                                                child: Text('Export Markdown')),
                                            PopupMenuItem(
                                                value: 'json',
                                                child: Text('Export JSON')),
                                            PopupMenuItem(
                                                value: 'delete',
                                                child: Text(
                                                    'Delete conversation')),
                                          ]),
                                );
                              })),
                ]))),
          ));
}

class _RenameDialog extends StatefulWidget {
  const _RenameDialog({required this.title});
  final String title;
  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final input = TextEditingController(text: widget.title);
  @override
  void dispose() {
    input.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
          title: const Text('Rename conversation'),
          content: TextField(
              controller: input,
              autofocus: true,
              maxLength: 120,
              decoration: const InputDecoration(labelText: 'Conversation name'),
              onSubmitted: (value) => Navigator.pop(context, value)),
          actions: [
            TextButton(
                onPressed: () => Navigator.pop(context),
                child: const Text('Cancel')),
            TextButton(
                onPressed: () => Navigator.pop(context, input.text),
                child: const Text('Save'))
          ]);
}
