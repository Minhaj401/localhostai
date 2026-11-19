import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:get/get.dart';
import 'package:go_router/go_router.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../../controllers/chat_controller.dart';
import '../../models/chat_message.dart';
import '../../services/storage_service.dart';
import '../../theme/theme.dart';
import '../../theme/tokens.dart';
import '../../widgets/brand.dart';
import '../../widgets/buttons.dart';
import '../../widgets/common.dart';
import '../../widgets/terminal_window.dart';
import '../connection/connection_controller.dart';
import 'conversations_page.dart';

class ChatScreen extends ConsumerStatefulWidget {
  const ChatScreen({super.key});

  @override
  ConsumerState<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends ConsumerState<ChatScreen> {
  late final ChatController controller;
  final _input = TextEditingController();
  final _focus = FocusNode();
  final _scroll = ScrollController();
  bool _following = true;
  bool _scrollQueued = false;
  Worker? _draftWatch;
  final Map<String, String> _drafts = {};
  String _draftConversation = '';

  @override
  void initState() {
    super.initState();
    controller = Get.isRegistered<ChatController>()
        ? Get.find<ChatController>()
        : Get.put(ChatController(Get.find<StorageService>()));
    _input.addListener(_inputChanged);
    _draftConversation = controller.shared.current?.id ?? '';
    controller.shared.addListener(_conversationChanged);
    _draftWatch = ever<String?>(controller.retryDraft, (text) {
      if (text != null && _input.text.isEmpty) _input.text = text;
      controller.retryDraft.value = null;
    });
  }

  void _inputChanged() {
    if (mounted) setState(() {});
  }

  void _conversationChanged() {
    if (!mounted) return;
    final next = controller.shared.current?.id ?? '';
    if (next == _draftConversation) return;
    _drafts[_draftConversation] = _input.text;
    _input.text = _drafts.remove(next) ?? '';
    _draftConversation = next;
    _following = true;
  }

  void _follow() {
    if (!_following || _scrollQueued) return;
    _scrollQueued = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _scrollQueued = false;
      if (mounted && _following && _scroll.hasClients) {
        _scroll.jumpTo(_scroll.position.maxScrollExtent);
      }
    });
  }

  bool _onScroll(UserScrollNotification notification) {
    if (notification.direction == ScrollDirection.forward) {
      if (_following) setState(() => _following = false);
    } else if (_scroll.hasClients &&
        _scroll.position.maxScrollExtent - _scroll.position.pixels < 60) {
      if (!_following) setState(() => _following = true);
    }
    return false;
  }

  @override
  void dispose() {
    _draftWatch?.dispose();
    controller.shared.removeListener(_conversationChanged);
    _input.removeListener(_inputChanged);
    _input.dispose();
    _focus.dispose();
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _newChat() async {
    if (controller.activeBackendId.value == 'laptop' &&
        controller.shared.ready) {
      await controller.clearChat();
      return;
    }
    if (controller.messages.isEmpty) return;
    final clear = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
              title: const Text('Start a new chat?'),
              content: const Text(
                  'This clears the current conversation on this phone.'),
              actions: [
                TextButton(
                    onPressed: () => Navigator.pop(context, false),
                    child: const Text('Keep chat')),
                FilledButton(
                    onPressed: () => Navigator.pop(context, true),
                    child: const Text('New chat')),
              ],
            ));
    if (clear == true) await controller.clearChat();
  }

  @override
  Widget build(BuildContext context) {
    final node = ref.watch(connectionProvider);
    return Scaffold(
      appBar: AppBar(
          title: const Brand(compact: true, size: 22, showMark: false),
          actions: [
            IconButton(
                tooltip: 'Saved conversations',
                onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                        builder: (_) => ConversationsPage(chat: controller))),
                icon: const Icon(LucideIcons.history, size: 20)),
            IconButton(
                tooltip: 'New chat',
                onPressed: _newChat,
                icon: const Icon(LucideIcons.plus, size: 20)),
          ]),
      body: SafeArea(
          top: false,
          bottom: false,
          child: Column(children: [
            AnimatedBuilder(
                animation: controller.shared,
                builder: (context, _) {
                  final chat = controller.shared.current;
                  if (chat == null ||
                      controller.activeBackendId.value != 'laptop') {
                    return const SizedBox.shrink();
                  }
                  return AdaptiveContent(
                      maxWidth: 720,
                      padding: const EdgeInsets.fromLTRB(20, 4, 20, 4),
                      child: Row(children: [
                        Expanded(
                            child: Text(chat.title,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: mono(13, color: C.text))),
                        const SizedBox(width: 8),
                        Text(
                            chat.activeRequest != null
                                ? 'Generating'
                                : controller.shared.online
                                    ? 'Synced'
                                    : 'Offline',
                            style: mono(12,
                                color: controller.shared.online
                                    ? C.accent
                                    : C.warn)),
                      ]));
                }),
            Obx(() {
              final error = controller.error.value;
              if (error == null) return const SizedBox.shrink();
              return AdaptiveContent(
                maxWidth: 720,
                padding: const EdgeInsets.fromLTRB(20, 8, 20, 8),
                child: NoticeBanner(
                    message: cleanError(error),
                    tone: StatusTone.danger,
                    action: IconButton(
                        tooltip: 'Dismiss error',
                        onPressed: () => controller.error.value = null,
                        icon: const Icon(LucideIcons.x, size: 16))),
              );
            }),
            Expanded(child: Obx(() {
              final count = controller.messages.length;
              final generating = controller.isGenerating.value;
              if (count == 0 && !generating) return _empty(node);
              _follow();
              return Align(
                  alignment: Alignment.topCenter,
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 720),
                    child: Stack(children: [
                      NotificationListener<UserScrollNotification>(
                        onNotification: _onScroll,
                        child: ListView.builder(
                          controller: _scroll,
                          keyboardDismissBehavior:
                              ScrollViewKeyboardDismissBehavior.onDrag,
                          padding: const EdgeInsets.fromLTRB(20, 12, 20, 24),
                          itemCount: count + (generating ? 1 : 0),
                          itemBuilder: (context, index) {
                            if (index < count) {
                              final message = controller.messages[index];
                              return RepaintBoundary(
                                  key: ObjectKey(message),
                                  child: _Message(
                                      message: message,
                                      streaming: message.generating));
                            }
                            return RepaintBoundary(child: Obx(() {
                              final text = controller.streamingText.value;
                              _follow();
                              return _Message(
                                  message: ChatMessage(
                                      role: 'assistant', content: text),
                                  streaming: true);
                            }));
                          },
                        ),
                      ),
                      if (!_following)
                        Positioned(
                            right: 20,
                            bottom: 16,
                            child: IconButton.filledTonal(
                              tooltip: 'Jump to latest message',
                              onPressed: () {
                                setState(() => _following = true);
                                _follow();
                              },
                              icon: const Icon(LucideIcons.arrowDown,
                                  color: C.accent),
                            )),
                    ]),
                  ));
            })),
            _composer(node),
          ])),
    );
  }

  Widget _empty(ConnectionController node) => AppBackdrop(
      hero: true,
      child: ListView(children: [
        AdaptiveContent(
            maxWidth: 720,
            child: Column(children: [
              const SizedBox(height: 32),
              SvgPicture.asset('assets/logo/mark.svg',
                  width: 120,
                  height: 120,
                  semanticsLabel: 'localhost.ai terminal mark'),
              const SizedBox(height: 24),
              const SectionHeader('Run AI locally.',
                  centered: true,
                  subtitle:
                      'A quiet place to think, ask, and build with your own models.'),
              const SizedBox(height: 24),
              Obx(() => controller.activeBackendId.value == 'laptop'
                  ? TerminalWindow(
                      title: node.ready
                          ? 'model ready'
                          : 'connect your coordinator',
                      child: Text(
                        node.ready
                            ? '${node.runningModel ?? node.info?.models.firstOrNull ?? 'Model ready'}\n\n>_ type a message below'
                            : '>_ localhostai --require-pairing\n\nChoose a model on the laptop.\nPress Ctrl+P to pair this phone.',
                        style: mono(13, color: C.text),
                      ))
                  : const NoticeBanner(
                      message:
                          'This conversation uses an external provider configured in Settings.')),
              const SizedBox(height: 24),
              if (!node.ready && controller.activeBackendId.value == 'laptop')
                SizedBox(
                    width: double.infinity,
                    child: PrimaryButton(
                      label: node.client == null
                          ? 'Connect laptop'
                          : 'Choose a model',
                      icon: node.client == null
                          ? LucideIcons.link
                          : LucideIcons.box,
                      onPressed: () => context
                          .go(node.client == null ? '/server' : '/models'),
                    )),
            ])),
      ]));

  Widget _composer(ConnectionController node) => Align(
        alignment: Alignment.bottomCenter,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 720),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(20, 8, 20, 16),
            child:
                Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Obx(() {
                final laptop = controller.activeBackendId.value == 'laptop';
                final label = laptop
                    ? node.runningModel ??
                        node.info?.models.firstOrNull ??
                        'No model loaded'
                    : controller.backend.displayName;
                return Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: InkWell(
                    onTap: () => context.go(laptop ? '/models' : '/settings'),
                    borderRadius: BorderRadius.circular(R.pill),
                    child: StatusPill(label,
                        tone: laptop && node.ready
                            ? StatusTone.positive
                            : StatusTone.neutral),
                  ),
                );
              }),
              Container(
                padding: const EdgeInsets.all(8),
                decoration: BoxDecoration(
                    color: C.surface,
                    border: Border.all(color: C.borderStrong),
                    borderRadius: BorderRadius.circular(R.xl)),
                child:
                    Row(crossAxisAlignment: CrossAxisAlignment.end, children: [
                  Expanded(
                      child: TextField(
                    controller: _input,
                    focusNode: _focus,
                    minLines: 1,
                    maxLines: 5,
                    textCapitalization: TextCapitalization.sentences,
                    decoration: const InputDecoration(
                      hintText: 'Message localhost.ai',
                      filled: false,
                      border: InputBorder.none,
                      enabledBorder: InputBorder.none,
                      focusedBorder: InputBorder.none,
                    ),
                    onSubmitted: (_) => _send(),
                  )),
                  Obx(() => IconButton.filled(
                        tooltip: controller.isGenerating.value
                            ? 'Stop generating'
                            : 'Send message',
                        style: IconButton.styleFrom(
                            shape: const CircleBorder(),
                            backgroundColor: C.accent,
                            foregroundColor: C.bg,
                            minimumSize: const Size.square(48)),
                        onPressed: controller.isGenerating.value
                            ? controller.stopGeneration
                            : _input.text.trim().isEmpty
                                ? null
                                : _send,
                        icon: Icon(
                            controller.isGenerating.value
                                ? LucideIcons.square
                                : LucideIcons.arrowUp,
                            size: 20),
                      )),
                ]),
              ),
            ]),
          ),
        ),
      );

  void _send() {
    if (controller.isGenerating.value || _input.text.trim().isEmpty) return;
    final text = _input.text;
    _input.clear();
    setState(() => _following = true);
    controller.send(text);
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.message, this.streaming = false});
  final ChatMessage message;
  final bool streaming;

  @override
  Widget build(BuildContext context) {
    final user = message.role == 'user';
    final content =
        Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      if (!user) ...[
        Row(children: [
          SvgPicture.asset('assets/logo/mark.svg',
              width: 28, height: 28, excludeFromSemantics: true),
          const SizedBox(width: 8),
          Text('localhost.ai', style: Theme.of(context).textTheme.titleMedium),
        ]),
        const SizedBox(height: 12),
      ],
      if (streaming && message.content.isEmpty)
        Semantics(
            liveRegion: true,
            child: Text('Preparing response...', style: mono(13)))
      else
        _MessageContent(text: message.content, user: user),
      if (streaming) const BlinkingCursor(),
      if (!user && message.backend != null) ...[
        const SizedBox(height: 8),
        Text(message.backend!, style: mono(12)),
      ],
      if (!user && message.interrupted && !streaming) ...[
        const SizedBox(height: 8),
        Text('Interrupted response', style: mono(12, color: C.warn)),
      ],
    ]);
    return Semantics(
      label: user ? 'You said' : 'localhost.ai replied',
      container: true,
      child: Padding(
        padding: const EdgeInsets.only(bottom: 24),
        child: LayoutBuilder(
            builder: (context, constraints) => Align(
                  alignment:
                      user ? Alignment.centerRight : Alignment.centerLeft,
                  child: Container(
                    constraints: BoxConstraints(
                        maxWidth: user
                            ? constraints.maxWidth * 0.9
                            : constraints.maxWidth),
                    padding: user ? const EdgeInsets.all(16) : EdgeInsets.zero,
                    decoration: user
                        ? const BoxDecoration(
                            color: C.surface2,
                            borderRadius: BorderRadius.only(
                                topLeft: Radius.circular(16),
                                topRight: Radius.circular(16),
                                bottomLeft: Radius.circular(16),
                                bottomRight: Radius.circular(4)))
                        : null,
                    child: content,
                  ),
                )),
      ),
    );
  }
}

