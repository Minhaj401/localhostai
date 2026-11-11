//! Split planning — decide how a model is divided across devices.
//!
//! llama.cpp's RPC backend offloads *layers*. Each device holds a contiguous
//! span of layers, so the planner's job is to map layer storage cost onto
//! each device's memory budget. Weights are roughly uniform per layer for the
//! models people actually run. The coordinator is filled first so remote
//! devices receive only the layers that do not fit locally, reducing RPC
//! traffic and the peak allocation required on phones.

use serde::{Deserialize, Serialize};

use crate::hardware::Hardware;

const REMOTE_RUNTIME_HEADROOM_MB: u64 = 256;

/// A model we can place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    /// Path to the GGUF (may be empty for catalog-only entries).
    pub path: String,
    /// Parameter count in billions, if known (e.g. 4.0).
    pub params_b: Option<f64>,
    /// File size on disk — our best estimate of weight memory.
    pub size_mb: u64,
    /// Transformer blocks. Needed to divide; estimated when unknown.
    pub layers: u32,
    /// KV cache per token across all layers, bytes. Estimated when unknown.
    pub kv_bytes_per_token: u64,
}

impl ModelInfo {
    pub fn from_file(path: &std::path::Path, name: &str) -> anyhow::Result<Self> {
        let metadata = crate::gguf::inspect(path)?;
        let bytes =
            crate::gguf::model_files(path, &metadata)?
                .iter()
                .try_fold(0u64, |total, file| {
                    total
                        .checked_add(std::fs::metadata(file)?.len())
                        .ok_or_else(|| anyhow::anyhow!("GGUF size overflow"))
                })?;
        Ok(Self {
            name: name.into(),
            path: path.to_string_lossy().into_owned(),
            params_b: metadata.params_b,
            size_mb: bytes.div_ceil(1024 * 1024),
            layers: metadata.layers,
            kv_bytes_per_token: metadata.kv_bytes_per_token.unwrap_or(160_000),
        })
    }

    /// Guess layer count from parameter count when the GGUF does not tell us.
    /// Fine for planning; the real GGUFs report their own metadata later.
    pub fn estimate_layers(params_b: f64) -> u32 {
        // Empirically: ~0.5B→24, 3B→36, 4B→36, 7-8B→32, 13B→40, 70B→80.
        match params_b {
            p if p < 1.0 => 24,
            p if p < 5.0 => 36,
            p if p < 9.0 => 32,
            p if p < 15.0 => 40,
            p if p < 35.0 => 60,
            _ => 80,
        }
    }

    /// Rough KV cache size for a context, in MB.
    pub fn kv_mb(&self, ctx: u32) -> u64 {
        let bytes = self.kv_bytes_per_token as u128 * ctx as u128;
        bytes.div_ceil(1024 * 1024).min(u64::MAX as u128) as u64
    }

    /// Total memory the model needs at runtime.
    pub fn runtime_mb(&self, ctx: u32) -> u64 {
        self.size_mb.saturating_add(self.kv_mb(ctx))
    }
}

/// One device's assignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitAssignment {
    pub device_id: String,
    pub device_name: String,
    pub kind: String,
    /// Layers this device holds.
    pub layers: u32,
    /// Memory this assignment will occupy.
    pub memory_mb: u64,
    /// Its budget, for the UI to show headroom against.
    pub budget_mb: u64,
    /// Where its RPC server is, if it is a remote peer.
    pub rpc: Option<String>,
    /// A restarted process cannot reuse the previous process's model buffers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_session: Option<String>,
}

/// The full plan for a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitPlan {
    pub model: String,
    pub total_layers: u32,
    pub ctx: u32,
    pub assignments: Vec<SplitAssignment>,
    /// True when the plan needs more memory than the devices can give.
    pub over_budget: bool,
    /// How much is still missing when over budget.
    pub shortfall_mb: u64,
    /// Plain-English summary for the TUI.
    pub summary: String,
}

impl SplitPlan {
    pub fn fits(&self) -> bool {
        !self.over_budget
    }

