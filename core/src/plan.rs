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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(default)]
    pub layer_weights_mb: Vec<u64>,
    /// CPU-only input/output weights, tokenizer/metadata, and native buffers.
    #[serde(default)]
    pub coordinator_mb: u64,
    #[serde(default)]
    pub local_only: bool,
    #[serde(default)]
    pub max_context: Option<u32>,
}

impl ModelInfo {
    pub fn from_file(path: &std::path::Path, name: &str) -> anyhow::Result<Self> {
        let metadata = crate::gguf::inspect(path)?;
        let files = crate::gguf::model_files(path, &metadata)?;
        let mut layer_bytes = metadata.layer_bytes.clone();
        let mut coordinator_bytes = metadata.coordinator_bytes;
        let mut metadata_bytes = metadata.metadata_bytes;
        for file in files.iter().skip(1) {
            let shard = crate::gguf::inspect(file)?;
            for (total, bytes) in layer_bytes.iter_mut().zip(shard.layer_bytes) {
                *total = total.saturating_add(bytes);
            }
            coordinator_bytes = coordinator_bytes.saturating_add(shard.coordinator_bytes);
            metadata_bytes = metadata_bytes.saturating_add(shard.metadata_bytes);
        }
        let local_only = layer_bytes.contains(&0);
        let bytes = files.iter().try_fold(0u64, |total, file| {
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
            layer_weights_mb: if local_only {
                Vec::new()
            } else {
                layer_bytes
                    .into_iter()
                    .map(|bytes| bytes.div_ceil(1024 * 1024))
                    .collect()
            },
            coordinator_mb: (if local_only { 0 } else { coordinator_bytes })
                .saturating_add(metadata_bytes.saturating_mul(2))
                .div_ceil(1024 * 1024)
                .saturating_add(512),
            local_only,
            max_context: metadata.max_context,
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
        let weights = if self.layer_weights_mb.len() == self.layers as usize {
            self.layer_weights_mb
                .iter()
                .fold(0u64, |sum, value| sum.saturating_add(*value))
        } else {
            self.size_mb
        };
        weights
            .saturating_add(self.kv_mb(ctx))
            .saturating_add(self.coordinator_mb)
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
            (hardware.can_host_layers || (rpc.is_none() && model.coordinator_mb > 0))
                && (!model.local_only || rpc.is_none())
                && placement_budget(hardware, rpc.is_some()) > 0
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

    let coordinator_budget = pool
        .iter()
        .find(|(_, rpc)| rpc.is_none())
        .map(|(hw, _)| hw.budget_mb)
        .unwrap_or(0);
    if model.coordinator_mb > coordinator_budget || (model.max_context.is_some_and(|max| ctx > max))
    {
        return SplitPlan {
            model: model.name.clone(),
            total_layers,
            ctx,
            assignments: Vec::new(),
            over_budget: true,
            shortfall_mb: model.coordinator_mb.saturating_sub(coordinator_budget),
            summary: if model.max_context.is_some_and(|max| ctx > max) {
                format!(
                    "Context {ctx} exceeds this model's {}-token trained context.",
                    model.max_context.unwrap()
                )
            } else {
                format!("The laptop needs {} MiB for input/output weights and native buffers; only {coordinator_budget} MiB is safely available.", model.coordinator_mb)
            },
        };
    }
    let needed = model.runtime_mb(ctx).saturating_sub(model.coordinator_mb);
    let per_layer: Vec<u64> = if model.layer_weights_mb.len() == total_layers as usize {
        let kv = model.kv_mb(ctx).div_ceil(u64::from(total_layers));
        model
            .layer_weights_mb
            .iter()
            .map(|weights| weights.saturating_add(kv))
            .collect()
    } else {
        (0..total_layers)
            .map(|index| {
                ((needed as u128 * u128::from(index + 1)).div_ceil(u128::from(total_layers))
                    - (needed as u128 * u128::from(index)).div_ceil(u128::from(total_layers)))
                    as u64
            })
            .collect()
    };
    let memory_for = |layers: u32| -> u64 {
        (needed as u128 * layers as u128).div_ceil(total_layers as u128) as u64
    };

    // Keep as much work as possible on the coordinator. Remote devices only
    // receive the whole-layer remainder that cannot fit in earlier devices.
    let mut assigned = 0u32;
    let mut out: Vec<SplitAssignment> = Vec::new();

    for (hw, rpc) in &pool {
        let remaining_layers = total_layers.saturating_sub(assigned);
        let overhead = if rpc.is_none() && out.iter().all(|a| a.rpc.is_some()) {
            model.coordinator_mb
        } else {
            0
        };
        let budget = placement_budget(hw, rpc.is_some()).saturating_sub(overhead);
        let mut take = 0;
        let mut memory = 0u64;
        if hw.can_host_layers {
            while take < remaining_layers {
                let next = per_layer[(assigned + take) as usize];
                if memory.saturating_add(next) > budget {
                    break;
                }
                memory = memory.saturating_add(next);
                take += 1;
            }
        }
        assigned += take;
        out.push(SplitAssignment {
            device_id: hw.id.clone(),
            device_name: hw.name.clone(),
            kind: hw.kind.clone(),
            layers: take,
            memory_mb: if model.layer_weights_mb.is_empty() {
                memory_for(take)
            } else {
                memory
            }
            .saturating_add(overhead),
            budget_mb: hw.budget_mb,
            rpc: rpc.clone(),
            worker_session: None,
        });
    }

    let shortfall = if model.layer_weights_mb.is_empty() {
        memory_for(total_layers - assigned)
    } else {
        per_layer[assigned as usize..]
            .iter()
            .fold(0u64, |total, bytes| total.saturating_add(*bytes))
    };
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

    #[test]
    fn exact_layer_weights_and_coordinator_overhead_drive_placement() {
        let model = ModelInfo {
            name: "uneven".into(),
            size_mb: 600,
            layers: 3,
            layer_weights_mb: vec![100, 200, 300],
            coordinator_mb: 512,
            ..Default::default()
        };
        let laptop = Hardware {
            id: "laptop".into(),
            name: "laptop".into(),
            kind: "laptop".into(),
            budget_mb: 800,
            can_host_layers: true,
            ..Default::default()
        };
        let phone = Hardware {
            id: "phone".into(),
            name: "phone".into(),
            kind: "phone".into(),
            budget_mb: 856,
            can_host_layers: true,
            ..Default::default()
        };
        let plan = plan(
            &model,
            &[(laptop, None), (phone, Some("10.0.0.2:50052".into()))],
            1024,
        );
        assert!(plan.fits(), "{}", plan.summary);
        assert_eq!(plan.assignments[0].layers, 1);
        assert_eq!(plan.assignments[0].memory_mb, 612);
        assert_eq!(plan.assignments[1].layers, 2);
        assert_eq!(plan.assignments[1].memory_mb, 500);
    }
}
