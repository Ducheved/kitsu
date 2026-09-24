//! `kitsu mcp`: a Model Context Protocol server an agent can ask instead of
//! grepping around.
//!
//! Kitsu doesn't run the agent's loop (decision `acp-client-not-harness`).
//! What it can do is answer the questions an agent otherwise spends tokens
//! and shell calls on: where is X, what rules cover this file, what is my
//! task and what counts as done, which of my checks are stale now. Every
//! answer is derived from state Kitsu owns, deterministically, with no
//! model involved.
//!
//! Read-only. Nothing here changes intent, runs checks or writes files, so
//! there's no effect whose outcome could be unknown after a crash. Rules
//! come from the main checkout, the same ones accept will judge by; the
//! agent's own copy of `.kitsu/` in its worktree is not authority.
//!
//! Transport: newline-delimited JSON-RPC 2.0 on stdio. Started per run by
//! the runner through ACP `session/new` `mcpServers`.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::check::status_at;
use crate::error::{Error, Result};
use crate::git::Git;
use crate::index::Index;
use crate::intent::{Intent, MemoryState, QuestionState};
use crate::memory::{self, Freshness};
use crate::scope::Scope;
use crate::status::required_checks;
use crate::store::Store;
use crate::workspace::Workspace;

/// Protocol revisions this server speaks; the newest is offered when the
/// client asks for one we don't know.
const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// A tool result larger than this is cut, and says so.
const MAX_RESULT: usize = 24 * 1024;

pub struct Server {
    ws: Workspace,
    run: String,
}

impl Server {
    pub fn new(ws: Workspace, run: String) -> Server {
        Server { ws, run }
    }

