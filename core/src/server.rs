//! Embedded HTTP node: OpenAI-compatible API, health, pairing, SSE streaming.
//!
//! Runs inside the TUI process. Written on raw tokio so we add no web framework
//! to the binary and stay in full control of streaming.
//!
//! Endpoints:
//!   GET  /health              node status (unauthenticated — discovery needs it)
//!   GET  /v1/models           model list
//!   POST /v1/chat/completions OpenAI chat, streaming (SSE) or not
//!   GET  /pair                pairing info for the QR code
//!   POST /pair/verify         phone confirms the token after scanning
//!
//! Auth: when `require_pairing` is on, every /v1 request needs
//! `Authorization: Bearer <pair_token>`.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};

use crate::cluster::{Cluster, ClusterStatus};
use crate::config::Config;
use crate::discovery::Advert;
use crate::hardware::Hardware;
use crate::plan::{self, ModelInfo, SplitPlan};

/// Live view of the node for the TUI status bar.
#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub addr: Option<SocketAddr>,
    pub backend: String,
    pub models: Vec<String>,
    pub requests: u64,
    pub last_error: Option<String>,
}

pub struct NodeRuntime {
    pub cfg: Config,
    pub status: watch::Receiver<ServerStatus>,
    pub shutdown: broadcast::Sender<()>,
    pub advert: Option<watch::Sender<Advert>>,
    pub pair_token: String,
}

/// Handle returned to the TUI.
pub struct NodeHandle {
    inner: Arc<Inner>,
}

/// A remote device that told us what it can contribute.
#[derive(Debug, Clone)]
struct Peer {
    hw: Hardware,
    rpc: Option<String>,
    last_seen: std::time::Instant,
}

struct Inner {
    cfg: Config,
    pair_token: String,
    status_tx: watch::Sender<ServerStatus>,
    http: reqwest::Client,
    advert: Option<watch::Sender<Advert>>,
    shutdown: broadcast::Sender<()>,
    port: u16,
    hardware: Hardware,
    cluster: Cluster,
    peers: tokio::sync::Mutex<Vec<Peer>>,
    // Model inventory (GGUF files we can place across the cluster).
    models: tokio::sync::Mutex<Vec<ModelInfo>>,
}

impl NodeHandle {
    pub fn port(&self) -> u16 {
        self.inner.port
    }

    pub fn status(&self) -> watch::Receiver<ServerStatus> {
        self.inner.status_tx.subscribe()
    }

    pub fn pair_token(&self) -> &str {
        &self.inner.pair_token
    }

    pub fn hardware(&self) -> Hardware {
        self.inner.hardware.clone()
    }

    pub fn cluster(&self) -> Cluster {
        self.inner.cluster.clone()
    }

