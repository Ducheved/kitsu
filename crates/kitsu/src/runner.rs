//! One run, owned by one process.
//!
//! `kitsu run` is the custodian of exactly one agent attempt: it records the
//! intent, makes the worktree, starts the agent, answers it, snapshots what
//! it did and runs the checks. The desktop app starts runs by launching this
//! same command, so closing the app doesn't kill agents, and a terminal user
//! gets the exact same behavior as a UI user.
//!
//! Ordering is always: write down what we're about to do, do it, write down
//! that it happened. Whatever crashes in between, recovery can tell which
//! side of the effect it's on (see `recover.rs`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::process::Command;

use crate::acp::{self, Client, Incoming, Update};
use crate::agents::AgentSpec;
use crate::brief::{self, Context};
use crate::check::CheckRun;
use crate::error::{Error, Result};
use crate::git::Git;
use crate::intent::{Intent, TaskState};
use crate::run::{RunEvent, RunState};
use crate::status::required_checks;
use crate::store::{Applied, NewRun, Store};
use crate::util::short_id;
use crate::workspace::{Instance, Workspace};

const INIT_TIMEOUT: Duration = Duration::from_secs(180);
const CANCEL_GRACE: Duration = Duration::from_secs(15);

/// `KITSU_CANCEL_GRACE_MS` shortens the grace period; the test suite uses it
/// so the "agent ignores cancel" case doesn't take 15 seconds.
fn cancel_grace() -> Duration {
    std::env::var("KITSU_CANCEL_GRACE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(CANCEL_GRACE)
}
const EXIT_GRACE: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Reads and edits inside the worktree are allowed; anything else (shell
    /// commands, network, paths outside the worktree) waits for a human.
    Ask,
    /// Everything inside the worktree is allowed, including commands.
    /// Anything touching paths outside it still waits for a human.
    Auto,
}

impl Policy {
    pub fn parse(s: &str) -> Option<Policy> {
        match s {
            "ask" => Some(Policy::Ask),
            "auto" => Some(Policy::Auto),
            _ => None,
        }
    }
}

pub struct Options {
    pub id: Option<String>,
    pub task: String,
    pub agent: AgentSpec,
    pub from: Option<String>,
    pub note: Option<String>,
    pub policy: Policy,
    pub verify: bool,
}

/// Everything before the agent starts. Synchronous and quick.
pub struct Prepared {
    pub id: String,
    pub worktree: PathBuf,
    pub brief_path: PathBuf,
    pub brief: String,
}

pub fn prepare(ws: &Workspace, store: &Store, me: &Instance, opts: &Options) -> Result<Prepared> {
    if !ws.is_trusted()? {
        return Err(Error::Denied(format!(
            "{} is not trusted yet. Agents and checks run code from this repository; run `kitsu trust` once if that's fine.",
            ws.root.display()
        )));
    }
    let intent = Intent::load_dir(&ws.root)?;
    let task = intent
        .tasks
        .get(&opts.task)
        .ok_or_else(|| Error::NotFound(format!("task {}", opts.task)))?;
    if task.state != TaskState::Open {
        return Err(Error::Invalid(format!("task {} is not open", task.id)));
    }
    let git = ws.git();
    let base = match &opts.from {
        Some(from) => {
            let prev = store.run(from)?;
            if prev.task != task.id {
                return Err(Error::Invalid(format!(
                    "run {from} was for task {}, not {}",
                    prev.task, task.id
                )));
            }
            prev.snapshot.ok_or_else(|| {
                Error::Invalid(format!("run {from} has no snapshot to continue from"))
            })?
        }
        None => git
            .head()?
            .ok_or_else(|| Error::Invalid("the repository has no commits yet".into()))?,
    };

    let id = opts.id.clone().unwrap_or_else(|| short_id('r'));
    crate::intent::valid_id(&id).map_err(Error::Invalid)?;
    let worktree = ws.worktree_for(&id);
    let branch = format!("kitsu/run/{id}");
    let wt_str = worktree.display().to_string();
    store.insert_run(&NewRun {
        id: &id,
        task: &task.id,
        agent: &opts.agent.name,
        base: &base,
        branch: &branch,
        worktree: &wt_str,
        owner: &me.id,
        from_run: opts.from.as_deref(),
        note: opts.note.as_deref(),
    })?;

    let started = (|| {
        {
            let _guard = ws.lock_worktrees()?;
            git.worktree_add(&worktree, &branch, &base)?;
        }
        let run = store.run(&id)?;
        let b = brief::compile(
            &Context {
                intent: &intent,
                git: Some(&git),
                store: Some(store),
                base: Some(&base),
                worktree: Some(&wt_str),
                run: Some(&run),
                budget: brief::DEFAULT_BUDGET,
            },
            task,
        );
        let blob = ws.blobs().put(b.markdown.as_bytes())?;
        store.set_run_brief(&id, &blob)?;
        store.append(Some(&id), "run.brief", &json!({ "blob": blob, "included": b.included, "omitted": b.omitted, "problems": b.problems }))?;
        let dir = run_dir(ws, &id);
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.display().to_string(), e))?;
        let brief_path = dir.join("brief.md");
        std::fs::write(&brief_path, &b.markdown)
            .map_err(|e| Error::io(brief_path.display().to_string(), e))?;
        Ok::<_, Error>((brief_path, b.markdown))
    })();
    match started {
        Ok((brief_path, brief)) => Ok(Prepared {
            id,
            worktree,
            brief_path,
            brief,
        }),
        Err(e) => {
            let _ = store.apply_run_event(&id, &RunEvent::StartFailed(e.to_string()));
            Err(e)
        }
    }
}

