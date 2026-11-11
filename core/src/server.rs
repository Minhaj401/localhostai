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
//!   GET  /managed             built-in runtime/model catalog and install progress
//!   POST /managed/install     download and verify a catalog model
//!
//! Auth: when `require_pairing` is on, every /v1 request needs
//! `Authorization: Bearer <pair_token>`.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};

use crate::cluster::{Cluster, ClusterStatus};
use crate::config::Config;
use crate::discovery::Advert;
use crate::hardware::Hardware;
use crate::managed::Managed;
use crate::plan::{self, ModelInfo, SplitPlan};
use crate::rpc::{WorkerReport, WorkerStatus};

/// Live view of the node for the TUI status bar.
#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub addr: Option<SocketAddr>,
    pub backend: String,
    pub models: Vec<String>,
    pub upstream: Option<String>,
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
    /// Coordinator-side inclusion choice. Heartbeats update hardware without
    /// overriding the user's decision in the TUI.
    enabled: bool,
    worker: Option<WorkerReport>,
    retired_sessions: Vec<String>,
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
    self_enabled: AtomicBool,
    cluster: Cluster,
    peers: tokio::sync::Mutex<Vec<Peer>>,
    // Model inventory (GGUF files we can place across the cluster).
    models: tokio::sync::Mutex<Vec<ModelInfo>>,
    managed: Managed,
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
        current_self_hardware(&self.inner)
    }

    pub fn cluster(&self) -> Cluster {
        self.inner.cluster.clone()
    }

    /// LAN address to advertise (best-effort; loopback when unknown).
    pub fn lan_host(&self) -> String {
        if !self.inner.cfg.lan_enabled {
            return "127.0.0.1".into();
        }
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
        g.retain(|x| x.path != m.path && x.name != m.name);
        g.push(m);
    }

    pub async fn models_available(&self) -> Vec<ModelInfo> {
        self.inner.models.lock().await.clone()
    }

    /// Plan a split for a model using this machine plus reporting peers.
    pub async fn plan_model(&self, name_or_path: &str, ctx: u32) -> Option<SplitPlan> {
        let models = self.inner.models.lock().await.clone();
        let m = models
            .iter()
            .find(|m| m.name == name_or_path || m.path == name_or_path)
            .cloned()
            .or_else(|| Managed::catalog_model(name_or_path).map(|model| model.model_info()))?;
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
        let mut devices: Vec<(Hardware, Option<String>)> = Vec::new();
        let mut excluded = Vec::new();
        let mut sessions = std::collections::HashMap::new();
        let active = self.inner.cluster.status().await;

        if self.inner.self_enabled.load(Ordering::Relaxed) {
            let mut hardware = current_self_hardware(&self.inner);
            credit_released_allocation(&mut hardware, None, None, &active);
            devices.push((hardware, None));
        } else {
            excluded.push((
                self.inner.hardware.name.clone(),
                "excluded from the pool in the TUI".into(),
            ));
        }

        for (mut p, worker) in peer_states(&self.inner).await {
            if !p.enabled {
                excluded.push((p.hw.name, "excluded from the pool in the TUI".into()));
            } else if worker.compatible {
                credit_released_allocation(
                    &mut p.hw,
                    p.rpc.as_deref(),
                    p.worker.as_ref().map(|report| report.session.as_str()),
                    &active,
                );
                if let Some(report) = p.worker {
                    sessions.insert(p.hw.id.clone(), report.session);
                }
                devices.push((p.hw, p.rpc));
            } else {
                excluded.push((
                    p.hw.name,
                    worker
                        .error
                        .unwrap_or_else(|| "no compatible RPC worker".into()),
                ));
            }
        }
        let mut plan = plan::plan(m, &devices, ctx);
        for assignment in &mut plan.assignments {
            assignment.worker_session = sessions.remove(&assignment.device_id);
        }
        (plan, excluded)
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
    cfg.validate()?;
    let pair_token = cfg.pair_token()?;
    let bind_ip = if cfg.lan_enabled {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    // Keep the successful bind instead of probing loopback and racing another
    // process before binding the actual LAN interface.
    let first = if cfg.port == 0 {
        crate::DEFAULT_PORT
    } else {
        cfg.port
    };
    let mut listener = None;
    for port in first..=first.saturating_add(99) {
        match TcpListener::bind((bind_ip, port)).await {
            Ok(bound) => {
                listener = Some(bound);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e).with_context(|| format!("bind {bind_ip}:{port}")),
        }
    }
    let listener = listener.context("no free node port in the requested range")?;
    let local = listener.local_addr()?;
    let port = local.port();

    let (status_tx, status_rx) = watch::channel(ServerStatus {
        addr: Some(local),
        ..Default::default()
    });
    let _ = status_rx;
    let (shutdown_tx, _) = broadcast::channel(8);
    let (advert_tx, _) = if cfg.discovery && cfg.lan_enabled {
        match crate::discovery::spawn(
            true,
            Advert {
                name: cfg.node_name.clone(),
                port,
                backend: "starting".into(),
                models: vec![],
                pairing: cfg.require_pairing,
            },
            shutdown_tx.subscribe(),
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
        self_enabled: AtomicBool::new(true),
        cluster: Cluster::new(),
        peers: tokio::sync::Mutex::new(Vec::new()),
        models: tokio::sync::Mutex::new(Vec::new()),
        managed: Managed::new()?,
        cfg,
    });

    // Accept loop
    let acc = inner.clone();
    let mut shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = connections.join_next(), if !connections.is_empty() => {},
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => {
                            let conn = acc.clone();
                            connections.spawn(async move {
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
        connections.abort_all();
        let _ = acc.cluster.stop().await;
    });

    // Managed GGUFs first, so a file the user dropped in is immediately
    // plannable even when Ollama is absent.
    register_managed_models(&NodeHandle {
        inner: inner.clone(),
    })
    .await;

    // Engine probe: do it BEFORE returning so `--status` and the first chat
    // request never see "none" just because detection is still running.
    refresh_engine(&inner).await;

    // Re-probe periodically: Ollama may start later, models may be pulled.
    let recheck = inner.clone();
    let mut recheck_shutdown = shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.tick().await; // first tick is immediate; skip it
        loop {
            tokio::select! {
                _ = recheck_shutdown.recv() => break,
                _ = interval.tick() => {},
            }
            tokio::select! {
                _ = recheck_shutdown.recv() => break,
                _ = refresh_engine(&recheck) => {},
            }
        }
    });

    tracing::info!(%local, "node listening");

    Ok(NodeHandle { inner })
}

/// Register every `*.gguf` in the managed models directory.
///
/// A GGUF sitting in `~/.privateml/models/` is one the user already put there,
/// so requiring a separate registration call would make `GET /models` look
/// empty and the phone offer nothing splittable. Scanning keeps the advertised
/// problem to one instruction: drop a file in, restart, plan it.
async fn register_managed_models(handle: &NodeHandle) {
    for model in handle.inner.managed.installed_custom_models() {
        handle.add_model(model).await;
    }
    let dir = crate::config::models_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("gguf") {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        // Ignore partial downloads and placeholder links that dangle.
        if !meta.is_file() || meta.len() < 1024 * 1024 {
            continue;
        }
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "model".into());
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(crate::gguf::split_name)
            .is_some_and(|(_, part, _)| part != 1)
        {
            continue;
        }
        match ModelInfo::from_file(&path, &name) {
            Ok(model) => handle.add_model(model).await,
            Err(error) => tracing::warn!(path = %path.display(), %error, "ignoring invalid GGUF"),
        }
    }
}

