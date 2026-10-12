//! HTTP helpers shared by the transcription and post-processing clients.

use std::error::Error;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// An open connection waits this long for the next request
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const WARM_UP_TIMEOUT: Duration = Duration::from_secs(10);

/// One client for the whole app: its connections stay open between requests,
/// so a dictation doesn't pay for a new TLS handshake every time. Timeouts
/// make sure a stalled connection never hangs a dictation.
pub fn client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .pool_idle_timeout(IDLE_TIMEOUT)
                .user_agent(concat!("Typr/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {}", e))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// Opens a connection to the server of `url` in the background (a HEAD
/// request without credentials), so the request that follows finds it ready.
/// Called when a dictation starts: the handshake happens while the user speaks.
pub fn warm_up(url: String) {
    let Ok(client) = client() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        match client.head(&url).timeout(WARM_UP_TIMEOUT).send().await {
            Ok(_) => log::debug!(
                "Connection to {} ready in {} ms",
                url,
                started.elapsed().as_millis()
            ),
            Err(e) => log::debug!("Could not warm up {}: {}", url, describe_error(&e)),
        }
    });
}

/// reqwest's own message is just "error sending request"; this adds the
/// kind of failure and the underlying cause chain.
pub fn describe_error(err: &reqwest::Error) -> String {
    let mut message = if err.is_timeout() {
        "request timed out".to_string()
    } else if err.is_connect() {
        "could not connect to the server (check your internet connection, VPN or firewall)"
            .to_string()
    } else {
        err.to_string()
    };

    let mut source = err.source();
    while let Some(cause) = source {
        message.push_str(" → ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

/// Reads a Whisper-style JSON response (`{"text": "..."}`) and logs how it went.
pub async fn read_transcription(
    provider: &str,
    response: reqwest::Response,
    started: Instant,
) -> Result<String, String> {
    log::info!(
        "{} responded {} in {} ms",
        provider,
        response.status(),
        started.elapsed().as_millis()
    );

    let json = read_json(provider, response).await?;
    json["text"]
        .as_str()
        .map(|text| text.to_string())
        .ok_or_else(|| {
            format!(
                "No 'text' field in {} response: {}",
                provider,
                shorten(&json.to_string(), 300)
            )
        })
}

/// Reads a JSON response body; an HTTP error status becomes a readable error.
pub async fn read_json(
    provider: &str,
    response: reqwest::Response,
) -> Result<serde_json::Value, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("{} response could not be read: {}", provider, describe_error(&e)))?;

    if !status.is_success() {
        return Err(format!(
            "{} API error ({}): {}",
            provider,
            status,
            shorten(&body, 600)
        ));
    }

    serde_json::from_str(&body).map_err(|e| {
        format!(
            "Failed to parse {} response: {} (body: {})",
            provider,
            e,
            shorten(&body, 300)
        )
    })
}

pub(crate) fn shorten(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let cut: String = trimmed.chars().take(max_chars).collect();
        format!("{}…", cut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shorten() {
        assert_eq!(shorten("  short  ", 10), "short");
        assert_eq!(shorten("abcdef", 3), "abc…");
        assert_eq!(shorten("привет", 3), "при…");
    }
}
