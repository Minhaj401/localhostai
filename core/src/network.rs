//! Outbound model traffic: validate and pin DNS on every redirect hop.

use anyhow::{bail, Context, Result};
use reqwest::{header, Method, Response, Url};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct NetworkPolicy {
    /// Explicit exceptions configured on the coordinator, never by an API caller.
    /// Entries are a hostname/IP or hostname:port.
    allowed_hosts: Vec<String>,
    #[cfg(test)]
    local_test: bool,
}

impl NetworkPolicy {
    pub fn new(allowed_hosts: Vec<String>) -> Self {
        Self {
            allowed_hosts,
            #[cfg(test)]
            local_test: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn local_test() -> Self {
        Self {
            local_test: true,
            ..Default::default()
        }
    }

    fn explicitly_allowed(&self, url: &Url) -> bool {
        let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
        let endpoint = format!("{host}:{}", url.port_or_known_default().unwrap_or(0));
        self.allowed_hosts.iter().any(|allowed| {
            allowed.eq_ignore_ascii_case(host) || allowed.eq_ignore_ascii_case(&endpoint)
        })
    }

    pub async fn resolve(&self, url: &Url) -> Result<Vec<SocketAddr>> {
        let host = url
            .host_str()
            .context("model URL has no host")?
            .trim_matches(['[', ']']);
        let port = url
            .port_or_known_default()
            .context("model URL has no port")?;
        let addresses: Vec<_> = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .context("model host DNS lookup timed out")?
        .context("model host could not be resolved")?
        .collect();
        if addresses.is_empty() {
            bail!("model host has no usable addresses");
        }
        let allowed = self.explicitly_allowed(url);
        if !allowed
            && addresses.iter().any(|address| {
                #[cfg(test)]
                if self.local_test && address.ip().is_loopback() {
                    return false;
                }
                !is_public(address.ip())
            })
        {
            bail!("model URL resolves to a non-public address; add a trusted host to allowed_model_hosts in config.toml if this is an intentional LAN model host");
        }
        Ok(addresses)
    }

    pub async fn send(
        &self,
        method: Method,
        initial: &Url,
        headers: header::HeaderMap,
        timeout: Duration,
    ) -> Result<Response> {
        let mut url = initial.clone();
        let mut visited = std::collections::HashSet::new();
        let deadline = tokio::time::Instant::now() + timeout;
        for _ in 0..8 {
            crate::model_hub::http_url(url.as_str())?;
            if !visited.insert(url.to_string()) {
                bail!("model download redirect loop");
            }
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .context("model request deadline exceeded")?;
            let addresses = tokio::time::timeout(remaining, self.resolve(&url))
                .await
                .context("model request deadline exceeded")??;
            let host = url
                .host_str()
                .context("model URL has no host")?
                .trim_matches(['[', ']']);
            let client = reqwest::Client::builder()
                .user_agent(concat!("localhostai/", env!("CARGO_PKG_VERSION")))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .resolve_to_addrs(host, &addresses)
                .connect_timeout(Duration::from_secs(20))
                .read_timeout(Duration::from_secs(300))
                .timeout(
                    deadline
                        .checked_duration_since(tokio::time::Instant::now())
                        .context("model request deadline exceeded")?,
                )
                .build()?;
            let mut request = client
                .request(method.clone(), url.clone())
                .headers(headers.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            if url.scheme() == "https"
                && host == "huggingface.co"
                && url.port_or_known_default() == Some(443)
            {
                if let Ok(token) = std::env::var("HF_TOKEN") {
                    if !token.trim().is_empty() {
                        request = request.bearer_auth(token.trim());
                    }
                }
            }
            let response = request
                .send()
                .await
                .map_err(reqwest::Error::without_url)
                .context("model host request failed")?;
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .context("model host redirect has no valid Location")?;
                let next = url
                    .join(location)
                    .context("invalid model download redirect")?;
                if url.scheme() == "https" && next.scheme() != "https" {
                    bail!("model host attempted an insecure redirect downgrade");
                }
                url = next;
                continue;
            }
            return Ok(response);
        }
        bail!("model host returned too many redirects")
    }
}

pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0 && (c == 0 || c == 2))
                && !(a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                && !(a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return is_public(IpAddr::V4(ip));
            }
            let segments = ip.segments();
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && (segments[0] & 0xfe00) != 0xfc00
                && (segments[0] & 0xffc0) != 0xfe80
                && (segments[0] & 0xffc0) != 0xfec0
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
                && (segments[0] & 0xe000) == 0x2000
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn excludes_loopback_private_metadata_and_mapped_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.1.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "198.18.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        assert!(is_public("1.1.1.1".parse().unwrap()));
        assert!(is_public("2606:4700::1111".parse().unwrap()));
    }

    #[tokio::test]
    async fn does_not_contact_disallowed_initial_or_redirect_addresses() {
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_url = Url::parse(&format!(
            "http://{}/secret.gguf",
            target.local_addr().unwrap()
        ))
        .unwrap();
        assert!(NetworkPolicy::default()
            .send(
                Method::HEAD,
                &target_url,
                header::HeaderMap::new(),
                Duration::from_secs(2)
            )
            .await
            .is_err());
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = origin.local_addr().unwrap();
        let destination = target_url.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.unwrap();
            let mut buf = [0];
            socket.read_exact(&mut buf).await.unwrap();
            socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        let policy = NetworkPolicy::new(vec![address.to_string()]);
        let url = Url::parse(&format!("http://{address}/runtime.tar.gz")).unwrap();
        assert!(policy
            .send(
                Method::GET,
                &url,
                header::HeaderMap::new(),
                Duration::from_secs(2)
            )
            .await
            .is_err());
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), target.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn follows_validated_runtime_style_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for i in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0];
                socket.read_exact(&mut buf).await.unwrap();
                let reply = if i == 0 {
                    "HTTP/1.1 302 Found\r\nLocation: /asset\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                };
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let policy = NetworkPolicy::new(vec![address.to_string()]);
        let response = policy
            .send(
                Method::GET,
                &Url::parse(&format!("http://{address}/release")).unwrap(),
                header::HeaderMap::new(),
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "ok");
        task.await.unwrap();
    }
}
