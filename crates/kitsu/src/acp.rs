//! A small ACP (Agent Client Protocol, v1) client.
//!
//! Only the part of the protocol Kitsu uses: `initialize`, `session/new`,
//! `session/prompt`, `session/cancel`, `session/update` and
//! `session/request_permission`. Kitsu does not offer `fs/*` or `terminal/*`
//! to agents: the agent works in its own worktree with its own tools, and
//! both capabilities are removed in the ACP v2 draft anyway.
//!
//! Reading is tolerant. Unknown update variants and extra fields are kept as
//! raw JSON and ignored by callers that don't understand them, so a newer
//! agent doesn't break an older Kitsu. Malformed JSON or a response to a
//! request we never sent is a protocol violation and ends the run.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot};

use crate::error::{Error, Result};

pub const PROTOCOL_VERSION: u64 = 1;

/// A line longer than this is not a message, it's a bug or an attack.
const MAX_LINE: usize = 32 * 1024 * 1024;

/// Things the agent sends that aren't answers to our requests.
#[derive(Debug)]
pub enum Incoming {
    /// `session/update` params.
    Update(Value),
    /// `session/request_permission`: answer with `Client::respond`.
    Permission { id: Value, params: Value },
    /// The agent broke the protocol. The connection is unusable after this.
    Violation(String),
    /// A second response to a request that was already answered. The first
    /// answer stands; this one is worth recording but not fatal.
    DuplicateResponse(u64),
    /// stdout closed.
    Closed,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, Value>>>>>;

#[derive(Clone)]
pub struct Client {
    out: mpsc::Sender<String>,
    pending: Pending,
    next: Arc<std::sync::atomic::AtomicU64>,
}

impl Client {
    /// Start reader and writer tasks. Incoming traffic arrives on the
    /// returned receiver, which is bounded: if Kitsu falls behind, it stops
    /// reading the agent's stdout and the agent blocks on write. That is the
    /// backpressure, there is no buffer that grows.
    pub fn start(stdin: ChildStdin, stdout: ChildStdout) -> (Client, mpsc::Receiver<Incoming>) {
        let (out_tx, mut out_rx) = mpsc::channel::<String>(64);
        let (in_tx, in_rx) = mpsc::channel::<Incoming>(256);
        let pending: Pending = Arc::default();

        let mut stdin = stdin;
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                {
                    break;
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
            // Dropping stdin closes the pipe; well-behaved agents exit on EOF.
        });

        let reader_pending = pending.clone();
        // Weak, so the reader alone doesn't keep the agent's stdin open.
        let responder = out_tx.downgrade();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut answered = std::collections::HashSet::new();
            let mut buf = Vec::new();
            loop {
                buf.clear();
                let n = match read_line_bounded(&mut reader, &mut buf).await {
                    Ok(n) => n,
                    Err(e) => {
                        let _ = in_tx.send(Incoming::Violation(e)).await;
                        break;
                    }
                };
                if n == 0 {
                    let _ = in_tx.send(Incoming::Closed).await;
                    break;
                }
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let msg: Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = in_tx
                            .send(Incoming::Violation(format!("invalid JSON from agent: {e}")))
                            .await;
                        break;
                    }
                };
                match classify(msg) {
                    Msg::Response { id, result } => {
                        let waiter = reader_pending.lock().ok().and_then(|mut p| p.remove(&id));
                        match waiter {
                            Some(tx) => {
                                answered.insert(id);
                                let _ = tx.send(result);
                            }
                            None if answered.contains(&id) => {
                                if in_tx.send(Incoming::DuplicateResponse(id)).await.is_err() {
                                    break;
                                }
                            }
                            None => {
                                // An answer to something we never asked:
                                // the agent's view of the conversation is
                                // not ours anymore.
                                let _ = in_tx
                                    .send(Incoming::Violation(format!(
                                        "response to unknown request id {id}"
                                    )))
                                    .await;
                                break;
                            }
                        }
                    }
                    Msg::Request { id, method, params } => {
                        if method == "session/request_permission" {
                            if in_tx
                                .send(Incoming::Permission { id, params })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        } else {
                            let err = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("{method} is not offered by this client") } });
                            if let Some(tx) = responder.upgrade() {
                                let _ = tx.send(err.to_string()).await;
                            }
                        }
                    }
                    Msg::Notification { method, params } => {
                        if method == "session/update"
                            && in_tx.send(Incoming::Update(params)).await.is_err()
                        {
                            break;
                        }
                    }
                    Msg::Invalid(why) => {
                        let _ = in_tx.send(Incoming::Violation(why)).await;
                        break;
                    }
                }
            }
            // Wake anyone still waiting for a response.
            if let Ok(mut p) = reader_pending.lock() {
                p.clear();
            }
        });

        (
            Client {
                out: out_tx,
                pending,
                next: Arc::new(1.into()),
            },
            in_rx,
        )
    }

    /// Send a request and wait for its response. Returns `Protocol` if the
    /// connection dies first, and the agent's JSON-RPC error verbatim if it
    /// answers with one.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| Error::Protocol("pending map poisoned".into()))?
            .insert(id, tx);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.out
            .send(msg.to_string())
            .await
            .map_err(|_| Error::Protocol("agent stdin closed".into()))?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(Error::Protocol(format!("{method} failed: {e}"))),
            Err(_) => Err(Error::Protocol(format!(
                "connection closed while waiting for {method}"
            ))),
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.out
            .send(msg.to_string())
            .await
            .map_err(|_| Error::Protocol("agent stdin closed".into()))
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<()> {
        let msg = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        self.out
            .send(msg.to_string())
            .await
            .map_err(|_| Error::Protocol("agent stdin closed".into()))
    }
}

pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
        "clientInfo": { "name": "kitsu", "version": env!("CARGO_PKG_VERSION") }
    })
}

enum Msg {
    Response {
        id: u64,
        result: std::result::Result<Value, Value>,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Invalid(String),
}

fn classify(mut v: Value) -> Msg {
    let Some(obj) = v.as_object_mut() else {
        return Msg::Invalid("message is not a JSON object".into());
    };
    let method = obj.get("method").and_then(Value::as_str).map(str::to_owned);
    let id = obj.remove("id");
    match (method, id) {
        (Some(method), Some(id)) => Msg::Request {
            id,
            method,
            params: obj.remove("params").unwrap_or(Value::Null),
        },
        (Some(method), None) => Msg::Notification {
            method,
            params: obj.remove("params").unwrap_or(Value::Null),
        },
        (None, Some(id)) => {
            let Some(id) = id.as_u64() else {
                return Msg::Invalid(format!("response id {id} was never issued by this client"));
            };
            if let Some(err) = obj.remove("error") {
                Msg::Response {
                    id,
                    result: Err(err),
                }
            } else {
                Msg::Response {
                    id,
                    result: Ok(obj.remove("result").unwrap_or(Value::Null)),
                }
            }
        }
        (None, None) => Msg::Invalid("message has neither method nor id".into()),
    }
}

async fn read_line_bounded<R: tokio::io::AsyncBufRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
) -> std::result::Result<usize, String> {
    loop {
        let chunk = r
            .fill_buf()
            .await
            .map_err(|e| format!("reading agent stdout: {e}"))?;
        if chunk.is_empty() {
            return Ok(buf.len());
        }
        if let Some(pos) = chunk.iter().position(|b| *b == b'\n') {
            buf.extend_from_slice(&chunk[..=pos]);
            r.consume(pos + 1);
            return Ok(buf.len());
        }
        let n = chunk.len();
        buf.extend_from_slice(chunk);
        r.consume(n);
        if buf.len() > MAX_LINE {
            return Err(format!("agent sent a line longer than {MAX_LINE} bytes"));
        }
    }
}

/// The parts of a `session/update` Kitsu understands.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    MessageChunk(String),
    /// Model reasoning. Kitsu does not keep its text, only that it happened.
    Thought,
    ToolCall {
        id: String,
        title: Option<String>,
        kind: Option<String>,
        status: Option<String>,
        locations: Vec<String>,
        is_new: bool,
    },
    Plan(Vec<(String, String)>),
    Usage(Value),
    Other(String),
}

pub fn parse_update(params: &Value) -> Update {
    let u = &params["update"];
    let kind = u["sessionUpdate"].as_str().unwrap_or("");
    match kind {
        "agent_message_chunk" => {
            let text = u["content"]["text"].as_str().unwrap_or_default();
            Update::MessageChunk(text.to_string())
        }
        "agent_thought_chunk" => Update::Thought,
        "tool_call" | "tool_call_update" => Update::ToolCall {
            id: u["toolCallId"].as_str().unwrap_or_default().to_string(),
            title: u["title"].as_str().map(str::to_owned),
            kind: u["kind"].as_str().map(str::to_owned),
            status: u["status"].as_str().map(str::to_owned),
            locations: u["locations"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|l| l["path"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            is_new: kind == "tool_call",
        },
        "plan" => Update::Plan(
            u["entries"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            (
                                e["content"].as_str().unwrap_or_default().to_string(),
                                e["status"].as_str().unwrap_or("pending").to_string(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        "usage_update" => Update::Usage(u.clone()),
        other => Update::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_messages() {
        assert!(matches!(
            classify(json!({"jsonrpc":"2.0","id":3,"result":{"ok":1}})),
            Msg::Response {
                id: 3,
                result: Ok(_)
            }
        ));
        assert!(matches!(
            classify(json!({"jsonrpc":"2.0","id":3,"error":{"code":1}})),
            Msg::Response {
                id: 3,
                result: Err(_)
            }
        ));
        assert!(matches!(
            classify(json!({"jsonrpc":"2.0","id":"x","method":"session/request_permission"})),
            Msg::Request { .. }
        ));
        assert!(matches!(
            classify(json!({"jsonrpc":"2.0","method":"session/update","params":{}})),
            Msg::Notification { .. }
        ));
        assert!(matches!(
            classify(json!({"jsonrpc":"2.0","id":"str","result":{}})),
            Msg::Invalid(_)
        ));
        assert!(matches!(classify(json!([1, 2])), Msg::Invalid(_)));
    }

    #[test]
    fn parses_updates_tolerantly() {
        let p = json!({"sessionId":"s","update":{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Read a.rs","kind":"read","status":"pending","locations":[{"path":"/w/a.rs"}],"futureField":1}});
        assert_eq!(
            parse_update(&p),
            Update::ToolCall {
                id: "t1".into(),
                title: Some("Read a.rs".into()),
                kind: Some("read".into()),
                status: Some("pending".into()),
                locations: vec!["/w/a.rs".into()],
                is_new: true
            }
        );
        let p = json!({"update":{"sessionUpdate":"something_from_v3"}});
        assert_eq!(parse_update(&p), Update::Other("something_from_v3".into()));
    }
}
