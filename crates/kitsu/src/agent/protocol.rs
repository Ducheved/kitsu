//! The boundary between the brain (the loop that calls the model) and the
//! host (tools, journal, checks, the run's state).
//!
//! Only JSON-RPC strings cross it, even in process, so a brain elsewhere
//! (another process, a cloud service) later needs a transport and nothing
//! else. The method names are ACP extension methods (`_kitsu/*`); tool
//! payloads have MCP's `tools/call` shape.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

pub const SESSION: &str = "_kitsu/session";
pub const JOURNAL_READ: &str = "_kitsu/journal/read";
pub const JOURNAL_APPEND: &str = "_kitsu/journal/append";
pub const STATE: &str = "_kitsu/state";
pub const TOOLS_CALL: &str = "_kitsu/tools/call";
pub const STOP: &str = "_kitsu/stop";

/// Journal kinds a brain may write. Everything else (tool effects, run
/// state, evidence) is the host's to record.
pub const BRAIN_KINDS: &[&str] = &[
    "model.response",
    "model.error",
    "ctx.compacted",
    "loop.signal",
];

pub type Wire = (String, oneshot::Sender<String>);

/// The brain's end of the in-process transport.
pub struct HostLink {
    tx: mpsc::Sender<Wire>,
    next: AtomicU64,
}

impl HostLink {
    pub fn new(tx: mpsc::Sender<Wire>) -> HostLink {
        HostLink {
            tx,
            next: AtomicU64::new(1),
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let line =
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        let (back, rx) = oneshot::channel();
        self.tx
            .send((line, back))
            .await
            .map_err(|_| "the host is gone".to_string())?;
        let reply = rx
            .await
            .map_err(|_| "the host dropped the request".to_string())?;
        let v: Value = serde_json::from_str(&reply).map_err(|e| format!("bad reply: {e}"))?;
        if v["id"] != json!(id) {
            return Err(format!("reply for another request: {}", v["id"]));
        }
        match v.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
            None => Ok(v["result"].clone()),
        }
    }
}

pub struct Request {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

pub fn parse(line: &str) -> Result<Request, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    let method = v["method"].as_str().ok_or("no method")?.to_string();
    Ok(Request {
        id: v["id"].clone(),
        method,
        params: v["params"].clone(),
    })
}

pub fn ok(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

pub fn err(id: &Value, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": message } })
        .to_string()
}
