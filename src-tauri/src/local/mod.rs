//! Local speech recognition with transcribe.cpp: models run on this
//! computer (on the GPU through Vulkan, or on the CPU), nothing is sent
//! anywhere.
//!
//! - `catalog`: the models and the runtime that can be downloaded
//! - `hardware`: whether this computer can use Vulkan
//! - `download` / `runtime`: verified downloads and the files on disk
//! - `ffi`: transcribe.dll, loaded at run time from the runtime folder
//!
//! [`LocalEngine`] keeps at most one model in memory. It loads it when a
//! dictation starts (while the user is still speaking) and frees it after
//! the delay chosen in Engine → Local. The runtime itself stays loaded once
//! used.

pub mod catalog;
pub mod download;
pub mod ffi;
pub mod hardware;
pub mod runtime;
pub mod worker;

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

use catalog::{Backend, ModelSpec, MODELS, RUNTIME};
use download::{DownloadError, Expected};
use hardware::Hardware;
use runtime::Paths;
use worker::{Worker, WorkerError};

use crate::settings::Engine;

/// Download progress, payload [`DownloadProgress`]
pub const DOWNLOAD_EVENT: &str = "local-download";
/// Something changed (installed, deleted, loaded, unloaded); the UI re-reads
/// `local_status`
pub const CHANGED_EVENT: &str = "local-changed";

const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// The components' download (models are "model:<id>")
const RUNTIME_KEY: &str = "runtime";

/// Local recognition works with the Windows x64 runtime only
pub fn is_supported() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}

