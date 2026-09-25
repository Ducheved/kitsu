//! The host: everything the brain may ask for, and everything it may not do
//! itself. Tools run here, inside the worktree, under the run's policy; every
//! effect is journaled before it happens; checks decide whether the run is
//! done.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use super::protocol::{self, BRAIN_KINDS};
use super::tools::{self, Effect};
use crate::agents::{AgentSpec, NativeSpec};
use crate::check::{CheckRun, CheckStatus, status_at};
use crate::error::Result;
use crate::git::Git;
use crate::intent::Intent;
use crate::run::RunState;
use crate::runner::{Policy, Wake};
use crate::status::required_checks;
use crate::store::{CheckOutcome, Store, Usage};
use crate::util::content_id;
use crate::workspace::Workspace;

/// How many times `finish` may be refused before the run stops unverified.
pub const MAX_REJECTIONS: u32 = 3;

pub struct Host<'a> {
    pub ws: &'a Workspace,
    pub store: &'a Store,
    pub run: String,
    pub task: String,
    pub base: String,
    pub worktree: PathBuf,
    pub spec: &'a AgentSpec,
    pub native: &'a NativeSpec,
    pub policy: Policy,
    pub echo: bool,
    pub harness: String,
    pub brief: String,
    /// Results of calls already finished, by call id.
    ended: BTreeMap<String, Value>,
    /// `tool.begin` bodies with no end: a process died mid-call.
    begun: BTreeMap<String, Value>,
    /// Earlier runs this one resumes, oldest first; their journals come
    /// before this run's.
    chain: Vec<String>,
    rejections: u32,
    /// Set when the run should end, with the stop reason.
    pub stop: Option<String>,
    plan: Vec<tools::PlanEntry>,
    usage: Usage,
    mcp: crate::mcp::Server,
}

