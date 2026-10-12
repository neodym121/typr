//! The quick ask's answer, streamed from the language model as it is
//! written: server-sent events from Gemini (`streamGenerateContent`) or an
//! OpenAI-compatible `/chat/completions` (OpenRouter, Groq, Polza).
//!
//! With web search on, the request offers the model one tool, `web_search`.
//! A round then ends either with the answer or with the searches the model
//! wants; the caller runs them, adds the exchange (`tool_exchange`) and asks
//! again.

use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::net;
use crate::postprocess::{openai_base, provider_label, GEMINI_URL};
use crate::settings::PostProvider;

/// A long answer may take a while; the shared client's 2 minutes are for
/// whole requests, not streams.
const STREAM_TIMEOUT: Duration = Duration::from_secs(300);
/// How often the sink gets a tick to pass on what it holds back
const TICK: Duration = Duration::from_millis(50);

pub const SEARCH_TOOL: &str = "web_search";
const SEARCH_TOOL_ABOUT: &str = "Search the web for current or specific information: news, recent events, prices, versions, facts about a page or a product. Returns the matching pages with their addresses and the relevant passages.";
const SEARCH_QUERY_ABOUT: &str = "The search query, in the language that suits the topic best";

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

/// Whether the request offers the search tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tools {
    /// No tools at all
    Off,
    /// The model may search
    Search,
    /// The tool is listed (earlier searches refer to it) but the model must
    /// answer now
    AnswerNow,
}

pub struct Request<'a> {
    pub provider: PostProvider,
    pub api_key: &'a str,
    pub model: &'a str,
    /// Polza's sub-provider; empty lets the service choose
    pub sub_provider: &'a str,
    pub system: &'a str,
    pub messages: &'a [ChatMessage],
    pub tools: Tools,
    /// This answer's earlier searches and their results, in the provider's
    /// format (`tool_exchange`), after the messages
    pub exchange: &'a [Value],
}

/// A search the model asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON arguments, e.g. {"query": "..."}
    pub arguments: String,
    /// Gemini: the part as it came, sent back unchanged (with its thought
    /// signature)
    pub raw: Value,
}

impl ToolCall {
    pub fn query(&self) -> Option<String> {
        let args: Value = serde_json::from_str(&self.arguments).ok()?;
        args["query"].as_str().map(str::trim).filter(|q| !q.is_empty()).map(str::to_string)
    }
}

/// What a round brought: the text written and the searches asked for.
#[derive(Debug, Default)]
pub struct Round {
    pub text: String,
    pub calls: Vec<ToolCall>,
}

/// Where the answer goes while it is written.
pub trait Sink {
    /// A piece of the answer (reasoning already left out)
    fn text(&mut self, text: &str);
    /// Called regularly while the answer is written, also between pieces
    fn tick(&mut self);
}

/// Streams one round into `sink`: the answer, or the searches the model
/// wants (with whatever text it wrote before them).
pub async fn stream(request: &Request<'_>, sink: &mut impl Sink) -> Result<Round, String> {
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
    let mut round = Round::default();
    // OpenAI-compatible tool calls arrive in pieces, by index
    let mut pieces: BTreeMap<u64, ToolCall> = BTreeMap::new();
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
                                round.text.push_str(&text);
                                sink.text(&text);
                            }
                        }
                        Event::CallPiece { index, id, name, arguments } => {
                            let call = pieces.entry(index).or_insert_with(|| ToolCall {
                                id: String::new(),
                                name: String::new(),
                                arguments: String::new(),
                                raw: Value::Null,
                            });
                            if let Some(id) = id {
                                call.id = id;
                            }
                            if let Some(name) = name {
                                call.name.push_str(&name);
                            }
                            call.arguments.push_str(&arguments);
                        }
                        Event::Call(call) => round.calls.push(call),
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
        round.text.push_str(&rest);
        sink.text(&rest);
    }
    round.calls.extend(pieces.into_values().filter(|call| !call.name.is_empty()).enumerate().map(
        |(i, mut call)| {
            if call.id.is_empty() {
                call.id = format!("call_{}", i);
            }
            call
        },
    ));
    if round.text.trim().is_empty() && round.calls.is_empty() {
        return Err(match events.finish_reason() {
            Some(reason) => format!("{} returned no answer (finish reason: {})", label, reason),
            None => format!("{} returned no answer", label),
        });
    }
    Ok(round)
}

/// Whether a failed request was refused because of the tool (the model or
/// the provider doesn't support tools): then the question goes again
/// without search.
pub fn tools_refused(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("api error")
        && (lower.contains("tool") || lower.contains("function call") || lower.contains("function_call"))
}

