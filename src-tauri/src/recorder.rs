//! One dictation from the hotkey to the paste: recording, transcription
//! (with the local fallback), quick ask or post-processing, pasting.

use std::sync::{Mutex, MutexGuard};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::audio::{self, AudioFormat, AudioRecorder, CapturedAudio, EncodedAudio};
use crate::cleanup::cleanup_text;
use crate::overlay::{self, Indicator};
use crate::settings::{Engine, Settings};
use crate::{
    ask, net, paste, postprocess, transcribe_assemblyai, transcribe_groq, transcribe_local, transcribe_polza,
    translate,
};

/// Shorter recordings are almost always accidental taps; Whisper tends to
/// hallucinate text for them.
const MIN_RECORDING_SECS: f32 = 0.3;

/// Where the dictation is; the window shows it as the status, the indicator
/// follows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub enum RecordingState {
    #[default]
    Ready,
    Recording,
    Transcribing,
}

fn publish_state(app: &AppHandle, state: RecordingState) {
    if let Err(e) = app.emit("recording-state", state) {
        log::warn!("Failed to emit recording-state: {}", e);
    }
    let indicator = match state {
        RecordingState::Ready => Indicator::Idle,
        RecordingState::Recording => Indicator::Recording,
        RecordingState::Transcribing => Indicator::Transcribing,
    };
    overlay::set_indicator(app, indicator);
    // An open quick ask window listens while the user asks a follow-up
    ask::recording_state(&state);
}

/// Shows a failed dictation in the main window (and in an open quick ask
/// window) and briefly flashes the overlay.
pub fn notify_error(app: &AppHandle, message: &str) {
    if let Err(e) = app.emit("recording-error", message.to_string()) {
        log::warn!("Failed to emit recording-error: {}", e);
    }
    ask::recording_error(message);
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
/// speaking: the cloud engine and, if they are on, post-processing (or
/// translation) and quick ask (or its window that is open).
pub fn warm_up_connections(settings: &Settings) {
    let engine_url = match settings.engine {
        Engine::Local => None,
        Engine::Groq => Some(transcribe_groq::HOST.to_string()),
        Engine::AssemblyAi => Some(transcribe_assemblyai::HOST.to_string()),
        Engine::Polza => Some(transcribe_polza::HOST.to_string()),
    };
    let post_url = settings
        .post_process
        .uses_model()
        .then(|| postprocess::host(settings.post_process.provider).to_string());
    let ask = &settings.quick_ask;
    let asking = ask.enabled || ask::is_open();
    let ask_url = asking.then(|| postprocess::host(ask.provider).to_string());
    let search_url = (asking && ask.web_search && !settings.api_keys.exa().is_empty())
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
        // Never packed: the local engine takes the samples
        Engine::Local => AudioFormat::Wav,
    }
}

/// Runs one dictation at a time: Ready → Recording → Transcribing → Ready.
#[derive(Default)]
pub struct Recorder {
    /// The state and the microphone change together, under one lock
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    state: RecordingState,
    microphone: AudioRecorder,
}

impl Recorder {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn state(&self) -> RecordingState {
        self.lock().state
    }

    /// Opens `microphone` and starts recording; only from `Ready`.
    pub fn start(&self, app: &AppHandle, microphone: &str) -> Result<(), String> {
        {
            let mut inner = self.lock();
            if inner.state != RecordingState::Ready {
                return Err(format!("Cannot start recording while {:?}", inner.state));
            }
            log::info!("Starting recording (microphone: '{}')", microphone);
            inner.microphone.start(microphone)?;
            inner.state = RecordingState::Recording;
        }
        publish_state(app, RecordingState::Recording);
        log::info!("State: Ready → Recording");
        Ok(())
    }

    /// Stops an unfinished recording and throws the audio away. Returns
    /// whether there was one.
    pub fn cancel(&self, app: &AppHandle) -> bool {
        {
            let mut inner = self.lock();
            if inner.state != RecordingState::Recording {
                return false;
            }
            let _ = inner.microphone.stop();
            inner.state = RecordingState::Ready;
        }
        publish_state(app, RecordingState::Ready);
        log::info!("Recording cancelled, audio discarded");
        true
    }

