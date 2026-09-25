//! Everything the window is allowed to ask for.
//!
//! The webview gets no generic powers: no "read any path", no "run any
//! command". Each command below does one thing, validates its arguments,
//! and goes through the same library the CLI uses. Paths from the UI are
//! repo-relative and checked against the repository root.
//!
//! The window shows several repositories, and every command that touches
//! one names it: `repo` is the id from the workspaces list. There is no
//! "current repository" on this side, so a command sent just before a
//! switch still lands where it was meant to. An id that isn't in the list
//! is refused; the only way to add one is `add_workspace`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

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
use kitsu::workspaces::{self, Entry, List};
use kitsu::{Error, agents};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::State;

/// A repository the window has opened: recovered once, and owned by this
/// process's instance lock until the app exits or it leaves the list.
struct Open {
    ws: Workspace,
    _instance: Instance,
}

struct Inner {
    list: List,
    repos: Mutex<BTreeMap<String, Open>>,
    cache: workspaces::Cache,
    /// Where the app was started; offered as the first project.
    launch_dir: Option<PathBuf>,
}

/// Cheap to clone: commands move a copy into their worker thread.
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl AppState {
    pub fn new(list: List, launch_dir: Option<PathBuf>) -> AppState {
        AppState(Arc::new(Inner {
            list,
            repos: Mutex::new(BTreeMap::new()),
            cache: workspaces::Cache::default(),
            launch_dir,
        }))
    }

    /// The repository `id` names, opened on first use: its state database,
    /// crash recovery, an instance lock. Blocking; call it off the async
    /// runtime. Only ids in the workspaces list resolve.
    pub fn ws(&self, id: &str) -> R<Workspace> {
        if let Some(o) = self.lock()?.get(id) {
            return Ok(o.ws.clone());
        }
        let entry = self.entry(id)?;
        let w = open_entry(&entry)?;
        let store = w.open_store()?;
        let instance = Instance::acquire(&w)?;
        recover::recover(&w, &store)?;
        let mut repos = self.lock()?;
        // Another command may have opened it meanwhile; keep the first.
        let o = repos.entry(id.to_string()).or_insert(Open {
            ws: w,
            _instance: instance,
        });
        Ok(o.ws.clone())
    }

    fn entry(&self, id: &str) -> R<Entry> {
        self.0.list.find(id)?.ok_or_else(|| UiError {
            kind: "no_repo",
            message: format!("project {id} is not in the list"),
        })
    }

    fn lock(&self) -> R<std::sync::MutexGuard<'_, BTreeMap<String, Open>>> {
        self.0
            .repos
            .lock()
            .map_err(|_| invalid("state lock poisoned"))
    }

    /// Repositories opened so far, for the change watcher.
    pub fn opened(&self) -> Vec<(String, Workspace)> {
        self.lock()
            .map(|r| r.iter().map(|(id, o)| (id.clone(), o.ws.clone())).collect())
            .unwrap_or_default()
    }

    fn close(&self, id: &str) {
        if let Ok(mut r) = self.lock() {
            r.remove(id);
        }
    }
}

/// `Workspace::open` on the listed root, keeping the root exactly as listed
/// so ids derived from it stay stable.
fn open_entry(e: &Entry) -> R<Workspace> {
    if !e.root.is_dir() {
        return Err(Error::NotFound(format!("{} is gone", e.root.display())).into());
    }
    let mut w = Workspace::open(&e.root)?;
    w.root = e.root.clone();
    Ok(w)
}

