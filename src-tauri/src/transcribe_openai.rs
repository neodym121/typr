//! OpenAI's transcriptions and any server that speaks the same API (a local
//! whisper.cpp or faster-whisper server, for one).

use crate::audio::{AudioFormat, EncodedAudio};
use crate::net;

const DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "whisper-1";

/// OpenAI itself takes FLAC; other OpenAI-compatible servers (whisper.cpp,
/// local Whisper servers) may read only WAV, so they get WAV.
pub fn format_for(endpoint: &str) -> AudioFormat {
    if transcriptions_url(endpoint).starts_with("https://api.openai.com/") {
        AudioFormat::Flac
    } else {
        AudioFormat::Wav
    }
}

/// `language` is an ISO 639-1 code; `None` lets the model detect it. An
/// empty key sends none: a local server may need no key at all.
pub async fn transcribe(
    endpoint: &str,
    model: &str,
    api_key: &str,
    language: Option<&str>,
    audio: &EncodedAudio,
) -> Result<String, String> {
    let url = transcriptions_url(endpoint);
    let model = net::non_empty(model).unwrap_or(DEFAULT_MODEL);
    net::post_transcription("OpenAI Compatible", &url, net::non_empty(api_key), model, language, audio).await
}

/// Accepts either a base URL (`https://host/v1`) or the full
/// `/audio/transcriptions` URL.
pub fn transcriptions_url(endpoint: &str) -> String {
    let base = if endpoint.trim().is_empty() {
        DEFAULT_ENDPOINT
    } else {
        endpoint.trim()
    };

    if base.ends_with("/audio/transcriptions") {
        base.to_string()
    } else {
        format!("{}/audio/transcriptions", base.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcriptions_url() {
        assert_eq!(
            transcriptions_url(""),
            "https://api.openai.com/v1/audio/transcriptions"
        );
        assert_eq!(
            transcriptions_url("https://host/v1/"),
            "https://host/v1/audio/transcriptions"
        );
        assert_eq!(
            transcriptions_url("https://host/v1/audio/transcriptions"),
            "https://host/v1/audio/transcriptions"
        );
    }

    #[test]
    fn test_only_openai_itself_gets_flac() {
        assert_eq!(format_for(""), AudioFormat::Flac);
        assert_eq!(format_for("https://api.openai.com/v1"), AudioFormat::Flac);
        assert_eq!(format_for("http://localhost:8080/v1"), AudioFormat::Wav);
    }
}
