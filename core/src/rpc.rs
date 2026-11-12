//! Bounded preflight for idle workers using the pinned llama.cpp RPC protocol.
//! Do not probe a worker whose single client connection belongs to a running
//! cluster. Its listen backlog does not reflect the health of that connection.

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Process/session evidence reported by the phone, separate from reachability.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct WorkerReport {
    pub session: String,
    pub sequence: u64,
    pub state: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub log_tail: Option<String>,
    /// Resident anonymous pages that disappear when this worker process stops.
    pub releasable_mb: Option<u64>,
}

impl WorkerReport {
    pub fn validate(&self) -> bool {
        !self.session.is_empty()
            && self.session.len() <= 128
            && self.sequence > 0
            && matches!(self.state.as_str(), "running" | "stopped" | "error")
            && self.error.as_ref().is_none_or(|error| error.len() <= 8192)
            && self.log_tail.as_ref().is_none_or(|log| log.len() <= 8192)
            && self.releasable_mb.is_none_or(|memory| memory <= 1_048_576)
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkerStatus {
    pub reachable: bool,
    pub compatible: bool,
    pub in_use: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

impl WorkerStatus {
    pub fn active() -> Self {
        Self {
            reachable: true,
            compatible: true,
            in_use: true,
            ..Default::default()
        }
    }
}

pub async fn probe(address: &str) -> WorkerStatus {
    let mut reachable = false;
    let handshake = async {
        // b12110 parses an endpoint at its first colon, so accepting bracketed
        // IPv6 here would pass preflight and then abort inside llama-server.
        let (host, port) = address
            .rsplit_once(':')
            .context("RPC address must be host:port")?;
        if host.is_empty()
            || host.contains([':', ',', '/', ' ', '\n', '\r'])
            || port.parse::<u16>().ok().filter(|p| *p > 0).is_none()
        {
            bail!("invalid IPv4/hostname RPC address");
        }
        let mut socket = TcpStream::connect(address)
            .await
            .context("RPC connection failed")?;
        reachable = true;
        socket.set_nodelay(true)?;
        // HELLO = 14, 24 zero capability bytes select plain TCP (no RDMA).
        let mut hello = vec![14];
        hello.extend_from_slice(&24u64.to_le_bytes());
        hello.extend_from_slice(&[0; 24]);
        socket.write_all(&hello).await?;
        let size = socket
            .read_u64_le()
            .await
            .context("RPC HELLO response missing")?;
        if size != 28 {
            bail!("invalid RPC HELLO response size {size}");
        }
        let mut reply = [0u8; 28];
        socket.read_exact(&mut reply).await?;
        let version = format!("{}.{}.{}", reply[0], reply[1], reply[2]);
        if reply[0] != 8 || reply[1] != 0 {
            bail!("incompatible RPC {version}; install the matching localhost.ai phone worker (RPC 8.0.x)");
        }
        // Only one CPU device per endpoint is supported by the split mapping.
        socket.write_all(&[15, 0, 0, 0, 0, 0, 0, 0, 0]).await?;
        let size = socket.read_u64_le().await?;
        if size != 4 {
            bail!("invalid RPC device-count response size {size}");
        }
        let count = socket.read_u32_le().await?;
        if count != 1 {
            bail!("RPC worker exposes {count} devices; start it with --device CPU");
        }
        socket.shutdown().await?;
        Ok::<_, anyhow::Error>(version)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(2), handshake).await {
        Ok(Ok(version)) => WorkerStatus {
            reachable: true,
            compatible: true,
            version: Some(version),
            ..Default::default()
        },
        Ok(Err(error)) => WorkerStatus {
            reachable,
            error: Some(format!("{error:#}")),
            ..Default::default()
        },
        Err(_) => WorkerStatus {
            reachable,
            error: Some("RPC handshake timed out (worker busy or unreachable)".into()),
            ..Default::default()
        },
    }
}

#[cfg(test)]
pub(crate) async fn serve_test_probe(mut socket: TcpStream) {
    let mut request = [0u8; 33];
    if socket.read_exact(&mut request).await.is_err() {
        return;
    }
    assert_eq!(request[0], 14);
    assert_eq!(&request[1..9], &24u64.to_le_bytes());
    let mut reply = 28u64.to_le_bytes().to_vec();
    reply.extend_from_slice(&[8, 0, 1, 0]);
    reply.extend_from_slice(&[0; 24]);
    if socket.write_all(&reply).await.is_err() {
        return;
    }
    let mut request = [0; 9];
    if socket.read_exact(&mut request).await.is_err() {
        return;
    }
    assert_eq!(request[0], 15);
    let mut reply = 4u64.to_le_bytes().to_vec();
    reply.extend_from_slice(&1u32.to_le_bytes());
    let _ = socket.write_all(&reply).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn idle_worker_must_complete_the_pinned_protocol() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let task =
            tokio::spawn(async move { serve_test_probe(listener.accept().await.unwrap().0).await });
        let status = probe(&address).await;
        assert!(status.reachable && status.compatible, "{status:?}");
        assert_eq!(status.version.as_deref(), Some("8.0.1"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_tcp_listener_is_not_necessarily_an_rpc_worker() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .await
                .unwrap();
        });
        let status = probe(&address).await;
        assert!(status.reachable);
        assert!(!status.compatible);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn incompatible_worker_version_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 33];
            socket.read_exact(&mut request).await.unwrap();
            socket.write_all(&28u64.to_le_bytes()).await.unwrap();
            socket.write_all(&[7; 28]).await.unwrap();
        });
        let status = probe(&address).await;
        assert!(!status.compatible);
        assert!(status.error.unwrap().contains("incompatible RPC"));
        task.await.unwrap();
    }
}