/// How long an idle model stays in memory; `None` keeps it loaded.
pub fn unload_delay(policy: &str) -> Option<Duration> {
    match policy {
        "immediate" => Some(Duration::ZERO),
        "30s" => Some(Duration::from_secs(30)),
        "5m" => Some(Duration::from_secs(5 * 60)),
        "10m" => Some(Duration::from_secs(10 * 60)),
        "never" => None,
        _ => Some(Duration::from_secs(5 * 60)),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn megabytes(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

struct LoadedModel {
    spec: &'static ModelSpec,
    /// The backend that was asked for (the model may have fallen back to the CPU)
    requested: Backend,
    worker: Worker,
}

/// The model in memory, as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedView {
    pub model: String,
    pub name: String,
    /// The backend it actually runs on, e.g. "Vulkan" or "CPU"
    pub backend: String,
    pub device: String,
    /// Why the requested GPU backend wasn't used, when it fell back to the CPU
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    /// "model:<id>" or "runtime"
    pub key: String,
    /// "downloading" | "installing" | "done" | "error" | "cancelled"
    pub state: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
}

struct DownloadJob {
    cancel: AtomicBool,
    progress: Mutex<DownloadProgress>,
    last_emit: Mutex<Option<Instant>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub id: Backend,
    /// This computer has the hardware and driver for it
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub size: u64,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    pub supported: bool,
    pub models: Vec<ModelStatus>,
    pub backends: Vec<BackendStatus>,
    pub recommended: Backend,
    pub runtime: RuntimeStatus,
    pub loaded: Option<LoadedView>,
    pub downloads: Vec<DownloadProgress>,
    pub folder: String,
}

#[derive(Clone)]
pub struct LocalEngine {
    inner: Arc<Inner>,
}

struct Inner {
    paths: Paths,
    app: OnceLock<AppHandle>,
    hardware: OnceLock<Hardware>,
    /// Started as the recognition process (Typr's own executable)
    worker_exe: PathBuf,
    /// The model in memory, locked for the whole of a load or a run
    model: Mutex<Option<LoadedModel>>,
    /// Copy of `model` for the UI, readable while a load runs
    view: Mutex<Option<LoadedView>>,
    /// Bumped at every dictation; a pending unload fires only if unchanged
    generation: AtomicU64,
    /// Why the GPU's recognition process crashed: the CPU is used until the
    /// acceleration setting changes
    gpu_crash: Mutex<Option<String>>,
    downloads: Mutex<HashMap<String, Arc<DownloadJob>>>,
}

impl LocalEngine {
    pub fn new(paths: Paths) -> Self {
        let exe = std::env::current_exe().unwrap_or_default();
        Self::with_worker_exe(paths, exe)
    }

    /// `exe` runs the recognition process (`exe --local-recognition`)
    pub fn with_worker_exe(paths: Paths, exe: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                paths,
                app: OnceLock::new(),
                hardware: OnceLock::new(),
                worker_exe: exe,
                model: Mutex::new(None),
                view: Mutex::new(None),
                generation: AtomicU64::new(0),
                gpu_crash: Mutex::new(None),
                downloads: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Lets the engine send events to the main window
    pub fn attach(&self, app: AppHandle) {
        let _ = self.inner.app.set(app);
    }

    fn notify_changed(&self) {
        if let Some(app) = self.inner.app.get() {
            if let Err(e) = app.emit(CHANGED_EVENT, ()) {
                log::debug!("Failed to emit {}: {}", CHANGED_EVENT, e);
            }
        }
    }

    pub fn hardware(&self) -> &Hardware {
        self.inner.hardware.get_or_init(Hardware::detect)
    }

    fn paths(&self) -> &Paths {
        &self.inner.paths
    }

    // ── Status ───────────────────────────────────────────

    pub fn status(&self) -> LocalStatus {
        let hardware = self.hardware();
        let paths = self.paths();

        let models = MODELS
            .iter()
            .map(|m| ModelStatus {
                id: m.id.to_string(),
                name: m.name.to_string(),
                size: m.size,
                installed: runtime::is_model_installed(paths, m),
            })
            .collect();

        let backends = Backend::ALL
            .into_iter()
            .map(|backend| BackendStatus {
                id: backend,
                available: hardware.is_available(backend),
            })
            .collect();

        let downloads = lock(&self.inner.downloads)
            .values()
            .map(|job| lock(&job.progress).clone())
            .collect();

        LocalStatus {
            supported: is_supported(),
            models,
            backends,
            recommended: hardware.recommended(),
            runtime: RuntimeStatus {
                size: RUNTIME.size,
                installed: runtime::is_runtime_installed(paths),
            },
            loaded: lock(&self.inner.view).clone(),
            downloads,
            folder: paths.models().display().to_string(),
        }
    }

    // ── Model ────────────────────────────────────────────

    /// Makes `spec` the model in memory, on `backend`: starts a recognition
    /// process with it. The process falls back to the CPU itself when the
    /// GPU can't take the model; if it crashes on the GPU, a CPU one starts.
    fn ensure_model(&self, slot: &mut Option<LoadedModel>, spec: &'static ModelSpec, backend: Backend) -> Result<(), String> {
        if let Some(current) = slot.as_ref() {
            if current.spec.id == spec.id && current.requested == backend {
                return Ok(());
            }
        }
        self.free_model(slot, "another model or backend was chosen");

        if !is_supported() {
            return Err("Local recognition is available on 64-bit Windows only".to_string());
        }
        if !runtime::is_model_installed(self.paths(), spec) {
            return Err(format!(
                "{} isn't downloaded yet. Download it in Engine → Local",
                spec.name
            ));
        }
        if !runtime::is_runtime_installed(self.paths()) {
            // They come with the first model; fetch them again if they went missing
            if let Err(e) = self.download_runtime() {
                log::debug!("Components not downloaded: {}", e);
            }
            return Err(
                "The local recognition components are being downloaded. Try again in a moment"
                    .to_string(),
            );
        }

        let started = Instant::now();
        let crash = if backend == Backend::Cpu {
            None
        } else {
            lock(&self.inner.gpu_crash).clone()
        };
        let worker = match crash {
            Some(reason) => self.start_on_cpu(spec, reason)?,
            None => match self.start_worker(spec, backend) {
                Err(WorkerError::Crashed(reason)) if backend != Backend::Cpu => {
                    log::error!("Loading {} on {} crashed: {}", spec.name, backend.label(), reason);
                    let reason = format!("{} crashed ({})", backend.label(), reason);
                    *lock(&self.inner.gpu_crash) = Some(reason.clone());
                    self.start_on_cpu(spec, reason)?
                }
                other => other.map_err(|e| e.message().to_string())?,
            },
        };
        let placement = worker.placement().clone();
        log::info!(
            "{} loaded on {} ({}) in {} ms",
            spec.name,
            placement.backend,
            placement.device,
            started.elapsed().as_millis()
        );
        *lock(&self.inner.view) = Some(LoadedView {
            model: spec.id.to_string(),
            name: spec.name.to_string(),
            backend: placement.backend,
            device: placement.device,
            fallback: placement.fallback,
        });
        *slot = Some(LoadedModel {
            spec,
            requested: backend,
            worker,
        });
        self.notify_changed();
        Ok(())
    }

    fn start_worker(&self, spec: &ModelSpec, backend: Backend) -> Result<Worker, WorkerError> {
        Worker::start(
            &self.inner.worker_exe,
            &self.paths().runtime_dir(),
            &self.paths().model_file(spec),
            backend,
        )
    }

    /// A CPU process in place of a GPU one that crashed; the reason shows
    /// in the UI
    fn start_on_cpu(&self, spec: &ModelSpec, reason: String) -> Result<Worker, String> {
        let mut worker = self
            .start_worker(spec, Backend::Cpu)
            .map_err(|e| e.message().to_string())?;
        worker.set_fallback(reason);
        Ok(worker)
    }

    /// Ends the recognition process, which frees everything it loaded.
    fn free_model(&self, slot: &mut Option<LoadedModel>, reason: &str) {
        let Some(current) = slot.take() else {
            return;
        };
        drop(current.worker);
        *lock(&self.inner.view) = None;
        log::info!("{} unloaded from memory ({})", current.spec.name, reason);
        self.notify_changed();
    }

    fn model_for(choice: &str) -> &'static ModelSpec {
        catalog::model(choice)
            .or_else(|| catalog::model(catalog::DEFAULT_MODEL))
            .expect("the default model is in the catalog")
    }

    /// Loads the chosen model if needed. Blocking.
    fn prepare(&self, model: &str, backend_choice: &str) -> Result<(), String> {
        let backend = self.hardware().resolve(backend_choice);
        let spec = Self::model_for(model);
        let mut slot = lock(&self.inner.model);
        self.ensure_model(&mut slot, spec, backend)
    }

    fn transcribe_blocking(
        &self,
        model: &str,
        backend_choice: &str,
        language: Option<&str>,
        samples: &[f32],
    ) -> Result<String, String> {
        let backend = self.hardware().resolve(backend_choice);
        let spec = Self::model_for(model);
        let mut slot = lock(&self.inner.model);
        self.ensure_model(&mut slot, spec, backend)?;

        let current = slot.as_mut().expect("ensure_model leaves a model loaded");
        let result = current.worker.transcribe(samples, language);
        let placement = current.worker.placement().clone();
        match result {
            Ok(text) => {
                // The process may have moved the model to the CPU
                if let Some(view) = lock(&self.inner.view).as_mut() {
                    if view.backend != placement.backend || view.fallback != placement.fallback {
                        view.backend = placement.backend;
                        view.device = placement.device;
                        view.fallback = placement.fallback;
                        self.notify_changed();
                    }
                }
                Ok(text)
            }
            Err(WorkerError::Failed(message)) => Err(message),
            Err(WorkerError::Crashed(reason)) => {
                self.free_model(&mut slot, "the recognition process stopped");
                if placement.backend == Backend::Cpu.label() {
                    return Err(format!("Local recognition stopped unexpectedly: {}", reason));
                }
                log::error!(
                    "{} crashed during transcription: {}; retrying on the CPU",
                    placement.backend,
                    reason
                );
                *lock(&self.inner.gpu_crash) = Some(format!("{} crashed ({})", placement.backend, reason));
                self.ensure_model(&mut slot, spec, backend)?;
                let current = slot.as_mut().expect("ensure_model leaves a model loaded");
                current.worker.transcribe(samples, language).map_err(|e| e.message().to_string())
            }
        }
    }

    /// Transcribes 16 kHz mono samples with `model`. `language` is a hint
    /// such as "ru"; `None` detects it.
    pub async fn transcribe(
        &self,
        model: &str,
        backend: &str,
        language: Option<&str>,
        samples: Vec<f32>,
    ) -> Result<String, String> {
        let engine = self.clone();
        let model = model.to_string();
        let backend = backend.to_string();
        let language = language.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            engine.transcribe_blocking(&model, &backend, language.as_deref(), &samples)
        })
        .await
        .map_err(|e| format!("Local recognition stopped unexpectedly: {}", e))?
    }

    /// A downloaded model that can stand in when a cloud engine fails:
    /// `preferred` if it is downloaded, else the first downloaded one.
    /// `None` when nothing local can run.
    pub fn fallback_model(&self, preferred: &str) -> Option<&'static str> {
        let paths = self.paths();
        if !is_supported() || !runtime::is_runtime_installed(paths) {
            return None;
        }
        catalog::model(preferred)
            .into_iter()
            .chain(MODELS.iter())
            .find(|spec| runtime::is_model_installed(paths, spec))
            .map(|spec| spec.id)
    }

    /// Whether a model is in memory now.
    pub fn is_loaded(&self) -> bool {
        lock(&self.inner.view).is_some()
    }

    /// A dictation started: load the model while the user speaks, so the
    /// transcription doesn't wait for it. Errors surface at transcription.
    pub fn preload(&self, model: &str, backend: &str) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        let engine = self.clone();
        let model = model.to_string();
        let backend = backend.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = engine.prepare(&model, &backend) {
                log::debug!("Model not preloaded: {}", e);
            }
        });
    }

    /// A dictation ended (pasted, failed or cancelled): unload the model
    /// after the delay of `policy`.
    pub fn schedule_unload(&self, policy: &str) {
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(delay) = unload_delay(policy) else {
            return;
        };
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if engine.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let _ = tokio::task::spawn_blocking(move || {
                let mut slot = lock(&engine.inner.model);
                // A dictation may have started while waiting for the lock
                if engine.inner.generation.load(Ordering::SeqCst) == generation {
                    engine.free_model(&mut slot, "idle");
                }
            })
            .await;
        });
    }

    /// Frees the model now (in the background; waits for a running dictation).
    pub fn unload(&self, reason: &'static str) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut slot = lock(&engine.inner.model);
            engine.free_model(&mut slot, reason);
        });
    }

    /// Settings were saved: drop a model that is no longer wanted and
    /// restart the unload timer under a new delay.
    pub fn settings_changed(&self, old: &crate::settings::Settings, new: &crate::settings::Settings) {
        let was_local = old.engine == Engine::Local;
        let is_local = new.engine == Engine::Local;
        if old.local_backend != new.local_backend {
            // A new choice gets a fresh try on the GPU
            *lock(&self.inner.gpu_crash) = None;
        }
        if was_local && !is_local {
            self.unload("another engine was chosen");
        } else if old.local_model != new.local_model || old.local_backend != new.local_backend {
            self.unload("another model or backend was chosen");
        } else if is_local && old.local_unload != new.local_unload {
            self.schedule_unload(&new.local_unload);
        }
    }

    // ── Downloads ────────────────────────────────────────

    fn start_job(&self, key: &str) -> Result<Arc<DownloadJob>, String> {
        let mut downloads = lock(&self.inner.downloads);
        if downloads.contains_key(key) {
            return Err("This download is already running".to_string());
        }
        let job = Arc::new(DownloadJob {
            cancel: AtomicBool::new(false),
            progress: Mutex::new(DownloadProgress {
                key: key.to_string(),
                state: "downloading".to_string(),
                downloaded: 0,
                total: None,
                error: None,
            }),
            last_emit: Mutex::new(None),
        });
        downloads.insert(key.to_string(), job.clone());
        drop(downloads);
        self.emit_progress(&job, true);
        Ok(job)
    }

    fn emit_progress(&self, job: &DownloadJob, force: bool) {
        {
            let mut last = lock(&job.last_emit);
            if !force && last.map_or(false, |t| t.elapsed() < PROGRESS_INTERVAL) {
                return;
            }
            *last = Some(Instant::now());
        }
        let progress = lock(&job.progress).clone();
        if let Some(app) = self.inner.app.get() {
            let _ = app.emit(DOWNLOAD_EVENT, progress);
        }
    }

    fn report(&self, job: &DownloadJob, downloaded: u64, total: u64) {
        {
            let mut progress = lock(&job.progress);
            progress.downloaded = downloaded;
            progress.total = Some(total);
        }
        self.emit_progress(job, total == downloaded);
    }

    fn set_state(&self, job: &DownloadJob, state: &str, error: Option<String>) {
        {
            let mut progress = lock(&job.progress);
            progress.state = state.to_string();
            progress.error = error;
        }
        self.emit_progress(job, true);
    }

    fn finish_job(&self, key: &str, job: &DownloadJob, result: Result<(), DownloadError>) {
        match result {
            Ok(()) => {
                log::info!("Download '{}' finished", key);
                self.set_state(job, "done", None);
            }
            Err(DownloadError::Cancelled) => {
                log::info!("Download '{}' cancelled", key);
                self.set_state(job, "cancelled", None);
            }
            Err(DownloadError::Failed(e)) => {
                log::error!("Download '{}' failed: {}", key, e);
                self.set_state(job, "error", Some(e));
            }
        }
        lock(&self.inner.downloads).remove(key);
        self.notify_changed();
    }

    pub fn cancel_download(&self, key: &str) {
        if let Some(job) = lock(&self.inner.downloads).get(key) {
            job.cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn download_model(&self, id: &str) -> Result<(), String> {
        let spec = catalog::model(id).ok_or_else(|| format!("Unknown model: {}", id))?;
        if runtime::is_model_installed(self.paths(), spec) {
            return Ok(());
        }
        let key = format!("model:{}", spec.id);
        let job = self.start_job(&key)?;
        log::info!("Downloading {} ({} MB)", spec.name, megabytes(spec.size));
        // The components come with the model: it can't run without them
        self.fetch_missing_runtime();
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = engine.fetch_model(&job, spec).await;
            engine.finish_job(&key, &job, result);
        });
        Ok(())
    }

    async fn fetch_model(&self, job: &DownloadJob, spec: &'static ModelSpec) -> Result<(), DownloadError> {
        let client = download::client()?;
        let target = self.paths().model_file(spec);
        let part = runtime::part_path(&target);
        let expected = Expected {
            size: spec.size,
            sha256: spec.sha256,
        };
        let result = download::fetch(&client, &spec.url(), &part, &expected, &job.cancel, |done, total| {
            self.report(job, done, total)
        })
        .await;
        if result == Err(DownloadError::Cancelled) {
            let _ = tokio::fs::remove_file(&part).await;
        }
        result?;
        tokio::fs::rename(&part, &target)
            .await
            .map_err(|e| format!("Failed to save {}: {}", target.display(), e))?;
        Ok(())
    }

    /// At start: a downloaded model never stays without the components
    /// (they can't be deleted; a failed or cancelled download is retried).
    pub fn restore_runtime(&self) {
        if MODELS.iter().any(|m| runtime::is_model_installed(self.paths(), m)) {
            self.fetch_missing_runtime();
        }
    }

    /// Starts downloading the components unless they are there or on the way.
    fn fetch_missing_runtime(&self) {
        if !is_supported()
            || runtime::is_runtime_installed(self.paths())
            || lock(&self.inner.downloads).contains_key(RUNTIME_KEY)
        {
            return;
        }
        if let Err(e) = self.download_runtime() {
            log::warn!("The local recognition components didn't start downloading: {}", e);
        }
    }

    pub fn download_runtime(&self) -> Result<(), String> {
        if !is_supported() {
            return Err("Local recognition is available on 64-bit Windows only".to_string());
        }
        if runtime::is_runtime_installed(self.paths()) {
            return Ok(());
        }
        let key = RUNTIME_KEY;
        let job = self.start_job(key)?;
        log::info!("Downloading the transcribe.cpp runtime ({} MB)", megabytes(RUNTIME.size));
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = engine.fetch_runtime(&job).await;
            engine.finish_job(key, &job, result);
        });
        Ok(())
    }

    async fn fetch_runtime(&self, job: &DownloadJob) -> Result<(), DownloadError> {
        let client = download::client()?;
        let target = self.paths().runtime_download();
        let part = runtime::part_path(&target);
        let expected = Expected {
            size: RUNTIME.size,
            sha256: RUNTIME.sha256,
        };
        let result = download::fetch(&client, RUNTIME.url, &part, &expected, &job.cancel, |done, total| {
            self.report(job, done, total)
        })
        .await;
        if result == Err(DownloadError::Cancelled) {
            let _ = tokio::fs::remove_file(&part).await;
        }
        result?;
        tokio::fs::rename(&part, &target)
            .await
            .map_err(|e| format!("Failed to save {}: {}", target.display(), e))?;

        self.set_state(job, "installing", None);
        let paths = self.paths().clone();
        let archive = target.clone();
        let installed = tokio::task::spawn_blocking(move || runtime::install_runtime(&paths, &archive))
            .await
            .map_err(|e| format!("Install stopped unexpectedly: {}", e))?;
        let _ = tokio::fs::remove_file(&target).await;
        installed?;
        Ok(())
    }

    pub fn delete_model(&self, id: &str) -> Result<(), String> {
        let spec = catalog::model(id).ok_or_else(|| format!("Unknown model: {}", id))?;
        {
            let mut slot = lock(&self.inner.model);
            if slot.as_ref().map_or(false, |m| m.spec.id == spec.id) {
                self.free_model(&mut slot, "the model was deleted");
            }
        }
        let path = self.paths().model_file(spec);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("Failed to delete {}: {}", path.display(), e))?;
        }
        let _ = std::fs::remove_file(runtime::part_path(&path));
        log::info!("{} deleted", spec.name);
        self.notify_changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unload_delay() {
        assert_eq!(unload_delay("immediate"), Some(Duration::ZERO));
        assert_eq!(unload_delay("30s"), Some(Duration::from_secs(30)));
        assert_eq!(unload_delay("10m"), Some(Duration::from_secs(600)));
        assert_eq!(unload_delay("never"), None);
        assert_eq!(unload_delay("unknown"), Some(Duration::from_secs(300)));
    }

    #[test]
    fn test_status_of_an_empty_install() {
        let root = std::env::temp_dir().join("typr_test_local_status");
        let _ = std::fs::remove_dir_all(&root);
        let engine = LocalEngine::new(Paths::new(root.clone()));
        let status = engine.status();
        assert_eq!(status.models.len(), MODELS.len());
        assert!(status.models.iter().all(|m| !m.installed));
        assert!(!status.runtime.installed);
        assert_eq!(status.runtime.size, RUNTIME.size);
        assert!(status.loaded.is_none());
        assert_eq!(status.backends.len(), 2);
        let cpu = status.backends.iter().find(|b| b.id == Backend::Cpu).unwrap();
        assert!(cpu.available);
    }

    #[test]
    fn test_no_fallback_without_a_downloaded_model() {
        let root = std::env::temp_dir().join("typr_test_local_fallback");
        let _ = std::fs::remove_dir_all(&root);
        let engine = LocalEngine::new(Paths::new(root));
        assert_eq!(engine.fallback_model(catalog::DEFAULT_MODEL), None);
        assert!(!engine.is_loaded());
    }

    #[test]
    fn test_transcribing_without_downloads_explains_what_is_missing() {
        let root = std::env::temp_dir().join("typr_test_local_missing");
        let _ = std::fs::remove_dir_all(&root);
        let engine = LocalEngine::new(Paths::new(root));
        let error = engine
            .transcribe_blocking(catalog::DEFAULT_MODEL, "cpu", None, &[0.0; 16000])
            .unwrap_err();
        if is_supported() {
            assert!(error.contains("isn't downloaded yet"), "{}", error);
        }
    }
}