/// Run `f` on a worker thread with the repository `repo` names.
async fn in_repo<T, F>(state: &State<'_, AppState>, repo: String, f: F) -> R<T>
where
    T: Send + 'static,
    F: FnOnce(Workspace) -> R<T> + Send + 'static,
{
    let st = state.inner().clone();
    blocking(move || f(st.ws(&repo)?)).await
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
    id: String,
    root: String,
    name: String,
    branch: Option<String>,
    trusted: bool,
    initialized: bool,
}

fn repo_info(ws: &Workspace) -> R<Repo> {
    Ok(Repo {
        id: workspaces::id_of(&ws.root),
        root: ws.root.display().to_string(),
        name: ws
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        branch: ws.current_branch()?,
        trusted: ws.is_trusted()?,
        initialized: ws.root.join(intent::DIR).exists(),
    })
}

// Workspace-level commands: the list itself. These are the only ones
// without a `repo` argument (see `every_repo_command_names_its_repo`).

#[derive(Serialize)]
pub struct Listed {
    id: String,
    name: String,
    root: String,
}

fn listed(e: &Entry) -> Listed {
    Listed {
        id: e.id(),
        name: e.display_name(),
        root: e.root.display().to_string(),
    }
}

/// The project the app was started in (a git repository around the
/// current directory), added to the list if it isn't there yet. `None`
/// when started elsewhere.
#[tauri::command]
pub async fn launch_repo(state: State<'_, AppState>) -> R<Option<Listed>> {
    let st = state.inner().clone();
    blocking(move || launch(&st)).await
}

fn launch(st: &AppState) -> R<Option<Listed>> {
    let Some(dir) = &st.0.launch_dir else {
        return Ok(None);
    };
    let Ok(w) = Workspace::discover(dir) else {
        return Ok(None);
    };
    let (e, _) = st.0.list.add(&w.root, None, true)?;
    Ok(Some(listed(&e)))
}

#[tauri::command]
pub async fn list_workspaces(state: State<'_, AppState>) -> R<Vec<Listed>> {
    let st = state.inner().clone();
    blocking(move || Ok(st.0.list.load()?.iter().map(listed).collect())).await
}

/// Add the git repository at `path`. Refused with a reason: not a folder,
/// not a repository, inside one, already listed. Doesn't trust it.
#[tauri::command]
pub async fn add_workspace(
    state: State<'_, AppState>,
    path: String,
    name: Option<String>,
) -> R<Listed> {
    let st = state.inner().clone();
    blocking(move || add(&st, &path, name.as_deref())).await
}

fn add(st: &AppState, path: &str, name: Option<&str>) -> R<Listed> {
    let path = path.trim();
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    };
    if !expanded.is_absolute() {
        return Err(invalid(format!(
            "{path} is not a full path; start it with / or ~/"
        )));
    }
    let (e, _) = st.0.list.add(&expanded, name, false)?;
    Ok(listed(&e))
}

/// Take a project off the list. Never deletes anything: its files, rules,
/// runs and trust stay as they are, and runs in progress keep going.
#[tauri::command]
pub async fn remove_workspace(state: State<'_, AppState>, repo: String) -> R<Listed> {
    let st = state.inner().clone();
    blocking(move || {
        let e = st.0.list.remove(&repo)?;
        st.close(&repo);
        Ok(listed(&e))
    })
    .await
}

#[tauri::command]
pub async fn rename_workspace(
    state: State<'_, AppState>,
    repo: String,
    name: Option<String>,
) -> R<Listed> {
    let st = state.inner().clone();
    blocking(move || Ok(listed(&st.0.list.rename(&repo, name.as_deref())?))).await
}

#[tauri::command]
pub async fn move_workspace(state: State<'_, AppState>, repo: String, index: usize) -> R<()> {
    let st = state.inner().clone();
    blocking(move || Ok(st.0.list.reorder(&repo, index)?)).await
}

/// Per project: name, branch, what needs you, trusted or not. The switcher
/// polls this; unchanged repositories are answered from a cache keyed by
/// a `stat`-only fingerprint.
#[tauri::command]
pub async fn workspace_overview(state: State<'_, AppState>) -> R<Vec<workspaces::Summary>> {
    let st = state.inner().clone();
    blocking(move || Ok(st.0.cache.overview(&st.0.list.load()?))).await
}

/// Branches and worktrees of one repository, marked yours or Kitsu's.
#[tauri::command]
pub async fn git_view(state: State<'_, AppState>, repo: String) -> R<workspaces::GitView> {
    in_repo(&state, repo, move |w| Ok(workspaces::git_view(&w)?)).await
}

#[tauri::command]
pub async fn open_repo(state: State<'_, AppState>, repo: String) -> R<Repo> {
    in_repo(&state, repo, move |w| repo_info(&w)).await
}

