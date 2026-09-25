//! OpenAI Responses (`POST {base_url}/responses`), streamed and stateless.
//!
//! `store: false`: nothing is kept on the provider's side, so every request
//! carries the whole input, the way the other two formats do, and a resumed
//! run doesn't depend on a response id that may be gone. Reasoning models
//! then return their reasoning as encrypted items that the next request
//! must send back; they are journaled with the reply (`Reply::replay`) and
//! rendered verbatim, so a resumed run sends the same input.

use serde_json::{Value, json};

use super::provider::{
    Msg, ProviderError, Reply, ReplyUsage, Secret, ToolCall, classify_in_stream, parse_event,
    stream, tool_parts,
};

pub const NAME: &str = "openai-responses";

pub struct Responses {
    pub(super) http: reqwest::Client,
    pub(super) url: String,
    pub(super) model: String,
    pub(super) max_output: u64,
    pub(super) key: Secret,
}

impl Responses {
    pub fn body(&self, messages: &[Msg], tools: &Value) -> Value {
        // `strict: false`: the schemas mean what they say for every
        // provider; Responses would otherwise rewrite them into strict mode
        // where it can.
        let tools: Vec<Value> = tool_parts(tools)
            .map(|(name, description, parameters)| {
                json!({ "type": "function", "name": name, "description": description, "parameters": parameters, "strict": false })
            })
            .collect();
        json!({
            "model": self.model,
            "input": render(messages),
            "tools": tools,
            "tool_choice": "auto",
            "max_output_tokens": self.max_output,
            "store": false,
            // Stateless requests get encrypted reasoning by default now;
            // asking keeps servers that follow the older contract giving it.
            "include": ["reasoning.encrypted_content"],
            "stream": true,
        })
    }

    pub(super) async fn send(&self, body: &Value) -> Result<Reply, ProviderError> {
        let req = self
            .http
            .post(&self.url)
            .header("authorization", self.key.bearer());
        let mut acc = Accumulator::default();
        stream(req, body, |data| acc.event(&parse_event(data)?)).await?;
        acc.finish()
    }
}

fn render(msgs: &[Msg]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in msgs {
        match m {
            Msg::System(t) => out.push(json!({ "role": "system", "content": t })),
            Msg::User(t) => out.push(json!({ "role": "user", "content": t })),
            Msg::Assistant {
                text,
                calls,
                replay,
            } => match replay.as_ref().filter(|r| r["provider"] == NAME) {
                // The reply's own output items, in their order, with each
                // call's arguments from the conversation (compaction may
                // have shortened them).
                Some(r) => {
                    for item in r["items"].as_array().into_iter().flatten() {
                        let mut item = item.clone();
                        if item["type"] == "function_call" {
                            let args = calls
                                .iter()
                                .find(|c| item["call_id"] == c.id.as_str())
                                .map_or("{}", |c| c.arguments.as_str());
                            item["arguments"] = json!(args);
                        }
                        out.push(item);
                    }
                }
                None => {
                    if let Some(t) = text {
                        out.push(json!({ "role": "assistant", "content": t }));
                    }
                    for c in calls {
                        out.push(json!({ "type": "function_call", "call_id": c.id, "name": c.name, "arguments": c.arguments }));
                    }
                }
            },
            Msg::Tool { call_id, text } => out.push(
                json!({ "type": "function_call_output", "call_id": call_id, "output": text }),
            ),
        }
    }
    out
}

/// Error codes in a stream by the HTTP status they stand for.
fn status_of(code: &str) -> u16 {
    match code {
        "rate_limit_exceeded" => 429,
        "context_length_exceeded" | "invalid_prompt" | "invalid_request_error" => 400,
        _ => 500,
    }
}

fn stream_error(e: &Value) -> ProviderError {
    let code = e["code"].as_str().unwrap_or("");
    let message = e["message"].as_str().unwrap_or("error in the stream");
    let err = classify_in_stream(status_of(code), &format!("{message} ({code})"));
    match err {
        // The code says it where the message may not.
        ProviderError::Fatal(d) if code == "context_length_exceeded" => ProviderError::Overflow(d),
        e => e,
    }
}

#[derive(Default)]
struct Accumulator {
    /// Items as `response.output_item.done` delivered them, by index.
    items: Vec<(u64, Value)>,
    response: Option<Value>,
}

impl Accumulator {
    /// One event; `Ok(true)` once the response is complete or incomplete.
    fn event(&mut self, v: &Value) -> Result<bool, ProviderError> {
        match v["type"].as_str().unwrap_or("") {
            "response.output_item.done" => {
                self.items
                    .push((v["output_index"].as_u64().unwrap_or(0), v["item"].clone()));
            }
            "response.completed" | "response.incomplete" => {
                self.response = Some(v["response"].clone());
                return Ok(true);
            }
            "response.failed" => return Err(stream_error(&v["response"]["error"])),
            "error" => return Err(stream_error(v)),
            // Deltas: the done events carry the same, whole.
            _ => {}
        }
        Ok(false)
    }

