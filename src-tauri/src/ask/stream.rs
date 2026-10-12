//! The quick ask's answer, streamed from the language model as it is
//! written: server-sent events from Gemini (`streamGenerateContent`) or an
//! OpenAI-compatible `/chat/completions` (OpenRouter, Groq, Polza).

use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;

use crate::net;
use crate::postprocess::{openai_base, provider_label, GEMINI_URL};
use crate::settings::PostProvider;

/// A long answer may take a while; the shared client's 2 minutes are for
/// whole requests, not streams.
const STREAM_TIMEOUT: Duration = Duration::from_secs(300);
/// How often the sink gets a tick to pass on what it holds back
const TICK: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

pub struct Request<'a> {
    pub provider: PostProvider,
    pub api_key: &'a str,
    pub model: &'a str,
    /// Polza's sub-provider; empty lets the service choose
    pub sub_provider: &'a str,
    pub system: &'a str,
    pub messages: &'a [ChatMessage],
}

/// Where the answer goes while it is written.
pub trait Sink {
    /// A piece of the answer (reasoning already left out)
    fn text(&mut self, text: &str);
    /// Called regularly while the answer is written, also between pieces
    fn tick(&mut self);
}

/// Streams the answer into `sink` and returns all of it.
pub async fn stream(request: &Request<'_>, sink: &mut impl Sink) -> Result<String, String> {
    let label = provider_label(request.provider);
    if request.api_key.trim().is_empty() {
        return Err(format!("{} API key for quick ask is not set", label));
    }
    if request.model.trim().is_empty() {
        return Err(format!("No {} model is chosen for quick ask", label));
    }

    let client = net::client()?;
    let gemini = openai_base(request.provider).is_none();
    let builder = match openai_base(request.provider) {
        Some(base) => client
            .post(format!("{}/chat/completions", base))
            .bearer_auth(request.api_key.trim())
            .header("X-Title", "Typr")
            .json(&openai_body(request)),
        None => client
            .post(format!(
                "{}/models/{}:streamGenerateContent?alt=sse",
                GEMINI_URL,
                request.model.trim().trim_start_matches("models/")
            ))
            .header("x-goog-api-key", request.api_key.trim())
            .json(&gemini_body(request)),
    };

    let response = builder
        .timeout(STREAM_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("{} request failed: {}", label, net::describe_error(&e)))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("{} API error ({}): {}", label, status, net::shorten(&body, 600)));
    }

    let mut events = EventParser::new(gemini);
    let mut filter = ThinkFilter::default();
    let mut answer = String::new();
    let mut body = response.bytes_stream();
    let mut tick = tokio::time::interval(TICK);

    'read: loop {
        tokio::select! {
            next = body.next() => {
                let bytes = match next {
                    Some(Ok(bytes)) => bytes,
                    Some(Err(e)) => {
                        return Err(format!("{} stopped answering: {}", label, net::describe_error(&e)));
                    }
                    None => break 'read,
                };
                for event in events.push(&bytes) {
                    match event {
                        Event::Text(text) => {
                            let text = filter.push(&text);
                            if !text.is_empty() {
                                answer.push_str(&text);
                                sink.text(&text);
                            }
                        }
                        Event::Done => break 'read,
                        Event::Error(message) => return Err(format!("{}: {}", label, message)),
                    }
                }
            }
            _ = tick.tick() => sink.tick(),
        }
    }

    let rest = filter.finish();
    if !rest.is_empty() {
        answer.push_str(&rest);
        sink.text(&rest);
    }
    if answer.trim().is_empty() {
        return Err(match events.finish_reason() {
            Some(reason) => format!("{} returned no answer (finish reason: {})", label, reason),
            None => format!("{} returned no answer", label),
        });
    }
    Ok(answer)
}

fn openai_body(request: &Request) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    messages.extend(request.messages.iter().map(|message| {
        json!({
            "role": match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            },
            "content": message.content,
        })
    }));
    let mut body = json!({
        "model": request.model.trim(),
        "messages": messages,
        "stream": true,
    });
    if !request.sub_provider.trim().is_empty() {
        body["provider"] = json!({ "only": [request.sub_provider.trim()] });
    }
    body
}

fn gemini_body(request: &Request) -> Value {
    let contents: Vec<Value> = request
        .messages
        .iter()
        .map(|message| {
            json!({
                "role": match message.role {
                    Role::User => "user",
                    Role::Assistant => "model",
                },
                "parts": [{ "text": message.content }],
            })
        })
        .collect();
    json!({
        "systemInstruction": { "parts": [{ "text": request.system }] },
        "contents": contents,
    })
}

// ── Server-sent events ──────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Text(String),
    Done,
    Error(String),
}

/// Cuts the response into `data:` lines and reads the answer's text out of
/// them. Bytes may split anywhere, even inside a character.
struct EventParser {
    gemini: bool,
    buffer: Vec<u8>,
    finish_reason: Option<String>,
}