#[tauri::command]
pub async fn init_repo(state: State<'_, AppState>, repo: String) -> R<Repo> {
    in_repo(&state, repo, move |w| {
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
pub async fn trust_repo(state: State<'_, AppState>, repo: String) -> R<Repo> {
    in_repo(&state, repo, move |w| {
        w.trust()?;
        repo_info(&w)
    })
    .await
}

/// Everything the main list needs, in one round trip.
#[tauri::command]
pub async fn overview(state: State<'_, AppState>, repo: String) -> R<Value> {
    in_repo(&state, repo, move |w| overview_of(&w)).await
}

fn overview_of(w: &Workspace) -> R<Value> {
    let store = w.open_store()?;
    let intent = Intent::load_dir(&w.root)?;
    let git = w.git();
    let tasks = Snapshot {
        intent: &intent,
        git: &git,
        store: &store,
    }
    .tasks()?;
    let seen: i64 = store
        .meta("seen.app")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let since = digest::since(&store, seen)?;
    let asks = store.open_asks()?;
    let agents: Vec<Value> = agents::all()?
        .into_iter()
        .map(|a| json!({ "name": a.name, "command": a.command.join(" "), "source": a.source }))
        .collect();
    let personal = kitsu::memory::personal(&kitsu::workspace::config_dir());
    let problems: Vec<Value> = intent
        .problems
        .iter()
        .map(|p| json!({ "path": p.path, "detail": p.detail }))
        .chain(
            personal
                .problems
                .iter()
                .map(|(p, d)| json!({ "path": p, "detail": d })),
        )
        .collect();
    let counts = json!({
        "decisions": intent.decisions.len(),
        "open_questions": intent.questions.values().filter(|q| q.state == QuestionState::Open).count(),
        "memory": intent.memory.len(),
        "checks": intent.config.checks.len(),
    });
    Ok(
        json!({ "repo": repo_info(w)?, "tasks": tasks, "asks": asks, "since": since, "agents": agents, "problems": problems, "counts": counts }),
    )
}

#[tauri::command]
pub async fn mark_seen(state: State<'_, AppState>, repo: String, seq: i64) -> R<()> {
    in_repo(&state, repo, move |w| {
        w.open_store()?.set_meta("seen.app", &seq.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn task_detail(state: State<'_, AppState>, repo: String, id: String) -> R<Value> {
    in_repo(&state, repo, move |w| {
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
        // What done means: the task's checks plus every check guarding its
        // scope, same list the brief gives the agent.
        let required: Vec<String> = kitsu::status::required_checks(&intent, task, None).into_iter().map(|r| r.name).collect();
        Ok(json!({
            "required": required,
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
pub async fn run_detail(
    state: State<'_, AppState>,
    repo: String,
    id: String,
    after: i64,
) -> R<Value> {
    in_repo(&state, repo, move |w| {
        let store = w.open_store()?;
        let run = store.run(&id)?;
        let events = store.run_events(&id, after, 2_000)?;
        let evidence = store.evidence_for_run(&id)?;
        Ok(json!({ "run": run, "events": events, "evidence": evidence }))
    })
    .await
}

#[tauri::command]
pub async fn evidence_log(state: State<'_, AppState>, repo: String, id: i64) -> R<String> {
    in_repo(&state, repo, move |w| {
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
pub async fn review(state: State<'_, AppState>, repo: String, run: String) -> R<Value> {
    in_repo(&state, repo, move |w| {
        let store = w.open_store()?;
        serde_json::to_value(integrate::review(&w, &store, &run)?)
            .map_err(|e| invalid(e.to_string()))
    })
    .await
}

#[tauri::command]
pub async fn file_diff(
    state: State<'_, AppState>,
    repo: String,
    run: String,
    path: String,
) -> R<Value> {
    in_repo(&state, repo, move |w| {
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
    repo: String,
    task: String,
    agent: String,
    policy: String,
    note: Option<String>,
    from: Option<String>,
) -> R<String> {
    let st = state.inner().clone();
    let w = blocking(move || st.ws(&repo)).await?;
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
pub async fn stop_run(state: State<'_, AppState>, repo: String, id: String) -> R<()> {
    in_repo(&state, repo, move |w| {
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
pub async fn answer_ask(
    state: State<'_, AppState>,
    repo: String,
    id: i64,
    option: String,
) -> R<bool> {
    in_repo(&state, repo, move |w| {
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
pub async fn answer_question(
    state: State<'_, AppState>,
    repo: String,
    id: String,
    answer: String,
) -> R<()> {
    in_repo(&state, repo, move |w| {
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
    repo: String,
    run: String,
    close_task: bool,
    approval: Option<String>,
) -> R<Value> {
    in_repo(&state, repo, move |w| {
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
pub async fn discard_run(state: State<'_, AppState>, repo: String, run: String) -> R<Vec<String>> {
    in_repo(&state, repo, move |w| {
        Ok(integrate::discard(&w, &w.open_store()?, &run)?)
    })
    .await
}

// Tauri maps IPC arguments to parameters one to one; grouping them into a
// struct would only move the same list into the JS call.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn new_entity(
    state: State<'_, AppState>,
    repo: String,
    kind: String,
    title: String,
    scope: Vec<String>,
    checks: Vec<String>,
    after: Vec<String>,
    blocks: Vec<String>,
    body: Option<String>,
) -> R<Value> {
    in_repo(&state, repo, move |w| {
        new_entity_in(&w, &kind, &title, &scope, &checks, &after, &blocks, body)
    })
    .await
}

#[allow(clippy::too_many_arguments)]
fn new_entity_in(
    w: &Workspace,
    kind: &str,
    title: &str,
    scope: &[String],
    checks: &[String],
    after: &[String],
    blocks: &[String],
    body: Option<String>,
) -> R<Value> {
    let kind = Kind::parse(kind).ok_or_else(|| invalid("unknown kind"))?;
    if title.trim().is_empty() {
        return Err(invalid("a title is required"));
    }
    let mut text = kitsu::cli::render_new(kind, title.trim(), scope, checks, after, blocks);
    if let Some(b) = body.filter(|b| !b.trim().is_empty()) {
        let (_, default_body) = intent::split_front_matter(&text).map_err(invalid)?;
        let cut = text.len() - default_body.len();
        text = format!("{}{}\n", &text[..cut], b.trim_end());
    }
    let path = kitsu::cli::write_new(w, kind, title, None, &text)?;
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(
        json!({ "id": id, "path": path.strip_prefix(&w.root).unwrap_or(&path).display().to_string() }),
    )
}

/// The task graph for the Plan view: every task with the lists the view
/// edits, the version of its file, and the checks it can pick from. Status
/// comes from `overview`, same as the rail.
#[tauri::command]
pub async fn plan(state: State<'_, AppState>, repo: String) -> R<Value> {
    in_repo(&state, repo, move |w| {
        let intent = Intent::load_dir(&w.root)?;
        let tasks: Vec<Value> = intent
            .tasks
            .values()
            .map(|t| {
                let st = match t.state {
                    intent::TaskState::Open => "open",
                    intent::TaskState::Done => "done",
                    intent::TaskState::Dropped => "dropped",
                };
                json!({ "id": t.id, "title": t.title, "state": st, "scope": t.scope.globs(), "checks": t.checks, "after": t.after, "path": t.source.path, "version": t.source.content_id })
            })
            .collect();
        let checks: Vec<&String> = intent.config.checks.keys().collect();
        Ok(json!({ "tasks": tasks, "checks": checks }))
    })
    .await
}

/// Change a task's `after`, `checks` and/or `scope` (a `null` list is left
/// alone). Only those keys of its front matter are rewritten; an edit that
/// would leave `.kitsu/` with a new problem, a cycle included, is refused
/// before anything is written. Returns the file's new version.
#[tauri::command]
pub async fn update_task(
    state: State<'_, AppState>,
    repo: String,
    id: String,
    after: Option<Vec<String>>,
    checks: Option<Vec<String>>,
    scope: Option<Vec<String>>,
    version: Option<String>,
) -> R<String> {
    in_repo(&state, repo, move |w| {
        let lists = kitsu::cli::TaskLists {
            after,
            checks,
            scope,
        };
        Ok(kitsu::cli::update_task(
            &w.root,
            &id,
            &lists,
            version.as_deref(),
        )?)
    })
    .await
}

#[tauri::command]
pub async fn rules(state: State<'_, AppState>, repo: String) -> R<Value> {
    in_repo(&state, repo, move |w| {
        let store = w.open_store()?;
        let intent = Intent::load_dir(&w.root)?;
        let git = w.git();
        let tree = git.worktree_tree(&w.scratch())?;
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
            .map(|c| -> R<Value> { Ok(json!({ "name": c.name, "run": c.run, "scope": c.scope.globs(), "guards": c.guards.globs(), "why": c.why, "status": status_at(&git, &store, c, &tree)? })) })
            .collect::<R<_>>()?;
        // Staleness against HEAD: uncommitted edits to anchors don't count
        // until committed, same as for runs.
        let fresh = match git.head()? {
            Some(h) => kitsu::memory::freshness(&git, &h, intent.memory.values())?,
            None => Default::default(),
        };
        let personal = kitsu::memory::personal(&kitsu::workspace::config_dir());
        let superseded = intent.superseded();
        let memory: Vec<Value> = intent
            .memory
            .values()
            .map(|m| (m, fresh.get(&m.id).cloned()))
            .chain(personal.notes.iter().map(|m| (m, Some(kitsu::memory::Freshness::Unanchored))))
            .map(|(m, f)| {
                json!({ "id": m.id, "title": m.title, "kind": m.kind, "scope": m.scope.globs(), "anchors": m.anchors.globs(),
                        "by": m.by, "run": m.run, "body": m.body, "path": m.source.path, "freshness": f,
                        "personal": m.id.starts_with("personal/"), "state": m.state,
                        "superseded_by": superseded.get(&m.id), "reason": m.reason, "key": m.key })
            })
            .collect();
        Ok(json!({ "decisions": decisions, "questions": questions, "memory": memory, "checks": checks }))
    })
    .await
}

#[tauri::command]
pub async fn run_checks(state: State<'_, AppState>, repo: String, names: Vec<String>) -> R<Value> {
    in_repo(&state, repo, move |w| {
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
pub async fn read_file(state: State<'_, AppState>, repo: String, path: String) -> R<FileText> {
    in_repo(&state, repo, move |w| {
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
    repo: String,
    path: String,
    text: String,
    version: Option<String>,
) -> R<String> {
    in_repo(&state, repo, move |w| {
        write_file_in(&w, &path, &text, version)
    })
    .await
}

fn write_file_in(w: &Workspace, path: &str, text: &str, version: Option<String>) -> R<String> {
    let full = safe_path(&w.root, path)?;
    let current = match std::fs::read(&full) {
        Ok(b) => Some(content_id(&b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::io(path, e).into()),
    };
    if current != version {
        return Err(UiError {
            kind: "conflict",
            message: format!("{path} changed on disk since you opened it"),
        });
    }
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent.display().to_string(), e))?;
    }
    let tmp = full.with_extension(format!("kitsu-save-{}", short_id('s')));
    std::fs::write(&tmp, text).map_err(|e| Error::io(tmp.display().to_string(), e))?;
    std::fs::rename(&tmp, &full).map_err(|e| Error::io(path, e))?;
    Ok(content_id(text.as_bytes()))
}

#[tauri::command]
pub async fn list_files(state: State<'_, AppState>, repo: String) -> R<Vec<String>> {
    in_repo(&state, repo, move |w| {
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

    /// A scratch directory, removed on drop.
    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Temp {
            let p = std::env::temp_dir().join(format!("kitsu-app-{tag}-{}", short_id('t')));
            std::fs::create_dir_all(&p).expect("mkdir");
            Temp(p)
        }

        /// A git repository with `.kitsu/` and one commit.
        fn repo(tag: &str) -> Temp {
            let t = Temp::new(tag);
            std::fs::create_dir_all(t.0.join(".kitsu/tasks")).expect("mkdir");
            std::fs::write(t.0.join(".kitsu/kitsu.toml"), "").expect("write");
            std::fs::write(
                t.0.join(".kitsu/tasks/mine.md"),
                format!("+++\ntitle = \"{tag}'s own\"\n+++\n"),
            )
            .expect("write");
            let git = kitsu::git::Git::new(&t.0);
            for args in [
                &["init", "--quiet", "-b", "main"][..],
                &["add", "-A"],
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "commit",
                    "--quiet",
                    "-m",
                    "init",
                ],
            ] {
                git.run(args).expect("git");
            }
            t
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn task_ids(ov: &Value) -> Vec<String> {
        ov["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .map(|t| t["id"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// The only commands without a `repo` argument are the ones about the
    /// list itself. A new command that touches a repository and forgets
    /// the argument would have to read some shared "current" one, which is
    /// exactly what lets a command land in the wrong repository.
    #[test]
    fn every_repo_command_names_its_repo() {
        const LIST_LEVEL: &[&str] = &[
            "launch_repo",
            "list_workspaces",
            "add_workspace",
            "workspace_overview",
        ];
        let src = include_str!("commands.rs");
        let main = include_str!("main.rs");
        let mut names = Vec::new();
        for chunk in src.split(concat!("#[tauri::", "command]")).skip(1) {
            let at = chunk
                .find("pub async fn ")
                .expect("a command is a pub async fn");
            let sig = &chunk[at + "pub async fn ".len()..];
            let name = sig.split('(').next().expect("name").to_string();
            let params = &sig[..sig.find(") -> ").expect("signature")];
            if LIST_LEVEL.contains(&name.as_str()) {
                assert!(!params.contains("repo:"), "{name} is list-level");
            } else {
                assert!(
                    params.contains("repo: String"),
                    "{name} touches a repository, so it must take `repo`"
                );
            }
            assert!(
                main.contains(&format!("commands::{name},")),
                "{name} is not registered in main.rs"
            );
            names.push(name);
        }
        assert!(names.len() >= 30, "{names:?}");
        for l in LIST_LEVEL {
            assert!(names.iter().any(|n| n == l), "{l} missing");
        }
    }

    #[test]
    fn a_command_for_one_repo_never_touches_another() {
        let cfg = Temp::new("cfg");
        let a = Temp::repo("a");
        let b = Temp::repo("b");
        let state = AppState::new(List::new(cfg.0.join(workspaces::FILE)), None);
        let id_a = add(&state, &a.0.display().to_string(), None)
            .expect("add a")
            .id;
        let id_b = add(&state, &b.0.display().to_string(), Some("Bee"))
            .expect("add b")
            .id;
        assert_ne!(id_a, id_b);

        // Work in B, the way the window's commands do.
        let wb = state.ws(&id_b).expect("open b");
        assert_eq!(
            std::fs::canonicalize(&wb.root).ok(),
            std::fs::canonicalize(&b.0).ok()
        );
        let made = new_entity_in(&wb, "task", "Only in B", &[], &[], &[], &[], None)
            .expect("new task in b");
        assert_eq!(made["id"], "only-in-b");
        write_file_in(&wb, "notes.txt", "b's notes", None).expect("write in b");
        assert!(b.0.join(".kitsu/tasks/only-in-b.md").exists());
        assert!(b.0.join("notes.txt").exists());
        let ov_b = overview_of(&wb).expect("overview b");
        assert_eq!(ov_b["repo"]["id"], id_b.as_str());
        assert_eq!(task_ids(&ov_b), ["mine", "only-in-b"]);

        // A was never even opened: no state directory, no files, clean tree.
        assert!(!a.0.join(".git/kitsu").exists(), "A's git dir untouched");
        assert!(!a.0.join(".kitsu/tasks/only-in-b.md").exists());
        assert!(!a.0.join("notes.txt").exists());
        assert!(kitsu::git::Git::new(&a.0).is_clean().expect("status"));
        let opened: Vec<String> = state.opened().into_iter().map(|(id, _)| id).collect();
        assert_eq!(opened, std::slice::from_ref(&id_b));

        // Asking for A gives A, whatever was asked of B before.
        let wa = state.ws(&id_a).expect("open a");
        let ov_a = overview_of(&wa).expect("overview a");
        assert_eq!(ov_a["repo"]["id"], id_a.as_str());
        assert_eq!(task_ids(&ov_a), ["mine"]);

        // Ids that aren't in the list don't resolve to anything.
        assert_eq!(
            state
                .ws("0123456789ab")
                .map(|_| ())
                .expect_err("unknown")
                .kind,
            "no_repo"
        );
        state.0.list.remove(&id_b).expect("remove b");
        state.close(&id_b);
        assert_eq!(
            state.ws(&id_b).map(|_| ()).expect_err("removed").kind,
            "no_repo"
        );
        assert!(b.0.join("notes.txt").exists(), "removing never deletes");
    }

    #[test]
    fn add_wants_a_full_path_and_launch_joins_the_list() {
        let cfg = Temp::new("cfg");
        let a = Temp::repo("a");
        let state = AppState::new(
            List::new(cfg.0.join(workspaces::FILE)),
            Some(a.0.join(".kitsu")),
        );
        let e = add(&state, "relative/dir", None)
            .map(|_| ())
            .expect_err("relative");
        assert!(e.message.contains("full path"), "{}", e.message);
        // Started from a folder inside A: A is added, once.
        let first = launch(&state).expect("launch").expect("a repo");
        let again = launch(&state).expect("launch").expect("a repo");
        assert_eq!(first.id, again.id);
        assert_eq!(state.0.list.load().expect("list").len(), 1);
        let dup = add(&state, &a.0.display().to_string(), None)
            .map(|_| ())
            .expect_err("dup");
        assert_eq!(dup.kind, "conflict");
    }

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
