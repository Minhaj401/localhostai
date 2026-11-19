//! Runtime-reported token counts and coordinator-observed latency. SSE chunks
//! are never treated as tokens. Missing measurements remain null in the API.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenerationMetrics {
    pub id: String,
    pub model: Option<String>,
    pub conversation_id: Option<String>,
    pub state: String,
    pub started_at: u64,
    pub elapsed_ms: f64,
    pub ttft_ms: Option<f64>,
    pub output_tokens: Option<u64>,
    pub tokens_per_second: Option<f64>,
    pub token_source: Option<String>,
    pub error: Option<String>,
}

pub struct StreamMeasurement {
    started: Instant,
    pub metrics: GenerationMetrics,
    pub text: String,
    pub finished: bool,
    buffer: Vec<u8>,
    data: Vec<u8>,
}
impl StreamMeasurement {
    pub fn new(model: Option<String>, conversation_id: Option<String>) -> Self {
        Self {
            started: Instant::now(),
            metrics: GenerationMetrics {
                id: crate::conversations::new_id()
                    .unwrap_or_else(|_| crate::conversations::now_ms().to_string()),
                model,
                conversation_id,
                state: "generating".into(),
                started_at: crate::conversations::now_ms(),
                ..Default::default()
            },
            text: String::new(),
            finished: false,
            buffer: Vec::new(),
            data: Vec::new(),
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > 2 * 1024 * 1024 {
            bail!("upstream SSE frame exceeds limit");
        }
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line = self.buffer.drain(..=end).collect::<Vec<_>>();
            self.line(line[..end].strip_suffix(b"\r").unwrap_or(&line[..end]))?;
        }
        self.tick();
        Ok(())
    }
    fn line(&mut self, line: &[u8]) -> Result<()> {
        if line.is_empty() {
            let data = std::mem::take(&mut self.data);
            self.event(&data)?;
        } else if let Some(data) = line.strip_prefix(b"data:") {
            if !self.data.is_empty() {
                self.data.push(b'\n');
            }
            self.data
                .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
            if self.data.len() > 2 * 1024 * 1024 {
                bail!("upstream SSE event exceeds limit");
            }
        }
        Ok(())
    }
    fn event(&mut self, bytes: &[u8]) -> Result<()> {
        let text = std::str::from_utf8(bytes)
            .context("invalid UTF-8 in upstream stream")?
            .trim();
        if text.is_empty() {
            return Ok(());
        }
        if text == "[DONE]" {
            self.finished = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(text).context("invalid upstream SSE JSON")?;
        self.observe(&value, true)
    }
    pub fn observe(&mut self, value: &Value, streaming: bool) -> Result<()> {
        if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
            bail!("upstream: {error}");
        }
        let choice = &value["choices"][0];
        let content = if streaming {
            &choice["delta"]["content"]
        } else {
            &choice["message"]["content"]
        };
        if let Some(text) = content.as_str().filter(|text| !text.is_empty()) {
            if streaming && self.metrics.ttft_ms.is_none() {
                self.metrics.ttft_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
            }
            if self.text.len() + text.len() > 512 * 1024 {
                bail!("reply exceeds 512 KiB limit");
            }
            self.text.push_str(text);
        }
        if !choice["finish_reason"].is_null() {
            self.finished = true;
        }
        let timings = &value["timings"];
        if let Some(count) = timings["predicted_n"].as_u64() {
            self.metrics.output_tokens = Some(count);
            self.metrics.token_source = Some("runtime timings".into());
            self.metrics.tokens_per_second = timings["predicted_per_second"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .or_else(|| {
                    timings["predicted_ms"]
                        .as_f64()
                        .filter(|ms| *ms > 0.0)
                        .map(|ms| count as f64 * 1000.0 / ms)
                });
        } else if let Some(count) = value["usage"]["completion_tokens"].as_u64() {
            self.metrics.output_tokens = Some(count);
            if self.metrics.token_source.is_none() {
                self.metrics.token_source = Some("upstream usage / wall time".into());
            }
            if self.metrics.tokens_per_second.is_none() {
                let seconds = self.started.elapsed().as_secs_f64();
                if seconds > 0.0 {
                    self.metrics.tokens_per_second = Some(count as f64 / seconds);
                }
            }
        }
        self.tick();
        Ok(())
    }
    pub fn end(&mut self) -> Result<()> {
        let remaining = std::mem::take(&mut self.buffer);
        if !remaining.is_empty() {
            self.line(remaining.strip_suffix(b"\r").unwrap_or(&remaining))?;
        }
        let data = std::mem::take(&mut self.data);
        self.event(&data)?;
        if !self.finished {
            bail!("reply stream ended before completion");
        }
        Ok(())
    }
    pub fn tick(&mut self) {
        self.metrics.elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
    }
}

#[derive(Clone, Default)]
pub struct Performance {
    inner: Arc<Mutex<Vec<GenerationMetrics>>>,
}
impl Performance {
    pub fn track(
        &self,
        model: Option<String>,
        conversation_id: Option<String>,
    ) -> TrackedGeneration {
        let sample = StreamMeasurement::new(model, conversation_id);
        self.record(&sample.metrics);
        TrackedGeneration {
            sample,
            performance: self.clone(),
        }
    }
    pub fn record(&self, metrics: &GenerationMetrics) {
        let mut entries = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = entries.iter_mut().find(|m| m.id == metrics.id) {
            *existing = metrics.clone();
        } else {
            entries.insert(0, metrics.clone());
            entries.truncate(24);
        }
    }
    pub fn snapshot(&self) -> Vec<GenerationMetrics> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub struct TrackedGeneration {
    pub sample: StreamMeasurement,
    performance: Performance,
}
impl TrackedGeneration {
    pub fn publish(&mut self) {
        self.sample.tick();
        self.performance.record(&self.sample.metrics);
    }
    pub fn finish(&mut self, error: Option<String>) {
        self.sample.metrics.state = if error.is_none() && self.sample.finished {
            "complete"
        } else {
            "interrupted"
        }
        .into();
        self.sample.metrics.error = error;
        self.publish();
    }
}
impl Drop for TrackedGeneration {
    fn drop(&mut self) {
        if self.sample.metrics.state == "generating" {
            self.finish(Some("Request stopped before completion".into()));
        }
    }
}

/// Read actual resident memory, separately from anonymous replacement credit.
pub fn process_rss_mb(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .map(|kb| kb / 1024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut system = sysinfo::System::new();
        let pid = sysinfo::Pid::from_u32(pid);
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        system.process(pid).map(|p| p.memory() / (1024 * 1024))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_utf8_events_use_reported_tokens_not_chunk_counts() {
        let mut measurement = StreamMeasurement::new(None, None);
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"你好🙂\"}}]}\r\n\r\ndata: {\"choices\":[{\"finish_reason\":\"stop\"}],\"timings\":{\"predicted_n\":7,\"predicted_ms\":500,\"predicted_per_second\":14}}\n\ndata: [DONE]\n\n";
        for byte in body.as_bytes() {
            measurement.feed(&[*byte]).unwrap();
        }
        measurement.end().unwrap();
        assert_eq!(measurement.text, "你好🙂");
        assert_eq!(measurement.metrics.output_tokens, Some(7));
        assert_eq!(measurement.metrics.tokens_per_second, Some(14.0));
        assert!(measurement.metrics.ttft_ms.is_some());
        let mut missing = StreamMeasurement::new(None, None);
        missing
            .feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n")
            .unwrap();
        assert!(missing.metrics.output_tokens.is_none());
        assert!(missing.end().is_err());
    }
}
