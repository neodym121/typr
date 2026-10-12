use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::ask;
use crate::audio::{self, AudioFormat, AudioRecorder, EncodedAudio};
use crate::cleanup::cleanup_text;
use crate::net;
use crate::overlay::{self, Indicator};
use crate::paste::paste_text;
use crate::postprocess;
use crate::settings::{Engine, Settings};
use crate::transcribe_assemblyai;
use crate::transcribe_groq;
use crate::transcribe_local;
use crate::transcribe_openai;
use crate::transcribe_polza;

/// Shorter recordings are almost always accidental taps; Whisper tends to
/// hallucinate text for them.
const MIN_RECORDING_SECS: f32 = 0.3;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum RecordingState {
    Ready,
    Recording,
    Transcribing,
}

fn publish_state(app: &AppHandle, state: &RecordingState) {
    if let Err(e) = app.emit("recording-state", state.clone()) {
        log::warn!("Failed to emit recording-state: {}", e);
    }
    let indicator = match state {
        RecordingState::Ready => Indicator::Idle,
        RecordingState::Recording => Indicator::Recording,
        RecordingState::Transcribing => Indicator::Transcribing,
    };
    overlay::set_indicator(app, indicator);
}

/// Shows a failed dictation in the main window and briefly flashes the overlay.
pub fn notify_error(app: &AppHandle, message: &str) {
    if let Err(e) = app.emit("recording-error", message.to_string()) {
        log::warn!("Failed to emit recording-error: {}", e);
    }
    overlay::flash_error(app);
}

/// What became of a recording.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Pasted into the active window (empty: nothing was said)
    Pasted(String),
    /// A quick ask question, answered in its window
    Asked,
}

/// A cloud engine failed and a local model transcribed the dictation instead.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FallbackNotice {
    /// The cloud engine that failed, e.g. "Groq"
    pub engine: String,
    pub error: String,
}

/// Opens the connections a dictation will need while the user is still
/// speaking: the cloud engine and, if they are on, post-processing and quick
/// ask.
pub fn warm_up_connections(settings: &Settings) {
    let engine_url = match settings.engine {
        Engine::Local => None,
        Engine::Groq => Some(transcribe_groq::HOST.to_string()),
        Engine::Polza => Some(transcribe_polza::HOST.to_string()),
        Engine::AssemblyAi => Some(transcribe_assemblyai::HOST.to_string()),
        Engine::OpenAi => Some(transcribe_openai::transcriptions_url(&settings.openai_endpoint)),
    };
    let post_url = settings
        .post_process
        .enabled
        .then(|| postprocess::host(settings.post_process.provider).to_string());
    let ask = &settings.quick_ask;
    let ask_url = ask.enabled.then(|| postprocess::host(ask.provider).to_string());
    let search_url = (ask.enabled && ask.web_search && !ask.exa_api_key.trim().is_empty())
        .then(|| ask::search::HOST.to_string());
    // The same server for two of them: one connection serves both requests
    let mut urls: Vec<String> = engine_url
        .into_iter()
        .chain(post_url)
        .chain(ask_url)
        .chain(search_url)
        .collect();
    urls.sort();
    urls.dedup();
    for url in urls {
        net::warm_up(url);
    }
}

/// How the recording is packed for `settings.engine`.
fn upload_format(settings: &Settings) -> AudioFormat {
    match settings.engine {
        Engine::Groq => transcribe_groq::FORMAT,
        Engine::AssemblyAi => transcribe_assemblyai::FORMAT,
        Engine::Polza => transcribe_polza::FORMAT,
        Engine::OpenAi | Engine::Local => transcribe_openai::format_for(&settings.openai_endpoint),
    }
}