    /// LAN address to advertise (best-effort; loopback when unknown).
    pub fn lan_host(&self) -> String {
        crate::discovery::local_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "127.0.0.1".into())
    }

    /// The payload the QR code carries.
    pub fn pair_payload(&self) -> crate::qr::PairPayload {
        crate::qr::PairPayload::new(
            &self.lan_host(),
            self.inner.port,
            &self.inner.pair_token,
            &self.inner.cfg.node_name,
        )
    }

    /// Register a GGUF model so it can be planned/served.
    pub async fn add_model(&self, m: ModelInfo) {
        let mut g = self.inner.models.lock().await;
        g.retain(|x| x.path != m.path);
        g.push(m);
    }

    pub async fn models_available(&self) -> Vec<ModelInfo> {
        self.inner.models.lock().await.clone()
    }

    /// Plan a split for a model using this machine plus reporting peers.
    pub async fn plan_model(&self, name_or_path: &str, ctx: u32) -> Option<SplitPlan> {
        let models = self.inner.models.lock().await.clone();
        let m = models.iter().find(|m| m.name == name_or_path || m.path == name_or_path)?.clone();
        Some(self.plan_with(&m, ctx).await)
    }

    async fn plan_with(&self, m: &ModelInfo, ctx: u32) -> SplitPlan {
        self.plan_with_detail(m, ctx).await.0
    }

    /// Plan, also returning the devices that were excluded and why — so the UI
    /// can say "phone reported but has no reachable worker" instead of quietly
    /// promising memory that does not exist.
    async fn plan_with_detail(
        &self,
        m: &ModelInfo,
        ctx: u32,
    ) -> (SplitPlan, Vec<(String, String)>) {
        let mut devices: Vec<(Hardware, Option<String>)> =
            vec![(self.inner.hardware.clone(), None)];
        let mut excluded = Vec::new();

        let peers = self.inner.peers.lock().await;
        for p in peers.iter() {
            if p.last_seen.elapsed() >= std::time::Duration::from_secs(30) {
                excluded.push((p.hw.name.clone(), "stopped reporting (>30s)".into()));
                continue;
            }
            match &p.rpc {
                // A device can only hold layers if we can actually reach its
                // worker. Counting it otherwise would invent memory.
                Some(rpc) if !rpc.is_empty() => {
                    if crate::cluster::rpc_reachable(rpc).await {
                        devices.push((p.hw.clone(), Some(rpc.clone())));
                    } else {
                        excluded.push((
                            p.hw.name.clone(),
                            format!("reported {rpc} but no RPC worker answered"),
                        ));
                    }
                }
                _ => excluded.push((
                    p.hw.name.clone(),
                    "reported hardware but no RPC worker address".into(),
                )),
            }
        }
        drop(peers);
        (plan::plan(m, &devices, ctx), excluded)
    }

    /// Update the advert when backend/model list changes.
    pub fn publish_advert(&self, backend: &str, models: &[String]) {
        if let Some(tx) = &self.inner.advert {
            let mut a = tx.borrow().clone();
            a.backend = backend.to_string();
            a.models = models.to_vec();
            a.port = self.inner.port;
            let _ = tx.send(a);
        }
    }

    pub fn shutdown(&self) {
        let _ = self.inner.shutdown.send(());
    }

    /// Clone the handle (shared inner state) so the UI can hold one too.
    pub fn clone_ref(&self) -> NodeHandle {
        NodeHandle {
            inner: self.inner.clone(),
        }
    }
}

/// Start the node. Returns immediately; the listener runs on a task.
pub async fn start(cfg: Config) -> Result<NodeHandle> {
    let pair_token = cfg.pair_token()?;
    let port = crate::config::pick_port(cfg.port)?;
    let bind_ip = if cfg.lan_enabled {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    let addr: SocketAddr = format!("{bind_ip}:{port}").parse()?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr} — port in use?"))?;
    let local = listener.local_addr()?;

    let (status_tx, status_rx) = watch::channel(ServerStatus {
        addr: Some(local),
        ..Default::default()
    });
    let _ = status_rx;
    let (shutdown_tx, _) = broadcast::channel(8);
    let (advert_tx, _) = if cfg.discovery {
        match crate::discovery::spawn(
            true,
            Advert {
                name: cfg.node_name.clone(),
                port,
                backend: "starting".into(),
                models: vec![],
                pairing: cfg.require_pairing,
            },
        ) {
            Ok(t) => (Some(t), ()),
            Err(e) => {
                // Discovery is a convenience; never block the node over it.
                tracing::warn!("discovery disabled: {e}");
                (None, ())
            }
        }
    } else {
        (None, ())
    };

    let hardware = Hardware::probe_self(&cfg.node_name);
    let inner = Arc::new(Inner {
        pair_token,
        status_tx,
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()?,
        advert: advert_tx,
        shutdown: shutdown_tx.clone(),
        port,
        hardware,
        cluster: Cluster::new(),
        peers: tokio::sync::Mutex::new(Vec::new()),
        models: tokio::sync::Mutex::new(Vec::new()),
        cfg,
    });

    // Accept loop
    let acc = inner.clone();
    let mut shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => {
                            let conn = acc.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_conn(conn, stream, peer).await {
                                    tracing::debug!("conn error: {e}");
                                }
                            });
                        }
                        Err(e) => tracing::warn!("accept: {e}"),
                    }
                }
            }
        }
    });

    // Engine probe: do it BEFORE returning so `--status` and the first chat
    // request never see "none" just because detection is still running.
    let (backend, models) = probe_engine(&inner.cfg).await;
    inner.status_tx.send_modify(|s| {
        s.backend = backend.clone();
        s.models = models.clone();
    });
    if let Some(tx) = &inner.advert {
        let mut a = tx.borrow().clone();
        a.backend = backend.clone();
        a.models = models.clone();
        let _ = tx.send(a);
    }

    // Re-probe periodically: Ollama may start later, models may be pulled.
    let recheck = inner.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.tick().await; // first tick is immediate; skip it
        loop {
            interval.tick().await;
            let (backend, models) = probe_engine(&recheck.cfg).await;
            let changed = {
                let cur = recheck.status_tx.borrow();
                cur.backend != backend || cur.models != models
            };
            if changed {
                recheck.status_tx.send_modify(|s| {
                    s.backend = backend.clone();
                    s.models = models.clone();
                });
                if let Some(tx) = &recheck.advert {
                    let mut a = tx.borrow().clone();
                    a.backend = backend;
                    a.models = models;
                    let _ = tx.send(a);
                }
            }
        }
    });

    tracing::info!(%local, "node listening");

    Ok(NodeHandle { inner })
}

