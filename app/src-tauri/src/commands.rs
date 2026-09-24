//! Everything the window is allowed to ask for.
//!
//! The webview gets no generic powers: no "read any path", no "run any
//! command". Each command below does one thing, validates its arguments,
//! and goes through the same library the CLI uses. Paths from the UI are
//! repo-relative and checked against the repository root.

use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use kitsu::brief::{self, Context};
use kitsu::check::{CheckRun, status_at};
use kitsu::digest;
use kitsu::integrate::{self, AcceptOptions};
use kitsu::intent::{self, Intent, Kind, QuestionState};
use kitsu::recover;
use kitsu::run::RunEvent;
use kitsu::status::Snapshot;
use kitsu::util::{content_id, short_id};
use kitsu::workspace::{Instance, Workspace};
use kitsu::{Error, agents};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::State;

pub struct AppState {
    pub ws: Mutex<Option<Workspace>>,
    /// Held for the app's lifetime: accepts made from the UI are owned by
    /// this process.
    pub instance: Mutex<Option<Instance>>,
}

/// Errors cross the IPC boundary with their category intact, so the UI
/// can tell "you need to approve this" from "git failed".
#[derive(Debug, Serialize)]
pub struct UiError {
    kind: &'static str,
    message: String,
}

impl From<Error> for UiError {
    fn from(e: Error) -> Self {
        UiError {
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}

type R<T> = Result<T, UiError>;

fn invalid(msg: impl Into<String>) -> UiError {
    UiError {
        kind: "invalid",
        message: msg.into(),
    }
}

fn ws(state: &State<'_, AppState>) -> R<Workspace> {
    state
        .ws
        .lock()
        .map_err(|_| invalid("state lock poisoned"))?
        .clone()
        .ok_or_else(|| UiError {
            kind: "no_repo",
            message: "no repository open".into(),
        })
}

async fn blocking<T, F>(f: F) -> R<T>
where
    T: Send + 'static,
    F: FnOnce() -> R<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| invalid(format!("worker thread failed: {e}")))?
}

/// Resolve a repo-relative path from the UI to an absolute one inside the
/// main worktree, refusing anything that could step outside it or into
/// `.git`.
pub fn safe_path(root: &Path, rel: &str) -> R<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty()
        || p.is_absolute()
        || p.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid(format!("not a repository path: {rel}")));
    }
    if p.components()
        .next()
        .is_some_and(|c| c.as_os_str() == ".git")
    {
        return Err(invalid("paths inside .git are off limits"));
    }
    let full = root.join(p);
    // Symlinks can still point outside; resolve what exists and re-check.
    let real_root = std::fs::canonicalize(root).map_err(|e| invalid(e.to_string()))?;
    let mut probe = full.clone();
    while !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    let real = std::fs::canonicalize(&probe).map_err(|e| invalid(e.to_string()))?;
    if !real.starts_with(&real_root) {
        return Err(invalid(format!("{rel} resolves outside the repository")));
    }
    Ok(full)
}

#[derive(Serialize)]
pub struct Repo {
    root: String,
    name: String,
    branch: Option<String>,
    trusted: bool,
    initialized: bool,
}

fn repo_info(ws: &Workspace) -> R<Repo> {
    Ok(Repo {
        root: ws.root.display().to_string(),
        name: ws
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        branch: ws.git().current_branch()?,
        trusted: ws.is_trusted()?,
        initialized: ws.root.join(intent::DIR).exists(),
    })
}

#[tauri::command]
pub async fn open_repo(state: State<'_, AppState>, path: Option<String>) -> R<Repo> {
    let start = match path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().map_err(|e| invalid(e.to_string()))?,
    };
    let (w, inst) = blocking(move || {
        let w = Workspace::discover(&start)?;
        let store = w.open_store()?;
        let inst = Instance::acquire(&w)?;
        recover::recover(&w, &store)?;
        Ok::<_, UiError>((w, inst))
    })
    .await?;
    let info = repo_info(&w)?;
    *state.ws.lock().map_err(|_| invalid("lock"))? = Some(w);
    *state.instance.lock().map_err(|_| invalid("lock"))? = Some(inst);
    Ok(info)
}

#[tauri::command]
pub async fn init_repo(state: State<'_, AppState>) -> R<Repo> {
    let w = ws(&state)?;
    blocking(move || {
        for k in Kind::ALL {
            let d = w.root.join(intent::DIR).join(k.dir());
            std::fs::create_dir_all(&d).map_err(|e| Error::io(d.display().to_string(), e))?;
        }
        let cfg = w.root.join(intent::CONFIG);
        if !cfg.exists() {
            std::fs::write(
                &cfg,
                "# Checks: [checks.<name>] run = \"...\"\n\n[protect]\npaths = []\n",
            )
            .map_err(|e| Error::io(cfg.display().to_string(), e))?;
        }
        repo_info(&w)
    })
    .await
}

