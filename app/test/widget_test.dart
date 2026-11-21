import 'package:flutter_test/flutter_test.dart';
import 'package:localhostai/models/device_profile.dart';
import 'package:localhostai/models/pairing.dart';

void main() {
  group('PairPayload', () {
    test('parses the URI the laptop generates', () {
      // Produced by `core/src/qr.rs` — must stay in sync with it.
      const raw = 'localhostai://pair?h=192.168.1.20&p=8765'
          '&t=0123456789abcdef0123456789abcdef&n=workstation';
      final p = PairPayload.tryParse(raw);
      expect(p, isNotNull);
      expect(p!.host, '192.168.1.20');
      expect(p.port, 8765);
      expect(p.token, '0123456789abcdef0123456789abcdef');
      expect(p.name, 'workstation');
      expect(p.baseUrl, 'http://192.168.1.20:8765');
    });

    test('still parses the pre-rename privateml scheme', () {
      // QRs from laptop binaries built before the localhost.ai rename.
      const raw = 'privateml://pair?h=192.168.1.20&p=8765'
          '&t=0123456789abcdef0123456789abcdef&n=workstation';
      final p = PairPayload.tryParse(raw);
      expect(p, isNotNull);
      expect(p!.host, '192.168.1.20');
      expect(p.port, 8765);
      expect(p.baseUrl, 'http://192.168.1.20:8765');
    });

    test('parses params in any order, like the Rust parser does', () {
      // core/src/qr.rs splits on '&' and matches keys, so order must not matter.
      const raw = 'localhostai://pair?t=tok&p=9000&h=10.1.2.3&n=box';
      final p = PairPayload.tryParse(raw);
      expect(p, isNotNull);
      expect(p!.host, '10.1.2.3');
      expect(p.port, 9000);
      expect(p.token, 'tok');
      expect(p.name, 'box');
    });

    test('trims whitespace a scanner may leave around the URI', () {
      final p = PairPayload.tryParse(
          '  localhostai://pair?h=10.0.0.2&p=8765&t=tok&n=n  ');
      expect(p, isNotNull);
      expect(p!.host, '10.0.0.2');
    });

    test('accepts the full port range but nothing outside it', () {
      expect(PairPayload.tryParse('localhostai://pair?h=h&p=1&t=x'), isNotNull);
      expect(
          PairPayload.tryParse('localhostai://pair?h=h&p=65535&t=x'), isNotNull);
      expect(PairPayload.tryParse('localhostai://pair?h=h&p=0&t=x'), isNull);
      expect(
          PairPayload.tryParse('localhostai://pair?h=h&p=65536&t=x'), isNull);
      expect(PairPayload.tryParse('localhostai://pair?h=h&p=-1&t=x'), isNull);
    });

    test('falls back to the host as the node name when n is missing', () {
      final p = PairPayload.tryParse('localhostai://pair?h=10.0.0.2&p=8765&t=t');
      expect(p, isNotNull);
      expect(p!.name, '10.0.0.2');
      // An empty n is treated as missing, matching core/src/qr.rs.
      final empty =
          PairPayload.tryParse('localhostai://pair?h=10.0.0.2&p=8765&t=t&n=');
      expect(empty, isNotNull);
      expect(empty!.name, '10.0.0.2');
    });

    test('requires host, port and token — none may be missing', () {
      expect(PairPayload.tryParse('localhostai://pair?p=8765&t=x'), isNull);
      expect(PairPayload.tryParse('localhostai://pair?h=h&t=x'), isNull);
      expect(PairPayload.tryParse('localhostai://pair?h=h&p=8765'), isNull);
      expect(PairPayload.tryParse(''), isNull);
    });

    test('legacy privateml QRs round trip every field', () {
      const raw = 'privateml://pair?h=fe80%3A%3A1&p=8765'
          '&t=a%2Bb%26c&n=my%20old%20laptop';
      final p = PairPayload.tryParse(raw);
      expect(p, isNotNull);
      expect(p!.host, 'fe80::1');
      expect(p.token, 'a+b&c');
      expect(p.name, 'my old laptop');
      expect(p.baseUrl, 'http://[fe80::1]:8765');
    });

    test('a QR with an explicit port in the authority is rejected', () {
      // The laptop never emits one; a port there means a hand-crafted URI.
      expect(
          PairPayload.tryParse('localhostai://pair:8765?h=h&p=8765&t=x'),
          isNull);
    });

    test('fromJson throws on payloads the scanner would reject', () {
      expect(
          () => PairPayload.fromJson(
              {'host': '', 'port': 8765, 'token': 't', 'name': 'n'}),
          throwsFormatException);
      expect(
          () => PairPayload.fromJson(
              {'host': 'h', 'port': 0, 'token': 't', 'name': 'n'}),
          throwsFormatException);
    });

    test('rejects foreign QR codes instead of guessing', () {
      expect(PairPayload.tryParse('https://example.com'), isNull);
      expect(PairPayload.tryParse('hello'), isNull);
      expect(
          PairPayload.tryParse('localhostai://pair?h=1.2.3.4&p=8765'), isNull);
      expect(PairPayload.tryParse('localhostai://pair?h=&p=8765&t=x'), isNull);
      expect(PairPayload.tryParse('localhostai://pair?h=a&p=notaport&t=x'),
          isNull);
      expect(
          PairPayload.tryParse('localhostai://pair?h=a&p=99999&t=x'), isNull);
    });

    test('round trips through toJson/fromJson', () {
      const p =
          PairPayload(host: '10.0.0.5', port: 9000, token: 't', name: 'n');
      final back = PairPayload.fromJson(p.toJson());
      expect(back.host, p.host);
      expect(back.port, p.port);
      expect(back.token, p.token);
    });

    test('IPv6 and reserved token characters round trip', () {
      const pair = PairPayload(
          host: '2001:db8::1', port: 8765, token: 'a+b&c=你好', name: 'laptop');
      final parsed = PairPayload.tryParse(pair.toUri());
      expect(parsed?.token, pair.token);
      expect(parsed?.baseUrl, 'http://[2001:db8::1]:8765');
    });

    test('rejects misleading prefixes and broken percent encodings', () {
      for (final raw in [
        'localhostai://pair.evil?h=host&p=8765&t=x',
        'localhostai://pair/extra?h=host&p=8765&t=x',
        'localhostai://pair?h=%FF&p=8765&t=x',
        'localhostai://pair?h=host&p=0&t=x',
        'localhostai://pair?h=host%2Fpath&p=8765&t=x',
      ]) {
        expect(PairPayload.tryParse(raw), isNull, reason: raw);
      }
    });
  });

  group('DeviceProfile', () {
    test('budget subtracts a real reserve, never goes negative', () {
      final big = DeviceProfile.detect(name: 'big');
      expect(big.budgetMb, lessThanOrEqualTo(big.ramMb - 900));
      expect(big.budgetMb, lessThanOrEqualTo(big.availableMb));
      expect(DeviceProfile.shareBudgetMb(8000, 8000), 7100);
      expect(DeviceProfile.shareBudgetMb(8000, 3000), 2100);
      expect(DeviceProfile.shareBudgetMb(700, 700), 0);

      // The reserve must not be able to produce a negative budget on a tiny
      // device; that would report nonsense to the laptop.
      expect(big.ramMb > 900, isTrue);
    });

    test('thread tiers match the laptop rules', () {
      expect(DeviceProfile.tierThreads(16000, 8), 6);
      expect(DeviceProfile.tierThreads(8000, 8), 4);
      expect(DeviceProfile.tierThreads(4000, 8), 3);
      expect(DeviceProfile.tierThreads(2000, 8), 2);
    });

    test('threads are capped by core count', () {
      expect(DeviceProfile.tierThreads(16000, 2), 2);
      expect(DeviceProfile.tierThreads(16000, 1), 1);
    });

    test('reports an rpc address only when one is supplied', () {
      final p = DeviceProfile.detect(name: 'phone');
      expect(p.availableMb, lessThanOrEqualTo(p.ramMb));
      expect(p.toJson().containsKey('rpc'), isFalse);
      expect(p.toJson()['available_mb'], p.availableMb);
      expect(p.toJson(rpc: '10.0.0.9:50052')['rpc'], '10.0.0.9:50052');
    });

    test('device identities do not collide on identical hardware', () {
      final firstId = DeviceProfile.createId();
      final secondId = DeviceProfile.createId();
      expect(firstId, isNot(secondId));
      expect(DeviceProfile.detect(name: 'first', id: firstId).id, firstId);
      expect(DeviceProfile.detect(name: 'second', id: secondId).id, secondId);
    });
  });
}
