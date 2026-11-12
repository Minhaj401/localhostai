import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../../models/managed_model.dart';
import '../../theme/theme.dart';
import '../../theme/tokens.dart';
import '../../widgets/app_card.dart';
import '../../widgets/brand.dart';
import '../../widgets/buttons.dart';
import '../../widgets/common.dart';
import '../../widgets/reveal.dart';
import '../../widgets/terminal_window.dart';
import '../connection/connection_controller.dart';

class ModelsPage extends ConsumerStatefulWidget {
  const ModelsPage({super.key});

  @override
  ConsumerState<ModelsPage> createState() => _ModelsPageState();
}

class _ModelsPageState extends ConsumerState<ModelsPage> {
  String _query = '';
  bool _installedOnly = false;

  @override
  Widget build(BuildContext context) {
    final node = ref.watch(connectionProvider);
    final visible = node.models
        .where((model) =>
            (!_installedOnly || model.installed) &&
            '${model.name} ${model.quantization}'
                .toLowerCase()
                .contains(_query.toLowerCase()))
        .toList();
    final selected = node.models
        .where((model) => model.id == node.selectedModel)
        .firstOrNull;
    return Scaffold(
      appBar: AppBar(title: const Text('Models'), actions: [
        IconButton(
            tooltip: 'Refresh models',
            onPressed: node.refreshing ? null : node.refresh,
            icon: const Icon(LucideIcons.refreshCw, size: 20)),
      ]),
      body: SafeArea(
          top: false,
          child: RefreshIndicator(
            onRefresh: node.refresh,
            child: ListView(
              physics: const AlwaysScrollableScrollPhysics(),
              keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
              children: [
                AdaptiveContent(
                    child: Reveal(
                        child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    const SectionHeader('Your model library',
                        eyebrow: 'Models',
                        subtitle:
                            'Choose your own GGUFs. Download once, run on your own devices.'),
                    const SizedBox(height: 24),
                    if (node.error != null) ...[
                      NoticeBanner(
                          message: node.error!, tone: StatusTone.danger),
                      const SizedBox(height: 16),
                    ],
                    if (node.client == null) ...[
                      _unpaired(context),
                    ] else ...[
                      SecondaryButton(
                        label: 'Find and download models',
                        icon: LucideIcons.search,
                        onPressed: node.connected
                            ? () => context.push('/models/browse')
                            : null,
                      ),
                      const SizedBox(height: 16),
                      TextField(
                        onChanged: (value) => setState(() => _query = value),
                        decoration: const InputDecoration(
                          hintText: 'Filter your library or quantization',
                          prefixIcon: Icon(LucideIcons.search, size: 18),
                        ),
                      ),
                      const SizedBox(height: 12),
                      Wrap(spacing: 8, runSpacing: 8, children: [
                        ChoiceChip(
                            label: const Text('All models'),
                            selected: !_installedOnly,
                            showCheckmark: false,
                            selectedColor: C.accentDim,
                            onSelected: (_) =>
                                setState(() => _installedOnly = false)),
                        ChoiceChip(
                            label: const Text('Installed'),
                            selected: _installedOnly,
                            showCheckmark: false,
                            selectedColor: C.accentDim,
                            onSelected: (_) =>
                                setState(() => _installedOnly = true)),
                      ]),
                      const SizedBox(height: 24),
                      if (node.refreshing && node.models.isEmpty)
                        const Padding(
                            padding: EdgeInsets.all(24),
                            child: Center(
                                child: CircularProgressIndicator(
                                    semanticsLabel: 'Reading model catalog')))
                      else if (visible.isEmpty)
                        const AppCard(
                            child: Text('No models match this filter.'))
                      else
                        ...visible.map((model) => Padding(
                              padding: const EdgeInsets.only(bottom: 12),
                              child: _modelCard(node, model),
                            )),
                      if (selected != null) ...[
                        const SizedBox(height: 12),
                        _selection(node, selected),
                      ],
                      if (node.operationMessage != null) ...[
                        const SizedBox(height: 16),
                        NoticeBanner(
                            message: node.operationMessage!,
                            tone: node.operationError == null
                                ? StatusTone.accent
                                : StatusTone.danger),
                      ],
                    ],
                  ],
                )))
              ],
            ),
          )),
    );
  }

  Widget _unpaired(BuildContext context) => AppBackdrop(
        hero: true,
        child: Column(children: [
          const SizedBox(height: 24),
          const Brand(size: 30),
          const SizedBox(height: 24),
          const TerminalWindow(
              title: 'model catalog',
              child: Text(
                  '>_ localhostai\n\nChoose a model on the laptop,\nthen pair this phone with Ctrl+P.')),
          const SizedBox(height: 24),
          PrimaryButton(
              label: 'Connect a laptop',
              icon: LucideIcons.link,
              onPressed: () => context.go('/server')),
        ]),
      );

  Widget _modelCard(ConnectionController node, ManagedModel model) {
    final active =
        node.runningModel == model.id && node.clusterState == 'running';
    final downloading = node.installing && node.downloadingModel == model.id;
    final selected = node.selectedModel == model.id;
    return AppCard(
      selected: selected,
      onTap: () {
        node.selectModel(model.id);
        if (node.connected && !node.operationBusy && model.canPreview) {
          unawaited(node.preview(model.id));
        }
      },
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Container(
            padding: const EdgeInsets.all(10),
            decoration: BoxDecoration(
                color: selected ? C.accentDim : C.surface2,
                borderRadius: BorderRadius.circular(R.md)),
            child: Icon(LucideIcons.box,
                size: 20, color: selected ? C.accent : C.text2),
          ),
          const SizedBox(width: 12),
          Expanded(
              child: Text(model.name,
                  style: Theme.of(context).textTheme.titleLarge)),
          const SizedBox(width: 8),
          Icon(selected ? LucideIcons.checkCircle2 : LucideIcons.circle,
              size: 18, color: selected ? C.accent : C.text3),
        ]),
        const SizedBox(height: 12),
        Wrap(spacing: 12, runSpacing: 8, children: [
          Text('${model.sizeLabel} / ${model.quantization}', style: mono(13)),
          StatusPill(
              active
                  ? 'Running'
                  : downloading
                      ? 'Downloading'
                      : model.installed
                          ? 'Installed'
                          : 'Download',
              tone: active
                  ? StatusTone.positive
                  : downloading
                      ? StatusTone.warning
                      : StatusTone.neutral),
        ]),
        if (model.description.isNotEmpty) ...[
          const SizedBox(height: 12),
          Text(model.description),
        ],
        if (downloading) ...[
          const SizedBox(height: 16),
          LinearProgressIndicator(
            minHeight: 2,
            value: node.totalBytes > 0
                ? (node.downloadedBytes / node.totalBytes).clamp(0.0, 1.0)
                : null,
            semanticsLabel: 'Downloading ${model.name}',
          ),
          const SizedBox(height: 8),
          Text(
              '${(node.downloadedBytes / (1024 * 1024)).toStringAsFixed(0)} / '
              '${(node.totalBytes / (1024 * 1024)).toStringAsFixed(0)} MB',
              style: mono(12)),
        ],
      ]),
    );
  }

  Widget _selection(ConnectionController node, ManagedModel model) => AppCard(
        child:
            Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Text('Placement', style: Theme.of(context).textTheme.titleLarge),
          const SizedBox(height: 8),
          Text('Laptop first. Phone layers only when needed.', style: mono(12)),
          const SizedBox(height: 16),
          DropdownButtonFormField<int>(
            value: node.contextTokens,
            dropdownColor: C.surface2,
            borderRadius: BorderRadius.circular(R.md),
            elevation: 0,
            decoration: const InputDecoration(labelText: 'Context size'),
            items: const [1024, 2048, 4096, 8192, 16384]
                .map((size) => DropdownMenuItem(
                    value: size, child: Text('$size tokens', style: mono(13))))
                .toList(),
            onChanged: node.operationBusy
                ? null
                : (value) {
                    if (value == null) return;
                    node.selectContext(value);
                    unawaited(node.preview(model.id));
                  },
          ),
          if (node.lastPlan != null && node.lastPlan!.model == model.id) ...[
            const SizedBox(height: 16),
            NoticeBanner(
                message: node.lastPlan!.summary,
                tone: node.lastPlan!.fits
                    ? StatusTone.positive
                    : StatusTone.warning),
          ],
          const SizedBox(height: 16),
          if (!model.canPreview) ...[
            const Text(
                'Download this GGUF first. Its metadata will be checked before planning and loading.'),
            const SizedBox(height: 16),
          ],
          PrimaryButton(
              label: node.operationBusy
                  ? 'Preparing model...'
                  : model.canPreview
                      ? 'Load model'
                      : 'Download model',
              icon: LucideIcons.play,
              onPressed:
                  !node.connected || node.operationBusy || node.installing
                      ? null
                      : () => node.install(model, load: model.canPreview)),
          if (model.catalog && model.canPreview) ...[
            const SizedBox(height: 8),
            SecondaryButton(
                label: model.installed ? 'Verify download' : 'Download only',
                icon: LucideIcons.download,
                onPressed:
                    !node.connected || node.operationBusy || node.installing
                        ? null
                        : () => node.install(model)),
          ],
          if (model.installed && node.runningModel != model.id) ...[
            const SizedBox(height: 8),
            TextButton.icon(
              icon: const Icon(LucideIcons.trash2, size: 18),
              label: const Text('Delete model'),
              onPressed: node.operationBusy
                  ? null
                  : () async {
                      final confirmed = await showDialog<bool>(
                        context: context,
                        builder: (context) => AlertDialog(
                          title: const Text('Delete this model?'),
                          content: Text(
                              '${model.name} will be removed from the laptop. Interrupted download data is removed too.'),
                          actions: [
                            TextButton(
                                onPressed: () => Navigator.pop(context, false),
                                child: const Text('Keep model')),
                            FilledButton(
                                onPressed: () => Navigator.pop(context, true),
                                child: const Text('Delete')),
                          ],
                        ),
                      );
                      if (confirmed == true) await node.deleteModel(model);
                    },
            ),
          ],
          if (model.license.isNotEmpty) ...[
            const SizedBox(height: 12),
            Text(model.license, style: mono(12)),
          ],
        ]),
      );
}