    /// Whether the plan actually spans more than one device.
    pub fn is_distributed(&self) -> bool {
        self.assignments.iter().filter(|a| a.layers > 0).count() > 1
    }
}

/// Plan a split of `model` across `devices` at context size `ctx`.
///
/// Model memory is split across layers uniformly; KV cache is charged to the
/// device holding each layer. The coordinator is filled first, followed by
/// remote devices from largest to smallest budget.
pub fn plan(model: &ModelInfo, devices: &[(Hardware, Option<String>)], ctx: u32) -> SplitPlan {
    let total_layers = model.layers.max(1);
    let placement_budget = |hardware: &Hardware, remote: bool| {
        if remote {
            hardware
                .budget_mb
                .saturating_sub(REMOTE_RUNTIME_HEADROOM_MB)
        } else {
            hardware.budget_mb
        }
    };

    // Only devices that can host layers, and are big enough to be useful.
    let mut pool: Vec<&(Hardware, Option<String>)> = devices
        .iter()
        .filter(|(hardware, rpc)| {
            hardware.can_host_layers && placement_budget(hardware, rpc.is_some()) > 0
        })
        .collect();
    pool.sort_by_key(|(hardware, rpc)| (rpc.is_some(), std::cmp::Reverse(hardware.budget_mb)));

    if pool.is_empty() {
        return SplitPlan {
            model: model.name.clone(),
            total_layers,
            ctx,
            assignments: vec![],
            over_budget: true,
            shortfall_mb: model.runtime_mb(ctx),
            summary: "No device has memory to host this model.".into(),
        };
    }

    let needed = model.runtime_mb(ctx);
    let memory_for = |layers: u32| -> u64 {
        (needed as u128 * layers as u128).div_ceil(total_layers as u128) as u64
    };
    let capacity = |budget: u64| -> u32 {
        if needed == 0 {
            total_layers
        } else {
            (budget as u128 * total_layers as u128 / needed as u128).min(total_layers as u128)
                as u32
        }
    };

    // Keep as much work as possible on the coordinator. Remote devices only
    // receive the whole-layer remainder that cannot fit in earlier devices.
    let mut assigned = 0u32;
    let mut out: Vec<SplitAssignment> = Vec::new();

    for (hw, rpc) in &pool {
        let remaining_layers = total_layers.saturating_sub(assigned);
        let take = capacity(placement_budget(hw, rpc.is_some())).min(remaining_layers);
        assigned += take;
        out.push(SplitAssignment {
            device_id: hw.id.clone(),
            device_name: hw.name.clone(),
            kind: hw.kind.clone(),
            layers: take,
            memory_mb: memory_for(take),
            budget_mb: hw.budget_mb,
            rpc: rpc.clone(),
            worker_session: None,
        });
    }

    let shortfall = memory_for(total_layers - assigned);
    let over = assigned < total_layers;

    let covered = out.iter().filter(|a| a.layers > 0).count();
    let summary = if over {
        format!(
            "Does not fit: {} of {} layers placed across {} device(s); {:.0} MB short. \
             Lower the context or add a device.",
            assigned, total_layers, covered, shortfall as f64
        )
    } else if covered > 1 {
        format!(
            "Fits across {} devices: {}",
            covered,
            out.iter()
                .filter(|a| a.layers > 0)
                .map(|a| format!("{} {}/{}L", a.device_name, a.layers, total_layers))
                .collect::<Vec<_>>()
                .join(" + ")
        )
    } else if let Some(a) = out.iter().find(|a| a.layers > 0) {
        format!(
            "Fits on {} alone ({} layers) — no split needed.",
            a.device_name, a.layers
        )
    } else {
        "Does not fit.".to_string()
    };

    SplitPlan {
        model: model.name.clone(),
        total_layers,
        ctx,
        assignments: out,
        over_budget: over,
        shortfall_mb: shortfall,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model4b() -> ModelInfo {
        ModelInfo {
            name: "qwen3:4b".into(),
            path: "/x.gguf".into(),
            params_b: Some(4.0),
            size_mb: 2400,
            layers: 36,
            kv_bytes_per_token: 160_000,
        }
    }

    fn phone(ram: u64) -> Hardware {
        Hardware::from_raw("phone-1", "Pixel", "phone", ram, 8, "arm64")
    }
    fn laptop(ram: u64) -> Hardware {
        Hardware::from_raw("lap-1", "Laptop", "laptop", ram, 8, "x86_64")
    }

    #[test]
    fn refuses_when_no_device_qualifies() {
        let m = model4b();
        let tiny = Hardware::from_raw("t", "Tiny", "phone", 700, 4, "arm64");
        let p = plan(&m, &[(tiny, None)], 4096);
        assert!(p.over_budget);
        assert_eq!(p.assignments.len(), 0);
    }

    #[test]
    fn single_big_device_takes_everything() {
        let m = model4b();
        let p = plan(&m, &[(laptop(32000), None)], 4096);
        assert!(p.fits());
        assert!(!p.is_distributed());
        let total: u32 = p.assignments.iter().map(|a| a.layers).sum();
        assert_eq!(total, 36);
    }

    #[test]
    fn splits_across_two_devices_when_neither_fits_alone() {
        // 8GB laptop can hold ~6.6GB, 6GB phone ~5.1GB; alone each is fine,
        // so make the model big: 8B/8GB across a small-ish pair.
        let big = ModelInfo {
            name: "llama3:8b".into(),
            path: "/y.gguf".into(),
            params_b: Some(8.0),
            size_mb: 8200,
            layers: 32,
            kv_bytes_per_token: 200_000,
        };
        let p = plan(
            &big,
            &[
                (laptop(8000), None),
                (phone(4000), Some("10.0.0.5:50052".into())),
            ],
            4096,
        );
        let used: u32 = p.assignments.iter().map(|a| a.layers).sum();
        assert_eq!(used, 32, "every layer must be placed: {}", p.summary);
        assert!(p.is_distributed());
    }

    #[test]
    fn all_layers_placed_and_none_over_budget_marks_fit() {
        let m = model4b();
        let p = plan(
            &m,
            &[
                (laptop(16000), None),
                (phone(6000), Some("1.1.1.1:1".into())),
            ],
            4096,
        );
        let used: u32 = p.assignments.iter().map(|a| a.layers).sum();
        assert_eq!(used, 36);
        assert!(p.fits(), "{}", p.summary);
        assert!(p.summary.contains("Fits"));
    }

    #[test]
    fn fills_coordinator_before_spilling_minimum_layers_to_phone() {
        let model = ModelInfo {
            size_mb: 1179,
            layers: 28,
            kv_bytes_per_token: 0,
            ..model4b()
        };
        let coordinator = Hardware {
            id: "laptop".into(),
            name: "Laptop".into(),
            kind: "laptop".into(),
            budget_mb: 1082,
            can_host_layers: true,
            ..Default::default()
        };
        let phone = Hardware {
            id: "phone".into(),
            name: "Phone".into(),
            kind: "phone".into(),
            budget_mb: 786,
            can_host_layers: true,
            ..Default::default()
        };

        let p = plan(
            &model,
            &[(coordinator, None), (phone, Some("10.0.0.5:50052".into()))],
            4096,
        );

        assert!(p.fits(), "{}", p.summary);
        assert_eq!(p.assignments[0].device_id, "laptop");
        assert_eq!(p.assignments[0].layers, 25);
        assert_eq!(p.assignments[1].device_id, "phone");
        assert_eq!(p.assignments[1].layers, 3);
        assert!(p.assignments.iter().all(|a| a.memory_mb <= a.budget_mb));
    }

    #[test]
    fn leaves_remote_devices_idle_when_the_coordinator_fits() {
        let model = ModelInfo {
            size_mb: 1000,
            layers: 10,
            kv_bytes_per_token: 0,
            ..model4b()
        };
        let coordinator = Hardware {
            id: "laptop".into(),
            budget_mb: 1000,
            can_host_layers: true,
            ..Default::default()
        };
        let phone = Hardware {
            id: "phone".into(),
            budget_mb: 8000,
            can_host_layers: true,
            ..Default::default()
        };

        let p = plan(
            &model,
            &[(phone, Some("10.0.0.5:50052".into())), (coordinator, None)],
            4096,
        );

        assert!(p.fits(), "{}", p.summary);
        assert_eq!(p.assignments[0].device_id, "laptop");
        assert_eq!(p.assignments[0].layers, 10);
        assert_eq!(p.assignments[1].layers, 0);
        assert!(!p.is_distributed());
    }

    #[test]
    fn keeps_native_runtime_headroom_out_of_remote_model_allocations() {
        let model = ModelInfo {
            size_mb: 400,
            layers: 4,
            kv_bytes_per_token: 0,
            ..model4b()
        };
        let phone = Hardware {
            id: "phone".into(),
            budget_mb: 600,
            can_host_layers: true,
            ..Default::default()
        };

        let p = plan(&model, &[(phone, Some("10.0.0.5:50052".into()))], 4096);

        assert!(!p.fits());
        assert_eq!(p.assignments[0].layers, 3);
        assert_eq!(p.assignments[0].memory_mb, 300);
        assert_eq!(p.shortfall_mb, 100);
    }

    #[test]
    fn over_budget_reports_shortfall() {
        let big = ModelInfo {
            name: "70b".into(),
            path: "/z.gguf".into(),
            params_b: Some(70.0),
            size_mb: 40_000,
            layers: 80,
            kv_bytes_per_token: 500_000,
        };
        let p = plan(&big, &[(laptop(8000), None)], 4096);
        assert!(p.over_budget);
        assert!(p.shortfall_mb > 0);
    }

    #[test]
    fn layer_estimates_are_sane() {
        assert_eq!(ModelInfo::estimate_layers(0.5), 24);
        assert_eq!(ModelInfo::estimate_layers(4.0), 36);
        assert_eq!(ModelInfo::estimate_layers(70.0), 80);
    }

    #[test]
    fn redistributes_layers_that_small_peers_cannot_hold() {
        let model = ModelInfo {
            size_mb: 1000,
            layers: 3,
            kv_bytes_per_token: 0,
            ..model4b()
        };
        let devices: Vec<_> = [1000, 1000, 100]
            .into_iter()
            .enumerate()
            .map(|(i, budget)| {
                (
                    Hardware {
                        id: i.to_string(),
                        budget_mb: budget,
                        can_host_layers: true,
                        ..Default::default()
                    },
                    None,
                )
            })
            .collect();
        let p = plan(&model, &devices, 4096);
        assert!(p.fits(), "{}", p.summary);
        assert_eq!(p.assignments.iter().map(|a| a.layers).sum::<u32>(), 3);
        assert!(p.assignments.iter().all(|a| a.memory_mb <= a.budget_mb));
    }

    #[test]
    fn fractional_megabytes_do_not_create_a_false_shortfall() {
        let model = ModelInfo {
            size_mb: 1000,
            layers: 3,
            kv_bytes_per_token: 0,
            ..model4b()
        };
        let hw = Hardware {
            budget_mb: 500,
            can_host_layers: true,
            ..Default::default()
        };
        let p = plan(
            &model,
            &[(hw.clone(), None), (hw.clone(), None), (hw, None)],
            4096,
        );
        assert!(p.fits(), "{}", p.summary);
        assert_eq!(p.shortfall_mb, 0);
        assert!(p.assignments.iter().all(|a| a.memory_mb == 334));
    }

    #[test]
    fn memory_estimates_do_not_overflow_on_large_metadata() {
        let model = ModelInfo {
            size_mb: u64::MAX,
            kv_bytes_per_token: u64::MAX,
            ..model4b()
        };
        assert_eq!(model.runtime_mb(u32::MAX), u64::MAX);
        assert!(!plan(&model, &[(laptop(8000), None)], u32::MAX).fits());
    }
}
