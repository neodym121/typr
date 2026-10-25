//! AssemblyAI client. Unlike the Whisper-style APIs it works in three steps:
//! upload the WAV, create a transcript job, then poll the job until it is done.

use serde_json::json;
use std::time::{Duration, Instant};

use crate::audio::{AudioFormat, EncodedAudio};
use crate::net;

pub const HOST: &str = "https://api.assemblyai.com";
/// AssemblyAI reads FLAC, which uploads faster than WAV
pub const FORMAT: AudioFormat = AudioFormat::Flac;
const UPLOAD_URL: &str = "https://api.assemblyai.com/v2/upload";
const TRANSCRIPT_URL: &str = "https://api.assemblyai.com/v2/transcript";
pub const DEFAULT_MODEL: &str = "universal-3-5-pro";
/// Universal-3.5 Pro covers 18 languages, Russian is not one of them
const PRO_MODEL: &str = "universal-3-5-pro";
const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// Gives up on a job that is still unfinished after this long
const MAX_WAIT: Duration = Duration::from_secs(180);

/// `language` is an ISO 639-1 code; `None` turns on language detection.
pub async fn transcribe(
    api_key: &str,
    model: &str,
    language: Option<&str>,
    audio: &EncodedAudio,
) -> Result<String, String> {
    let Some(api_key) = net::non_empty(api_key) else {
        return Err("AssemblyAI API key not set. Add it in the API keys section.".to_string());
    };
    let model = net::non_empty(model).unwrap_or(DEFAULT_MODEL);

    let client = net::client()?;
    let started = Instant::now();

    log::info!("AssemblyAI: POST {} ({} KB {:?})", UPLOAD_URL, audio.size_kb(), audio.format);
    let response = client
        .post(UPLOAD_URL)
        .header("authorization", api_key)
        .header("content-type", "application/octet-stream")
        .body(audio.bytes.clone())
        .send()
        .await
        .map_err(|e| format!("AssemblyAI upload failed: {}", net::describe_error(&e)))?;
    let uploaded = net::read_json("AssemblyAI", response).await?;
    let audio_url = uploaded["upload_url"]
        .as_str()
        .ok_or_else(|| "No 'upload_url' in AssemblyAI upload response".to_string())?
        .to_string();
    log::info!("AssemblyAI: audio uploaded in {} ms", started.elapsed().as_millis());

    let mut body = json!({
        "audio_url": audio_url,
        "speech_models": [model],
        "punctuate": true,
        "format_text": true
    });
    match language {
        Some(language) => body["language_code"] = json!(language),
        // Detection, so Russian and other languages aren't forced into English
        None => body["language_detection"] = json!(true),
    }
    log::info!(
        "AssemblyAI: POST {} (model {}, language {})",
        TRANSCRIPT_URL,
        model,
        language.unwrap_or("auto")
    );
    let response = client
        .post(TRANSCRIPT_URL)
        .header("authorization", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AssemblyAI request failed: {}", net::describe_error(&e)))?;
    let mut transcript = net::read_json("AssemblyAI", response).await?;
    let id = transcript["id"]
        .as_str()
        .ok_or_else(|| "No transcript 'id' in AssemblyAI response".to_string())?
        .to_string();
    log::debug!("AssemblyAI: transcript {} created", id);

    loop {
        match transcript["status"].as_str().unwrap_or("") {
            "completed" => {
                log::info!(
                    "AssemblyAI finished in {} ms (model {}, language {})",
                    started.elapsed().as_millis(),
                    transcript["speech_model_used"].as_str().unwrap_or(model),
                    transcript["language_code"].as_str().unwrap_or("unknown")
                );
                return Ok(transcript["text"].as_str().unwrap_or("").to_string());
            }
            "error" => {
                let reason = transcript["error"].as_str().unwrap_or("unknown error");
                return Err(describe_failure(model, reason));
            }
            _ => {}
        }

        if started.elapsed() > MAX_WAIT {
            return Err(format!(
                "AssemblyAI did not finish the transcript within {} s",
                MAX_WAIT.as_secs()
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;

        let response = client
            .get(format!("{}/{}", TRANSCRIPT_URL, id))
            .header("authorization", api_key)
            .send()
            .await
            .map_err(|e| format!("AssemblyAI request failed: {}", net::describe_error(&e)))?;
        transcript = net::read_json("AssemblyAI", response).await?;
    }
}

/// Adds a hint when Universal-3.5 Pro rejects a language it doesn't support.
fn describe_failure(model: &str, reason: &str) -> String {
    let mut message = format!("AssemblyAI could not transcribe the audio: {}", reason);
    if model == PRO_MODEL && reason.to_lowercase().contains("language") {
        message.push_str(
            " (Universal-3.5 Pro doesn't support Russian; choose Universal-2 in Engine)",
        );
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nothing_is_sent_without_a_key() {
        let recording = EncodedAudio { bytes: Vec::new(), format: FORMAT };
        let error = transcribe("", "", None, &recording).await.unwrap_err();
        assert!(error.contains("API key not set"), "{}", error);
    }

    #[test]
    fn test_language_hint_only_for_pro_model() {
        let reason = "language_code ru is not supported";
        assert!(describe_failure(PRO_MODEL, reason).contains("Universal-2"));
        assert!(!describe_failure("universal-2", reason).contains("choose Universal-2"));
    }
}
