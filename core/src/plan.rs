//! Split planning — decide how a model is divided across devices.
//!
//! llama.cpp's RPC backend offloads *layers*. Each device holds a contiguous
//! span of layers, so the planner's job is to map layer storage cost onto
//! each device's memory budget. Weights are roughly uniform per layer for the
//! models people actually run, so proportional division by budget is close to
//! optimal and keeps the plan explainable in the UI.

use serde::{Deserialize, Serialize};

use crate::hardware::Hardware;

/// A model we can place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    /// Path to the GGUF (may be empty for catalog-only entries).
    pub path: String,
    /// Total parameter count, if known (e.g. 4.0e9).
    pub params_b: Option<f64>,
    /// File size on disk — our best estimate of weight memory.
    pub size_mb: u64,
    /// Transformer blocks. Needed to divide; estimated when unknown.
    pub layers: u32,
    /// KV cache per token per layer, bytes. Estimated when unknown.
    pub kv_bytes_per_token: u64,
}

impl ModelInfo {
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
        (self.kv_bytes_per_token * ctx as u64) / (1024 * 1024)
    }

    /// Total memory the model needs at runtime.
    pub fn runtime_mb(&self, ctx: u32) -> u64 {
        self.size_mb + self.kv_mb(ctx)
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
/// `weights_mb` is split across layers uniformly; KV cache is charged to the
/// device holding each layer. Devices are filled largest-budget-first so the
/// strongest machine takes the bulk instead of being left with leftovers.
pub fn plan(model: &ModelInfo, devices: &[(Hardware, Option<String>)], ctx: u32) -> SplitPlan {
    let total_layers = model.layers.max(1);

    // Only devices that can host layers, and are big enough to be useful.
    let mut pool: Vec<&(Hardware, Option<String>)> = devices
        .iter()
        .filter(|(h, _)| h.can_host_layers && h.budget_mb > 0)
        .collect();
    pool.sort_by(|a, b| b.0.budget_mb.cmp(&a.0.budget_mb));

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

    let weights_mb = model.size_mb;
    let kv_total_mb = model.kv_mb(ctx);
    let per_layer = (weights_mb as f64 / total_layers as f64)
        + (kv_total_mb as f64 / total_layers as f64);

    // Distribute layers by budget share, largest first.
    let total_budget: u64 = pool.iter().map(|(h, _)| h.budget_mb).sum();
    let mut assigned = 0u32;
    let mut out: Vec<SplitAssignment> = Vec::new();

    for (idx, (hw, rpc)) in pool.iter().enumerate() {
        let remaining_layers = total_layers.saturating_sub(assigned);
        if remaining_layers == 0 {
            out.push(SplitAssignment {
                device_id: hw.id.clone(),
                device_name: hw.name.clone(),
                kind: hw.kind.clone(),
                layers: 0,
                memory_mb: 0,
                budget_mb: hw.budget_mb,
                rpc: rpc.clone(),
            });
            continue;
        }

        // Count of layers this device can hold within budget.
        let affordable = if per_layer <= 0.0 {
            remaining_layers
        } else {
            (hw.budget_mb as f64 / per_layer).floor() as u32
        };

        // Share by budget for the last device, so we finish the model when the
        // cheap ones could not take their whole share.
        let mut take = if idx + 1 == pool.len() {
            remaining_layers
        } else {
            let share = (total_layers as f64 * (hw.budget_mb as f64 / total_budget as f64))
                .round() as u32;
            share.min(remaining_layers)
        };
        take = take.min(affordable).max(if remaining_layers > 0 { 0 } else { 0 });

        // Never leave the final device with zero if anything remains.
        if idx + 1 == pool.len() && take == 0 && remaining_layers > 0 {
            let need = (per_layer * remaining_layers as f64) as u64;
            if hw.budget_mb >= need {
                take = remaining_layers;
            }
        }

        let mem = (per_layer * take as f64).round() as u64;
        assigned += take;
        out.push(SplitAssignment {
            device_id: hw.id.clone(),
            device_name: hw.name.clone(),
            kind: hw.kind.clone(),
            layers: take,
            memory_mb: mem,
            budget_mb: hw.budget_mb,
            rpc: rpc.clone(),
        });
    }

    let placed: u64 = out.iter().map(|a| a.memory_mb).sum();
    let needed = model.runtime_mb(ctx);
    let shortfall = needed.saturating_sub(placed);
    let over = assigned < total_layers || shortfall > 0;

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
        format!("Fits on {} alone ({} layers) — no split needed.", a.device_name, a.layers)
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
        let m = model4b();
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
        let p = plan(&big, &[(laptop(8000), None), (phone(4000), Some("10.0.0.5:50052".into()))], 4096);
        let used: u32 = p.assignments.iter().map(|a| a.layers).sum();
        assert_eq!(used, 32, "every layer must be placed: {}", p.summary);
        assert!(p.is_distributed());
    }

    #[test]
    fn all_layers_placed_and_none_over_budget_marks_fit() {
        let m = model4b();
        let p = plan(&m, &[(laptop(16000), None), (phone(6000), Some("1.1.1.1:1".into()))], 4096);
        let used: u32 = p.assignments.iter().map(|a| a.layers).sum();
        assert_eq!(used, 36);
        assert!(p.fits(), "{}", p.summary);
        assert!(p.summary.contains("Fits"));
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
}
