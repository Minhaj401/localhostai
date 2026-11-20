//! Hardware probing — what this machine can contribute to a split model.
//!
//! Every device in the cluster reports the same shape, so the planner can
//! divide a model by available memory rather than by guesswork. The phone
//! sends its own numbers over `POST /peers`; the laptop reads /proc directly.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MemoryTelemetry {
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    pub zram_logical_used_mb: u64,
    pub zram_physical_used_mb: u64,
    pub pressure_some_avg10: Option<f64>,
    pub pressure_full_avg10: Option<f64>,
    pub limit_mb: Option<u64>,
}

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
    /// Memory currently available without swapping. This caps the safe shared
    /// budget used by the planner.
    pub available_mb: u64,
    /// Memory we are willing to lend to a model, after OS + app overhead.
    pub budget_mb: u64,
    /// Logical CPUs.
    pub cores: u32,
    /// Threads worth using for inference (tiered, like localhost.ai).
    pub threads: u32,
    /// True when this device can host llama.cpp RPC layers.
    pub can_host_layers: bool,
    /// E.g. "arm64" / "x86_64".
    pub arch: String,
    /// Reserve this device keeps free for its own OS and apps (MiB). Phones
    /// report their own choice; `None` falls back to the kind-based default.
    #[serde(default)]
    pub reserve_mb: Option<u64>,
    #[serde(default)]
    pub memory: MemoryTelemetry,
}

impl Hardware {
    /// Overhead to leave free so we never swap: OS, browser, IDE.
    pub fn default_reserve_mb(kind: &str) -> u64 {
        match kind {
            "phone" => 900, // Android itself + the app
            "desktop" => 2048,
            _ => 1400, // laptop: desktop env is heavier
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

    /// Safe upper bound for a new model allocation. Busy devices advertise
    /// less memory instead of promising RAM already in use by other apps.
    pub fn share_budget_mb(kind: &str, ram_mb: u64, available_mb: u64) -> u64 {
        Self::share_budget_mb_with_reserve(ram_mb, available_mb, Self::default_reserve_mb(kind))
    }

    /// Same budget with an explicit reserve (MiB kept free for the OS/apps).
    pub fn share_budget_mb_with_reserve(ram_mb: u64, available_mb: u64, reserve_mb: u64) -> u64 {
        ram_mb
            .saturating_sub(reserve_mb)
            .min(available_mb.min(ram_mb).saturating_sub(reserve_mb))
    }

    /// Reserve this device actually uses: its reported choice, or the
    /// kind-based default when it did not report one.
    pub fn effective_reserve_mb(&self) -> u64 {
        self.reserve_mb
            .unwrap_or_else(|| Self::default_reserve_mb(&self.kind))
    }

    /// Apply a user-configured reserve, overriding the kind-based default.
    /// Records the choice on the hardware so later recomputations (released
    /// allocations, live refresh) keep honoring it.
    pub fn apply_reserve_override(&mut self, reserve_mb: Option<u64>) {
        let Some(reserve) = reserve_mb else { return };
        self.set_reserve_override(Some(reserve));
    }

    /// Set or clear the user reserve, recomputing the budget either way.
    /// Clearing falls back to the kind-based default instead of keeping a
    /// stale explicit value.
    pub fn set_reserve_override(&mut self, reserve_mb: Option<u64>) {
        self.reserve_mb = reserve_mb;
        let reserve = self.effective_reserve_mb();
        self.budget_mb =
            Self::share_budget_mb_with_reserve(self.ram_mb, self.available_mb, reserve);
        self.can_host_layers = self.budget_mb > 0;
    }

    /// Build from raw numbers, applying reserve + thread tiering.
    pub fn from_raw(id: &str, name: &str, kind: &str, ram_mb: u64, cores: u32, arch: &str) -> Self {
        Self::from_raw_available(id, name, kind, ram_mb, ram_mb, cores, arch)
    }

    pub fn from_raw_available(
        id: &str,
        name: &str,
        kind: &str,
        ram_mb: u64,
        available_mb: u64,
        cores: u32,
        arch: &str,
    ) -> Self {
        let budget = Self::share_budget_mb(kind, ram_mb, available_mb);
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            ram_mb,
            available_mb: available_mb.min(ram_mb),
            budget_mb: budget,
            cores,
            threads: Self::tier_threads(ram_mb, cores),
            can_host_layers: budget > 0,
            arch: arch.to_string(),
            reserve_mb: None,
            memory: MemoryTelemetry::default(),
        }
    }
}

impl Hardware {
    /// Probe this machine. Linux is the target; other platforms get sane fallbacks.
    pub fn probe_self(name: &str) -> Hardware {
        probe_self(name)
    }

