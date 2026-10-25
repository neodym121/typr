//! Local engine: transcribes the recording on this computer with
//! transcribe.cpp (see `local`). Unlike the cloud engines it takes the
//! samples straight from memory, nothing is encoded. It also stands in for a
//! cloud engine that failed, when a model is downloaded.

use tauri::{AppHandle, Manager};

use crate::local::LocalEngine;
use crate::settings::{Engine, Settings};

fn engine(app: &AppHandle) -> Result<LocalEngine, String> {
    app.try_state::<LocalEngine>()
        .map(|engine| engine.inner().clone())
        .ok_or_else(|| "Local recognition is not initialised".to_string())
}

/// Transcribes with `model` (normally `settings.local_model`).
pub async fn transcribe(app: &AppHandle, settings: &Settings, model: &str, samples: Vec<f32>) -> Result<String, String> {
    let engine = engine(app)?;
    log::info!(
        "Local: model {}, backend {}, language {}",
        model,
        if settings.local_backend.is_empty() { "auto" } else { settings.local_backend.as_str() },
        settings.language_hint().unwrap_or("auto")
    );
    engine
        .transcribe(model, &settings.local_backend, settings.language_hint(), samples)
        .await
}

/// The downloaded model that can take over a failed cloud transcription.
pub fn fallback_model(app: &AppHandle, settings: &Settings) -> Option<&'static str> {
    engine(app).ok()?.fallback_model(&settings.local_model)
}

/// A dictation ended one way or another: start the unload countdown. A model
/// that a fallback brought in while a cloud engine is chosen never stays for
/// good.
pub fn dictation_ended(app: &AppHandle, settings: &Settings) {
    let Ok(engine) = engine(app) else {
        return;
    };
    if settings.engine == Engine::Local {
        engine.schedule_unload(&settings.local_unload);
    } else if engine.is_loaded() {
        let policy = match settings.local_unload.as_str() {
            "never" => "5m",
            other => other,
        };
        engine.schedule_unload(policy);
    }
}
