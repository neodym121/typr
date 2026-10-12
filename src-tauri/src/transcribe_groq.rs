use reqwest::multipart;
use std::time::Instant;

use crate::audio::{AudioFormat, EncodedAudio};
use crate::net;

pub const HOST: &str = "https://api.groq.com";
const GROQ_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
/// Groq accepts FLAC, which uploads faster than WAV
pub const FORMAT: AudioFormat = AudioFormat::Flac;

/// `model` is "whisper-large-v3-turbo" (faster) or "whisper-large-v3".
/// `language` is an ISO 639-1 code; `None` lets Whisper detect it.
pub async fn transcribe_groq(
    api_key: &str,
    model: &str,
    language: Option<&str>,
    audio: &EncodedAudio,
) -> Result<String, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("Groq API key not set. Add it in the API keys section.".to_string());
    }
    let model = match model.trim() {
        "" => DEFAULT_MODEL,
        other => other,
    };

    let file_part = multipart::Part::bytes(audio.bytes.clone())
        .file_name(audio.format.file_name())
        .mime_str(audio.format.mime())
        .map_err(|e| e.to_string())?;

    let mut form = multipart::Form::new()
        .text("model", model.to_string())
        .text("response_format", "json")
        .part("file", file_part);
    if let Some(language) = language {
        form = form.text("language", language.to_string());
    }

    log::info!(
        "Groq: POST {} (model {}, language {}, {} KB {:?})",
        GROQ_URL,
        model,
        language.unwrap_or("auto"),
        audio.size_kb(),
        audio.format
    );
    let started = Instant::now();
    let response = net::client()?
        .post(GROQ_URL)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Groq API request failed: {}", net::describe_error(&e)))?;

    net::read_transcription("Groq", response, started).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_api_key() {
        let audio = EncodedAudio { bytes: Vec::new(), format: FORMAT };
        let result = transcribe_groq("", "", None, &audio).await;
        assert!(result.unwrap_err().contains("API key not set"));
    }
}
