//! UDP discovery: the phone broadcasts, the laptop answers.
//!
//! Protocol (unchanged from the Python node so existing APK builds keep working):
//!   client -> "PRIVATELM_DISCOVER"  (UDP broadcast to :50897)
//!   server <- one-line JSON: {"name","port","backend","models":[...],"v":2}
//!
//! Broadcast does not survive every AP (client isolation, guest networks), so
//! the phone also subnet-scans :port directly. Here we only need to answer.

use anyhow::Result;
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::Arc;
use tokio::sync::watch;

use crate::DISCOVERY_MAGIC;
use crate::DISCOVERY_PORT;

#[derive(Debug, Clone, Default)]
pub struct Advert {
    pub name: String,
    pub port: u16,
    pub backend: String,
    pub models: Vec<String>,
    /// Whether this node requires the pair token.
    pub pairing: bool,
}

impl Advert {
    pub fn to_json(&self) -> String {
        let models: Vec<String> = self.models.iter().map(|m| format!("\"{}\"", esc(m))).collect();
        format!(
            "{{\"name\":\"{}\",\"port\":{},\"backend\":\"{}\",\"models\":[{}],\"pairing\":{},\"v\":2}}",
            esc(&self.name),
            self.port,
            esc(&self.backend),
            models.join(","),
            self.pairing
        )
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Spawn the responder. Returns a watch receiver whose value is the current
/// advert — update it when the model list or backend changes.
pub fn spawn(enabled: bool, seed: Advert) -> Result<watch::Sender<Advert>> {
    let (tx, rx) = watch::channel(seed);
    if !enabled {
        return Ok(tx);
    }

    std::thread::Builder::new()
        .name("privateml-discovery".into())
        .spawn(move || {
            let sock = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
                Ok(s) => s,
                Err(e) => {
                    // Not fatal: another instance may hold the port, or the box
                    // may not allow broadcast sockets. The phone can still use
                    // a manual IP, so stay quiet unless asked to be noisy.
                    if std::env::var("PRIVATELM_DEBUG").is_ok() {
                        eprintln!("[discovery] disabled: {e} (phone can use manual IP)");
                    }
                    return;
                }
            };
            let _ = sock.set_broadcast(true);
            // Generous timeout so we can notice shutdown between reads.
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(500)));
            let rx = rx;
            let mut buf = [0u8; 512];
            loop {
                if rx.has_changed().is_err() {
                    return; // sender dropped == shutting down
                }
                match sock.recv_from(&mut buf) {
                    Ok((n, peer)) => {
                        let msg = String::from_utf8_lossy(&buf[..n]);
                        if msg.trim() == DISCOVERY_MAGIC {
                            let advert = rx.borrow().clone();
                            let _ = sock.send_to(advert.to_json().as_bytes(), peer);
                        }
                    }
                    Err(e) => match e.kind() {
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => continue,
                        _ => continue,
                    },
                }
            }
        })?;

    Ok(tx)
}

/// Broadcast a discovery query and collect replies (used for diagnostics and
/// by the TUI's "find other nodes" view). Best effort: returns what answered.
pub async fn probe(timeout: std::time::Duration) -> Vec<(Ipv4Addr, Advert)> {
    tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let sock = match UdpSocket::bind(("0.0.0.0", 0)) {
            Ok(s) => s,
            Err(_) => return out,
        };
        let _ = sock.set_broadcast(true);
        let _ = sock.set_read_timeout(Some(timeout));
        let _ = sock.send_to(DISCOVERY_MAGIC.as_bytes(), ("255.255.255.255", DISCOVERY_PORT));
        let mut buf = [0u8; 512];
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            match sock.recv_from(&mut buf) {
                Ok((n, peer)) => {
                    let body = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(a) = parse_advert(&body) {
                        if let std::net::SocketAddr::V4(v4) = peer {
                            out.push((*v4.ip(), a));
                        }
                    }
                }
                Err(_) => break,
            }
        }
        out
    })
    .await
    .unwrap_or_default()
}

/// Minimal parser for the advert JSON (no serde needed for this shape).
pub fn parse_advert(s: &str) -> Option<Advert> {
    let get_str = |key: &str| -> Option<String> {
        let pat = format!("\"{key}\":\"");
        let start = s.find(&pat)? + pat.len();
        let rest = &s[start..];
        // Walk to the terminating quote, honouring backslash escapes.
        let mut out = String::new();
        let mut escaped = false;
        for c in rest.chars() {
            if escaped {
                out.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                return Some(out);
            } else {
                out.push(c);
            }
        }
        None
    };
    let port = {
        let pat = "\"port\":";
        let start = s.find(pat)? + pat.len();
        let rest = &s[start..];
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        rest[..end].parse().ok()?
    };
    let models = {
        let pat = "\"models\":[";
        let start = s.find(pat).map(|i| i + pat.len()).unwrap_or(0);
        let rest = &s[start..];
        let end = rest.find(']').unwrap_or(0);
        rest[..end]
            .split(',')
            .map(|x| x.trim().trim_matches('"').to_string())
            .filter(|x| !x.is_empty())
            .collect()
    };
    let pairing = s.contains("\"pairing\":true");
    Some(Advert {
        name: get_str("name").unwrap_or_else(|| "node".into()),
        port,
        backend: get_str("backend").unwrap_or_else(|| "none".into()),
        models,
        pairing,
    })
}

pub type SharedAdvert = Arc<watch::Sender<Advert>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advert_roundtrip() {
        let a = Advert {
            name: "lap\"top".into(),
            port: 8765,
            backend: "ollama".into(),
            models: vec!["qwen3:4b".into(), "llama3".into()],
            pairing: true,
        };
        let j = a.to_json();
        let p = parse_advert(&j).expect("parse");
        assert_eq!(p.name, "lap\"top");
        assert_eq!(p.port, 8765);
        assert_eq!(p.backend, "ollama");
        assert_eq!(p.models, vec!["qwen3:4b", "llama3"]);
        assert!(p.pairing);
    }
}
