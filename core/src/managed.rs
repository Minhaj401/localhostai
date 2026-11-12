//! Self-contained llama.cpp runtime and GGUF model installation.
//!
//! The TUI drives this manager through the node API. Downloads are pinned and
//! checksummed, partial model downloads resume, and finished files are moved
//! into place atomically. No Ollama process or manual setup is required.

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{watch, Mutex, Semaphore};

use crate::cluster::Toolchain;
use crate::model_hub::{DownloadFile, HubResults, ModelHub, ModelSource, RemoteModel};
use crate::plan::ModelInfo;

const RUNTIME_VERSION: &str = "b12110";
const RUNTIME_DIR: &str = "llama-b12110";

#[derive(Debug, Clone, Serialize)]
pub struct CatalogModel {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub params_b: f64,
    pub layers: u32,
    pub kv_bytes_per_token: u64,
    pub license: &'static str,
    pub license_url: &'static str,
    pub recommended: bool,
}

impl CatalogModel {
    fn path(&self) -> PathBuf {
        crate::config::models_dir().join(self.filename)
    }

    fn installed(&self) -> bool {
        self.path()
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() == self.bytes)
    }

    pub fn model_info(&self) -> ModelInfo {
        ModelInfo {
            name: self.id.to_string(),
            path: self.path().to_string_lossy().into_owned(),
            params_b: Some(self.params_b),
            size_mb: self.bytes.div_ceil(1024 * 1024),
            layers: self.layers,
            kv_bytes_per_token: self.kv_bytes_per_token,
            coordinator_mb: 512,
            ..Default::default()
        }
    }
}