    /// Stops the recording, transcribes it and pastes the result, or answers
    /// it when it is a quick ask question.
    /// The recorder always returns to `Ready`, whether this succeeds or fails.
    pub async fn stop_and_transcribe(&self, app: &AppHandle, settings: &Settings) -> Result<Outcome, String> {
        let captured = {
            let mut inner = self.lock();
            if inner.state != RecordingState::Recording {
                return Err("Not currently recording".to_string());
            }
            inner.state = RecordingState::Transcribing;
            inner.microphone.stop()
        };
        publish_state(app, RecordingState::Transcribing);
        log::info!("State: Recording → Transcribing");

        let started = Instant::now();
        let result = match captured {
            Ok(captured) => process(app, settings, captured).await,
            Err(e) => Err(e),
        };

        self.lock().state = RecordingState::Ready;
        publish_state(app, RecordingState::Ready);
        transcribe_local::dictation_ended(app, settings);

        let elapsed = started.elapsed().as_millis();
        match &result {
            Ok(Outcome::Pasted(text)) => {
                log::info!("Dictation finished in {} ms ({} characters)", elapsed, text.chars().count())
            }
            Ok(Outcome::Asked) => log::info!("Quick ask question sent in {} ms", elapsed),
            Err(e) => {
                log::error!("Dictation failed after {} ms: {}", elapsed, e);
                notify_error(app, e);
            }
        }
        log::info!("State: Transcribing → Ready");
        result
    }
}

/// What happens to a finished recording: it is transcribed, then answered
/// as a question or (post-processed and) pasted.
async fn process(app: &AppHandle, settings: &Settings, captured: CapturedAudio) -> Result<Outcome, String> {
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
        transcribe_local::transcribe(app, settings, &settings.local_model, captured.samples).await?
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

    // A dictation already in the language it would be translated into
    // isn't translated; with post-processing off it isn't sent at all
    let config = &settings.post_process;
    let translate_to = config.translate.then_some(config.translate_to).filter(|&language| {
        let already = translate::is_written_in(&text, language);
        if already {
            log::info!("The text is already in {}, not translating it", language.name());
        }
        !already
    });

    // A failed post-processing never loses the dictation: the text is
    // pasted as transcribed and the error is shown afterwards
    let mut post_process_error = None;
    let step = if config.enabled { "Post-processing" } else { "Translation" };
    if config.enabled || translate_to.is_some() {
        match postprocess::process(settings, &text, translate_to).await {
            Ok(edited) => {
                log::info!("{} result: {}", step, edited);
                text = edited;
            }
            Err(e) => {
                log::error!("{} failed, pasting the text as transcribed: {}", step, e);
                post_process_error = Some(e);
            }
        }
    }

    // So the next dictation doesn't stick to this one
    if settings.append_space {
        text.push(' ');
    }

    paste::paste_text(&text)?;
    log::info!("Text pasted into the active window");

    if let Some(e) = post_process_error {
        notify_error(app, &format!("{} failed, the text was pasted as transcribed: {}", step, e));
    }

    Ok(Outcome::Pasted(text))
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
    match transcribe_local::transcribe(app, settings, model, samples).await {
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

/// Sends the packed recording to the chosen cloud engine.
async fn transcribe(settings: &Settings, audio: &EncodedAudio) -> Result<String, String> {
    let language = settings.language_hint();
    let keys = &settings.api_keys;
    match settings.engine {
        Engine::Groq => transcribe_groq::transcribe(&keys.groq, &settings.groq_model, language, audio).await,
        Engine::AssemblyAi => {
            transcribe_assemblyai::transcribe(&keys.assemblyai, &settings.assemblyai_model, language, audio).await
        }
        Engine::Polza => {
            transcribe_polza::transcribe(&keys.polza, &settings.polza_model, &settings.polza_provider, language, audio)
                .await
        }
        Engine::Local => Err("The local engine takes the samples, not a file".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Recorder, RecordingState};

    #[test]
    fn a_new_recorder_is_ready() {
        assert_eq!(Recorder::default().state(), RecordingState::Ready);
    }
}
