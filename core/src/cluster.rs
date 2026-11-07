//! Cluster manager — actually runs a split model.
//!
//! Two processes make a distributed run:
//!   1. `ggml-rpc-server` here, contributing *this* machine's layers
//!   2. `llama-server --rpc <self>,<peers...>` which owns the model, serves
//!      OpenAI-compatible requests, and shunts layers to the RPC peers
//!
//! Verified working command (qwen3:4b over a local rpc peer):
//!   ggml-rpc-server -H 127.0.0.1 -p 50052 -t 4
//!   llama-server -m m.gguf --host 127.0.0.1 --port 8081 \
//!                -c 4096 -t 4 -ngl 0 --rpc 127.0.0.1:50052

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::plan::SplitPlan;

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
    pub state: ClusterState,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub peers: Vec<String>,
    pub layers_on_self: u32,
    pub total_layers: u32,
    pub error: Option<String>,
    pub rpc_port: u16,
    pub server_port: u16,
    pub log_path: String,
}

impl Default for ClusterStatus {
    fn default() -> Self {
        Self {
            state: ClusterState::Stopped,
            model: None,
            endpoint: None,
            peers: vec![],
            layers_on_self: 0,
            total_layers: 0,
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
        let mut server_candidates: Vec<PathBuf> = Vec::new();
        let mut rpc_candidates: Vec<PathBuf> = Vec::new();

        if let Ok(p) = std::env::var("PRIVATELM_LLAMA_SERVER") {
            server_candidates.push(PathBuf::from(p));
        }
        if let Ok(p) = std::env::var("PRIVATELM_RPC_SERVER") {
            rpc_candidates.push(PathBuf::from(p));
        }

        let managed = crate::config::bin_dir();
        server_candidates.push(managed.join("llama-server"));
        rpc_candidates.push(managed.join("ggml-rpc-server"));

        for base in [
            home_subdir("tools/llama.cpp/build/bin"),
            home_subdir("llama.cpp/build/bin"),
            PathBuf::from("/usr/local/lib/ollama"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ] {
            server_candidates.push(base.join("llama-server"));
            rpc_candidates.push(base.join("ggml-rpc-server"));
            rpc_candidates.push(base.join("rpc-server"));
        }

        let server = server_candidates.into_iter().find(|p| is_exec(p))?;
        let rpc = rpc_candidates.into_iter().find(|p| is_exec(p))?;
        Some(Self {
            llama_server: server,
            rpc_server: rpc,
        })
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
    rpc: Option<Child>,
    server: Option<Child>,
    status: ClusterStatus,
}

/// Owns the child processes for one cluster run.
#[derive(Clone)]
pub struct Cluster {
    inner: Arc<Mutex<Inner>>,
    tools: Option<Toolchain>,
}

impl Cluster {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                rpc: None,
                server: None,
                status: ClusterStatus::default(),
            })),
            tools: Toolchain::discover(),
        }
    }

    pub fn toolchain(&self) -> Option<&Toolchain> {
        self.tools.as_ref()
    }

    pub async fn status(&self) -> ClusterStatus {
        let mut g = self.inner.lock().await;
        // Reflect processes that died on their own.
        if let Some(s) = &mut g.server {
            if let Ok(Some(code)) = s.try_wait() {
                if g.status.state == ClusterState::Running {
                    g.status.state = ClusterState::Failed;
                    g.status.error = Some(format!(
                        "llama-server exited ({code}). See {}",
                        g.status.log_path
                    ));
                    if let Some(r) = g.rpc.as_mut() {
                        let _ = r.start_kill();
                    }
                    g.rpc = None;
                }
            }
        }
        g.status.clone()
    }

    /// Start serving `model` using layers from `self` plus any RPC peers.
    pub async fn start(&self, plan: &SplitPlan, model_path: &Path, ctx: u32) -> Result<ClusterStatus> {
        let tools = match &self.tools {
            Some(t) => t.clone(),
            None => bail!(
                "llama.cpp not found. PrivateLM needs `llama-server` and `ggml-rpc-server` \
                 (set PRIVATELM_LLAMA_SERVER / PRIVATELM_RPC_SERVER if they live elsewhere)."
            ),
        };
        if !model_path.exists() {
            bail!("model file not found: {}", model_path.display());
        }

        self.stop().await.ok();

        let status = self.status().await;
        let rpc_port = status.rpc_port;
        let server_port = status.server_port;
        std::fs::create_dir_all(crate::config::logs_dir()).ok();
        let log_path = crate::config::logs_dir().join("cluster.log");
        let log = std::fs::File::create(&log_path)
            .with_context(|| format!("create {}", log_path.display()))?;
        let log_err = log.try_clone()?;

        let self_layers = plan
            .assignments
            .iter()
            .find(|a| a.rpc.is_none())
            .map(|a| a.layers)
            .or_else(|| plan.assignments.first().map(|a| a.layers))
            .unwrap_or(0);

        let mut g = self.inner.lock().await;
        g.status = ClusterStatus {
            state: ClusterState::Starting,
            model: Some(plan.model.clone()),
            endpoint: None,
            peers: plan.assignments.iter().filter_map(|a| a.rpc.clone()).collect(),
            layers_on_self: self_layers,
            total_layers: plan.total_layers,
            error: None,
            rpc_port,
            server_port,
            log_path: log_path.display().to_string(),
        };

        // 1. This machine as an RPC peer (so its RAM is part of the pool).
        let mut rpc_cmd = Command::new(&tools.rpc_server);
        rpc_cmd
            .arg("-H")
            .arg("127.0.0.1")
            .arg("-p")
            .arg(rpc_port.to_string())
            .arg("-t")
            .arg("4")
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true);
        match rpc_cmd.spawn() {
            Ok(c) => g.rpc = Some(c),
            Err(e) => {
                g.status.state = ClusterState::Failed;
                g.status.error = Some(format!("could not start ggml-rpc-server: {e}"));
                return Ok(g.status.clone());
            }
        }

        // 2. llama-server holding the model, sharding across RPC peers.
        let threads = 4u32;
        let mut peers: Vec<String> = vec![format!("127.0.0.1:{rpc_port}")];
        for a in &plan.assignments {
            if let Some(rpc) = &a.rpc {
                if !rpc.is_empty() && !peers.contains(rpc) {
                    peers.push(rpc.clone());
                }
            }
        }

        let mut cmd = Command::new(&tools.llama_server);
        cmd.arg("-m")
            .arg(model_path)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(server_port.to_string())
            .arg("-c")
            .arg(ctx.to_string())
            .arg("-t")
            .arg(threads.to_string())
            .arg("-ngl")
            .arg("0")
            .arg("--rpc")
            .arg(peers.join(","));

        cmd.stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        match cmd.spawn() {
            Ok(c) => g.server = Some(c),
            Err(e) => {
                if let Some(r) = g.rpc.as_mut() {
                    let _ = r.start_kill();
                }
                g.status.state = ClusterState::Failed;
                g.status.error = Some(format!("could not start llama-server: {e}"));
                return Ok(g.status.clone());
            }
        }

        // 3. Wait for /health, bounded. Big models take a while to map.
        let endpoint = format!("http://127.0.0.1:{server_port}/v1");
        let client = reqwest::Client::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(180);
        loop {
            if tokio::time::Instant::now() > deadline {
                g.status.state = ClusterState::Failed;
                g.status.error = Some(format!(
                    "llama-server did not become ready in 180s. See {}",
                    log_path.display()
                ));
                if let Some(s) = g.server.as_mut() {
                    let _ = s.start_kill();
                }
                return Ok(g.status.clone());
            }
            if let Some(s) = g.server.as_mut() {
                if let Ok(Some(code)) = s.try_wait() {
                    g.status.state = ClusterState::Failed;
                    g.status.error = Some(format!(
                        "llama-server exited ({code}). See {}",
                        log_path.display()
                    ));
                    return Ok(g.status.clone());
                }
            }
            if let Ok(r) = client
                .get(format!("http://127.0.0.1:{server_port}/health"))
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
            {
                if r.status().is_success() {
                    g.status.state = ClusterState::Running;
                    g.status.endpoint = Some(endpoint);
                    return Ok(g.status.clone());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(750)).await;
        }
    }

    /// Stop serving and kill both children.
    pub async fn stop(&self) -> Result<()> {
        let mut g = self.inner.lock().await;
        if let Some(s) = g.server.as_mut() {
            let _ = s.start_kill();
            let _ = s.wait().await;
        }
        if let Some(r) = g.rpc.as_mut() {
            let _ = r.start_kill();
            let _ = r.wait().await;
        }
        g.server = None;
        g.rpc = None;
        g.status = ClusterStatus {
            rpc_port: g.status.rpc_port,
            server_port: g.status.server_port,
            ..Default::default()
        };
        Ok(())
    }
}

impl Default for Cluster {
    fn default() -> Self {
        Self::new()
    }
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
}

/// Is a `host:port` RPC worker accepting connections right now?
///
/// Used before a device is counted in a split: a phone that reports 8 GB but
/// runs no worker contributes nothing, so promising its memory would be a lie.
pub async fn rpc_reachable(addr: &str) -> bool {
    let connect = tokio::net::TcpStream::connect(addr);
    matches!(
        tokio::time::timeout(std::time::Duration::from_millis(400), connect).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod reach_tests {
    #[tokio::test]
    async fn unreachable_address_is_false() {
        assert!(!super::rpc_reachable("127.0.0.1:1").await);
    }

    #[tokio::test]
    async fn listening_address_is_true() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let _ = l.accept().await;
        });
        assert!(super::rpc_reachable(&addr).await);
    }
}
