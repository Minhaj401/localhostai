//! Exclusive, bounded A/B runs. Device inclusion settings are never changed.
use super::*;
use crate::cluster::ClusterState;
use crate::conversations::StoreError;
use crate::performance::{process_rss_mb, StreamMeasurement};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Benchmarks {
    report: Mutex<Value>,
    cancel: Mutex<Option<watch::Sender<bool>>>,
}
impl Benchmarks {
    pub fn snapshot(&self) -> Value {
        self.report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn cancel(&self) {
        if let Some(tx) = self
            .cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            tx.send_replace(true);
        }
    }
    fn phase(&self, phase: &str) {
        self.report.lock().unwrap_or_else(|e| e.into_inner())["phase"] = json!(phase);
    }
    fn push(&self, result: Value) {
        if let Some(results) =
            self.report.lock().unwrap_or_else(|e| e.into_inner())["results"].as_array_mut()
        {
            results.push(result);
        }
    }
}

pub(super) async fn dispatch(
    inner: &Arc<Inner>,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(u16, Value)> {
    match (method, path) {
        ("GET", "/benchmarks") => Ok((200, json!({"benchmark":inner.benchmarks.snapshot()}))),
        ("POST", "/benchmarks/cancel") => {
            inner.benchmarks.cancel();
            Ok((200, json!({"ok":true})))
        }
        ("POST", "/benchmarks/start") => {
            let req: Value = serde_json::from_slice(body)
                .map_err(|_| StoreError(400, "invalid benchmark request".into()))?;
            let model = req["model"].as_str().unwrap_or("");
            let ctx = request_context(&req, inner.cfg.ctx)
                .filter(|ctx| (128..=16384).contains(ctx))
                .ok_or_else(|| StoreError(400, "benchmark context must be 128–16384".into()))?;
            let tokens = req["max_tokens"].as_u64().unwrap_or(128);
            if !(16..=512).contains(&tokens) {
                return Err(
                    StoreError(400, "benchmark output must be 16–512 tokens".into()).into(),
                );
            }
            let prompt = req["prompt"].as_str().unwrap_or("Explain how a computer uses memory to run a language model. Use clear, short sentences.").to_string();
            if prompt.trim().is_empty() || prompt.len() > (ctx as usize / 2).min(2048) {
                return Err(StoreError(400, "benchmark prompt is empty or too long".into()).into());
            }
            let model = inner
                .models
                .lock()
                .await
                .iter()
                .find(|m| m.name == model)
                .cloned()
                .ok_or_else(|| StoreError(404, "select an installed GGUF model".into()))?;
            if model.local_only || model.layers < 2 {
                return Err(
                    StoreError(400, "this model cannot be split for comparison".into()).into(),
                );
            }
            let remote_layers = req["remote_layers"]
                .as_u64()
                .unwrap_or(u64::from((model.layers / 4).max(1)));
            if remote_layers == 0 || remote_layers >= u64::from(model.layers) {
                return Err(StoreError(
                    400,
                    "remote_layers must leave at least one transformer layer on the laptop".into(),
                )
                .into());
            }
            let guard = inner
                .inference_gate
                .clone()
                .try_write_owned()
                .map_err(|_| {
                    StoreError(
                        409,
                        "Wait for the current reply or model operation before benchmarking.".into(),
                    )
                })?;
            if inner.cluster.status().await.state == ClusterState::Starting {
                return Err(StoreError(409, "Wait for model loading to finish.".into()).into());
            }
            let peers = peer_states(inner).await;
            if !peers.iter().any(|(p, w)| p.enabled && w.compatible) {
                return Err(StoreError(
                    400,
                    "Enable a reachable phone worker before running a comparison.".into(),
                )
                .into());
            }
            let id = crate::conversations::new_id()?;
            let (cancel, mut cancelled) = watch::channel(false);
            *inner
                .benchmarks
                .cancel
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(cancel);
            *inner
                .benchmarks
                .report
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = json!({"id":id,"state":"running","phase":"validating",
                "model":model.name,"ctx":ctx,"max_tokens":tokens,"prompt":prompt,"remote_layers":remote_layers,
                "temperature":0,"seed":42,"cache_prompt":false,"warmup_tokens":8,"results":[],
                "note":"Each mode is reloaded and warmed up. The model is stopped after the comparison. Local mode is skipped if it does not fit."});
            let inner = inner.clone();
            let response = inner.benchmarks.snapshot();
            let mut shutdown = inner.shutdown.subscribe();
            tokio::spawn(async move {
                let _guard = guard;
                let run = run(&inner, model, ctx, tokens, remote_layers as u32, &prompt);
                let outcome = tokio::select! {
                    result = tokio::time::timeout(Duration::from_secs(900), run) => result.context("benchmark exceeded its 15-minute deadline").and_then(|r| r),
                    _ = cancelled.wait_for(|cancelled| *cancelled) => Err(anyhow::anyhow!("Benchmark cancelled")),
                    _ = shutdown.recv() => Err(anyhow::anyhow!("Coordinator stopped")),
                };
                inner.benchmarks.phase("stopping model");
                let cleanup = inner.cluster.stop().await;
                refresh_engine(&inner).await;
                let outcome = outcome.and(cleanup);
                let mut report = inner
                    .benchmarks
                    .report
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                report["state"] = json!(if outcome.is_ok() {
                    "complete"
                } else if *cancelled.borrow() {
                    "cancelled"
                } else {
                    "failed"
                });
                report["phase"] = json!("finished");
                report["error"] = outcome
                    .err()
                    .map(|e| json!(format!("{e:#}")))
                    .unwrap_or(Value::Null);
                report["finished_at"] = json!(crate::conversations::now_ms());
                *inner
                    .benchmarks
                    .cancel
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = None;
            });
            Ok((202, json!({"benchmark":response})))
        }
        _ => Ok((404, json!({"error":"not found"}))),
    }
}

async fn run(
    inner: &Arc<Inner>,
    original: ModelInfo,
    ctx: u32,
    tokens: u64,
    remote_layers: u32,
    prompt: &str,
) -> Result<()> {
    let model = ModelInfo::from_file(std::path::Path::new(&original.path), &original.name)?;
    inner.managed.validate_model(&model).await?;
    inner.cluster.stop().await?;
    refresh_engine(inner).await;
    for mode in ["laptop", "distributed"] {
        inner.benchmarks.phase(&format!("planning {mode}"));
        let mut devices = vec![(current_self_hardware(inner), None)];
        let mut sessions = std::collections::HashMap::new();
        if mode == "distributed" {
            for (peer, worker) in peer_states(inner).await {
                if peer.enabled && worker.compatible {
                    if let Some(report) = peer.worker {
                        sessions.insert(peer.hw.id.clone(), report.session);
                    }
                    devices.push((peer.hw, peer.rpc));
                }
            }
        }
        let mut plan = comparison_plan(
            &model,
            &devices,
            ctx,
            if mode == "distributed" {
                Some(remote_layers)
            } else {
                None
            },
        );
        for assignment in &mut plan.assignments {
            assignment.worker_session = sessions.get(&assignment.device_id).cloned();
        }
        if !plan.fits()
            || (mode == "distributed"
                && !plan
                    .assignments
                    .iter()
                    .any(|a| a.rpc.is_some() && a.layers > 0))
        {
            inner
                .benchmarks
                .push(json!({"mode":mode,"state":"skipped","reason":plan.summary,"plan":plan}));
            continue;
        }
        inner.benchmarks.phase(&format!("loading {mode}"));
        let loaded_at = Instant::now();
        let result = inner
            .cluster
            .start(&plan, std::path::Path::new(&model.path), ctx)
            .await?;
        if result.state != ClusterState::Running {
            bail!(
                "{}",
                result
                    .error
                    .unwrap_or_else(|| "benchmark model did not load".into())
            );
        }
        let load_ms = loaded_at.elapsed().as_secs_f64() * 1000.0;
        refresh_engine(inner).await;
        let url = format!(
            "{}/chat/completions",
            result.endpoint.context("loaded model has no endpoint")?
        );
        inner.benchmarks.phase(&format!("warming up {mode}"));
        measure(inner, &url, &model.name, prompt, 8, result.server_pid).await?;
        inner.benchmarks.phase(&format!("measuring {mode}"));
        let (metrics, peak) =
            measure(inner, &url, &model.name, prompt, tokens, result.server_pid).await?;
        inner.benchmarks.push(json!({"mode":mode,"state":"complete","metrics":metrics,"peak_engine_rss_mb":peak,"load_ms":load_ms,"plan":plan,"log_path":result.log_path}));
        inner.cluster.stop().await?;
        refresh_engine(inner).await;
    }
    Ok(())
}

fn comparison_plan(
    model: &ModelInfo,
    devices: &[(Hardware, Option<String>)],
    ctx: u32,
    remote_layers: Option<u32>,
) -> SplitPlan {
    let mut capped = devices.to_vec();
    if let Some(remote_layers) = remote_layers {
        // Deliberately leave a tail of layers for workers while retaining the
        // same physical memory limits. Never inflate a device's safe capacity.
        let local_layers = model.layers.saturating_sub(remote_layers) as usize;
        let kv = model.kv_mb(ctx).div_ceil(u64::from(model.layers));
        let weights = if model.layer_weights_mb.len() == model.layers as usize {
            model.layer_weights_mb[..local_layers]
                .iter()
                .fold(0u64, |sum, w| sum.saturating_add(w.saturating_add(kv)))
        } else {
            model
                .runtime_mb(ctx)
                .saturating_sub(model.coordinator_mb)
                .saturating_mul(local_layers as u64)
                / u64::from(model.layers)
        };
        capped[0].0.budget_mb = capped[0]
            .0
            .budget_mb
            .min(model.coordinator_mb.saturating_add(weights));
    }
    let mut plan = plan::plan(model, &capped, ctx);
    for assignment in &mut plan.assignments {
        if let Some((device, _)) = devices.iter().find(|(d, _)| d.id == assignment.device_id) {
            assignment.budget_mb = device.budget_mb;
        }
    }
    plan
}

async fn measure(
    inner: &Inner,
    url: &str,
    model: &str,
    prompt: &str,
    tokens: u64,
    pid: Option<u32>,
) -> Result<(crate::performance::GenerationMetrics, Option<u64>)> {
    use futures::StreamExt;
    let mut sample = StreamMeasurement::new(Some(model.into()), None);
    let response = inner.http.post(url).timeout(Duration::from_secs(240)).json(&json!({"model":model,"messages":[{"role":"user","content":prompt}],
        "stream":true,"max_tokens":tokens,"temperature":0,"seed":42,"cache_prompt":false,"timings_per_token":true,"stream_options":{"include_usage":true}})).send().await?.error_for_status()?;
    let mut stream = response.bytes_stream();
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut peak = pid.and_then(process_rss_mb);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Some(rss) = pid.and_then(process_rss_mb) { peak = Some(peak.unwrap_or(0).max(rss)); }
                sample.tick();
                inner.benchmarks.report.lock().unwrap_or_else(|e| e.into_inner())["current"] = json!(sample.metrics);
            }
            chunk = stream.next() => match chunk {
                Some(bytes) => sample.feed(&bytes?)?,
                None => break,
            }
        }
    }
    sample.end()?;
    sample.metrics.state = "complete".into();
    if let Some(rss) = pid.and_then(process_rss_mb) {
        peak = Some(peak.unwrap_or(0).max(rss));
    }
    Ok((sample.metrics, peak))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comparison_respects_capacity_and_forces_real_remote_layers() {
        let model = ModelInfo {
            name: "test".into(),
            layers: 4,
            size_mb: 400,
            layer_weights_mb: vec![100; 4],
            coordinator_mb: 100,
            ..Default::default()
        };
        let devices = vec![
            (
                Hardware::from_raw("laptop", "Laptop", "laptop", 4000, 4, "x86_64"),
                None,
            ),
            (
                Hardware::from_raw("phone", "Phone", "phone", 3000, 4, "arm64"),
                Some("127.0.0.1:50052".into()),
            ),
        ];
        let local = comparison_plan(&model, &devices[..1], 1024, None);
        assert_eq!(local.assignments[0].layers, 4);
        let split = comparison_plan(&model, &devices, 1024, Some(2));
        assert!(split.fits());
        assert_eq!(split.assignments[0].layers, 2);
        assert_eq!(split.assignments[1].layers, 2);
        let mut small = devices;
        small[0].0.budget_mb = 200;
        small[1].0.budget_mb = 300;
        assert!(!comparison_plan(&model, &small, 1024, Some(2)).fits());
    }
}
