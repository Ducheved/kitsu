//! Talking to a model, streamed, in one of three wire formats:
//! OpenAI-compatible Chat Completions (OpenRouter and anything that speaks
//! the same API), Anthropic Messages (`anthropic.rs`) and OpenAI Responses
//! (`responses.rs`).
//!
//! The conversation (`Msg`) is the same for all three; each provider only
//! renders it into its request body and reads its stream back into a
//! `Reply`. What a format needs carried back that the conversation can't
//! express (Responses' reasoning items) rides in `Reply::replay`, which the
//! brain journals with the reply and each later request renders verbatim.
//!
//! Streaming isn't for showing tokens as they come: idle proxies cut long
//! non-streamed requests, and a cancel should drop the request right away.
//! Only the complete response is returned; the brain journals it whole.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;

use super::anthropic::Messages;
use super::responses::Responses;
use crate::agents::NativeSpec;

/// An API key. Never printed, never serialized; attached to one header.
pub struct Secret(String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

impl Secret {
    /// The `authorization` header's value.
    pub fn bearer(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    System(String),
    User(String),
    Assistant {
        text: Option<String>,
        calls: Vec<ToolCall>,
        /// What the provider that wrote this reply asked to get back
        /// verbatim (see `Reply::replay`). Other providers ignore it.
        replay: Option<Value>,
    },
    Tool {
        call_id: String,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The provider's id, echoed back with the result.
    pub id: String,
    pub name: String,
    /// The raw arguments string, exactly as the model produced it.
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
    pub text: Option<String>,
    pub calls: Vec<ToolCall>,
    pub finish: Option<String>,
    pub usage: ReplyUsage,
    /// `{"provider": <name>, ...}`: provider state later requests must
    /// carry (Responses' encrypted reasoning). Journaled with the reply, so
    /// a resumed run sends the same input.
    pub replay: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReplyUsage {
    /// Every input token, read from the cache or not.
    pub prompt: Option<u64>,
    pub completion: Option<u64>,
    /// Input tokens read from the provider's cache.
    pub cached: Option<u64>,
    /// Input tokens written to it (Anthropic's cache creation).
    pub cache_write: Option<u64>,
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderError {
    /// The request didn't fit the model's context.
    Overflow(String),
    RateLimited {
        retry_after: Option<Duration>,
        detail: String,
    },
    /// Worth retrying: 5xx, connection trouble, a stream cut short.
    Transient(String),
    /// Not worth retrying: bad key, bad request, unknown model.
    Fatal(String),
    Cancelled,
}

impl ProviderError {
    pub fn class(&self) -> &'static str {
        match self {
            ProviderError::Overflow(_) => "overflow",
            ProviderError::RateLimited { .. } => "rate_limited",
            ProviderError::Transient(_) => "transient",
            ProviderError::Fatal(_) => "fatal",
            ProviderError::Cancelled => "cancelled",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            ProviderError::Overflow(d)
            | ProviderError::Transient(d)
            | ProviderError::Fatal(d)
            | ProviderError::RateLimited { detail: d, .. } => d,
            ProviderError::Cancelled => "cancelled",
        }
    }
}

/// What `NativeSpec::provider` may name.
pub const PROVIDERS: &[&str] = &["openai-chat", "anthropic-messages", "openai-responses"];

pub enum Provider {
    Chat(OpenAiChat),
    Messages(Messages),
    Responses(Responses),
}

impl Provider {
    /// Finds the key (in the environment variable or the keychain entry the
    /// spec names) and builds the client. A missing key is an error here,
    /// before any request.
    pub fn new(spec: &NativeSpec) -> Result<Provider, String> {
        let key = key(spec)?;
        let http = client()?;
        let base = spec.base_url.trim_end_matches('/');
        let model = spec.model.clone();
        let max_output = spec.max_output;
        Ok(match spec.provider.as_str() {
            // Each URL follows its vendor's convention for a base URL
            // (ANTHROPIC_BASE_URL has no version, OPENAI_BASE_URL has one).
            "anthropic-messages" => Provider::Messages(Messages {
                http,
                url: format!("{base}/v1/messages"),
                model,
                max_output,
                key,
            }),
            "openai-responses" => Provider::Responses(Responses {
                http,
                url: format!("{base}/responses"),
                model,
                max_output,
                key,
            }),
            _ => Provider::Chat(OpenAiChat {
                http,
                url: format!("{base}/chat/completions"),
                cache: marks_cache(&model),
                model,
                max_output,
                key,
            }),
        })
    }

    /// The request body, without the key. Also what `request_sha` hashes.
    /// `tools` come in Chat Completions' shape (`tools::definitions`).
    pub fn body(&self, messages: &[Msg], tools: &Value) -> Value {
        match self {
            Provider::Chat(p) => p.body(messages, tools),
            Provider::Messages(p) => p.body(messages, tools),
            Provider::Responses(p) => p.body(messages, tools),
        }
    }

    pub async fn complete(
        &self,
        body: &Value,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Reply, ProviderError> {
        if *cancel.borrow() {
            return Err(ProviderError::Cancelled);
        }
        let send = async {
            match self {
                Provider::Chat(p) => p.send(body).await,
                Provider::Messages(p) => p.send(body).await,
                Provider::Responses(p) => p.send(body).await,
            }
        };
        tokio::select! {
            r = send => r,
            _ = cancel.wait_for(|c| *c) => Err(ProviderError::Cancelled),
        }
    }
}

fn key(spec: &NativeSpec) -> Result<Secret, String> {
    if let Some(var) = &spec.api_key_env {
        return std::env::var(var)
            .ok()
            .filter(|k| !k.trim().is_empty())
            .map(Secret)
            .ok_or_else(|| format!("${var} is not set; the agent needs its API key there"));
    }
    match spec.auth.as_deref().and_then(|a| a.strip_prefix("login:")) {
        Some(provider) => super::login::stored(provider).map(Secret),
        None => Err("the agent has no key source (api_key_env or auth)".into()),
    }
}

/// The HTTP client for model requests and for `kitsu login`.
pub fn client() -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .timeout(Duration::from_secs(15 * 60))
        .user_agent(concat!("kitsu/", env!("CARGO_PKG_VERSION")));
    // Extra roots (a proxy's CA, a company CA): loaded explicitly, and a
    // file that is named but unreadable is an error, not a fallback.
    if let Some(file) = std::env::var_os("SSL_CERT_FILE").filter(|f| !f.is_empty()) {
        let pem = std::fs::read(&file)
            .map_err(|e| format!("SSL_CERT_FILE {}: {e}", file.to_string_lossy()))?;
        for c in reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|e| format!("SSL_CERT_FILE {}: {e}", file.to_string_lossy()))?
        {
            b = b.add_root_certificate(c);
        }
    }
    b.build().map_err(|e| format!("http client: {e}"))
}

/// Sends a streamed request and hands each event's data to `on` until `on`
/// says the response is complete (`Ok(true)`) or the stream ends. HTTP
/// errors come back classified.
pub(super) async fn stream(
    req: reqwest::RequestBuilder,
    body: &Value,
    mut on: impl FnMut(&str) -> Result<bool, ProviderError>,
) -> Result<(), ProviderError> {
    let resp = req
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| ProviderError::Transient(format!("request failed: {}", without_url(&e))))?;
    let status = resp.status();
    if !status.is_success() {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let text = resp.text().await.unwrap_or_default();
        return Err(classify(status.as_u16(), &text, retry_after));
    }
    let mut sse = Sse::default();
    let mut resp = resp;
    loop {
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                for data in sse.push(&bytes) {
                    if on(&data)? {
                        return Ok(());
                    }
                }
            }
            Ok(None) => return Ok(()),
            Err(e) => {
                return Err(ProviderError::Transient(format!(
                    "stream cut: {}",
                    without_url(&e)
                )));
            }
        }
    }
}