#[derive(Default)]
struct Engine {
    backend: String,
    models: Vec<String>,
    url: Option<String>,
}

async fn probe_engine(cfg: &Config, cluster: &ClusterStatus) -> Engine {
    let client = reqwest::Client::new();
    let mut candidates = Vec::new();
    if cluster.state == crate::cluster::ClusterState::Running {
        if let Some(url) = &cluster.endpoint {
            candidates.push((
                if cluster.peers.is_empty() {
                    "managed"
                } else {
                    "distributed"
                },
                url.clone(),
            ));
        }
    }
    for (backend, url) in [
        (
            "ollama",
            cfg.ollama_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:11434/v1".into()),
        ),
        (
            "llamacpp",
            cfg.llamacpp_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:8080/v1".into()),
        ),
    ] {
        if cfg.engine == "auto" || cfg.engine == backend {
            candidates.push((backend, url));
        }
    }
    for (backend, url) in candidates {
        let url = url.trim_end_matches('/').to_string();
        if let Ok(r) = client
            .get(format!("{url}/models"))
            .timeout(std::time::Duration::from_millis(400))
            .send()
            .await
        {
            if r.status().is_success() {
                if let Ok(v) = r.json::<Value>().await {
                    if v["data"].is_array() {
                        return Engine {
                            backend: backend.into(),
                            models: models_from(&v),
                            url: Some(url),
                        };
                    }
                }
            }
        }
    }
    Engine {
        backend: "none".into(),
        ..Default::default()
    }
}

async fn refresh_engine(inner: &Inner) {
    let before = inner.cluster.status().await;
    let engine = probe_engine(&inner.cfg, &before).await;
    let after = inner.cluster.status().await;
    // A probe started before a cluster transition must not overwrite the new
    // endpoint with an old backend after that transition completes.
    if before.generation != after.generation
        || before.state != after.state
        || before.endpoint != after.endpoint
    {
        return;
    }
    inner.status_tx.send_modify(|s| {
        s.backend = engine.backend.clone();
        s.models = engine.models.clone();
        s.upstream = engine.url;
    });
    if let Some(tx) = &inner.advert {
        tx.send_modify(|a| {
            a.backend = engine.backend;
            a.models = engine.models;
        });
    }
}

async fn refresh_stopped_upstream(inner: &Inner) {
    let owned = matches!(
        inner.status_tx.borrow().backend.as_str(),
        "managed" | "distributed"
    );
    if owned && inner.cluster.status().await.state != crate::cluster::ClusterState::Running {
        refresh_engine(inner).await;
    }
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

async fn peer_states(inner: &Inner) -> Vec<(Peer, WorkerStatus)> {
    let peers = inner.peers.lock().await.clone();
    let mut seen = std::collections::HashSet::new();
    futures::future::join_all(peers.into_iter().map(|p| {
        let unique = p.rpc.as_ref().is_some_and(|rpc| seen.insert(rpc.clone()));
        async move {
            let worker = match &p.rpc {
                Some(_) if !unique => WorkerStatus {
                    error: Some("RPC address already counted".into()),
                    ..Default::default()
                },
                Some(rpc) => inner.cluster.worker_status(rpc).await,
                None => WorkerStatus {
                    error: p
                        .worker
                        .as_ref()
                        .and_then(|report| report.error.clone())
                        .or_else(|| Some("phone is not sharing a worker".into())),
                    ..Default::default()
                },
            };
            (p, worker)
        }
    }))
    .await
}

async fn register_peer(
    inner: &Inner,
    hw: Hardware,
    rpc: Option<String>,
    worker: Option<WorkerReport>,
) -> bool {
    let mut peers = inner.peers.lock().await;
    let previous = peers.iter().find(|peer| peer.hw.id == hw.id).cloned();
    let mut retired_sessions = previous
        .as_ref()
        .map(|peer| peer.retired_sessions.clone())
        .unwrap_or_default();
    if let (Some(previous), Some(incoming)) = (&previous, &worker) {
        if retired_sessions.contains(&incoming.session) {
            return false;
        }
        if let Some(old) = &previous.worker {
            if old.session == incoming.session {
                if incoming.sequence <= old.sequence {
                    return false;
                }
            } else {
                // An old delayed withdrawal must never erase a newer worker.
                if rpc.is_none() {
                    return false;
                }
                retired_sessions.push(old.session.clone());
                if retired_sessions.len() > 16 {
                    retired_sessions.remove(0);
                }
            }
        }
    }
    let enabled = peers
        .iter()
        .find(|peer| peer.hw.id == hw.id || (rpc.is_some() && peer.rpc == rpc))
        .map(|peer| peer.enabled)
        .unwrap_or(true);
    for old in peers
        .iter()
        .filter(|peer| peer.hw.id != hw.id && rpc.is_some() && peer.rpc == rpc)
    {
        inner.cluster.observe_worker(&old.hw.id, None, None).await;
    }
    inner
        .cluster
        .observe_worker(&hw.id, rpc.clone(), worker.clone())
        .await;
    peers.retain(|peer| peer.hw.id != hw.id && (rpc.is_none() || peer.rpc != rpc));
    peers.push(Peer {
        hw,
        rpc,
        worker,
        retired_sessions,
        enabled,
        last_seen: std::time::Instant::now(),
    });
    true
}

fn credit_released_allocation(
    hardware: &mut Hardware,
    rpc: Option<&str>,
    session: Option<&str>,
    active: &ClusterStatus,
) {
    if active.state != crate::cluster::ClusterState::Running {
        return;
    }
    if let Some(assignment) = active.assignments.iter().find(|a| {
        a.device_id == hardware.id
            && a.rpc.as_deref() == rpc
            && (a.worker_session.is_none() || a.worker_session.as_deref() == session)
    }) {
        // A replacement load stops the old process first. Count the allocation
        // it releases once, capped by physical capacity; do not make the model
        // appear too large because its previous run is still resident.
        let available = hardware
            .available_mb
            .saturating_add(assignment.memory_mb)
            .min(hardware.ram_mb);
        hardware.budget_mb = Hardware::share_budget_mb(&hardware.kind, hardware.ram_mb, available);
        hardware.can_host_layers = hardware.budget_mb > 0;
    }
}

fn current_self_hardware(inner: &Inner) -> Hardware {
    let stored = &inner.hardware;
    let live = Hardware::probe_self(&stored.name);
    let available = if live.ram_mb.abs_diff(stored.ram_mb) <= 16 {
        live.available_mb
    } else {
        stored.available_mb
    };
    Hardware::from_raw_available(
        &stored.id,
        &stored.name,
        &stored.kind,
        stored.ram_mb,
        available.min(stored.ram_mb),
        stored.cores,
        &stored.arch,
    )
}

// ---------------------------------------------------------------- connection

async fn handle_conn(inner: Arc<Inner>, mut stream: TcpStream, peer: SocketAddr) -> Result<()> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        read_request(&mut stream),
    )
    .await
    {
        Ok(Ok(Some((head, body)))) => dispatch(inner, stream, peer, &head, body).await,
        Ok(Ok(None)) => Ok(()),
        Ok(Err(e)) => {
            respond_cors(
                &mut stream,
                400,
                "application/json",
                json!({"error": {"message": e.to_string()}})
                    .to_string()
                    .as_bytes(),
            )
            .await
        }
        Err(_) => respond_cors(&mut stream, 408, "text/plain", b"request timed out").await,
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<Option<(String, Vec<u8>)>> {
    const MAX_HEADERS: usize = 64 * 1024;
    const MAX_BODY: usize = 4 * 1024 * 1024;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut parsed = None;
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if parsed.is_none() {
            if let Some(end) = find_headers_end(&buf) {
                if end > MAX_HEADERS {
                    respond_cors(stream, 413, "text/plain", b"headers too large").await?;
                    return Ok(None);
                }
                let head = std::str::from_utf8(&buf[..end])
                    .context("invalid HTTP headers")?
                    .to_string();
                let want = content_length(&head)?;
                if want > MAX_BODY {
                    respond_cors(stream, 413, "text/plain", b"request body exceeds 4 MiB").await?;
                    return Ok(None);
                }
                parsed = Some((head, end + 4, want));
            } else if buf.len() > MAX_HEADERS {
                respond_cors(stream, 413, "text/plain", b"headers too large").await?;
                return Ok(None);
            }
        }
        if let Some((head, start, want)) = &parsed {
            if buf.len() - start >= *want {
                return Ok(Some((head.clone(), buf[*start..*start + *want].to_vec())));
            }
        }
    }
}

