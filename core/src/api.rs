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
            .timeout(std::time::Duration::from_secs(4))
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
            // /health returns plain strings; /v1/models returns objects.
            models: h["models"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| {
                            m.as_str()
                                .map(String::from)
                                .or_else(|| m["id"].as_str().map(String::from))
                        })
                        .collect()
                })
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
        let result = self.chat_stream_inner(model, history, system, &tx).await;
        if let Err(error) = &result {
            let _ = tx.send(ApiEvent::Failed(format!("{error:#}")));
        }
        result
    }

    async fn chat_stream_inner(
        &self,
        model: Option<&str>,
        history: Vec<(String, String)>,
        system: Option<&str>,
        tx: &mpsc::UnboundedSender<ApiEvent>,
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
            bail!("{}", friendly(status.as_u16(), &body));
        }

        let mut stream = resp.bytes_stream();
        // TCP chunks may end in the middle of a UTF-8 character. Decode only
        // complete lines, after their original bytes have been reassembled.
        let mut buf = Vec::new();
        let mut data = Vec::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("stream interrupted")?;
            buf.extend_from_slice(&bytes);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let event = parse_sse_line(&buf[..pos], &mut data)?;
                buf.drain(..pos + 1);
                if event.is_some_and(|event| forward_frame(event, tx)) {
                    return Ok(());
                }
            }
        }
        for event in [parse_sse_line(&buf, &mut data)?, parse_sse_data(&data)?]
            .into_iter()
            .flatten()
        {
            if forward_frame(event, tx) {
                return Ok(());
            }
        }
        bail!("reply stream ended before completion; check the connection and retry")
    }
}

struct SseFrame {
    text: String,
    finished: bool,
}

fn forward_frame(frame: SseFrame, tx: &mpsc::UnboundedSender<ApiEvent>) -> bool {
    if !frame.text.is_empty() && tx.send(ApiEvent::Chunk(frame.text)).is_err() {
        return true;
    }
    if frame.finished {
        let _ = tx.send(ApiEvent::Done);
    }
    frame.finished
}

fn parse_sse_line(bytes: &[u8], data: &mut Vec<u8>) -> Result<Option<SseFrame>> {
    let line = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    if line.is_empty() {
        return parse_sse_data(&std::mem::take(data));
    }
    if let Some(payload) = line.strip_prefix(b"data:") {
        if !data.is_empty() {
            data.push(b'\n');
        }
        data.extend_from_slice(payload.strip_prefix(b" ").unwrap_or(payload));
    }
    Ok(None)
}