pub(super) fn parse_event(data: &str) -> Result<Value, ProviderError> {
    serde_json::from_str(data)
        .map_err(|e| ProviderError::Transient(format!("bad stream chunk: {e}")))
}

/// An error that arrived inside a stream, by the HTTP status it stands for
/// (Anthropic's `overloaded_error` is a 529 outside a stream), so it's
/// classified the same way.
pub(super) fn classify_in_stream(status: u16, message: &str) -> ProviderError {
    classify(
        status,
        &json!({ "error": { "message": message } }).to_string(),
        None,
    )
}

/// Chat Completions' tool list as `(name, description, parameters)`, for
/// the formats that shape it differently.
pub(super) fn tool_parts(tools: &Value) -> impl Iterator<Item = (&Value, &Value, &Value)> {
    tools.as_array().into_iter().flatten().map(|t| {
        let f = &t["function"];
        (&f["name"], &f["description"], &f["parameters"])
    })
}

pub struct OpenAiChat {
    http: reqwest::Client,
    url: String,
    model: String,
    max_output: u64,
    /// Mark cache breakpoints in the request (see `mark_cache`).
    cache: bool,
    key: Secret,
}

impl OpenAiChat {
    /// The request body, without the key. Also what `request_sha` hashes.
    pub fn body(&self, messages: &[Msg], tools: &Value) -> Value {
        let mut msgs: Vec<Value> = messages.iter().map(to_openai).collect();
        if self.cache {
            mark_cache(&mut msgs);
        }
        json!({
            "model": self.model,
            "messages": msgs,
            "tools": tools,
            "tool_choice": "auto",
            "max_tokens": self.max_output,
            "stream": true,
            "stream_options": { "include_usage": true },
        })
    }

