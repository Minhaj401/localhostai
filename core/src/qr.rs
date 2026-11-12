//! QR pairing payload + terminal rendering.
//!
//! The phone needs three facts to join: address, port, token. We encode them in
//! one compact URI that the APK parses straight into its "add node" form:
//!
//!   localhostai://pair?h=192.168.1.20&p=8765&t=<token>&n=<name>
//!
//! Rendering is done with half-blocks (▀ ▄ █) so a QR still scans from a
//! terminal at a readable width. Encoding uses the maintained qrcode crate.

use serde::{Deserialize, Serialize};

/// Everything needed to pair a phone with this node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PairPayload {
    /// LAN address the phone should dial.
    pub host: String,
    pub port: u16,
    /// Shared secret.
    pub token: String,
    /// Node name, shown in the phone's list.
    pub name: String,
}

impl PairPayload {
    pub fn new(host: &str, port: u16, token: &str, name: &str) -> Self {
        Self {
            host: host.into(),
            port,
            token: token.into(),
            name: name.into(),
        }
    }

    /// The string that goes into the QR code.
    pub fn uri(&self) -> String {
        format!(
            "localhostai://pair?h={}&p={}&t={}&n={}",
            url_encode(&self.host),
            self.port,
            url_encode(&self.token),
            url_encode(&self.name)
        )
    }

    /// Parse a scanned URI back into a payload.
    pub fn parse(uri: &str) -> Option<Self> {
        let rest = uri.strip_prefix("localhostai://pair?")?;
        let mut host = None;
        let mut port = None;
        let mut token = None;
        let mut name = String::new();
        for kv in rest.split('&') {
            let (k, v) = kv.split_once('=')?;
            let v = url_decode(v)?;
            match k {
                "h" => host = Some(v),
                "p" => port = v.parse().ok(),
                "t" => token = Some(v),
                "n" => name = v,
                _ => {}
            }
        }
        let host = host?;
        let port = port?;
        let token = token?;
        if host.trim().is_empty() || token.is_empty() || port == 0 {
            return None;
        }
        Some(Self {
            host,
            port,
            token,
            name,
        })
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn url_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = (*b.get(i + 1)? as char).to_digit(16)?;
            let lo = (*b.get(i + 2)? as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8(out).ok()
}

// ------------------------------------------------------------------ QR core

/// Encode `data` as a QR module grid (`true` = dark).
///
/// Delegates to the `qrcode` crate. An earlier hand-written encoder produced
/// grids that looked plausible and failed to scan, so correctness here is not
/// worth reinventing: Reed-Solomon, masking and format bits all have to be
/// exactly right or real scanners refuse the code.
pub fn encode(data: &str) -> Option<Vec<Vec<bool>>> {
    use qrcode::{EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::M).ok()?;
    let w = code.width();
    let mut grid = vec![vec![false; w]; w];
    for (y, row) in grid.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            *cell = code[(x, y)] == qrcode::Color::Dark;
        }
    }
    Some(grid)
}

/// Render as terminal text using half blocks; 2 QR rows per text line.
pub fn render_terminal(grid: &[Vec<bool>], quiet: usize) -> Vec<String> {
    let n = grid.len();
    let total = n + quiet * 2;
    let at = |y: usize, x: usize| -> bool {
        if y < quiet || x < quiet {
            return false;
        }
        let (yy, xx) = (y - quiet, x - quiet);
        if yy < n && xx < n {
            grid[yy][xx]
        } else {
            false
        }
    };

    let mut lines = Vec::new();
    let mut y = 0;
    while y < total {
        let mut line = String::new();
        for x in 0..total {
            let top = at(y, x);
            let bot = if y + 1 < total { at(y + 1, x) } else { false };
            line.push(match (top, bot) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        lines.push(line);
        y += 2;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        let p = PairPayload::new("192.168.1.20", 8765, "abc123", "my laptop");
        let uri = p.uri();
        let back = PairPayload::parse(&uri).expect("parse");
        assert_eq!(back, p);
    }

    #[test]
    fn payload_encodes_spaces_and_ipv6() {
        let p = PairPayload::new("fe80::1", 9000, "t k", "a b");
        let back = PairPayload::parse(&p.uri()).unwrap();
        assert_eq!(back.host, "fe80::1");
        assert_eq!(back.name, "a b");
        assert_eq!(back.token, "t k");
    }

    #[test]
    fn reserved_token_characters_roundtrip() {
        let p = PairPayload::new("10.0.0.2", 8765, "a+b&c=%你好", "node");
        assert_eq!(PairPayload::parse(&p.uri()), Some(p));
    }

    #[test]
    fn malformed_payloads_are_rejected_without_panicking() {
        for uri in [
            "localhostai://pair?h=%界&p=8765&t=x",
            "localhostai://pair?h=%FF&p=8765&t=x",
            "localhostai://pair?h=host&p=0&t=x",
            "localhostai://pair?h=&p=8765&t=x",
            "localhostai://pair?h=host&p=8765&t=",
        ] {
            assert!(PairPayload::parse(uri).is_none(), "{uri}");
        }
    }

    #[test]
    fn qr_has_finder_patterns() {
        let g = encode("localhostai://pair?h=10.0.0.2&p=8765&t=x&n=y").expect("qr");
        let n = g.len();
        assert_eq!(g[0].len(), n);
        // Top-left finder: corner dark, inner ring light, center dark.
        assert!(g[0][0]);
        assert!(!g[1][1]);
        assert!(g[3][3]);
        // Three finders present.
        assert!(g[0][n - 1]);
        assert!(g[n - 1][0]);
    }

    #[test]
    fn qr_renders_withouth_panic() {
        let g = encode("localhostai://pair?h=127.0.0.1&p=8765&t=deadbeef&n=node").unwrap();
        let lines = render_terminal(&g, 2);
        assert!(!lines.is_empty());
        assert!(lines[0].len() >= g.len());
    }

    #[test]
    fn handles_realistic_payload_sizes() {
        // A real pairing URI is ~90-120 chars; make sure we are nowhere near
        // the limit and that absurd input fails instead of panicking.
        let uri = PairPayload::new("192.168.100.200", 8765, &"a".repeat(32), "workstation").uri();
        assert!(uri.len() < 200, "payload grew unexpectedly: {uri}");
        assert!(encode(&uri).is_some());
        assert!(encode(&"x".repeat(5000)).is_none());
    }
}