impl EventParser {
    fn new(gemini: bool) -> Self {
        Self {
            gemini,
            buffer: Vec::new(),
            finish_reason: None,
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=newline).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            // Other fields (event:, id:) and comments (": OPENROUTER PROCESSING")
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            if let Some(event) = self.data(data.trim()) {
                events.push(event);
            }
        }
        events
    }

    fn data(&mut self, data: &str) -> Option<Event> {
        if data == "[DONE]" {
            return Some(Event::Done);
        }
        let json: Value = serde_json::from_str(data).ok()?;
        if let Some(error) = json.get("error").filter(|e| !e.is_null()) {
            let message = error["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            return Some(Event::Error(message));
        }
        if self.gemini {
            self.gemini_text(&json)
        } else {
            self.openai_text(&json)
        }
    }

    fn openai_text(&mut self, json: &Value) -> Option<Event> {
        let choice = &json["choices"][0];
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        let content = &choice["delta"]["content"];
        let text = match content.as_str() {
            Some(text) => text.to_string(),
            None => content
                .as_array()?
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect(),
        };
        (!text.is_empty()).then_some(Event::Text(text))
    }

    fn gemini_text(&mut self, json: &Value) -> Option<Event> {
        if let Some(reason) = json["promptFeedback"]["blockReason"].as_str() {
            return Some(Event::Error(format!("the request was blocked ({})", reason)));
        }
        let candidate = &json["candidates"][0];
        if let Some(reason) = candidate["finishReason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        // Thinking models also send their thoughts, as parts marked "thought"
        let text: String = candidate["content"]["parts"]
            .as_array()?
            .iter()
            .filter(|part| part["thought"].as_bool() != Some(true))
            .filter_map(|part| part["text"].as_str())
            .collect();
        (!text.is_empty()).then_some(Event::Text(text))
    }

    fn finish_reason(&self) -> Option<&str> {
        self.finish_reason.as_deref()
    }
}

// ── Reasoning ───────────────────────────────────────────

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// Leaves out what reasoning models write between <think> and </think> in
/// the answer itself, and the blank lines before the answer starts. A tag
/// split between two pieces is held back until it is complete.
#[derive(Default)]
pub struct ThinkFilter {
    inside: bool,
    held: String,
    started: bool,
}

impl ThinkFilter {
    pub fn push(&mut self, text: &str) -> String {
        self.held.push_str(text);
        let mut out = String::new();
        loop {
            if self.inside {
                if let Some(end) = self.held.find(THINK_CLOSE) {
                    self.held.drain(..end + THINK_CLOSE.len());
                    self.inside = false;
                    continue;
                }
                let keep = partial_tag(&self.held, THINK_CLOSE);
                self.held.drain(..self.held.len() - keep);
            } else {
                if let Some(start) = self.held.find(THINK_OPEN) {
                    out.push_str(&self.held[..start]);
                    self.held.drain(..start + THINK_OPEN.len());
                    self.inside = true;
                    continue;
                }
                let keep = partial_tag(&self.held, THINK_OPEN);
                let cut = self.held.len() - keep;
                out.push_str(&self.held[..cut]);
                self.held.drain(..cut);
            }
            break;
        }
        self.trim_start(out)
    }

    /// What is still held back once the answer is complete.
    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.held);
        if self.inside {
            return String::new();
        }
        self.trim_start(rest)
    }

    fn trim_start(&mut self, out: String) -> String {
        if self.started {
            return out;
        }
        let trimmed = out.trim_start();
        if trimmed.is_empty() {
            return String::new();
        }
        self.started = true;
        trimmed.to_string()
    }
}

/// Length of the longest end of `text` that could be the start of `tag`.
fn partial_tag(text: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&len| text.ends_with(&tag[..len]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(events: Vec<Event>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|event| match event {
                Event::Text(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_openai_events_split_anywhere() {
        let mut parser = EventParser::new(false);
        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"При\"}}]}\n\n: OPENROUTER PROCESSING\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"вет\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\n\n";
        let bytes = stream.as_bytes();
        // Split inside a line, in the middle of a two-byte Cyrillic letter
        let (a, b) = bytes.split_at(stream.find("При").unwrap() + 1);
        let mut events = parser.push(a);
        events.extend(parser.push(b));
        assert_eq!(events.last(), Some(&Event::Done));
        assert_eq!(texts(events).concat(), "Привет");
        assert_eq!(parser.finish_reason(), Some("stop"));
    }

    #[test]
    fn test_gemini_events_skip_thoughts() {
        let mut parser = EventParser::new(true);
        let events = parser.push(
            b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hmm\",\"thought\":true},{\"text\":\"Answer\"}]}}]}\n\n",
        );
        assert_eq!(texts(events), vec!["Answer"]);
        let blocked = parser.push(b"data: {\"promptFeedback\":{\"blockReason\":\"SAFETY\"}}\n\n");
        assert_eq!(blocked, vec![Event::Error("the request was blocked (SAFETY)".to_string())]);
    }

    #[test]
    fn test_errors_in_the_stream() {
        let mut parser = EventParser::new(false);
        let events = parser.push(b"data: {\"error\":{\"message\":\"Rate limit exceeded\",\"code\":429}}\n");
        assert_eq!(events, vec![Event::Error("Rate limit exceeded".to_string())]);
    }

    #[test]
    fn test_think_blocks_are_left_out_even_when_split() {
        let mut filter = ThinkFilter::default();
        let mut out = String::new();
        for piece in ["<thi", "nk>let me", " think</th", "ink>\n\nThe ", "answer <b", "> is 42"] {
            out.push_str(&filter.push(piece));
        }
        out.push_str(&filter.finish());
        assert_eq!(out, "The answer <b> is 42");
    }

    #[test]
    fn test_an_unfinished_think_block_shows_nothing() {
        let mut filter = ThinkFilter::default();
        assert_eq!(filter.push("<think>still thinking"), "");
        assert_eq!(filter.finish(), "");
    }

    #[test]
    fn test_request_bodies() {
        let messages = vec![
            ChatMessage { role: Role::User, content: "q1".into() },
            ChatMessage { role: Role::Assistant, content: "a1".into() },
            ChatMessage { role: Role::User, content: "q2".into() },
        ];
        let request = Request {
            provider: PostProvider::Polza,
            api_key: "k",
            model: "openai/gpt-5-mini",
            sub_provider: "openai",
            system: "sys",
            messages: &messages,
        };
        let body = openai_body(&request);
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert_eq!(body["provider"]["only"][0], "openai");

        let body = gemini_body(&request);
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
    }
}