    async fn send(&self, body: &Value) -> Result<Reply, ProviderError> {
        let req = self
            .http
            .post(&self.url)
            .header("authorization", self.key.bearer())
            .header("x-title", "Kitsu");
        let mut acc = Accumulator::default();
        stream(req, body, |data| {
            if data == "[DONE]" {
                return Ok(true);
            }
            let v = parse_event(data)?;
            if let Some(err) = v.get("error") {
                return Err(classify(400, &json!({ "error": err }).to_string(), None));
            }
            acc.chunk(&v);
            Ok(false)
        })
        .await?;
        acc.finish()
    }
}

fn without_url(e: &reqwest::Error) -> String {
    // The URL has no secret, but errors are recorded; keep them short.
    let mut s = e.to_string();
    if let Some(u) = e.url() {
        s = s.replace(u.as_str(), "(the model endpoint)");
    }
    s
}

fn to_openai(m: &Msg) -> Value {
    match m {
        Msg::System(t) => json!({ "role": "system", "content": t }),
        Msg::User(t) => json!({ "role": "user", "content": t }),
        Msg::Assistant { text, calls, .. } => {
            let mut v = json!({ "role": "assistant", "content": text });
            if !calls.is_empty() {
                v["tool_calls"] = calls
                    .iter()
                    .map(|c| json!({ "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.arguments } }))
                    .collect();
            }
            v
        }
        Msg::Tool { call_id, text } => {
            json!({ "role": "tool", "tool_call_id": call_id, "content": text })
        }
    }
}

/// Anthropic models cache only what the request marks with `cache_control`;
/// OpenRouter passes the marks through (and names these models
/// `anthropic/…`). OpenAI, DeepSeek and Gemini cache on their own, and a
/// plain OpenAI-compatible server may refuse a field it doesn't know, so
/// nothing else gets marks.
fn marks_cache(model: &str) -> bool {
    model.trim_start_matches('~').starts_with("anthropic/")
}

/// Cache breakpoints (Anthropic allows four; this uses three): after the
/// harness prompt, the same bytes for every run, so the tool list and the
/// harness hit across runs; after the brief, the same for the whole run;
/// and on the message before the ledger, so each request reads the
/// conversation the previous one wrote. The ledger changes on every request
/// and stays after the last mark (see `context`).
fn mark_cache(msgs: &mut [Value]) {
    let systems: Vec<usize> = (0..msgs.len())
        .filter(|&i| msgs[i]["role"] == "system")
        .collect();
    let mut at: Vec<usize> = systems
        .first()
        .into_iter()
        .chain(systems.last())
        .copied()
        .collect();
    if msgs.len() >= 2 {
        at.push(msgs.len() - 2);
    }
    at.sort_unstable();
    at.dedup();
    for i in at {
        // An assistant turn with only tool calls has no text to mark.
        if let Some(t) = msgs[i]["content"].as_str().map(str::to_string) {
            msgs[i]["content"] =
                json!([{ "type": "text", "text": t, "cache_control": { "type": "ephemeral" } }]);
        }
    }
}

/// The one place that reads error prose: providers don't agree on a code
/// for "too long". Each pattern here came from a real provider's reply.
pub fn classify(status: u16, body: &str, retry_after: Option<Duration>) -> ProviderError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let err = &v["error"];
    let message = err["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(300).collect());
    let code = err["code"].as_str().unwrap_or("");
    let low = message.to_lowercase();
    let overflow = code == "context_length_exceeded"
        || low.contains("maximum context length")
        || low.contains("context length")
        || low.contains("context window")
        || low.contains("prompt is too long")
        || low.contains("too many tokens");
    let detail = format!("HTTP {status}: {message}");
    match status {
        _ if overflow && (status == 400 || status == 413) => ProviderError::Overflow(detail),
        429 => ProviderError::RateLimited {
            retry_after,
            detail,
        },
        408 | 409 | 425 | 500..=599 => ProviderError::Transient(detail),
        _ => ProviderError::Fatal(detail),
    }
}

/// Server-sent events: `data:` lines, blank-line separated. Comments
/// (`: keepalive`, which OpenRouter sends) and other fields are ignored.
#[derive(Default)]
pub(super) struct Sse {
    buf: Vec<u8>,
    data: String,
}

impl Sse {
    pub(super) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    out.push(std::mem::take(&mut self.data));
                }
            } else if let Some(d) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(d.strip_prefix(' ').unwrap_or(d));
            }
        }
        out
    }
}

