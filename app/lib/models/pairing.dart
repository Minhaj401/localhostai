/// Pairing payload carried by the QR code.
///
/// The laptop renders a QR containing a `localhostai://pair?...` URI. Scanning it
/// is the only setup step a user performs: address, port and secret all arrive
/// in one string, so there is no manual IP typing and no shared-key copying.
///
/// Format (must match `core/src/qr.rs`):
///   localhostai://pair?h=<host>&p=<port>&t=<token>&n=<name>
class PairPayload {
  final String host;
  final int port;
  final String token;
  final String name;

  const PairPayload({
    required this.host,
    required this.port,
    required this.token,
    required this.name,
  });

  String get baseUrl => Uri(scheme: 'http', host: host, port: port).origin;

  bool get isValid {
    if (host.trim().isEmpty ||
        port <= 0 ||
        port > 65535 ||
        token.isEmpty ||
        RegExp(r'[\s/?#@]').hasMatch(host)) {
      return false;
    }
    try {
      return Uri(scheme: 'http', host: host, port: port).host.isNotEmpty;
    } on FormatException {
      return false;
    }
  }

  String toUri() => 'localhostai://pair?h=${Uri.encodeComponent(host)}'
      '&p=$port'
      '&t=${Uri.encodeComponent(token)}'
      '&n=${Uri.encodeComponent(name)}';

  /// Parse a scanned string. Returns null when it is not a pairing URI, so the
  /// caller can show "that is not a localhost.ai code" instead of crashing.
  static PairPayload? tryParse(String raw) {
    try {
      final uri = Uri.parse(raw.trim());
      // `privateml` was the scheme before the localhost.ai rename; keep
      // accepting it so an older laptop binary or APK still pairs.
      if (uri.scheme != 'localhostai' && uri.scheme != 'privateml' ||
          uri.host != 'pair' ||
          uri.path.isNotEmpty ||
          uri.userInfo.isNotEmpty ||
          uri.hasPort ||
          uri.hasFragment) {
        return null;
      }
      final host = uri.queryParameters['h'];
      final token = uri.queryParameters['t'];
      final port = int.tryParse(uri.queryParameters['p'] ?? '');
      if (host == null || token == null || port == null) return null;
      final payload = PairPayload(
        host: host,
        port: port,
        token: token,
        name: uri.queryParameters['n']?.isNotEmpty == true
            ? uri.queryParameters['n']!
            : host,
      );
      return payload.isValid ? payload : null;
    } on FormatException {
      return null;
    }
  }

  Map<String, dynamic> toJson() => {
        'host': host,
        'port': port,
        'token': token,
        'name': name,
      };

  static PairPayload fromJson(Map<String, dynamic> j) {
    final payload = PairPayload(
      host: j['host'] as String,
      port: j['port'] as int,
      token: j['token'] as String,
      name: (j['name'] as String?) ?? j['host'] as String,
    );
    if (!payload.isValid) {
      throw const FormatException('Invalid pairing payload');
    }
    return payload;
  }
}