impl<'a> Host<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ws: &'a Workspace,
        store: &'a Store,
        run: &str,
        spec: &'a AgentSpec,
        native: &'a NativeSpec,
        policy: Policy,
        echo: bool,
        harness: String,
        brief: String,
    ) -> Result<Host<'a>> {
        let row = store.run(run)?;
        let mut h = Host {
            ws,
            store,
            run: run.to_string(),
            task: row.task.clone(),
            base: row.base.clone(),
            worktree: PathBuf::from(&row.worktree),
            spec,
            native,
            policy,
            echo,
            harness,
            brief,
            ended: BTreeMap::new(),
            begun: BTreeMap::new(),
            chain: resume_chain(store, run)?,
            rejections: 0,
            stop: None,
            plan: Vec::new(),
            usage: Usage::default(),
            mcp: crate::mcp::Server::new(ws.clone(), run.to_string()),
        };
        for run in h.chain.iter().chain(std::iter::once(&h.run)) {
            for e in all_events(store, run)? {
                let Some(c) = e.body["call"].as_str() else {
                    continue;
                };
                match e.kind.as_str() {
                    "tool.begin" => {
                        h.begun.insert(c.to_string(), e.body.clone());
                    }
                    "tool.end" => {
                        h.begun.remove(c);
                        h.ended.insert(c.to_string(), e.body.clone());
                    }
                    _ => {}
                }
            }
        }
        Ok(h)
    }

    /// One request from the brain, one reply. Never fails: errors go back
    /// as JSON-RPC errors.
    pub async fn serve(&mut self, line: &str, wake: &mut Wake) -> String {
        let req = match protocol::parse(line) {
            Ok(r) => r,
            Err(e) => return protocol::err(&Value::Null, &e),
        };
        let out = match req.method.as_str() {
            protocol::SESSION => self.session(),
            protocol::JOURNAL_READ => self
                .journal(req.params["after_seq"].as_i64().unwrap_or(0))
                .map(|entries| json!({ "entries": entries }))
                .map_err(|e| e.to_string()),
            protocol::JOURNAL_APPEND => self.append(&req.params),
            protocol::STATE => self.state(&req.params).map(|t| json!({ "text": t })),
            protocol::TOOLS_CALL => self.tools_call(&req.params, wake).await.map(|mut v| {
                if let Some(s) = &self.stop {
                    v["_meta"]["kitsu/stop"] = json!(s);
                }
                // The tree after anything that can change files: the brain's
                // measure of progress.
                let name = req.params["name"].as_str().unwrap_or("");
                if tools::effect(name) != Effect::None
                    && let Ok(t) = Git::new(&self.worktree).worktree_tree(&self.ws.scratch())
                {
                    v["_meta"]["kitsu/tree"] = json!(t);
                }
                v
            }),
            protocol::STOP => {
                let reason = req.params["reason"]
                    .as_str()
                    .unwrap_or("stopped")
                    .to_string();
                self.stop.get_or_insert(reason);
                Ok(json!({}))
            }
            other => Err(format!("unknown method `{other}`")),
        };
        match out {
            Ok(v) => protocol::ok(&req.id, v),
            Err(e) => protocol::err(&req.id, &e),
        }
    }

    fn session(&self) -> std::result::Result<Value, String> {
        Ok(json!({
            "run": self.run,
            "prefix": { "harness": self.harness, "brief": self.brief },
            "tools": tools::definitions(),
            "budgets": { "turns": self.native.turns, "tokens": self.native.tokens },
            "model": {
                "provider": self.native.provider, "base_url": self.native.base_url, "model": self.native.model,
                "window": self.native.context_window, "max_output": self.native.max_output,
                "api_key_env": self.native.api_key_env,
            },
        }))
    }

    /// The events a brain folds into its conversation.
    /// The events a brain folds into its conversation: the resumed runs'
    /// first, then this run's.
    fn journal(&self, after: i64) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let runs = self.chain.iter().chain(std::iter::once(&self.run));
        for run in runs {
            for e in all_events(self.store, run)? {
                if e.seq <= after && run == &self.run {
                    continue;
                }
                if matches!(
                    e.kind.as_str(),
                    "model.response" | "tool.end" | "ctx.compacted" | "loop.signal"
                ) {
                    out.push(json!({ "seq": e.seq, "run": run, "kind": e.kind, "body": e.body }));
                }
            }
        }
        Ok(out)
    }

    fn append(&mut self, params: &Value) -> std::result::Result<Value, String> {
        let kind = params["kind"].as_str().unwrap_or("");
        if !BRAIN_KINDS.contains(&kind) {
            return Err(format!("the brain may not write `{kind}` events"));
        }
        let body = &params["body"];
        let seq = self
            .store
            .append(Some(&self.run), kind, body)
            .map_err(|e| e.to_string())?;
        if kind == "model.response" {
            if let Some(t) = body["text"].as_str().filter(|t| !t.trim().is_empty()) {
                if self.echo {
                    eprintln!("{t}");
                }
                let _ = self
                    .store
                    .append(Some(&self.run), "agent.message", &json!({ "text": t }));
            }
            let u = &body["usage"];
            let add = |a: &mut Option<u64>, v: &Value| {
                if let Some(n) = v.as_u64() {
                    *a = Some(a.unwrap_or(0) + n);
                }
            };
            add(&mut self.usage.input, &u["prompt"]);
            add(&mut self.usage.output, &u["completion"]);
            add(&mut self.usage.cached_read, &u["cached"]);
            if let Some(c) = u["cost"].as_f64() {
                self.usage.cost = Some(self.usage.cost.unwrap_or(0.0) + c);
            }
            if u["prompt"].is_u64() {
                self.usage.context_used = u["prompt"].as_u64();
                self.usage.context_size = Some(self.native.context_window);
            }
            let _ = self.store.set_run_usage(&self.run, &self.usage);
        }
        Ok(json!({ "seq": seq }))
    }

    /// The ledger: where the run stands, rendered fresh for every request.
    fn state(&self, params: &Value) -> std::result::Result<String, String> {
        let mut out = String::from(
            "# Kitsu: state of your run (live; replaces any earlier state message)\n\n",
        );
        out.push_str(&format!(
            "Turn {} of {}. Tokens used {} of {}.",
            params["turn"].as_u64().unwrap_or(0),
            self.native.turns,
            params["tokens"].as_u64().unwrap_or(0),
            self.native.tokens,
        ));
        if let Some(c) = params["context"].as_u64() {
            out.push_str(&format!(
                " Context about {c} of {} tokens.",
                self.native.context_window
            ));
        }
        out.push('\n');
        let orient = self
            .mcp
            .tool_text("orient", &Value::Null)
            .map_err(|e| e.to_string())?;
        // orient's pointer to the brief file is for external agents; the
        // brief is already the second system message here.
        let orient: String = orient
            .lines()
            .filter(|l| !l.starts_with("Your full brief:"))
            .map(|l| format!("{l}\n"))
            .collect();
        out.push('\n');
        out.push_str(orient.trim_end());
        out.push('\n');
        if !self.plan.is_empty() {
            out.push_str("\nYour plan (yours, not a rule):\n");
            for p in &self.plan {
                out.push_str(&format!("- [{}] {}\n", p.status, p.content));
            }
        }
        if self.rejections > 0 {
            out.push_str(&format!(
                "\n`finish` was refused {} of {MAX_REJECTIONS} times; at {MAX_REJECTIONS} the run stops unverified.\n",
                self.rejections
            ));
        }
        for w in params["warnings"].as_array().into_iter().flatten() {
            if let Some(w) = w.as_str() {
                out.push_str(&format!("\nWarning: {w}\n"));
            }
        }
        Ok(out)
    }

    async fn tools_call(
        &mut self,
        params: &Value,
        wake: &mut Wake,
    ) -> std::result::Result<Value, String> {
        let name = params["name"].as_str().unwrap_or("").to_string();
        let raw = params["arguments"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| params["arguments"].to_string());
        let call = params["_meta"]["kitsu/call"]
            .as_str()
            .ok_or("tools/call without a `kitsu/call` id")?
            .to_string();
        let implicit = params["_meta"]["kitsu/implicit"].as_bool().unwrap_or(false);
        let turn = params["_meta"]["kitsu/turn"].as_u64();
        if let Some(prev) = self.ended.get(&call) {
            return Ok(result(
                prev["text"].as_str().unwrap_or(""),
                prev["is_error"].as_bool().unwrap_or(false),
                "recorded",
            ));
        }
        let title = title(&name, &raw);
        let tool_id = call.clone();
        let ui = |status: &str| json!({ "id": tool_id, "title": title, "kind": kind(&name), "status": status, "locations": [] });

        // Policy first: a denied call never starts.
        if name == "shell" && self.policy == Policy::Ask {
            match self.ask_human(&title, wake).await? {
                true => {}
                false => {
                    let text = "A human declined this command. Do it another way, or call finish with outcome blocked.".to_string();
                    self.end(&call, &name, "denied", true, &text, implicit, turn)?;
                    let _ = self
                        .store
                        .append(Some(&self.run), "agent.tool", &ui("failed"));
                    return Ok(result(&text, true, "denied"));
                }
            }
        }
        if let Some(begin) = self.begun.get(&call).cloned()
            && let Some((outcome, text)) = self.reconcile(&name, &begin)
        {
            let body = json!({ "call": call, "tool": name, "outcome": outcome, "is_error": false, "text": text, "implicit": implicit, "turn": turn, "reconciled": true });
            self.store
                .append(Some(&self.run), "tool.end", &body)
                .map_err(|e| e.to_string())?;
            self.begun.remove(&call);
            self.ended.insert(call.clone(), body);
            let _ = self
                .store
                .append(Some(&self.run), "agent.tool", &ui("completed"));
            return Ok(result(&text, false, outcome));
        }
        let pre = self.pre(&name, &raw);
        self.store
            .append(
                Some(&self.run),
                "tool.begin",
                &json!({ "call": call, "tool": name, "args": clip(&raw, 4096), "pre": pre }),
            )
            .map_err(|e| e.to_string())?;
        let _ = self
            .store
            .append(Some(&self.run), "agent.tool", &ui("in_progress"));
        if self.echo {
            eprintln!("[tool] {title}");
        }
        fault("before_effect", &name);
        let executed = self.execute(&name, &raw, wake).await;
        fault("after_effect", &name);
        let (text, is_error, outcome) = match executed {
            Ok(t) => (t, false, "done"),
            Err(Failure::Invalid(e)) => (e, true, "invalid"),
            Err(Failure::Failed(e)) => (e, true, "done"),
            Err(Failure::Cancelled(e)) => (e, true, "cancelled"),
        };
        let text = tools::bounded(text);
        if outcome == "cancelled" {
            self.stop.get_or_insert_with(|| "cancelled".into());
        }
        self.end(&call, &name, outcome, is_error, &text, implicit, turn)?;
        let _ = self.store.append(
            Some(&self.run),
            "agent.tool",
            &ui(if is_error { "failed" } else { "completed" }),
        );
        Ok(result(&text, is_error, outcome))
    }

    /// A call that began in a process that died, and never ended. Its
    /// effect is settled from what was recorded before it ran; an effect
    /// that can't be known is reported as unknown and never repeated.
    /// `None`: the call only reads, so it simply runs again.
    fn reconcile(&self, name: &str, begin: &Value) -> Option<(&'static str, String)> {
        let died = "The Kitsu process running this call died before recording its result.";
        match tools::effect(name) {
            Effect::None => None,
            Effect::Worktree => {
                let pre = &begin["pre"];
                let path = pre["path"].as_str().unwrap_or("");
                let now = tools::confine(&self.worktree, path)
                    .ok()
                    .and_then(|p| std::fs::read(p).ok())
                    .map(|b| content_id(&b));
                let (outcome, what) = if now.is_some()
                    && now.as_deref() == pre["sha_after"].as_str()
                {
                    (
                        "applied",
                        format!("{path} has the new content: the change was applied."),
                    )
                } else if now.as_deref() == pre["sha_before"].as_str() {
                    (
                        "not_applied",
                        format!(
                            "{path} is unchanged: the change was not applied. Call it again if you still want it."
                        ),
                    )
                } else {
                    (
                        "unknown",
                        format!(
                            "{path} matches neither the content before nor the intended content after. Read it before changing it."
                        ),
                    )
                };
                Some((outcome, format!("{died} {what} It was not repeated.\n")))
            }
            Effect::Process => {
                let git = Git::new(&self.worktree);
                let changed = match (
                    begin["pre"]["tree_before"].as_str(),
                    git.worktree_tree(&self.ws.scratch()),
                ) {
                    (Some(before), Ok(now)) => git.changed_paths(before, &now).unwrap_or_default(),
                    _ => Vec::new(),
                };
                let files = if changed.is_empty() {
                    "No files changed since it started.".to_string()
                } else {
                    format!("Files changed since it started: {}.", changed.join(", "))
                };
                Some((
                    "unknown",
                    format!(
                        "{died} Whether it finished, and what it printed, is unknown; it was not run again. {files}\n"
                    ),
                ))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn end(
        &mut self,
        call: &str,
        tool: &str,
        outcome: &str,
        is_error: bool,
        text: &str,
        implicit: bool,
        turn: Option<u64>,
    ) -> std::result::Result<(), String> {
        let body = json!({ "call": call, "tool": tool, "outcome": outcome, "is_error": is_error, "text": text, "implicit": implicit, "turn": turn });
        self.store
            .append(Some(&self.run), "tool.end", &body)
            .map_err(|e| e.to_string())?;
        self.ended.insert(call.to_string(), body);
        Ok(())
    }

    /// What a write will change, computed before it happens, so a crash in
    /// between can be settled by looking at the file.
    fn pre(&self, name: &str, raw: &str) -> Value {
        let sha = |p: &Path| std::fs::read(p).ok().map(|b| content_id(&b));
        match name {
            "write_file" => match tools::args::<tools::WriteFile>(raw) {
                Ok(a) => match tools::confine(&self.worktree, &a.path) {
                    Ok(p) => {
                        json!({ "path": a.path, "sha_before": sha(&p), "sha_after": content_id(a.content.as_bytes()) })
                    }
                    Err(_) => Value::Null,
                },
                Err(_) => Value::Null,
            },
            "edit_file" => match tools::args::<tools::EditFile>(raw) {
                Ok(a) => match tools::confine(&self.worktree, &a.path) {
                    Ok(p) => {
                        let before = std::fs::read_to_string(&p).ok();
                        let after = before
                            .as_deref()
                            .and_then(|t| tools::apply_edit(t, &a.old, &a.new, a.replace_all).ok())
                            .map(|(t, _)| content_id(t.as_bytes()));
                        json!({ "path": a.path, "sha_before": before.map(|t| content_id(t.as_bytes())), "sha_after": after })
                    }
                    Err(_) => Value::Null,
                },
                Err(_) => Value::Null,
            },
            _ if tools::effect(name) == Effect::Process => {
                json!({ "tree_before": Git::new(&self.worktree).worktree_tree(&self.ws.scratch()).ok() })
            }
            _ => Value::Null,
        }
    }

    async fn execute(
        &mut self,
        name: &str,
        raw: &str,
        wake: &mut Wake,
    ) -> std::result::Result<String, Failure> {
        use Failure::{Failed, Invalid};
        let wt = self.worktree.clone();
        let git = Git::new(&wt);
        match name {
            "read_file" => {
                let a: tools::ReadFile = tools::args(raw).map_err(Invalid)?;
                let p = tools::confine(&wt, &a.path).map_err(Invalid)?;
                let bytes = std::fs::read(&p).map_err(|e| Failed(format!("{}: {e}", a.path)))?;
                if bytes.contains(&0) {
                    return Err(Failed(format!(
                        "{} looks binary ({} bytes); not shown",
                        a.path,
                        bytes.len()
                    )));
                }
                Ok(tools::numbered(
                    &String::from_utf8_lossy(&bytes),
                    a.start_line.unwrap_or(1),
                    a.end_line,
                ))
            }
            "list_files" => {
                let a: tools::ListFiles = tools::args(raw).map_err(Invalid)?;
                let mut argv = vec!["ls-files", "-z", "-c", "-o", "--exclude-standard"];
                let path = a.path.clone().unwrap_or_default();
                if !path.is_empty() {
                    tools::confine(&wt, &path).map_err(Invalid)?;
                    argv.push("--");
                    argv.push(&path);
                }
                let listed = git.run(argv).map_err(|e| Failed(e.to_string()))?;
                let limit = a.limit.unwrap_or(500).clamp(1, 1000);
                let mut files: Vec<&str> = listed
                    .split('\0')
                    .filter(|f| !f.is_empty())
                    .filter(|f| {
                        a.glob
                            .as_deref()
                            .is_none_or(|g| crate::scope::glob_match(g, f))
                    })
                    .collect();
                files.sort_unstable();
                files.dedup();
                let total = files.len();
                let mut out: String = files.iter().take(limit).map(|f| format!("{f}\n")).collect();
                if total > limit {
                    out.push_str(&format!(
                        "[kitsu: {limit} of {total} files; narrow with path or glob]\n"
                    ));
                } else if total == 0 {
                    out.push_str("(no files)\n");
                }
                Ok(out)
            }
            "grep" => {
                let a: tools::Grep = tools::args(raw).map_err(Invalid)?;
                let mut argv: Vec<String> = ["grep", "-n", "-I", "--untracked", "--no-color"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                if a.fixed {
                    argv.push("-F".into());
                } else {
                    argv.push("-E".into());
                }
                if a.ignore_case {
                    argv.push("-i".into());
                }
                argv.push("-e".into());
                argv.push(a.pattern.clone());
                if let Some(p) = a.path.as_deref().filter(|p| !p.is_empty()) {
                    tools::confine(&wt, p).map_err(Invalid)?;
                    argv.push("--".into());
                    argv.push(p.to_string());
                }
                let out = tokio::process::Command::new("git")
                    .args(&argv)
                    .current_dir(&wt)
                    .stdin(Stdio::null())
                    .output()
                    .await
                    .map_err(|e| Failed(format!("git grep: {e}")))?;
                match out.status.code() {
                    Some(0) => {
                        let text = String::from_utf8_lossy(&out.stdout);
                        let lines: Vec<&str> = text.lines().collect();
                        let mut s: String = lines
                            .iter()
                            .take(200)
                            .map(|l| {
                                let l: String = l.chars().take(300).collect();
                                format!("{l}\n")
                            })
                            .collect();
                        if lines.len() > 200 {
                            s.push_str(&format!(
                                "[kitsu: 200 of {} matches; narrow the pattern or path]\n",
                                lines.len()
                            ));
                        }
                        Ok(s)
                    }
                    Some(1) => Ok("No matches.\n".into()),
                    _ => Err(Failed(
                        String::from_utf8_lossy(&out.stderr).trim().to_string(),
                    )),
                }
            }
            "search" | "rules_for" => {
                let v: Value = serde_json::from_str(if raw.trim().is_empty() { "{}" } else { raw })
                    .map_err(|e| Invalid(format!("invalid arguments: {e}")))?;
                if name == "search" {
                    let _: tools::Search = tools::args(raw).map_err(Invalid)?;
                } else {
                    let _: tools::PathOnly = tools::args(raw).map_err(Invalid)?;
                }
                self.mcp
                    .tool_text(name, &v)
                    .map_err(|e| Failed(e.to_string()))
            }
            "git_diff" => {
                let a: tools::GitDiff = tools::args(raw).map_err(Invalid)?;
                let tree = git
                    .worktree_tree(&self.ws.scratch())
                    .map_err(|e| Failed(e.to_string()))?;
                if a.stat {
                    let stats = git
                        .numstat(&self.base, &tree)
                        .map_err(|e| Failed(e.to_string()))?;
                    if stats.is_empty() {
                        return Ok("No changes since your base commit.\n".into());
                    }
                    return Ok(stats
                        .iter()
                        .map(|s| {
                            format!(
                                "{}\t+{} -{}\n",
                                s.path,
                                s.added.map_or("?".into(), |n| n.to_string()),
                                s.removed.map_or("?".into(), |n| n.to_string())
                            )
                        })
                        .collect());
                }
                let paths: Vec<&str> = a.path.as_deref().into_iter().collect();
                let d = git
                    .diff(&self.base, &tree, &paths)
                    .map_err(|e| Failed(e.to_string()))?;
                Ok(if d.trim().is_empty() {
                    "No changes since your base commit.\n".into()
                } else {
                    d
                })
            }
            "edit_file" => {
                let a: tools::EditFile = tools::args(raw).map_err(Invalid)?;
                let p = tools::confine(&wt, &a.path).map_err(Invalid)?;
                let before =
                    std::fs::read_to_string(&p).map_err(|e| Failed(format!("{}: {e}", a.path)))?;
                let (after, n) =
                    tools::apply_edit(&before, &a.old, &a.new, a.replace_all).map_err(Failed)?;
                write_atomic(&p, after.as_bytes()).map_err(Failed)?;
                Ok(format!(
                    "Edited {} ({n} replacement{}), from line {}.\n",
                    a.path,
                    if n == 1 { "" } else { "s" },
                    tools::first_changed_line(&before, &after)
                ))
            }
            "write_file" => {
                let a: tools::WriteFile = tools::args(raw).map_err(Invalid)?;
                let p = tools::confine(&wt, &a.path).map_err(Invalid)?;
                let existed = p.exists();
                if let Some(dir) = p.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| Failed(format!("{}: {e}", a.path)))?;
                }
                write_atomic(&p, a.content.as_bytes()).map_err(Failed)?;
                Ok(format!(
                    "{} {} ({} lines).\n",
                    if existed { "Replaced" } else { "Created" },
                    a.path,
                    a.content.lines().count()
                ))
            }
            "shell" => {
                let a: tools::Shell = tools::args(raw).map_err(Invalid)?;
                self.shell(
                    &a.command,
                    a.timeout_secs.unwrap_or(120).clamp(1, 600),
                    wake,
                )
                .await
            }
            "run_check" => {
                let a: tools::RunCheck = tools::args(raw).map_err(Invalid)?;
                let intent = Intent::load_dir(&self.ws.root).map_err(|e| Failed(e.to_string()))?;
                let def = intent.config.checks.get(&a.name).ok_or_else(|| {
                    Invalid(format!(
                        "no check `{}` (there are: {})",
                        a.name,
                        intent
                            .config
                            .checks
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })?;
                if !self.ws.is_trusted().map_err(|e| Failed(e.to_string()))? {
                    return Err(Failed(
                        "this repository isn't trusted, so Kitsu runs none of its checks".into(),
                    ));
                }
                let ev = CheckRun {
                    ws: self.ws,
                    store: self.store,
                    dir: &wt,
                    run: Some(&self.run),
                }
                .execute(def)
                .map_err(|e| Failed(e.to_string()))?;
                Ok(format!(
                    "Check `{}`: {} (evidence {}, tree {}).\n{}",
                    a.name,
                    ev.outcome.as_str(),
                    ev.id,
                    &ev.tree[..ev.tree.len().min(12)],
                    self.log_tail(ev.log.as_deref(), 8 * 1024)
                ))
            }
            "update_plan" => {
                let a: tools::UpdatePlan = tools::args(raw).map_err(Invalid)?;
                let entries: Vec<Value> = a
                    .entries
                    .iter()
                    .map(|e| json!({ "content": e.content, "status": e.status }))
                    .collect();
                let _ = self.store.append(
                    Some(&self.run),
                    "agent.plan",
                    &json!({ "entries": entries }),
                );
                self.plan = a.entries;
                Ok("Plan recorded; it's shown in the state message.\n".into())
            }
            "finish" => {
                let a: tools::Finish = tools::args(raw).map_err(Invalid)?;
                self.finish(&a)
            }
            other => Err(Invalid(format!("unknown tool `{other}`"))),
        }
    }

    fn log_tail(&self, log: Option<&str>, max: usize) -> String {
        let Some(id) = log else { return String::new() };
        let bytes = self.ws.blobs().get(id).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        if text.trim().is_empty() {
            return String::new();
        }
        let start = text.len().saturating_sub(max);
        let mut i = start;
        while !text.is_char_boundary(i) {
            i += 1;
        }
        format!(
            "--- log{} ---\n{}",
            if start > 0 { " (tail)" } else { "" },
            &text[i..]
        )
    }

    /// Done is decided here, by the checks the task requires on the files as
    /// they are now, never by the model saying so.
    fn finish(&mut self, a: &tools::Finish) -> std::result::Result<String, Failure> {
        let f = |e: crate::error::Error| Failure::Failed(e.to_string());
        if a.outcome == "blocked" {
            self.stop = Some("blocked".into());
            return Ok(format!("Stopped as blocked: {}\n", a.summary.trim()));
        }
        if a.outcome != "done" {
            return Err(Failure::Invalid(format!(
                "outcome must be done or blocked, not `{}`",
                a.outcome
            )));
        }
        let git = Git::new(&self.worktree);
        let tree = git.worktree_tree(&self.ws.scratch()).map_err(f)?;
        let changed = git.changed_paths(&self.base, &tree).map_err(f)?;
        if changed.is_empty() {
            self.stop = Some("unverified".into());
            return Ok(
                "You changed no files, so there is nothing to verify. Stopped; a human decides.\n"
                    .into(),
            );
        }
        let intent = Intent::load_dir(&self.ws.root).map_err(f)?;
        let Some(task) = intent.tasks.get(&self.task) else {
            return Err(Failure::Failed(format!(
                "task `{}` is gone from the main checkout",
                self.task
            )));
        };
        let reqs = required_checks(&intent, task, Some(&changed));
        if reqs.is_empty() {
            self.stop = Some("unverified".into());
            return Ok("No checks are required for this task, so a human will judge the change by reading it. Stopped.\n".into());
        }
        if !self.ws.is_trusted().map_err(f)? {
            self.stop = Some("unverified".into());
            return Ok(
                "This repository isn't trusted, so its checks don't run. Stopped unverified.\n"
                    .into(),
            );
        }
        let mut failing = Vec::new();
        let mut passed = Vec::new();
        for r in &reqs {
            let def = &intent.config.checks[&r.name];
            let reuse = match status_at(&git, self.store, def, &tree).map_err(f)? {
                CheckStatus::Current {
                    outcome: CheckOutcome::Pass,
                    evidence,
                } => Some(evidence),
                _ => None,
            };
            match reuse {
                Some(ev) => passed.push(format!("`{}` pass (evidence {ev})", r.name)),
                None => {
                    let ev = CheckRun {
                        ws: self.ws,
                        store: self.store,
                        dir: &self.worktree,
                        run: Some(&self.run),
                    }
                    .execute(def)
                    .map_err(f)?;
                    if ev.outcome == CheckOutcome::Pass {
                        passed.push(format!("`{}` pass (evidence {})", r.name, ev.id));
                    } else {
                        failing.push(format!(
                            "`{}` {} (evidence {}), required by {}\n{}",
                            r.name,
                            ev.outcome.as_str(),
                            ev.id,
                            r.why.join(", "),
                            self.log_tail(ev.log.as_deref(), 2048)
                        ));
                    }
                }
            }
        }
        let short = &tree[..tree.len().min(12)];
        if failing.is_empty() {
            self.stop = Some("verified".into());
            return Ok(format!(
                "Verified on tree {short}: {}. Stopped; a human reviews and accepts.\n",
                passed.join(", ")
            ));
        }
        self.rejections += 1;
        if self.rejections >= MAX_REJECTIONS {
            self.stop = Some("unverified".into());
        }
        let mut out = format!(
            "Not done: {} of {} required checks fail on tree {short}.{}\n\n",
            failing.len(),
            reqs.len(),
            if self.stop.is_some() {
                format!(
                    " That was refusal {MAX_REJECTIONS} of {MAX_REJECTIONS}; the run stops unverified."
                )
            } else {
                " Fix it and call finish again.".into()
            }
        );
        for x in failing {
            out.push_str(&x);
            out.push('\n');
        }
        Ok(out)
    }

    async fn shell(
        &self,
        command: &str,
        timeout: u64,
        wake: &mut Wake,
    ) -> std::result::Result<String, Failure> {
        let limits = crate::agents::limits().map_err(|e| Failure::Failed(e.to_string()))?;
        let (argv, _) = crate::agents::limited(
            &["sh".to_string(), "-c".to_string(), command.to_string()],
            limits,
            &crate::agents::allowed_cpus(),
            crate::agents::on_path,
        );
        let mut cmd = tokio::process::Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .current_dir(&self.worktree)
            .env_clear()
            .envs(
                crate::agents::agent_env(self.spec, std::env::vars())
                    .into_iter()
                    .filter(|(k, _)| k != &self.native.api_key_env),
            )
            .env("KITSU_RUN", &self.run)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd
            .spawn()
            .map_err(|e| Failure::Failed(format!("could not start sh: {e}")))?;
        let _ = self.store.set_run_pid(&self.run, child.id());
        let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
            return Err(Failure::Failed("no pipes".into()));
        };
        let readers = async { tokio::join!(capture(out), capture(err)) };
        tokio::pin!(readers);
        let deadline = tokio::time::sleep(Duration::from_secs(timeout));
        tokio::pin!(deadline);
        let mut tick = tokio::time::interval(crate::runner::TICK);
        let (stdout, stderr, why) = loop {
            tokio::select! {
                (o, e) = &mut readers => break (o, e, None),
                _ = &mut deadline => break (Captured::default(), Captured::default(), Some(format!("timed out after {timeout}s; killed"))),
                _ = wake.recv() => if self.stopping() { break (Captured::default(), Captured::default(), Some("cancelled; killed".into())) },
                _ = tick.tick() => if self.stopping() { break (Captured::default(), Captured::default(), Some("cancelled; killed".into())) },
            }
        };
        if let Some(why) = why {
            crate::runner::kill_group(&mut child).await;
            let _ = self.store.set_run_pid(&self.run, None);
            return Err(if why.starts_with("cancelled") {
                Failure::Cancelled(format!("Command {why}."))
            } else {
                Failure::Failed(format!("Command {why}."))
            });
        }
        let status = child
            .wait()
            .await
            .map_err(|e| Failure::Failed(e.to_string()))?;
        let _ = self.store.set_run_pid(&self.run, None);
        let mut text = format!(
            "exit {}\n",
            status.code().map_or("signal".into(), |c| c.to_string())
        );
        if !stdout.text.is_empty() {
            text.push_str(&format!(
                "--- stdout{} ---\n{}\n",
                stdout.cut_note(),
                stdout.text
            ));
        }
        if !stderr.text.is_empty() {
            text.push_str(&format!(
                "--- stderr{} ---\n{}\n",
                stderr.cut_note(),
                stderr.text
            ));
        }
        if status.success() {
            Ok(text)
        } else {
            Err(Failure::Failed(text))
        }
    }

    fn stopping(&self) -> bool {
        self.store
            .run(&self.run)
            .map(|r| r.state == RunState::Stopping)
            .unwrap_or(false)
    }

    /// A human decides, through the same asks the ACP path uses.
    async fn ask_human(&self, title: &str, wake: &mut Wake) -> std::result::Result<bool, String> {
        let request = json!({
            "title": title, "kind": "execute", "locations": [],
            "options": [
                { "optionId": "allow_once", "name": "Allow once", "kind": "allow_once" },
                { "optionId": "reject_once", "name": "Reject", "kind": "reject_once" },
            ],
        });
        let ask = self
            .store
            .insert_ask(&self.run, &request)
            .map_err(|e| e.to_string())?;
        if self.echo {
            eprintln!(
                "kitsu: agent asks: {title}\n       answer with: kitsu answer {ask} <option>   (options: allow_once, reject_once)"
            );
        }
        let mut tick = tokio::time::interval(crate::runner::TICK);
        loop {
            if let Some(a) = self.store.ask(ask).map_err(|e| e.to_string())?.answer {
                let yes = a == "allow_once";
                let _ = self.store.append(
                    Some(&self.run),
                    "permission",
                    &json!({ "title": title, "kind": "execute", "decision": a, "by": "human" }),
                );
                return Ok(yes);
            }
            if self.stopping() {
                let _ = self.store.answer_ask(ask, "cancelled");
                return Ok(false);
            }
            tokio::select! {
                _ = wake.recv() => {}
                _ = tick.tick() => {}
            }
        }
    }
}

pub enum Failure {
    /// The arguments were wrong; nothing ran.
    Invalid(String),
    /// It ran, or tried to, and failed.
    Failed(String),
    Cancelled(String),
}

fn result(text: &str, is_error: bool, outcome: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error, "_meta": { "kitsu/outcome": outcome } })
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    format!("{}…[{} bytes]", &s[..i], s.len())
}