class _MessageContent extends StatelessWidget {
  const _MessageContent({required this.text, required this.user});
  final String text;
  final bool user;

  @override
  Widget build(BuildContext context) {
    if (user) {
      return SelectableText(text,
          style:
              Theme.of(context).textTheme.bodyLarge?.copyWith(color: C.text));
    }
    final matches =
        RegExp(r'```[^\n]*\n([\s\S]*?)```').allMatches(text).toList();
    if (matches.isEmpty) {
      return SelectableText(text,
          style:
              Theme.of(context).textTheme.bodyLarge?.copyWith(color: C.text));
    }
    final children = <Widget>[];
    var offset = 0;
    for (final match in matches) {
      if (match.start > offset) {
        children.add(SelectableText(text.substring(offset, match.start),
            style: Theme.of(context)
                .textTheme
                .bodyLarge
                ?.copyWith(color: C.text)));
      }
      children.add(Container(
        width: double.infinity,
        margin: const EdgeInsets.symmetric(vertical: 12),
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
            color: C.termBg,
            border: Border.all(color: C.border),
            borderRadius: BorderRadius.circular(R.md)),
        child: SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: SelectableText(match.group(1)!,
                style: mono(13, color: C.text))),
      ));
      offset = match.end;
    }
    if (offset < text.length) {
      children.add(SelectableText(text.substring(offset),
          style:
              Theme.of(context).textTheme.bodyLarge?.copyWith(color: C.text)));
    }
    return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch, children: children);
  }
}