pub static CATALOG: &[CatalogModel] = &[
    CatalogModel {
        id: "qwen2.5-0.5b-instruct-q4_k_m",
        display_name: "Qwen 2.5 0.5B Instruct",
        description: "Fast first model for testing and low-memory laptops",
        filename: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/9217f5db79a29953eb74d5343926648285ec7e67/qwen2.5-0.5b-instruct-q4_k_m.gguf?download=true",
        sha256: "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db",
        bytes: 491_400_032,
        params_b: 0.5,
        layers: 24,
        kv_bytes_per_token: 24_576,
        license: "Apache-2.0",
        license_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/blob/9217f5db79a29953eb74d5343926648285ec7e67/LICENSE",
        recommended: false,
    },
    CatalogModel {
        id: "qwen2.5-1.5b-instruct-q4_k_m",
        display_name: "Qwen 2.5 1.5B Instruct",
        description: "Better quality while remaining practical on CPU",
        filename: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/91cad51170dc346986eccefdc2dd33a9da36ead9/qwen2.5-1.5b-instruct-q4_k_m.gguf?download=true",
        sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
        bytes: 1_117_320_736,
        params_b: 1.5,
        layers: 28,
        kv_bytes_per_token: 28_672,
        license: "Apache-2.0",
        license_url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/blob/91cad51170dc346986eccefdc2dd33a9da36ead9/LICENSE",
        recommended: true,
    },
    CatalogModel {
        id: "qwen2.5-3b-instruct-q4_k_m",
        display_name: "Qwen 2.5 3B Instruct",
        description: "Higher-quality model for a laptop and phone pool",
        filename: "qwen2.5-3b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/7dabda4d13d513e3e842b20f0d435c732f172cbe/qwen2.5-3b-instruct-q4_k_m.gguf?download=true",
        sha256: "626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d",
        bytes: 2_104_932_768,
        params_b: 3.0,
        layers: 36,
        kv_bytes_per_token: 36_864,
        license: "Qwen Research",
        license_url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/blob/7dabda4d13d513e3e842b20f0d435c732f172cbe/LICENSE",
        recommended: false,
    },
    CatalogModel {
        id: "qwen2.5-7b-instruct-q3_k_m",
        display_name: "Qwen 2.5 7B Instruct",
        description: "Stronger general model; Q3 keeps the download and RAM demand lower",
        filename: "qwen2.5-7b-instruct-q3_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/bb5d59e06d9551d752d08b292a50eb208b07ab1f/qwen2.5-7b-instruct-q3_k_m.gguf?download=true",
        sha256: "a96b16179dc6cc9afdf0cf7a96a80c199cbd00b9be207c3465be21cb721cca5e",
        bytes: 3_808_391_072,
        params_b: 7.6,
        layers: 28,
        kv_bytes_per_token: 57_344,
        license: "Apache-2.0",
        license_url: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/blob/bb5d59e06d9551d752d08b292a50eb208b07ab1f/LICENSE",
        recommended: false,
    },
    CatalogModel {
        id: "qwen2.5-coder-7b-instruct-q4_k_m",
        display_name: "Qwen 2.5 Coder 7B Instruct",
        description: "Larger coding assistant for a high-memory laptop or device pool",
        filename: "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/13fb94bfda8c8cf22497dc57b78f391a9acb426a/qwen2.5-coder-7b-instruct-q4_k_m.gguf?download=true",
        sha256: "509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c",
        bytes: 4_683_073_536,
        params_b: 7.6,
        layers: 28,
        kv_bytes_per_token: 57_344,
        license: "Apache-2.0",
        license_url: "https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/blob/13fb94bfda8c8cf22497dc57b78f391a9acb426a/LICENSE",
        recommended: false,
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct ManagedStatus {
    pub phase: String,
    pub model: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub message: Option<String>,
    pub job_id: Option<String>,
}

impl Default for ManagedStatus {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            model: None,
            downloaded_bytes: 0,
            total_bytes: 0,
            message: None,
            job_id: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CatalogEntry<'a> {
    #[serde(flatten)]
    pub model: &'a CatalogModel,
    pub installed: bool,
}

#[derive(Debug, Serialize)]
pub struct ManagedView<'a> {
    pub runtime_ready: bool,
    pub runtime_version: &'static str,
    pub status: ManagedStatus,
    pub catalog: Vec<CatalogEntry<'a>>,
    pub custom: Vec<CustomEntry>,
    pub jobs: Vec<InstallJobView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallJobView {
    pub id: String,
    pub state: String,
    pub status: ManagedStatus,
    pub result: Option<ModelInfo>,
    pub error: Option<String>,
}

struct InstallJob {
    view: watch::Sender<InstallJobView>,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone)]
pub enum InstallTask {
    Model(String),
    Download(ModelSource),
}

#[derive(Debug, Serialize)]
pub struct CustomEntry {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub bytes: u64,
    pub license: String,
    pub installed: bool,
    pub custom: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CustomRecord {
    model: RemoteModel,
    complete: bool,
}

impl CustomRecord {
    fn path(&self, root: &Path) -> PathBuf {
        root.join(&self.model.id)
            .join(&self.model.files[0].filename)
    }

    fn installed(&self, root: &Path) -> bool {
        self.complete
            && self.model.files.iter().all(|file| {
                root.join(&self.model.id)
                    .join(&file.filename)
                    .metadata()
                    .is_ok_and(|meta| meta.is_file() && meta.len() == file.bytes)
            })
    }

    fn save(&self, root: &Path) -> Result<()> {
        self.model.validate()?;
        let dir = root.join(&self.model.id);
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let tmp = dir.join("source.json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(tmp, dir.join("source.json"))?;
        Ok(())
    }
}

struct Inner {
    status: Mutex<ManagedStatus>,
    operation: Mutex<()>,
    downloaded_bytes: AtomicU64,
    verified_models: Mutex<HashMap<PathBuf, (u64, std::time::SystemTime)>>,
    custom_dir: PathBuf,
    network: crate::network::NetworkPolicy,
    jobs: Mutex<Vec<Arc<InstallJob>>>,
    active_job: Mutex<Option<String>>,
    job_slots: Arc<Semaphore>,
    next_job: AtomicU64,
}

#[derive(Clone)]
pub struct Managed {
    inner: Arc<Inner>,
}

impl Managed {
    pub fn new() -> Result<Self> {
        Self::with_custom_dir(crate::config::models_dir().join("custom"))
    }

    pub fn with_model_hosts(hosts: Vec<String>) -> Result<Self> {
        Self::with_options(
            crate::config::models_dir().join("custom"),
            crate::network::NetworkPolicy::new(hosts),
        )
    }

    fn with_custom_dir(custom_dir: PathBuf) -> Result<Self> {
        #[cfg(test)]
        let network = crate::network::NetworkPolicy::local_test();
        #[cfg(not(test))]
        let network = crate::network::NetworkPolicy::default();
        Self::with_options(custom_dir, network)
    }

    fn with_options(custom_dir: PathBuf, network: crate::network::NetworkPolicy) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Inner {
                status: Mutex::new(ManagedStatus::default()),
                operation: Mutex::new(()),
                downloaded_bytes: AtomicU64::new(0),
                verified_models: Mutex::new(HashMap::new()),
                custom_dir,
                network,
                jobs: Mutex::new(Vec::new()),
                active_job: Mutex::new(None),
                job_slots: Arc::new(Semaphore::new(3)),
                next_job: AtomicU64::new(1),
            }),
        })
    }

    pub fn catalog_model(id: &str) -> Option<&'static CatalogModel> {
        CATALOG.iter().find(|m| m.id == id)
    }

    pub async fn view(&self) -> ManagedView<'static> {
        let mut status = self.inner.status.lock().await.clone();
        status.downloaded_bytes = self.inner.downloaded_bytes.load(Ordering::Relaxed);
        ManagedView {
            runtime_ready: Toolchain::managed().is_some() || Toolchain::explicit().is_some(),
            runtime_version: RUNTIME_VERSION,
            status,
            catalog: CATALOG
                .iter()
                .map(|model| CatalogEntry {
                    model,
                    installed: model.installed(),
                })
                .collect(),
            custom: self
                .custom_records()
                .into_iter()
                .map(|record| CustomEntry {
                    installed: record.installed(&self.inner.custom_dir),
                    id: record.model.id,
                    display_name: record.model.display_name,
                    description: record.model.description,
                    bytes: record.model.bytes,
                    license: record.model.license,
                    custom: true,
                })
                .collect(),
            jobs: self.jobs().await,
        }
    }

    /// Installation jobs survive a control connection closing. Work is bounded
    /// to one installation plus two queued jobs and a two-hour whole-job clock.
    pub async fn enqueue<F, Fut>(&self, task: InstallTask, ready: F) -> Result<InstallJobView>
    where
        F: FnOnce(ModelInfo) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let slot = self
            .inner
            .job_slots
            .clone()
            .try_acquire_owned()
            .context("the installation queue is full; retry after a current job finishes")?;
        let id = format!(
            "install-{}-{}",
            std::process::id(),
            self.inner.next_job.fetch_add(1, Ordering::Relaxed)
        );
        let view = InstallJobView {
            id: id.clone(),
            state: "queued".into(),
            status: ManagedStatus {
                phase: "queued".into(),
                job_id: Some(id.clone()),
                model: match &task {
                    InstallTask::Model(id) => Some(id.clone()),
                    _ => None,
                },
                ..Default::default()
            },
            result: None,
            error: None,
        };
        let (tx, _) = watch::channel(view.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Arc::new(InstallJob { view: tx, cancel });
        {
            let mut jobs = self.inner.jobs.lock().await;
            while jobs.len() >= 32 {
                if let Some(index) = jobs.iter().position(|job| {
                    matches!(
                        job.view.borrow().state.as_str(),
                        "succeeded" | "failed" | "cancelled"
                    )
                }) {
                    jobs.remove(index);
                } else {
                    break;
                }
            }
            jobs.push(job.clone());
        }
        let managed = self.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let work_cancel = job.cancel.clone();
            let managed_work = managed.clone();
            let job_work = job.clone();
            let id_work = id.clone();
            let mut work = tokio::spawn(async move {
                let _operation = managed_work.inner.operation.lock().await;
                ensure_not_cancelled(&work_cancel)?;
                *managed_work.inner.active_job.lock().await = Some(id_work.clone());
                job_work.view.send_modify(|view| {
                    view.state = "running".into();
                    view.status.phase = "preparing".into();
                });
                managed_work
                    .set_progress(
                        "preparing",
                        None,
                        0,
                        0,
                        Some("Preparing installation".into()),
                    )
                    .await;
                let model = match task {
                    InstallTask::Model(model) => {
                        managed_work.install_locked(&model, &work_cancel).await?
                    }
                    InstallTask::Download(source) => {
                        managed_work.download_locked(&source, &work_cancel).await?
                    }
                };
                ensure_not_cancelled(&work_cancel)?;
                ready(model.clone()).await;
                Ok::<_, anyhow::Error>(model)
            });
            let outcome = tokio::select! {
                result = &mut work => result.context("installation task failed unexpectedly").and_then(|result| result),
                _ = tokio::time::sleep(std::time::Duration::from_secs(7200)) => {
                    job.cancel.store(true, Ordering::Relaxed);
                    let _ = work.await;
                    Err(anyhow::anyhow!("installation exceeded its two-hour operation deadline; retry to resume"))
                }
            };
            let active = managed.inner.active_job.lock().await.clone();
            let mut status = if active.as_deref() == Some(&id) {
                managed.inner.status.lock().await.clone()
            } else {
                job.view.borrow().status.clone()
            };
            status.downloaded_bytes = if active.as_deref() == Some(&id) {
                managed.inner.downloaded_bytes.load(Ordering::Relaxed)
            } else {
                status.downloaded_bytes
            };
            let cancelled = job.cancel.load(Ordering::Relaxed);
            let state = if outcome.is_ok() {
                "succeeded"
            } else if cancelled {
                "cancelled"
            } else {
                "failed"
            };
            if outcome.is_err() {
                status.phase = state.into();
                status.message = outcome.as_ref().err().map(|error| format!("{error:#}"));
            }
            job.view.send_replace(InstallJobView {
                id: id.clone(),
                state: state.into(),
                status: status.clone(),
                result: outcome.as_ref().ok().cloned(),
                error: outcome.err().map(|error| format!("{error:#}")),
            });
            let mut active = managed.inner.active_job.lock().await;
            if active.as_deref() == Some(&id) {
                *managed.inner.status.lock().await = status;
                *active = None;
            }
        });
        Ok(view)
    }

    pub async fn jobs(&self) -> Vec<InstallJobView> {
        let jobs = self.inner.jobs.lock().await.clone();
        let active = self.inner.active_job.lock().await.clone();
        let mut current = self.inner.status.lock().await.clone();
        current.downloaded_bytes = self.inner.downloaded_bytes.load(Ordering::Relaxed);
        jobs.into_iter()
            .map(|job| {
                let mut view = job.view.borrow().clone();
                if view.state == "running" && active.as_deref() == Some(&view.id) {
                    view.status = current.clone();
                }
                view
            })
            .collect()
    }

    pub async fn job(&self, id: &str) -> Option<InstallJobView> {
        self.jobs().await.into_iter().find(|job| job.id == id)
    }

    pub async fn cancel(&self, id: &str) -> bool {
        let jobs = self.inner.jobs.lock().await;
        let Some(job) = jobs.iter().find(|job| job.view.borrow().id == id) else {
            return false;
        };
        if matches!(job.view.borrow().state.as_str(), "queued" | "running") {
            job.cancel.store(true, Ordering::Relaxed);
        }
        true
    }

    pub async fn wait_job(&self, id: &str) -> Result<ModelInfo> {
        let job = self
            .inner
            .jobs
            .lock()
            .await
            .iter()
            .find(|job| job.view.borrow().id == id)
            .cloned()
            .context("unknown installation job")?;
        let mut changed = job.view.subscribe();
        loop {
            let view = changed.borrow().clone();
            if view.state == "succeeded" {
                return view
                    .result
                    .context("installation completed without a model");
            }
            if matches!(view.state.as_str(), "failed" | "cancelled") {
                bail!(
                    "{}",
                    view.error.unwrap_or_else(|| "installation failed".into())
                );
            }
            changed
                .changed()
                .await
                .context("installation session ended; retry to resume")?;
        }
    }

    pub async fn browse(&self, query: &str) -> Result<HubResults> {
        ModelHub::with_policy(self.inner.network.clone())
            .browse(query)
            .await
    }

    pub fn has_custom_model(&self, id: &str) -> bool {
        self.custom_record(id).is_some()
    }

    pub async fn remove_model(&self, id: &str) -> Result<()> {
        let _operation = self.inner.operation.lock().await;
        if let Some(model) = Self::catalog_model(id) {
            for path in [model.path(), partial_path(&model.path())] {
                match tokio::fs::remove_file(&path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).with_context(|| format!("remove {}", path.display()))
                    }
                }
            }
            self.inner
                .verified_models
                .lock()
                .await
                .remove(&model.path());
            return Ok(());
        }
        let record = self.custom_record(id).context("unknown managed model")?;
        let directory = self.inner.custom_dir.join(&record.model.id);
        match tokio::fs::remove_dir_all(&directory).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("remove {}", directory.display()))
            }
        }
        self.inner
            .verified_models
            .lock()
            .await
            .retain(|path, _| !path.starts_with(&directory));
        Ok(())
    }

    pub async fn reset_runtime(&self) -> Result<()> {
        let _operation = self.inner.operation.lock().await;
        let _lock = install_lock(&crate::config::home())?;
        for path in [
            crate::config::bin_dir().join(RUNTIME_DIR),
            crate::config::bin_dir().join(format!("{RUNTIME_DIR}.tar.gz")),
        ] {
            if path.is_dir() {
                let _ = tokio::fs::remove_dir_all(path).await;
            } else {
                let _ = tokio::fs::remove_file(path).await;
            }
        }
        self.set_progress(
            "idle",
            None,
            0,
            0,
            Some(
                "Managed runtime removed; the next model load installs a fresh pinned copy".into(),
            ),
        )
        .await;
        Ok(())
    }

    fn custom_record(&self, id: &str) -> Option<CustomRecord> {
        if !crate::model_hub::valid_custom_id(id) {
            return None;
        }
        let text = std::fs::read(self.inner.custom_dir.join(id).join("source.json")).ok()?;
        let record: CustomRecord = serde_json::from_slice(&text).ok()?;
        (record.model.id == id && record.model.validate().is_ok()).then_some(record)
    }

    fn custom_records(&self) -> Vec<CustomRecord> {
        let Ok(entries) = std::fs::read_dir(&self.inner.custom_dir) else {
            return Vec::new();
        };
        let mut records: Vec<_> = entries
            .flatten()
            .filter_map(|entry| self.custom_record(&entry.file_name().to_string_lossy()))
            .collect();
        records.sort_by(|a, b| a.model.display_name.cmp(&b.model.display_name));
        records
    }

    pub fn installed_custom_models(&self) -> Vec<ModelInfo> {
        self.custom_records()
            .into_iter()
            .filter(|record| record.installed(&self.inner.custom_dir))
            .filter_map(|record| {
                ModelInfo::from_file(&record.path(&self.inner.custom_dir), &record.model.id).ok()
            })
            .collect()
    }

    pub async fn download(&self, source: &ModelSource) -> Result<ModelInfo> {
        let _operation = self.inner.operation.lock().await;
        self.download_locked(source, &Arc::new(AtomicBool::new(false)))
            .await
    }

    async fn download_locked(
        &self,
        source: &ModelSource,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ModelInfo> {
        ensure_not_cancelled(cancel)?;
        self.set_progress(
            "resolving_model",
            None,
            0,
            0,
            Some("Resolving the selected GGUF revision".into()),
        )
        .await;
        let model = ModelHub::with_policy(self.inner.network.clone())
            .resolve(source)
            .await?;
        let record = self.custom_record(&model.id).unwrap_or(CustomRecord {
            model,
            complete: false,
        });
        self.install_custom(record, cancel).await
    }

    async fn install_custom(
        &self,
        mut record: CustomRecord,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ModelInfo> {
        let result = async {
            let lock = install_lock(&crate::config::home())?;
            record.save(&self.inner.custom_dir)?;
            self.ensure_runtime(lock.clone(), cancel).await?;
            self.ensure_custom_model(&mut record, cancel).await
        }
        .await;
        match &result {
            Ok(_) => {
                self.set_progress(
                    "ready",
                    Some(record.model.id.clone()),
                    record.model.bytes,
                    record.model.bytes,
                    Some("Model downloaded. Select it in your library to plan and load.".into()),
                )
                .await
            }
            Err(error) => {
                self.set_progress(
                    "failed",
                    Some(record.model.id.clone()),
                    self.inner.downloaded_bytes.load(Ordering::Relaxed),
                    record.model.bytes,
                    Some(format!("{error:#}")),
                )
                .await
            }
        }
        result
    }

    async fn ensure_custom_model(
        &self,
        record: &mut CustomRecord,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ModelInfo> {
        ensure_not_cancelled(cancel)?;
        let root = &self.inner.custom_dir;
        record.model.validate()?;
        std::fs::create_dir_all(root)?;
        let mut verified_files = Vec::with_capacity(record.model.files.len());
        let mut remaining = 0u64;
        for file in &record.model.files {
            let path = root.join(&record.model.id).join(&file.filename);
            self.set_progress(
                "verifying_model",
                Some(record.model.id.clone()),
                0,
                record.model.bytes,
                Some("Checking reusable model files before reserving disk space".into()),
            )
            .await;
            let verified = match &file.sha256 {
                Some(sha) => self.file_verified(&path, file.bytes, sha).await?,
                None => false,
            };
            verified_files.push(verified);
            if !verified {
                remaining = remaining.saturating_add(download_remaining(file, &path).await?);
            }
        }
        ensure_disk_space(root, remaining)?;
        // An incomplete/repaired set must never be advertised after restart.
        record.complete = false;
        record.save(root)?;
        let mut finished = 0u64;
        for (index, verified) in verified_files.iter().copied().enumerate() {
            let file = record.model.files[index].clone();
            let path = root.join(&record.model.id).join(&file.filename);
            self.set_progress(
                "verifying_model",
                Some(record.model.id.clone()),
                finished,
                record.model.bytes,
                Some(format!("Checking {}", file.filename)),
            )
            .await;
            if !verified {
                self.set_progress(
                    "downloading_model",
                    Some(record.model.id.clone()),
                    finished,
                    record.model.bytes,
                    Some(format!(
                        "Downloading {} ({}/{})",
                        file.filename,
                        index + 1,
                        record.model.files.len()
                    )),
                )
                .await;
                let progress = self.inner.clone();
                let sha = download_artifact_policy(
                    &self.inner.network,
                    &file,
                    &path,
                    move |downloaded| {
                        progress
                            .downloaded_bytes
                            .store(finished.saturating_add(downloaded), Ordering::Relaxed);
                    },
                    cancel,
                )
                .await?;
                // Reject HTML, Git LFS pointers, projectors, and invalid GGUFs
                // before exposing the model through /models or marking ready.
                if let Err(error) = crate::gguf::inspect(&path) {
                    let _ = tokio::fs::remove_file(&path).await;
                    bail!("download is not a supported GGUF model: {error:#}");
                }
                record.model.files[index].sha256 = Some(sha);
                record.save(root)?;
            }
            finished = finished.saturating_add(record.model.files[index].bytes);
        }
        let model = ModelInfo::from_file(&record.path(root), &record.model.id)?;
        record.complete = true;
        record.save(root)?;
        Ok(model)
    }

    pub async fn install(&self, id: &str) -> Result<ModelInfo> {
        let _operation = self.inner.operation.lock().await;
        self.install_locked(id, &Arc::new(AtomicBool::new(false)))
            .await
    }

    async fn install_locked(&self, id: &str, cancel: &Arc<AtomicBool>) -> Result<ModelInfo> {
        ensure_not_cancelled(cancel)?;
        if let Some(record) = self.custom_record(id) {
            return self.install_custom(record, cancel).await;
        }
        let model =
            Self::catalog_model(id).with_context(|| format!("unknown catalog model '{id}'"))?;
        let result = async {
            let lock = install_lock(&crate::config::home())?;
            self.ensure_runtime(lock.clone(), cancel).await?;
            self.ensure_model(model, cancel).await?;
            ModelInfo::from_file(&model.path(), model.id)
        }
        .await;
        let mut status = self.inner.status.lock().await;
        match &result {
            Ok(_) => {
                self.inner
                    .downloaded_bytes
                    .store(model.bytes, Ordering::Relaxed);
                status.phase = "ready".into();
                status.model = Some(model.id.into());
                status.downloaded_bytes = model.bytes;
                status.total_bytes = model.bytes;
                status.message = Some("Model installed and ready to load".into());
            }
            Err(error) => {
                status.phase = "failed".into();
                status.message = Some(format!("{error:#}"));
            }
        }
        result
    }

    async fn ensure_runtime(
        &self,
        lock: Arc<std::fs::File>,
        cancel: &Arc<AtomicBool>,
    ) -> Result<()> {
        ensure_not_cancelled(cancel)?;
        if Toolchain::explicit().is_some() || Toolchain::managed().is_some() {
            return Ok(());
        }
        let asset = runtime_asset()?;
        let archive = crate::config::bin_dir().join(format!("{RUNTIME_DIR}.tar.gz"));
        ensure_disk_space(&crate::config::bin_dir(), asset.bytes.saturating_mul(8))?;
        self.set_progress(
            "downloading_runtime",
            None,
            0,
            asset.bytes,
            Some(format!("Downloading llama.cpp {RUNTIME_VERSION}")),
        )
        .await;
        let progress = self.inner.clone();
        download_verified_policy(
            &self.inner.network,
            asset.url,
            &archive,
            asset.bytes,
            asset.sha256,
            move |downloaded| {
                progress
                    .downloaded_bytes
                    .store(downloaded, Ordering::Relaxed)
            },
            cancel,
        )
        .await?;

        self.set_progress(
            "installing_runtime",
            None,
            asset.bytes,
            asset.bytes,
            Some("Installing the local inference runtime".into()),
        )
        .await;
        let archive_for_task = archive.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = lock;
            install_runtime_archive(&archive_for_task)
        })
        .await
        .context("runtime installer task failed")??;
        ensure_not_cancelled(cancel)?;
        let _ = tokio::fs::remove_file(archive).await;
        if Toolchain::managed().is_none() {
            bail!("downloaded runtime does not contain llama-server and ggml-rpc-server");
        }
        Ok(())
    }

    async fn ensure_model(&self, model: &CatalogModel, cancel: &Arc<AtomicBool>) -> Result<()> {
        let path = model.path();
        self.ensure_model_at(model, &path, cancel).await
    }

    async fn ensure_model_at(
        &self,
        model: &CatalogModel,
        path: &Path,
        cancel: &Arc<AtomicBool>,
    ) -> Result<()> {
        ensure_not_cancelled(cancel)?;
        self.set_progress(
            "verifying_model",
            Some(model.id.into()),
            0,
            model.bytes,
            Some(format!("Checking {}", model.display_name)),
        )
        .await;
        if self.model_verified(path, model).await? {
            return Ok(());
        }
        let source = DownloadFile {
            filename: model.filename.into(),
            url: model.url.into(),
            bytes: model.bytes,
            sha256: Some(model.sha256.into()),
            validator: None,
        };
        ensure_disk_space(
            path.parent().context("model destination has no parent")?,
            download_remaining(&source, path).await?,
        )?;
        self.set_progress(
            "downloading_model",
            Some(model.id.into()),
            0,
            model.bytes,
            Some(format!(
                "Downloading {} ({:.2} GiB, resumes automatically)",
                model.display_name,
                model.bytes as f64 / 1_073_741_824.0
            )),
        )
        .await;
        let progress = self.inner.clone();
        download_verified_policy(
            &self.inner.network,
            model.url,
            path,
            model.bytes,
            model.sha256,
            move |downloaded| {
                progress
                    .downloaded_bytes
                    .store(downloaded, Ordering::Relaxed)
            },
            cancel,
        )
        .await?;
        if let Some(stamp) = file_stamp(path).await {
            self.inner
                .verified_models
                .lock()
                .await
                .insert(path.to_path_buf(), stamp);
        }
        Ok(())
    }

    pub async fn validate_model(&self, model: &ModelInfo) -> Result<()> {
        if let Some(catalog) = Self::catalog_model(&model.name) {
            if !self.model_verified(Path::new(&model.path), catalog).await? {
                bail!(
                    "{} failed its size/checksum check; select it in the TUI to repair the file",
                    model.name
                );
            }
        }
        if let Some(record) = self.custom_record(&model.name) {
            if !record.installed(&self.inner.custom_dir)
                || Path::new(&model.path) != record.path(&self.inner.custom_dir)
            {
                bail!("custom model download is incomplete; download it again from the library");
            }
            for file in &record.model.files {
                let path = self.inner.custom_dir.join(&model.name).join(&file.filename);
                let sha = file
                    .sha256
                    .as_deref()
                    .context("custom model checksum is missing")?;
                if !self.file_verified(&path, file.bytes, sha).await? {
                    bail!("{} failed its checksum check; download it again from the library to repair it", file.filename);
                }
            }
        }
        Ok(())
    }

    async fn model_verified(&self, path: &Path, model: &CatalogModel) -> Result<bool> {
        self.file_verified(path, model.bytes, model.sha256).await
    }

    async fn file_verified(&self, path: &Path, bytes: u64, sha256: &str) -> Result<bool> {
        let Some(stamp) = file_stamp(path).await else {
            return Ok(false);
        };
        if stamp.0 != bytes {
            return Ok(false);
        }
        if self.inner.verified_models.lock().await.get(path) == Some(&stamp) {
            return Ok(true);
        }
        let file = path.to_path_buf();
        let actual = tokio::task::spawn_blocking(move || sha256_file(&file))
            .await
            .context("model checksum task failed")??;
        if actual != sha256 || file_stamp(path).await != Some(stamp) {
            return Ok(false);
        }
        self.inner
            .verified_models
            .lock()
            .await
            .insert(path.to_path_buf(), stamp);
        Ok(true)
    }

    async fn set_progress(
        &self,
        phase: &str,
        model: Option<String>,
        downloaded_bytes: u64,
        total_bytes: u64,
        message: Option<String>,
    ) {
        let job_id = self.inner.active_job.lock().await.clone();
        self.inner
            .downloaded_bytes
            .store(downloaded_bytes, Ordering::Relaxed);
        *self.inner.status.lock().await = ManagedStatus {
            phase: phase.into(),
            model,
            downloaded_bytes,
            total_bytes,
            message,
            job_id,
        };
    }
}