    fn finish(mut self) -> Result<Reply, ProviderError> {
        let Some(resp) = self.response else {
            return Err(ProviderError::Transient(
                "the stream ended before the response completed".into(),
            ));
        };
        let mut output: Vec<Value> = resp["output"].as_array().cloned().unwrap_or_default();
        if output.is_empty() {
            self.items.sort_by_key(|(i, _)| *i);
            output = self.items.into_iter().map(|(_, item)| item).collect();
        }
        let mut text = String::new();
        let mut calls = Vec::new();
        for item in &mut output {
            match item["type"].as_str() {
                Some("message") => {
                    for part in item["content"].as_array().into_iter().flatten() {
                        if let Some(t) = part["text"].as_str().or(part["refusal"].as_str()) {
                            text.push_str(t);
                        }
                    }
                }
                Some("function_call") => {
                    calls.push(ToolCall {
                        id: item["call_id"].as_str().unwrap_or("").to_string(),
                        name: item["name"].as_str().unwrap_or("").to_string(),
                        arguments: item["arguments"].as_str().unwrap_or("").to_string(),
                    });
                    // Journaled once, with the call; rendered from there.
                    if let Some(o) = item.as_object_mut() {
                        o.remove("arguments");
                    }
                }
                _ => {}
            }
        }
        let u = &resp["usage"];
        let finish = match resp["status"].as_str() {
            Some("incomplete") => resp["incomplete_details"]["reason"].as_str(),
            s => s,
        };
        Ok(Reply {
            text: (!text.trim().is_empty()).then_some(text),
            calls,
            finish: finish.map(str::to_string),
            usage: ReplyUsage {
                prompt: u["input_tokens"].as_u64(),
                completion: u["output_tokens"].as_u64(),
                cached: u["input_tokens_details"]["cached_tokens"].as_u64(),
                cache_write: u["input_tokens_details"]["cache_write_tokens"].as_u64(),
                cost: None,
            },
            replay: (!output.is_empty()).then(|| json!({ "provider": NAME, "items": output })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::Sse;

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

    const REASONING: &str =
        r#"{"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"gAAA-opaque"}"#;

    fn stream_of(output: &str) -> String {
        format!(
            "event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{REASONING}}}\n\n\
             event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"Rea\"}}\n\n\
             event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"status\":\"completed\",\"output\":{output},\"usage\":{{\"input_tokens\":1200,\"output_tokens\":30,\"input_tokens_details\":{{\"cached_tokens\":1024}}}}}}}}\n\n"
        )
    }

    #[test]
    fn a_reply_keeps_its_reasoning_for_the_next_request() {
        let output = format!(
            r#"[{REASONING},{{"type":"message","id":"msg_1","role":"assistant","phase":"commentary","content":[{{"type":"output_text","text":"Reading."}}]}},{{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":"{{\"path\":\"a\"}}"}}]"#
        );
        let r = play(&stream_of(&output)).expect("reply");
        assert_eq!(r.text.as_deref(), Some("Reading."));
        assert_eq!(
            r.calls,
            [ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: "{\"path\":\"a\"}".into()
            }]
        );
        assert_eq!(
            (r.usage.prompt, r.usage.cached, r.usage.completion),
            (Some(1200), Some(1024), Some(30))
        );
        let replay = r.replay.clone().expect("replay");
        assert_eq!(replay["items"][0]["encrypted_content"], "gAAA-opaque");
        assert!(
            replay["items"][2].get("arguments").is_none(),
            "arguments are journaled once"
        );

        // Rendered back: reasoning first, the message with its phase, then
        // the call with its arguments (here elided by compaction).
        let input = render(&[
            Msg::User("begin".into()),
            Msg::Assistant {
                text: r.text.clone(),
                calls: vec![ToolCall {
                    arguments: "{\"path\":\"<elided>\"}".into(),
                    ..r.calls[0].clone()
                }],
                replay: r.replay.clone(),
            },
            Msg::Tool {
                call_id: "call_1".into(),
                text: "1\tx".into(),
            },
            Msg::User("ledger".into()),
        ]);
        assert_eq!(
            input[1],
            serde_json::from_str::<Value>(REASONING).expect("json")
        );
        assert_eq!(input[2]["phase"], "commentary");
        assert_eq!(input[3]["arguments"], "{\"path\":\"<elided>\"}");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(
            input[4],
            json!({ "type": "function_call_output", "call_id": "call_1", "output": "1\tx" })
        );
        assert_eq!(input[5], json!({ "role": "user", "content": "ledger" }));
    }

    #[test]
    fn items_come_from_the_done_events_when_the_final_response_omits_them() {
        let r = play(&stream_of("[]")).expect("reply");
        assert_eq!(r.replay.expect("replay")["items"][0]["id"], "rs_1");
        assert!(r.text.is_none() && r.calls.is_empty());
    }

    #[test]
    fn replies_from_another_provider_render_from_the_conversation() {
        let input = render(&[Msg::Assistant {
            text: Some("Hi".into()),
            calls: vec![ToolCall {
                id: "c1".into(),
                name: "grep".into(),
                arguments: "{}".into(),
            }],
            replay: Some(json!({ "provider": "someone-else", "items": [1] })),
        }]);
        assert_eq!(
            input,
            [
                json!({ "role": "assistant", "content": "Hi" }),
                json!({ "type": "function_call", "call_id": "c1", "name": "grep", "arguments": "{}" }),
            ]
        );
    }

    #[test]
    fn stream_errors_are_classified() {
        let failed = |code: &str| {
            format!(
                "data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"code\":\"{code}\",\"message\":\"no\"}}}}}}\n\n"
            )
        };
        assert!(matches!(
            play(&failed("server_error")),
            Err(ProviderError::Transient(_))
        ));
        assert!(matches!(
            play(&failed("context_length_exceeded")),
            Err(ProviderError::Overflow(_))
        ));
        assert!(matches!(
            play(&failed("rate_limit_exceeded")),
            Err(ProviderError::RateLimited { .. })
        ));
        assert!(matches!(
            play(&failed("invalid_prompt")),
            Err(ProviderError::Fatal(_))
        ));
        assert!(matches!(
            play("data: {\"type\":\"response.created\"}\n\n"),
            Err(ProviderError::Transient(_))
        ));
    }
}