pub fn run_dir(ws: &Workspace, id: &str) -> PathBuf {
    ws.state.join("runs").join(id)
}

/// Start the agent, drive one prompt turn, wind it down. Returns after the
/// run reached a terminal state.
pub async fn drive(
    ws: &Workspace,
    store: &Store,
    prep: &Prepared,
    opts: &Options,
    echo: bool,
) -> Result<RunState> {
    let id = prep.id.as_str();
    let log_path = run_dir(ws, id).join("agent.log");
    let log = std::fs::File::create(&log_path)
        .map_err(|e| Error::io(log_path.display().to_string(), e))?;

    let (program, args) = opts
        .agent
        .command
        .split_first()
        .ok_or_else(|| Error::Invalid("empty agent command".into()))?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(&prep.worktree)
        .envs(&opts.agent.env)
        .env("KITSU_RUN", id)
        .env("KITSU_TASK", &opts.task)
        .env("KITSU_BRIEF", &prep.brief_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log))
        .kill_on_drop(true);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        cmd.env_remove(var);
    }
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let detail = format!("could not start `{}`: {e}", opts.agent.command.join(" "));
            store.apply_run_event(id, &RunEvent::StartFailed(detail.clone()))?;
            return Err(Error::Invalid(detail));
        }
    };
    store.set_run_pid(id, child.id())?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Invalid("agent stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Invalid("agent stdout".into()))?;
    let (client, mut incoming) = Client::start(stdin, stdout);
    let mut rec = Recorder::new(id, &prep.worktree, echo);

    // ---- handshake ----
    let session = match handshake(&client, &prep.worktree, &mut incoming).await {
        Ok(s) => s,
        Err(e) => {
            let detail = format!("{e} (agent log: {})", log_path.display());
            store.apply_run_event(id, &RunEvent::StartFailed(detail))?;
            shut_down(&mut child).await;
            return Ok(store.run(id)?.state);
        }
    };
    let cancelled_early = matches!(
        store.apply_run_event(id, &RunEvent::Started)?,
        Applied::Unchanged
    );

    // ---- the turn ----
    let prompt_client = client.clone();
    let prompt_params =
        json!({ "sessionId": session, "prompt": [{ "type": "text", "text": prep.brief }] });
    let mut prompt =
        tokio::spawn(async move { prompt_client.request("session/prompt", prompt_params).await });

    let mut asks: Vec<PendingAsk> = Vec::new();
    let mut cancel_sent: Option<Instant> = None;
    let mut version = store.data_version()?;
    let mut tick = tokio::time::interval(TICK);
    if cancelled_early {
        send_cancel(&client, &session, store, &mut asks, &mut cancel_sent).await?;
    }

    let end: RunEvent = loop {
        // Biased toward the agent's stream: updates and protocol errors that
        // arrived before the prompt response are handled before it.
        tokio::select! {
            biased;
            msg = incoming.recv() => match msg {
                Some(Incoming::Update(p)) => rec.update(acp::parse_update(&p)),
                Some(Incoming::Permission { id: rpc, params }) => {
                    rec.flush(store)?;
                    let decision = decide(opts.policy, &params, &prep.worktree);
                    let title = params["toolCall"]["title"].as_str().unwrap_or("(untitled)").to_string();
                    match decision {
                        Decision::Answer(option, why) => {
                            store.append(Some(id), "permission", &json!({ "title": title, "kind": params["toolCall"]["kind"], "decision": option, "by": why }))?;
                            client.respond(rpc, json!({ "outcome": { "outcome": "selected", "optionId": option } })).await?;
                        }
                        Decision::Human => {
                            if cancel_sent.is_some() {
                                client.respond(rpc, json!({ "outcome": { "outcome": "cancelled" } })).await?;
                            } else {
                                let request = json!({
                                    "title": title,
                                    "kind": params["toolCall"]["kind"],
                                    "locations": params["toolCall"]["locations"],
                                    "options": params["options"],
                                });
                                let ask = store.insert_ask(id, &request)?;
                                if echo {
                                    eprintln!("kitsu: agent asks: {title}\n       answer with: kitsu answer {ask} <option>   (options: {})", option_ids(&params).join(", "));
                                }
                                asks.push(PendingAsk { ask, rpc });
                            }
                        }
                    }
                }
                Some(Incoming::Violation(v)) => break RunEvent::ProtocolError(v),
                Some(Incoming::DuplicateResponse(rpc)) => {
                    store.append(Some(id), "protocol.duplicate_response", &json!({ "request": rpc }))?;
                }
                Some(Incoming::Closed) | None => {
                    let status = tokio::time::timeout(EXIT_GRACE, child.wait()).await;
                    break RunEvent::Exited(match status {
                        Ok(Ok(s)) => format!("agent closed its output and exited ({s}) before finishing the turn"),
                        _ => "agent closed its output before finishing the turn".into(),
                    });
                }
            },
            res = &mut prompt => {
                break match res {
                    Ok(Ok(v)) => match v["stopReason"].as_str() {
                        Some(r) => RunEvent::TurnEnded(r.to_string()),
                        None => RunEvent::ProtocolError(format!("session/prompt returned without stopReason: {v}")),
                    },
                    Ok(Err(e)) => match child.try_wait() {
                        Ok(Some(status)) => RunEvent::Exited(format!("agent exited ({status}) during the turn: {e}")),
                        _ => RunEvent::ProtocolError(e.to_string()),
                    },
                    Err(join) => RunEvent::ProtocolError(format!("prompt task failed: {join}")),
                };
            }
            _ = tick.tick() => {
                rec.flush_if_due(store)?;
                let v = store.data_version()?;
                if v != version {
                    version = v;
                    let run = store.run(id)?;
                    if run.state == RunState::Stopping && cancel_sent.is_none() {
                        send_cancel(&client, &session, store, &mut asks, &mut cancel_sent).await?;
                    }
                    answer_asks(&client, store, &mut asks).await?;
                }
                if let Some(at) = cancel_sent
                    && at.elapsed() > cancel_grace() {
                        kill_group(&mut child).await;
                        break RunEvent::Exited(format!("agent did not stop within {}s of cancel; killed", cancel_grace().as_secs_f32()));
                    }
            }
        }
    };
    prompt.abort();
    let applied = store.apply_run_event(id, &end)?;
    if applied == Applied::Duplicate && echo {
        eprintln!("kitsu: agent reported completion twice; recorded");
    }
    // Close our end so the agent sees EOF, then read whatever it still had
    // to say until it closes its end: trailing updates, or a second answer
    // to session/prompt, which is recorded and not believed.
    drop(client);
    let until = Instant::now() + EXIT_GRACE;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, incoming.recv()).await {
            Ok(Some(Incoming::Update(p))) => rec.update(acp::parse_update(&p)),
            Ok(Some(Incoming::Violation(v))) => {
                store.append(Some(id), "protocol.violation", &json!({ "detail": v }))?;
            }
            Ok(Some(Incoming::DuplicateResponse(rpc))) => {
                store.append(
                    Some(id),
                    "protocol.duplicate_response",
                    &json!({ "request": rpc }),
                )?;
            }
            Ok(Some(Incoming::Permission { .. })) => {}
            Ok(Some(Incoming::Closed) | None) | Err(_) => break,
        }
    }
    rec.finish(store)?;
    drop(incoming);
    shut_down(&mut child).await;
    let exit = match child.try_wait() {
        Ok(Some(s)) => format!("exited ({s})"),
        _ => "exited".into(),
    };
    store.apply_run_event(id, &RunEvent::Exited(exit))?;
    store.set_run_pid(id, None)?;
    Ok(store.run(id)?.state)
}

