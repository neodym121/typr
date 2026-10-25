//! Update check: compares this version with the latest release on GitHub
//! (drafts and pre-releases don't count). Nothing is downloaded; the release
//! page opens in the browser when the user asks for it.

use serde::Serialize;
use std::time::Duration;

use crate::net;

/// The repository releases come from
pub const RELEASES_PAGE: &str = "https://github.com/neodym121/typr/releases";
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/neodym121/typr/releases/latest";
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// A release newer than this build.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdateInfo {
    /// e.g. "2.1.0"
    pub version: String,
    /// The release page
    pub url: String,
}

/// `major.minor.patch` of "v2.0.1", "2.0" or "2.0.1-beta" (pre-release
/// suffixes are ignored).
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.trim().parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// The latest release if it is newer than this build, `None` if Typr is up
/// to date.
pub async fn check() -> Result<Option<UpdateInfo>, String> {
    let response = net::client()?
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .timeout(CHECK_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Update check failed: {}", net::describe_error(&e)))?;
    let release = net::read_json("GitHub", response).await?;

    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| "The latest release on GitHub has no tag".to_string())?;
    let current = env!("CARGO_PKG_VERSION");
    if !is_newer(tag, current) {
        log::info!("Typr is up to date (v{}, latest release {})", current, tag);
        return Ok(None);
    }
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let url = release["html_url"]
        .as_str()
        .filter(|url| url.starts_with(RELEASES_PAGE))
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/tag/{}", RELEASES_PAGE, tag));
    log::info!("Typr {} is available (this is v{})", version, current);
    Ok(Some(UpdateInfo { version, url }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("v2.0.1"), Some((2, 0, 1)));
        assert_eq!(parse_version("2.0"), Some((2, 0, 0)));
        assert_eq!(parse_version("v3.1.0-beta.2"), Some((3, 1, 0)));
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("v2.0.1", "2.0.0"));
        assert!(is_newer("v2.1.0", "2.0.9"));
        assert!(is_newer("v10.0.0", "9.9.9"));
        assert!(!is_newer("v2.0.0", "2.0.0"));
        assert!(!is_newer("v1.3.0", "2.0.0"));
        assert!(!is_newer("nonsense", "2.0.0"));
    }
}