/// Detect the engine and list models. Order: managed llama.cpp, Ollama, llama-server.
async fn probe_engine(cfg: &Config) -> (String, Vec<String>) {
    let client = reqwest::Client::new();

    // 1. Managed model (llama-server we started) — cheapest to check.
    if let Ok(r) = client
        .get("http://127.0.0.1:8081/v1/models")
        .timeout(std::time::Duration::from_millis(400))
        .send()
        .await
    {
        if r.status().is_success() {
            if let Ok(v) = r.json::<Value>().await {
                return ("managed".into(), models_from(&v));
            }
        }
    }

    // 2. Ollama
    let ollama = cfg
        .ollama_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:11434/v1".into());
    if let Ok(r) = client
        .get(format!("{ollama}/models"))
        .timeout(std::time::Duration::from_millis(400))
        .send()
        .await
    {
        if r.status().is_success() {
            if let Ok(v) = r.json::<Value>().await {
                return ("ollama".into(), models_from(&v));
            }
        }
    }

    // 3. External llama-server
    let llama = cfg
        .llamacpp_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:8080/v1".into());
    if let Ok(r) = client
        .get(format!("{llama}/models"))
        .timeout(std::time::Duration::from_millis(400))
        .send()
        .await
    {
        if r.status().is_success() {
            if let Ok(v) = r.json::<Value>().await {
                return ("llamacpp".into(), models_from(&v));
            }
        }
    }

    ("none".into(), Vec::new())
}

fn models_from(v: &Value) -> Vec<String> {
    v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn upstream(cfg: &Config, backend: &str) -> String {
    match backend {
        "ollama" => cfg
            .ollama_url
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:11434/v1".into()),
        _ => cfg
            .llamacpp_url
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:8080/v1".into()),
    }
}

// ---------------------------------------------------------------- connection

async fn handle_conn(inner: Arc<Inner>, mut stream: TcpStream, peer: SocketAddr) -> Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0usize;
    // Read until headers complete, then honour Content-Length.
    loop {
        let n = stream.read(&mut buf[total..]).await?;
        if n == 0 {
            return Ok(());
        }
        total += n;
        if let Some(head_end) = find_headers_end(&buf[..total]) {
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let want = content_length(&head).unwrap_or(0);
            let have = total - head_end - 4;
            if have >= want {
                return dispatch(inner, stream, peer, &head, buf[head_end + 4..head_end + 4 + want].to_vec()).await;
            }
        }
        if total >= buf.len() {
            let _ = respond(&mut stream, 413, "Payload Too Large", "text/plain", b"too large").await;
            return Ok(());
        }
    }
}

