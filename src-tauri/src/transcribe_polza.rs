use base64::Engine as _;
use serde_json::json;
use std::time::Instant;

use crate::audio::{AudioFormat, EncodedAudio};
use crate::net;

pub const HOST: &str = "https://polza.ai";
const POLZA_URL: &str = "https://polza.ai/api/v1/audio/transcriptions";
const DEFAULT_MODEL: &str = "openai/whisper-large-v3";
/// Polza accepts request bodies up to ~15 MB
const MAX_BODY_BYTES: usize = 14 * 1024 * 1024;
/// Polza documents WAV uploads only, so it keeps getting WAV
pub const FORMAT: AudioFormat = AudioFormat::Wav;

/// Transcribe audio via Polza.AI aggregator.
///
/// Polza's documented upload format for `/v1/audio/transcriptions` is a JSON
/// body whose `file` field is a base64 data URL (`data:audio/wav;base64,...`).
/// The `model@provider=...` alias syntax only works for chat/responses
/// endpoints, so a pinned sub-provider is sent as `provider.only` instead.
pub async fn transcribe_polza(
    api_key: &str,
    model: &str,
    provider: &str,
    language: Option<&str>,
    audio: &EncodedAudio,
) -> Result<String, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("Polza API key not set. Add it in the API keys section.".to_string());
    }

    let data_url = format!(
        "data:{};base64,{}",
        audio.format.mime(),
        base64::engine::general_purpose::STANDARD.encode(&audio.bytes)
    );
    if data_url.len() > MAX_BODY_BYTES {
        return Err(format!(
            "Recording is too long for Polza ({} KB of audio, the limit is about 10 MB)",
            audio.size_kb()
        ));
    }

    let model_name = if model.trim().is_empty() {
        DEFAULT_MODEL
    } else {
        model.trim()
    };

    let mut body = json!({
        "model": model_name,
        "file": data_url,
        "response_format": "json"
    });
    if let Some(language) = language {
        body["language"] = json!(language);
    }

    let provider = provider.trim();
    if !provider.is_empty() {
        body["provider"] = json!({ "only": [provider] });
    }

    log::info!(
        "Polza: POST {} (model {}, provider {}, language {}, {} KB)",
        POLZA_URL,
        model_name,
        if provider.is_empty() { "auto" } else { provider },
        language.unwrap_or("auto"),
        audio.size_kb()
    );
    let started = Instant::now();
    let response = net::client()?
        .post(POLZA_URL)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Polza API request failed: {}", net::describe_error(&e)))?;

    net::read_transcription("Polza", response, started).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_api_key() {
        let audio = EncodedAudio { bytes: Vec::new(), format: FORMAT };
        let result = transcribe_polza("", "", "", None, &audio).await;
        assert!(result.unwrap_err().contains("API key not set"));
    }
}
