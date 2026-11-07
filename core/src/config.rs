//! Configuration, directories and identity.
//!
//! Everything lives under `~/.privateml/` (override with `PRIVATELM_HOME`):
//!   config.toml   user settings
//!   pair_token    shared secret the APK must present (generated on first run)
//!   models/       downloaded GGUFs
//!   bin/          managed llama.cpp binaries
//!   logs/

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Where the node listens and what it publishes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Name shown in the phone's node list. Defaults to the hostname.
    pub node_name: String,
    /// Preferred TCP port. 0 = pick the first free port at or above DEFAULT_PORT.
    pub port: u16,
    /// Keep the node reachable from the LAN. When false it binds loopback only.
    pub lan_enabled: bool,
    /// Answer UDP discovery broadcasts (lets the APK auto-find this laptop).
    pub discovery: bool,
    /// Engine preference: "auto" | "ollama" | "llamacpp" | "managed".
    pub engine: String,
    /// Overrides for engines we do not manage.
    pub ollama_url: Option<String>,
    pub llamacpp_url: Option<String>,
    /// Model used when the client does not ask for one.
    pub default_model: Option<String>,
    /// Context size passed to llama-server in managed/distributed mode.
    pub ctx: u32,
    /// Require the pair token on every request.
    pub require_pairing: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            node_name: default_node_name(),
            port: super::DEFAULT_PORT,
            lan_enabled: true,
            discovery: true,
            engine: "auto".into(),
            ollama_url: None,
            llamacpp_url: None,
            default_model: None,
            ctx: 4096,
            require_pairing: false,
        }
    }
}

fn default_node_name() -> String {
    if let Ok(h) = std::env::var("PRIVATELM_NODE_NAME") {
        return h;
    }
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "laptop".into())
}

/// Resolved home directory (created on demand).
pub fn home() -> PathBuf {
    if let Ok(h) = std::env::var("PRIVATELM_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join(".privateml")
}

pub fn models_dir() -> PathBuf {
    home().join("models")
}
pub fn bin_dir() -> PathBuf {
    home().join("bin")
}
pub fn logs_dir() -> PathBuf {
    home().join("logs")
}
pub fn config_path() -> PathBuf {
    home().join("config.toml")
}
pub fn pair_token_path() -> PathBuf {
    home().join("pair_token")
}
pub fn db_path() -> PathBuf {
    home().join("history.db")
}

impl Config {
    /// Load config, creating a default one on first run.
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            let cfg = Config::default();
            cfg.save()?;
            return Ok(cfg);
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        // Hand-rolled TOML subset: "key = value" lines, quotes optional.
        let mut cfg = Config::default();
        for line in raw.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let unq = v.trim_matches('"').trim_matches('\'');
            match k {
                "node_name" => cfg.node_name = unq.to_string(),
                "port" => { if let Ok(p) = unq.parse() { cfg.port = p } }
                "lan_enabled" => cfg.lan_enabled = unq == "true",
                "discovery" => cfg.discovery = unq == "true",
                "engine" => cfg.engine = unq.to_string(),
                "ollama_url" => cfg.ollama_url = Some(unq.to_string()),
                "llamacpp_url" => cfg.llamacpp_url = Some(unq.to_string()),
                "default_model" => cfg.default_model = Some(unq.to_string()),
                "ctx" => { if let Ok(c) = unq.parse() { cfg.ctx = c } }
                "require_pairing" => cfg.require_pairing = unq == "true",
                _ => {}
            }
        }
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(home())?;
        let mut out = String::from("# PrivateLM node configuration\n");
        out.push_str(&format!("node_name = \"{}\"\n", self.node_name));
        out.push_str(&format!("port = {}\n", self.port));
        out.push_str(&format!("lan_enabled = {}\n", self.lan_enabled));
        out.push_str(&format!("discovery = {}\n", self.discovery));
        out.push_str(&format!("engine = \"{}\"\n", self.engine));
        if let Some(u) = &self.ollama_url {
            out.push_str(&format!("ollama_url = \"{}\"\n", u));
        }
        if let Some(u) = &self.llamacpp_url {
            out.push_str(&format!("llamacpp_url = \"{}\"\n", u));
        }
        if let Some(m) = &self.default_model {
            out.push_str(&format!("default_model = \"{}\"\n", m));
        }
        out.push_str(&format!("ctx = {}\n", self.ctx));
        out.push_str(&format!("require_pairing = {}\n", self.require_pairing));
        atomic_write(&config_path(), out.as_bytes())?;
        Ok(())
    }

    /// Load or create the pair token (32 hex chars from /dev/urandom).
    pub fn pair_token(&self) -> Result<String> {
        let p = pair_token_path();
        if let Ok(s) = fs::read_to_string(&p) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                return Ok(s);
            }
        }
        let mut buf = [0u8; 16];
        // NOTE: /dev/urandom is an endless stream — never `fs::read` it whole.
        {
            use std::io::Read;
            let mut f = fs::File::open("/dev/urandom").context("open /dev/urandom")?;
            f.read_exact(&mut buf).context("read /dev/urandom")?;
        }
        let tok: String = buf.iter().map(|b| format!("{:02x}", b)).collect();
        atomic_write(&p, tok.as_bytes())?;
        restrict_permissions(&p)?;
        Ok(tok)
    }
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

/// Find a usable TCP port. `preferred == 0` means "first free at or above DEFAULT".
pub fn pick_port(preferred: u16) -> Result<u16> {
    let start = if preferred == 0 { super::DEFAULT_PORT } else { preferred };
    for p in start..start.saturating_add(100) {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return Ok(p);
        }
    }
    bail!("no free port in {start}..{start}+100");
}
