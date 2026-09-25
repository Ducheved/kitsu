//! Anthropic Messages (`POST {base_url}/v1/messages`), streamed.
//!
//! The harness and the brief go in `system`; the conversation becomes
//! alternating user and assistant turns, tool results as `tool_result`
//! blocks in the user turn that follows the calls. Anthropic caches only
//! what the request marks, so every request carries three marks, placed as
//! decision `native-prompt-cache` places them for OpenRouter.

use serde_json::{Value, json};

use super::provider::{
    Msg, ProviderError, Reply, ReplyUsage, Secret, ToolCall, classify_in_stream, parse_event,
    stream, tool_parts,
};

/// The API version this module speaks.
const VERSION: &str = "2023-06-01";

pub struct Messages {
    pub(super) http: reqwest::Client,
    pub(super) url: String,
    pub(super) model: String,
    pub(super) max_output: u64,
    pub(super) key: Secret,
}

impl Messages {
    pub fn body(&self, messages: &[Msg], tools: &Value) -> Value {
        let (system, messages) = render(messages);
        let tools: Vec<Value> = tool_parts(tools)
            .map(|(name, description, schema)| {
                json!({ "name": name, "description": description, "input_schema": schema })
            })
            .collect();
        json!({
            "model": self.model,
            "max_tokens": self.max_output,
            "system": system,
            "messages": messages,
            "tools": tools,
            "tool_choice": { "type": "auto" },
            "stream": true,
        })
    }

    pub(super) async fn send(&self, body: &Value) -> Result<Reply, ProviderError> {
        // The bearer header, not the legacy `x-api-key`: Anthropic's API
        // takes both, OpenRouter's Messages endpoint documents only this one.
        let req = self
            .http
            .post(&self.url)
            .header("authorization", self.key.bearer())
            .header("anthropic-version", VERSION);
        let mut acc = Accumulator::default();
        stream(req, body, |data| acc.event(&parse_event(data)?)).await?;
        acc.finish()
    }
}

fn mark(block: &mut Value) {
    block["cache_control"] = json!({ "type": "ephemeral" });
}

/// `(system, messages)`. Marks: the harness and the brief (the first and
/// last system blocks), and the block right before the ledger, which is the
/// last block of the request and stays unmarked.
fn render(msgs: &[Msg]) -> (Vec<Value>, Vec<Value>) {
    let mut system: Vec<Value> = Vec::new();
    let mut turns: Vec<(&str, Vec<Value>)> = Vec::new();
    let mut push = |role: &'static str, block: Value| match turns.last_mut() {
        Some((r, blocks)) if *r == role => blocks.push(block),
        _ => turns.push((role, vec![block])),
    };
    for m in msgs {
        match m {
            Msg::System(t) => system.push(json!({ "type": "text", "text": t })),
            Msg::User(t) => push("user", json!({ "type": "text", "text": t })),
            Msg::Assistant { text, calls, .. } => {
                let mut blocks = Vec::new();
                if let Some(t) = text.as_deref().filter(|t| !t.trim().is_empty()) {
                    blocks.push(json!({ "type": "text", "text": t }));
                }
                for c in calls {
                    blocks.push(json!({ "type": "tool_use", "id": c.id, "name": c.name, "input": input(&c.arguments) }));
                }
                // The API refuses an empty turn; a reply with neither text
                // nor calls still happened and was answered.
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "(no reply)" }));
                }
                for b in blocks {
                    push("assistant", b);
                }
            }
            Msg::Tool { call_id, text } => {
                // Nor does it take an empty text; an empty output is "".
                let text = if text.is_empty() { "(no output)" } else { text };
                push(
                    "user",
                    json!({ "type": "tool_result", "tool_use_id": call_id, "content": text }),
                );
            }
        }
    }
    let n = system.len();
    if n > 0 {
        mark(&mut system[0]);
        mark(&mut system[n - 1]);
    }
    let mut blocks: Vec<&mut Value> = turns.iter_mut().flat_map(|(_, b)| b.iter_mut()).collect();
    if blocks.len() >= 2 {
        let i = blocks.len() - 2;
        mark(blocks[i]);
    }
    let messages = turns
        .into_iter()
        .map(|(role, content)| json!({ "role": role, "content": content }))
        .collect();
    (system, messages)
}

/// A call's arguments as the object `tool_use` needs. Arguments that aren't
/// an object were already refused by the tool; `{}` keeps the turn valid.
fn input(arguments: &str) -> Value {
    match serde_json::from_str::<Value>(arguments) {
        Ok(v @ Value::Object(_)) => v,
        _ => json!({}),
    }
}

/// Error types (from Anthropic's error reference) by the HTTP status they
/// come with outside a stream.
fn status_of(kind: &str) -> u16 {
    match kind {
        "overloaded_error" => 529,
        "rate_limit_error" => 429,
        "api_error" => 500,
        "timeout_error" => 504,
        "authentication_error" => 401,
        "permission_error" => 403,
        _ => 400,
    }
}

