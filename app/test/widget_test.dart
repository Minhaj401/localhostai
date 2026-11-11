import 'package:flutter_test/flutter_test.dart';
import 'package:privatelm_lan/models/device_profile.dart';
import 'package:privatelm_lan/models/pairing.dart';

void main() {
  group('PairPayload', () {
    test('parses the URI the laptop generates', () {
      // Produced by `core/src/qr.rs` — must stay in sync with it.
      const raw = 'privateml://pair?h=192.168.1.20&p=8765'
          '&t=0123456789abcdef0123456789abcdef&n=workstation';
      final p = PairPayload.tryParse(raw);
      expect(p, isNotNull);
      expect(p!.host, '192.168.1.20');
      expect(p.port, 8765);
      expect(p.token, '0123456789abcdef0123456789abcdef');
      expect(p.name, 'workstation');
      expect(p.baseUrl, 'http://192.168.1.20:8765');
    });

    test('rejects foreign QR codes instead of guessing', () {
      expect(PairPayload.tryParse('https://example.com'), isNull);
      expect(PairPayload.tryParse('hello'), isNull);
      expect(PairPayload.tryParse('privateml://pair?h=1.2.3.4&p=8765'), isNull);
      expect(PairPayload.tryParse('privateml://pair?h=&p=8765&t=x'), isNull);
      expect(
          PairPayload.tryParse('privateml://pair?h=a&p=notaport&t=x'), isNull);
      expect(PairPayload.tryParse('privateml://pair?h=a&p=99999&t=x'), isNull);
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
        'privateml://pair.evil?h=host&p=8765&t=x',
        'privateml://pair/extra?h=host&p=8765&t=x',
        'privateml://pair?h=%FF&p=8765&t=x',
        'privateml://pair?h=host&p=0&t=x',
        'privateml://pair?h=host%2Fpath&p=8765&t=x',
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
