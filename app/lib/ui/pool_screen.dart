import 'package:flutter/material.dart';

import '../models/device_profile.dart';
import '../services/node_client.dart';
import 'design_system.dart';

/// Shows only capacity backed by a reachable, enabled worker.
class PoolScreen extends StatefulWidget {
  const PoolScreen({
    super.key,
    required this.client,
    required this.profile,
    this.contextTokens = 4096,
  });

  final NodeClient client;
  final DeviceProfile profile;
  final int contextTokens;

  @override
  State<PoolScreen> createState() => _PoolScreenState();
}

class _PoolScreenState extends State<PoolScreen> {
  List<PoolDevice> _devices = const [];
  List<String> _models = const [];
  SplitPlan? _plan;
  String? _selectedModel;
  String? _notice;
  bool _noticeError = false;
  bool _connectionError = false;
  bool _busy = false;
  bool _initialLoading = true;
  Map<String, dynamic>? _cluster;
  late int _contextTokens;

  @override
  void initState() {
    super.initState();
    _contextTokens = widget.contextTokens;
    _refresh();
  }

  Future<void> _refresh() async {
    if (_busy) return;
    setState(() => _busy = true);
    final results = await Future.wait<Object?>([
      widget.client.plannableModels(),
      widget.client.peers(),
      widget.client.clusterStatus(),
      widget.client.health(),
    ]);
    if (!mounted) return;
    final models = (results[0] as List<String>).toSet().toList();
    final devices = results[1] as List<PoolDevice>;
    final cluster = results[2] as Map<String, dynamic>?;
    final health = results[3] as NodeInfo?;
    if (health == null) {
      setState(() {
        _busy = false;
        _initialLoading = false;
        _connectionError = true;
        _noticeError = true;
        _notice =
            'The laptop is not responding. Last known pool data is preserved.';
      });
      return;
    }
    final running = cluster?['state'] == 'running';
    setState(() {
      _devices = devices;
      _models = models;
      if (!_models.contains(_selectedModel)) {
        _selectedModel = _models.isNotEmpty ? _models.first : null;
        _plan = null;
      } else if (!running) {
        _plan = null;
      }
      _cluster = cluster;
      _connectionError = false;
      _busy = false;
      _initialLoading = false;
    });
  }

  int get _usableMb => _devices
      .where((device) => device.usable)
      .fold(0, (sum, device) => sum + device.budgetMb);

  int get _allocatedMb =>
      _devices.fold(0, (sum, device) => sum + device.assignedMemoryMb);

  Future<void> _planSplit() async {
    final model = _selectedModel;
    if (model == null) return;
    setState(() {
      _busy = true;
      _notice = null;
      _noticeError = false;
    });
    final plan = await widget.client.plan(model, ctx: _contextTokens);
    if (!mounted) return;
    setState(() {
      _plan = plan;
      _busy = false;
      if (plan == null) {
        _notice =
            'The laptop did not return a split plan. Check the connection and try again.';
        _noticeError = true;
      }
    });
  }

  Future<void> _startDistributed() async {
    final model = _selectedModel;
    if (model == null) return;
    setState(() {
      _busy = true;
      _noticeError = false;
      _notice = 'Loading the model across the selected devices...';
    });
    final result = await widget.client.startCluster(model, ctx: _contextTokens);
    if (!mounted) return;
    setState(() {
      _busy = false;
      _plan = result.plan ?? _plan;
      _noticeError = !result.ok;
      _notice = result.ok
          ? 'The model is live. Open Chat to use this compute pool.'
          : 'Could not start: ${result.error}';
    });
    await _refresh();
  }