#[tauri::command]
pub async fn trust_repo(state: State<'_, AppState>) -> R<Repo> {
    let w = ws(&state)?;
    blocking(move || {
        w.trust()?;
        repo_info(&w)
    })
    .await
}

/// Everything the main list needs, in one round trip.
#[tauri::command]
pub async fn overview(state: State<'_, AppState>) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        let intent = Intent::load_dir(&w.root)?;
        let git = w.git();
        let tasks = Snapshot { intent: &intent, git: &git, store: &store }.tasks()?;
        let seen: i64 = store.meta("seen.app")?.and_then(|s| s.parse().ok()).unwrap_or(0);
        let since = digest::since(&store, seen)?;
        let asks = store.open_asks()?;
        let agents: Vec<Value> = agents::all()?.into_iter().map(|a| json!({ "name": a.name, "command": a.command.join(" "), "source": a.source })).collect();
        let personal = kitsu::memory::personal(&kitsu::workspace::config_dir());
        let problems: Vec<Value> = intent
            .problems
            .iter()
            .map(|p| json!({ "path": p.path, "detail": p.detail }))
            .chain(personal.problems.iter().map(|(p, d)| json!({ "path": p, "detail": d })))
            .collect();
        let counts = json!({
            "invariants": intent.invariants.len(),
            "decisions": intent.decisions.len(),
            "open_questions": intent.questions.values().filter(|q| q.state == QuestionState::Open).count(),
            "memory": intent.memory.len(),
            "checks": intent.config.checks.len(),
        });
        Ok(json!({ "repo": repo_info(&w)?, "tasks": tasks, "asks": asks, "since": since, "agents": agents, "problems": problems, "counts": counts }))
    })
    .await
}

#[tauri::command]
pub async fn mark_seen(state: State<'_, AppState>, seq: i64) -> R<()> {
    let w = ws(&state)?;
    blocking(move || {
        w.open_store()?.set_meta("seen.app", &seq.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn task_detail(state: State<'_, AppState>, id: String) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        let intent = Intent::load_dir(&w.root)?;
        let task = intent.tasks.get(&id).ok_or_else(|| Error::NotFound(format!("task {id}")))?;
        let git = w.git();
        let head = git.head()?;
        let b = brief::compile(&Context { intent: &intent, git: Some(&git), store: Some(&store), base: head.as_deref(), worktree: None, run: None, personal: &kitsu::memory::personal(&kitsu::workspace::config_dir()), budget: brief::DEFAULT_BUDGET }, task);
        let runs = store.runs_for_task(&id)?;
        let state = match task.state {
            intent::TaskState::Open => "open",
            intent::TaskState::Done => "done",
            intent::TaskState::Dropped => "dropped",
        };
        let questions: Vec<Value> = intent
            .questions
            .values()
            .filter(|q| q.blocks.contains(&id))
            .map(|q| json!({ "id": q.id, "title": q.title, "open": q.state == QuestionState::Open, "answer": q.answer, "body": q.body }))
            .collect();
        Ok(json!({
            "task": { "id": task.id, "title": task.title, "state": state, "scope": task.scope.globs(), "checks": task.checks, "after": task.after, "body": task.body, "path": task.source.path },
            "brief": b,
            "runs": runs,
            "questions": questions,
            "dirty_checkout": !git.is_clean()?,
        }))
    })
    .await
}

#[tauri::command]
pub async fn run_detail(state: State<'_, AppState>, id: String, after: i64) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        let run = store.run(&id)?;
        let events = store.run_events(&id, after, 2_000)?;
        let evidence = store.evidence_for_run(&id)?;
        Ok(json!({ "run": run, "events": events, "evidence": evidence }))
    })
    .await
}

#[tauri::command]
pub async fn evidence_log(state: State<'_, AppState>, id: i64) -> R<String> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        let e = store.evidence(id)?;
        let blob = e.log.ok_or_else(|| Error::NotFound("log".into()))?;
        let bytes = w.blobs().get(&blob)?;
        // The UI shows logs as text; cap what crosses the IPC boundary.
        const MAX: usize = 512 * 1024;
        let start = bytes.len().saturating_sub(MAX);
        let mut text = String::from_utf8_lossy(&bytes[start..]).into_owned();
        if start > 0 {
            text = format!(
                "[showing the last {} KiB of {} KiB]\n{text}",
                MAX / 1024,
                bytes.len() / 1024
            );
        }
        Ok(text)
    })
    .await
}