fn find_headers_end(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(head: &str) -> Result<usize> {
    let mut length = None;
    for line in head.lines().skip(1) {
        let (name, value) = line.split_once(':').context("malformed HTTP header")?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            bail!("Transfer-Encoding is unsupported; send Content-Length");
        }
        if name.eq_ignore_ascii_case("content-length") {
            let value = value
                .trim()
                .parse::<usize>()
                .context("invalid Content-Length")?;
            if length.is_some_and(|previous| previous != value) {
                bail!("conflicting Content-Length headers");
            }
            length = Some(value);
        }
    }
    Ok(length.unwrap_or(0))
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

    tracing::debug!(%peer, method, %path, "request");

    // CORS preflight — the phone's WebView and any browser client may send it.
    if method == "OPTIONS" {
        return respond_cors(&mut stream, 204, "application/json", b"").await;
    }

    // Health is public: discovery and diagnostics rely on it.
    if path == "/health" {
        refresh_stopped_upstream(&inner).await;
        let st = inner.status_tx.borrow().clone();
        let body = json!({
            "ok": st.upstream.is_some(),
            "name": inner.cfg.node_name,
            "backend": st.backend,
            "models": st.models,
            "pairing": inner.cfg.require_pairing,
            "version": env!("CARGO_PKG_VERSION"),
        });
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            body.to_string().as_bytes(),
        )
        .await;
    }

    // Pairing endpoints.
    if path == "/pair" && method == "GET" {
        // The QR is displayed locally. Publishing its secret to an unpaired
        // LAN caller would bypass pairing entirely.
        if !peer.ip().is_loopback() && !authorized(&inner, head) {
            return respond_cors(
                &mut stream,
                401,
                "application/json",
                b"{\"error\":{\"message\":\"read the pairing code on the laptop\"}}",
            )
            .await;
        }
        let handle = NodeHandle {
            inner: inner.clone(),
        };
        let payload = handle.pair_payload();
        let hw = current_self_hardware(&inner);
        let body = json!({
            "name": inner.cfg.node_name,
            "host": payload.host,
            "port": inner.port,
            "token": inner.pair_token,
            // Everything a phone needs, in one string it can scan.
            "uri": payload.uri(),
            "hardware": {
                "ram_mb": hw.ram_mb,
                "available_mb": hw.available_mb,
                "budget_mb": hw.budget_mb,
                "cores": hw.cores,
                "threads": hw.threads,
                "kind": hw.kind,
                "arch": hw.arch,
            },
            "cluster_ready": handle.cluster().toolchain().is_some(),
        });
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            body.to_string().as_bytes(),
        )
        .await;
    }
    if path == "/pair/verify" && method == "POST" {
        let ok = authorized(&inner, head);
        let body = json!({ "ok": ok });
        return respond_cors(
            &mut stream,
            if ok { 200 } else { 401 },
            "application/json",
            body.to_string().as_bytes(),
        )
        .await;
    }

    if inner.cfg.require_pairing && !authorized(&inner, head) {
        let body = json!({ "error": { "message": "pair token required", "type": "auth_error" } });
        return respond_cors(
            &mut stream,
            401,
            "application/json",
            body.to_string().as_bytes(),
        )
        .await;
    }

    // Even nodes that allow unauthenticated chat must not let arbitrary LAN or
    // browser callers make the laptop fetch and persist multi-gigabyte files.
    if matches!(
        path.as_str(),
        "/models/browse" | "/managed/download" | "/managed/install"
    ) && !authorized(&inner, head)
    {
        return respond_cors(
            &mut stream,
            401,
            "application/json",
            b"{\"error\":{\"message\":\"pair token required for model downloads\"}}",
        )
        .await;
    }

    // ---- device / cluster API -------------------------------------------------
    if path == "/managed" && method == "GET" {
        let body = serde_json::to_vec(&inner.managed.view().await)?;
        return respond_cors(&mut stream, 200, "application/json", &body).await;
    }
    if path == "/models/browse" && method == "POST" {
        let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let Some(query) = req["query"].as_str() else {
            return respond_cors(
                &mut stream,
                400,
                "application/json",
                b"{\"error\":\"query must be a search phrase, owner/repo, or GGUF link\"}",
            )
            .await;
        };
        let (code, body) = match inner.managed.browse(query).await {
            Ok(results) => (200, serde_json::to_value(results)?),
            Err(error) => (400, json!({"error": format!("{error:#}")})),
        };
        return respond_cors(
            &mut stream,
            code,
            "application/json",
            body.to_string().as_bytes(),
        )
        .await;
    }
    if path == "/managed/download" && method == "POST" {
        let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let source =
            match serde_json::from_value::<crate::model_hub::ModelSource>(req["source"].clone()) {
                Ok(source) => source,
                Err(error) => {
                    let body = json!({"error":format!("invalid model source: {error}")});
                    return respond_cors(
                        &mut stream,
                        400,
                        "application/json",
                        body.to_string().as_bytes(),
                    )
                    .await;
                }
            };
        let (code, body) = match inner.managed.download(&source).await {
            Ok(model) => {
                NodeHandle {
                    inner: inner.clone(),
                }
                .add_model(model.clone())
                .await;
                (200, json!({"ok":true,"model":model}))
            }
            Err(error) => (400, json!({"ok":false,"error":format!("{error:#}")})),
        };
        return respond_cors(
            &mut stream,
            code,
            "application/json",
            body.to_string().as_bytes(),
        )
        .await;
    }
    if path == "/managed/install" && method == "POST" {
        let req: Value = match serde_json::from_slice(&body) {
            Ok(Value::Object(value)) => Value::Object(value),
            Ok(_) => {
                return respond_cors(
                    &mut stream,
                    400,
                    "application/json",
                    b"{\"error\":\"request must be a JSON object\"}",
                )
                .await
            }
            Err(error) => {
                let body = json!({"error": format!("bad json: {error}")});
                return respond_cors(
                    &mut stream,
                    400,
                    "application/json",
                    body.to_string().as_bytes(),
                )
                .await;
            }
        };
        let Some(model) = req["model"].as_str().filter(|model| !model.is_empty()) else {
            return respond_cors(
                &mut stream,
                400,
                "application/json",
                b"{\"error\":\"model is required\"}",
            )
            .await;
        };
        if Managed::catalog_model(model).is_none() && !inner.managed.has_custom_model(model) {
            let body = json!({"error": format!("unknown model '{model}' — browse models or provide a GGUF source first")});
            return respond_cors(
                &mut stream,
                404,
                "application/json",
                body.to_string().as_bytes(),
            )
            .await;
        }
        match inner.managed.install(model).await {
            Ok(model) => {
                NodeHandle {
                    inner: inner.clone(),
                }
                .add_model(model.clone())
                .await;
                let body = json!({"ok": true, "model": model});
                return respond_cors(
                    &mut stream,
                    200,
                    "application/json",
                    body.to_string().as_bytes(),
                )
                .await;
            }
            Err(error) => {
                let body = json!({"ok": false, "error": error.to_string()});
                return respond_cors(
                    &mut stream,
                    500,
                    "application/json",
                    body.to_string().as_bytes(),
                )
                .await;
            }
        }
    }

    // Phones report their hardware here so the planner can use real numbers.
    if path == "/peers" && method == "POST" {
        let body: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                let b = json!({ "error": format!("bad json: {e}") });
                return respond_cors(
                    &mut stream,
                    400,
                    "application/json",
                    b.to_string().as_bytes(),
                )
                .await;
            }
        };
        let ram_mb = body["ram_mb"].as_u64().unwrap_or(0);
        let hw = Hardware::from_raw_available(
            body["id"].as_str().unwrap_or("peer"),
            body["name"].as_str().unwrap_or("device"),
            body["kind"].as_str().unwrap_or("phone"),
            ram_mb,
            body["available_mb"].as_u64().unwrap_or(ram_mb),
            body["cores"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(4),
            body["arch"].as_str().unwrap_or("arm64"),
        );
        if hw.id.is_empty() || hw.id == inner.hardware.id || hw.ram_mb == 0 {
            return respond_cors(
                &mut stream,
                400,
                "application/json",
                b"{\"error\":\"a unique device id and positive ram_mb are required\"}",
            )
            .await;
        }
        let rpc = body["rpc"].as_str().map(String::from);
        let worker = match body.get("worker") {
            Some(value) => match serde_json::from_value::<WorkerReport>(value.clone()) {
                Ok(report)
                    if report.validate() && (rpc.is_some() == (report.state == "running")) =>
                {
                    Some(report)
                }
                _ => {
                    return respond_cors(
                        &mut stream,
                        400,
                        "application/json",
                        b"{\"error\":\"invalid worker report\"}",
                    )
                    .await
                }
            },
            None => None,
        };
        if !register_peer(&inner, hw.clone(), rpc, worker).await {
            return respond_cors(
                &mut stream,
                200,
                "application/json",
                b"{\"ok\":true,\"ignored\":true}",
            )
            .await;
        }
        // Only count memory we can actually use: a peer must have a reachable
        // RPC worker, otherwise its budget is a promise it cannot keep.
        let self_hw = current_self_hardware(&inner);
        let self_enabled = inner.self_enabled.load(Ordering::Relaxed);
        let mut usable = if self_enabled { self_hw.budget_mb } else { 0 };
        let mut usable_devices = u32::from(self_enabled && self_hw.can_host_layers);
        let mut unusable = 0u32;
        let mut disabled = u32::from(!self_enabled);
        for (p, worker) in peer_states(&inner).await {
            if !p.enabled {
                disabled += 1;
            } else if worker.compatible && (p.hw.can_host_layers || worker.in_use) {
                usable = usable.saturating_add(p.hw.budget_mb);
                usable_devices += 1;
            } else {
                unusable += 1;
            }
        }
        let b = json!({
            "ok": true,
            "you": { "budget_mb": hw.budget_mb, "threads": hw.threads },
            "pool": {
                "devices": usable_devices,
                "total_budget_mb": usable,
                "unusable_devices": unusable,
                "disabled_devices": disabled,
                "note": if unusable > 0 {
                    "devices that reported hardware but have no reachable RPC worker are not counted"
                } else { "" },
            }
        });
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    }
    if path == "/peers/toggle" && method == "POST" {
        let req: Value = match serde_json::from_slice(&body) {
            Ok(Value::Object(value)) => Value::Object(value),
            _ => {
                return respond_cors(
                    &mut stream,
                    400,
                    "application/json",
                    b"{\"error\":\"id and enabled are required\"}",
                )
                .await
            }
        };
        let Some(id) = req["id"].as_str().filter(|id| !id.is_empty()) else {
            return respond_cors(
                &mut stream,
                400,
                "application/json",
                b"{\"error\":\"id is required\"}",
            )
            .await;
        };
        let Some(enabled) = req["enabled"].as_bool() else {
            return respond_cors(
                &mut stream,
                400,
                "application/json",
                b"{\"error\":\"enabled must be a boolean\"}",
            )
            .await;
        };
        let found = if id == inner.hardware.id {
            inner.self_enabled.store(enabled, Ordering::Relaxed);
            true
        } else {
            let mut peers = inner.peers.lock().await;
            if let Some(peer) = peers.iter_mut().find(|peer| peer.hw.id == id) {
                peer.enabled = enabled;
                true
            } else {
                false
            }
        };
        if !found {
            return respond_cors(
                &mut stream,
                404,
                "application/json",
                b"{\"error\":\"unknown device\"}",
            )
            .await;
        }
        let body = json!({"ok": true, "id": id, "enabled": enabled});
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            body.to_string().as_bytes(),
        )
        .await;
    }
    if path == "/peers" && method == "GET" {
        let self_hw = current_self_hardware(&inner);
        let self_enabled = inner.self_enabled.load(Ordering::Relaxed);
        let cluster = inner.cluster.status().await;
        let assignments = if matches!(
            cluster.state,
            crate::cluster::ClusterState::Starting | crate::cluster::ClusterState::Running
        ) {
            cluster.assignments.clone()
        } else {
            Vec::new()
        };
        let allocation = |id: &str| {
            assignments
                .iter()
                .find(|assignment| assignment.device_id == id)
                .map(|assignment| (assignment.layers, assignment.memory_mb))
                .unwrap_or((0, 0))
        };
        let (self_layers, self_memory) = allocation(&self_hw.id);
        let list: Vec<Value> = std::iter::once(json!({
            "id": self_hw.id, "name": self_hw.name, "kind": self_hw.kind,
            "ram_mb": self_hw.ram_mb, "available_mb": self_hw.available_mb,
            "budget_mb": self_hw.budget_mb,
            "cores": self_hw.cores, "threads": self_hw.threads,
            "arch": self_hw.arch, "rpc": null, "self": true,
            "enabled": self_enabled, "reachable": true,
            "usable": self_enabled && (self_hw.can_host_layers || self_layers > 0),
            "in_use": self_layers > 0,
            "heartbeat_seconds": 0,
            "assigned_layers": self_layers, "assigned_memory_mb": self_memory,
        }))
        .chain(peer_states(&inner).await.into_iter().map(|(p, worker)| {
            let (assigned_layers, assigned_memory) = allocation(&p.hw.id);
            json!({
                "id": p.hw.id, "name": p.hw.name, "kind": p.hw.kind,
                "ram_mb": p.hw.ram_mb, "available_mb": p.hw.available_mb,
                "budget_mb": p.hw.budget_mb,
                "cores": p.hw.cores, "threads": p.hw.threads,
                "arch": p.hw.arch, "rpc": p.rpc, "self": false,
                "stale": p.last_seen.elapsed() >= std::time::Duration::from_secs(30),
                "reachable": worker.reachable,
                "in_use": worker.in_use,
                "rpc_version": worker.version,
                "reason": worker.error,
                "worker": p.worker,
                "enabled": p.enabled,
                "heartbeat_seconds": p.last_seen.elapsed().as_secs(),
                // Usable = we can put layers there right now.
                "usable": p.enabled && worker.compatible && (p.hw.can_host_layers || worker.in_use),
                "assigned_layers": assigned_layers,
                "assigned_memory_mb": assigned_memory,
            })
        }))
        .collect();
        let b = json!({ "devices": list });
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    }

    if path == "/models" && method == "GET" {
        let models = inner.models.lock().await;
        let list: Vec<Value> = models
            .iter()
            .map(|m| {
                json!({
                    "name": m.name, "path": m.path, "params_b": m.params_b,
                    "size_mb": m.size_mb, "layers": m.layers,
                })
            })
            .collect();
        let b = json!({ "models": list, "cluster_ready": inner.cluster.toolchain().is_some() });
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    }

    // Register a GGUF so it can be planned and served.
    // Body: {"path": "/x.gguf", "name": "optional", "params_b": 4.0, "layers": 36}
    if path == "/models" && method == "POST" {
        let req: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
        let p = req["path"].as_str().unwrap_or("");
        let name = req["name"].as_str().map(String::from).unwrap_or_else(|| {
            std::path::Path::new(p)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "model".into())
        });
        let mut m = match ModelInfo::from_file(std::path::Path::new(p), &name) {
            Ok(model) => model,
            Err(error) => {
                let body = json!({"error": format!("invalid GGUF: {error:#}")});
                return respond_cors(
                    &mut stream,
                    400,
                    "application/json",
                    body.to_string().as_bytes(),
                )
                .await;
            }
        };
        if req.get("layers").is_some() && req["layers"].as_u64() != Some(u64::from(m.layers)) {
            return respond_cors(
                &mut stream,
                400,
                "application/json",
                b"{\"error\":\"layers must match the GGUF block count\"}",
            )
            .await;
        }
        m.params_b = m.params_b.or_else(|| req["params_b"].as_f64());
        if let Some(kv) = req["kv_bytes_per_token"].as_u64() {
            m.kv_bytes_per_token = m.kv_bytes_per_token.max(kv);
        }
        NodeHandle {
            inner: inner.clone(),
        }
        .add_model(m.clone())
        .await;
        let b = json!({ "ok": true, "model": m });
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    }

    if path == "/cluster" && method == "GET" {
        let st: ClusterStatus = inner.cluster.status().await;
        let b = json!({
            "state": st.state, "model": st.model, "endpoint": st.endpoint,
            "generation": st.generation,
            "peers": st.peers, "layers_on_self": st.layers_on_self,
            "total_layers": st.total_layers, "assignments": st.assignments,
            "error": st.error,
            "log": st.log_path,
            "toolchain": inner.cluster.toolchain().map(|t| json!({
                "llama_server": t.llama_server.display().to_string(),
                "rpc_server": t.rpc_server.display().to_string(),
            })),
        });
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    }

    // Body: {"model": "<name|path>", "ctx": 4096} — plans, then serves.
    if path == "/cluster/start" && method == "POST" {
        let requested_generation = inner.cluster.status().await.generation;
        let req: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
        let want = req["model"].as_str().unwrap_or("").to_string();
        let ctx = match request_context(&req, inner.cfg.ctx) {
            Some(ctx) => ctx,
            None => {
                return respond_cors(
                    &mut stream,
                    400,
                    "application/json",
                    b"{\"error\":\"ctx must be a positive 32-bit integer\"}",
                )
                .await
            }
        };
        let handle = NodeHandle {
            inner: inner.clone(),
        };

        let models = handle.models_available().await;
        let model = match models.iter().find(|m| m.name == want || m.path == want) {
            Some(m) => m.clone(),
            None => {
                let b = json!({ "error": format!("unknown model '{want}' — see GET /models") });
                return respond_cors(
                    &mut stream,
                    404,
                    "application/json",
                    b.to_string().as_bytes(),
                )
                .await;
            }
        };

        let model = match ModelInfo::from_file(std::path::Path::new(&model.path), &model.name) {
            Ok(current) => ModelInfo {
                kv_bytes_per_token: current.kv_bytes_per_token.max(model.kv_bytes_per_token),
                ..current
            },
            Err(error) => {
                let body = json!({"error": format!("invalid GGUF: {error:#}")});
                return respond_cors(
                    &mut stream,
                    400,
                    "application/json",
                    body.to_string().as_bytes(),
                )
                .await;
            }
        };
        if let Err(error) = inner.managed.validate_model(&model).await {
            let body = json!({"error": error.to_string()});
            return respond_cors(
                &mut stream,
                400,
                "application/json",
                body.to_string().as_bytes(),
            )
            .await;
        }
        handle.add_model(model.clone()).await;

        let plan = handle.plan_with(&model, ctx).await;
        if !plan.fits() {
            let b = json!({ "error": "split does not fit", "plan": plan });
            return respond_cors(
                &mut stream,
                507,
                "application/json",
                b.to_string().as_bytes(),
            )
            .await;
        }

        let st = inner
            .cluster
            .start_if_current(
                &plan,
                std::path::Path::new(&model.path),
                ctx,
                requested_generation,
            )
            .await;
        refresh_engine(&inner).await;
        let code = if st
            .as_ref()
            .is_ok_and(|s| s.state == crate::cluster::ClusterState::Running)
        {
            200
        } else {
            500
        };
        let b = match st {
            Ok(s) => {
                json!({ "ok": s.state == crate::cluster::ClusterState::Running, "status": s, "plan": plan })
            }
            Err(e) => json!({ "ok": false, "error": e.to_string(), "plan": plan }),
        };
        return respond_cors(
            &mut stream,
            code,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    }

    if path == "/cluster/plan" && method == "POST" {
        let req: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
        let want = req["model"].as_str().unwrap_or("").to_string();
        let ctx = match request_context(&req, inner.cfg.ctx) {
            Some(ctx) => ctx,
            None => {
                return respond_cors(
                    &mut stream,
                    400,
                    "application/json",
                    b"{\"error\":\"ctx must be a positive 32-bit integer\"}",
                )
                .await
            }
        };
        let handle = NodeHandle {
            inner: inner.clone(),
        };
        match handle.plan_model(&want, ctx).await {
            Some(p) => {
                let b = json!({ "plan": p });
                return respond_cors(
                    &mut stream,
                    200,
                    "application/json",
                    b.to_string().as_bytes(),
                )
                .await;
            }
            None => {
                let b = json!({ "error": format!("unknown model '{want}'") });
                return respond_cors(
                    &mut stream,
                    404,
                    "application/json",
                    b.to_string().as_bytes(),
                )
                .await;
            }
        }
    }

    if path == "/cluster/stop" && method == "POST" {
        let r = inner.cluster.stop().await;
        refresh_engine(&inner).await;
        let b = match r {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        };
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    }

    inner.status_tx.send_modify(|s| s.requests += 1);

    match (method, path.as_str()) {
        ("GET", "/v1/models") => {
            refresh_stopped_upstream(&inner).await;
            let st = inner.status_tx.borrow().clone();
            let data: Vec<Value> = st
                .models
                .iter()
                .map(|m| json!({ "id": m, "object": "model", "owned_by": st.backend }))
                .collect();
            let body = json!({ "object": "list", "data": data });
            respond_cors(
                &mut stream,
                200,
                "application/json",
                body.to_string().as_bytes(),
            )
            .await
        }
        ("POST", "/v1/chat/completions") => chat_completions(inner, stream, body).await,
        _ => {
            respond_cors(
                &mut stream,
                404,
                "application/json",
                b"{\"error\":\"not found\"}",
            )
            .await
        }
    }
}

