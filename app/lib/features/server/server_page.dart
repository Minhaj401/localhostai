import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../../models/pairing.dart';
import '../../theme/theme.dart';
import '../../theme/tokens.dart';
import '../../widgets/app_card.dart';
import '../../widgets/brand.dart';
import '../../widgets/buttons.dart';
import '../../widgets/command_pill.dart';
import '../../widgets/common.dart';
import '../../widgets/reveal.dart';
import '../../widgets/status_dot.dart';
import '../../widgets/terminal_window.dart';
import '../connection/connection_controller.dart';

class ServerPage extends ConsumerWidget {
  const ServerPage({super.key});

  Future<void> _pair(BuildContext context, WidgetRef ref) async {
    final result = await context.push<PairPayload>('/pair');
    if (result != null && context.mounted) {
      await ref.read(connectionProvider).connect(result);
    }
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final node = ref.watch(connectionProvider);
    final running = node.connected && node.clusterState == 'running';
    final externalReady =
        node.connected && node.info?.ready == true && !running;
    final loading = node.checking || node.clusterState == 'starting';
    final failed = node.clusterState == 'failed';
    final status = loading
        ? 'Loading'
        : running
            ? 'Running'
            : failed
                ? 'Model failed'
                : externalReady
                    ? '${node.info!.backend} ready'
                    : node.connected
                        ? 'Online / no model'
                        : node.pair != null
                            ? 'Offline'
                            : 'Not connected';
    final color = loading
        ? C.warn
        : running || externalReady
            ? C.accent
            : failed
                ? C.danger
                : node.connected
                    ? C.text2
                    : C.danger;
    final selected = node.models
        .where((model) => model.id == node.selectedModel)
        .firstOrNull;

    return Scaffold(
      appBar: AppBar(title: const Text('Server'), actions: [
        IconButton(
            tooltip: 'Refresh server',
            onPressed: node.refreshing ? null : node.refresh,
            icon: const Icon(LucideIcons.refreshCw, size: 20)),
      ]),
      body: SafeArea(
          top: false,
          child: RefreshIndicator(
            onRefresh: node.refresh,
            child: ListView(
                physics: const AlwaysScrollableScrollPhysics(),
                children: [
                  AdaptiveContent(
                      child: Reveal(
                          child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      const Brand(size: 28),
                      const SizedBox(height: 24),
                      const SectionHeader('Your local coordinator',
                          eyebrow: 'Server',
                          subtitle:
                              'Pair a laptop. Control models and share spare phone memory.'),
                      const SizedBox(height: 24),
                      AppCard(
                          featured: running,
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              Row(children: [
                                StatusDot(
                                    color: color,
                                    pulse: running || externalReady,
                                    size: 10,
                                    label: 'Server $status'),
                                const SizedBox(width: 12),
                                Expanded(
                                    child: Text(status,
                                        style: Theme.of(context)
                                            .textTheme
                                            .titleLarge)),
                                Icon(LucideIcons.server,
                                    color: color, size: 22),
                              ]),
                              const SizedBox(height: 16),
                              Text(
                                  node.info?.name ??
                                      node.pair?.name ??
                                      'Start the laptop TUI',
                                  style: Theme.of(context).textTheme.bodyLarge),
                              const SizedBox(height: 12),
                              CommandPill(
                                  command: node.endpoint.isEmpty
                                      ? 'privateml --require-pairing'
                                      : node.endpoint,
                                  label: node.endpoint.isEmpty
                                      ? 'Copy laptop command'
                                      : 'Copy server address'),
                              if (node.runningModel != null ||
                                  node.info?.models.firstOrNull != null) ...[
                                const SizedBox(height: 12),
                                Text(
                                    node.runningModel ??
                                        node.info!.models.first,
                                    style: mono(13, color: C.text)),
                              ],
                              const SizedBox(height: 20),
                              if (node.client == null)
                                PrimaryButton(
                                    label: 'Pair laptop',
                                    icon: LucideIcons.qrCode,
                                    onPressed: node.checking
                                        ? null
                                        : () => _pair(context, ref))
                              else if (running ||
                                  node.clusterState == 'starting')
                                SecondaryButton(
                                    label: node.clusterState == 'starting'
                                        ? 'Cancel loading'
                                        : 'Stop model',
                                    icon: LucideIcons.square,
                                    onPressed:
                                        node.installing ? null : node.stopModel)
                              else if (externalReady)
                                PrimaryButton(
                                    label: 'Open chat',
                                    icon: LucideIcons.messageSquare,
                                    onPressed: () => context.go('/chat'))
                              else
                                PrimaryButton(
                                    label:
                                        selected != null && selected.installed
                                            ? 'Start selected model'
                                            : 'Choose a model',
                                    icon: LucideIcons.play,
                                    onPressed: node.operationBusy ||
                                            !node.connected
                                        ? null
                                        : selected != null && selected.installed
                                            ? () => node.install(selected,
                                                load: true)
                                            : () => context.go('/models')),
                              const SizedBox(height: 8),
                              Wrap(spacing: 12, runSpacing: 4, children: [
                                TextButton.icon(
                                    onPressed:
                                        node.operationBusy || node.checking
                                            ? null
                                            : () => _pair(context, ref),
                                    icon:
                                        const Icon(LucideIcons.link, size: 16),
                                    label: const Text('Pairing')),
                                TextButton.icon(
                                    onPressed: node.operationBusy
                                        ? null
                                        : () => context.push('/nearby'),
                                    icon:
                                        const Icon(LucideIcons.wifi, size: 16),
                                    label: const Text('Nearby laptops')),
                                if (node.client != null)
                                  TextButton.icon(
                                      onPressed: () => context.push('/devices'),
                                      icon:
                                          const Icon(LucideIcons.cpu, size: 16),
                                      label: const Text('Compute pool')),
                              ]),
                            ],
                          )),
                      if (node.error != null) ...[
                        const SizedBox(height: 16),
                        NoticeBanner(
                            message: node.error!, tone: StatusTone.danger),
                      ],
                      if (failed && node.cluster?['error'] is String) ...[
                        const SizedBox(height: 16),
                        NoticeBanner(
                            message: node.cluster!['error'] as String,
                            tone: StatusTone.danger),
                      ],
                      const SizedBox(height: 16),
                      const NoticeBanner(
                        message:
                            'Use localhost.ai on a trusted LAN. Coordinator HTTP and the raw RPC worker are not encrypted.',
                        tone: StatusTone.warning,
                      ),
                      if (node.operationMessage != null) ...[
                        const SizedBox(height: 12),
                        NoticeBanner(
                            message: node.operationMessage!,
                            tone: node.operationError == null
                                ? StatusTone.accent
                                : StatusTone.danger),
                      ],
                      const SizedBox(height: 24),
                      AppCard(
                          child: Column(
                              crossAxisAlignment: CrossAxisAlignment.stretch,
                              children: [
                            SwitchListTile.adaptive(
                              contentPadding: EdgeInsets.zero,
                              title: const Text('Share this phone'),
                              subtitle: const Text(
                                  'Run only the layers that do not fit on the laptop. Keep the app open while sharing.'),
                              value: node.worker.sharing.value,
                              secondary: const Icon(LucideIcons.smartphone,
                                  color: C.text2),
                              onChanged: node.sharingBusy ||
                                      node.worker.isBusy ||
                                      (!node.paired &&
                                          !node.worker.sharing.value)
                                  ? null
                                  : node.sharePhone,
                            ),
                            const SizedBox(height: 16),
                            Wrap(spacing: 32, runSpacing: 16, children: [
                              MetricTile(
                                  label: 'Safe share cap',
                                  value:
                                      '${(node.profile.budgetMb / 1024).toStringAsFixed(1)} GB'),
                              MetricTile(
                                  label: 'Inference threads',
                                  value: '${node.profile.threads}'),
                              MetricTile(
                                  label: 'Currently available',
                                  value:
                                      '${(node.profile.availableMb / 1024).toStringAsFixed(1)} GB'),
                            ]),
                            const SizedBox(height: 12),
                            Text(
                                'The planner keeps 256 MB of each remote budget for RPC buffers. Sharing can use battery and generate heat.',
                                style: Theme.of(context).textTheme.bodyMedium),
                          ])),
                      const SizedBox(height: 24),
                      TerminalWindow(
                          title: 'coordinator events',
                          running: running,
                          child: SelectableText(
                              node.eventLog.isEmpty
                                  ? 'Waiting for a coordinator connection.'
                                  : node.eventLog,
                              style: mono(13))),
                      if (node.cluster?['log'] is String) ...[
                        const SizedBox(height: 12),
                        Text(node.cluster!['log'] as String, style: mono(12)),
                      ],
                    ],
                  ))),
                ]),
          )),
    );
  }
}