fn parse_sse_data(bytes: &[u8]) -> Result<Option<SseFrame>> {
    let payload = std::str::from_utf8(bytes)
        .context("invalid UTF-8 in stream")?
        .trim();
    if payload.is_empty() {
        return Ok(None);
    }
    if payload == "[DONE]" {
        return Ok(Some(SseFrame {
            text: String::new(),
            finished: true,
        }));
    }
    let v: Value = serde_json::from_str(payload).context("invalid JSON in stream")?;
    if let Some(error) = v.get("error").filter(|e| !e.is_null()) {
        let message = error["message"].as_str().or_else(|| error.as_str());
        bail!(
            "node error: {}",
            message
                .map(String::from)
                .unwrap_or_else(|| error.to_string())
        );
    }
    let choice = &v["choices"][0];
    let text = choice["delta"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let finished = !choice["finish_reason"].is_null();
    Ok((!text.is_empty() || finished).then_some(SseFrame { text, finished }))
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

impl Node {
    /// GET a JSON document from the node.
    pub async fn get_json(&self, path: &str) -> Result<Value> {
        let r = self
            .auth(self.http.get(format!("{}{}", self.base, path)))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?;
        Ok(r.json::<Value>().await?)
    }

    /// POST JSON and return the response document (errors carry the body).
    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        self.post_json_timeout(path, body, std::time::Duration::from_secs(600))
            .await
    }

    /// Long-running control operation, such as a resumable model download.
    pub async fn post_json_long(&self, path: &str, body: &Value) -> Result<Value> {
        self.post_json_timeout(path, body, std::time::Duration::from_secs(7200))
            .await
    }

    async fn post_json_timeout(
        &self,
        path: &str,
        body: &Value,
        timeout: std::time::Duration,
    ) -> Result<Value> {
        let r = self
            .auth(self.http.post(format!("{}{}", self.base, path)))
            .json(body)
            .timeout(timeout)
            .send()
            .await?;
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
        if !status.is_success() {
            let msg = v["error"]["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .unwrap_or(&text)
                .to_string();
            bail!("{}", friendly(status.as_u16(), &msg));
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Regression: /health returns `"models": ["a","b"]` (strings) while
    /// /v1/models returns `"data":[{"id":"a"}]` (objects). A single parser that
    /// only handled the object shape silently reported zero models.
    #[test]
    fn health_models_parse_both_shapes() {
        let strings = json!({ "ok": true, "name": "n", "backend": "ollama",
                              "models": ["qwen3:4b", "llama3"], "pairing": false,
                              "version": "0.2.0" });
        let objs = json!({ "ok": true, "name": "n", "backend": "ollama",
                           "models": [{"id": "qwen3:4b"}, {"id": "llama3"}],
                           "pairing": false, "version": "0.2.0" });

        for v in [strings, objs] {
            let list: Vec<String> = v["models"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| {
                            m.as_str()
                                .map(String::from)
                                .or_else(|| m["id"].as_str().map(String::from))
                        })
                        .collect()
                })
                .unwrap_or_default();
            assert_eq!(list, vec!["qwen3:4b", "llama3"], "shape: {v}");
        }
    }

    #[test]
    fn friendly_hints_are_actionable() {
        assert!(friendly(401, "").contains("paired"));
        assert!(friendly(503, "").contains("model"));
        assert_eq!(friendly(500, "boom"), "HTTP 500: boom");
    }

    #[tokio::test]
    async fn streaming_preserves_split_utf8_and_the_final_unterminated_line() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let node = Node::new(&format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            let body = "data: {\"choices\":\r\ndata: [{\"delta\":{\"content\":\"你好🙂\"}}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"final\"},\"finish_reason\":\"stop\"}]}";
            stream
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
                )
                .await
                .unwrap();
            let cut = body.find('你').unwrap() + 1;
            stream.write_all(&body.as_bytes()[..cut]).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            stream.write_all(&body.as_bytes()[cut..]).await.unwrap();
        });
        let (tx, mut rx) = mpsc::unbounded_channel();
        node.chat_stream(None, vec![], None, tx).await.unwrap();
        let mut text = String::new();
        let mut done = 0;
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Chunk(s) => text.push_str(&s),
                ApiEvent::Done => done += 1,
                ApiEvent::Failed(e) => panic!("{e}"),
            }
        }
        assert_eq!(text, "你好🙂final");
        assert_eq!(done, 1);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn transport_errors_produce_one_failure_event() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let node = Node::new(&format!("http://{}", listener.local_addr().unwrap()));
        drop(listener);
        let (tx, mut rx) = mpsc::unbounded_channel();
        assert!(node.chat_stream(None, vec![], None, tx).await.is_err());
        assert!(matches!(rx.recv().await, Some(ApiEvent::Failed(_))));
        assert!(rx.recv().await.is_none());
        assert!(parse_sse_data(br#"{"error":"upstream failed"}"#).is_err());
    }

    #[tokio::test]
    async fn eof_without_a_finish_event_is_a_failure() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let node = Node::new(&format!("http://{}", listener.local_addr().unwrap()));
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let (tx, mut rx) = mpsc::unbounded_channel();
        let error = node.chat_stream(None, vec![], None, tx).await.unwrap_err();
        assert!(error.to_string().contains("before completion"));
        assert!(matches!(rx.recv().await, Some(ApiEvent::Chunk(text)) if text == "partial"));
        assert!(matches!(rx.recv().await, Some(ApiEvent::Failed(_))));
        assert!(rx.recv().await.is_none());
        task.await.unwrap();
    }
}
