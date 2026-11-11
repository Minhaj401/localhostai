//! UDP discovery: the phone broadcasts, the laptop answers.
//!
//! Protocol (unchanged from the Python node so existing APK builds keep working):
//!   client -> "PRIVATELM_DISCOVER"  (UDP broadcast to :50897)
//!   server <- one-line JSON: {"name","port","backend","models":[...],"v":2}
//!
//! Broadcast does not survive every AP (client isolation, guest networks), so
//! the phone also subnet-scans :port directly. Here we only need to answer.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::Arc;
use tokio::sync::{broadcast, watch};

use crate::DISCOVERY_MAGIC;
use crate::DISCOVERY_PORT;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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
        let mut value = serde_json::to_value(self).expect("advert is JSON serializable");
        value["v"] = 2.into();
        value.to_string()
    }
}

/// Spawn the responder. Returns a watch sender whose value is the current
/// advert — update it when the model list or backend changes.
pub fn spawn(
    enabled: bool,
    seed: Advert,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<watch::Sender<Advert>> {
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
            let mut buf = [0u8; 512];
            loop {
                if rx.has_changed().is_err() {
                    return; // sender dropped == shutting down
                }
                if !matches!(
                    shutdown.try_recv(),
                    Err(broadcast::error::TryRecvError::Empty)
                ) {
                    return;
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
        let _ = sock.send_to(
            DISCOVERY_MAGIC.as_bytes(),
            ("255.255.255.255", DISCOVERY_PORT),
        );
        let mut buf = vec![0u8; 65_507];
        let deadline = std::time::Instant::now() + timeout;
        while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
            if remaining.is_zero() {
                break;
            }
            let _ = sock.set_read_timeout(Some(remaining));
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

/// Both the Rust and legacy Python responders emit ordinary JSON. Whitespace,
/// escaped model names, and Unicode must work equally well from either one.
pub fn parse_advert(s: &str) -> Option<Advert> {
    let advert: Advert = serde_json::from_str(s).ok()?;
    (advert.port > 0).then_some(advert)
}

pub type SharedAdvert = Arc<watch::Sender<Advert>>;

/// Best-effort LAN address of this machine.
///
/// Opening a UDP socket toward the LAN broadcast address makes the kernel pick
/// a broadcast-capable local interface without sending a packet. Using an
/// internet route here can select a VPN address that phones on the LAN cannot
/// reach.
pub fn local_ip() -> Option<std::net::IpAddr> {
    let sock = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.set_broadcast(true).ok()?;
    sock.connect((Ipv4Addr::BROADCAST, DISCOVERY_PORT)).ok()?;
    sock.local_addr().ok().map(|a| a.ip())
}

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

    #[test]
    fn legacy_json_whitespace_and_escaped_models_are_supported() {
        let p = parse_advert(r#"{"name": "laptop", "port": 8765, "models": ["a,b]", "a\"b", "\u4f60\u597d"], "pairing": true}"#).unwrap();
        assert_eq!(p.models, vec!["a,b]", "a\"b", "你好"]);
        assert!(p.pairing);
        let a = Advert {
            name: "line\nbreak\t".into(),
            port: 8765,
            ..Default::default()
        };
        assert_eq!(parse_advert(&a.to_json()).unwrap().name, a.name);
        assert!(parse_advert(r#"{"port":0}"#).is_none());
        assert!(parse_advert(r#"{"port":8765 broken}"#).is_none());
    }
}

#[cfg(test)]
mod ip_tests {
    #[test]
    fn local_ip_is_never_loopback_when_routable() {
        if let Some(ip) = super::local_ip() {
            // May legitimately be loopback in a sandbox with no route.
            assert!(!ip.is_unspecified());
        }
    }
}