    pub fn current_available_mb() -> Option<u64> {
        read_meminfo_mb().map(|(_, available)| available)
    }

    pub fn current_memory_telemetry() -> MemoryTelemetry {
        memory_telemetry()
    }
}

/// Probe this machine. Linux is the target; other platforms get sane fallbacks.
pub fn probe_self(name: &str) -> Hardware {
    let (ram_mb, available_mb) = read_meminfo_mb().unwrap_or_else(|| {
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        (
            system.total_memory() / (1024 * 1024),
            system.available_memory() / (1024 * 1024),
        )
    });
    let (ram_mb, available_mb, limit) = apply_process_limits(ram_mb, available_mb);
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
    let mut hardware = Hardware::from_raw_available(
        &id,
        name,
        kind,
        ram_mb,
        available_mb,
        cores,
        std::env::consts::ARCH,
    );
    hardware.memory = memory_telemetry();
    hardware.memory.limit_mb = limit;
    hardware
}

fn read_meminfo_mb() -> Option<(u64, u64)> {
    let txt = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo_mb(&txt)
}

fn parse_meminfo_mb(txt: &str) -> Option<(u64, u64)> {
    let value = |key: &str| -> Option<u64> {
        txt.lines()
            .find(|line| line.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    let total = value("MemTotal:")? / 1024;
    let available = value("MemAvailable:")
        .or_else(|| value("MemFree:"))
        .unwrap_or(0)
        / 1024;
    Some((total, available.min(total)))
}

fn apply_process_limits(total: u64, available: u64) -> (u64, u64, Option<u64>) {
    #[cfg(target_os = "linux")]
    {
        let cgroups = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
        let mut limits = Vec::new();
        for line in cgroups.lines() {
            let parts: Vec<_> = line.splitn(3, ':').collect();
            if parts.len() != 3 {
                continue;
            }
            let (root, max, used) = if parts[0] == "0" && parts[1].is_empty() {
                (
                    std::path::Path::new("/sys/fs/cgroup"),
                    "memory.max",
                    "memory.current",
                )
            } else if parts[1].split(',').any(|controller| controller == "memory") {
                (
                    std::path::Path::new("/sys/fs/cgroup/memory"),
                    "memory.limit_in_bytes",
                    "memory.usage_in_bytes",
                )
            } else {
                continue;
            };
            let relative = std::path::Path::new(parts[2].trim_start_matches('/'));
            if relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                continue;
            }
            let mut path = root.join(relative);
            for _ in 0..64 {
                if let (Some(maximum), Some(current)) =
                    (read_number(&path.join(max)), read_number(&path.join(used)))
                {
                    limits.push((maximum, current));
                }
                if path == root || !path.pop() || !path.starts_with(root) {
                    break;
                }
            }
            // Namespaced containers can expose only the mounted cgroup root.
            if let (Some(maximum), Some(current)) =
                (read_number(&root.join(max)), read_number(&root.join(used)))
            {
                limits.push((maximum, current));
            }
        }
        limit_memory(total, available, &limits)
    }
    #[cfg(not(target_os = "linux"))]
    (total, available, None)
}

fn limit_memory(
    mut total: u64,
    mut available: u64,
    limits: &[(u64, u64)],
) -> (u64, u64, Option<u64>) {
    let original = total;
    for &(maximum, used) in limits {
        total = total.min(maximum / (1024 * 1024));
        available = available.min(maximum.saturating_sub(used) / (1024 * 1024));
    }
    (
        total,
        available.min(total),
        (total < original).then_some(total),
    )
}

fn read_number(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub fn process_releasable_mb(pid: u32) -> u64 {
    // File-backed RSS may already be included in MemAvailable. Only resident
    // anonymous pages are credited to a replacement preview.
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return 0;
    };
    status
        .lines()
        .find(|line| line.starts_with("RssAnon:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        / 1024
}

fn memory_telemetry() -> MemoryTelemetry {
    let txt = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let value = |key: &str| {
        txt.lines()
            .find(|line| line.starts_with(key))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            / 1024
    };
    let swap_total = value("SwapTotal:");
    let mut telemetry = MemoryTelemetry {
        swap_total_mb: swap_total,
        swap_used_mb: swap_total.saturating_sub(value("SwapFree:")),
        ..Default::default()
    };
    if let Ok(swaps) = std::fs::read_to_string("/proc/swaps") {
        for line in swaps.lines().skip(1) {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() >= 4 && fields[0].contains("/zram") {
                telemetry.zram_logical_used_mb += fields[3].parse::<u64>().unwrap_or(0) / 1024;
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir("/sys/block") {
        for entry in entries
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("zram"))
        {
            if let Ok(stats) = std::fs::read_to_string(entry.path().join("mm_stat")) {
                telemetry.zram_physical_used_mb += stats
                    .split_whitespace()
                    .nth(2)
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0)
                    / (1024 * 1024);
            }
        }
    }
    if let Ok(pressure) = std::fs::read_to_string("/proc/pressure/memory") {
        for line in pressure.lines() {
            let average = line
                .split_whitespace()
                .find_map(|part| part.strip_prefix("avg10="))
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite());
            if line.starts_with("some ") {
                telemetry.pressure_some_avg10 = average;
            }
            if line.starts_with("full ") {
                telemetry.pressure_full_avg10 = average;
            }
        }
    }
    telemetry
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
    fn reserve_override_replaces_the_kind_default() {
        let mut hw =
            Hardware::from_raw_available("local", "laptop", "laptop", 16000, 12000, 8, "x86_64");
        let default_budget = hw.budget_mb;
        assert_eq!(default_budget, 10600);

        hw.apply_reserve_override(Some(4000));
        assert_eq!(hw.reserve_mb, Some(4000));
        assert_eq!(hw.budget_mb, 8000);
        assert!(hw.can_host_layers);

        // The recorded choice survives a budget recomputation, e.g. after a
        // released allocation frees 500 MiB more.
        let recomputed = Hardware::share_budget_mb_with_reserve(
            hw.ram_mb,
            hw.available_mb + 500,
            hw.effective_reserve_mb(),
        );
        assert_eq!(recomputed, 8500);

        // None keeps the kind-based default.
        let mut untouched =
            Hardware::from_raw_available("local", "laptop", "laptop", 16000, 12000, 8, "x86_64");
        untouched.apply_reserve_override(None);
        assert_eq!(untouched.reserve_mb, None);
        assert_eq!(untouched.budget_mb, default_budget);
    }

    #[test]
    fn missing_availability_never_promises_total_memory() {
        assert_eq!(parse_meminfo_mb("MemTotal: 8388608 kB\n"), Some((8192, 0)));
        assert_eq!(
            parse_meminfo_mb("MemTotal: 8388608 kB\nMemFree: 102400 kB\nSwapFree: 999999999 kB\n"),
            Some((8192, 100))
        );
    }

    #[test]
    fn process_and_parent_cgroup_limits_cap_total_and_remaining_capacity() {
        let mib = 1024 * 1024;
        assert_eq!(
            limit_memory(64000, 30000, &[(1024 * mib, 700 * mib)]),
            (1024, 324, Some(1024))
        );
        assert_eq!(
            limit_memory(
                64000,
                30000,
                &[(4096 * mib, 1000 * mib), (2048 * mib, 1900 * mib)]
            ),
            (2048, 148, Some(2048))
        );
        assert_eq!(
            limit_memory(4096, 100, &[(u64::MAX / 2, 0)]),
            (4096, 100, None)
        );
    }

    #[test]
    fn reserve_is_subtracted() {
        let h = Hardware::from_raw("id", "phone", "phone", 8000, 8, "arm64");
        assert_eq!(h.ram_mb, 8000);
        assert_eq!(h.available_mb, 8000);
        assert_eq!(h.budget_mb, 8000 - 900);
        assert!(h.can_host_layers);
    }

    #[test]
    fn available_memory_is_reported_separately_from_the_share_cap() {
        let h = Hardware::from_raw_available("id", "phone", "phone", 8000, 2300, 8, "arm64");
        assert_eq!(h.available_mb, 2300);
        assert_eq!(h.budget_mb, 1400);
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