#[derive(Default)]
struct Accumulator {
    text: String,
    calls: Vec<(String, String, String)>,
    finish: Option<String>,
    usage: ReplyUsage,
    any: bool,
}

impl Accumulator {
    fn chunk(&mut self, v: &Value) {
        self.any = true;
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage = ReplyUsage {
                prompt: u["prompt_tokens"].as_u64(),
                completion: u["completion_tokens"].as_u64(),
                cached: u["prompt_tokens_details"]["cached_tokens"].as_u64(),
                cache_write: u["prompt_tokens_details"]["cache_write_tokens"].as_u64(),
                cost: u["cost"].as_f64(),
            };
        }
        let Some(choice) = v["choices"].get(0) else {
            return;
        };
        if let Some(f) = choice["finish_reason"].as_str() {
            self.finish = Some(f.to_string());
        }
        let delta = &choice["delta"];
        if let Some(t) = delta["content"].as_str() {
            self.text.push_str(t);
        }
        for tc in delta["tool_calls"].as_array().into_iter().flatten() {
            let i = tc["index"].as_u64().unwrap_or(0) as usize;
            while self.calls.len() <= i {
                self.calls.push(Default::default());
            }
            let slot = &mut self.calls[i];
            if let Some(id) = tc["id"].as_str() {
                slot.0 = id.to_string();
            }
            if let Some(n) = tc["function"]["name"].as_str() {
                slot.1.push_str(n);
            }
            if let Some(a) = tc["function"]["arguments"].as_str() {
                slot.2.push_str(a);
            }
        }
    }

    fn finish(self) -> Result<Reply, ProviderError> {
        if !self.any {
            return Err(ProviderError::Transient("empty response".into()));
        }
        Ok(Reply {
            text: (!self.text.trim().is_empty()).then_some(self.text),
            calls: self
                .calls
                .into_iter()
                // A gap in the stream's indexes leaves a slot nothing was
                // sent for: not a call.
                .filter(|(_, name, arguments)| !(name.is_empty() && arguments.is_empty()))
                .enumerate()
                .map(|(i, (id, name, arguments))| ToolCall {
                    id: if id.is_empty() {
                        format!("call_{i}")
                    } else {
                        id
                    },
                    name,
                    arguments,
                })
                .collect(),
            finish: self.finish,
            usage: self.usage,
            replay: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_survives_any_chunking_and_ignores_comments() {
        let stream = ": keepalive\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"He\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"a\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\r\n\r\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n";
        for split in [1, 3, 7, 64, stream.len()] {
            let mut sse = Sse::default();
            let mut acc = Accumulator::default();
            let mut done = false;
            for part in stream.as_bytes().chunks(split) {
                for d in sse.push(part) {
                    if d == "[DONE]" {
                        done = true;
                    } else {
                        acc.chunk(&serde_json::from_str(&d).expect("json"));
                    }
                }
            }
            assert!(done);
            let r = acc.finish().expect("reply");
            assert_eq!(r.text.as_deref(), Some("He"));
            assert_eq!(r.calls.len(), 1);
            assert_eq!(r.calls[0].name, "read_file");
            assert_eq!(r.calls[0].arguments, "{\"path\":\"a\"}");
            assert_eq!(r.finish.as_deref(), Some("tool_calls"));
            assert_eq!((r.usage.prompt, r.usage.completion), (Some(7), Some(2)));
        }
    }

    #[test]
    fn a_gap_in_the_call_indexes_is_not_a_call() {
        // Some proxies number the only call 1: slot 0 was never sent.
        let mut acc = Accumulator::default();
        acc.chunk(&json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 1, "id": "c1", "function": { "name": "read_file", "arguments": "{}" } }] } }] }));
        let r = acc.finish().expect("reply");
        assert_eq!(r.calls.len(), 1, "{:?}", r.calls);
        assert_eq!(
            (r.calls[0].id.as_str(), r.calls[0].name.as_str()),
            ("c1", "read_file")
        );
    }

    #[test]
    fn errors_are_classified_by_what_can_be_done_about_them() {
        let openai = r#"{"error":{"message":"This model's maximum context length is 128000 tokens.","code":"context_length_exceeded"}}"#;
        assert!(matches!(
            classify(400, openai, None),
            ProviderError::Overflow(_)
        ));
        let anthropicish =
            r#"{"error":{"message":"prompt is too long: 210000 tokens > 200000 maximum"}}"#;
        assert!(matches!(
            classify(400, anthropicish, None),
            ProviderError::Overflow(_)
        ));
        assert!(matches!(
            classify(429, "{}", Some(Duration::from_secs(2))),
            ProviderError::RateLimited {
                retry_after: Some(_),
                ..
            }
        ));
        assert!(matches!(
            classify(503, "busy", None),
            ProviderError::Transient(_)
        ));
        assert!(matches!(
            classify(401, r#"{"error":{"message":"bad key"}}"#, None),
            ProviderError::Fatal(_)
        ));
        assert!(matches!(
            classify(400, r#"{"error":{"message":"unknown model"}}"#, None),
            ProviderError::Fatal(_)
        ));
    }

    #[test]
    fn anthropic_requests_mark_the_stable_prefix_and_not_the_ledger() {
        assert!(marks_cache("anthropic/claude-sonnet-5"));
        assert!(marks_cache("~anthropic/claude-sonnet-latest"));
        assert!(!marks_cache("openai/gpt-5-mini"));
        assert!(!marks_cache("claude-sonnet-5"));

        let conv = [
            Msg::System("harness".into()),
            Msg::System("brief".into()),
            Msg::User("begin".into()),
            Msg::Assistant {
                text: None,
                calls: vec![ToolCall {
                    id: "t1".into(),
                    name: "read_file".into(),
                    arguments: "{}".into(),
                }],
                replay: None,
            },
            Msg::Tool {
                call_id: "t1".into(),
                text: "result".into(),
            },
            Msg::User("ledger".into()),
        ];
        let mut msgs: Vec<Value> = conv.iter().map(to_openai).collect();
        mark_cache(&mut msgs);
        let marked: Vec<bool> = msgs
            .iter()
            .map(|m| m["content"][0]["cache_control"]["type"] == "ephemeral")
            .collect();
        assert_eq!(marked, [true, true, false, false, true, false]);
        assert_eq!(msgs[4]["content"][0]["text"], "result");
        assert_eq!(msgs[4]["tool_call_id"], "t1");
        assert_eq!(msgs[5]["content"], "ledger");

        // The first request: the mark before the ledger lands on "begin".
        let mut first: Vec<Value> = conv[..3].iter().chain(&conv[5..]).map(to_openai).collect();
        mark_cache(&mut first);
        assert!(
            first[..3]
                .iter()
                .all(|m| m["content"][0]["cache_control"].is_object())
        );
        assert_eq!(first[3]["content"], "ledger");
    }

    #[test]
    fn the_key_never_prints() {
        assert_eq!(format!("{:?}", Secret("sk-live-123".into())), "***");
    }
}