    /// Serve until stdin closes.
    pub fn serve(&self, input: impl BufRead, mut output: impl Write) -> Result<()> {
        for line in input.lines() {
            let line = line.map_err(|e| Error::io("mcp stdin", e))?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(
                    json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } }),
                ),
            };
            if let Some(r) = reply {
                writeln!(output, "{r}").map_err(|e| Error::io("mcp stdout", e))?;
                output.flush().map_err(|e| Error::io("mcp stdout", e))?;
            }
        }
        Ok(())
    }

    /// One message in, at most one message out (notifications get none).
    pub fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned()?;
        let method = msg["method"].as_str().unwrap_or("");
        let result = match method {
            "initialize" => {
                let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
                let version = VERSIONS
                    .iter()
                    .find(|v| **v == asked)
                    .unwrap_or(&VERSIONS[0]);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "kitsu", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Kitsu knows this run's task, the rules that apply, what counts as done and which checks are current. Ask it before searching the repository by hand. Everything it says is derived from recorded state; notes it returns are advisory.",
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => Ok(self.call(&msg["params"])),
            other => Err((-32601, format!("method not found: {other}"))),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        })
    }

    fn call(&self, params: &Value) -> Value {
        let name = params["name"].as_str().unwrap_or("");
        let args = &params["arguments"];
        let out = match name {
            "search" => self.search(args),
            "orient" => self.orient(),
            "rules_for" => self.rules_for(args),
            "brief" => self.brief(),
            "memory" => self.memory(),
            other => Err(Error::Invalid(format!("unknown tool `{other}`"))),
        };
        match out {
            Ok(text) => json!({ "content": [{ "type": "text", "text": bounded(text) }] }),
            // A tool error is a result the model can read, not a protocol
            // error, per MCP.
            Err(e) => {
                json!({ "content": [{ "type": "text", "text": format!("error: {e}") }], "isError": true })
            }
        }
    }

    fn store(&self) -> Result<Store> {
        self.ws.open_store()
    }

    fn worktree(&self) -> Result<Git> {
        let run = self.store()?.run(&self.run)?;
        Ok(Git::new(&run.worktree))
    }

    fn search(&self, args: &Value) -> Result<String> {
        let query = args["query"].as_str().unwrap_or("").trim();
        if query.is_empty() {
            return Err(Error::Invalid("`query` is required".into()));
        }
        let limit = args["limit"].as_u64().unwrap_or(10).clamp(1, 30) as usize;
        let run = self.store()?.run(&self.run)?;
        let git = self.ws.git();
        let mut ix = Index::open(&self.ws.state.join("index.db"))?;
        ix.update(&git, &run.base)?;
        let hits = ix.search(&git, query, limit)?;
        let mut out = format!(
            "Searched the code at your base commit {} (your own edits aren't indexed).\n",
            &run.base[..run.base.len().min(12)]
        );
        if hits.is_empty() {
            out.push_str("No matches. Try other words or an identifier.\n");
        }
        for h in hits {
            out.push_str(&format!("\n{}:{}-{}\n", h.path, h.start, h.end));
            for (n, l) in h.lines {
                out.push_str(&format!("  {n}: {l}\n"));
            }
        }
        Ok(out)
    }

    /// Where am I: the task, what done means and where each required check
    /// stands on the worktree as it is right now, what I've changed, what's
    /// unanswered. For re-orienting after the agent's own context was
    /// compacted.
    fn orient(&self) -> Result<String> {
        let store = self.store()?;
        let run = store.run(&self.run)?;
        let intent = Intent::load_dir(&self.ws.root)?;
        let task = intent
            .tasks
            .get(&run.task)
            .ok_or_else(|| Error::NotFound(format!("task {}", run.task)))?;
        let wt = self.worktree()?;
        let tree = wt.worktree_tree(&self.ws.scratch())?;
        let changed = wt.changed_paths(&run.base, &tree)?;
        let mut out = format!(
            "Task `{}`: {}\nRun {} by {}, based on {}.\n",
            task.id,
            task.title,
            run.id,
            run.agent,
            &run.base[..run.base.len().min(12)]
        );

        out.push_str(&format!(
            "\nYou have changed {} file(s) since the base",
            changed.len()
        ));
        if changed.is_empty() {
            out.push_str(".\n");
        } else {
            out.push_str(":\n");
            for p in changed.iter().take(40) {
                out.push_str(&format!("- {p}\n"));
            }
            if changed.len() > 40 {
                out.push_str(&format!("- and {} more\n", changed.len() - 40));
            }
        }

        out.push_str("\nDone means these checks pass on your change (Kitsu runs them itself when you finish):\n");
        let reqs = required_checks(&intent, task, Some(&changed));
        if reqs.is_empty() {
            out.push_str("- no checks are defined; a human will judge by reading the change\n");
        }
        for r in &reqs {
            let def = &intent.config.checks[&r.name];
            let st = status_at(&wt, &store, def, &tree)?;
            out.push_str(&format!(
                "- `{}` ({}): {} on your current files; required by {}\n",
                r.name,
                def.run,
                st.word(),
                r.why.join(", ")
            ));
        }

        let open: Vec<_> = intent.open_questions_blocking(&task.id).collect();
        if !open.is_empty() {
            out.push_str("\nOpen questions about this task (don't guess; stop and say so if you depend on one):\n");
            for q in open {
                out.push_str(&format!("- {} (`{}`)\n", q.title, q.id));
            }
        }
        let answered: Vec<_> = intent
            .questions
            .values()
            .filter(|q| q.state == QuestionState::Answered && q.blocks.contains(&task.id))
            .collect();
        for q in answered {
            out.push_str(&format!(
                "- answered: {} — {}\n",
                q.title,
                q.answer.as_deref().unwrap_or("").trim()
            ));
        }

        if let Some(path) = run.brief.as_deref().map(|b| self.ws.blobs().path(b)) {
            out.push_str(&format!(
                "\nYour full brief: {} (or the `brief` tool).\n",
                path.display()
            ));
        }
        Ok(out)
    }

    fn rules_for(&self, args: &Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_start_matches("./");
        if path.is_empty() {
            return Err(Error::Invalid("`path` is required".into()));
        }
        let intent = Intent::load_dir(&self.ws.root)?;
        let here = Scope::new([path.to_string()]);
        let mut out = format!(
            "Rules that cover `{path}` (from the main checkout, which is what accept checks against):\n"
        );
        let mut any = false;
        for c in intent
            .config
            .checks
            .values()
            .filter(|c| !c.guards.is_everything() && c.guards.may_overlap(&here))
        {
            any = true;
            out.push_str(&format!(
                "\n- must pass: check `{}` guards it (`{}`)\n",
                c.name, c.run
            ));
            for l in excerpt(c.why.as_deref().unwrap_or(""), 8) {
                out.push_str(&format!("  {l}\n"));
            }
        }
        for d in intent
            .decisions
            .values()
            .filter(|d| !d.scope.is_everything() && d.scope.may_overlap(&here))
        {
            any = true;
            out.push_str(&format!("\n- decision: {} (`{}`)\n", d.title, d.id));
            for l in excerpt(&d.body, 8) {
                out.push_str(&format!("  {l}\n"));
            }
            for r in &d.rejected {
                out.push_str(&format!("  rejected: {r}\n"));
            }
        }
        for c in intent.config.checks.values().filter(|c| {
            c.guards.is_everything() && !c.scope.is_everything() && c.scope.may_overlap(&here)
        }) {
            any = true;
            out.push_str(&format!("\n- check `{}` covers it: `{}`\n", c.name, c.run));
        }
        if !any {
            out.push_str("\nNo decision or check names this path specifically. Repository-wide rules are in your brief.\n");
        }
        Ok(out)
    }

    fn brief(&self) -> Result<String> {
        let run = self.store()?.run(&self.run)?;
        let blob = run
            .brief
            .ok_or_else(|| Error::NotFound(format!("brief of run {}", self.run)))?;
        Ok(String::from_utf8_lossy(&self.ws.blobs().get(&blob)?).into_owned())
    }

    fn memory(&self) -> Result<String> {
        let store = self.store()?;
        let run = store.run(&self.run)?;
        let intent = Intent::load_dir(&self.ws.root)?;
        let task = intent
            .tasks
            .get(&run.task)
            .ok_or_else(|| Error::NotFound(format!("task {}", run.task)))?;
        let fresh = memory::freshness(&self.ws.git(), &run.base, intent.memory.values())?;
        let superseded = intent.superseded();
        let mut out = String::from(
            "Notes earlier work left, for this task's scope. They are what someone believed, with where it came from; if one disagrees with a rule or the code, the rule or the code wins.\n",
        );
        let mut n = 0;
        for m in intent
            .memory
            .values()
            .filter(|m| m.scope.is_everything() || m.scope.may_overlap(&task.scope))
        {
            let state = if m.state == MemoryState::Retired {
                "retired".to_string()
            } else if let Some(by) = superseded.get(&m.id) {
                format!("superseded by {by}")
            } else {
                match fresh.get(&m.id) {
                    Some(Freshness::Stale { changed, .. }) => {
                        format!("possibly out of date: {} changed since", changed.join(", "))
                    }
                    Some(Freshness::Uncommitted) => "not committed".into(),
                    _ => "current".into(),
                }
            };
            n += 1;
            out.push_str(&format!(
                "\n- {} ({}, `{}`, {state}; {})\n",
                m.title,
                m.kind.as_str(),
                m.id,
                m.source.path
            ));
            for l in excerpt(&m.body, 6) {
                out.push_str(&format!("  {l}\n"));
            }
        }
        if n == 0 {
            out.push_str("\nNone.\n");
        }
        Ok(out)
    }
}

