import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:http/http.dart' as http;
import 'package:flutter/services.dart';

/// Finds localhost.ai-LAN nodes on the local network.
///
/// Two strategies, results merged:
///  1. UDP broadcast to :50897 (fast, but many APs / phones drop broadcasts)
///  2. HTTP probe of every host in the phone's /24 subnet on the node port
///     (slower, ~1-2s, works wherever unicast HTTP works)
///
/// Web builds can't do either — the UI offers manual node entry there.
class NodeDiscovery {
  static const _channel = MethodChannel('localhostai/native');
  static const int discoveryPort = 50897;
  static const int nodePort = 8765;
  static const String magic = 'LOCALHOSTAI_DISCOVER';

  Future<List<Map<String, dynamic>>> discover({
    Duration timeout = const Duration(seconds: 2),
  }) async {
    // Browsers have no raw sockets — UDP and subnet scans can't run on web.
    if (kIsWeb) return const [];
    final found = <String, Map<String, dynamic>>{};

    final udp = _udpDiscover(timeout: timeout)
        .catchError((_) => <Map<String, dynamic>>[]);
    final scan = _scanSubnet().catchError((_) => <Map<String, dynamic>>[]);
    final results = await Future.wait([udp, scan]);

    for (final node in [...results[0], ...results[1]]) {
      final key = '${node['address']}:${node['port']}';
      found.putIfAbsent(key, () => node);
    }
    return found.values.toList();
  }

  /// Strategy 1: UDP broadcast, node replies unicast.
  Future<List<Map<String, dynamic>>> _udpDiscover({
    required Duration timeout,
  }) async {
    final results = <Map<String, dynamic>>[];
    final seen = <String>{};
    final sockets = <RawDatagramSocket>[];

    try {
      final interfaces = await NetworkInterface.list(
        includeLoopback: false,
        type: InternetAddressType.IPv4,
      );
      for (final iface in interfaces) {
        for (final address in iface.addresses) {
          try {
            final sock = await RawDatagramSocket.bind(address, 0);
            sock.broadcastEnabled = true;
            sockets.add(sock);
            sock.send(utf8.encode(magic), InternetAddress('255.255.255.255'),
                discoveryPort);
          } catch (_) {}
        }
      }

      final deadline = DateTime.now().add(timeout);
      while (DateTime.now().isBefore(deadline)) {
        for (final sock in sockets) {
          final dg = sock.receive();
          if (dg == null) continue;
          try {
            final node =
                jsonDecode(utf8.decode(dg.data)) as Map<String, dynamic>;
            final port = node['port'];
            if (port is! int ||
                port <= 0 ||
                port > 65535 ||
                node['name'] is! String) continue;
            final key = '${dg.address.address}:$port';
            if (seen.add(key)) {
              node['address'] = dg.address.address;
              results.add(node);
            }
          } catch (_) {}
        }
        await Future.delayed(const Duration(milliseconds: 50));
      }
    } catch (_) {
      // Broadcast blocked — the subnet scan still runs.
    } finally {
      for (final s in sockets) {
        s.close();
      }
    }
    return results;
  }

  /// Strategy 2: probe x.y.z.1–254 on [nodePort] for /health.
  Future<List<Map<String, dynamic>>> _scanSubnet() async {
    final network = await _localNetwork();
    final me = network.address;
    if (me == null) return [];
    final octets = me.split('.');
    if (octets.length != 4) return [];
    final bits = network.prefix.clamp(22, 30);
    final value =
        octets.fold<int>(0, (value, octet) => (value << 8) | int.parse(octet));
    final mask = (0xffffffff << (32 - bits)) & 0xffffffff;
    final base = value & mask;
    final count = 1 << (32 - bits);

    final client = http.Client();
    try {
      final results = <Map<String, dynamic>>[];
      final addresses = <String>[];
      for (var i = 1; i < count - 1; i++) {
        final number = base + i;
        final ip =
            '${number >> 24 & 255}.${number >> 16 & 255}.${number >> 8 & 255}.${number & 255}';
        if (ip == me) continue;
        addresses.add(ip);
      }
      for (var offset = 0; offset < addresses.length; offset += 128) {
        final batch = addresses.skip(offset).take(128);
        results.addAll((await Future.wait(
                batch.map((ip) => _probeHealth(client, ip, nodePort))))
            .whereType<Map<String, dynamic>>());
        if (results.isNotEmpty) return results;
      }
      // Port fallback is bounded to the local /24-sized neighborhood. UDP QR
      // discovery remains authoritative for wider networks and arbitrary ports.
      for (var port = nodePort + 1;
          port < nodePort + 10 && addresses.length <= 254;
          port++) {
        results.addAll((await Future.wait(
                addresses.map((ip) => _probeHealth(client, ip, port))))
            .whereType<Map<String, dynamic>>());
        if (results.isNotEmpty) return results;
      }
      return results;
    } finally {
      // Future.timeout alone does not close the underlying HTTP sockets.
      client.close();
    }
  }

  Future<Map<String, dynamic>?> _probeHealth(
      http.Client client, String ip, int port) async {
    try {
      final r = await client
          .get(Uri.parse('http://$ip:$port/health'))
          .timeout(const Duration(milliseconds: 600));
      if (r.statusCode == 200) {
        final h = jsonDecode(r.body) as Map<String, dynamic>;
        if (h['name'] != null) {
          h['address'] = ip;
          h['port'] = port;
          return h;
        }
      }
    } catch (_) {
      // not a node / host down — ignore
    }
    return null;
  }

  Future<InternetAddress?> _localIPv4() async {
    try {
      final interfaces = await NetworkInterface.list(
        includeLoopback: false,
        type: InternetAddressType.IPv4,
      );
      for (final iface in interfaces) {
        for (final addr in iface.addresses) {
          if (!addr.isLoopback) return addr;
        }
      }
    } catch (_) {}
    return null;
  }

  Future<({String? address, int prefix})> _localNetwork() async {
    if (Platform.isAndroid) {
      try {
        final value =
            await _channel.invokeMapMethod<String, dynamic>('lanNetwork');
        final address = value?['address'] as String?;
        final prefix = value?['prefix'] as int?;
        if (address != null && prefix != null) {
          return (address: address, prefix: prefix);
        }
      } catch (_) {}
    }
    final address = await _localIPv4();
    return (address: address?.address, prefix: 24);
  }
}