pub struct Recorder {
    state: Arc<Mutex<RecordingState>>,
    audio_recorder: Arc<Mutex<AudioRecorder>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingState::Ready)),
            audio_recorder: Arc::new(Mutex::new(AudioRecorder::new())),
        }
    }

    pub fn get_state(&self) -> RecordingState {
        self.state.lock().unwrap().clone()
    }

    pub fn start_recording(&self, app: &AppHandle, mic_name: &str) -> Result<(), String> {
        {
            let mut state = self.state.lock().unwrap();
            if *state != RecordingState::Ready {
                return Err(format!("Cannot start recording while {:?}", *state));
            }

            log::info!("Starting recording (microphone: '{}')", mic_name);
            let mut recorder = self.audio_recorder.lock().unwrap();
            recorder.start(mic_name)?;
            *state = RecordingState::Recording;
        }

        publish_state(app, &RecordingState::Recording);
        log::info!("State: Ready → Recording");
        Ok(())
    }

    /// Stops an unfinished recording and throws the audio away. Returns
    /// whether there was one.
    pub fn cancel_recording(&self, app: &AppHandle) -> bool {
        let cancelled = {
            let mut state = self.state.lock().unwrap();
            if *state == RecordingState::Recording {
                let _ = self.audio_recorder.lock().unwrap().stop();
                *state = RecordingState::Ready;
                true
            } else {
                false
            }
        };
        if cancelled {
            publish_state(app, &RecordingState::Ready);
            log::info!("Recording cancelled, audio discarded");
        }
        cancelled
    }

    /// Stops the recording, transcribes it and pastes the result, or answers
    /// it when it is a quick ask question.
    /// The recorder always returns to `Ready`, whether this succeeds or fails.
    pub async fn stop_and_transcribe(
        &self,
        app: &AppHandle,
        settings: &Settings,
    ) -> Result<Outcome, String> {
        {
            let mut state = self.state.lock().unwrap();
            if *state != RecordingState::Recording {
                return Err("Not currently recording".to_string());
            }
            *state = RecordingState::Transcribing;
        }
        publish_state(app, &RecordingState::Transcribing);
        log::info!("State: Recording → Transcribing");

        let started = Instant::now();
        let result = self.process_recording(app, settings).await;

        {
            let mut state = self.state.lock().unwrap();
            *state = RecordingState::Ready;
        }
        publish_state(app, &RecordingState::Ready);
        transcribe_local::dictation_ended(app, settings);

        match &result {
            Ok(Outcome::Pasted(text)) => log::info!(
                "Dictation finished in {} ms ({} characters)",
                started.elapsed().as_millis(),
                text.chars().count()
            ),
            Ok(Outcome::Asked) => log::info!(
                "Quick ask question sent in {} ms",
                started.elapsed().as_millis()
            ),
            Err(e) => {
                log::error!(
                    "Dictation failed after {} ms: {}",
                    started.elapsed().as_millis(),
                    e
                );
                notify_error(app, e);
            }
        }
        log::info!("State: Transcribing → Ready");

        result
    }

    async fn process_recording(
        &self,
        app: &AppHandle,
        settings: &Settings,
    ) -> Result<Outcome, String> {
        // The guard is a temporary, released at the end of this statement
        let stop_result = self.audio_recorder.lock().unwrap().stop();
        let captured = stop_result?;

        if captured.duration_secs < MIN_RECORDING_SECS {
            return Err(format!(
                "Recording is too short ({:.2} s), nothing to transcribe",
                captured.duration_secs
            ));
        }
        if captured.peak <= 0.0 {
            return Err(
                "The microphone recorded pure silence. Check that the right input device is selected and not muted."
                    .to_string(),
            );
        }

        log::info!("Transcribing with engine '{}'", settings.engine);
        let raw_text = if settings.engine == Engine::Local {
            transcribe_local::transcribe_local(app, settings, &settings.local_model, captured.samples).await?
        } else {
            transcribe_cloud(app, settings, captured.samples).await?
        };
        log::debug!("Raw transcription: {:?}", raw_text);

        let mut text = cleanup_text(&raw_text);
        if text.is_empty() {
            log::warn!("Transcription is empty, nothing to paste");
            return Ok(Outcome::Pasted(text));
        }
        log::info!("Transcribed text: {}", text);

        // Something was selected, or the quick ask window is open: the
        // speech is a question for the model, not text to paste
        if let Some(question) = ask::claim(settings).await {
            ask::ask(app, settings, question, &text).await?;
            return Ok(Outcome::Asked);
        }

        // A failed post-processing never loses the dictation: the text is
        // pasted as transcribed and the error is shown afterwards
        let mut post_process_error = None;
        if settings.post_process.enabled {
            match postprocess::process(settings, &text).await {
                Ok(edited) => {
                    log::info!("Post-processed text: {}", edited);
                    text = edited;
                }
                Err(e) => {
                    log::error!("Post-processing failed, pasting the text as transcribed: {}", e);
                    post_process_error = Some(e);
                }
            }
        }

        // So the next dictation doesn't stick to this one
        if settings.append_space {
            text.push(' ');
        }

        paste_text(&text)?;
        log::info!("Text pasted into the active window");

        if let Some(e) = post_process_error {
            notify_error(
                app,
                &format!("Post-processing failed, the text was pasted as transcribed: {}", e),
            );
        }

        Ok(Outcome::Pasted(text))
    }
}