fn find_headers_end(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(head: &str) -> Option<usize> {
    head.lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse().ok())
}

async fn dispatch(
    inner: Arc<Inner>,
    mut stream: TcpStream,
    peer: SocketAddr,
    head: &str,
    body: Vec<u8>,
) -> Result<()> {
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path_raw = parts.next().unwrap_or("/");
    let path = path_raw.split('?').next().unwrap_or("/").to_string();
    let headers = head.to_ascii_lowercase();

    tracing::debug!(%peer, method, %path, "request");

    // CORS preflight — the phone's WebView and any browser client may send it.
    if method == "OPTIONS" {
        return respond_cors(&mut stream, 204, "application/json", b"").await;
    }

    // Health is public: discovery and diagnostics rely on it.
    if path == "/health" {
        let st = inner.status_tx.borrow().clone();
        let body = json!({
            "ok": st.backend != "none",
            "name": inner.cfg.node_name,
            "backend": st.backend,
            "models": st.models,
            "pairing": inner.cfg.require_pairing,
            "version": env!("CARGO_PKG_VERSION"),
        });
        return respond_cors(&mut stream, 200, "application/json", body.to_string().as_bytes()).await;
    }

    // Pairing endpoints.
    if path == "/pair" && method == "GET" {
        let handle = NodeHandle { inner: inner.clone() };
        let payload = handle.pair_payload();
        let hw = inner.hardware.clone();
        let body = json!({
            "name": inner.cfg.node_name,
            "host": payload.host,
            "port": inner.port,
            "token": inner.pair_token,
            // Everything a phone needs, in one string it can scan.
            "uri": payload.uri(),
            "hardware": {
                "ram_mb": hw.ram_mb,
                "budget_mb": hw.budget_mb,
                "cores": hw.cores,
                "threads": hw.threads,
                "kind": hw.kind,
                "arch": hw.arch,
            },
            "cluster_ready": handle.cluster().toolchain().is_some(),
        });
        return respond_cors(&mut stream, 200, "application/json", body.to_string().as_bytes()).await;
    }
    if path == "/pair/verify" && method == "POST" {
        let ok = authorized(&inner, &headers);
        let body = json!({ "ok": ok });
        return respond_cors(&mut stream, if ok { 200 } else { 401 }, "application/json", body.to_string().as_bytes()).await;
    }

    // ---- device / cluster API -------------------------------------------------
    // Phones report their hardware here so the planner can use real numbers.
    if path == "/peers" && method == "POST" {
        let body: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                let b = json!({ "error": format!("bad json: {e}") });
                return respond_cors(&mut stream, 400, "application/json", b.to_string().as_bytes()).await;
            }
        };
        let hw = Hardware::from_raw(
            body["id"].as_str().unwrap_or("peer"),
            body["name"].as_str().unwrap_or("device"),
            body["kind"].as_str().unwrap_or("phone"),
            body["ram_mb"].as_u64().unwrap_or(0),
            body["cores"].as_u64().unwrap_or(4) as u32,
            body["arch"].as_str().unwrap_or("arm64"),
        );
        let rpc = body["rpc"].as_str().map(String::from);
        {
            let mut peers = inner.peers.lock().await;
            peers.retain(|p| p.hw.id != hw.id);
            peers.push(Peer { hw: hw.clone(), rpc: rpc.clone(), last_seen: std::time::Instant::now() });
        }
        // Only count memory we can actually use: a peer must have a reachable
        // RPC worker, otherwise its budget is a promise it cannot keep.
        let self_hw = inner.hardware.clone();
        let mut usable = self_hw.budget_mb;
        let mut usable_devices = 1u32;
        let mut unusable = 0u32;
        {
            let peers = inner.peers.lock().await;
            for p in peers.iter() {
                let fresh = p.last_seen.elapsed() < std::time::Duration::from_secs(30);
                let live = match &p.rpc {
                    Some(r) if !r.is_empty() => crate::cluster::rpc_reachable(r).await,
                    _ => false,
                };
                if fresh && live {
                    usable += p.hw.budget_mb;
                    usable_devices += 1;
                } else {
                    unusable += 1;
                }
            }
        }
        let b = json!({
            "ok": true,
            "you": { "budget_mb": hw.budget_mb, "threads": hw.threads },
            "pool": {
                "devices": usable_devices,
                "total_budget_mb": usable,
                "unusable_devices": unusable,
                "note": if unusable > 0 {
                    "devices that reported hardware but have no reachable RPC worker are not counted"
                } else { "" },
            }
        });
        return respond_cors(&mut stream, 200, "application/json", b.to_string().as_bytes()).await;
    }
    if path == "/peers" && method == "GET" {
        let self_hw = inner.hardware.clone();
        let peers = inner.peers.lock().await;
        let list: Vec<Value> = std::iter::once(json!({
            "id": self_hw.id, "name": self_hw.name, "kind": self_hw.kind,
            "ram_mb": self_hw.ram_mb, "budget_mb": self_hw.budget_mb,
            "cores": self_hw.cores, "threads": self_hw.threads,
            "arch": self_hw.arch, "rpc": null, "self": true,
        }))
        .chain(
            futures::future::join_all(peers.iter().map(|p| async move {
                // Reachability is the honest signal: a stale heartbeat with a
                // live worker can still host layers, and vice versa.
                let reachable = match &p.rpc {
                    Some(r) if !r.is_empty() => crate::cluster::rpc_reachable(r).await,
                    _ => false,
                };
                json!({
                    "id": p.hw.id, "name": p.hw.name, "kind": p.hw.kind,
                    "ram_mb": p.hw.ram_mb, "budget_mb": p.hw.budget_mb,
                    "cores": p.hw.cores, "threads": p.hw.threads,
                    "arch": p.hw.arch, "rpc": p.rpc, "self": false,
                    "stale": p.last_seen.elapsed() >= std::time::Duration::from_secs(30),
                    "reachable": reachable,
                    // Usable = we can put layers there right now.
                    "usable": reachable,
                })
            }))
            .await,
        )
        .collect();
        let b = json!({ "devices": list });
        return respond_cors(&mut stream, 200, "application/json", b.to_string().as_bytes()).await;
    }

    if path == "/models" && method == "GET" {
        let models = inner.models.lock().await;
        let list: Vec<Value> = models.iter().map(|m| json!({
            "name": m.name, "path": m.path, "params_b": m.params_b,
            "size_mb": m.size_mb, "layers": m.layers,
        })).collect();
        let b = json!({ "models": list, "cluster_ready": inner.cluster.toolchain().is_some() });
        return respond_cors(&mut stream, 200, "application/json", b.to_string().as_bytes()).await;
    }

    // Register a GGUF so it can be planned and served.
    // Body: {"path": "/x.gguf", "name": "optional", "params_b": 4.0, "layers": 36}
    if path == "/models" && method == "POST" {
        let req: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
        let p = req["path"].as_str().unwrap_or("");
        let meta = std::fs::metadata(p);
        let size_mb = meta.as_ref().map(|m| m.len() / (1024 * 1024)).unwrap_or(0);
        if meta.is_err() {
            let b = json!({ "error": format!("no such file: {p}") });
            return respond_cors(&mut stream, 400, "application/json", b.to_string().as_bytes()).await;
        }
        let name = req["name"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| {
                std::path::Path::new(p)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "model".into())
            });
        let params_b = req["params_b"].as_f64();
        let layers = req["layers"]
            .as_u64()
            .map(|l| l as u32)
            .unwrap_or_else(|| ModelInfo::estimate_layers(params_b.unwrap_or(4.0)));
        let m = ModelInfo {
            name: name.clone(),
            path: p.to_string(),
            params_b,
            size_mb,
            layers,
            kv_bytes_per_token: req["kv_bytes_per_token"].as_u64().unwrap_or(160_000),
        };
        NodeHandle { inner: inner.clone() }.add_model(m.clone()).await;
        let b = json!({ "ok": true, "model": m });
        return respond_cors(&mut stream, 200, "application/json", b.to_string().as_bytes()).await;
    }

    if path == "/cluster" && method == "GET" {
        let st: ClusterStatus = inner.cluster.status().await;
        let b = json!({
            "state": st.state, "model": st.model, "endpoint": st.endpoint,
            "peers": st.peers, "layers_on_self": st.layers_on_self,
            "total_layers": st.total_layers, "error": st.error,
            "log": st.log_path,
            "toolchain": inner.cluster.toolchain().map(|t| json!({
                "llama_server": t.llama_server.display().to_string(),
                "rpc_server": t.rpc_server.display().to_string(),
            })),
        });
        return respond_cors(&mut stream, 200, "application/json", b.to_string().as_bytes()).await;
    }

    // Body: {"model": "<name|path>", "ctx": 4096} — plans, then serves.
    if path == "/cluster/start" && method == "POST" {
        let req: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
        let want = req["model"].as_str().unwrap_or("").to_string();
        let ctx = req["ctx"].as_u64().unwrap_or(inner.cfg.ctx as u64) as u32;
        let handle = NodeHandle { inner: inner.clone() };

        let models = handle.models_available().await;
        let model = match models.iter().find(|m| m.name == want || m.path == want) {
            Some(m) => m.clone(),
            None => {
                let b = json!({ "error": format!("unknown model '{want}' — see GET /models") });
                return respond_cors(&mut stream, 404, "application/json", b.to_string().as_bytes()).await;
            }
        };

        let plan = handle.plan_with(&model, ctx).await;
        if !plan.fits() {
            let b = json!({ "error": "split does not fit", "plan": plan });
            return respond_cors(&mut stream, 507, "application/json", b.to_string().as_bytes()).await;
        }

        let st = inner
            .cluster
            .start(&plan, std::path::Path::new(&model.path), ctx)
            .await;
        let code = match &st {
            Ok(s) => {
                let running = s.state == crate::cluster::ClusterState::Running;
                inner.status_tx.send_modify(|s2| {
                    if running {
                        s2.backend = "distributed".to_string();
                    }
                });
                200
            }
            Err(_) => 500,
        };
        let b = match st {
            Ok(s) => json!({ "ok": s.state == crate::cluster::ClusterState::Running, "status": s, "plan": plan }),
            Err(e) => json!({ "ok": false, "error": e.to_string(), "plan": plan }),
        };
        return respond_cors(&mut stream, code, "application/json", b.to_string().as_bytes()).await;
    }

    if path == "/cluster/plan" && method == "POST" {
        let req: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
        let want = req["model"].as_str().unwrap_or("").to_string();
        let ctx = req["ctx"].as_u64().unwrap_or(inner.cfg.ctx as u64) as u32;
        let handle = NodeHandle { inner: inner.clone() };
        match handle.plan_model(&want, ctx).await {
            Some(p) => {
                let b = json!({ "plan": p });
                return respond_cors(&mut stream, 200, "application/json", b.to_string().as_bytes()).await;
            }
            None => {
                let b = json!({ "error": format!("unknown model '{want}'") });
                return respond_cors(&mut stream, 404, "application/json", b.to_string().as_bytes()).await;
            }
        }
    }

    if path == "/cluster/stop" && method == "POST" {
        let r = inner.cluster.stop().await;
        let b = match r {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        };
        return respond_cors(&mut stream, 200, "application/json", b.to_string().as_bytes()).await;
    }

    // Everything else under /v1 requires the token when pairing is on.
    if inner.cfg.require_pairing && !authorized(&inner, &headers) {
        let body = json!({ "error": { "message": "pair token required", "type": "auth_error" } });
        return respond_cors(&mut stream, 401, "application/json", body.to_string().as_bytes()).await;
    }

    inner.status_tx.send_modify(|s| s.requests += 1);

    match (method, path.as_str()) {
        ("GET", "/v1/models") => {
            let st = inner.status_tx.borrow().clone();
            let data: Vec<Value> = st
                .models
                .iter()
                .map(|m| json!({ "id": m, "object": "model", "owned_by": st.backend }))
                .collect();
            let body = json!({ "object": "list", "data": data });
            respond_cors(&mut stream, 200, "application/json", body.to_string().as_bytes()).await
        }
        ("POST", "/v1/chat/completions") => {
            chat_completions(inner, stream, body).await
        }
        _ => respond_cors(&mut stream, 404, "application/json", b"{\"error\":\"not found\"}").await,
    }
}

