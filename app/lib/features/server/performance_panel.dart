import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:lucide_icons/lucide_icons.dart';
import '../../theme/theme.dart';
import '../../theme/tokens.dart';
import '../../widgets/app_card.dart';
import '../../widgets/buttons.dart';
import '../../widgets/common.dart';
import '../connection/connection_controller.dart';

String _number(Object? value, String unit) => value is num && value.isFinite
    ? '${value.toStringAsFixed(1)}$unit'
    : 'Not reported';

class PerformancePage extends ConsumerWidget {
  const PerformancePage({super.key});
  @override
  Widget build(BuildContext context, WidgetRef ref) => Scaffold(
      appBar: AppBar(title: const Text('Performance')),
      body: SafeArea(
          child: SingleChildScrollView(
              child: AdaptiveContent(
                  child:
                      PerformancePanel(node: ref.watch(connectionProvider))))));
}

class PerformancePanel extends StatelessWidget {
  const PerformancePanel({super.key, required this.node});
  final ConnectionController node;
  @override
  Widget build(BuildContext context) {
    final data = node.performance;
    final benchmark = data?['benchmark'] as Map?;
    final generations = data?['generations'] as List? ?? [];
    final Map? latest = node.benchmarking
        ? (benchmark?['current'] as Map?)
        : (generations.firstOrNull as Map?);
    final devices = data?['devices'] as List? ?? [];
    final results = benchmark?['results'] as List? ?? [];
    return AppCard(
        child:
            Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SectionHeader('Live performance',
          eyebrow: 'Measurements',
          subtitle:
              'Runtime token counts, observed latency, and actual process memory.'),
      const SizedBox(height: 16),
      Text(
          node.performanceError == null && data != null
              ? 'Updated every second while the app is active'
              : 'Waiting for live data · last snapshot retained',
          style: mono(12, color: C.text2)),
      if (node.performanceError != null)
        Padding(
            padding: const EdgeInsets.only(top: 12),
            child: NoticeBanner(
                message: node.performanceError!, tone: StatusTone.warning)),
      const SizedBox(height: 16),
      Wrap(spacing: 24, runSpacing: 16, children: [
        MetricTile(
            label: 'Generation speed',
            value: _number(latest?['tokens_per_second'], ' tok/s')),
        MetricTile(
            label: 'First token', value: _number(latest?['ttft_ms'], ' ms')),
        MetricTile(
            label: 'HTTP round trip',
            value: node.performanceError == null
                ? _number(node.latencyMs, ' ms')
                : 'Unavailable'),
        MetricTile(
            label: 'Output tokens',
            value: '${latest?['output_tokens'] ?? 'Not reported'}'),
      ]),
      const SizedBox(height: 12),
      Text(
          'Token source: ${latest?['token_source'] ?? 'Waiting for runtime timings or usage'}. First-token time includes prompt processing.',
          style: Theme.of(context).textTheme.bodyMedium),
      for (final device in devices.whereType<Map>()) ...[
        const Divider(height: 32),
        Text('${device['name']}',
            style: Theme.of(context).textTheme.titleMedium),
        const SizedBox(height: 8),
        Wrap(spacing: 24, runSpacing: 12, children: [
          MetricTile(
              label: 'Measured RSS', value: _number(device['rss_mb'], ' MiB')),
          MetricTile(
              label: 'Planned allocation',
              value: _number(device['planned_mb'], ' MiB')),
          MetricTile(
              label: 'Assigned layers',
              value: '${device['assigned_layers'] ?? 0}'),
        ]),
        const SizedBox(height: 8),
        Text(
            '${device['source']} · sample ${device['sample_age_seconds'] ?? 0}s old${device['self'] == false ? ' · report round trip ${_number(device['coordinator_rtt_ms'], ' ms')}' : ''}',
            style: mono(12)),
      ],
      const Divider(height: 32),
      const SectionHeader('Laptop vs distributed',
          subtitle:
              'Compare the selected model and context with the same prompt, seed, and 128-token output limit.'),
      const SizedBox(height: 12),
      const Text(
          'Each mode reloads and warms up the model. Chat waits during the test, and the model stops afterward. Local mode is skipped if it cannot fit.'),
      const SizedBox(height: 12),
      Text(
          'Model: ${node.selectedModel ?? node.runningModel ?? 'Choose an installed model'} · context ${node.contextTokens}',
          style: mono(12, color: C.text)),
      const SizedBox(height: 16),
      SecondaryButton(
          label: node.benchmarking ? 'Cancel benchmark' : 'Run comparison',
          icon: node.benchmarking ? LucideIcons.square : LucideIcons.gauge,
          onPressed: node.benchmarking
              ? node.cancelBenchmark
              : node.connected &&
                      !node.operationBusy &&
                      node.selectedModel != null
                  ? node.startBenchmark
                  : null),
      if (benchmark != null) ...[
        const SizedBox(height: 12),
        Text('${benchmark['state'] ?? ''} · ${benchmark['phase'] ?? ''}',
            style: mono(12, color: node.benchmarking ? C.warn : C.text2)),
        if (benchmark['error'] != null)
          Text('${benchmark['error']}',
              style: const TextStyle(color: C.danger)),
      ],
      for (final result in results.whereType<Map>()) ...[
        const SizedBox(height: 16),
        Text(
            '${result['mode'] == 'laptop' ? 'Laptop only' : 'Laptop + phone'} · ${result['state']}',
            style: Theme.of(context).textTheme.titleMedium),
        const SizedBox(height: 8),
        if (result['state'] == 'complete') ...[
          Wrap(spacing: 24, runSpacing: 12, children: [
            MetricTile(
                label: 'Speed',
                value: _number(
                    (result['metrics'] as Map?)?['tokens_per_second'],
                    ' tok/s')),
            MetricTile(
                label: 'First token',
                value: _number((result['metrics'] as Map?)?['ttft_ms'], ' ms')),
            MetricTile(
                label: 'Peak laptop RSS',
                value: _number(result['peak_engine_rss_mb'], ' MiB')),
          ]),
          const SizedBox(height: 8),
          Text('${(result['plan'] as Map?)?['summary'] ?? ''}',
              style: Theme.of(context).textTheme.bodyMedium),
        ] else
          Text('${result['reason'] ?? 'No measurement available'}'),
      ],
    ]));
  }
}