fn search_parameters(gemini: bool) -> Value {
    // Gemini's schema types are upper case
    let (object, string) = if gemini { ("OBJECT", "STRING") } else { ("object", "string") };
    json!({
        "type": object,
        "properties": { "query": { "type": string, "description": SEARCH_QUERY_ABOUT } },
        "required": ["query"],
    })
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
    messages.extend(request.exchange.iter().cloned());
    let mut body = json!({
        "model": request.model.trim(),
        "messages": messages,
        "stream": true,
    });
    if request.tools != Tools::Off {
        body["tools"] = json!([{
            "type": "function",
            "function": {
                "name": SEARCH_TOOL,
                "description": SEARCH_TOOL_ABOUT,
                "parameters": search_parameters(false),
            },
        }]);
        body["tool_choice"] = json!(if request.tools == Tools::Search { "auto" } else { "none" });
    }
    if !request.sub_provider.trim().is_empty() {
        body["provider"] = json!({ "only": [request.sub_provider.trim()] });
    }
    body
}

fn gemini_body(request: &Request) -> Value {
    let mut contents: Vec<Value> = request
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
    contents.extend(request.exchange.iter().cloned());
    let mut body = json!({
        "systemInstruction": { "parts": [{ "text": request.system }] },
        "contents": contents,
    });
    if request.tools != Tools::Off {
        body["tools"] = json!([{
            "functionDeclarations": [{
                "name": SEARCH_TOOL,
                "description": SEARCH_TOOL_ABOUT,
                "parameters": search_parameters(true),
            }],
        }]);
        body["toolConfig"] = json!({
            "functionCallingConfig": { "mode": if request.tools == Tools::Search { "AUTO" } else { "NONE" } },
        });
    }
    body
}

/// The model's searches and their results, to add to the next round's
/// request: an assistant turn with the calls, then one result per call.
pub fn tool_exchange(provider: PostProvider, text: &str, calls: &[ToolCall], results: &[String]) -> Vec<Value> {
    if openai_base(provider).is_none() {
        let mut parts: Vec<Value> = Vec::new();
        if !text.trim().is_empty() {
            parts.push(json!({ "text": text }));
        }
        parts.extend(calls.iter().map(|call| call.raw.clone()));
        let responses: Vec<Value> = calls
            .iter()
            .zip(results)
            .map(|(call, result)| {
                let mut response = json!({
                    "name": call.name,
                    "response": { "result": result },
                });
                // Newer models give each call an id the response must carry
                if let Some(id) = call.raw["functionCall"]["id"].as_str() {
                    response["id"] = json!(id);
                }
                json!({ "functionResponse": response })
            })
            .collect();
        return vec![
            json!({ "role": "model", "parts": parts }),
            json!({ "role": "user", "parts": responses }),
        ];
    }

    let mut exchange = vec![json!({
        "role": "assistant",
        "content": if text.trim().is_empty() { Value::Null } else { json!(text) },
        "tool_calls": calls.iter().map(|call| json!({
            "id": call.id,
            "type": "function",
            "function": { "name": call.name, "arguments": call.arguments },
        })).collect::<Vec<_>>(),
    })];
    exchange.extend(calls.iter().zip(results).map(|(call, result)| {
        json!({ "role": "tool", "tool_call_id": call.id, "content": result })
    }));
    exchange
}

// ── Server-sent events ──────────────────────────────────