async fn file_stamp(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    metadata.is_file().then(|| {
        (
            metadata.len(),
            metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
        )
    })
}

fn partial_path(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}.part",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("download")
    ))
}

async fn download_remaining(source: &DownloadFile, path: &Path) -> Result<u64> {
    // A final file being repaired remains on disk until atomic replacement. It
    // never reduces the additional bytes required for the new partial file.
    if source.sha256.is_none() && source.validator.is_none() {
        let _ = tokio::fs::remove_file(partial_path(path)).await;
    }
    let mut reusable = if source.sha256.is_some() || source.validator.is_some() {
        tokio::fs::metadata(partial_path(path))
            .await
            .map(|file| file.len())
            .unwrap_or(0)
    } else {
        0
    };
    if reusable > source.bytes {
        reusable = 0;
    }
    if reusable == source.bytes {
        if let Some(expected) = &source.sha256 {
            let part = partial_path(path);
            let actual = tokio::task::spawn_blocking(move || sha256_file(&part)).await??;
            if actual != *expected {
                tokio::fs::remove_file(partial_path(path)).await?;
                reusable = 0;
            }
        }
    }
    Ok(source.bytes.saturating_sub(reusable))
}

fn ensure_disk_space(directory: &Path, additional: u64) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    if fs2::available_space(directory)? < additional.saturating_add(512 * 1024 * 1024) {
        bail!("not enough free disk space for the additional download bytes and 512 MiB of installation headroom");
    }
    Ok(())
}