fn authorized(inner: &Inner, headers: &str) -> bool {
    headers
        .lines()
        .filter_map(|l| l.split_once(':'))
        .any(|(name, value)| {
            if !name.eq_ignore_ascii_case("authorization") {
                return false;
            }
            let mut parts = value.split_whitespace();
            let scheme = parts.next().unwrap_or("");
            let token = parts.next().unwrap_or("");
            scheme.eq_ignore_ascii_case("bearer")
                && !token.is_empty()
                && token == inner.pair_token
                && parts.next().is_none()
        })
}

fn request_context(req: &Value, default: u32) -> Option<u32> {
    match req.get("ctx") {
        None => Some(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0),
    }
}

// ------------------------------------------------------------------- chat

async fn chat_completions(inner: Arc<Inner>, mut stream: TcpStream, body: Vec<u8>) -> Result<()> {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let b = json!({ "error": { "message": format!("bad json: {e}"), "type": "invalid_request_error" } });
            return respond_cors(
                &mut stream,
                400,
                "application/json",
                b.to_string().as_bytes(),
            )
            .await;
        }
    };

    if !req.is_object() {
        return respond_cors(
            &mut stream,
            400,
            "application/json",
            b"{\"error\":{\"message\":\"request must be a JSON object\"}}",
        )
        .await;
    }
    refresh_stopped_upstream(&inner).await;
    let status = inner.status_tx.borrow().clone();
    let Some(upstream) = status.upstream else {
        let msg = "No model is loaded. Open the TUI model picker with Up, select a model, \
                   and press Enter; PrivateLM will download and start it.";
        let b = json!({ "error": { "message": msg, "type": "server_error" } });
        return respond_cors(
            &mut stream,
            503,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    };

    let mut req = req;
    if req
        .get("model")
        .and_then(|m| m.as_str())
        .map(|m| m.is_empty())
        .unwrap_or(true)
    {
        let st = inner.status_tx.borrow().clone();
        let fallback = inner
            .cfg
            .default_model
            .clone()
            .or_else(|| st.models.first().cloned());
        if let Some(m) = fallback {
            req["model"] = json!(m);
        }
    }

    let streaming = req.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let url = format!("{}/chat/completions", upstream.trim_end_matches('/'));

    let resp = inner.http.post(&url).json(&req).send().await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            refresh_engine(&inner).await;
            inner
                .status_tx
                .send_modify(|s| s.last_error = Some(format!("upstream: {e}")));
            let b = json!({ "error": { "message": format!("upstream unreachable: {e}"), "type": "upstream_error" } });
            return respond_cors(
                &mut stream,
                502,
                "application/json",
                b.to_string().as_bytes(),
            )
            .await;
        }
    };

    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let b = json!({ "error": { "message": text, "type": "upstream_error" } });
        return respond_cors(
            &mut stream,
            code,
            "application/json",
            b.to_string().as_bytes(),
        )
        .await;
    }

    if !streaming {
        let v: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                return respond_cors(
                    &mut stream,
                    502,
                    "application/json",
                    json!({"error": {"message": format!("invalid upstream response: {e}")}})
                        .to_string()
                        .as_bytes(),
                )
                .await
            }
        };
        return respond_cors(
            &mut stream,
            200,
            "application/json",
            v.to_string().as_bytes(),
        )
        .await;
    }

    // SSE passthrough — headers first, then chunks as they arrive.
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
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
                inner
                    .status_tx
                    .send_modify(|s| s.last_error = Some(format!("stream: {e}")));
                let error =
                    json!({"error": {"message": format!("upstream stream interrupted: {e}")}});
                let _ = stream
                    .write_all(format!("data: {error}\n\n").as_bytes())
                    .await;
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
        408 => "Request Timeout",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct TestNode {
        inner: Arc<Inner>,
        url: String,
        task: tokio::task::JoinHandle<()>,
    }

    impl TestNode {
        async fn new() -> Self {
            Self::with_pairing(true).await
        }

        async fn with_pairing(require_pairing: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (status_tx, _) = watch::channel(ServerStatus::default());
            let (shutdown, _) = broadcast::channel(1);
            let inner = Arc::new(Inner {
                cfg: Config {
                    require_pairing,
                    ..Default::default()
                },
                pair_token: "MixedCaseToken".into(),
                status_tx,
                http: reqwest::Client::new(),
                advert: None,
                shutdown,
                port,
                hardware: Hardware::from_raw("local", "laptop", "laptop", 2400, 4, "x86_64"),
                self_enabled: AtomicBool::new(true),
                cluster: Cluster::new(),
                peers: Default::default(),
                models: Default::default(),
                managed: Managed::new().unwrap(),
            });
            let state = inner.clone();
            let task = tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let state = state.clone();
                    tokio::spawn(async move {
                        // Exercise the LAN auth policy, not the local QR exception.
                        handle_conn(state, stream, "192.0.2.2:1234".parse().unwrap())
                            .await
                            .unwrap();
                    });
                }
            });
            Self {
                inner,
                url: format!("http://127.0.0.1:{port}"),
                task,
            }
        }
    }

    #[tokio::test]
    async fn remote_model_download_controls_always_require_the_pair_token() {
        let node = TestNode::with_pairing(false).await;
        let client = reqwest::Client::new();
        for path in ["/models/browse", "/managed/download", "/managed/install"] {
            assert_eq!(
                client
                    .post(format!("{}{path}", node.url))
                    .json(&json!({}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                401,
                "{path}"
            );
            assert_ne!(
                client
                    .post(format!("{}{path}", node.url))
                    .bearer_auth("MixedCaseToken")
                    .json(&json!({}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                401,
                "{path}"
            );
        }
    }

    impl Drop for TestNode {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    #[tokio::test]
    async fn pairing_protects_control_endpoints_and_keeps_tokens_case_sensitive() {
        let node = TestNode::new().await;
        let client = reqwest::Client::new();
        for path in [
            "/pair",
            "/peers",
            "/peers/toggle",
            "/models",
            "/managed",
            "/cluster",
            "/v1/models",
        ] {
            assert_eq!(
                client
                    .get(format!("{}{path}", node.url))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                401,
                "{path}"
            );
        }
        for path in [
            "/peers",
            "/models",
            "/models/browse",
            "/managed/install",
            "/managed/download",
            "/cluster/start",
            "/cluster/plan",
            "/cluster/stop",
        ] {
            assert_eq!(
                client
                    .post(format!("{}{path}", node.url))
                    .json(&json!({}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                401,
                "{path}"
            );
        }
        assert_eq!(
            client
                .get(format!("{}/health", node.url))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        for (token, expected) in [("MixedCaseToken", 200), ("mixedcasetoken", 401)] {
            let r = client
                .post(format!("{}/pair/verify", node.url))
                .header("aUtHoRiZaTiOn", format!("bEaReR {token}"))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), expected);
        }
        let managed: Value = client
            .get(format!("{}/managed", node.url))
            .bearer_auth("MixedCaseToken")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(managed["catalog"].as_array().unwrap().len(), 5);
        assert_eq!(
            managed["catalog"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["recommended"] == true)
                .count(),
            1
        );
        assert_eq!(
            client
                .post(format!("{}/managed/install", node.url))
                .bearer_auth("MixedCaseToken")
                .json(&json!({"model":"not-in-catalog"}))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }

    #[tokio::test]
    async fn catalog_models_can_be_planned_before_install_but_not_started() {
        let node = TestNode::new().await;
        let client = reqwest::Client::new();
        let model = crate::managed::CATALOG.last().unwrap();
        let request = json!({"model": model.id, "ctx": 1024});

        let response = client
            .post(format!("{}/cluster/plan", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["plan"]["model"], model.id);
        assert_eq!(value["plan"]["total_layers"], model.layers);
        assert_eq!(value["plan"]["ctx"], 1024);
        assert_eq!(value["plan"]["over_budget"], true);
        assert!(node.inner.models.lock().await.is_empty());

        assert_eq!(
            client
                .post(format!("{}/cluster/start", node.url))
                .bearer_auth("MixedCaseToken")
                .json(&request)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .post(format!("{}/cluster/plan", node.url))
                .bearer_auth("MixedCaseToken")
                .json(&json!({"model":"not-a-model", "ctx":1024}))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }

    #[tokio::test]
    async fn selected_upstream_accepts_history_over_64k() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_url = format!("http://{}/v1", upstream.local_addr().unwrap());
        let received = tokio::spawn(async move {
            let (mut stream, _) = upstream.accept().await.unwrap();
            let (head, body) = read_request(&mut stream).await.unwrap().unwrap();
            assert!(head.starts_with("POST /v1/chat/completions "));
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["model"], "registered-model");
            assert_eq!(
                body["messages"][0]["content"].as_str().unwrap().len(),
                90_000
            );
            respond_cors(
                &mut stream,
                200,
                "application/json",
                br#"{"choices":[{"message":{"content":"ok"}}]}"#,
            )
            .await
            .unwrap();
        });
        let node = TestNode::new().await;
        node.inner.status_tx.send_modify(|s| {
            s.backend = "llamacpp".into();
            s.models = vec!["registered-model".into()];
            s.upstream = Some(upstream_url);
        });
        let response = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&json!({"messages": [{"role":"user", "content": "x".repeat(90_000)}]}))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.json::<Value>().await.unwrap()["choices"][0]["message"]["content"],
            "ok"
        );
        received.await.unwrap();
    }

    #[tokio::test]
    async fn stopped_managed_runs_do_not_leave_health_and_models_ready() {
        let node = TestNode::new().await;
        node.inner.status_tx.send_modify(|s| {
            s.backend = "managed".into();
            s.models = vec!["old-model".into()];
            s.upstream = Some("http://127.0.0.1:1/v1".into());
        });
        let client = reqwest::Client::new();
        let health: Value = client
            .get(format!("{}/health", node.url))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["ok"], false);
        let models: Value = client
            .get(format!("{}/v1/models", node.url))
            .bearer_auth("MixedCaseToken")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(models["data"], json!([]));
    }

    #[tokio::test]
    async fn malformed_requests_are_rejected_without_breaking_the_node() {
        let node = TestNode::new().await;
        let client = reqwest::Client::new();
        let r = client
            .post(format!("{}/v1/chat/completions", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&json!([]))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        for ctx in [json!(0), json!(u64::MAX), json!("4096")] {
            let r = client
                .post(format!("{}/cluster/plan", node.url))
                .bearer_auth("MixedCaseToken")
                .json(&json!({"model":"test", "ctx":ctx}))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 400);
        }
        assert_eq!(
            client
                .get(format!("{}/health", node.url))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert!(content_length("POST / HTTP/1.1\r\nContent-Length: nope").is_err());
        assert!(
            content_length("POST / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2").is_err()
        );
        assert!(content_length("POST / HTTP/1.1\r\nTransfer-Encoding: chunked").is_err());
    }

    #[tokio::test]
    async fn stale_but_reachable_peers_are_used_in_plans_and_pool_totals() {
        let worker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc = worker.local_addr().unwrap().to_string();
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = worker.accept().await {
                tokio::spawn(crate::rpc::serve_test_probe(socket));
            }
        });
        let node = TestNode::new().await;
        node.inner.peers.lock().await.push(Peer {
            hw: Hardware::from_raw("phone", "phone", "phone", 2900, 4, "arm64"),
            rpc: Some(rpc.clone()),
            last_seen: Instant::now() - Duration::from_secs(60),
            enabled: true,
            worker: None,
            retired_sessions: Vec::new(),
        });
        let model = ModelInfo {
            name: "test".into(),
            path: String::new(),
            params_b: None,
            size_mb: 2500,
            layers: 10,
            kv_bytes_per_token: 0,
        };
        let handle = NodeHandle {
            inner: node.inner.clone(),
        };
        let plan = handle.plan_with(&model, 4096).await;
        assert!(plan.fits(), "{}", plan.summary);
        assert!(plan
            .assignments
            .iter()
            .any(|a| a.device_id == "phone" && a.layers > 0));
        let client = reqwest::Client::new();
        let pool: Value = client
            .post(format!("{}/peers", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&json!({"id":"viewer", "ram_mb":1000}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(pool["pool"]["devices"], 2);
        assert_eq!(pool["pool"]["total_budget_mb"], 3000);
        let pool: Value = client
            .post(format!("{}/peers", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&json!({"id":"phone-new", "ram_mb":3900, "rpc":rpc}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(pool["pool"]["devices"], 2);
        assert_eq!(pool["pool"]["total_budget_mb"], 4000);
        assert!(node
            .inner
            .peers
            .lock()
            .await
            .iter()
            .all(|p| p.hw.id != "phone"));

        assert_eq!(
            client
                .post(format!("{}/peers/toggle", node.url))
                .bearer_auth("MixedCaseToken")
                .json(&json!({"id":"phone-new", "enabled":false}))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        // A heartbeat refreshes stats but must not undo the TUI exclusion.
        let pool: Value = client
            .post(format!("{}/peers", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&json!({
                "id":"phone-new", "ram_mb":3900, "available_mb":1234,
                "cores":6, "rpc":rpc
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(pool["pool"]["devices"], 1);
        assert_eq!(pool["pool"]["disabled_devices"], 1);
        let devices: Value = client
            .get(format!("{}/peers", node.url))
            .bearer_auth("MixedCaseToken")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let phone = devices["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|device| device["id"] == "phone-new")
            .unwrap();
        assert_eq!(phone["enabled"], false);
        assert_eq!(phone["usable"], false);
        assert_eq!(phone["available_mb"], 1234);
        assert_eq!(phone["cores"], 6);
        assert!(phone["heartbeat_seconds"].as_u64().unwrap() <= 1);
        let plan = handle.plan_with(&model, 4096).await;
        assert!(
            !plan.fits(),
            "disabled phone must not contribute: {}",
            plan.summary
        );

        client
            .post(format!("{}/peers", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&json!({
                "id":"phone-new", "ram_mb":3900, "available_mb":3900,
                "cores":6, "rpc":rpc
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        client
            .post(format!("{}/peers/toggle", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&json!({"id":"phone-new", "enabled":true}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        client
            .post(format!("{}/peers/toggle", node.url))
            .bearer_auth("MixedCaseToken")
            .json(&json!({"id":"local", "enabled":false}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let plan = handle.plan_with(&model, 4096).await;
        assert!(
            plan.fits(),
            "phone can host this test model: {}",
            plan.summary
        );
        assert!(plan
            .assignments
            .iter()
            .all(|assignment| assignment.device_id != "local"));
        task.abort();
    }

    #[tokio::test]
    async fn explicit_engine_preference_and_managed_endpoint_are_respected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                read_request(&mut stream).await.unwrap();
                respond_cors(
                    &mut stream,
                    200,
                    "application/json",
                    br#"{"data":[{"id":"chosen"}]}"#,
                )
                .await
                .unwrap();
            }
        });
        let cfg = Config {
            engine: "llamacpp".into(),
            llamacpp_url: Some(format!("{url}/")),
            ..Default::default()
        };
        let engine = probe_engine(&cfg, &ClusterStatus::default()).await;
        assert_eq!(engine.backend, "llamacpp");
        assert_eq!(engine.url, Some(url.clone()));
        let cluster = ClusterStatus {
            state: crate::cluster::ClusterState::Running,
            endpoint: Some(url.clone()),
            peers: vec!["phone:50052".into()],
            ..Default::default()
        };
        let engine = probe_engine(&cfg, &cluster).await;
        assert_eq!(engine.backend, "distributed");
        assert_eq!(engine.url, Some(url));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn active_single_client_workers_are_not_probed_by_pool_refreshes() {
        let node = TestNode::new().await;
        let worker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = worker.local_addr().unwrap().to_string();
        node.inner.peers.lock().await.push(Peer {
            hw: Hardware::from_raw_available("phone", "phone", "phone", 3900, 100, 4, "arm64"),
            rpc: Some(address.clone()),
            last_seen: Instant::now(),
            enabled: true,
            worker: None,
            retired_sessions: Vec::new(),
        });
        let active = ClusterStatus {
            state: crate::cluster::ClusterState::Running,
            peers: vec![address],
            ..Default::default()
        };
        node.inner.cluster.set_test_status(active).await;
        for _ in 0..10 {
            let peers = peer_states(&node.inner).await;
            assert!(peers[0].1.in_use && peers[0].1.compatible);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), worker.accept())
                .await
                .is_err()
        );
    }

    fn phone_report(session: &str, sequence: u64, state: &str) -> WorkerReport {
        WorkerReport {
            session: session.into(),
            sequence,
            state: state.into(),
            ..Default::default()
        }
    }

    fn assigned_phone(session: &str) -> ClusterStatus {
        ClusterStatus {
            state: crate::cluster::ClusterState::Running,
            peers: vec!["10.0.0.2:50054".into()],
            assignments: vec![crate::plan::SplitAssignment {
                device_id: "phone".into(),
                device_name: "phone".into(),
                kind: "phone".into(),
                layers: 3,
                memory_mb: 414,
                budget_mb: 1600,
                rpc: Some("10.0.0.2:50054".into()),
                worker_session: Some(session.into()),
            }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn worker_restart_on_the_same_port_invalidates_loaded_buffers() {
        let node = TestNode::new().await;
        let hw = Hardware::from_raw("phone", "phone", "phone", 3900, 4, "arm64");
        let rpc = Some("10.0.0.2:50054".into());
        assert!(
            register_peer(
                &node.inner,
                hw.clone(),
                rpc.clone(),
                Some(phone_report("one", 1, "running"))
            )
            .await
        );
        node.inner
            .cluster
            .set_test_status(assigned_phone("one"))
            .await;
        assert!(
            register_peer(
                &node.inner,
                hw.clone(),
                rpc.clone(),
                Some(phone_report("one", 2, "running"))
            )
            .await
        );
        assert_eq!(
            node.inner.cluster.status().await.state,
            crate::cluster::ClusterState::Running
        );
        assert!(
            register_peer(
                &node.inner,
                hw,
                rpc,
                Some(phone_report("two", 1, "running"))
            )
            .await
        );
        let status = node.inner.cluster.status().await;
        assert_eq!(status.state, crate::cluster::ClusterState::Failed);
        assert!(status.error.unwrap().contains("worker restarted"));
    }

    #[tokio::test]
    async fn late_heartbeats_and_withdrawals_cannot_replace_a_newer_session() {
        let node = TestNode::new().await;
        let hw = Hardware::from_raw("phone", "phone", "phone", 3900, 4, "arm64");
        let rpc = Some("10.0.0.2:50054".into());
        assert!(
            register_peer(
                &node.inner,
                hw.clone(),
                rpc.clone(),
                Some(phone_report("one", 1, "running"))
            )
            .await
        );
        assert!(
            register_peer(
                &node.inner,
                hw.clone(),
                rpc.clone(),
                Some(phone_report("two", 1, "running"))
            )
            .await
        );
        node.inner
            .cluster
            .set_test_status(assigned_phone("two"))
            .await;
        for address in [None, rpc.clone()] {
            let state = if address.is_some() {
                "running"
            } else {
                "stopped"
            };
            assert!(
                !register_peer(
                    &node.inner,
                    hw.clone(),
                    address,
                    Some(phone_report("one", 99, state))
                )
                .await
            );
        }
        assert!(
            !register_peer(
                &node.inner,
                hw.clone(),
                None,
                Some(phone_report("two", 1, "stopped"))
            )
            .await
        );
        assert_eq!(
            node.inner.cluster.status().await.state,
            crate::cluster::ClusterState::Running
        );
        assert!(
            register_peer(
                &node.inner,
                hw.clone(),
                None,
                Some(phone_report("two", 3, "stopped"))
            )
            .await
        );
        assert!(
            !register_peer(
                &node.inner,
                hw,
                rpc,
                Some(phone_report("two", 2, "running"))
            )
            .await
        );
        assert!(node.inner.peers.lock().await[0].rpc.is_none());
    }

    #[tokio::test]
    async fn address_changes_and_delayed_exit_evidence_are_reported() {
        let node = TestNode::new().await;
        let hw = Hardware::from_raw("phone", "phone", "phone", 3900, 4, "arm64");
        node.inner
            .cluster
            .set_test_status(assigned_phone("one"))
            .await;
        register_peer(
            &node.inner,
            hw.clone(),
            Some("10.0.0.3:50054".into()),
            Some(phone_report("one", 1, "running")),
        )
        .await;
        let status = node.inner.cluster.status().await;
        assert_eq!(status.state, crate::cluster::ClusterState::Failed);
        assert!(status.error.unwrap().contains("address changed"));

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("run.log");
        std::fs::write(&path, "native abort\n").unwrap();
        let mut failed = assigned_phone("one");
        failed.state = crate::cluster::ClusterState::Failed;
        failed.error = Some("RPC connection lost".into());
        failed.log_path = path.display().to_string();
        node.inner.cluster.set_test_status(failed).await;
        let mut report = phone_report("one", 2, "error");
        report.exit_code = Some(-9);
        report.error = Some("worker exited with SIGKILL; Android may have killed it".into());
        report.log_tail = Some("last native worker output".into());
        register_peer(&node.inner, hw, None, Some(report)).await;
        let status = node.inner.cluster.status().await;
        assert!(status.error.unwrap().contains("SIGKILL"));
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .contains("last native worker output"));
    }

    #[test]
    fn a_replacement_plan_can_reuse_the_previous_runs_memory() {
        let mut hardware =
            Hardware::from_raw_available("phone", "phone", "phone", 3900, 1000, 4, "arm64");
        let original = hardware.budget_mb;
        let active = ClusterStatus {
            state: crate::cluster::ClusterState::Running,
            assignments: vec![crate::plan::SplitAssignment {
                device_id: "phone".into(),
                device_name: "phone".into(),
                kind: "phone".into(),
                layers: 14,
                memory_mb: 1400,
                budget_mb: 3000,
                rpc: Some("10.0.0.2:50052".into()),
                worker_session: None,
            }],
            ..Default::default()
        };
        credit_released_allocation(&mut hardware, Some("10.0.0.2:50053"), None, &active);
        assert_eq!(
            hardware.budget_mb, original,
            "a different worker cannot reclaim the old allocation"
        );
        credit_released_allocation(&mut hardware, Some("10.0.0.2:50052"), None, &active);
        assert_eq!(hardware.budget_mb, 1500);
    }

    #[test]
    fn restarted_worker_cannot_reclaim_memory_from_a_stale_run_snapshot() {
        let mut hardware =
            Hardware::from_raw_available("phone", "phone", "phone", 3900, 1000, 4, "arm64");
        let original = hardware.budget_mb;
        let active = assigned_phone("old-session");
        credit_released_allocation(
            &mut hardware,
            Some("10.0.0.2:50054"),
            Some("new-session"),
            &active,
        );
        assert_eq!(hardware.budget_mb, original);
    }
}
