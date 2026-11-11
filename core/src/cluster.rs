//! Cluster manager — actually runs a split model.
//!
//! llama-server owns the model and serves OpenAI-compatible requests. Laptop
//! layers stay on its local CPU backend; only remote phone layers use RPC.
//!
//! Local-only command:
//!   llama-server -m m.gguf --host 127.0.0.1 --port 8081 \
//!                -c 4096 -t 4 -ngl 0

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};

use crate::plan::{SplitAssignment, SplitPlan};
use crate::rpc::{WorkerReport, WorkerStatus};

/// Where a cluster run stands.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ClusterState {
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterStatus {
    #[serde(default)]
    pub generation: u64,
    pub state: ClusterState,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub peers: Vec<String>,
    pub layers_on_self: u32,
    pub total_layers: u32,
    pub assignments: Vec<SplitAssignment>,
    pub error: Option<String>,
    pub rpc_port: u16,
    pub server_port: u16,
    pub log_path: String,
}

impl Default for ClusterStatus {
    fn default() -> Self {
        Self {
            generation: 0,
            state: ClusterState::Stopped,
            model: None,
            endpoint: None,
            peers: vec![],
            layers_on_self: 0,
            total_layers: 0,
            assignments: vec![],
            error: None,
            rpc_port: crate::RPC_PORT,
            server_port: 8081,
            log_path: String::new(),
        }
    }
}

/// llama.cpp binaries we need.
#[derive(Debug, Clone)]
pub struct Toolchain {
    pub llama_server: PathBuf,
    pub rpc_server: PathBuf,
}

impl Toolchain {
    /// Find llama.cpp. Order: explicit override, PrivateLM's own bin dir,
    /// then the common dev locations people end up with.
    pub fn discover() -> Option<Self> {
        Self::explicit()
            .or_else(Self::managed)
            .or_else(Self::discover_system)
    }

    pub fn explicit() -> Option<Self> {
        let server = std::env::var("PRIVATELM_LLAMA_SERVER").ok()?;
        let rpc = std::env::var("PRIVATELM_RPC_SERVER").ok()?;
        let tools = Self {
            llama_server: PathBuf::from(server),
            rpc_server: PathBuf::from(rpc),
        };
        (is_exec(&tools.llama_server) && is_exec(&tools.rpc_server)).then_some(tools)
    }

    pub fn managed() -> Option<Self> {
        let base = crate::config::bin_dir().join("llama-b12110");
        let tools = Self {
            llama_server: base.join("llama-server"),
            rpc_server: base.join("ggml-rpc-server"),
        };
        (is_exec(&tools.llama_server) && is_exec(&tools.rpc_server)).then_some(tools)
    }

    fn discover_system() -> Option<Self> {
        for base in [
            crate::config::bin_dir(),
            home_subdir("tools/llama.cpp/build/bin"),
            home_subdir("llama.cpp/build/bin"),
            PathBuf::from("/usr/local/lib/ollama"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ] {
            let server = base.join("llama-server");
            for name in ["ggml-rpc-server", "rpc-server"] {
                let rpc = base.join(name);
                if is_exec(&server) && is_exec(&rpc) {
                    return Some(Self {
                        llama_server: server,
                        rpc_server: rpc,
                    });
                }
            }
        }
        None
    }

    /// Is this llama-server new enough to understand `--rpc`?
    pub fn supports_rpc(&self) -> bool {
        // The flag exists in every build that ships a ggml-rpc-server, and we
        // only pair binaries from the same tree, so presence is enough.
        self.rpc_server.exists()
    }
}

fn home_subdir(rel: &str) -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(rel))
        .unwrap_or_else(|_| PathBuf::from(rel))
}

