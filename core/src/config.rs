//! Configuration, directories and identity.
//!
//! Everything lives under `~/.localhostai/` (override with `LOCALHOSTAI_HOME`):
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
    /// Explicit trusted exceptions to public-only model download destinations.
    pub allowed_model_hosts: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            node_name: default_node_name(),
            port: super::DEFAULT_PORT,
            lan_enabled: true,
            discovery: true,
            engine: "managed".into(),
            ollama_url: None,
            llamacpp_url: None,
            default_model: None,
            ctx: 4096,
            require_pairing: true,
            allowed_model_hosts: Vec::new(),
        }
    }
}

fn default_node_name() -> String {
    if let Ok(h) = std::env::var("LOCALHOSTAI_NODE_NAME") {
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
    if let Ok(h) = std::env::var("LOCALHOSTAI_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join(".localhostai")
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
pub fn device_state_path() -> PathBuf {
    home().join("devices.json")
}

pub fn load_device_state() -> std::collections::HashMap<String, bool> {
    std::fs::read(device_state_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_device_state(state: &std::collections::HashMap<String, bool>) -> Result<()> {
    save_device_state_at(&device_state_path(), state)
}

pub fn save_device_state_at(
    path: &Path,
    state: &std::collections::HashMap<String, bool>,
) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

impl Config {
    /// Load config, creating a default one on first run.
    pub fn load() -> Result<Self> {
        for dir in [home(), models_dir(), bin_dir(), logs_dir()] {
            fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let path = config_path();
        if !path.exists() {
            let cfg = Config::default();
            cfg.save()?;
            return Ok(cfg);
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let cfg: Self =
            toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if !matches!(
            self.engine.as_str(),
            "auto" | "ollama" | "llamacpp" | "managed"
        ) {
            bail!(
                "unknown engine '{}': use auto, ollama, llamacpp, or managed",
                self.engine
            );
        }
        if self.ctx == 0 {
            bail!("ctx must be greater than zero");
        }
        for url in [&self.ollama_url, &self.llamacpp_url].into_iter().flatten() {
            let parsed = reqwest::Url::parse(url).context("invalid engine URL")?;
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                bail!("engine URL must be an HTTP(S) address: {url}");
            }
        }
        for host in &self.allowed_model_hosts {
            if host.is_empty()
                || host.len() > 512
                || host.contains(['/', '\\', '@', ' ', '\n', '\r'])
            {
                bail!("allowed_model_hosts entries must be hostnames/IPs with an optional :port");
            }
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        self.validate()?;
        fs::create_dir_all(home())?;
        let mut out = String::from("# localhost.ai node configuration\n");
        out.push_str(&toml::to_string_pretty(self)?);
        atomic_write(&config_path(), out.as_bytes())?;
        Ok(())
    }

    /// Load or create the pair token (32 hex chars from /dev/urandom).
    pub fn pair_token(&self) -> Result<String> {
        pair_token_at(&pair_token_path())
    }
}

fn pair_token_at(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(token) => {
            let token = token.trim().to_string();
            if token.is_empty() {
                bail!("empty pair token at {}", path.display());
            }
            restrict_permissions(path)?;
            return Ok(token);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("read pair token"),
    }
    let mut buf = [0u8; 16];
    // /dev/urandom is endless: read only the bytes required for the token.
    use std::io::Read;
    fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    let token: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    match write_atomic(path, token.as_bytes(), false) {
        Ok(()) => Ok(token),
        // Another node won first-run initialization. Both must use the same
        // completely written token, rather than overwrite one another's secret.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => pair_token_at(path),
        Err(e) => Err(e).context("create pair token"),
    }
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    Ok(write_atomic(path, data, true)?)
}

fn write_atomic(path: &Path, data: &[u8], replace: bool) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let (tmp, mut file) = loop {
        let tmp = path.with_extension(format!(
            "{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match options.open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        if replace {
            fs::rename(&tmp, path)
        } else {
            fs::hard_link(&tmp, path)
        }
    })();
    let _ = fs::remove_file(tmp);
    result
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
    let start = if preferred == 0 {
        super::DEFAULT_PORT
    } else {
        preferred
    };
    for p in start..=start.saturating_add(99) {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return Ok(p);
        }
    }
    bail!("no free port in {start}..{start}+100");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_preserves_quoted_names_and_comments() {
        let cfg = Config {
            node_name: "Studio #1 \"laptop\"\nsecond line".into(),
            ..Default::default()
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let restored: Config = toml::from_str(&text).unwrap();
        assert_eq!(restored.node_name, cfg.node_name);
        let commented: Config = toml::from_str("node_name = 'Studio #1' # comment").unwrap();
        assert_eq!(commented.node_name, "Studio #1");
    }

    #[test]
    fn invalid_settings_are_not_silently_ignored() {
        assert_eq!(Config::default().engine, "managed");
        assert!(toml::from_str::<Config>("port = 70000").is_err());
        assert!(toml::from_str::<Config>("require_pairing = 'false'").is_err());
        assert!(Config {
            ctx: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(Config {
            engine: "typo".into(),
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn concurrent_nodes_share_one_complete_private_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pair_token");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    pair_token_at(&path).unwrap()
                })
            })
            .collect();
        let tokens: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(tokens.iter().all(|t| t == &tokens[0] && t.len() == 32));
        assert_eq!(fs::read_to_string(&path).unwrap(), tokens[0]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