fn authorized(inner: &Inner, lower_headers: &str) -> bool {
    lower_headers
        .lines()
        .find(|l| l.starts_with("authorization:"))
        .map(|l| {
            let v = l.splitn(2, ':').nth(1).unwrap_or("").trim();
            v.strip_prefix("bearer ")
                .or_else(|| v.strip_prefix("Bearer "))
                .unwrap_or(v)
                .trim()
                == inner.pair_token
        })
        .unwrap_or(false)
}

// ------------------------------------------------------------------- chat

async fn chat_completions(inner: Arc<Inner>, mut stream: TcpStream, body: Vec<u8>) -> Result<()> {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let b = json!({ "error": { "message": format!("bad json: {e}"), "type": "invalid_request_error" } });
            return respond_cors(&mut stream, 400, "application/json", b.to_string().as_bytes()).await;
        }
    };

    let backend = inner.status_tx.borrow().backend.clone();
    if backend == "none" {
        let msg = "No model backend found. Start Ollama (`ollama serve`) or \
                   llama-server on this machine, then retry — PrivateLM picks it up \
                   within 15s, or restart it.";
        let b = json!({ "error": { "message": msg, "type": "server_error" } });
        return respond_cors(&mut stream, 503, "application/json", b.to_string().as_bytes()).await;
    }

    let mut req = req;
    if req.get("model").and_then(|m| m.as_str()).map(|m| m.is_empty()).unwrap_or(true) {
        let st = inner.status_tx.borrow().clone();
        let fallback = inner.cfg.default_model.clone()
            .or_else(|| st.models.first().cloned());
        if let Some(m) = fallback {
            req["model"] = json!(m);
        }
    }

    let streaming = req.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let url = format!("{}/chat/completions", upstream(&inner.cfg, &backend));

    let resp = inner.http.post(&url).json(&req).send().await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            inner.status_tx.send_modify(|s| s.last_error = Some(format!("upstream: {e}")));
            let b = json!({ "error": { "message": format!("upstream unreachable: {e}"), "type": "upstream_error" } });
            return respond_cors(&mut stream, 502, "application/json", b.to_string().as_bytes()).await;
        }
    };

    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let b = json!({ "error": { "message": text, "type": "upstream_error" } });
        return respond_cors(&mut stream, code, "application/json", b.to_string().as_bytes()).await;
    }

    if !streaming {
        let v: Value = resp.json().await.unwrap_or_else(|_| json!({}));
        return respond_cors(&mut stream, 200, "application/json", v.to_string().as_bytes()).await;
    }

    // SSE passthrough — headers first, then chunks as they arrive.
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
    stream.write_all(head.as_bytes()).await?;
    let mut upstream = resp.bytes_stream();
    use futures::StreamExt;
    while let Some(chunk) = upstream.next().await {
        match chunk {
            Ok(bytes) => {
                if stream.write_all(&bytes).await.is_err() {
                    break;
                }
                let _ = stream.flush().await;
            }
            Err(e) => {
                inner.status_tx.send_modify(|s| s.last_error = Some(format!("stream: {e}")));
                break;
            }
        }
    }
    let _ = stream.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- response

async fn respond_cors(
    stream: &mut TcpStream,
    code: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let reason = match code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
    Ok(())
}

async fn respond(
    stream: &mut TcpStream,
    code: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    let _ = stream.shutdown().await;
    Ok(())
}