fn install_lock(home: &Path) -> Result<Arc<std::fs::File>> {
    std::fs::create_dir_all(home)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join("install.lock"))?;
    file.try_lock().context(
        "another localhost.ai instance is installing a model; wait for it to finish and retry",
    )?;
    Ok(Arc::new(file))
}

struct RuntimeAsset {
    url: &'static str,
    sha256: &'static str,
    bytes: u64,
}

fn runtime_asset() -> Result<RuntimeAsset> {
    let asset = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => RuntimeAsset {
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b12110/llama-b12110-bin-ubuntu-x64.tar.gz",
            sha256: "368227edcc229be5f9bed16a2223d79b5e41d0ac2e19bcf5825e82291c060e55",
            bytes: 23_051_958,
        },
        ("linux", "aarch64") => RuntimeAsset {
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b12110/llama-b12110-bin-ubuntu-arm64.tar.gz",
            sha256: "4758b51335e37b4346251f2687d64841ff1c07ecb27fc46323945bb2ec9c44d2",
            bytes: 18_723_578,
        },
        ("macos", "aarch64") => RuntimeAsset {
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b12110/llama-b12110-bin-macos-arm64.tar.gz",
            sha256: "4f5b8de12e0b9516658ca6a5764c6ffb3d7c659c3dd538556b1413c7b4050662",
            bytes: 16_650_691,
        },
        ("macos", "x86_64") => RuntimeAsset {
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b12110/llama-b12110-bin-macos-x64.tar.gz",
            sha256: "c8094268b6bf62f4998f0e45656fd9f5e2d4afb6b1cd0044d761db64be1dd47d",
            bytes: 12_388_601,
        },
        (os, arch) => bail!("automatic llama.cpp installation is not available for {os}/{arch}"),
    };
    Ok(asset)
}