fn excerpt(body: &str, lines: usize) -> Vec<&str> {
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .take(lines)
        .collect()
}

fn bounded(mut text: String) -> String {
    if text.len() > MAX_RESULT {
        let mut cut = MAX_RESULT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        let total = text.len();
        text.truncate(cut);
        text.push_str(&format!("\n[truncated: showed {cut} of {total} bytes]\n"));
    }
    text
}

fn tools() -> Value {
    let none = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    json!([
        {
            "name": "orient",
            "description": "Where you stand: your task, the checks that decide 'done' and whether each passes on your current files, what you've changed, open questions. Call it at the start and whenever you've lost track (for example after your context was compacted).",
            "inputSchema": none,
            "annotations": { "readOnlyHint": true, "idempotentHint": true },
        },
        {
            "name": "search",
            "description": "Full-text search over the repository at your base commit. Finds identifiers split into words (retryBudget matches 'retry budget') and ranks files whose path matches. Returns file:line ranges with the matching lines. Cheaper than reading files to find something.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words or identifiers" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 30, "default": 10 }
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "idempotentHint": true },
        },
        {
            "name": "rules_for",
            "description": "The checks and decisions that cover a path, from the rules accept will judge by. Ask before changing a file you don't know.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Repository-relative path or glob" } },
                "required": ["path"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "idempotentHint": true },
        },
        {
            "name": "brief",
            "description": "The full brief this run was started with.",
            "inputSchema": none,
            "annotations": { "readOnlyHint": true, "idempotentHint": true },
        },
        {
            "name": "memory",
            "description": "Notes earlier work left for this task's scope, with whether each is current, possibly out of date, retired or superseded. Advisory: rules and code win over notes.",
            "inputSchema": none,
            "annotations": { "readOnlyHint": true, "idempotentHint": true },
        },
    ])
}

/// How the runner offers this server to an agent in ACP `session/new`.
pub fn acp_server_entry(kitsu: &std::path::Path, worktree: &std::path::Path, run: &str) -> Value {
    json!({
        "name": "kitsu",
        "command": kitsu.display().to_string(),
        "args": ["-C", worktree.display().to_string(), "mcp", "--run", run],
        "env": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_results_say_they_were_cut() {
        let s = bounded("é".repeat(MAX_RESULT));
        assert!(s.contains("[truncated: showed"));
        assert!(s.len() < MAX_RESULT + 100);
        assert_eq!(bounded("short".into()), "short");
    }

    #[test]
    fn every_tool_is_read_only_and_schemas_are_closed() {
        for t in tools().as_array().expect("tools") {
            assert_eq!(t["annotations"]["readOnlyHint"], true, "{}", t["name"]);
            assert_eq!(
                t["inputSchema"]["additionalProperties"], false,
                "{}",
                t["name"]
            );
        }
    }
}
