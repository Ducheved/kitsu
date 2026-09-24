//! `kitsu-test-agent`: a scripted ACP agent.
//!
//! It exists so the whole loop (brief, worktree, permissions, cancel,
//! snapshot, checks, accept) can be exercised and demoed without a model or
//! an API key, and so failure cases can be produced on purpose: crashing
//! mid-turn, hanging, ignoring cancel, answering twice, sending garbage.
//!
//! The script is a TOML file named by `KITSU_TEST_SCRIPT`:
//!
//! ```toml
//! [[steps]]
//! say = "Looking at the retry loop."
//! [[steps]]
//! replace = { path = "src/client.rs", find = "loop {", with = "for _ in 0..3 {" }
//! [[steps]]
//! ask = { title = "Run the tests", kind = "execute" }
//! [[steps]]
//! shell = "cargo test"
//! ```
//!
//! A top-level `usage = { input = 1200, output = 300, used = 5000, size = 200000, cost = 0.02 }`
//! makes it report token usage the way ACP agents do: a `usage_update`
//! before the turn ends and a `usage` object on the prompt response.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Script {
    #[serde(default)]
    ignore_cancel: bool,
    #[serde(default)]
    duplicate_response: bool,
    #[serde(default)]
    steps: Vec<Step>,
    usage: Option<ScriptUsage>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct ScriptUsage {
    input: Option<u64>,
    output: Option<u64>,
    cached_read: Option<u64>,
    used: Option<u64>,
    size: Option<u64>,
    cost: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    say: Option<String>,
    plan: Option<Vec<String>>,
    read: Option<String>,
    write: Option<WriteFile>,
    replace: Option<Replace>,
    ask: Option<Ask>,
    shell: Option<String>,
    sleep_ms: Option<u64>,
    #[serde(default)]
    crash: bool,
    #[serde(default)]
    hang: bool,
    #[serde(default)]
    garbage: bool,
    stop: Option<String>,
    /// Call a tool on the first MCP server offered in `session/new` and say
    /// its text result: `mcp = { tool = "orient" }`.
    mcp: Option<McpCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct McpCall {
    tool: String,
    #[serde(default)]
    args: toml::Table,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteFile {
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Replace {
    path: String,
    find: String,
    with: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ask {
    title: String,
    kind: Option<String>,
}

struct Agent {
    rx: Receiver<Value>,
    cwd: PathBuf,
    next_id: u64,
    session: String,
    cancelled: bool,
    script: Script,
    mcp_servers: Vec<Value>,
}

fn send(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

impl Agent {
    fn update(&self, update: Value) {
        send(
            &json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": self.session, "update": update } }),
        );
    }

    fn say(&self, text: &str) {
        // Stream in small chunks like a real model would.
        for chunk in text.as_bytes().chunks(24) {
            self.update(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": String::from_utf8_lossy(chunk) } }));
        }
    }

    fn tool(&self, id: &str, title: &str, kind: &str, status: &str, path: Option<&str>) {
        let locations: Vec<Value> = path
            .map(|p| vec![json!({ "path": self.cwd.join(p).display().to_string() })])
            .unwrap_or_default();
        let kind_of = if status == "pending" {
            "tool_call"
        } else {
            "tool_call_update"
        };
        self.update(json!({ "sessionUpdate": kind_of, "toolCallId": id, "title": title, "kind": kind, "status": status, "locations": locations }));
    }

    /// Drain notifications; note cancel. Returns other messages.
    fn poll(&mut self, wait: Duration) -> Option<Value> {
        match self.rx.recv_timeout(wait) {
            Ok(msg) => {
                if msg["method"] == "session/cancel" {
                    self.cancelled = true;
                    None
                } else {
                    Some(msg)
                }
            }
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => std::process::exit(0),
        }
    }

    fn should_stop(&mut self) -> bool {
        while let Ok(msg) = self.rx.try_recv() {
            if msg["method"] == "session/cancel" {
                self.cancelled = true;
            }
        }
        self.cancelled && !self.script.ignore_cancel
    }

    fn ask(&mut self, title: &str, kind: &str) -> bool {
        let id = self.next_id;
        self.next_id += 1;
        let call = format!("perm-{id}");
        send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "session/request_permission",
            "params": {
                "sessionId": self.session,
                "toolCall": { "toolCallId": call, "title": title, "kind": kind },
                "options": [
                    { "optionId": "allow", "name": "Allow", "kind": "allow_once" },
                    { "optionId": "deny", "name": "Deny", "kind": "reject_once" }
                ]
            }
        }));
        loop {
            if let Some(msg) = self.poll(Duration::from_millis(50))
                && msg["id"] == id
            {
                return msg["result"]["outcome"]["optionId"] == "allow";
            }
        }
    }

    fn run_turn(&mut self) -> String {
        let steps = std::mem::take(&mut self.script.steps);
        for (n, step) in steps.iter().enumerate() {
            if self.should_stop() {
                return "cancelled".into();
            }
            let tid = format!("t{n}");
            if let Some(s) = &step.say {
                self.say(s);
            }
            if let Some(p) = &step.plan {
                let entries: Vec<Value> = p
                    .iter()
                    .map(|c| json!({ "content": c, "priority": "medium", "status": "pending" }))
                    .collect();
                self.update(json!({ "sessionUpdate": "plan", "entries": entries }));
            }
            if let Some(path) = &step.read {
                self.tool(&tid, &format!("Read {path}"), "read", "pending", Some(path));
                let ok = std::fs::read(self.cwd.join(path)).is_ok();
                self.tool(
                    &tid,
                    &format!("Read {path}"),
                    "read",
                    if ok { "completed" } else { "failed" },
                    Some(path),
                );
            }
            if let Some(w) = &step.write {
                self.tool(
                    &tid,
                    &format!("Write {}", w.path),
                    "edit",
                    "pending",
                    Some(&w.path),
                );
                let p = self.cwd.join(&w.path);
                if let Some(parent) = p.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let ok = std::fs::write(&p, &w.content).is_ok();
                self.tool(
                    &tid,
                    &format!("Write {}", w.path),
                    "edit",
                    if ok { "completed" } else { "failed" },
                    Some(&w.path),
                );
            }
            if let Some(r) = &step.replace {
                self.tool(
                    &tid,
                    &format!("Edit {}", r.path),
                    "edit",
                    "pending",
                    Some(&r.path),
                );
                let p = self.cwd.join(&r.path);
                let ok = match std::fs::read_to_string(&p) {
                    Ok(text) if text.contains(&r.find) => {
                        std::fs::write(&p, text.replacen(&r.find, &r.with, 1)).is_ok()
                    }
                    _ => false,
                };
                self.tool(
                    &tid,
                    &format!("Edit {}", r.path),
                    "edit",
                    if ok { "completed" } else { "failed" },
                    Some(&r.path),
                );
            }
            if let Some(a) = &step.ask
                && !self.ask(&a.title, a.kind.as_deref().unwrap_or("execute"))
            {
                self.say("Permission denied, stopping here.");
                return if self.cancelled {
                    "cancelled".into()
                } else {
                    "end_turn".into()
                };
            }
            if let Some(cmd) = &step.shell {
                self.tool(
                    &tid,
                    &format!("Run `{cmd}`"),
                    "execute",
                    "in_progress",
                    None,
                );
                let ok = std::process::Command::new("sh")
                    .args(["-c", cmd])
                    .current_dir(&self.cwd)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                self.tool(
                    &tid,
                    &format!("Run `{cmd}`"),
                    "execute",
                    if ok { "completed" } else { "failed" },
                    None,
                );
            }
            if let Some(ms) = step.sleep_ms {
                let until = std::time::Instant::now() + Duration::from_millis(ms);
                while std::time::Instant::now() < until {
                    let _ = self.poll(Duration::from_millis(20));
                    if self.cancelled && !self.script.ignore_cancel {
                        return "cancelled".into();
                    }
                }
            }
            if step.garbage {
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{{this is not json");
                let _ = out.flush();
            }
            if step.crash {
                std::process::exit(3);
            }
            if step.hang {
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
            }
            if let Some(call) = &step.mcp {
                let text = match self.mcp_servers.first() {
                    Some(server) => call_mcp(server, &call.tool, &call.args),
                    None => "no MCP server was offered".into(),
                };
                self.say(&format!("[mcp {}]\n{text}", call.tool));
            }
            if let Some(reason) = &step.stop {
                return reason.clone();
            }
        }
        if self.cancelled && !self.script.ignore_cancel {
            "cancelled".into()
        } else {
            "end_turn".into()
        }
    }
}

/// Start an MCP server the way an agent would (command + args from
/// `session/new`), initialize, call one tool, return its text.
fn call_mcp(server: &Value, tool: &str, args: &toml::Table) -> String {
    use std::io::{BufReader, Write as _};
    let command = server["command"].as_str().unwrap_or("");
    let argv: Vec<String> = server["args"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut child = match std::process::Command::new(command)
        .args(&argv)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return format!("could not start MCP server: {e}"),
    };
    let mut stdin = child.stdin.take().expect("stdin");
    let mut out = BufReader::new(child.stdout.take().expect("stdout"));
    let args = serde_json::to_value(args).unwrap_or(json!({}));
    for m in [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "kitsu-test-agent", "version": "0" } } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": tool, "arguments": args } }),
    ] {
        let _ = writeln!(stdin, "{m}");
    }
    drop(stdin);
    let mut text = String::new();
    let mut line = String::new();
    while out.read_line(&mut line).unwrap_or(0) > 0 {
        if let Ok(v) = serde_json::from_str::<Value>(&line)
            && v["id"] == 2
        {
            text = v["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
            if v["result"]["isError"] == true {
                text = format!("tool error: {text}");
            }
        }
        line.clear();
    }
    let _ = child.wait();
    text
}

fn main() {
    let script: Script = match std::env::var_os("KITSU_TEST_SCRIPT") {
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str(&t).map_err(|e| e.to_string()))
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "kitsu-test-agent: bad script {}: {e}",
                    PathBuf::from(p).display()
                );
                std::process::exit(2);
            }
        },
        None => Script::default(),
    };
    // Bounded: if the script is busy, stop reading instead of buffering.
    let (tx, rx) = mpsc::sync_channel(64);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<Value>(&line) {
                Ok(v) => {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
                Err(e) => eprintln!("kitsu-test-agent: bad input: {e}"),
            }
        }
    });
    let mut agent = Agent {
        rx,
        cwd: std::env::current_dir().unwrap_or_default(),
        next_id: 1000,
        session: String::new(),
        cancelled: false,
        script,
        mcp_servers: Vec::new(),
    };
    loop {
        let Some(msg) = agent.poll(Duration::from_secs(3600)) else {
            continue;
        };
        let id = msg["id"].clone();
        match msg["method"].as_str() {
            Some("initialize") => send(&json!({ "jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": 1,
                "agentCapabilities": { "loadSession": false, "promptCapabilities": { "image": false, "audio": false, "embeddedContext": false } },
                "agentInfo": { "name": "kitsu-test-agent", "version": env!("CARGO_PKG_VERSION") },
                "authMethods": []
            }})),
            Some("session/new") => {
                if let Some(cwd) = msg["params"]["cwd"].as_str() {
                    agent.cwd = PathBuf::from(cwd);
                }
                agent.mcp_servers = msg["params"]["mcpServers"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                agent.session = format!("s-{}", std::process::id());
                send(
                    &json!({ "jsonrpc": "2.0", "id": id, "result": { "sessionId": agent.session } }),
                );
            }
            Some("session/prompt") => {
                if agent.script.steps.is_empty() {
                    agent.say("No script given (set KITSU_TEST_SCRIPT). Nothing to do.");
                }
                let reason = agent.run_turn();
                let mut result = json!({ "stopReason": reason });
                if let Some(u) = agent.script.usage.clone() {
                    let mut update = json!({ "sessionUpdate": "usage_update", "used": u.used.unwrap_or(0), "size": u.size.unwrap_or(0) });
                    if let Some(c) = u.cost {
                        update["cost"] = json!({ "amount": c, "currency": "USD" });
                    }
                    agent.update(update);
                    let (i, o) = (u.input.unwrap_or(0), u.output.unwrap_or(0));
                    result["usage"] = json!({ "inputTokens": i, "outputTokens": o, "cachedReadTokens": u.cached_read, "totalTokens": i + o });
                }
                let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
                send(&response);
                if agent.script.duplicate_response {
                    send(&response);
                }
            }
            Some(other) if !id.is_null() => {
                send(
                    &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("{other} not supported") } }),
                );
            }
            _ => {}
        }
    }
}
