import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../services/node_client.dart';
import '../../theme/theme.dart';
import '../../theme/tokens.dart';
import '../../widgets/app_card.dart';
import '../../widgets/common.dart';
import '../connection/connection_controller.dart';

/// RAM sliders for the paired laptop and this phone.
///
/// The laptop reserve is what the coordinator keeps for its own OS and apps;
/// the phone reserve is what this device keeps free for Android before
/// offering the rest to the pool. Both apply to the next model load.
class MemorySettings extends ConsumerStatefulWidget {
  const MemorySettings({super.key});
  @override
  ConsumerState<MemorySettings> createState() => _MemorySettingsState();
}

class _MemorySettingsState extends ConsumerState<MemorySettings> {
  LaptopMemory? _memory;
  bool _loading = true;
  bool _saving = false;
  String? _notice;
  bool _noticeError = false;

  int? _laptopDraft;
  int? _phoneDraft;

  /// Slider bounds and step, in MiB. Wide enough for small phones and
  /// big-desktop reserves, coarse enough to be meaningful.
  static const double _reserveMinMb = 256;
  static const double _reserveMaxMb = 8192;
  static const int _reserveDivisions = (8192 - 256) ~/ 128;

  ConnectionController get _connection => ref.read(connectionProvider);

  @override
  void initState() {
    super.initState();
    _phoneDraft = _connection.phoneReserveMb;
    _load();
  }

  Future<void> _load() async {
    final client = _connection.client;
    if (client == null) {
      if (mounted) setState(() => _loading = false);
      return;
    }
    final memory = await client.memoryConfig();
    if (!mounted) return;
    setState(() {
      _memory = memory;
      _loading = false;
      _laptopDraft ??= memory?.effectiveReserveMb;
    });
  }

  Future<void> _saveLaptopReserve(double value) async {
    final client = _connection.client;
    if (client == null) return;
    final mb = value.round();
    setState(() {
      _saving = true;
      _laptopDraft = mb;
    });
    final updated = await client.setMemoryConfig(reserveMb: mb);
    if (!mounted) return;
    setState(() {
      _saving = false;
      if (updated != null) {
        _memory = updated;
        _laptopDraft = updated.reserveMb ?? updated.effectiveReserveMb;
        _notice = 'Laptop reserve saved. Applies to the next model load.';
        _noticeError = false;
      } else {
        _notice =
            'The laptop did not accept the new reserve. Check the connection and try again.';
        _noticeError = true;
      }
    });
  }

  Future<void> _savePhoneReserve(double value) async {
    final mb = value.round();
    setState(() => _phoneDraft = mb);
    await _connection.setPhoneReserveMb(mb);
    if (!mounted) return;
    setState(() {
      _notice = 'Phone reserve saved. Sharing picks it up within seconds.';
      _noticeError = false;
    });
  }

  /// Budget a device would offer with [reserve] kept free — the same
  /// `min(total, available) - reserve` rule the coordinator applies.
  static int _budgetMb(int totalMb, int availableMb, int reserve) {
    final totalCapacity = totalMb > reserve ? totalMb - reserve : 0;
    final freeCapacity = availableMb > reserve ? availableMb - reserve : 0;
    return totalCapacity < freeCapacity ? totalCapacity : freeCapacity;
  }

  Widget _reserveRow(
    TextTheme theme, {
    required String title,
    required int valueMb,
    required int offeredMb,
    required int totalMb,
    required ValueChanged<double> onChanged,
    required ValueChanged<double> onChangeEnd,
  }) {
    final clamped = valueMb.clamp(_reserveMinMb.toInt(), _reserveMaxMb.toInt());
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Expanded(child: Text(title, style: theme.titleSmall)),
            Text(
              '${(clamped / 1024).toStringAsFixed(1)} GB',
              style: mono(13, color: C.accent, w: FontWeight.w600),
            ),
          ],
        ),
        Slider(
          value: clamped.toDouble(),
          min: _reserveMinMb,
          max: _reserveMaxMb,
          divisions: _reserveDivisions,
          label: '${(clamped / 1024).toStringAsFixed(1)} GB',
          onChanged: _saving ? null : onChanged,
          onChangeEnd: _saving ? null : onChangeEnd,
        ),
        Text(
          'keeps ${(clamped / 1024).toStringAsFixed(1)} GB for itself · '
          'offers ${(offeredMb / 1024).toStringAsFixed(1)} GB of '
          '${(totalMb / 1024).toStringAsFixed(1)} GB to the pool',
          style: theme.bodySmall,
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context).textTheme;
    final connection = ref.watch(connectionProvider);
    final memory = _memory;
    final laptop =
        (memory != null && memory.totalMb > 0 && connection.client != null)
            ? memory
            : null;
    final laptopValue = _laptopDraft ?? memory?.effectiveReserveMb ?? 0;
    final phoneValue = _phoneDraft ?? connection.phoneReserveMb;

    final children = <Widget>[];
    if (_loading) {
      children.add(const Padding(
        padding: EdgeInsets.symmetric(vertical: 16),
        child: Center(
          child: SizedBox(
            width: 20,
            height: 20,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        ),
      ));
    } else if (connection.client == null) {
      children.add(Text(
        'Pair a laptop to set its RAM reserve.',
        style: theme.bodySmall,
      ));
    }
    if (laptop != null) {
      if (children.isNotEmpty) children.add(const SizedBox(height: 8));
      children.add(_reserveRow(
        theme,
        title: 'Laptop reserve',
        valueMb: laptopValue,
        offeredMb: laptop.budgetForReserveMb(laptopValue),
        totalMb: laptop.totalMb,
        onChanged: (value) => setState(() => _laptopDraft = value.round()),
        onChangeEnd: _saveLaptopReserve,
      ));
    }
    if (children.isNotEmpty) children.add(const SizedBox(height: 8));
    children.add(_reserveRow(
      theme,
      title: 'Phone reserve',
      valueMb: phoneValue,
      offeredMb: _budgetMb(connection.profile.ramMb,
          connection.profile.availableMb, phoneValue),
      totalMb: connection.profile.ramMb,
      onChanged: (value) => setState(() => _phoneDraft = value.round()),
      onChangeEnd: _savePhoneReserve,
    ));
    if (_notice != null) {
      children.add(const SizedBox(height: 8));
      children.add(NoticeBanner(
          message: _notice!,
          tone: _noticeError ? StatusTone.danger : StatusTone.positive));
    }

    return AppCard(
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text('Memory', style: mono(12, color: C.text, w: FontWeight.w500)),
      const SizedBox(height: 12),
      ...children,
    ]));
  }
}
