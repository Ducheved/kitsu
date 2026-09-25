//! Talking to a model: OpenAI-compatible Chat Completions (OpenRouter and
//! anything that speaks the same API), streamed.
//!
//! Streaming isn't for showing tokens as they come: idle proxies cut long
//! non-streamed requests, and a cancel should drop the request right away.
//! Only the complete response is returned; the brain journals it whole.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;

use crate::agents::NativeSpec;

/// An API key. Never printed, never serialized; attached to one header.
pub struct Secret(String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    System(String),
    User(String),
    Assistant {
        text: Option<String>,
        calls: Vec<ToolCall>,
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
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReplyUsage {
    pub prompt: Option<u64>,
    pub completion: Option<u64>,
    pub cached: Option<u64>,
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

pub struct OpenAiChat {
    http: reqwest::Client,
    url: String,
    model: String,
    max_output: u64,
    key: Secret,
}

impl OpenAiChat {
    /// Reads the key from the environment variable the spec names. A missing
    /// key is an error here, before any request.
    pub fn new(spec: &NativeSpec) -> Result<OpenAiChat, String> {
        let key = std::env::var(&spec.api_key_env)
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "${} is not set; the agent needs its API key there",
                    spec.api_key_env
                )
            })?;
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
        let http = b.build().map_err(|e| format!("http client: {e}"))?;
        Ok(OpenAiChat {
            http,
            url: format!("{}/chat/completions", spec.base_url.trim_end_matches('/')),
            model: spec.model.clone(),
            max_output: spec.max_output,
            key: Secret(key),
        })
    }

    /// The request body, without the key. Also what `request_sha` hashes.
    pub fn body(&self, messages: &[Msg], tools: &Value) -> Value {
        json!({
            "model": self.model,
            "messages": messages.iter().map(to_openai).collect::<Vec<_>>(),
            "tools": tools,
            "tool_choice": "auto",
            "max_tokens": self.max_output,
            "stream": true,
            "stream_options": { "include_usage": true },
        })
    }

    pub async fn complete(
        &self,
        body: &Value,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Reply, ProviderError> {
        if *cancel.borrow() {
            return Err(ProviderError::Cancelled);
        }
        tokio::select! {
            r = self.send(body) => r,
            _ = cancel.wait_for(|c| *c) => Err(ProviderError::Cancelled),
        }
    }

    async fn send(&self, body: &Value) -> Result<Reply, ProviderError> {
        let resp = self
            .http
            .post(&self.url)
            .header("authorization", format!("Bearer {}", self.key.0))
            .header("x-title", "Kitsu")
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| {
                ProviderError::Transient(format!("request failed: {}", without_url(&e)))
            })?;
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
        let mut acc = Accumulator::default();
        let mut resp = resp;
        loop {
            match resp.chunk().await {
                Ok(Some(bytes)) => {
                    for data in sse.push(&bytes) {
                        if data == "[DONE]" {
                            return acc.finish();
                        }
                        let v: Value = serde_json::from_str(&data).map_err(|e| {
                            ProviderError::Transient(format!("bad stream chunk: {e}"))
                        })?;
                        if let Some(err) = v.get("error") {
                            return Err(classify(400, &json!({ "error": err }).to_string(), None));
                        }
                        acc.chunk(&v);
                    }
                }
                Ok(None) => return acc.finish(),
                Err(e) => {
                    return Err(ProviderError::Transient(format!(
                        "stream cut: {}",
                        without_url(&e)
                    )));
                }
            }
        }
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
        Msg::Assistant { text, calls } => {
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
struct Sse {
    buf: Vec<u8>,
    data: String,
}

impl Sse {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
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
    fn the_key_never_prints() {
        assert_eq!(format!("{:?}", Secret("sk-live-123".into())), "***");
    }
}