fn kind(tool: &str) -> &'static str {
    match tool {
        "read_file" | "list_files" | "git_diff" => "read",
        "grep" | "search" | "rules_for" => "search",
        "edit_file" | "write_file" => "edit",
        "shell" | "run_check" => "execute",
        _ => "other",
    }
}

fn title(tool: &str, raw: &str) -> String {
    let v: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let s = |k: &str| {
        v[k].as_str()
            .unwrap_or("")
            .chars()
            .take(120)
            .collect::<String>()
    };
    match tool {
        "read_file" => format!("Read {}", s("path")),
        "list_files" => format!(
            "List {}",
            if s("path").is_empty() {
                "files".into()
            } else {
                s("path")
            }
        ),
        "grep" => format!("Grep {}", s("pattern")),
        "search" => format!("Search {}", s("query")),
        "rules_for" => format!("Rules for {}", s("path")),
        "git_diff" => "Diff".into(),
        "edit_file" => format!("Edit {}", s("path")),
        "write_file" => format!("Write {}", s("path")),
        "shell" => format!("Run `{}`", s("command")),
        "run_check" => format!("Check {}", s("name")),
        "update_plan" => "Plan".into(),
        "finish" => format!("Finish ({})", s("outcome")),
        other => other.to_string(),
    }
}