  Future<void> _stop() async {
    setState(() {
      _busy = true;
      _noticeError = false;
      _notice = 'Stopping the active model...';
    });
    final stopped = await widget.client.stopCluster();
    if (!mounted) return;
    setState(() {
      _busy = false;
      _noticeError = !stopped;
      _notice = stopped
          ? 'The distributed model has stopped.'
          : 'Could not stop the model. Check the laptop connection and retry.';
    });
    await _refresh();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final state = _cluster?['state']?.toString() ?? 'stopped';
    final running = state == 'running';
    final usableCount = _devices.where((device) => device.usable).length;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Compute pool'),
        actions: [
          IconButton(
            onPressed: _busy ? null : _refresh,
            icon: const AsciiGlyph('[R]', semanticLabel: 'Refresh'),
            tooltip: 'Refresh',
          ),
        ],
      ),
      body: AppBackdrop(
        child: RefreshIndicator(
          onRefresh: _refresh,
          child: ListView(
            physics: const AlwaysScrollableScrollPhysics(),
            children: [
              AdaptiveContent(
                maxWidth: 1120,
                child: _initialLoading
                    ? const Padding(
                        padding: EdgeInsets.only(top: 110),
                        child: Center(
                          child: Column(
                            children: [
                              CircularProgressIndicator(
                                semanticsLabel: 'Loading compute pool',
                              ),
                              SizedBox(height: 16),
                              Text('Reading devices and models...'),
                            ],
                          ),
                        ),
                      )
                    : Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          if (_connectionError) ...[
                            NoticeBanner(
                              message: _notice ??
                                  'The laptop is not responding. Pull to retry.',
                              tone: StatusTone.danger,
                            ),
                            const SizedBox(height: 12),
                          ],
                          AppPanel(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                LayoutBuilder(
                                  builder: (context, constraints) {
                                    final compact =
                                        constraints.maxWidth < 640 ||
                                            MediaQuery.textScalerOf(context)
                                                    .scale(1) >
                                                1.3;
                                    final summary = Row(
                                      children: [
                                        const AsciiGlyph(
                                          '[::]',
                                          semanticLabel: 'Compute pool',
                                        ),
                                        const SizedBox(width: 12),
                                        Expanded(
                                          child: Column(
                                            crossAxisAlignment:
                                                CrossAxisAlignment.start,
                                            children: [
                                              Text(
                                                running
                                                    ? 'Distributed model live'
                                                    : '${(_usableMb / 1024).toStringAsFixed(1)} GB ready to load',
                                                style:
                                                    theme.textTheme.titleLarge,
                                              ),
                                              Text(
                                                '$usableCount ${usableCount == 1 ? "device" : "devices"} contributing',
                                                style:
                                                    theme.textTheme.bodySmall,
                                              ),
                                            ],
                                          ),
                                        ),
                                      ],
                                    );
                                    final statusWidget = StatusPill(
                                      state,
                                      tone: running
                                          ? StatusTone.positive
                                          : state == 'failed'
                                              ? StatusTone.danger
                                              : StatusTone.neutral,
                                    );
                                    if (compact) {
                                      return Column(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.start,
                                        children: [
                                          summary,
                                          const SizedBox(height: 8),
                                          statusWidget,
                                        ],
                                      );
                                    }
                                    return Row(
                                      children: [
                                        Expanded(child: summary),
                                        const SizedBox(width: 12),
                                        statusWidget,
                                      ],
                                    );
                                  },
                                ),
                                if (_allocatedMb > 0) ...[
                                  const SizedBox(height: 14),
                                  Text(
                                    '${(_allocatedMb / 1024).toStringAsFixed(1)} GB currently assigned',
                                    style: theme.textTheme.bodyMedium,
                                  ),
                                ],
                              ],
                            ),
                          ),
                          const SectionGap(),
                          const SectionHeader(
                            'Devices',
                            subtitle: 'Only reachable, enabled workers count.',
                          ),
                          const SizedBox(height: 12),
                          if (_devices.isEmpty)
                            const AppPanel(
                              child: NoticeBanner(
                                message:
                                    'No devices were reported by the laptop node.',
                              ),
                            )
                          else
                            ..._devices.map(
                              (device) => Padding(
                                padding: const EdgeInsets.only(bottom: 10),
                                child: _deviceCard(context, device),
                              ),
                            ),
                          const SectionGap(),
                          const SectionHeader(
                            'Model placement',
                            subtitle:
                                'Plan first, then load only when every layer fits.',
                          ),
                          const SizedBox(height: 12),
                          AppPanel(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.stretch,
                              children: [
                                if (_models.isEmpty)
                                  const NoticeBanner(
                                    message:
                                        'No GGUF model is ready. Install one from the laptop model picker, then refresh.',
                                    tone: StatusTone.warning,
                                  )
                                else
                                  DropdownButtonFormField<String>(
                                    value: _selectedModel,
                                    isExpanded: true,
                                    elevation: 0,
                                    dropdownColor: AppColors.canvas,
                                    borderRadius: BorderRadius.circular(4),
                                    decoration: const InputDecoration(
                                      labelText: 'Model',
                                    ),
                                    items: _models
                                        .map(
                                          (model) => DropdownMenuItem(
                                            value: model,
                                            child: Text(
                                              model,
                                              overflow: TextOverflow.ellipsis,
                                            ),
                                          ),
                                        )
                                        .toList(),
                                    onChanged: _busy
                                        ? null
                                        : (value) => setState(() {
                                              _selectedModel = value;
                                              _plan = null;
                                            }),
                                  ),
                                if (_models.isNotEmpty) ...[
                                  const SizedBox(height: 12),
                                  DropdownButtonFormField<int>(
                                    value: _contextTokens,
                                    elevation: 0,
                                    dropdownColor: C.surface2,
                                    borderRadius: BorderRadius.circular(R.md),
                                    decoration: const InputDecoration(
                                      labelText: 'Context size',
                                    ),
                                    items: const [1024, 2048, 4096, 8192, 16384]
                                        .map((value) => DropdownMenuItem(
                                              value: value,
                                              child: Text('$value tokens'),
                                            ))
                                        .toList(),
                                    onChanged: _busy
                                        ? null
                                        : (value) => setState(() {
                                              _contextTokens = value!;
                                              _plan = null;
                                            }),
                                  ),
                                ],
                                const SizedBox(height: 14),
                                ResponsiveActions(
                                  children: [
                                    OutlinedButton(
                                      onPressed: _busy || _selectedModel == null
                                          ? null
                                          : _planSplit,
                                      child: const Text('Preview split'),
                                    ),
                                    FilledButton(
                                      onPressed: _busy || _plan?.fits != true
                                          ? null
                                          : _startDistributed,
                                      child: const Text('Load model'),
                                    ),
                                  ],
                                ),
                                if (running) ...[
                                  const SizedBox(height: 10),
                                  TextButton(
                                    onPressed: _busy ? null : _stop,
                                    child: const Text('Stop active model'),
                                  ),
                                ],
                              ],
                            ),
                          ),
                          if (_plan != null) ...[
                            const SizedBox(height: 12),
                            _planCard(context, _plan!),
                          ],
                          if (_notice != null && !_connectionError) ...[
                            const SizedBox(height: 12),
                            NoticeBanner(
                              message: _notice!,
                              tone: _noticeError
                                  ? StatusTone.danger
                                  : StatusTone.accent,
                            ),
                          ],
                          if (_busy) ...[
                            const SizedBox(height: 16),
                            const LinearProgressIndicator(
                              semanticsLabel:
                                  'Compute pool operation in progress',
                            ),
                          ],
                        ],
                      ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _deviceCard(BuildContext context, PoolDevice device) {
    final theme = Theme.of(context);
    final status = !device.enabled
        ? 'Excluded'
        : device.inUse
            ? 'In use'
            : device.usable
                ? 'Ready'
                : device.stale
                    ? 'Silent'
                    : 'Unavailable';
    final tone = device.inUse || device.usable
        ? StatusTone.positive
        : !device.enabled
            ? StatusTone.neutral
            : StatusTone.danger;
    final reason = !device.enabled
        ? 'Excluded in the laptop TUI'
        : device.inUse
            ? 'Hosting ${device.assignedLayers} model layers'
            : device.reason ??
                (device.isSelf
                    ? 'Laptop coordinator'
                    : device.reachable
                        ? 'RPC worker reachable'
                        : 'No RPC worker answered');

    return AppPanel(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          LayoutBuilder(
            builder: (context, constraints) {
              final compact = constraints.maxWidth < 640 ||
                  MediaQuery.textScalerOf(context).scale(1) > 1.3;
              final identity = Row(
                children: [
                  AsciiGlyph(
                    device.isSelf ? '[PC]' : '[PH]',
                    semanticLabel: device.isSelf ? 'Laptop' : 'Phone',
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(device.name, style: theme.textTheme.titleMedium),
                        Text(
                          '${device.kind} / $reason',
                          style: theme.textTheme.bodySmall,
                        ),
                      ],
                    ),
                  ),
                ],
              );
              final statusWidget = StatusPill(status, tone: tone);
              if (compact) {
                return Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    identity,
                    const SizedBox(height: 8),
                    statusWidget,
                  ],
                );
              }
              return Row(
                children: [
                  Expanded(child: identity),
                  const SizedBox(width: 12),
                  statusWidget,
                ],
              );
            },
          ),
          const SizedBox(height: 14),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              _compactMetric(
                context,
                '${(device.availableMb / 1024).toStringAsFixed(1)} GB free',
              ),
              _compactMetric(
                context,
                '${(device.budgetMb / 1024).toStringAsFixed(1)} GB cap',
              ),
              _compactMetric(context, '${device.threads}t / ${device.cores}c'),
              if (device.assignedMemoryMb > 0)
                _compactMetric(
                  context,
                  '${(device.assignedMemoryMb / 1024).toStringAsFixed(1)} GB assigned',
                ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _compactMetric(BuildContext context, String value) {
    return Padding(
      padding: const EdgeInsets.only(right: 12),
      child: Text('[-] $value', style: Theme.of(context).textTheme.labelMedium),
    );
  }

  Widget _planCard(BuildContext context, SplitPlan plan) {
    final theme = Theme.of(context);
    return AppPanel(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Text('Split plan', style: theme.textTheme.titleMedium),
              ),
              StatusPill(
                plan.fits ? 'Fits' : '${plan.shortfallMb} MB short',
                tone: plan.fits ? StatusTone.positive : StatusTone.danger,
              ),
            ],
          ),
          const SizedBox(height: 8),
          Text(plan.summary),
          const SizedBox(height: 14),
          ...plan.rows.map(
            (row) => Padding(
              padding: const EdgeInsets.only(bottom: 10),
              child: Semantics(
                label:
                    '${row.deviceName}, ${row.layers} of ${plan.totalLayers} layers',
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            row.deviceName,
                            overflow: TextOverflow.ellipsis,
                            style: theme.textTheme.labelLarge,
                          ),
                        ),
                        Text('${row.layers}/${plan.totalLayers} layers'),
                      ],
                    ),
                    const SizedBox(height: 5),
                    LinearProgressIndicator(
                      value: plan.totalLayers == 0
                          ? 0
                          : row.layers / plan.totalLayers,
                      minHeight: 7,
                      borderRadius: BorderRadius.zero,
                    ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