/// Sends the recording to the cloud engine. If that fails for any reason
/// and a local model is downloaded (and the fallback is on), the local model
/// transcribes it instead; the next dictation tries the cloud again.
async fn transcribe_cloud(app: &AppHandle, settings: &Settings, samples: Vec<f32>) -> Result<String, String> {
    let format = upload_format(settings);
    let (samples, encoded) = tauri::async_runtime::spawn_blocking(move || {
        let encoded = audio::encode(&samples, format);
        (samples, encoded)
    })
    .await
    .map_err(|e| format!("Audio encoding stopped unexpectedly: {}", e))?;

    let error = match transcribe(settings, &encoded?).await {
        Ok(text) => return Ok(text),
        Err(error) => error,
    };
    if !settings.fallback_local {
        return Err(error);
    }
    let Some(model) = transcribe_local::fallback_model(app, settings) else {
        return Err(error);
    };

    log::warn!(
        "{} failed ({}); transcribing with the local model {} instead",
        settings.engine.label(),
        error,
        model
    );
    match transcribe_local::transcribe_local(app, settings, model, samples).await {
        Ok(text) => {
            let notice = FallbackNotice {
                engine: settings.engine.label().to_string(),
                error,
            };
            if let Err(e) = app.emit("transcription-fallback", notice) {
                log::debug!("Failed to emit transcription-fallback: {}", e);
            }
            Ok(text)
        }
        Err(local_error) => Err(format!(
            "{}. The local model failed too: {}",
            error.trim_end_matches('.'),
            local_error
        )),
    }
}

async fn transcribe(settings: &Settings, audio: &EncodedAudio) -> Result<String, String> {
    let language = settings.language_hint();
    match settings.engine {
        Engine::Groq => {
            transcribe_groq::transcribe_groq(&settings.groq_api_key, &settings.groq_model, language, audio)
                .await
        }
        Engine::OpenAi => {
            transcribe_openai::transcribe_openai(
                &settings.openai_endpoint,
                &settings.openai_model,
                &settings.openai_api_key,
                language,
                audio,
            )
            .await
        }
        Engine::Polza => {
            transcribe_polza::transcribe_polza(
                &settings.polza_api_key,
                &settings.polza_model,
                &settings.polza_provider,
                language,
                audio,
            )
            .await
        }
        Engine::AssemblyAi => {
            transcribe_assemblyai::transcribe_assemblyai(
                &settings.assemblyai_api_key,
                &settings.assemblyai_model,
                language,
                audio,
            )
            .await
        }
        Engine::Local => Err("The local engine takes the samples, not a file".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_is_ready() {
        let recorder = Recorder::new();
        assert_eq!(recorder.get_state(), RecordingState::Ready);
    }
}