#[tauri::command]
pub async fn review(state: State<'_, AppState>, run: String) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        serde_json::to_value(integrate::review(&w, &store, &run)?)
            .map_err(|e| invalid(e.to_string()))
    })
    .await
}

#[tauri::command]
pub async fn file_diff(state: State<'_, AppState>, run: String, path: String) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        let r = integrate::review(&w, &store, &run)?;
        let snap = store.run(&run)?.snapshot.ok_or_else(|| Error::NotFound("snapshot".into()))?;
        let git = w.git();
        let text = |rev: &str| -> R<Option<String>> { Ok(git.show_file(rev, &path)?.map(|b| String::from_utf8_lossy(&b).into_owned())) };
        Ok(json!({ "path": path, "old": text(&r.from)?, "new": text(&snap)?, "protected": r.protected.contains(&path) }))
    })
    .await
}

/// Start an agent. Launches `kitsu run` as its own process group, so runs
/// survive the window closing and behave exactly like terminal runs.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn start_run(
    state: State<'_, AppState>,
    task: String,
    agent: String,
    policy: String,
    note: Option<String>,
    from: Option<String>,
) -> R<String> {
    let w = ws(&state)?;
    if !matches!(policy.as_str(), "ask" | "auto") {
        return Err(invalid("policy must be ask or auto"));
    }
    // These end up on a command line. An id like `--from=x` must never be
    // parsed as a flag, whatever the window sends.
    intent::valid_id(&task).map_err(invalid)?;
    if let Some(f) = &from {
        intent::valid_id(f).map_err(invalid)?;
    }
    agents::resolve(&agent)?;
    let id = short_id('r');
    let exe = std::env::current_exe().map_err(|e| invalid(e.to_string()))?;
    let log_dir = w.state.join("runs");
    std::fs::create_dir_all(&log_dir).map_err(|e| invalid(e.to_string()))?;
    let log = std::fs::File::create(log_dir.join(format!("{id}.worker.log")))
        .map_err(|e| invalid(e.to_string()))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--kitsu-cli", "-C"]).arg(&w.root).args([
        "run", &task, "--agent", &agent, "--policy", &policy, "--id", &id, "-q",
    ]);
    if let Some(n) = note.as_deref().filter(|n| !n.trim().is_empty()) {
        // `--note=<text>` so a note starting with `-` stays a value.
        cmd.arg(format!("--note={n}"));
    }
    if let Some(f) = &from {
        cmd.args(["--from", f]);
    }
    // After `--`, clap treats the task id as a positional no matter what.
    cmd.args(["--", &task]);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd
        .spawn()
        .map_err(|e| invalid(format!("could not start the run worker: {e}")))?;
    // Reap it when it exits so it doesn't linger as a zombie while the app
    // is open. The run's state lives in the database, not in this handle.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(id)
}

#[tauri::command]
pub async fn stop_run(state: State<'_, AppState>, id: String) -> R<()> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        let run = store.run(&id)?;
        if run.state.is_terminal() {
            return Ok(());
        }
        if let Some(owner) = &run.owner
            && Instance::liveness(&w, owner)? == kitsu::workspace::Liveness::Dead
        {
            recover::recover(&w, &store)?;
            return Ok(());
        }
        store.apply_run_event(&id, &RunEvent::CancelRequested)?;
        if let Some(owner) = &run.owner {
            Instance::nudge(&w, owner);
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn answer_ask(state: State<'_, AppState>, id: i64, option: String) -> R<bool> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        let ask = store.ask(id)?;
        let valid = ask.request["options"]
            .as_array()
            .is_some_and(|o| o.iter().any(|x| x["optionId"] == option.as_str()));
        if !valid {
            return Err(invalid("not one of the offered options"));
        }
        let first = store.answer_ask(id, &option)?;
        if let Some(owner) = store.run(&ask.run)?.owner {
            Instance::nudge(&w, &owner);
        }
        Ok(first)
    })
    .await
}