enum Block {
    Text(String),
    /// id, name, the streamed JSON, the input from the block's start.
    Tool(String, String, String, Value),
    Other,
}

#[derive(Default)]
struct Accumulator {
    blocks: Vec<Block>,
    stop: Option<String>,
    input: Option<u64>,
    output: Option<u64>,
    read: Option<u64>,
    write: Option<u64>,
    done: bool,
}

impl Accumulator {
    /// One event; `Ok(true)` at `message_stop`.
    fn event(&mut self, v: &Value) -> Result<bool, ProviderError> {
        match v["type"].as_str().unwrap_or("") {
            "message_start" => self.usage(&v["message"]["usage"]),
            "content_block_start" => {
                let i = v["index"].as_u64().unwrap_or(self.blocks.len() as u64) as usize;
                while self.blocks.len() <= i {
                    self.blocks.push(Block::Other);
                }
                let b = &v["content_block"];
                let s = |k: &str| b[k].as_str().unwrap_or("").to_string();
                self.blocks[i] = match b["type"].as_str() {
                    Some("text") => Block::Text(s("text")),
                    Some("tool_use") => {
                        Block::Tool(s("id"), s("name"), String::new(), b["input"].clone())
                    }
                    _ => Block::Other,
                };
            }
            "content_block_delta" => {
                let i = v["index"].as_u64().unwrap_or(0) as usize;
                let d = &v["delta"];
                match (self.blocks.get_mut(i), d["type"].as_str()) {
                    (Some(Block::Text(t)), Some("text_delta")) => {
                        t.push_str(d["text"].as_str().unwrap_or(""));
                    }
                    (Some(Block::Tool(_, _, json, _)), Some("input_json_delta")) => {
                        json.push_str(d["partial_json"].as_str().unwrap_or(""));
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(s) = v["delta"]["stop_reason"].as_str() {
                    self.stop = Some(s.to_string());
                }
                self.usage(&v["usage"]);
            }
            "message_stop" => {
                self.done = true;
                return Ok(true);
            }
            "error" => {
                let e = &v["error"];
                return Err(classify_in_stream(
                    status_of(e["type"].as_str().unwrap_or("")),
                    e["message"].as_str().unwrap_or("error in the stream"),
                ));
            }
            // `ping`, and event types added later.
            _ => {}
        }
        Ok(false)
    }

    /// Counts in `message_delta` are cumulative: the latest one wins.
    fn usage(&mut self, u: &Value) {
        let set = |slot: &mut Option<u64>, v: &Value| {
            if let Some(n) = v.as_u64() {
                *slot = Some(n);
            }
        };
        set(&mut self.input, &u["input_tokens"]);
        set(&mut self.output, &u["output_tokens"]);
        set(&mut self.read, &u["cache_read_input_tokens"]);
        set(&mut self.write, &u["cache_creation_input_tokens"]);
    }

    fn finish(self) -> Result<Reply, ProviderError> {
        if !self.done {
            return Err(ProviderError::Transient(
                "the stream ended before message_stop".into(),
            ));
        }
        // The reply filled the whole window: the next request, with it,
        // can't fit either, so it's handled as an overflow.
        if self.stop.as_deref() == Some("model_context_window_exceeded") {
            return Err(ProviderError::Overflow(
                "stop_reason model_context_window_exceeded".into(),
            ));
        }
        let mut text = Vec::new();
        let mut calls = Vec::new();
        for b in self.blocks {
            match b {
                Block::Text(t) => text.push(t),
                Block::Tool(id, name, json, start) => calls.push(ToolCall {
                    id,
                    name,
                    // No deltas: the input was complete at the start ({}).
                    arguments: if json.is_empty() {
                        start.to_string()
                    } else {
                        json
                    },
                }),
                Block::Other => {}
            }
        }
        let text = text.join("\n");
        // `input_tokens` is only what came after the last cache mark.
        let prompt = match (self.input, self.read, self.write) {
            (None, None, None) => None,
            (i, r, w) => Some(i.unwrap_or(0) + r.unwrap_or(0) + w.unwrap_or(0)),
        };
        Ok(Reply {
            text: (!text.trim().is_empty()).then_some(text),
            calls,
            finish: self.stop,
            usage: ReplyUsage {
                prompt,
                completion: self.output,
                cached: self.read,
                cache_write: self.write,
                cost: None,
            },
            replay: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::Sse;

    fn conv() -> Vec<Msg> {
        let call = |id: &str| ToolCall {
            id: id.into(),
            name: "read_file".into(),
            arguments: "{\"path\":\"a\"}".into(),
        };
        vec![
            Msg::System("harness".into()),
            Msg::System("brief".into()),
            Msg::User("begin".into()),
            Msg::Assistant {
                text: Some("Reading.".into()),
                calls: vec![call("t1"), call("t2")],
                replay: None,
            },
            Msg::Tool {
                call_id: "t1".into(),
                text: "one".into(),
            },
            Msg::Tool {
                call_id: "t2".into(),
                text: "".into(),
            },
            Msg::User("ledger".into()),
        ]
    }

    fn marked(b: &Value) -> bool {
        b["cache_control"]["type"] == "ephemeral"
    }

    #[test]
    fn marks_the_harness_the_brief_and_the_block_before_the_ledger() {
        let (system, msgs) = render(&conv());
        assert_eq!(system.len(), 2);
        assert!(system.iter().all(marked));
        assert_eq!(
            (system[0]["text"].as_str(), system[1]["text"].as_str()),
            (Some("harness"), Some("brief"))
        );

        // user begin / assistant text + two calls / user results + ledger.
        let roles: Vec<&str> = msgs.iter().filter_map(|m| m["role"].as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        let last = msgs[2]["content"].as_array().expect("blocks");
        assert_eq!(last.len(), 3);
        assert_eq!(last[0]["type"], "tool_result");
        assert_eq!(last[0]["tool_use_id"], "t1");
        assert_eq!(last[1]["content"], "(no output)");
        assert!(marked(&last[1]), "the last block before the ledger");
        assert_eq!(last[2]["text"], "ledger");
        assert!(!marked(&last[2]), "the ledger is never marked");
        let marks = msgs
            .iter()
            .flat_map(|m| m["content"].as_array().into_iter().flatten())
            .filter(|b| marked(b))
            .count();
        assert_eq!(marks, 1, "three marks in all, one in the conversation");
        assert_eq!(msgs[1]["content"][1]["input"], json!({ "path": "a" }));

        // The first request: the mark lands on "begin", the ledger's
        // neighbour in the same user turn.
        let first: Vec<Msg> = [conv()[..3].to_vec(), vec![Msg::User("ledger".into())]].concat();
        let (_, msgs) = render(&first);
        assert_eq!(msgs.len(), 1);
        assert!(marked(&msgs[0]["content"][0]));
        assert!(!marked(&msgs[0]["content"][1]));
    }

    #[test]
    fn a_reply_with_nothing_in_it_still_renders_as_a_turn() {
        let (_, msgs) = render(&[
            Msg::User("begin".into()),
            Msg::Assistant {
                text: None,
                calls: vec![],
                replay: None,
            },
            Msg::User("not done".into()),
            Msg::User("ledger".into()),
        ]);
        assert_eq!(msgs[1]["content"][0]["text"], "(no reply)");
        assert_eq!(msgs[2]["content"].as_array().map(Vec::len), Some(2));
    }

    fn play(stream: &str) -> Result<Reply, ProviderError> {
        let mut sse = Sse::default();
        let mut acc = Accumulator::default();
        for d in sse.push(stream.as_bytes()) {
            if acc.event(&serde_json::from_str(&d).expect("json"))? {
                break;
            }
        }
        acc.finish()
    }

    #[test]
    fn the_stream_becomes_a_reply_with_cache_usage() {
        let s = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":900,\"cache_creation_input_tokens\":90,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: ping\ndata: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Reading\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"a\\\"}\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"list_files\",\"input\":{}}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":42}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let r = play(s).expect("reply");
        assert_eq!(r.text.as_deref(), Some("Reading"));
        assert_eq!(r.calls.len(), 2);
        assert_eq!(
            (r.calls[0].id.as_str(), r.calls[0].arguments.as_str()),
            ("toolu_1", "{\"path\":\"a\"}")
        );
        assert_eq!(r.calls[1].arguments, "{}");
        assert_eq!(r.finish.as_deref(), Some("tool_use"));
        assert_eq!(
            r.usage,
            ReplyUsage {
                prompt: Some(1000),
                completion: Some(42),
                cached: Some(900),
                cache_write: Some(90),
                cost: None
            }
        );
    }

    #[test]
    fn errors_in_and_out_of_the_stream_map_to_what_can_be_done() {
        let overloaded = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        assert!(matches!(play(overloaded), Err(ProviderError::Transient(_))));
        let cut = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{}}}\n\n";
        assert!(matches!(play(cut), Err(ProviderError::Transient(_))));
        let full = "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"model_context_window_exceeded\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n";
        assert!(matches!(play(full), Err(ProviderError::Overflow(_))));

        use super::super::provider::classify;
        let body = |t: &str, m: &str| {
            json!({ "type": "error", "error": { "type": t, "message": m } }).to_string()
        };
        assert!(matches!(
            classify(529, &body("overloaded_error", "Overloaded"), None),
            ProviderError::Transient(_)
        ));
        assert!(matches!(
            classify(
                400,
                &body(
                    "invalid_request_error",
                    "prompt is too long: 210000 tokens > 200000 maximum"
                ),
                None
            ),
            ProviderError::Overflow(_)
        ));
        assert!(matches!(
            classify(
                401,
                &body("authentication_error", "invalid x-api-key"),
                None
            ),
            ProviderError::Fatal(_)
        ));
        assert!(matches!(
            classify(429, &body("rate_limit_error", "slow down"), None),
            ProviderError::RateLimited { .. }
        ));
    }
}
