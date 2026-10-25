//! Groq's hosted Whisper models.

use crate::audio::{AudioFormat, EncodedAudio};
use crate::net;

pub const HOST: &str = "https://api.groq.com";
const TRANSCRIPTIONS_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
/// Groq accepts FLAC, which uploads faster than WAV
pub const FORMAT: AudioFormat = AudioFormat::Flac;

/// `model` is "whisper-large-v3-turbo" (faster, also when empty) or
/// "whisper-large-v3". `language` is an ISO 639-1 code; `None` lets Whisper
/// detect it.
pub async fn transcribe(
    api_key: &str,
    model: &str,
    language: Option<&str>,
    audio: &EncodedAudio,
) -> Result<String, String> {
    let Some(api_key) = net::non_empty(api_key) else {
        return Err("Groq API key not set. Add it in the API keys section.".to_string());
    };
    let model = net::non_empty(model).unwrap_or(DEFAULT_MODEL);
    net::post_transcription("Groq", TRANSCRIPTIONS_URL, Some(api_key), model, language, audio).await
}

#[cfg(test)]
mod tests {
    use super::{transcribe, EncodedAudio, FORMAT};

    #[tokio::test]
    async fn nothing_is_sent_without_a_key() {
        let recording = EncodedAudio { bytes: Vec::new(), format: FORMAT };
        let error = transcribe(" ", "", None, &recording).await.unwrap_err();
        assert!(error.contains("API key not set"), "{}", error);
    }
}