#[tauri::command]
pub async fn answer_question(state: State<'_, AppState>, id: String, answer: String) -> R<()> {
    let w = ws(&state)?;
    blocking(move || {
        let intent = Intent::load_dir(&w.root)?;
        let q = intent
            .questions
            .get(&id)
            .ok_or_else(|| Error::NotFound(format!("question {id}")))?;
        let path = w.root.join(&q.source.path);
        let text =
            std::fs::read_to_string(&path).map_err(|e| Error::io(path.display().to_string(), e))?;
        let edited = kitsu::cli::answer_question(&text, &answer).map_err(|d| Error::Parse {
            path: path.clone(),
            detail: d,
        })?;
        std::fs::write(&path, edited).map_err(|e| Error::io(path.display().to_string(), e))?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn accept_run(
    state: State<'_, AppState>,
    run: String,
    close_task: bool,
    approval: Option<String>,
) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        // Custody for this accept. If the app dies mid-way, recovery sees a
        // dead owner and settles the integration by asking git.
        let owner = Instance::acquire(&w)?;
        let me = &owner;
        let res = integrate::accept(
            &w,
            &store,
            me,
            &run,
            &AcceptOptions {
                close_task,
                approval,
            },
        )?;
        serde_json::to_value(res).map_err(|e| invalid(e.to_string()))
    })
    .await
}

#[tauri::command]
pub async fn discard_run(state: State<'_, AppState>, run: String) -> R<Vec<String>> {
    let w = ws(&state)?;
    blocking(move || Ok(integrate::discard(&w, &w.open_store()?, &run)?)).await
}

// Tauri maps IPC arguments to parameters one to one; grouping them into a
// struct would only move the same list into the JS call.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn new_entity(
    state: State<'_, AppState>,
    kind: String,
    title: String,
    scope: Vec<String>,
    checks: Vec<String>,
    after: Vec<String>,
    blocks: Vec<String>,
    body: Option<String>,
) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        let kind = Kind::parse(&kind).ok_or_else(|| invalid("unknown kind"))?;
        if title.trim().is_empty() {
            return Err(invalid("a title is required"));
        }
        let mut text = kitsu::cli::render_new(kind, title.trim(), &scope, &checks, &after, &blocks);
        if let Some(b) = body.filter(|b| !b.trim().is_empty()) {
            let (_, default_body) = intent::split_front_matter(&text).map_err(invalid)?;
            let cut = text.len() - default_body.len();
            text = format!("{}{}\n", &text[..cut], b.trim_end());
        }
        let path = kitsu::cli::write_new(&w, kind, &title, None, &text)?;
        let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(json!({ "id": id, "path": path.strip_prefix(&w.root).unwrap_or(&path).display().to_string() }))
    })
    .await
}

#[tauri::command]
pub async fn rules(state: State<'_, AppState>) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        let store = w.open_store()?;
        let intent = Intent::load_dir(&w.root)?;
        let git = w.git();
        let tree = git.worktree_tree(&w.scratch())?;
        let check_status = |names: &[String]| -> R<Vec<Value>> {
            let mut out = Vec::new();
            for n in names {
                match intent.config.checks.get(n) {
                    Some(def) => out.push(json!({ "name": n, "status": status_at(&git, &store, def, &tree)? })),
                    None => out.push(json!({ "name": n, "status": { "status": "missing" } })),
                }
            }
            Ok(out)
        };
        let mut invariants = Vec::new();
        for i in intent.invariants.values() {
            invariants.push(json!({
                "id": i.id, "title": i.title, "active": i.state == intent::InvariantState::Active, "scope": i.scope.globs(),
                "checks": check_status(&i.checks)?, "decision": i.decision, "body": i.body, "path": i.source.path
            }));
        }
        let decisions: Vec<Value> = intent
            .decisions
            .values()
            .map(|d| {
                let st = match d.state {
                    intent::DecisionState::Proposed => "proposed",
                    intent::DecisionState::Accepted => "accepted",
                    intent::DecisionState::Superseded => "superseded",
                };
                json!({ "id": d.id, "title": d.title, "state": st, "scope": d.scope.globs(), "rejected": d.rejected, "body": d.body, "path": d.source.path })
            })
            .collect();
        let questions: Vec<Value> = intent
            .questions
            .values()
            .map(|q| json!({ "id": q.id, "title": q.title, "open": q.state == QuestionState::Open, "blocks": q.blocks, "answer": q.answer, "body": q.body, "path": q.source.path }))
            .collect();
        let checks: Vec<Value> = intent
            .config
            .checks
            .values()
            .map(|c| -> R<Value> { Ok(json!({ "name": c.name, "run": c.run, "scope": c.scope.globs(), "status": status_at(&git, &store, c, &tree)? })) })
            .collect::<R<_>>()?;
        // Staleness against HEAD: uncommitted edits to anchors don't count
        // until committed, same as for runs.
        let fresh = match git.head()? {
            Some(h) => kitsu::memory::freshness(&git, &h, intent.memory.values())?,
            None => Default::default(),
        };
        let personal = kitsu::memory::personal(&kitsu::workspace::config_dir());
        let memory: Vec<Value> = intent
            .memory
            .values()
            .map(|m| (m, fresh.get(&m.id).cloned()))
            .chain(personal.notes.iter().map(|m| (m, Some(kitsu::memory::Freshness::Unanchored))))
            .map(|(m, f)| {
                json!({ "id": m.id, "title": m.title, "kind": m.kind, "scope": m.scope.globs(), "anchors": m.anchors.globs(),
                        "by": m.by, "run": m.run, "body": m.body, "path": m.source.path, "freshness": f,
                        "personal": m.id.starts_with("personal/") })
            })
            .collect();
        Ok(json!({ "invariants": invariants, "decisions": decisions, "questions": questions, "memory": memory, "checks": checks }))
    })
    .await
}

