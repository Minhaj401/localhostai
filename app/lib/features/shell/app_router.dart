import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:go_router/go_router.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../../theme/tokens.dart';
import '../../ui/nodes_screen.dart';
import '../../ui/pair_screen.dart';
import '../../ui/pool_screen.dart';
import '../../widgets/brand.dart';
import '../chat/chat_page.dart';
import '../connection/connection_controller.dart';
import '../models/models_page.dart';
import '../models/model_browser_page.dart';
import '../onboarding/onboarding_page.dart';
import '../server/server_page.dart';
import '../settings/settings_page.dart';

final routerProvider = Provider<GoRouter>((ref) {
  final storage = ref.read(storageProvider);
  final router = GoRouter(
    initialLocation:
        storage.getString('localhost_onboarding_complete') == 'true'
            ? '/chat'
            : '/welcome',
    routes: [
      GoRoute(
          path: '/welcome',
          pageBuilder: (context, state) =>
              _page(state, const OnboardingPage())),
      StatefulShellRoute(
        builder: (context, state, shell) => AppShell(navigationShell: shell),
        navigatorContainerBuilder: (context, shell, children) =>
            _BranchContainer(index: shell.currentIndex, children: children),
        branches: [
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/chat',
                pageBuilder: (context, state) =>
                    _page(state, const ChatScreen()))
          ]),
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/models',
                pageBuilder: (context, state) =>
                    _page(state, const ModelsPage()))
          ]),
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/server',
                pageBuilder: (context, state) =>
                    _page(state, const ServerPage()))
          ]),
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/settings',
                pageBuilder: (context, state) => _page(
                    state,
                    SettingsScreen(
                      onNodeChanged: (url) =>
                          ref.read(connectionProvider).connectAddress(url),
                      onReplaySetup: () => context.go('/welcome'),
                    )))
          ]),
        ],
      ),
      GoRoute(
          path: '/models/browse',
          pageBuilder: (context, state) =>
              _page(state, const ModelBrowserPage())),
      GoRoute(
          path: '/pair',
          pageBuilder: (context, state) => _page(state, const PairScreen())),
      GoRoute(
          path: '/nearby',
          pageBuilder: (context, state) => _page(
              state,
              NodesScreen(
                onSelected: (url) =>
                    ref.read(connectionProvider).connectAddress(url),
              ))),
      GoRoute(
          path: '/devices',
          pageBuilder: (context, state) =>
              _page(state, Consumer(builder: (context, ref, _) {
                final node = ref.watch(connectionProvider);
                return node.client == null
                    ? const ServerPage()
                    : PoolScreen(
                        client: node.client!,
                        profile: node.profile,
                        contextTokens: node.contextTokens,
                        phoneReserveMb: node.phoneReserveMb,
                        onPhoneReserveChanged: (mb) =>
                            ref.read(connectionProvider).setPhoneReserveMb(mb),
                      );
              }))),
    ],
  );
  ref.onDispose(router.dispose);
  return router;
});

CustomTransitionPage<dynamic> _page(GoRouterState state, Widget child) =>
    CustomTransitionPage<dynamic>(
      key: state.pageKey,
      child: child,
      transitionDuration: Motion.base,
      reverseTransitionDuration: Motion.base,
      transitionsBuilder: (context, animation, secondaryAnimation, child) =>
          MediaQuery.disableAnimationsOf(context)
              ? child
              : FadeTransition(
                  opacity:
                      CurvedAnimation(parent: animation, curve: Motion.curve),
                  child: child),
    );

class AppShell extends StatelessWidget {
  const AppShell({super.key, required this.navigationShell});
  final StatefulNavigationShell navigationShell;

  static const _labels = ['Chat', 'Models', 'Server', 'Settings'];
  static const _icons = [
    LucideIcons.messageSquare,
    LucideIcons.box,
    LucideIcons.server,
    LucideIcons.settings
  ];

  @override
  Widget build(BuildContext context) {
    final wide = MediaQuery.sizeOf(context).width >= 840;
    final keyboard = MediaQuery.viewInsetsOf(context).bottom > 0;
    return Scaffold(
      body: Row(children: [
        if (wide) ...[
          SafeArea(
              child: NavigationRail(
            selectedIndex: navigationShell.currentIndex,
            onDestinationSelected: navigationShell.goBranch,
            labelType: NavigationRailLabelType.all,
            minWidth: 104,
            leading: Padding(
                padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
                child: SizedBox(
                    width: 124,
                    child: Column(children: [
                      SvgPicture.asset('assets/logo/mark.svg',
                          width: 56, height: 56, excludeFromSemantics: true),
                      const SizedBox(height: 8),
                      const Brand(compact: true, size: 18, showMark: false),
                    ]))),
            destinations: [
              for (var i = 0; i < _labels.length; i++)
                NavigationRailDestination(
                    icon: Icon(_icons[i]), label: Text(_labels[i])),
            ],
          )),
          const VerticalDivider(width: 1, color: C.border),
        ],
        Expanded(child: navigationShell),
      ]),
      bottomNavigationBar: wide || keyboard
          ? null
          : DecoratedBox(
              decoration: const BoxDecoration(
                  border: Border(top: BorderSide(color: C.border))),
              child: NavigationBar(
                selectedIndex: navigationShell.currentIndex,
                onDestinationSelected: navigationShell.goBranch,
                destinations: [
                  for (var i = 0; i < _labels.length; i++)
                    NavigationDestination(
                        icon: Icon(_icons[i]), label: _labels[i]),
                ],
              ),
            ),
    );
  }
}

class _BranchContainer extends StatefulWidget {
  const _BranchContainer({required this.index, required this.children});
  final int index;
  final List<Widget> children;
  @override
  State<_BranchContainer> createState() => _BranchContainerState();
}

class _BranchContainerState extends State<_BranchContainer>
    with SingleTickerProviderStateMixin {
  late final AnimationController _fade =
      AnimationController(vsync: this, duration: Motion.base, value: 1);

  @override
  void didUpdateWidget(_BranchContainer oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.index != widget.index) {
      if (MediaQuery.disableAnimationsOf(context)) {
        _fade.value = 1;
      } else {
        _fade.forward(from: 0);
      }
    }
  }

  @override
  void dispose() {
    _fade.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => FadeTransition(
        opacity: _fade,
        child: IndexedStack(index: widget.index, children: [
          for (var i = 0; i < widget.children.length; i++)
            TickerMode(enabled: i == widget.index, child: widget.children[i]),
        ]),
      );
}
