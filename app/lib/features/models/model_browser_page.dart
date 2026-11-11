import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:lucide_icons/lucide_icons.dart';

import '../../models/remote_model.dart';
import '../../theme/theme.dart';
import '../../widgets/app_card.dart';
import '../../widgets/buttons.dart';
import '../../widgets/common.dart';
import '../connection/connection_controller.dart';

class ModelBrowserPage extends ConsumerStatefulWidget {
  const ModelBrowserPage({super.key});

  @override
  ConsumerState<ModelBrowserPage> createState() => _ModelBrowserPageState();
}

class _ModelBrowserPageState extends ConsumerState<ModelBrowserPage> {
  final _query = TextEditingController();
  HubResults? _results;
  String? _error;
  bool _searching = false;
  int _requestId = 0;

  Future<void> _search(String query) async {
    final client = ref.read(connectionProvider).client;
    if (client == null) return;
    final id = ++_requestId;
    _query.text = query.trim();
    FocusScope.of(context).unfocus();
    setState(() {
      _searching = true;
      _error = null;
      _results = null;
    });
    try {
      final result = await client.browseModels(query.trim());
      if (!mounted ||
          id != _requestId ||
          !identical(client, ref.read(connectionProvider).client)) return;
      setState(() => _results = result);
    } catch (failure) {
      if (mounted &&
          id == _requestId &&
          identical(client, ref.read(connectionProvider).client)) {
        setState(() => _error = cleanError(failure));
      }
    } finally {
      if (mounted && id == _requestId) setState(() => _searching = false);
    }
  }

  Future<void> _download(RemoteModel model) async {
    final success = await ref.read(connectionProvider).download(model);
    if (success && mounted) context.pop();
  }

  @override
  void dispose() {
    _requestId++;
    _query.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final node = ref.watch(connectionProvider);
    final repositories = _results?.repositories ?? const <String>[];
    final models = _results?.models ?? const <RemoteModel>[];
    return Scaffold(
      appBar: AppBar(title: const Text('Find models')),
      body: SafeArea(
        top: false,
        child: Align(
          alignment: Alignment.topCenter,
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 1120),
            child: ListView.builder(
              padding: const EdgeInsets.all(24),
              keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
              itemCount: 1 + repositories.length + models.length,
              itemBuilder: (context, index) {
                if (index == 0) return _header(node);
                if (index <= repositories.length) {
                  final repository = repositories[index - 1];
                  return Padding(
                    padding: const EdgeInsets.only(bottom: 16),
                    child: AppCard(
                      onTap: _searching ? null : () => _search(repository),
                      child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            Text(repository,
                                style: Theme.of(context).textTheme.titleMedium),
                            const SizedBox(height: 12),
                            SecondaryButton(
                                label: 'Choose GGUF',
                                icon: LucideIcons.folderOpen,
                                onPressed: _searching
                                    ? null
                                    : () => _search(repository)),
                          ]),
                    ),
                  );
                }
                final model = models[index - repositories.length - 1];
                return Padding(
                  padding: const EdgeInsets.only(bottom: 16),
                  child: AppCard(
                      child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                        Text(model.name,
                            style: Theme.of(context).textTheme.titleLarge),
                        const SizedBox(height: 8),
                        Text('${model.sizeLabel} / ${model.fileCount} file(s)',
                            style: mono(14)),
                        if (model.description.isNotEmpty) ...[
                          const SizedBox(height: 8),
                          Text(model.description),
                        ],
                        if (model.license.isNotEmpty) ...[
                          const SizedBox(height: 8),
                          Text(model.license, style: mono(12)),
                        ],
                        const SizedBox(height: 16),
                        SecondaryButton(
                            label: 'Download GGUF',
                            icon: LucideIcons.download,
                            onPressed: !node.connected ||
                                    node.operationBusy ||
                                    node.installing
                                ? null
                                : () => _download(model)),
                      ])),
                );
              },
            ),
          ),
        ),
      ),
    );
  }

  Widget _header(ConnectionController node) => Padding(
        padding: const EdgeInsets.only(bottom: 24),
        child:
            Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          const SectionHeader('Your choice of model',
              subtitle:
                  'Search Hugging Face, enter owner/repository, or paste a GGUF file link. Downloads are stored on your laptop.'),
          const SizedBox(height: 24),
          TextField(
            key: const ValueKey('model-hub-query'),
            controller: _query,
            textInputAction: TextInputAction.search,
            autocorrect: false,
            maxLength: 4096,
            decoration: const InputDecoration(
                labelText: 'Search or GGUF link',
                hintText: 'Model name, owner/repo, or https://...',
                counterText: ''),
            onSubmitted: node.connected ? _search : null,
            onChanged: (_) => setState(() {
              _requestId++;
              _searching = false;
              _error = null;
              _results = null;
            }),
          ),
          const SizedBox(height: 12),
          PrimaryButton(
              label: _searching ? 'Searching...' : 'Search models',
              icon: LucideIcons.search,
              onPressed: !node.connected || _searching
                  ? null
                  : () => _search(_query.text)),
          const SizedBox(height: 12),
          const Text(
              'Leave search empty for popular repositories. Choose a quantization and download all its shards together.'),
          if (_searching) ...[
            const SizedBox(height: 16),
            const LinearProgressIndicator(
                semanticsLabel: 'Searching model repositories'),
          ],
          if (!node.connected) ...[
            const SizedBox(height: 16),
            const NoticeBanner(
                message: 'Connect the laptop to browse and download models.',
                tone: StatusTone.warning),
          ],
          if (_error != null) ...[
            const SizedBox(height: 16),
            NoticeBanner(message: _error!, tone: StatusTone.danger),
          ],
          if (node.operationBusy ||
              node.installing ||
              node.operationError != null) ...[
            const SizedBox(height: 16),
            NoticeBanner(
                message:
                    node.operationMessage ?? 'Downloading to the laptop...',
                tone: node.operationError == null
                    ? StatusTone.accent
                    : StatusTone.danger),
            if (node.operationBusy || node.installing) ...[
              const SizedBox(height: 12),
              LinearProgressIndicator(
                value: node.installing && node.totalBytes > 0
                    ? (node.downloadedBytes / node.totalBytes).clamp(0.0, 1.0)
                    : null,
                semanticsLabel: 'Downloading model to laptop',
              ),
              if (node.installing && node.totalBytes > 0) ...[
                const SizedBox(height: 8),
                Text(
                    '${(node.downloadedBytes / (1024 * 1024)).toStringAsFixed(0)} / '
                    '${(node.totalBytes / (1024 * 1024)).toStringAsFixed(0)} MiB',
                    style: mono(12)),
              ],
            ],
          ],
          if (_results != null) ...[
            const SizedBox(height: 24),
            Text(_results!.repository ?? 'Search results',
                style: Theme.of(context).textTheme.titleLarge),
            if (_results!.repositories.isEmpty && _results!.models.isEmpty) ...[
              const SizedBox(height: 12),
              const Text(
                  'No complete GGUF models found. Try another search or repository.'),
            ],
          ],
        ]),
      );
}