fn install_runtime_archive(archive_path: &Path) -> Result<()> {
    install_runtime_archive_into(archive_path, &crate::config::bin_dir())
}

fn install_runtime_archive_into(archive_path: &Path, bin: &Path) -> Result<()> {
    let staging = bin.join(format!(".{RUNTIME_DIR}.{}.staging", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let archive_file = std::fs::File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    tar::Archive::new(decoder)
        .unpack(&staging)
        .context("extract llama.cpp runtime")?;
    let source = staging.join(RUNTIME_DIR);
    for executable in ["llama-server", "ggml-rpc-server"] {
        if !source.join(executable).is_file() {
            let _ = std::fs::remove_dir_all(&staging);
            bail!("runtime archive is missing {executable}");
        }
    }
    let destination = bin.join(RUNTIME_DIR);
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(&source, &destination)?;
    let _ = std::fs::remove_dir_all(staging);
    Ok(())
}

#[cfg(test)]
async fn download_verified<F>(
    _client: &reqwest::Client,
    url: &str,
    destination: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    progress: F,
) -> Result<()>
where
    F: Fn(u64) + Send + Sync,
{
    #[cfg(test)]
    let policy = crate::network::NetworkPolicy::local_test();
    #[cfg(not(test))]
    let policy = crate::network::NetworkPolicy::default();
    download_verified_policy(
        &policy,
        url,
        destination,
        expected_bytes,
        expected_sha256,
        progress,
        &Arc::new(AtomicBool::new(false)),
    )
    .await
}

async fn download_verified_policy<F>(
    policy: &crate::network::NetworkPolicy,
    url: &str,
    destination: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    progress: F,
    cancel: &Arc<AtomicBool>,
) -> Result<()>
where
    F: Fn(u64) + Send + Sync,
{
    download_artifact_policy(
        policy,
        &DownloadFile {
            filename: String::new(),
            url: url.into(),
            bytes: expected_bytes,
            sha256: Some(expected_sha256.into()),
            validator: None,
        },
        destination,
        progress,
        cancel,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
async fn download_artifact<F>(
    _client: &reqwest::Client,
    source: &DownloadFile,
    destination: &Path,
    progress: F,
) -> Result<String>
where
    F: Fn(u64) + Send + Sync,
{
    #[cfg(test)]
    let policy = crate::network::NetworkPolicy::local_test();
    #[cfg(not(test))]
    let policy = crate::network::NetworkPolicy::default();
    download_artifact_policy(
        &policy,
        source,
        destination,
        progress,
        &Arc::new(AtomicBool::new(false)),
    )
    .await
}

async fn download_artifact_policy<F>(
    policy: &crate::network::NetworkPolicy,
    source: &DownloadFile,
    destination: &Path,
    progress: F,
    cancel: &Arc<AtomicBool>,
) -> Result<String>
where
    F: Fn(u64) + Send + Sync,
{
    const MAX_ATTEMPTS: u32 = 8;
    let mut last_error = None;
    for attempt in 1..=MAX_ATTEMPTS {
        ensure_not_cancelled(cancel)?;
        match download_artifact_once(policy, source, destination, &progress, cancel).await {
            Ok(sha) => return Ok(sha),
            Err(error) if attempt < MAX_ATTEMPTS && retryable_download_error(&error) => {
                last_error = Some(error);
                tokio::select! {
                    _ = tokio::time::sleep(retry_delay(attempt)) => {},
                    _ = wait_cancel(cancel) => return Err(anyhow::anyhow!("installation cancelled; valid partial data was retained")),
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("download failed")))
}

async fn download_artifact_once<F>(
    policy: &crate::network::NetworkPolicy,
    source: &DownloadFile,
    destination: &Path,
    progress: &F,
    cancel: &Arc<AtomicBool>,
) -> Result<String>
where
    F: Fn(u64) + Send + Sync,
{
    let expected_bytes = source.bytes;
    ensure_not_cancelled(cancel)?;
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = partial_path(destination);
    let mut existing = tokio::fs::metadata(&part)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if existing > expected_bytes
        || (existing > 0 && source.sha256.is_none() && source.validator.is_none())
    {
        tokio::fs::remove_file(&part).await?;
        existing = 0;
    }
    if existing == expected_bytes {
        let part_for_hash = part.clone();
        let actual = tokio::task::spawn_blocking(move || sha256_file(&part_for_hash))
            .await
            .context("checksum task failed")??;
        if source
            .sha256
            .as_ref()
            .is_none_or(|expected| *expected == actual)
        {
            tokio::fs::rename(&part, destination).await?;
            progress(expected_bytes);
            return Ok(actual);
        }
        tokio::fs::remove_file(&part).await?;
        existing = 0;
    }
    let mut headers = reqwest::header::HeaderMap::new();
    if existing > 0 {
        headers.insert(
            reqwest::header::RANGE,
            format!("bytes={existing}-").parse()?,
        );
        if let Some(validator) = &source.validator {
            headers.insert(reqwest::header::IF_RANGE, validator.parse()?);
        }
    }
    let response = policy
        .send(
            reqwest::Method::GET,
            &crate::model_hub::http_url(&source.url)?,
            headers,
            std::time::Duration::from_secs(7200),
        )
        .await?;
    let append = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() {
        bail!("download failed with HTTP {}", response.status());
    }
    if source.sha256.is_none() {
        if let Some(expected) = &source.validator {
            let same = [reqwest::header::ETAG, reqwest::header::LAST_MODIFIED]
                .iter()
                .any(|key| {
                    response
                        .headers()
                        .get(key)
                        .and_then(|value| value.to_str().ok())
                        == Some(expected.as_str())
                });
            if !same {
                bail!("download source changed; browse the link again before retrying");
            }
        }
    }
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let range = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .context("partial download response is missing Content-Range")?;
        validate_range(range, existing, expected_bytes)?;
        if response.content_length() != Some(expected_bytes.saturating_sub(existing)) {
            bail!(
                "partial response length does not match Content-Range; partial data was preserved"
            );
        }
    } else if response
        .content_length()
        .is_some_and(|length| length != expected_bytes)
    {
        bail!("download response has the wrong size; partial data was preserved");
    }
    if !append {
        existing = 0;
    }
    ensure_disk_space(
        destination
            .parent()
            .context("download has no parent directory")?,
        expected_bytes.saturating_sub(if append {
            existing
        } else {
            tokio::fs::metadata(&part)
                .await
                .map(|file| file.len().min(expected_bytes))
                .unwrap_or(0)
        }),
    )?;
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).write(true);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    let mut file = options.open(&part).await?;
    let mut downloaded = existing;
    progress(downloaded);
    let mut stream = response.bytes_stream();
    let mut checked_at = downloaded;
    loop {
        ensure_not_cancelled(cancel)?;
        let chunk =
            match tokio::time::timeout(std::time::Duration::from_secs(1), stream.next()).await {
                Err(_) => continue,
                Ok(None) => break,
                Ok(Some(chunk)) => chunk,
            };
        let chunk = chunk
            .map_err(reqwest::Error::without_url)
            .context("download interrupted")?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > expected_bytes {
            bail!("download exceeded its expected size");
        }
        file.write_all(&chunk).await?;
        progress(downloaded);
        if downloaded.saturating_sub(checked_at) >= 64 * 1024 * 1024 {
            ensure_disk_space(
                destination.parent().unwrap(),
                expected_bytes.saturating_sub(downloaded),
            )?;
            checked_at = downloaded;
        }
    }
    file.sync_all().await?;
    drop(file);
    if downloaded != expected_bytes {
        bail!("download is incomplete: received {downloaded} of {expected_bytes} bytes");
    }
    let part_for_hash = part.clone();
    let actual = tokio::task::spawn_blocking(move || sha256_file(&part_for_hash))
        .await
        .context("checksum task failed")??;
    if source
        .sha256
        .as_ref()
        .is_some_and(|expected| *expected != actual)
    {
        let _ = tokio::fs::remove_file(&part).await;
        bail!("download checksum mismatch");
    }
    tokio::fs::rename(&part, destination).await?;
    Ok(actual)
}

fn ensure_not_cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("installation cancelled; valid partial data was retained");
    }
    Ok(())
}

async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn validate_range(value: &str, expected_start: u64, expected_size: u64) -> Result<()> {
    let (range, total) = value
        .strip_prefix("bytes ")
        .and_then(|v| v.split_once('/'))
        .context("invalid Content-Range")?;
    let (start, end) = range.split_once('-').context("invalid Content-Range")?;
    let (start, end, total): (u64, u64, u64) = (start.parse()?, end.parse()?, total.parse()?);
    if start != expected_start
        || total != expected_size
        || end != total.saturating_sub(1)
        || end < start
    {
        bail!(
            "download Content-Range does not match the requested bytes; partial data was preserved"
        );
    }
    Ok(())
}

fn retryable_download_error(error: &anyhow::Error) -> bool {
    if error.chain().any(|cause| {
        cause.downcast_ref::<reqwest::Error>().is_some_and(|error| {
            error.is_timeout() || error.is_connect() || error.is_body() || error.is_request()
        })
    }) {
        return true;
    }
    let message = format!("{error:#}");
    message.contains("download is incomplete")
        || message.contains("error decoding response body")
        || message.contains("error reading a body")
        || message.contains("connection reset")
        || message.contains("end of file before message length")
        || message.contains("HTTP 408")
        || message.contains("HTTP 425")
        || message.contains("HTTP 429")
        || (500..=599).any(|status| message.contains(&format!("HTTP {status}")))
}

fn retry_delay(attempt: u32) -> std::time::Duration {
    #[cfg(test)]
    {
        let _ = attempt;
        std::time::Duration::from_millis(10)
    }
    #[cfg(not(test))]
    {
        std::time::Duration::from_millis(500 * (1u64 << attempt.saturating_sub(1).min(5)))
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[tokio::test]
    async fn corrupt_final_files_do_not_reduce_repair_disk_requirements() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        std::fs::write(&path, vec![0; 1024]).unwrap();
        let source = DownloadFile {
            filename: "model.gguf".into(),
            url: "https://example.com/model.gguf".into(),
            bytes: 1024,
            sha256: Some("a".repeat(64)),
            validator: None,
        };
        assert_eq!(download_remaining(&source, &path).await.unwrap(), 1024);
        std::fs::write(partial_path(&path), vec![0; 400]).unwrap();
        assert_eq!(download_remaining(&source, &path).await.unwrap(), 624);
    }

    #[tokio::test]
    async fn queued_installation_can_be_cancelled_without_starting_network_work() {
        let directory = tempfile::tempdir().unwrap();
        let managed = Managed::with_custom_dir(directory.path().join("custom")).unwrap();
        let operation = managed.inner.operation.lock().await;
        let job = managed
            .enqueue(InstallTask::Model(CATALOG[0].id.into()), |_| async {})
            .await
            .unwrap();
        assert!(managed.cancel(&job.id).await);
        drop(operation);
        let error = managed.wait_job(&job.id).await.unwrap_err();
        assert!(error.to_string().contains("cancel"));
        assert_eq!(managed.job(&job.id).await.unwrap().state, "cancelled");
    }

    async fn serve_download(
        body: Vec<u8>,
        status: &str,
        extra_headers: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/chosen.gguf", listener.local_addr().unwrap());
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n",
            body.len()
        );
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let count = socket.read(&mut request).await.unwrap();
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
            String::from_utf8(request[..count].to_vec()).unwrap()
        });
        (url, task)
    }

    fn custom_record(url: &str, body: &[u8]) -> CustomRecord {
        CustomRecord {
            model: RemoteModel {
                id: format!("custom-{}", "a".repeat(24)),
                display_name: "My chosen model Q4_K_M".into(),
                description: "A model outside the starter catalog".into(),
                license: String::new(),
                bytes: body.len() as u64,
                source: ModelSource::Url {
                    url: url.into(),
                    sha256: None,
                },
                files: vec![DownloadFile {
                    filename: "chosen.gguf".into(),
                    url: url.into(),
                    bytes: body.len() as u64,
                    sha256: Some(format!("{:x}", Sha256::digest(body))),
                    validator: None,
                }],
            },
            complete: false,
        }
    }

    #[tokio::test]
    async fn custom_download_survives_restart_and_detects_later_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("custom");
        let body = crate::gguf::fixture(28);
        let (url, server) = serve_download(body.clone(), "200 OK", "").await;
        let managed = Managed::with_custom_dir(root.clone()).unwrap();
        let mut record = custom_record(&url, &body);
        assert!(managed.installed_custom_models().is_empty());
        let installed = managed
            .ensure_custom_model(&mut record, &Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        assert!(server.await.unwrap().starts_with("GET /chosen.gguf"));
        assert_eq!(installed.layers, 28);
        assert!(record.complete);
        let restarted = Managed::with_custom_dir(root.clone()).unwrap();
        let models = restarted.installed_custom_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, installed.name);
        let public = serde_json::to_string(&restarted.view().await).unwrap();
        assert!(!public.contains(&url));
        assert!(!public.contains("\"source\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let manifest = root.join(&installed.name).join("source.json");
            assert_eq!(
                std::fs::metadata(manifest).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        restarted.validate_model(&models[0]).await.unwrap();
        let mut corrupt = body;
        *corrupt.last_mut().unwrap() ^= 1;
        std::fs::write(&installed.path, corrupt).unwrap();
        let reloaded = Managed::with_custom_dir(root).unwrap();
        assert!(reloaded
            .validate_model(&installed)
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum"));
    }

    #[tokio::test]
    async fn a_downloaded_html_page_is_never_an_installed_model() {
        let directory = tempfile::tempdir().unwrap();
        let body = b"<html>download page, not a model</html>".to_vec();
        let (url, server) = serve_download(body.clone(), "200 OK", "").await;
        let managed = Managed::with_custom_dir(directory.path().join("custom")).unwrap();
        let mut record = custom_record(&url, &body);
        assert!(managed
            .ensure_custom_model(&mut record, &Arc::new(AtomicBool::new(false)))
            .await
            .unwrap_err()
            .to_string()
            .contains("GGUF"));
        assert!(!record.complete);
        assert!(managed.installed_custom_models().is_empty());
        assert!(!record.path(&managed.inner.custom_dir).exists());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unversioned_links_restart_instead_of_splicing_old_partial_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("chosen.gguf");
        std::fs::write(directory.path().join("chosen.gguf.part"), b"OLD!").unwrap();
        let body = crate::gguf::fixture(24);
        let (url, server) = serve_download(body.clone(), "200 OK", "").await;
        let source = DownloadFile {
            filename: "chosen.gguf".into(),
            url,
            bytes: body.len() as u64,
            sha256: None,
            validator: None,
        };
        let sha = download_artifact(&reqwest::Client::new(), &source, &destination, |_| {})
            .await
            .unwrap();
        assert_eq!(sha, format!("{:x}", Sha256::digest(&body)));
        assert_eq!(std::fs::read(destination).unwrap(), body);
        assert!(!server.await.unwrap().to_lowercase().contains("range:"));
    }

    #[tokio::test]
    async fn custom_resume_uses_if_range_and_preserves_partial_when_source_changes() {
        for changed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("chosen.gguf");
            let part = directory.path().join("chosen.gguf.part");
            let body = crate::gguf::fixture(24);
            std::fs::write(&part, &body[..4]).unwrap();
            let headers = if changed {
                "ETag: \"new\"\r\n".into()
            } else {
                format!(
                    "ETag: \"old\"\r\nContent-Range: bytes 4-{}/{}\r\n",
                    body.len() - 1,
                    body.len()
                )
            };
            let (url, server) = serve_download(
                if changed {
                    body.clone()
                } else {
                    body[4..].to_vec()
                },
                if changed {
                    "200 OK"
                } else {
                    "206 Partial Content"
                },
                &headers,
            )
            .await;
            let source = DownloadFile {
                filename: "chosen.gguf".into(),
                url,
                bytes: body.len() as u64,
                sha256: None,
                validator: Some("\"old\"".into()),
            };
            let result =
                download_artifact(&reqwest::Client::new(), &source, &destination, |_| {}).await;
            let request = server.await.unwrap().to_lowercase();
            assert!(request.contains("range: bytes=4-"));
            assert!(request.contains("if-range: \"old\""));
            if changed {
                assert!(result.unwrap_err().to_string().contains("source changed"));
                assert_eq!(std::fs::read(part).unwrap(), &body[..4]);
                assert!(!destination.exists());
            } else {
                result.unwrap();
                assert_eq!(std::fs::read(destination).unwrap(), body);
            }
        }
    }

    #[test]
    fn catalog_is_pinned_and_has_one_recommendation() {
        assert_eq!(CATALOG.len(), 5);
        assert_eq!(CATALOG.iter().filter(|m| m.recommended).count(), 1);
        assert_eq!(
            CATALOG.iter().filter(|model| model.params_b >= 7.0).count(),
            2
        );
        assert!(CATALOG.iter().any(|model| model.bytes > u32::MAX as u64));
        for model in CATALOG {
            assert!(model.url.contains("/resolve/") && !model.url.contains("/main/"));
            assert_eq!(model.sha256.len(), 64);
            assert!(model.bytes > 100_000_000);
            assert!(model.layers > 0);
        }
    }

    #[test]
    fn resume_ranges_support_catalog_files_larger_than_four_gibibytes() {
        validate_range(
            "bytes 4294967296-4683073535/4683073536",
            4_294_967_296,
            4_683_073_536,
        )
        .unwrap();
    }

    #[test]
    fn supported_platform_runtime_is_pinned() {
        if matches!(std::env::consts::OS, "linux" | "macos")
            && matches!(std::env::consts::ARCH, "x86_64" | "aarch64")
        {
            let asset = runtime_asset().unwrap();
            assert!(asset.url.contains(RUNTIME_VERSION));
            assert_eq!(asset.sha256.len(), 64);
            assert!(asset.bytes > 10_000_000);
        }
    }

    #[tokio::test]
    async fn download_resumes_and_verifies_before_becoming_visible() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let data = b"a checksummed resumable model";
        let expected = format!("{:x}", Sha256::digest(data));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0u8; 4096];
            let read = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.to_ascii_lowercase().contains("range: bytes=5-"));
            let body = &data[5..];
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 5-{}/{}\r\nConnection: close\r\n\r\n",
                        body.len(), data.len() - 1, data.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.write_all(body).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("model.gguf");
        let part = destination.with_extension("gguf.part");
        std::fs::write(&part, &data[..5]).unwrap();
        let progress = Arc::new(AtomicU64::new(0));
        let progress_for_download = progress.clone();
        download_verified(
            &reqwest::Client::new(),
            &url,
            &destination,
            data.len() as u64,
            &expected,
            move |value| progress_for_download.store(value, Ordering::Relaxed),
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), data);
        assert!(!part.exists());
        assert_eq!(progress.load(Ordering::Relaxed), data.len() as u64);
    }

    #[tokio::test]
    async fn connection_reset_is_retried_from_the_partial_offset() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let data = vec![b'x'; 256 * 1024];
        let expected = format!("{:x}", Sha256::digest(&data));
        let served = data.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(first.read(&mut request).await.unwrap() > 0);
            first
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        served.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            first.write_all(&served[..64 * 1024]).await.unwrap();
            first.flush().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            first.shutdown().await.unwrap();

            let (mut second, _) = listener.accept().await.unwrap();
            let read = second.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
            let start: usize = request
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="))
                .and_then(|range| range.strip_suffix('-'))
                .unwrap()
                .parse()
                .unwrap();
            assert!(start > 0 && start <= 64 * 1024);
            let body = &served[start..];
            second
                .write_all(
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{}\r\nConnection: close\r\n\r\n",
                        body.len(), served.len() - 1, served.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            second.write_all(body).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("model.gguf");
        download_verified(
            &reqwest::Client::new(),
            &url,
            &destination,
            data.len() as u64,
            &expected,
            |_| {},
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), data);
    }

    #[test]
    fn runtime_extraction_requires_both_executables() {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("runtime.tar.gz");
        let archive_file = std::fs::File::create(&archive_path).unwrap();
        let mut archive = tar::Builder::new(GzEncoder::new(archive_file, Compression::fast()));
        for name in ["llama-server", "ggml-rpc-server"] {
            let data = format!("#!/bin/sh\necho {name}\n");
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o755);
            header.set_size(data.len() as u64);
            header.set_cksum();
            archive
                .append_data(
                    &mut header,
                    format!("{RUNTIME_DIR}/{name}"),
                    data.as_bytes(),
                )
                .unwrap();
        }
        archive
            .into_inner()
            .unwrap()
            .finish()
            .unwrap()
            .flush()
            .unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        install_runtime_archive_into(&archive_path, &bin).unwrap();
        assert!(bin.join(RUNTIME_DIR).join("llama-server").is_file());
        assert!(bin.join(RUNTIME_DIR).join("ggml-rpc-server").is_file());
    }

    #[test]
    fn two_managers_cannot_write_the_same_installation() {
        let directory = tempfile::tempdir().unwrap();
        let first = install_lock(directory.path()).unwrap();
        assert!(install_lock(directory.path()).is_err());
        drop(first);
        assert!(install_lock(directory.path()).is_ok());
    }

    #[tokio::test]
    async fn a_wrong_range_does_not_destroy_the_saved_partial_file() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            let _ = socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 11\r\nContent-Range: bytes 0-10/15\r\n\r\nabcdefghijk").await;
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        let partial = path.with_extension("gguf.part");
        std::fs::write(&partial, b"abcd").unwrap();
        let error = download_verified(
            &reqwest::Client::new(),
            &url,
            &path,
            15,
            &format!("{:x}", Sha256::digest(b"abcdefghijklmno")),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Content-Range"));
        assert_eq!(std::fs::read(&partial).unwrap(), b"abcd");
        assert!(!path.exists());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn equal_length_corruption_is_repaired_before_model_loading() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let good = b"verified model content";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", good.len()).as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(good).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        std::fs::write(&path, vec![b'x'; good.len()]).unwrap();
        let model = CatalogModel {
            bytes: good.len() as u64,
            url: Box::leak(url.into_boxed_str()),
            sha256: Box::leak(format!("{:x}", Sha256::digest(good)).into_boxed_str()),
            ..CATALOG[0].clone()
        };
        let manager = Managed::new().unwrap();
        manager
            .ensure_model_at(&model, &path, &Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), good);
        task.await.unwrap();
        // A second ensure succeeds offline using the validated local file.
        manager
            .ensure_model_at(&model, &path, &Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
    }
}