fn is_exec(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

struct Inner {
    server: Option<Child>,
    status: ClusterStatus,
    generation: u64,
    workers: HashMap<String, (Option<String>, Option<WorkerReport>)>,
}

#[derive(Default)]
struct ProbeSlot {
    cached: Option<(std::time::Instant, u64, WorkerStatus)>,
}

/// Owns the child processes for one cluster run.
#[derive(Clone)]
pub struct Cluster {
    inner: Arc<Mutex<Inner>>,
    rpc_gate: Arc<RwLock<()>>,
    probes: Arc<Mutex<HashMap<String, Arc<Mutex<ProbeSlot>>>>>,
    override_tools: Option<Toolchain>,
    log_dir: PathBuf,
}

impl Cluster {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                server: None,
                status: ClusterStatus::default(),
                generation: 0,
                workers: HashMap::new(),
            })),
            rpc_gate: Arc::new(RwLock::new(())),
            probes: Arc::new(Mutex::new(HashMap::new())),
            override_tools: None,
            log_dir: crate::config::logs_dir(),
        }
    }

    pub fn toolchain(&self) -> Option<Toolchain> {
        self.override_tools.clone().or_else(Toolchain::discover)
    }

    pub async fn status(&self) -> ClusterStatus {
        let mut g = self.inner.lock().await;
        poll_child(&mut g).await;
        g.status.clone()
    }

    /// Serialize probes to each single-client endpoint and exclude startup.
    /// Recheck ownership after waiting; an earlier status snapshot is unsafe.
    pub(crate) async fn worker_status(&self, address: &str) -> WorkerStatus {
        let slot = self
            .probes
            .lock()
            .await
            .entry(address.to_string())
            .or_default()
            .clone();
        let mut slot = slot.lock().await;
        let _access = self.rpc_gate.read().await;
        let active = self.status().await;
        if matches!(active.state, ClusterState::Starting | ClusterState::Running)
            && active.peers.iter().any(|peer| peer == address)
        {
            slot.cached = None;
            return WorkerStatus::active();
        }
        if let Some((at, generation, status)) = &slot.cached {
            if *generation == active.generation && at.elapsed() < std::time::Duration::from_secs(1)
            {
                return status.clone();
            }
        }
        let status = crate::rpc::probe(address).await;
        slot.cached = Some((std::time::Instant::now(), active.generation, status.clone()));
        status
    }

    pub(crate) async fn observe_worker(
        &self,
        device_id: &str,
        rpc: Option<String>,
        report: Option<WorkerReport>,
    ) {
        let mut g = self.inner.lock().await;
        let detail = worker_change(&g.status, device_id, rpc.as_deref(), report.as_ref());
        let previous = g
            .workers
            .insert(device_id.to_string(), (rpc, report.clone()));
        if let Some(detail) = detail {
            if matches!(
                g.status.state,
                ClusterState::Starting | ClusterState::Running
            ) {
                fail_locked(&mut g, detail).await;
            } else if g.status.state == ClusterState::Failed {
                // The native abort may reach the coordinator before the phone's
                // exit report. Preserve both rather than lose the useful cause.
                if let Some(error) = report.as_ref().and_then(|report| report.error.as_ref()) {
                    let detail = format!("Phone worker report: {error}");
                    if !g.status.error.as_deref().unwrap_or("").contains(&detail) {
                        append_log(&g.status.log_path, &detail);
                        g.status
                            .error
                            .get_or_insert_with(String::new)
                            .push_str(&format!("; {detail}"));
                    }
                }
            }
        }
        if g.status.state == ClusterState::Failed
            && g.status
                .assignments
                .iter()
                .any(|assignment| assignment.device_id == device_id && assignment.layers > 0)
        {
            if let Some(log) = report.as_ref().and_then(|report| report.log_tail.as_ref()) {
                let previous_log = previous
                    .as_ref()
                    .and_then(|(_, report)| report.as_ref())
                    .and_then(|report| report.log_tail.as_ref());
                if !log.is_empty() && previous_log != Some(log) {
                    append_log(
                        &g.status.log_path,
                        &format!("phone {device_id} worker output:\n{log}"),
                    );
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn set_test_status(&self, status: ClusterStatus) {
        self.inner.lock().await.status = status;
    }

    /// Start serving `model` using layers from `self` plus any RPC peers.
    pub async fn start(
        &self,
        plan: &SplitPlan,
        model_path: &Path,
        ctx: u32,
    ) -> Result<ClusterStatus> {
        let generation = self.status().await.generation;
        self.start_if_current(plan, model_path, ctx, generation)
            .await
    }

    pub(crate) async fn start_if_current(
        &self,
        plan: &SplitPlan,
        model_path: &Path,
        ctx: u32,
        requested_generation: u64,
    ) -> Result<ClusterStatus> {
        let tools = match self.toolchain() {
            Some(t) => t,
            None => bail!(
                "llama.cpp not found. PrivateLM needs `llama-server` and `ggml-rpc-server` \
                 (set PRIVATELM_LLAMA_SERVER / PRIVATELM_RPC_SERVER if they live elsewhere)."
            ),
        };
        if !model_path.is_file() {
            bail!("model file not found: {}", model_path.display());
        }
        let metadata = crate::gguf::inspect(model_path)?;
        if metadata.layers != plan.total_layers {
            bail!("GGUF has {} transformer blocks but the plan has {}; refresh the model registration", metadata.layers, plan.total_layers);
        }
        if !plan.fits() || plan.assignments.iter().all(|a| a.layers == 0) {
            bail!("cannot start a split that does not fit");
        }
        if ctx == 0 || ctx != plan.ctx {
            bail!("context must match the nonzero context used by the split plan");
        }
        let self_layers = plan
            .assignments
            .iter()
            .filter(|a| a.rpc.is_none())
            .map(|a| a.layers)
            .sum();
        let threads = crate::hardware::probe_self("local").threads;
        // Wait for any in-flight idle preflights before claiming these sockets.
        // Once Starting is published, new pool polls must use that ownership.
        let rpc_access = self.rpc_gate.write().await;
        let (generation, server_port, mut log) = {
            let mut g = self.inner.lock().await;
            if g.generation != requested_generation {
                bail!("cluster start cancelled");
            }
            stop_children(&mut g).await;
            g.generation += 1;
            g.status = ClusterStatus::default();
            let server_port = crate::config::pick_port(8081)?;
            std::fs::create_dir_all(&self.log_dir)?;
            let (log_path, log) = new_log(&self.log_dir)?;
            g.status = ClusterStatus {
                generation: g.generation,
                state: ClusterState::Starting,
                model: Some(plan.model.clone()),
                peers: plan
                    .assignments
                    .iter()
                    .filter(|a| a.layers > 0)
                    .filter_map(|a| a.rpc.clone())
                    .collect(),
                layers_on_self: self_layers,
                total_layers: plan.total_layers,
                assignments: plan.assignments.clone(),
                server_port,
                log_path: log_path.display().to_string(),
                ..Default::default()
            };
            (g.generation, server_port, log)
        };

        writeln!(
            log,
            "model={} context={} threads={threads}",
            plan.model, ctx
        )?;
        for assignment in &plan.assignments {
            writeln!(
                log,
                "device={} layers={} estimate_mib={} rpc={} worker_session={}",
                assignment.device_id,
                assignment.layers,
                assignment.memory_mb,
                assignment.rpc.as_deref().unwrap_or("local CPU"),
                assignment.worker_session.as_deref().unwrap_or("legacy")
            )?;
        }
        for address in plan
            .assignments
            .iter()
            .filter(|a| a.layers > 0)
            .filter_map(|a| a.rpc.as_ref())
        {
            self.current_status(generation).await?;
            let worker = crate::rpc::probe(address).await;
            self.current_status(generation).await?;
            if !worker.compatible {
                let error = format!(
                    "RPC worker {address} is not ready: {}",
                    worker.error.unwrap_or_default()
                );
                writeln!(log, "{error}")?;
                return self.fail(generation, error).await;
            }
        }

        {
            let mut g = self.inner.lock().await;
            let changed = g.workers.iter().find_map(|(id, (rpc, report))| {
                worker_change(&g.status, id, rpc.as_deref(), report.as_ref())
            });
            if let Some(error) = changed {
                fail_locked(&mut g, error).await;
                return Ok(g.status.clone());
            }
        }

        // Readiness waits never hold the state lock: /cluster and /stop remain
        // usable while a large model is loading.
        let mut cmd = server_command(&tools, plan, model_path, ctx, server_port, threads);
        writeln!(log, "launch: {:?}", cmd.as_std())?;
        cmd.stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .kill_on_drop(true);
        {
            let mut g = self.inner.lock().await;
            if g.generation != generation {
                bail!("cluster start cancelled");
            }
            if g.status.state == ClusterState::Failed {
                return Ok(g.status.clone());
            }
            match cmd.spawn() {
                Ok(child) => g.server = Some(child),
                Err(e) => {
                    fail_locked(&mut g, format!("could not start llama-server: {e}")).await;
                    return Ok(g.status.clone());
                }
            }
        }
        drop(rpc_access);
        self.monitor(generation);

        let endpoint = format!("http://127.0.0.1:{server_port}");
        let client = reqwest::Client::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(180);
        loop {
            let status = self.current_status(generation).await?;
            if status.state == ClusterState::Failed {
                return Ok(status);
            }
            if tokio::time::Instant::now() >= deadline {
                return self
                    .fail(
                        generation,
                        "llama-server did not become ready in 180s".into(),
                    )
                    .await;
            }
            if !log_failure(&status.log_path, &status.peers).is_empty() {
                return self
                    .fail(
                        generation,
                        "llama-server failed while loading the model".into(),
                    )
                    .await;
            }
            if let Ok(r) = client
                .get(format!("{endpoint}/health"))
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
            {
                if r.status().is_success() {
                    // A process winning the port race must not impersonate
                    // this run. The managed alias identifies the loaded model.
                    let models = client
                        .get(format!("{endpoint}/v1/models"))
                        .timeout(std::time::Duration::from_secs(2))
                        .send()
                        .await;
                    let ours = match models {
                        Ok(r) if r.status().is_success() => r
                            .json::<serde_json::Value>()
                            .await
                            .ok()
                            .and_then(|v| {
                                v["data"].as_array().map(|items| {
                                    items.iter().any(|m| m["id"].as_str() == Some(&plan.model))
                                })
                            })
                            .unwrap_or(false),
                        _ => false,
                    };
                    if !ours {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        continue;
                    }
                    self.current_status(generation).await?;
                    let mut g = self.inner.lock().await;
                    if g.generation != generation {
                        bail!("cluster start cancelled");
                    }
                    if g.status.state == ClusterState::Failed {
                        return Ok(g.status.clone());
                    }
                    g.status.state = ClusterState::Running;
                    g.status.endpoint = Some(format!("{endpoint}/v1"));
                    return Ok(g.status.clone());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(750)).await;
        }
    }

    /// Stop serving and wait for the managed child to exit.
    pub async fn stop(&self) -> Result<()> {
        let mut g = self.inner.lock().await;
        g.generation += 1;
        stop_children(&mut g).await;
        g.status = ClusterStatus {
            rpc_port: g.status.rpc_port,
            server_port: g.status.server_port,
            ..Default::default()
        };
        Ok(())
    }

    fn monitor(&self, generation: u64) {
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let Some(inner) = inner.upgrade() else {
                    return;
                };
                let mut g = inner.lock().await;
                if g.generation != generation
                    || !matches!(
                        g.status.state,
                        ClusterState::Starting | ClusterState::Running
                    )
                {
                    return;
                }
                poll_child(&mut g).await;
            }
        });
    }

    async fn current_status(&self, generation: u64) -> Result<ClusterStatus> {
        let mut g = self.inner.lock().await;
        if g.generation != generation {
            bail!("cluster start cancelled");
        }
        poll_child(&mut g).await;
        Ok(g.status.clone())
    }

    async fn fail(&self, generation: u64, error: String) -> Result<ClusterStatus> {
        let mut g = self.inner.lock().await;
        if g.generation != generation {
            bail!("cluster start cancelled");
        }
        fail_locked(&mut g, error).await;
        Ok(g.status.clone())
    }
}

fn server_command(
    tools: &Toolchain,
    plan: &SplitPlan,
    path: &Path,
    ctx: u32,
    server_port: u16,
    threads: u32,
) -> Command {
    let remote: Vec<_> = plan
        .assignments
        .iter()
        .filter(|assignment| assignment.layers > 0 && assignment.rpc.is_some())
        .collect();
    let local_layers: u32 = plan
        .assignments
        .iter()
        .filter(|assignment| assignment.rpc.is_none())
        .map(|assignment| assignment.layers)
        .sum();
    let remote_layers: u32 = remote.iter().map(|assignment| assignment.layers).sum();
    let peers = remote
        .iter()
        .filter_map(|assignment| assignment.rpc.clone())
        .collect::<Vec<_>>();
    // llama.cpp includes the output layer in -ngl and in its split fractions.
    // Give that extra position to the last remote device, then keep the output
    // weights on CPU for mixed runs. Transformer block counts now match plans.
    let weights = remote
        .iter()
        .enumerate()
        .map(|(i, assignment)| (assignment.layers + u32::from(i + 1 == remote.len())).to_string())
        .collect::<Vec<_>>()
        .join(",");
    let devices = peers
        .iter()
        .enumerate()
        .map(|(i, _)| format!("RPC{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut cmd = Command::new(&tools.llama_server);
    cmd.arg("-m").arg(path).args([
        "--alias",
        &plan.model,
        "--host",
        "127.0.0.1",
        "--port",
        &server_port.to_string(),
        "-c",
        &ctx.to_string(),
        "-t",
        &threads.to_string(),
        "--parallel",
        "1",
        "--batch-size",
        "256",
        "--ubatch-size",
        "64",
        "--cache-ram",
        "0",
        "--fit",
        "off",
    ]);
    if remote.is_empty() {
        cmd.args(["-ngl", "0"]);
    } else {
        // RPC devices use llama.cpp's offload path even when the phone worker
        // itself is CPU-only. Keep non-offloaded layers directly on the laptop
        // CPU instead of routing them through a fragile local RPC connection.
        let offload = if local_layers == 0 {
            "999".to_string()
        } else {
            remote_layers.saturating_add(1).to_string()
        };
        cmd.args([
            "-ngl",
            &offload,
            "--rpc",
            &peers.join(","),
            "--device",
            &devices,
            "--split-mode",
            "layer",
            "--tensor-split",
            &weights,
        ]);
        if local_layers > 0 {
            cmd.args(["--override-tensor", "^output.*=CPU"]);
        }
    }
    #[cfg(target_os = "linux")]
    kill_if_parent_exits(&mut cmd);
    cmd
}

#[cfg(target_os = "linux")]
fn kill_if_parent_exits(cmd: &mut Command) {
    let parent = std::process::id() as libc::pid_t;
    // kill_on_drop cannot run if the node crashes or receives SIGKILL. Have the
    // kernel release the native model process in that case as well.
    // SAFETY: the pre-exec callback only makes allocation-free system calls and
    // constructs OS-code errors between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            ) == -1
            {
                return Err(std::io::Error::last_os_error());
            }
            // The parent could have exited between fork and setting the guard.
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}

fn child_exit(child: &mut Option<Child>, name: &str) -> Option<String> {
    match child.as_mut()?.try_wait() {
        Ok(Some(code)) => Some(format!("{name} exited ({code})")),
        Err(e) => Some(format!("could not check {name}: {e}")),
        Ok(None) => None,
    }
}

async fn stop_children(g: &mut Inner) {
    if let Some(server) = g.server.as_mut() {
        let _ = server.start_kill();
        let _ = server.wait().await;
    }
    g.server = None;
}

async fn fail_locked(g: &mut Inner, error: String) {
    let details = log_failure(&g.status.log_path, &g.status.peers);
    stop_children(g).await;
    g.status.state = ClusterState::Failed;
    g.status.endpoint = None;
    g.status.error = Some(format!("{error}{details}. See {}", g.status.log_path));
    append_log(&g.status.log_path, g.status.error.as_deref().unwrap_or(""));
}

async fn poll_child(g: &mut Inner) {
    g.status.generation = g.generation;
    if matches!(
        g.status.state,
        ClusterState::Starting | ClusterState::Running
    ) {
        if let Some(error) = child_exit(&mut g.server, "llama-server") {
            fail_locked(g, error).await;
        } else if g.server.is_some() && !log_failure(&g.status.log_path, &g.status.peers).is_empty()
        {
            fail_locked(g, "llama-server reported a fatal error".into()).await;
        }
    }
}

fn new_log(directory: &Path) -> Result<(PathBuf, std::fs::File)> {
    static NEXT_LOG: AtomicU64 = AtomicU64::new(0);
    loop {
        let path = directory.join(format!(
            "cluster-{}-{}.log",
            std::process::id(),
            NEXT_LOG.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).with_context(|| format!("create {}", path.display())),
        }
    }
}

fn log_failure(path: &str, peers: &[String]) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file
        .seek(SeekFrom::Start(size.saturating_sub(32 * 1024)))
        .is_err()
    {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.take(32 * 1024).read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    if text.contains("failed to allocate") || text.contains("out of memory") {
        return "; model allocation failed — free RAM or choose a smaller model/context".into();
    }
    if text.contains("Remote RPC server crashed") {
        return format!("; RPC connection lost ({}) after connecting: the remote worker closed, crashed, or lost its transport. Being on the same LAN does not keep an Android worker alive. Check the phone worker exit report, then restart sharing and reload the model", peers.join(", "));
    }
    if text.contains("RPC handshake failed") || text.contains("Failed to connect to") {
        return format!("; RPC connection failed during setup ({}) — verify the advertised address, worker session, and LAN reachability", peers.join(", "));
    }
    String::new()
}

fn append_log(path: &str, message: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = writeln!(file, "coordinator: {message}");
    }
}

fn worker_change(
    status: &ClusterStatus,
    device_id: &str,
    rpc: Option<&str>,
    report: Option<&WorkerReport>,
) -> Option<String> {
    let assignment = status.assignments.iter().find(|assignment| {
        assignment.device_id == device_id && assignment.layers > 0 && assignment.rpc.is_some()
    })?;
    let reason = match rpc {
        None => report
            .and_then(|report| report.error.clone())
            .unwrap_or_else(|| "sharing stopped or the worker exited".into()),
        Some(address) if assignment.rpc.as_deref() != Some(address) => format!(
            "RPC address changed from {} to {}; the loaded session cannot migrate",
            assignment.rpc.as_deref().unwrap(),
            address
        ),
        Some(_)
            if assignment
                .worker_session
                .as_deref()
                .is_some_and(|session| report.is_some_and(|report| report.session != session)) =>
        {
            "worker restarted; its previous model buffers no longer exist".into()
        }
        _ => return None,
    };
    Some(format!(
        "RPC worker '{}': {reason}. Restart sharing if needed, then reload the model",
        assignment.device_name
    ))
}

impl Default for Cluster {
    fn default() -> Self {
        Self::new()
    }
}

/// Does an idle worker speak the pinned RPC protocol?
/// Active workers must be checked through the owning cluster, not probed again.
pub async fn rpc_reachable(addr: &str) -> bool {
    crate::rpc::probe(addr).await.compatible
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Hardware;
    use crate::plan::ModelInfo;

    #[test]
    fn discovers_the_toolchain_on_this_box() {
        // The dev machine has a llama.cpp build; a missing one is also valid,
        // so only assert consistency between the two answers.
        if let Some(t) = Toolchain::discover() {
            assert!(t.llama_server.exists());
            assert!(t.rpc_server.exists());
            assert!(t.supports_rpc());
        }
    }

    #[tokio::test]
    async fn status_starts_stopped() {
        let c = Cluster::new();
        let s = c.status().await;
        assert_eq!(s.state, ClusterState::Stopped);
    }

    #[tokio::test]
    async fn start_rejects_missing_model_file() {
        let c = Cluster::new();
        let m = ModelInfo {
            name: "x".into(),
            path: "/nope.gguf".into(),
            params_b: Some(4.0),
            size_mb: 2400,
            layers: 36,
            kv_bytes_per_token: 1,
        };
        let hw = Hardware::from_raw("a", "a", "laptop", 16000, 8, "x86_64");
        let p = crate::plan::plan(&m, &[(hw, None)], 4096);
        if c.toolchain().is_some() {
            let err = c.start(&p, Path::new("/nope.gguf"), 4096).await;
            assert!(err.is_err());
        }
    }

    #[test]
    fn launch_enables_offload_and_applies_the_plan_in_peer_order() {
        let model = ModelInfo {
            name: "test".into(),
            path: "model.gguf".into(),
            params_b: None,
            size_mb: 1000,
            layers: 10,
            kv_bytes_per_token: 0,
        };
        let local = Hardware::from_raw("local", "local", "laptop", 1800, 4, "x86_64");
        let phone = Hardware::from_raw("phone", "phone", "phone", 3900, 4, "arm64");
        let plan = crate::plan::plan(
            &model,
            &[(local, None), (phone, Some("10.0.0.2:50052".into()))],
            4096,
        );
        let tools = Toolchain {
            llama_server: "/bin/true".into(),
            rpc_server: "/bin/true".into(),
        };
        let command = server_command(&tools, &plan, Path::new("model.gguf"), 4096, 8082, 4);
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        let value = |flag: &str| args[args.iter().position(|s| s == flag).unwrap() + 1].clone();
        assert_eq!(value("-ngl"), "7");
        assert_eq!(value("--rpc"), "10.0.0.2:50052");
        assert_eq!(value("--tensor-split"), "7");
        assert_eq!(value("--override-tensor"), "^output.*=CPU");
        assert_eq!(value("--device"), "RPC0");
        assert_eq!(value("--alias"), "test");
    }

    #[test]
    fn local_only_launch_keeps_layers_on_the_direct_cpu_backend() {
        let model = ModelInfo {
            name: "local".into(),
            path: "model.gguf".into(),
            params_b: None,
            size_mb: 1000,
            layers: 10,
            kv_bytes_per_token: 0,
        };
        let plan = crate::plan::plan(
            &model,
            &[(
                Hardware::from_raw("local", "local", "laptop", 8000, 4, "x86_64"),
                None,
            )],
            4096,
        );
        let tools = Toolchain {
            llama_server: "/bin/true".into(),
            rpc_server: "/bin/true".into(),
        };
        let command = server_command(&tools, &plan, Path::new("model.gguf"), 4096, 8082, 4);
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args[args.iter().position(|arg| arg == "-ngl").unwrap() + 1],
            "0"
        );
        assert!(!args.iter().any(|arg| arg == "--rpc"));
        assert!(!args.iter().any(|arg| arg == "--device"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn starting_cluster_can_be_observed_and_cancelled_without_waiting_for_readiness() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let worker = dir.path().join("worker");
        std::fs::write(&worker, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let model_path = dir.path().join("model.gguf");
        std::fs::write(&model_path, crate::gguf::fixture(1)).unwrap();
        let mut cluster = Cluster::new();
        cluster.override_tools = Some(Toolchain {
            llama_server: worker.clone(),
            rpc_server: worker,
        });
        cluster.log_dir = dir.path().join("logs");
        let model = ModelInfo {
            name: "test".into(),
            path: model_path.display().to_string(),
            params_b: None,
            size_mb: 1,
            layers: 1,
            kv_bytes_per_token: 0,
        };
        let plan = crate::plan::plan(
            &model,
            &[(
                Hardware::from_raw("l", "l", "laptop", 8000, 4, "x86_64"),
                None,
            )],
            4096,
        );
        let running = cluster.clone();
        let start = tokio::spawn(async move { running.start(&plan, &model_path, 4096).await });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let status = loop {
                let status = cluster.status().await;
                if status.state == ClusterState::Starting {
                    break status;
                }
                tokio::task::yield_now().await;
            };
            assert_eq!(status.assignments.len(), 1);
            assert_eq!(status.assignments[0].device_id, "l");
            assert_eq!(status.assignments[0].layers, 1);
            cluster.stop().await.unwrap();
        })
        .await
        .expect("status and stop must not wait for the worker to become ready");
        assert!(start.await.unwrap().is_err());
        assert_eq!(cluster.status().await.state, ClusterState::Stopped);
        let inner = cluster.inner.lock().await;
        assert!(inner.server.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fatal_rpc_log_fails_startup_without_waiting_for_the_deadline() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let server = dir.path().join("server");
        std::fs::write(
            &server,
            "#!/bin/sh\necho 'Remote RPC server crashed or returned malformed response'\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o700)).unwrap();
        let model_path = dir.path().join("model.gguf");
        std::fs::write(&model_path, crate::gguf::fixture(1)).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            crate::rpc::serve_test_probe(listener.accept().await.unwrap().0).await;
        });

        let mut cluster = Cluster::new();
        cluster.override_tools = Some(Toolchain {
            llama_server: server.clone(),
            rpc_server: server,
        });
        cluster.log_dir = dir.path().join("logs");
        let model = ModelInfo {
            name: "test".into(),
            path: model_path.display().to_string(),
            params_b: None,
            size_mb: 1,
            layers: 1,
            kv_bytes_per_token: 0,
        };
        let phone = Hardware {
            id: "phone".into(),
            name: "phone".into(),
            kind: "phone".into(),
            budget_mb: 512,
            can_host_layers: true,
            ..Default::default()
        };
        let plan = crate::plan::plan(&model, &[(phone, Some(rpc))], 4096);

        let status = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            cluster.start(&plan, &model_path, 4096),
        )
        .await
        .expect("fatal RPC output should fail startup promptly")
        .unwrap();

        assert_eq!(status.state, ClusterState::Failed);
        assert!(status.error.unwrap().contains("RPC connection lost"));
    }

    #[tokio::test]
    async fn assigned_worker_withdrawal_fails_the_cluster() {
        let cluster = Cluster::new();
        {
            let mut inner = cluster.inner.lock().await;
            inner.status.state = ClusterState::Running;
            inner.status.assignments = vec![crate::plan::SplitAssignment {
                device_id: "phone".into(),
                device_name: "phone".into(),
                kind: "phone".into(),
                layers: 1,
                memory_mb: 100,
                budget_mb: 500,
                rpc: Some("10.0.0.2:50052".into()),
                worker_session: None,
            }];
        }

        cluster.observe_worker("other", None, None).await;
        assert_eq!(cluster.status().await.state, ClusterState::Running);
        cluster
            .observe_worker(
                "phone",
                None,
                Some(WorkerReport {
                    error: Some("phone worker stopped".into()),
                    ..Default::default()
                }),
            )
            .await;
        let status = cluster.status().await;
        assert_eq!(status.state, ClusterState::Failed);
        assert!(status.error.unwrap().contains("phone worker stopped"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fatal_rpc_output_fails_a_running_child_even_before_it_exits() {
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("run.log");
        std::fs::write(
            &log,
            "model loaded\nrecv returned 0 (peer closed?)\nRemote RPC server crashed or returned malformed response\n",
        )
        .unwrap();
        let child = Command::new("sleep")
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let cluster = Cluster::new();
        {
            let mut inner = cluster.inner.lock().await;
            inner.server = Some(child);
            inner.status.state = ClusterState::Running;
            inner.status.endpoint = Some("http://127.0.0.1:8081/v1".into());
            inner.status.peers = vec!["phone:50054".into()];
            inner.status.log_path = log.display().to_string();
        }
        cluster.monitor(0);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while cluster.inner.lock().await.status.state != ClusterState::Failed {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the monitor must fail the run even without client polling");
        let status = cluster.status().await;
        assert_eq!(status.state, ClusterState::Failed);
        assert!(status.endpoint.is_none());
        assert!(status.error.unwrap().contains("RPC connection lost"));
        assert!(cluster.inner.lock().await.server.is_none());
    }

    #[tokio::test]
    async fn queued_pool_probes_recheck_ownership_after_startup() {
        let cluster = Cluster::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let exclusive = cluster.rpc_gate.write().await;
        let mut queries = Vec::new();
        for _ in 0..8 {
            let cluster = cluster.clone();
            let address = address.clone();
            queries.push(tokio::spawn(async move {
                cluster.worker_status(&address).await
            }));
        }
        tokio::task::yield_now().await;
        cluster
            .set_test_status(ClusterStatus {
                state: ClusterState::Starting,
                peers: vec![address],
                ..Default::default()
            })
            .await;
        drop(exclusive);
        for query in queries {
            assert!(query.await.unwrap().in_use);
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_cancels_a_requested_load_waiting_for_rpc_access() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        std::fs::write(&path, crate::gguf::fixture(1)).unwrap();
        let model = ModelInfo::from_file(&path, "test").unwrap();
        let plan = crate::plan::plan(
            &model,
            &[(
                Hardware::from_raw("local", "local", "laptop", 8000, 4, "x86_64"),
                None,
            )],
            1024,
        );
        let mut cluster = Cluster::new();
        cluster.override_tools = Some(Toolchain {
            llama_server: "/bin/true".into(),
            rpc_server: "/bin/true".into(),
        });
        cluster.log_dir = directory.path().join("logs");
        let polling = cluster.rpc_gate.read().await;
        let starting = cluster.clone();
        let requested_generation = cluster.status().await.generation;
        let task = tokio::spawn(async move {
            starting
                .start_if_current(&plan, &path, 1024, requested_generation)
                .await
        });
        tokio::task::yield_now().await;
        cluster.stop().await.unwrap();
        drop(polling);
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("cancelled"));
        assert_eq!(cluster.status().await.state, ClusterState::Stopped);
        assert!(cluster.inner.lock().await.server.is_none());
    }

    #[tokio::test]
    async fn startup_waits_for_the_last_idle_probe_and_parallel_polls_share_it() {
        let cluster = Cluster::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let mut queries = Vec::new();
        for _ in 0..8 {
            let cluster = cluster.clone();
            let address = address.clone();
            queries.push(tokio::spawn(async move {
                cluster.worker_status(&address).await
            }));
        }
        let (socket, _) = listener.accept().await.unwrap();
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(30),
            cluster.rpc_gate.write()
        )
        .await
        .is_err());
        crate::rpc::serve_test_probe(socket).await;
        for query in queries {
            assert!(query.await.unwrap().compatible);
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
    }
}

#[cfg(test)]
mod reach_tests {
    #[tokio::test]
    async fn unreachable_address_is_false() {
        assert!(!super::rpc_reachable("127.0.0.1:1").await);
    }

    #[tokio::test]
    async fn compatible_worker_is_true() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            crate::rpc::serve_test_probe(l.accept().await.unwrap().0).await;
        });
        assert!(super::rpc_reachable(&addr).await);
    }
}