/// Write via a temp file and rename, so a crash leaves the old content or
/// the new, never half of either.
fn write_atomic(p: &Path, bytes: &[u8]) -> std::result::Result<(), String> {
    let tmp = p.with_file_name(format!(
        ".{}.kitsu-tmp",
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
    std::fs::rename(&tmp, p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", p.display())
    })
}

#[derive(Default)]
struct Captured {
    text: String,
    total: usize,
}

impl Captured {
    fn cut_note(&self) -> String {
        if self.total > self.text.len() + 64 {
            format!(" ({} bytes, head and tail)", self.total)
        } else {
            String::new()
        }
    }
}

/// Read a pipe to its end, keeping 8 KiB of head and 16 KiB of tail: a
/// command that prints gigabytes costs time, not memory.
async fn capture(mut r: impl tokio::io::AsyncRead + Unpin) -> Captured {
    const HEAD: usize = 8 * 1024;
    const TAIL: usize = 16 * 1024;
    let mut head = Vec::new();
    let mut tail: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
    let mut total = 0usize;
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n;
                let mut chunk = &buf[..n];
                if head.len() < HEAD {
                    let take = (HEAD - head.len()).min(chunk.len());
                    head.extend_from_slice(&chunk[..take]);
                    chunk = &chunk[take..];
                }
                tail.extend(chunk);
                while tail.len() > TAIL {
                    tail.pop_front();
                }
            }
        }
    }
    let mut text = String::from_utf8_lossy(&head).into_owned();
    if !tail.is_empty() {
        let tail: Vec<u8> = tail.into_iter().collect();
        if total > head.len() + tail.len() {
            text.push_str(&format!(
                "\n[kitsu: {} bytes cut from the middle]\n",
                total - head.len() - tail.len()
            ));
        }
        text.push_str(&String::from_utf8_lossy(&tail));
    }
    Captured {
        text: text.trim_end().to_string(),
        total,
    }
}

