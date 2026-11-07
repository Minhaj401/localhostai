//! HTTP client for the PrivateLM-LAN node (or any OpenAI-compatible server).

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct Health {
    pub ok: bool,
    pub name: String,
    pub backend: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ApiEvent {
    Chunk(String),
    Done,
    Failed(String),
}

#[derive(Clone)]
pub struct Node {
    base: String,
    http: reqwest::Client,
}

impl Node {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn health(&self) -> Result<Health> {
        let h: Value = self
            .http
            .get(format!("{}/health", self.base))
            .send()
            .await
            .context("node unreachable — is `python3 server.py` running?")?
            .error_for_status()?
            .json()
            .await?;
        Ok(Health {
            ok: h["ok"].as_bool().unwrap_or(false),
            name: h["name"].as_str().unwrap_or("?").into(),
            backend: h["backend"].as_str().unwrap_or("none").into(),
            models: h["models"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// POSTs a chat completion with stream=true and forwards deltas on `tx`.
    pub async fn chat_stream(
        &self,
        model: Option<&str>,
        history: Vec<(String, String)>,
        system: Option<&str>,
        tx: mpsc::UnboundedSender<ApiEvent>,
    ) -> Result<()> {
        let mut msgs = Vec::new();
        if let Some(s) = system.filter(|s| !s.is_empty()) {
            msgs.push(json!({"role": "system", "content": s}));
        }
        for (role, content) in history {
            msgs.push(json!({"role": role, "content": content}));
        }

        let resp = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .json(&json!({
                "model": model,
                "stream": true,
                "messages": msgs,
            }))
            .send()
            .await
            .context("request to node failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let _ = tx.send(ApiEvent::Failed(format!("HTTP {status}: {body}")));
            bail!("node returned {status}");
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("stream interrupted")?;
            buf.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..pos + 1);
                if !line.starts_with("data:") {
                    continue;
                }
                let payload = line[5..].trim();
                if payload == "[DONE]" {
                    let _ = tx.send(ApiEvent::Done);
                    return Ok(());
                }
                let Ok(v) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };
                if let Some(err) = v["error"].as_str() {
                    let _ = tx.send(ApiEvent::Failed(err.to_string()));
                    bail!("node error: {err}");
                }
                if let Some(delta) = v["choices"][0]["delta"]["content"].as_str() {
                    if !delta.is_empty() && tx.send(ApiEvent::Chunk(delta.into())).is_err() {
                        return Ok(()); // UI closed
                    }
                }
            }
        }
        let _ = tx.send(ApiEvent::Done);
        Ok(())
    }
}
