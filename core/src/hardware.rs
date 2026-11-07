//! Hardware probing — what this machine can contribute to a split model.
//!
//! Every device in the cluster reports the same shape, so the planner can
//! divide a model by available memory rather than by guesswork. The phone
//! sends its own numbers over `POST /peers`; the laptop reads /proc directly.

use serde::{Deserialize, Serialize};

/// What a device can give to a distributed run.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Hardware {
    /// Stable id for this device (hostname or phone-generated uuid).
    pub id: String,
    /// Human label for the UI.
    pub name: String,
    /// "laptop" | "phone" | "desktop"
    pub kind: String,
    /// Total physical memory.
    pub ram_mb: u64,
    /// Memory we are willing to lend to a model, after OS + app overhead.
    pub budget_mb: u64,
    /// Logical CPUs.
    pub cores: u32,
    /// Threads worth using for inference (tiered, like PrivateLM).
    pub threads: u32,
    /// True when this device can host llama.cpp RPC layers.
    pub can_host_layers: bool,
    /// E.g. "arm64" / "x86_64".
    pub arch: String,
}

impl Hardware {
    /// Overhead to leave free so we never swap: OS, browser, IDE.
    pub fn default_reserve_mb(kind: &str) -> u64 {
        match kind {
            "phone" => 900,   // Android itself + the app
            "desktop" => 2048,
            _ => 1400,        // laptop: desktop env is heavier
        }
    }

    /// Thread count by RAM tier — cheap devices should not thrash.
    pub fn tier_threads(ram_mb: u64, cores: u32) -> u32 {
        let by_ram = if ram_mb >= 12000 {
            6
        } else if ram_mb >= 8000 {
            4
        } else if ram_mb >= 4000 {
            3
        } else {
            2
        };
        by_ram.min(cores.max(1))
    }

    /// Build from raw numbers, applying reserve + thread tiering.
    pub fn from_raw(id: &str, name: &str, kind: &str, ram_mb: u64, cores: u32, arch: &str) -> Self {
        let reserve = Self::default_reserve_mb(kind);
        let budget = ram_mb.saturating_sub(reserve);
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            ram_mb,
            budget_mb: budget,
            cores,
            threads: Self::tier_threads(ram_mb, cores),
            can_host_layers: budget > 0,
            arch: arch.to_string(),
        }
    }
}

impl Hardware {
    /// Probe this machine. Linux is the target; other platforms get sane fallbacks.
    pub fn probe_self(name: &str) -> Hardware {
        probe_self(name)
    }
}

/// Probe this machine. Linux is the target; other platforms get sane fallbacks.
pub fn probe_self(name: &str) -> Hardware {
    let ram_mb = read_meminfo_mb().unwrap_or(4096);
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    let kind = if cfg!(target_os = "android") {
        "phone"
    } else if is_laptop().unwrap_or(true) {
        "laptop"
    } else {
        "desktop"
    };
    let id = read_hostname().unwrap_or_else(|| "local".into());
    Hardware::from_raw(&id, name, kind, ram_mb, cores, std::env::consts::ARCH)
}

fn read_meminfo_mb() -> Option<u64> {
    let txt = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = txt
        .lines()
        .find(|l| l.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(kb / 1024)
}

fn read_hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
}

/// A battery is a decent proxy for "this is a laptop".
fn is_laptop() -> Option<bool> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type"))
            .map(|t| t.trim() == "Battery")
            .unwrap_or(false)
        {
            return Some(true);
        }
    }
    Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_is_subtracted() {
        let h = Hardware::from_raw("id", "phone", "phone", 8000, 8, "arm64");
        assert_eq!(h.ram_mb, 8000);
        assert_eq!(h.budget_mb, 8000 - 900);
        assert!(h.can_host_layers);
    }

    #[test]
    fn tiny_device_cannot_host_layers() {
        let h = Hardware::from_raw("id", "old", "phone", 700, 4, "arm64");
        assert_eq!(h.budget_mb, 0);
        assert!(!h.can_host_layers);
    }

    #[test]
    fn threads_are_tiered_and_capped_by_cores() {
        assert_eq!(Hardware::tier_threads(16000, 8), 6);
        assert_eq!(Hardware::tier_threads(16000, 2), 2); // capped
        assert_eq!(Hardware::tier_threads(2000, 8), 2);
    }

    #[test]
    fn probe_self_returns_plausible_values() {
        let h = probe_self("test");
        assert!(h.ram_mb > 0);
        assert!(h.cores >= 1);
        assert!(h.threads >= 1);
        assert!(!h.arch.is_empty());
    }
}