#[derive(Debug, PartialEq)]
enum Event {
    Text(String),
    /// A piece of an OpenAI-compatible tool call
    CallPiece {
        index: u64,
        id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
    /// A whole tool call (Gemini sends them in one piece)
    Call(ToolCall),
    Done,
    Error(String),
}

/// Cuts the response into `data:` lines and reads the answer's text and
/// tool calls out of them. Bytes may split anywhere, even inside a character.
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
            events.extend(self.data(data.trim()));
        }
        events
    }

    fn data(&mut self, data: &str) -> Vec<Event> {
        if data == "[DONE]" {
            return vec![Event::Done];
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        if let Some(error) = json.get("error").filter(|e| !e.is_null()) {
            let message = error["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            return vec![Event::Error(message)];
        }
        if self.gemini {
            self.gemini_events(&json)
        } else {
            self.openai_events(&json)
        }
    }

    fn openai_events(&mut self, json: &Value) -> Vec<Event> {
        let choice = &json["choices"][0];
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        let mut events = Vec::new();
        let content = &choice["delta"]["content"];
        let text = match content.as_str() {
            Some(text) => text.to_string(),
            None => content
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|part| part["text"].as_str())
                .collect(),
        };
        if !text.is_empty() {
            events.push(Event::Text(text));
        }
        for (position, call) in choice["delta"]["tool_calls"].as_array().into_iter().flatten().enumerate() {
            events.push(Event::CallPiece {
                index: call["index"].as_u64().unwrap_or(position as u64),
                id: call["id"].as_str().filter(|id| !id.is_empty()).map(str::to_string),
                name: call["function"]["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string),
                arguments: call["function"]["arguments"].as_str().unwrap_or("").to_string(),
            });
        }
        events
    }

    fn gemini_events(&mut self, json: &Value) -> Vec<Event> {
        if let Some(reason) = json["promptFeedback"]["blockReason"].as_str() {
            return vec![Event::Error(format!("the request was blocked ({})", reason))];
        }
        let candidate = &json["candidates"][0];
        if let Some(reason) = candidate["finishReason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        let mut events = Vec::new();
        let mut text = String::new();
        for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
            if let Some(call) = part.get("functionCall") {
                events.push(Event::Call(ToolCall {
                    id: call["id"].as_str().unwrap_or("").to_string(),
                    name: call["name"].as_str().unwrap_or("").to_string(),
                    arguments: call["args"].to_string(),
                    raw: part.clone(),
                }));
            } else if part["thought"].as_bool() != Some(true) {
                // Thinking models also send their thoughts, as parts marked "thought"
                if let Some(piece) = part["text"].as_str() {
                    text.push_str(piece);
                }
            }
        }
        if !text.is_empty() {
            events.insert(0, Event::Text(text));
        }
        events
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
            tools: Tools::Off,
            exchange: &[],
        };
        let body = openai_body(&request);
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert_eq!(body["provider"]["only"][0], "openai");
        assert!(body.get("tools").is_none());

        let body = gemini_body(&request);
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
        assert!(body.get("tools").is_none());
    }

    fn call(id: &str, query: &str, raw: Value) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: SEARCH_TOOL.into(),
            arguments: json!({ "query": query }).to_string(),
            raw,
        }
    }

    #[test]
    fn test_the_search_tool_is_offered_and_can_be_closed() {
        let messages = vec![ChatMessage { role: Role::User, content: "q".into() }];
        let exchange = tool_exchange(PostProvider::Groq, "", &[call("c1", "rust", Value::Null)], &["results".into()]);
        let mut request = Request {
            provider: PostProvider::Groq,
            api_key: "k",
            model: "m",
            sub_provider: "",
            system: "sys",
            messages: &messages,
            tools: Tools::Search,
            exchange: &exchange,
        };
        let body = openai_body(&request);
        assert_eq!(body["tools"][0]["function"]["name"], SEARCH_TOOL);
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "c1");
        assert_eq!(body["messages"][2]["content"], Value::Null);
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "c1");
        request.tools = Tools::AnswerNow;
        assert_eq!(openai_body(&request)["tool_choice"], "none");

        request.provider = PostProvider::Gemini;
        request.tools = Tools::Search;
        let body = gemini_body(&request);
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["parameters"]["type"], "OBJECT");
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");
    }

    #[test]
    fn test_openai_tool_calls_come_in_pieces() {
        let mut parser = EventParser::new(false);
        let events = parser.push(concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"web_search\",\"arguments\":\"{\\\"que\"}}]}}]}
",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"ry\\\":\\\"rust 2.0\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}
",
        ).as_bytes());
        let mut arguments = String::new();
        for event in &events {
            if let Event::CallPiece { arguments: piece, .. } = event {
                arguments.push_str(piece);
            }
        }
        assert_eq!(arguments, r#"{"query":"rust 2.0"}"#);
        assert!(matches!(&events[0], Event::CallPiece { index: 0, id: Some(id), name: Some(name), .. } if id == "call_a" && name == "web_search"));
        assert_eq!(parser.finish_reason(), Some("tool_calls"));
    }

    #[test]
    fn test_gemini_tool_call_is_kept_whole() {
        let mut parser = EventParser::new(true);
        let events = parser.push(
            br#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"web_search","args":{"query":"rust"}},"thoughtSignature":"abc"}]}}]}
"#,
        );
        let Event::Call(call) = &events[0] else {
            panic!("no call: {:?}", events);
        };
        assert_eq!(call.query().as_deref(), Some("rust"));
        assert_eq!(call.raw["thoughtSignature"], "abc");

        // Sent back unchanged, signature included, with the result after it
        let exchange = tool_exchange(PostProvider::Gemini, "", std::slice::from_ref(call), &["found".into()]);
        assert_eq!(exchange[0]["role"], "model");
        assert_eq!(exchange[0]["parts"][0]["thoughtSignature"], "abc");
        assert_eq!(exchange[1]["parts"][0]["functionResponse"]["name"], "web_search");
        assert_eq!(exchange[1]["parts"][0]["functionResponse"]["response"]["result"], "found");
    }

    #[test]
    fn test_a_refused_tool_is_recognised() {
        assert!(tools_refused("Groq API error (400 Bad Request): model does not support tool use"));
        assert!(tools_refused("Polza API error (404): No endpoints found that support tool use"));
        assert!(!tools_refused("Groq API error (401 Unauthorized): invalid api key"));
        assert!(!tools_refused("Groq request failed: timed out"));
    }
}