#[tauri::command]
pub async fn run_checks(state: State<'_, AppState>, names: Vec<String>) -> R<Value> {
    let w = ws(&state)?;
    blocking(move || {
        if !w.is_trusted()? {
            return Err(UiError {
                kind: "denied",
                message: "trust this repository first; checks run its code".into(),
            });
        }
        let store = w.open_store()?;
        let intent = Intent::load_dir(&w.root)?;
        let names: Vec<String> = if names.is_empty() {
            intent.config.checks.keys().cloned().collect()
        } else {
            names
        };
        let cr = CheckRun {
            ws: &w,
            store: &store,
            dir: &w.root,
            run: None,
        };
        let mut out = Vec::new();
        for n in names {
            let def = intent
                .config
                .checks
                .get(&n)
                .ok_or_else(|| Error::NotFound(format!("check {n}")))?;
            out.push(cr.execute(def)?);
        }
        serde_json::to_value(out).map_err(|e| invalid(e.to_string()))
    })
    .await
}

#[derive(Serialize)]
pub struct FileText {
    path: String,
    text: String,
    /// Content id of what's on disk now; pass it back to `write_file`.
    version: String,
}

#[tauri::command]
pub async fn read_file(state: State<'_, AppState>, path: String) -> R<FileText> {
    let w = ws(&state)?;
    blocking(move || {
        let full = safe_path(&w.root, &path)?;
        let meta = std::fs::metadata(&full).map_err(|e| Error::io(path.clone(), e))?;
        if meta.len() > 16 * 1024 * 1024 {
            return Err(invalid(format!(
                "{path} is {} MiB; too big for the editor",
                meta.len() / (1024 * 1024)
            )));
        }
        let bytes = std::fs::read(&full).map_err(|e| Error::io(path.clone(), e))?;
        let version = content_id(&bytes);
        let text =
            String::from_utf8(bytes).map_err(|_| invalid(format!("{path} is not UTF-8 text")))?;
        Ok(FileText {
            path,
            text,
            version,
        })
    })
    .await
}

/// Save only if the file is still what the editor loaded. Someone (you in
/// another editor, `git checkout`, an accept) may have changed it; then
/// the save is refused instead of silently clobbering their version.
#[tauri::command]
pub async fn write_file(
    state: State<'_, AppState>,
    path: String,
    text: String,
    version: Option<String>,
) -> R<String> {
    let w = ws(&state)?;
    blocking(move || {
        let full = safe_path(&w.root, &path)?;
        let current = match std::fs::read(&full) {
            Ok(b) => Some(content_id(&b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::io(path.clone(), e).into()),
        };
        if current != version {
            return Err(UiError {
                kind: "conflict",
                message: format!("{path} changed on disk since you opened it"),
            });
        }
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::io(parent.display().to_string(), e))?;
        }
        let tmp = full.with_extension(format!("kitsu-save-{}", short_id('s')));
        std::fs::write(&tmp, &text).map_err(|e| Error::io(tmp.display().to_string(), e))?;
        std::fs::rename(&tmp, &full).map_err(|e| Error::io(path.clone(), e))?;
        Ok(content_id(text.as_bytes()))
    })
    .await
}

#[tauri::command]
pub async fn list_files(state: State<'_, AppState>) -> R<Vec<String>> {
    let w = ws(&state)?;
    blocking(move || {
        let out = w.git().run([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])?;
        Ok(out
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_refuses_escapes() {
        let root = std::env::temp_dir().join(format!("kitsu-safe-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        assert!(safe_path(&root, "src/a.rs").is_ok());
        assert!(safe_path(&root, "new/dir/file.txt").is_ok());
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "src/../../x",
            ".git/config",
            "./src/a.rs",
        ] {
            assert!(safe_path(&root, bad).is_err(), "{bad} should be refused");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", root.join("etc-link")).expect("symlink");
            assert!(
                safe_path(&root, "etc-link/passwd").is_err(),
                "symlink escape"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
