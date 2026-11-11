import 'package:flutter/material.dart';
import 'package:flutter_animate/flutter_animate.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../theme/theme.dart';
import '../../theme/tokens.dart';
import '../../widgets/buttons.dart';
import '../../widgets/common.dart';
import '../../widgets/corner_brackets.dart';
import '../../widgets/terminal_window.dart';
import '../connection/connection_controller.dart';

class OnboardingPage extends ConsumerStatefulWidget {
  const OnboardingPage({super.key});

  @override
  ConsumerState<OnboardingPage> createState() => _OnboardingPageState();
}

class _OnboardingPageState extends ConsumerState<OnboardingPage> {
  final _pages = PageController();
  int _page = 0;
  static const _headings = [
    'Run AI ',
    'More room.\nSame ',
    'Your models.\nYour '
  ];
  static const _accentWords = ['locally.', 'model.', 'choice.'];
  static const _descriptions = [
    'Your laptop is the coordinator. This phone is your private chat client and an optional compute worker.',
    'Fill the laptop first. Share spare phone RAM only when a model needs more room.',
    'Choose a pinned GGUF, preview its placement, then download and run it on your devices.',
  ];
  static const _commands = [
    '>_ privateml --require-pairing\n\nChoose a model on the laptop.\nPress Ctrl+P to pair this phone.',
    '>_ laptop first\n\nLocal layers stay on the laptop.\nThe phone hosts only the remainder.',
    '>_ choose a model\n\npreview -> download -> verify\nload -> chat',
  ];

  @override
  void dispose() {
    _pages.dispose();
    super.dispose();
  }

  Future<void> _finish() async {
    await ref
        .read(storageProvider)
        .setString('localhost_onboarding_complete', 'true');
    if (mounted) context.go('/chat');
  }

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final logo = Image.asset('assets/logo/lockup.png',
        height: 64,
        alignment: Alignment.centerLeft,
        semanticLabel: 'localhost.ai. Run AI Locally.');
    return Scaffold(
      body: AppBackdrop(
          hero: true,
          child: SafeArea(
              child: Column(children: [
            Padding(
                padding: const EdgeInsets.fromLTRB(20, 12, 20, 0),
                child: Row(children: [
                  Expanded(
                      child: reduceMotion
                          ? logo
                          : logo.animate().fadeIn(
                              duration: const Duration(milliseconds: 400))),
                  TextButton(onPressed: _finish, child: const Text('Skip')),
                ])),
            Expanded(
                child: PageView.builder(
              controller: _pages,
              itemCount: 3,
              onPageChanged: (page) => setState(() => _page = page),
              itemBuilder: (context, index) => TickerMode(
                enabled: index == _page,
                child: LayoutBuilder(
                    builder: (context, constraints) => SingleChildScrollView(
                          child: ConstrainedBox(
                            constraints: BoxConstraints(
                                minHeight: constraints.maxHeight),
                            child: AdaptiveContent(
                                maxWidth: 640,
                                child: Column(
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    const SizedBox(height: 20),
                                    Text.rich(
                                        TextSpan(children: [
                                          TextSpan(text: _headings[index]),
                                          TextSpan(
                                              text: _accentWords[index],
                                              style: const TextStyle(
                                                  color: C.accent)),
                                        ]),
                                        style: Theme.of(context)
                                            .textTheme
                                            .displayLarge,
                                        textAlign: TextAlign.center),
                                    const SizedBox(height: 16),
                                    Text(_descriptions[index],
                                        textAlign: TextAlign.center,
                                        style: Theme.of(context)
                                            .textTheme
                                            .bodyLarge),
                                    const SizedBox(height: 32),
                                    CornerBrackets(
                                        child: TerminalWindow(
                                            title: 'localhost.ai / setup',
                                            glow: true,
                                            child: _TerminalTyping(
                                                text: _commands[index]))),
                                    const SizedBox(height: 16),
                                    Text(
                                        'Cloud backends are optional and send messages to their provider.',
                                        style: Theme.of(context)
                                            .textTheme
                                            .bodySmall,
                                        textAlign: TextAlign.center),
                                  ],
                                )),
                          ),
                        )),
              ),
            )),
            AdaptiveContent(
                maxWidth: 640,
                padding: const EdgeInsets.fromLTRB(20, 12, 20, 24),
                child: Column(children: [
                  Row(mainAxisAlignment: MainAxisAlignment.center, children: [
                    for (var index = 0; index < 3; index++)
                      AnimatedContainer(
                          duration: reduceMotion ? Duration.zero : Motion.base,
                          width: index == _page ? 16 : 6,
                          height: 6,
                          margin: const EdgeInsets.symmetric(horizontal: 4),
                          decoration: BoxDecoration(
                              color: index == _page ? C.accent : C.borderStrong,
                              borderRadius: BorderRadius.circular(R.pill))),
                  ]),
                  const SizedBox(height: 20),
                  SizedBox(
                      width: double.infinity,
                      child: PrimaryButton(
                        label: _page == 2 ? 'Get started' : 'Continue',
                        onPressed: () {
                          if (_page == 2) {
                            _finish();
                          } else {
                            if (reduceMotion) {
                              _pages.jumpToPage(_page + 1);
                            } else {
                              _pages.nextPage(
                                  duration: Motion.base, curve: Motion.curve);
                            }
                          }
                        },
                      )),
                ])),
          ]))),
    );
  }
}

class _TerminalTyping extends StatefulWidget {
  const _TerminalTyping({required this.text});
  final String text;
  @override
  State<_TerminalTyping> createState() => _TerminalTypingState();
}

class _TerminalTypingState extends State<_TerminalTyping>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: Duration(milliseconds: widget.text.length * 30),
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _controller.value = 1;
    } else if (TickerMode.of(context)) {
      _controller.forward();
    } else {
      _controller.stop();
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
        animation: _controller,
        builder: (context, _) =>
            Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text(
              widget.text.substring(
                  0, (_controller.value * widget.text.length).floor()),
              style: mono(13, color: C.text)),
          const BlinkingCursor(),
        ]),
      );
}
