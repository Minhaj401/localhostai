//! HTTP client for the node (embedded or remote) — used by the TUI and tests.
//!
//! Speaks the OpenAI chat-completions dialect, so it also works against
//! Ollama, llama-server or any cloud endpoint you point it at.

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Default)]
pub struct Health {
    pub ok: bool,
    pub name: String,
    pub backend: String,
    pub models: Vec<String>,
    pub pairing: bool,
    pub version: String,
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
    token: Option<String>,
    http: reqwest::Client,
}

impl Node {
    pub fn new(base: &str) -> Self {
        Self::with_token(base, None)
    }

    pub fn with_token(base: &str, token: Option<String>) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
    }

    pub async fn health(&self) -> Result<Health> {
        let h: Value = self
            .auth(self.http.get(format!("{}/health", self.base)))
            .send()
            .await
            .context("node unreachable")?
            .error_for_status()?
            .json()
            .await?;
        Ok(Health {
            ok: h["ok"].as_bool().unwrap_or(false),
            name: h["name"].as_str().unwrap_or("?").into(),
            backend: h["backend"].as_str().unwrap_or("none").into(),
            models: h["models"]
                .as_array()
                .map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect())
                .unwrap_or_default(),
            pairing: h["pairing"].as_bool().unwrap_or(false),
            version: h["version"].as_str().unwrap_or("").into(),
        })
    }

    /// POST a streaming chat completion; deltas arrive on `tx`.
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
            .auth(self.http.post(format!("{}/v1/chat/completions", self.base)))
            .json(&json!({ "model": model, "stream": true, "messages": msgs }))
            .send()
            .await
            .context("request to node failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let _ = tx.send(ApiEvent::Failed(friendly(status.as_u16(), &body)));
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
                if let Some(err) = v["error"]["message"].as_str() {
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

/// Turn an HTTP failure into something a user can act on.
pub fn friendly(status: u16, body: &str) -> String {
    let detail = body.trim();
    let hint = match status {
        401 => " — the phone needs to be paired with this laptop",
        502 | 503 => " — no model is loaded yet on the laptop",
        _ => "",
    };
    if detail.is_empty() {
        format!("HTTP {status}{hint}")
    } else {
        format!("HTTP {status}{hint}: {detail}")
    }
}
