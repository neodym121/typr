//! Web search for quick ask, through Exa (`POST https://api.exa.ai/search`).
//! The model asks for it with the `web_search` tool when a question needs
//! facts it can't know; the results go back to it as text: title, address,
//! date and the passages of the page that match the query.

use serde_json::{json, Value};
use std::time::{Duration, Instant};

use crate::net;

pub const HOST: &str = "https://api.exa.ai";
const RESULTS: u32 = 5;
/// Matching passages per page; five pages stay a few thousand words
const PASSAGE_CHARS: u32 = 1500;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub published: Option<String>,
    pub text: String,
}

pub async fn search(api_key: &str, query: &str) -> Result<Vec<SearchResult>, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("Exa API key is not set".to_string());
    }
    let client = net::client()?;
    let started = Instant::now();
    let response = client
        .post(format!("{}/search", HOST))
        .header("x-api-key", api_key)
        .timeout(TIMEOUT)
        .json(&json!({
            "query": query,
            "type": "auto",
            "numResults": RESULTS,
            "contents": { "highlights": { "maxCharacters": PASSAGE_CHARS } },
        }))
        .send()
        .await
        .map_err(|e| format!("Exa request failed: {}", net::describe_error(&e)))?;
    let json = net::read_json("Exa", response).await?;
    let results = parse_results(&json);
    log::info!(
        "Exa found {} pages in {} ms",
        results.len(),
        started.elapsed().as_millis()
    );
    Ok(results)
}

fn parse_results(json: &Value) -> Vec<SearchResult> {
    json["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|result| {
            let url = result["url"].as_str()?.trim().to_string();
            let highlights: Vec<&str> = result["highlights"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect();
            let text = if highlights.is_empty() {
                result["text"].as_str().unwrap_or("").trim().to_string()
            } else {
                highlights.join("\n…\n")
            };
            Some(SearchResult {
                title: result["title"].as_str().unwrap_or("").trim().to_string(),
                url,
                published: result["publishedDate"]
                    .as_str()
                    .map(|date| date.chars().take(10).collect())
                    .filter(|date: &String| !date.is_empty()),
                text,
            })
        })
        .collect()
}

/// The results as the model reads them.
pub fn format_results(query: &str, results: &[SearchResult]) -> String {
    if results.is_empty() {
        return format!("Web search for \"{}\" found nothing.", query);
    }
    let mut out = format!("Web search results for \"{}\":\n", query);
    for (i, result) in results.iter().enumerate() {
        out.push_str(&format!("\n[{}] {}\nURL: {}\n", i + 1, result.title, result.url));
        if let Some(date) = &result.published {
            out.push_str(&format!("Published: {}\n", date));
        }
        if !result.text.is_empty() {
            out.push_str(&result.text);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_results_are_read_with_their_passages() {
        let json = json!({ "results": [
            { "title": "Rust 2.0 released", "url": "https://blog.rust-lang.org/x",
              "publishedDate": "2026-09-30T10:00:00.000Z",
              "highlights": ["Rust 2.0 is out.", "  ", "It brings…"] },
            { "title": "No passages", "url": "https://example.com", "text": "Full text" },
            { "title": "No address" }
        ]});
        let results = parse_results(&json);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].published.as_deref(), Some("2026-09-30"));
        assert_eq!(results[0].text, "Rust 2.0 is out.\n…\nIt brings…");
        assert_eq!(results[1].text, "Full text");
        assert_eq!(results[1].published, None);

        let text = format_results("rust 2.0", &results);
        assert!(text.contains("[1] Rust 2.0 released\nURL: https://blog.rust-lang.org/x\nPublished: 2026-09-30"));
        assert!(text.contains("[2] No passages"));
        assert_eq!(format_results("x", &[]), "Web search for \"x\" found nothing.");
    }

    #[tokio::test]
    async fn test_missing_key() {
        assert!(search(" ", "q").await.unwrap_err().contains("key is not set"));
    }
}