async fn handshake(
    client: &Client,
    worktree: &Path,
    incoming: &mut tokio::sync::mpsc::Receiver<Incoming>,
) -> Result<String> {
    let init = with_violations(
        incoming,
        tokio::time::timeout(
            INIT_TIMEOUT,
            client.request("initialize", acp::initialize_params()),
        ),
    )
    .await?;
    let version = init["protocolVersion"].as_u64();
    if version != Some(acp::PROTOCOL_VERSION) {
        return Err(Error::Protocol(format!(
            "agent speaks ACP protocol version {version:?}; kitsu speaks {}",
            acp::PROTOCOL_VERSION
        )));
    }
    let cwd = worktree.display().to_string();
    let session = with_violations(
        incoming,
        tokio::time::timeout(
            INIT_TIMEOUT,
            client.request("session/new", json!({ "cwd": cwd, "mcpServers": [] })),
        ),
    )
    .await?;
    session["sessionId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Protocol(format!("session/new returned no sessionId: {session}")))
}

/// Await a handshake request while still watching for protocol violations
/// and a dead agent, so a broken agent fails fast instead of timing out.
async fn with_violations<F>(
    incoming: &mut tokio::sync::mpsc::Receiver<Incoming>,
    fut: F,
) -> Result<Value>
where
    F: std::future::Future<
            Output = std::result::Result<Result<Value>, tokio::time::error::Elapsed>,
        >,
{
    tokio::pin!(fut);
    loop {
        tokio::select! {
            r = &mut fut => return match r {
                Ok(v) => v,
                Err(_) => Err(Error::Protocol(format!("agent did not answer within {}s", INIT_TIMEOUT.as_secs()))),
            },
            msg = incoming.recv() => match msg {
                Some(Incoming::Violation(v)) => return Err(Error::Protocol(v)),
                Some(Incoming::Closed) | None => return Err(Error::Protocol("agent exited during startup".into())),
                Some(_) => {}
            }
        }
    }
}

struct PendingAsk {
    ask: i64,
    rpc: Value,
}

async fn send_cancel(
    client: &Client,
    session: &str,
    store: &Store,
    asks: &mut Vec<PendingAsk>,
    sent: &mut Option<Instant>,
) -> Result<()> {
    client
        .notify("session/cancel", json!({ "sessionId": session }))
        .await?;
    *sent = Some(Instant::now());
    // The spec says every pending permission request gets `cancelled`.
    for a in asks.drain(..) {
        let _ = store.answer_ask(a.ask, "cancelled")?;
        client
            .respond(a.rpc, json!({ "outcome": { "outcome": "cancelled" } }))
            .await?;
    }
    Ok(())
}

async fn answer_asks(client: &Client, store: &Store, asks: &mut Vec<PendingAsk>) -> Result<()> {
    let mut still = Vec::new();
    for a in asks.drain(..) {
        match store.ask(a.ask)?.answer {
            Some(answer) if answer == "cancelled" => {
                client
                    .respond(a.rpc, json!({ "outcome": { "outcome": "cancelled" } }))
                    .await?
            }
            Some(option) => {
                client
                    .respond(
                        a.rpc,
                        json!({ "outcome": { "outcome": "selected", "optionId": option } }),
                    )
                    .await?
            }
            None => still.push(a),
        }
    }
    *asks = still;
    Ok(())
}

enum Decision {
    Answer(String, &'static str),
    Human,
}

fn option_ids(params: &Value) -> Vec<String> {
    params["options"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| o["optionId"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn decide(policy: Policy, params: &Value, worktree: &Path) -> Decision {
    let kind = params["toolCall"]["kind"].as_str().unwrap_or("other");
    let outside = params["toolCall"]["locations"]
        .as_array()
        .map(|locs| {
            locs.iter()
                .filter_map(|l| l["path"].as_str())
                .any(|p| is_outside(p, worktree))
        })
        .unwrap_or(false);
    let allow = params["options"].as_array().and_then(|opts| {
        let pick = |k: &str| {
            opts.iter()
                .find(|o| o["kind"] == k)
                .and_then(|o| o["optionId"].as_str())
                .map(str::to_owned)
        };
        pick("allow_once").or_else(|| pick("allow_always"))
    });
    let Some(allow) = allow else {
        return Decision::Human;
    };
    if outside {
        return Decision::Human;
    }
    let local = matches!(
        kind,
        "read" | "search" | "think" | "edit" | "delete" | "move"
    );
    match policy {
        Policy::Auto => Decision::Answer(allow, "policy auto"),
        Policy::Ask if local => Decision::Answer(allow, "policy: inside worktree"),
        Policy::Ask => Decision::Human,
    }
}

fn is_outside(path: &str, worktree: &Path) -> bool {
    let p = Path::new(path);
    if p.is_relative() {
        return p
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    }
    // Lexical check. Symlinks inside the worktree can still point out; this
    // is an approval heuristic, not a sandbox, and the UI says so.
    let norm: PathBuf = p.components().fold(PathBuf::new(), |mut acc, c| {
        match c {
            std::path::Component::ParentDir => {
                acc.pop();
            }
            std::path::Component::CurDir => {}
            other => acc.push(other),
        }
        acc
    });
    !norm.starts_with(worktree)
}

async fn shut_down(child: &mut tokio::process::Child) {
    if tokio::time::timeout(EXIT_GRACE, child.wait())
        .await
        .is_err()
    {
        kill_group(child).await;
    }
}

async fn kill_group(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .status()
            .await;
    }
    let _ = child.kill().await;
}

/// Everything after the agent is gone: capture the worktree, run checks.
pub fn finish(ws: &Workspace, store: &Store, id: &str, verify: bool) -> Result<Option<String>> {
    let run = store.run(id)?;
    let wt = PathBuf::from(&run.worktree);
    if !wt.exists() {
        return Ok(None);
    }
    let git = Git::new(&wt);
    let intent = Intent::load_dir(&ws.root)?;
    let title = intent
        .tasks
        .get(&run.task)
        .map(|t| t.title.clone())
        .unwrap_or_else(|| run.task.clone());
    let snapshot = git.snapshot(
        &format!("{title}\n\nKitsu-Run: {id}\nKitsu-Task: {}", run.task),
        &ws.no_hooks(),
    )?;
    store.set_run_snapshot(id, &snapshot)?;
    if verify
        && ws.is_trusted()?
        && let Some(task) = intent.tasks.get(&run.task)
    {
        let changed = git.changed_paths(&run.base, &snapshot)?;
        if !changed.is_empty() {
            let cr = CheckRun {
                ws,
                store,
                dir: &wt,
                run: Some(id),
            };
            for req in required_checks(&intent, task, Some(&changed)) {
                cr.execute(&intent.config.checks[&req.name])?;
            }
        }
    }
    Ok(Some(snapshot))
}

/// Turns the agent's stream into a small number of meaningful rows.
///
/// Message text is buffered and written as one event per paragraph-ish
/// chunk, at most a few transactions per second per run. Tool calls are
/// recorded when they start and when their status changes, not on every
/// content update. Thought text is never stored.
struct Recorder {
    run: String,
    root: PathBuf,
    echo: bool,
    text: String,
    text_since: Option<Instant>,
    tools: HashMap<String, (Option<String>, Option<String>)>,
    pending: Vec<(String, Value)>,
    last_flush: Instant,
    thoughts: u64,
    usage: Option<Value>,
    other: HashMap<String, u64>,
}

impl Recorder {
    fn new(run: &str, root: &Path, echo: bool) -> Recorder {
        Recorder {
            run: run.to_string(),
            root: root.to_path_buf(),
            echo,
            text: String::new(),
            text_since: None,
            tools: HashMap::new(),
            pending: Vec::new(),
            last_flush: Instant::now(),
            thoughts: 0,
            usage: None,
            other: HashMap::new(),
        }
    }

    fn update(&mut self, u: Update) {
        match u {
            Update::MessageChunk(t) => {
                if self.echo {
                    eprint!("{t}");
                }
                self.text.push_str(&t);
                self.text_since.get_or_insert_with(Instant::now);
                if self.text.len() > 8 * 1024 {
                    self.take_text();
                }
            }
            Update::Thought => self.thoughts += 1,
            Update::ToolCall {
                id,
                title,
                kind,
                status,
                locations,
                is_new,
            } => {
                self.take_text();
                let entry = self.tools.entry(id.clone()).or_default();
                let changed = is_new
                    || (title.is_some() && title != entry.0)
                    || (status.is_some() && status != entry.1);
                if title.is_some() {
                    entry.0 = title.clone();
                }
                if status.is_some() {
                    entry.1 = status.clone();
                }
                if changed {
                    let locations: Vec<String> = locations
                        .iter()
                        .map(|l| {
                            Path::new(l)
                                .strip_prefix(&self.root)
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| l.clone())
                        })
                        .collect();
                    if self.echo {
                        eprintln!(
                            "\n[{}] {}",
                            status.as_deref().unwrap_or("·"),
                            entry.0.as_deref().unwrap_or("tool")
                        );
                    }
                    self.pending.push(("agent.tool".into(), json!({ "id": id, "title": entry.0, "kind": kind, "status": entry.1, "locations": locations })));
                }
            }
            Update::Plan(entries) => {
                self.take_text();
                let entries: Vec<Value> = entries
                    .into_iter()
                    .map(|(c, s)| json!({ "content": c, "status": s }))
                    .collect();
                self.pending
                    .push(("agent.plan".into(), json!({ "entries": entries })));
            }
            Update::Usage(u) => self.usage = Some(u),
            Update::Other(k) => *self.other.entry(k).or_default() += 1,
        }
    }

    fn take_text(&mut self) {
        if !self.text.trim().is_empty() {
            let text = std::mem::take(&mut self.text);
            self.pending
                .push(("agent.message".into(), json!({ "text": text })));
        }
        self.text.clear();
        self.text_since = None;
    }

    fn flush_if_due(&mut self, store: &Store) -> Result<()> {
        if self
            .text_since
            .is_some_and(|t| t.elapsed() > Duration::from_millis(1500))
        {
            self.take_text();
        }
        if !self.pending.is_empty() && self.last_flush.elapsed() > Duration::from_millis(250) {
            self.flush(store)?;
        }
        Ok(())
    }

    fn flush(&mut self, store: &Store) -> Result<()> {
        self.take_text();
        store.append_batch(&self.run, &self.pending)?;
        self.pending.clear();
        self.last_flush = Instant::now();
        Ok(())
    }

    fn finish(&mut self, store: &Store) -> Result<()> {
        self.flush(store)?;
        store.append(Some(&self.run), "agent.summary", &json!({ "thoughts": self.thoughts, "usage": self.usage, "unhandled_updates": self.other }))?;
        if self.echo {
            eprintln!();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perm(kind: &str, path: &str) -> Value {
        json!({
            "toolCall": { "kind": kind, "title": "x", "locations": [{ "path": path }] },
            "options": [{ "optionId": "yes", "kind": "allow_once", "name": "Allow" }, { "optionId": "no", "kind": "reject_once", "name": "Reject" }]
        })
    }

    #[test]
    fn policy_decisions() {
        let wt = Path::new("/repo/.git/kitsu/worktrees/r1");
        let inside = "/repo/.git/kitsu/worktrees/r1/src/a.rs";
        let escape = "/repo/.git/kitsu/worktrees/r1/../../../../src/a.rs";
        assert!(
            matches!(decide(Policy::Ask, &perm("edit", inside), wt), Decision::Answer(o, _) if o == "yes")
        );
        assert!(matches!(
            decide(Policy::Ask, &perm("execute", inside), wt),
            Decision::Human
        ));
        assert!(matches!(
            decide(Policy::Auto, &perm("execute", inside), wt),
            Decision::Answer(..)
        ));
        assert!(matches!(
            decide(Policy::Auto, &perm("edit", escape), wt),
            Decision::Human
        ));
        assert!(matches!(
            decide(Policy::Auto, &perm("edit", "/home/me/.ssh/config"), wt),
            Decision::Human
        ));
        let no_allow = json!({ "toolCall": { "kind": "read" }, "options": [{ "optionId": "no", "kind": "reject_once" }] });
        assert!(matches!(
            decide(Policy::Auto, &no_allow, wt),
            Decision::Human
        ));
    }
}