/// Earlier runs that `run` resumes, oldest first. A run resumes its
/// `from_run` when it recorded `run.resume`; the chain follows that back.
fn resume_chain(store: &Store, run: &str) -> Result<Vec<String>> {
    let mut chain = Vec::new();
    let mut cur = run.to_string();
    loop {
        let resumes = all_events(store, &cur)?
            .iter()
            .any(|e| e.kind == "run.resume");
        let from = store.run(&cur)?.from_run;
        match from {
            Some(prev) if resumes && !chain.contains(&prev) && prev != run => {
                chain.push(prev.clone());
                cur = prev;
            }
            _ => break,
        }
    }
    chain.reverse();
    Ok(chain)
}

fn all_events(store: &Store, run: &str) -> Result<Vec<crate::store::EventRow>> {
    let mut out = Vec::new();
    let mut seq = 0;
    loop {
        let page = store.run_events(run, seq, 1000)?;
        let Some(last) = page.last() else { break };
        seq = last.seq;
        let full = page.len() == 1000;
        out.extend(page);
        if !full {
            break;
        }
    }
    Ok(out)
}

/// Test-only crash points: `KITSU_FAULT=before_effect:<tool>` or
/// `after_effect:<tool>` aborts the process there, the way a power cut or
/// a kill -9 would, so resume can be tested against a real crash.
fn fault(point: &str, tool: &str) {
    if std::env::var("KITSU_FAULT").is_ok_and(|f| f == format!("{point}:{tool}")) {
        std::process::abort();
    }
}
