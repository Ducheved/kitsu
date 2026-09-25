//! The `kitsu` command. The desktop app is a view over the same library and
//! launches `kitsu run` for agents, so everything here also works from a
//! plain terminal with no UI running.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde_json::json;

use crate::agents;
use crate::brief::{self, Context};
use crate::check::{CheckRun, CheckStatus, status_at};
use crate::digest;
use crate::error::{Error, Result};
use crate::git::Git;
use crate::integrate::{self, AcceptOptions, Accepted};
use crate::intent::{self, Intent, Kind, QuestionState};
use crate::recover;
use crate::run::{RunEvent, RunState};
use crate::runner::{self, Options, Policy};
use crate::status::{Attention, Snapshot, ready_order};
use crate::store::{Applied, CheckOutcome};
use crate::util::{ago, now_ms, slugify};
use crate::workspace::{Instance, Liveness, Workspace};

#[derive(Parser)]
#[command(name = "kitsu", version, about = "Tasks, checks and evidence for humans and coding agents.", long_about = None)]
struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Run as if started in this directory.
    #[arg(short = 'C', global = true, value_name = "DIR")]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create .kitsu/ in this repository.
    Init,
    /// Allow Kitsu to run this repository's checks and agents.
    Trust,
    /// What needs you, what's running, what's ready.
    Status,
    /// Tasks that can start right now, in dependency order.
    Next,
    /// Create a task, decision, question, memory note or architecture element.
    New {
        /// task | decision | question | memory | element
        kind: String,
        title: String,
        #[arg(long, value_delimiter = ',')]
        scope: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        checks: Vec<String>,
        /// Tasks this one waits for (tasks only).
        #[arg(long, value_delimiter = ',')]
        after: Vec<String>,
        /// Tasks this blocks (questions only).
        #[arg(long, value_delimiter = ',')]
        blocks: Vec<String>,
        /// Explicit id; defaults to a slug of the title.
        #[arg(long)]
        id: Option<String>,
    },
    /// Show a task, decision, question or run.
    Show { id: String },
    /// Print the brief an agent would get for a task. Inside a run's
    /// worktree with no task, prints the brief that run was given.
    Brief {
        task: Option<String>,
        /// Size limit for optional sections, in estimated tokens.
        #[arg(long, default_value_t = brief::DEFAULT_BUDGET)]
        budget: usize,
    },
    /// Run checks and record the evidence.
    Check {
        names: Vec<String>,
        /// Run in this run's worktree instead of your checkout.
        #[arg(long)]
        run: Option<String>,
    },
    /// Start an agent on a task in its own worktree. Blocks until it ends.
    Run {
        task: String,
        #[arg(long, short, default_value = "claude")]
        agent: String,
        /// Continue from an earlier run's result instead of HEAD.
        #[arg(long)]
        from: Option<String>,
        /// With --from and Kitsu's own agent: pick up that run's
        /// conversation where it stopped (after a crash, a budget, a stop).
        /// Tool calls it left unfinished are settled, never repeated.
        #[arg(long, requires = "from")]
        resume: bool,
        /// A note for the agent, added to its brief.
        #[arg(long)]
        note: Option<String>,
        /// ask: shell commands and network wait for you. auto: anything
        /// inside the worktree is allowed. triage: ask, but Kitsu's own
        /// loop lets a command through when the judge in agents.toml says
        /// it's low-risk and stays in the worktree.
        #[arg(long, default_value = "ask")]
        policy: String,
        #[arg(long)]
        no_verify: bool,
        /// Use this run id (the app picks ids up front).
        #[arg(long, hide = true)]
        id: Option<String>,
        /// Don't stream agent output.
        #[arg(long, short)]
        quiet: bool,
    },
    /// Ask a running agent to stop.
    Stop { run: String },
    /// Answer a live agent's permission request (numeric id) or an open
    /// question (question id).
    Answer { id: String, answer: String },
    /// What a run changed and whether it's acceptable.
    Review {
        run: String,
        #[arg(long)]
        diff: bool,
    },
    /// Put a run's change on your current branch.
    Accept {
        run: String,
        /// Accept but leave the task open.
        #[arg(long)]
        keep_open: bool,
        /// Approve protected-path changes (token from `kitsu review`).
        #[arg(long)]
        approve: Option<String>,
    },
    /// Throw a run's change away.
    Discard { run: String },
    /// The event log.
    Log {
        #[arg(long)]
        run: Option<String>,
        #[arg(long, default_value_t = 0)]
        since: i64,
        /// Keep printing new events.
        #[arg(long, short)]
        follow: bool,
    },
    /// Reconcile after crashes. Also runs automatically.
    Recover,
    /// Known agents and how they are started.
    Agents,
    /// Where the tokens went: per agent, per outcome, per task.
    Stats,
    /// Write down something the next person or agent should know.
    Remember {
        title: String,
        /// fact | gotcha | convention | preference | lesson
        #[arg(long, default_value = "fact")]
        kind: String,
        /// Where it applies (briefs for tasks here include it).
        #[arg(long, value_delimiter = ',')]
        scope: Vec<String>,
        /// Files it describes; the note is flagged when they change.
        /// Defaults to the scope.
        #[arg(long, value_delimiter = ',')]
        anchor: Vec<String>,
        /// The note itself.
        #[arg(long, default_value = "")]
        body: String,
        /// A personal note for every repository (preferences), kept in your
        /// config dir instead of `.kitsu/memory/`.
        #[arg(long)]
        personal: bool,
    },
    /// Memory notes and whether they are still current.
    Memory,
    /// Serve Kitsu's read-only tools over MCP (stdio) for the agent of a
    /// run. The runner offers this to every agent it starts.
    Mcp {
        #[arg(long)]
        run: String,
    },
    /// The architecture model (`.kitsu/architecture/`): elements, what they
    /// claim, what they use. `kitsu arch check` compares it with the files.
    Arch {
        #[command(subcommand)]
        cmd: Option<ArchCmd>,
    },
    /// Search the code at a commit (default HEAD). Builds or refreshes the
    /// local index first; only files that changed are read.
    Search {
        query: Vec<String>,
        #[arg(long, short = 'n', default_value_t = 10)]
        limit: usize,
        #[arg(long, default_value = "HEAD")]
        rev: String,
    },
}

#[derive(Subcommand)]
pub enum ArchCmd {
    /// Fail when the model and the working tree disagree: unknown
    /// references, bad nesting, claimed paths that match nothing, covered
    /// files with no component or several.
    Check,
}

pub fn main() -> std::process::ExitCode {
    main_with(std::env::args_os())
}

pub fn main_with<I, T>(args: I) -> std::process::ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    let json = cli.json;
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            if json {
                println!("{}", json!({ "error": e.kind(), "message": e.to_string() }));
            } else {
                eprintln!("{} {e}", paint("error:", RED));
            }
            std::process::ExitCode::from(match e {
                Error::Denied(_) => 3,
                Error::Conflict(_) => 4,
                Error::NotFound(_) => 5,
                _ => 1,
            })
        }
    }
}

fn dispatch(cli: Cli) -> Result<std::process::ExitCode> {
    let cwd = match &cli.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir().map_err(|e| Error::io("current directory", e))?,
    };
    let ok = std::process::ExitCode::SUCCESS;
    if let Cmd::Agents = cli.cmd {
        return agents_cmd(cli.json).map(|_| ok);
    }
    // Reads the working tree only, so it runs the same in a run's snapshot
    // (where checks run) as in a checkout, with or without a workspace.
    if let Cmd::Arch { cmd } = &cli.cmd {
        return arch_cmd(&cwd, cmd.is_some(), cli.json);
    }
    let ws = Workspace::discover(&cwd)?;
    let json = cli.json;
    match cli.cmd {
        Cmd::Init => init(&ws),
        Cmd::Trust => {
            ws.trust()?;
            println!("trusted {}", ws.root.display());
            Ok(())
        }
        Cmd::Status => status(&ws, json),
        Cmd::Next => {
            let intent = Intent::load_dir(&ws.root)?;
            let order = ready_order(&intent);
            if json {
                println!("{}", json!(order));
            } else if order.is_empty() {
                println!("nothing is ready");
            } else {
                for id in order {
                    println!("{id}  {}", intent.tasks[&id].title);
                }
            }
            Ok(())
        }
        Cmd::New {
            kind,
            title,
            scope,
            checks,
            after,
            blocks,
            id,
        } => new(&ws, &kind, &title, scope, checks, after, blocks, id),
        Cmd::Show { id } => show(&ws, &id, json),
        Cmd::Brief { task, budget } => brief_cmd(&ws, &cwd, task, budget, json),
        Cmd::Check { names, run } => return check(&ws, names, run, json),
        Cmd::Run {
            task,
            agent,
            from,
            resume,
            note,
            policy,
            no_verify,
            id,
            quiet,
        } => {
            let policy = Policy::parse(&policy).ok_or_else(|| {
                Error::Invalid(format!("unknown policy `{policy}` (ask, auto, triage)"))
            })?;
            return run(
                &ws,
                Options {
                    id,
                    task,
                    agent: agents::resolve(&agent)?,
                    from,
                    resume,
                    note,
                    policy,
                    verify: !no_verify,
                },
                quiet,
                json,
            );
        }
        Cmd::Stop { run } => stop(&ws, &run),
        Cmd::Answer { id, answer: text } => answer(&ws, &id, &text),
        Cmd::Review { run, diff } => review(&ws, &run, diff, json),
        Cmd::Accept {
            run,
            keep_open,
            approve,
        } => return accept(&ws, &run, !keep_open, approve, json),
        Cmd::Discard { run } => {
            let store = ws.open_store()?;
            let notes = integrate::discard(&ws, &store, &run)?;
            println!("discarded {run}");
            for n in notes {
                println!("  {n}");
            }
            Ok(())
        }
        Cmd::Log { run, since, follow } => log(&ws, run, since, follow, json),
        Cmd::Stats => stats_cmd(&ws, json),
        Cmd::Remember {
            title,
            kind,
            scope,
            anchor,
            body,
            personal,
        } => remember(&ws, &title, &kind, scope, anchor, &body, personal),
        Cmd::Memory => memory_cmd(&ws, json),
        Cmd::Mcp { run } => {
            ws.open_store()?.run(&run)?;
            let stdin = std::io::stdin();
            crate::mcp::Server::new(ws, run).serve(stdin.lock(), std::io::stdout().lock())
        }
        Cmd::Search { query, limit, rev } => search_cmd(&ws, &query.join(" "), limit, &rev, json),
        Cmd::Recover => {
            let store = ws.open_store()?;
            let r = recover::recover(&ws, &store)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            } else if r.is_empty() {
                println!("nothing to recover");
            } else {
                print_recovery(&r);
            }
            Ok(())
        }
        Cmd::Agents | Cmd::Arch { .. } => unreachable!("handled above"),
    }
    .map(|_| ok)
}

// ---- small output helpers ---------------------------------------------------

const RED: &str = "31";
const GREEN: &str = "32";
const YELLOW: &str = "33";
const DIM: &str = "2";
const BOLD: &str = "1";

fn paint(s: &str, code: &str) -> String {
    if std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn outcome_word(o: CheckOutcome) -> String {
    match o {
        CheckOutcome::Pass => paint("pass", GREEN),
        other => paint(other.as_str(), RED),
    }
}

fn status_word(s: &CheckStatus) -> String {
    match s {
        CheckStatus::Current { outcome, .. } => outcome_word(*outcome),
        CheckStatus::Carried { outcome, .. } => format!(
            "{} {}",
            outcome_word(*outcome),
            paint("(unchanged scope)", DIM)
        ),
        CheckStatus::Stale { changed, more, .. } => {
            let files = if changed.is_empty() {
                String::new()
            } else {
                format!(
                    " since: {}{}",
                    changed.join(", "),
                    if *more > 0 {
                        format!(" +{more}")
                    } else {
                        String::new()
                    }
                )
            };
            format!("{}{}", paint("stale", YELLOW), paint(&files, DIM))
        }
        CheckStatus::Unverified => paint("unverified", YELLOW),
    }
}

fn print_recovery(r: &recover::Report) {
    for id in &r.interrupted {
        println!(
            "{} run {id} was interrupted (its owner is gone); partial work snapshotted",
            paint("recovered:", YELLOW)
        );
    }
    for p in &r.reaped {
        println!("{} stopped orphaned agent {p}", paint("recovered:", YELLOW));
    }
    for p in &r.unknown_processes {
        println!("{} {p}", paint("unknown:", YELLOW));
    }
    for (id, state) in &r.integrations {
        println!("{} integration {id}: {state}", paint("recovered:", YELLOW));
    }
    for c in &r.cleaned {
        println!("{} {c}", paint("cleaned:", DIM));
    }
    for p in &r.problems {
        println!("{} {p}", paint("problem:", RED));
    }
    for o in &r.orphan_worktrees {
        println!("{} {o} is not claimed by any run", paint("orphan:", YELLOW));
    }
}

// ---- commands ---------------------------------------------------------------

const CONFIG_TEMPLATE: &str = r#"# Kitsu reads this file from your checkout. Agents' edits to it are shown
# to you as rule changes and need your approval before they land.

# Checks are how Kitsu learns something is true. Each one is a shell
# command run from the repository root; exit code 0 means pass.
#
# [checks.test]
# run = "cargo test --workspace"
# timeout = "10m"
# # Optional: evidence stays fresh while changes stay outside this scope.
# scope = ["src/**", "Cargo.toml"]

# Changes to these paths need your explicit approval on accept.
# `.kitsu/**` is always protected.
[protect]
paths = []
"#;

fn init(ws: &Workspace) -> Result<()> {
    let base = ws.root.join(intent::DIR);
    for k in Kind::ALL {
        let d = base.join(k.dir());
        std::fs::create_dir_all(&d).map_err(|e| Error::io(d.display().to_string(), e))?;
        let keep = d.join(".gitkeep");
        if !keep.exists() {
            std::fs::write(&keep, "").map_err(|e| Error::io(keep.display().to_string(), e))?;
        }
    }
    let cfg = ws.root.join(intent::CONFIG);
    if !cfg.exists() {
        std::fs::write(&cfg, CONFIG_TEMPLATE)
            .map_err(|e| Error::io(cfg.display().to_string(), e))?;
    }
    println!("created {}", base.display());
    println!("next: add a check to .kitsu/kitsu.toml, then `kitsu new task \"...\"`");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn new(
    ws: &Workspace,
    kind: &str,
    title: &str,
    scope: Vec<String>,
    checks: Vec<String>,
    after: Vec<String>,
    blocks: Vec<String>,
    id: Option<String>,
) -> Result<()> {
    let kind = Kind::parse(kind).ok_or_else(|| {
        Error::Invalid(format!(
            "unknown kind `{kind}` (task, decision, question, memory, element)"
        ))
    })?;
    let text = render_new(kind, title, &scope, &checks, &after, &blocks);
    let path = write_new(ws, kind, title, id.as_deref(), &text)?;
    println!("{}", path.display());
    Ok(())
}

/// Front matter for a new entity. Shared with the desktop app.
pub fn render_new(
    kind: Kind,
    title: &str,
    scope: &[String],
    checks: &[String],
    after: &[String],
    blocks: &[String],
) -> String {
    let list = |v: &[String]| {
        format!(
            "[{}]",
            v.iter().map(|s| toml_str(s)).collect::<Vec<_>>().join(", ")
        )
    };
    let mut front = format!("title = {}\n", toml_str(title));
    match kind {
        Kind::Task => {
            front.push_str(&format!(
                "scope = {}\nchecks = {}\n",
                list(scope),
                list(checks)
            ));
            if !after.is_empty() {
                front.push_str(&format!("after = {}\n", list(after)));
            }
        }
        Kind::Decision => front.push_str(&format!(
            "state = \"accepted\"\nscope = {}\nrejected = []\n",
            list(scope)
        )),
        Kind::Question => front.push_str(&format!("blocks = {}\n", list(blocks))),
        Kind::Architecture => front.push_str(&format!(
            "level = \"component\"\nparent = \"\"\npaths = {}\nuses = []\n",
            list(scope)
        )),
        Kind::Memory => front.push_str(&format!(
            "kind = \"fact\"\nscope = {}\nanchors = {}\n",
            list(scope),
            list(scope)
        )),
    }
    let body = match kind {
        Kind::Task => "What should be true when this is done, and why.\n",
        Kind::Decision => "Context, the choice, and what it costs.\n",
        Kind::Question => "What we need to know and what depends on it.\n",
        Kind::Memory => "What the next person or agent should know, and how you know it.\n",
        Kind::Architecture => "What this is for, what it must never do, and where its seams are.\n",
    };
    format!("+++\n{front}+++\n{body}")
}

pub fn write_new(
    ws: &Workspace,
    kind: Kind,
    title: &str,
    id: Option<&str>,
    text: &str,
) -> Result<PathBuf> {
    let dir = ws.root.join(intent::DIR).join(kind.dir());
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.display().to_string(), e))?;
    let base = id.map(str::to_owned).unwrap_or_else(|| slugify(title));
    intent::valid_id(&base).map_err(Error::Invalid)?;
    let mut n = 1;
    let path = loop {
        let name = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let p = dir.join(format!("{name}.md"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
        {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(text.as_bytes())
                    .map_err(|e| Error::io(p.display().to_string(), e))?;
                break p;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && id.is_none() => n += 1,
            Err(e) => return Err(Error::io(p.display().to_string(), e)),
        }
    };
    Ok(path)
}

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn status(ws: &Workspace, json: bool) -> Result<()> {
    let store = ws.open_store()?;
    let intent = Intent::load_dir(&ws.root)?;
    let git = ws.git();
    let views = Snapshot {
        intent: &intent,
        git: &git,
        store: &store,
    }
    .tasks()?;
    let seen: i64 = store
        .meta("seen.cli")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let d = digest::since(&store, seen)?;
    let asks = store.open_asks()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&json!({ "tasks": views, "asks": asks, "since": d, "problems": intent.problems.iter().map(|p| json!({"path": p.path, "detail": p.detail})).collect::<Vec<_>>() })).unwrap_or_default());
        store.set_meta("seen.cli", &d.to_seq.to_string())?;
        return Ok(());
    }
    if !d.items.is_empty() && seen > 0 {
        println!("{}", paint("Since you last looked", BOLD));
        for i in d.items.iter().take(8) {
            match &i.task {
                Some(t) => println!("  {t}: {}", i.text),
                None => println!("  {}", i.text),
            }
        }
        println!();
    }
    store.set_meta("seen.cli", &d.to_seq.to_string())?;
    for p in &intent.problems {
        println!(
            "{} {}: {}",
            paint("broken rule file:", RED),
            p.path,
            p.detail
        );
    }
    if !ws.is_trusted()? {
        println!(
            "{} this repository is not trusted, so checks and agents won't run. `kitsu trust` to allow.",
            paint("note:", YELLOW)
        );
    }
    for a in &asks {
        let opts: Vec<String> = a.request["options"]
            .as_array()
            .map(|o| {
                o.iter()
                    .filter_map(|x| x["optionId"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        println!(
            "{} run {} asks: {}   {}",
            paint("?", YELLOW),
            a.run,
            a.request["title"].as_str().unwrap_or("?"),
            paint(&format!("kitsu answer {} <{}>", a.id, opts.join("|")), DIM)
        );
        if let Some(note) = a.request["judge"]["note"].as_str() {
            println!("    {}", paint(note, DIM));
        }
    }
    let mut group: Option<Attention> = None;
    for v in &views {
        if group != Some(v.attention) {
            group = Some(v.attention);
            let heading = match v.attention {
                Attention::NeedsYou => "Needs you",
                Attention::Working => "Working",
                Attention::Ready => "Ready",
                Attention::Waiting => "Waiting",
                Attention::Quiet => "Done",
            };
            println!("{}", paint(heading, BOLD));
        }
        println!("  {:<28} {}  {}", v.id, v.title, paint(&v.reason, DIM));
    }
    if views.is_empty() {
        println!("no tasks yet. `kitsu new task \"...\"`");
    }
    Ok(())
}

fn show(ws: &Workspace, id: &str, json: bool) -> Result<()> {
    let intent = Intent::load_dir(&ws.root)?;
    let found = intent.find(id);
    if let Some((kind, source)) = found.first() {
        let text = std::fs::read_to_string(ws.root.join(&source.path))
            .map_err(|e| Error::io(source.path.clone(), e))?;
        if json {
            println!(
                "{}",
                json!({ "kind": kind.name(), "path": source.path, "text": text })
            );
        } else {
            println!("{} {}", paint(kind.name(), DIM), paint(&source.path, DIM));
            print!("{text}");
            if *kind == Kind::Task {
                show_checks(ws, &intent, id, *kind)?;
            }
        }
        return Ok(());
    }
    let store = ws.open_store()?;
    let run = store.run(id)?;
    if json {
        let events = store.run_events(id, 0, 10_000)?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "run": run, "events": events, "evidence": store.evidence_for_run(id)? })
            )
            .unwrap_or_default()
        );
        return Ok(());
    }
    let now = now_ms();
    println!(
        "{} {}  task {}  agent {}  {}",
        paint("run", DIM),
        run.id,
        run.task,
        run.agent,
        paint(&ago(run.created_at, now), DIM)
    );
    let state = match (run.state, &run.stop_reason) {
        (RunState::Finished, Some(r)) => format!("finished ({r})"),
        (s, _) => s.as_str().to_string(),
    };
    println!(
        "state {state}{}",
        run.resolution
            .as_deref()
            .map(|r| format!(", {r}"))
            .unwrap_or_default()
    );
    if let Some(d) = &run.detail {
        println!("detail {d}");
    }
    println!("worktree {}", run.worktree);
    for e in store.run_events(id, 0, 10_000)? {
        let line = match e.kind.as_str() {
            "agent.message" => e.body["text"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .replace('\n', " "),
            "agent.tool" => format!(
                "[{}] {}",
                e.body["status"].as_str().unwrap_or("·"),
                e.body["title"].as_str().unwrap_or("tool")
            ),
            "agent.plan" => format!(
                "plan: {}",
                e.body["entries"]
                    .as_array()
                    .map(|a| a
                        .iter()
                        .filter_map(|x| x["content"].as_str())
                        .collect::<Vec<_>>()
                        .join(" / "))
                    .unwrap_or_default()
            ),
            "permission" => format!(
                "{}: {} ({}{})",
                if e.body["decision"]
                    .as_str()
                    .is_some_and(|d| d.starts_with("allow"))
                {
                    "allowed"
                } else {
                    "decided"
                },
                e.body["title"].as_str().unwrap_or("?"),
                e.body["by"].as_str().unwrap_or("?"),
                match (e.body["judgment"].as_i64(), e.body["p_yes"].as_f64()) {
                    (Some(j), Some(p)) => format!(", judgment {j}: p={p:.2}"),
                    (Some(j), None) => format!(", judgment {j}: unknown"),
                    _ => String::new(),
                }
            ),
            "ask.open" => format!(
                "asked you: {}",
                e.body["request"]["title"].as_str().unwrap_or("?")
            ),
            "ask.answered" => format!("you answered: {}", e.body["answer"].as_str().unwrap_or("?")),
            "check.done" => format!(
                "check {} {}",
                e.body["check"].as_str().unwrap_or("?"),
                e.body["outcome"].as_str().unwrap_or("?")
            ),
            "run.state" => format!(
                "-> {}{}",
                e.body["to"].as_str().unwrap_or("?"),
                e.body["detail"]
                    .as_str()
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default()
            ),
            "run.snapshot" => format!(
                "snapshot {}",
                &e.body["commit"].as_str().unwrap_or("?")
                    [..10.min(e.body["commit"].as_str().unwrap_or("?").len())]
            ),
            _ => continue,
        };
        let short: String = line.chars().take(140).collect();
        println!("  {} {short}", paint(&format!("{:>5}", e.seq), DIM));
    }
    Ok(())
}

fn show_checks(ws: &Workspace, intent: &Intent, id: &str, kind: Kind) -> Result<()> {
    let names: Vec<String> = match kind {
        Kind::Task => crate::status::required_checks(intent, &intent.tasks[id], None)
            .into_iter()
            .map(|r| r.name)
            .collect(),
        _ => Vec::new(),
    };
    if names.is_empty() {
        return Ok(());
    }
    let store = ws.open_store()?;
    let git = ws.git();
    let tree = git.worktree_tree(&ws.scratch())?;
    println!("\n{}", paint("checks on your working tree", DIM));
    for n in names {
        if let Some(def) = intent.config.checks.get(&n) {
            println!(
                "  {n:<16} {}",
                status_word(&status_at(&git, &store, def, &tree)?)
            );
        }
    }
    Ok(())
}

fn brief_cmd(
    ws: &Workspace,
    cwd: &Path,
    task: Option<String>,
    budget: usize,
    json: bool,
) -> Result<()> {
    let store = ws.open_store()?;
    let task = match task {
        Some(t) => t,
        None => {
            let top = Git::new(cwd).toplevel()?;
            let run_id = ws.run_for_path(&top).ok_or_else(|| {
                Error::Invalid("give a task id (or run this inside a run's worktree)".into())
            })?;
            let run = store.run(&run_id)?;
            let blob = run
                .brief
                .ok_or_else(|| Error::NotFound(format!("brief of run {run_id}")))?;
            print!("{}", String::from_utf8_lossy(&ws.blobs().get(&blob)?));
            return Ok(());
        }
    };
    let intent = Intent::load_dir(&ws.root)?;
    let t = intent
        .tasks
        .get(&task)
        .ok_or_else(|| Error::NotFound(format!("task {task}")))?;
    let git = ws.git();
    let head = git.head()?;
    let b = brief::compile(
        &Context {
            intent: &intent,
            git: Some(&git),
            store: Some(&store),
            base: head.as_deref(),
            worktree: None,
            run: None,
            personal: &crate::memory::personal(&crate::workspace::config_dir()),
            budget,
        },
        t,
    );
    if json {
        println!("{}", serde_json::to_string_pretty(&b).unwrap_or_default());
    } else {
        print!("{}", b.markdown);
    }
    Ok(())
}

fn check(
    ws: &Workspace,
    names: Vec<String>,
    run: Option<String>,
    json: bool,
) -> Result<std::process::ExitCode> {
    if !ws.is_trusted()? {
        return Err(Error::Denied(
            "this repository is not trusted; checks run its code. `kitsu trust` to allow".into(),
        ));
    }
    let store = ws.open_store()?;
    let intent = Intent::load_dir(&ws.root)?;
    let dir = match &run {
        Some(r) => PathBuf::from(store.run(r)?.worktree),
        None => ws.root.clone(),
    };
    let names: Vec<String> = if names.is_empty() {
        intent.config.checks.keys().cloned().collect()
    } else {
        names
    };
    if names.is_empty() {
        return Err(Error::NotFound(
            "no checks defined in .kitsu/kitsu.toml".into(),
        ));
    }
    let cr = CheckRun {
        ws,
        store: &store,
        dir: &dir,
        run: run.as_deref(),
    };
    let mut all_pass = true;
    let mut out = Vec::new();
    for n in names {
        let def = intent
            .config
            .checks
            .get(&n)
            .ok_or_else(|| Error::NotFound(format!("check {n}")))?;
        if !json {
            eprint!("{} {n} ... ", paint("check", DIM));
        }
        let e = cr.execute(def)?;
        all_pass &= e.outcome == CheckOutcome::Pass;
        if !json {
            let bound = if e.is_bound() {
                String::new()
            } else {
                let moved = e
                    .tree_after
                    .as_deref()
                    .and_then(|after| Git::new(&dir).changed_paths(&e.tree, after).ok())
                    .unwrap_or_default();
                let shown: Vec<&str> = moved.iter().take(3).map(String::as_str).collect();
                let more = if moved.len() > 3 { ", ..." } else { "" };
                paint(
                    &format!(
                        " (it changed {}{more} while running, so the result isn't tied to a tree; build output belongs in .gitignore)",
                        shown.join(", ")
                    ),
                    YELLOW,
                )
            };
            eprintln!(
                "{} {}{bound}",
                outcome_word(e.outcome),
                paint(&format!("{:.1}s", e.duration_ms as f64 / 1000.0), DIM)
            );
            if e.outcome != CheckOutcome::Pass
                && let Some(log) = &e.log
            {
                let text = ws.blobs().get(log)?;
                let text = String::from_utf8_lossy(&text);
                let tail: Vec<&str> = text.lines().rev().take(15).collect();
                for l in tail.into_iter().rev() {
                    eprintln!("    {l}");
                }
            }
        }
        out.push(e);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    }
    Ok(if all_pass {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::from(2)
    })
}

fn run(ws: &Workspace, opts: Options, quiet: bool, json: bool) -> Result<std::process::ExitCode> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::io("starting async runtime", e))?;
    // Before anything can target this process with a nudge.
    let mut wake = rt.block_on(async { runner::Wake::install() })?;
    let store = ws.open_store()?;
    let me = Instance::acquire(ws)?;
    let report = recover::recover(ws, &store)?;
    if !report.is_empty() && !quiet && !json {
        print_recovery(&report);
    }
    let prep = runner::prepare(ws, &store, &me, &opts)?;
    if !json {
        eprintln!(
            "{} {} on {} in {}",
            paint("run", DIM),
            prep.id,
            opts.task,
            prep.worktree.display()
        );
    }
    let driven = rt.block_on(runner::drive(
        ws,
        &store,
        &prep,
        &opts,
        !quiet && !json,
        &mut wake,
    ));
    // Whatever went wrong after the run row exists goes into the run, so
    // it never sits in "starting" with the reason only on a closed stderr.
    let state = match driven {
        Ok(s) => s,
        Err(e) => {
            let event = match store.run(&prep.id).map(|r| r.state) {
                Ok(RunState::Starting) => RunEvent::StartFailed(e.to_string()),
                _ => RunEvent::ProtocolError(format!("kitsu failed while supervising: {e}")),
            };
            let _ = store.apply_run_event(&prep.id, &event);
            let _ = runner::finish(ws, &store, &prep.id, false);
            return Err(e);
        }
    };
    let snapshot = runner::finish(ws, &store, &prep.id, opts.verify)?;
    let run = store.run(&prep.id)?;
    if json {
        println!(
            "{}",
            json!({ "run": run, "snapshot": snapshot, "evidence": store.evidence_for_run(&prep.id)? })
        );
    } else {
        let reason = run
            .stop_reason
            .as_deref()
            .map(|r| format!(" ({r})"))
            .unwrap_or_default();
        eprintln!(
            "{} {}{reason}{}",
            paint("run", DIM),
            state.as_str(),
            run.detail
                .as_deref()
                .map(|d| format!(": {d}"))
                .unwrap_or_default()
        );
        for e in store.evidence_for_run(&prep.id)? {
            eprintln!("  check {:<14} {}", e.check_name, outcome_word(e.outcome));
        }
        eprintln!("{} kitsu review {}", paint("next:", DIM), prep.id);
    }
    Ok(match state {
        RunState::Finished => std::process::ExitCode::SUCCESS,
        _ => std::process::ExitCode::from(2),
    })
}

fn stop(ws: &Workspace, run: &str) -> Result<()> {
    let store = ws.open_store()?;
    let row = store.run(run)?;
    if row.state.is_terminal() {
        println!("run {run} already {}", row.state.as_str());
        return Ok(());
    }
    // An owner that is gone can't hear us; recovery settles it now.
    if let Some(owner) = &row.owner
        && Instance::liveness(ws, owner)? == Liveness::Dead
    {
        let r = recover::recover(ws, &store)?;
        print_recovery(&r);
        return Ok(());
    }
    let applied = store.apply_run_event(run, &RunEvent::CancelRequested)?;
    if let Some(owner) = &row.owner {
        Instance::nudge(ws, owner);
    }
    match applied {
        Applied::Moved(_) | Applied::Unchanged => {
            println!("asked run {run} to stop; the agent gets a cancel within ~100ms")
        }
        Applied::Duplicate => {}
    }
    Ok(())
}

fn answer(ws: &Workspace, id: &str, text: &str) -> Result<()> {
    if let Ok(n) = id.parse::<i64>() {
        let store = ws.open_store()?;
        let ask = store.ask(n)?;
        let options: Vec<String> = ask.request["options"]
            .as_array()
            .map(|o| {
                o.iter()
                    .filter_map(|x| x["optionId"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if !options.iter().any(|o| o == text) {
            return Err(Error::Invalid(format!(
                "`{text}` is not one of the options: {}",
                options.join(", ")
            )));
        }
        if store.answer_ask(n, text)? {
            if let Some(owner) = store.run(&ask.run)?.owner {
                Instance::nudge(ws, &owner);
            }
            println!("answered");
        } else {
            println!(
                "already answered: {}",
                store.ask(n)?.answer.unwrap_or_default()
            );
        }
        return Ok(());
    }
    let intent = Intent::load_dir(&ws.root)?;
    let q = intent
        .questions
        .get(id)
        .ok_or_else(|| Error::NotFound(format!("question {id}")))?;
    if q.state == QuestionState::Answered {
        return Err(Error::Conflict(format!(
            "question {id} is already answered"
        )));
    }
    let path = ws.root.join(&q.source.path);
    let current =
        std::fs::read_to_string(&path).map_err(|e| Error::io(path.display().to_string(), e))?;
    let edited = answer_question(&current, text).map_err(|detail| Error::Parse {
        path: path.clone(),
        detail,
    })?;
    std::fs::write(&path, edited).map_err(|e| Error::io(path.display().to_string(), e))?;
    println!("answered {}", q.source.path);
    Ok(())
}

/// Mark a question answered in place. Shared with the desktop app.
pub fn answer_question(text: &str, answer: &str) -> std::result::Result<String, String> {
    let with_state = intent::set_state(text, "answered")?;
    let (front, _) = intent::split_front_matter(&with_state)?;
    let start = front.as_ptr() as usize - with_state.as_ptr() as usize;
    let end = start + front.len();
    let kept: String = front
        .split_inclusive('\n')
        .filter(|l| !l.trim_start().starts_with("answer"))
        .collect();
    Ok(format!(
        "{}{kept}answer = {}\n{}",
        &with_state[..start],
        toml_str(answer),
        &with_state[end..]
    ))
}

fn review(ws: &Workspace, run: &str, diff: bool, json: bool) -> Result<()> {
    let store = ws.open_store()?;
    let r = integrate::review(ws, &store, run)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
        return Ok(());
    }
    let row = store.run(run)?;
    println!("run {} on {} → {}", r.run, row.task, r.target);
    for f in &r.files {
        let n = match (f.added, f.removed) {
            (Some(a), Some(d)) => format!(
                "{} {}",
                paint(&format!("+{a}"), GREEN),
                paint(&format!("-{d}"), RED)
            ),
            _ => "binary".into(),
        };
        let flag = if r.protected.contains(&f.path) && intent::is_memory_note(&f.path) {
            paint("  proposed note", YELLOW)
        } else if r.protected.contains(&f.path) {
            paint("  protected", YELLOW)
        } else {
            String::new()
        };
        println!("  {:<48} {n}{flag}", f.path);
    }
    if r.files.is_empty() {
        println!("  no changes");
    }
    for (name, s) in &r.checks {
        println!("  check {name:<14} {}", status_word(s));
    }
    if let Some(t) = &r.approval_token {
        let what = if r.protected.iter().all(|p| intent::is_memory_note(p)) {
            "proposes memory notes"
        } else {
            "changes rules or protected files"
        };
        println!(
            "{} {what}; review them, then: kitsu accept {run} --approve {t}",
            paint("note:", YELLOW)
        );
    }
    if diff {
        let snap = row.snapshot.unwrap_or_default();
        print!("{}", ws.git().diff(&r.from, &snap, &[])?);
    }
    Ok(())
}

fn accept(
    ws: &Workspace,
    run: &str,
    close_task: bool,
    approve: Option<String>,
    json: bool,
) -> Result<std::process::ExitCode> {
    let store = ws.open_store()?;
    let me = Instance::acquire(ws)?;
    let _ = recover::recover(ws, &store)?;
    let res = integrate::accept(
        ws,
        &store,
        &me,
        run,
        &AcceptOptions {
            close_task,
            approval: approve,
        },
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
    }
    Ok(match res {
        Accepted::Applied {
            commit,
            closed_task,
            notes,
        } => {
            if !json {
                println!(
                    "{} {}{}",
                    paint("accepted", GREEN),
                    &commit[..10],
                    if closed_task { ", task closed" } else { "" }
                );
                for n in notes {
                    println!("  {n}");
                }
            }
            std::process::ExitCode::SUCCESS
        }
        Accepted::NeedsApproval { paths, token } => {
            if !json {
                println!(
                    "{} this run changes protected paths:",
                    paint("needs approval:", YELLOW)
                );
                for p in paths {
                    println!("  {p}");
                }
                println!(
                    "look at them (kitsu review {run} --diff), then: kitsu accept {run} --approve {token}"
                );
            }
            std::process::ExitCode::from(3)
        }
        Accepted::Conflict { paths } => {
            if !json {
                println!(
                    "{} with your branch in: {}",
                    paint("conflict", RED),
                    paths.join(", ")
                );
                println!("continue on top of the current branch: kitsu run <task> --from {run}");
            }
            std::process::ExitCode::from(4)
        }
        Accepted::ChecksFailed { failing, .. } => {
            if !json {
                println!(
                    "{} on the combined result: {}",
                    paint("checks failed", RED),
                    failing.join(", ")
                );
            }
            std::process::ExitCode::from(2)
        }
    })
}

fn remember(
    ws: &Workspace,
    title: &str,
    kind: &str,
    scope: Vec<String>,
    anchor: Vec<String>,
    body: &str,
    personal: bool,
) -> Result<()> {
    let kind = intent::MemoryKind::parse(kind).ok_or_else(|| {
        Error::Invalid(format!(
            "unknown memory kind `{kind}` (fact, gotcha, convention, preference, lesson)"
        ))
    })?;
    let list = |v: &[String]| {
        format!(
            "[{}]",
            v.iter().map(|s| toml_str(s)).collect::<Vec<_>>().join(", ")
        )
    };
    let anchors = if anchor.is_empty() {
        scope.clone()
    } else {
        anchor
    };
    let mut front = format!(
        "title = {}\nkind = \"{}\"\n",
        toml_str(title),
        kind.as_str()
    );
    if !personal {
        front.push_str(&format!(
            "scope = {}\nanchors = {}\n",
            list(&scope),
            list(&anchors)
        ));
    }
    let body = if body.trim().is_empty() {
        "How you know this, and what would make it stop being true.\n".to_string()
    } else {
        format!("{}\n", body.trim_end())
    };
    let text = format!("+++\n{front}+++\n{body}");
    let path = if personal {
        let dir = crate::workspace::config_dir().join("memory");
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.display().to_string(), e))?;
        let p = dir.join(format!("{}.md", slugify(title)));
        if p.exists() {
            return Err(Error::Conflict(format!("{} already exists", p.display())));
        }
        std::fs::write(&p, text).map_err(|e| Error::io(p.display().to_string(), e))?;
        p
    } else {
        write_new(ws, Kind::Memory, title, None, &text)?
    };
    println!("{}", path.display());
    Ok(())
}

fn search_cmd(ws: &Workspace, query: &str, limit: usize, rev: &str, json: bool) -> Result<()> {
    let git = ws.git();
    let mut ix = crate::index::Index::open(&ws.state.join("index.db"))?;
    let updated = ix.update(&git, rev)?;
    let hits = ix.search(&git, query, limit)?;
    if json {
        println!("{}", json!({ "index": updated, "hits": hits }));
        return Ok(());
    }
    if updated.new_blobs > 0 {
        eprintln!(
            "{}",
            paint(
                &format!(
                    "indexed {} new files in {} ms",
                    updated.new_blobs, updated.ms
                ),
                DIM
            )
        );
    }
    if hits.is_empty() {
        println!("no matches");
    }
    for h in &hits {
        println!(
            "{}",
            paint(&format!("{}:{}-{}", h.path, h.start, h.end), BOLD)
        );
        for (n, line) in &h.lines {
            println!("  {n:>5}  {line}");
        }
    }
    Ok(())
}

fn memory_cmd(ws: &Workspace, json: bool) -> Result<()> {
    use crate::memory::{self, Freshness};
    let intent = Intent::load_dir(&ws.root)?;
    let git = ws.git();
    let fresh = match git.head()? {
        Some(h) => memory::freshness(&git, &h, intent.memory.values())?,
        None => Default::default(),
    };
    let personal = memory::personal(&crate::workspace::config_dir());
    if json {
        let rows: Vec<_> = intent
            .memory
            .values()
            .chain(personal.notes.iter())
            .map(|m| json!({ "id": m.id, "title": m.title, "kind": m.kind, "path": m.source.path, "freshness": fresh.get(&m.id) }))
            .collect();
        println!(
            "{}",
            json!({ "notes": rows, "problems": personal.problems })
        );
        return Ok(());
    }
    if intent.memory.is_empty() && personal.notes.is_empty() {
        println!("nothing remembered yet; `kitsu remember \"...\" --kind gotcha --scope <paths>`");
    }
    let superseded = intent.superseded();
    for m in intent.memory.values().chain(personal.notes.iter()) {
        let state = if m.state == intent::MemoryState::Retired {
            paint("retired", DIM)
        } else if let Some(by) = superseded.get(&m.id) {
            paint(&format!("superseded by {by}"), DIM)
        } else {
            match fresh.get(&m.id) {
                Some(Freshness::Stale { changed, .. }) => {
                    paint(&format!("stale: {} changed", changed.join(", ")), YELLOW)
                }
                Some(Freshness::Uncommitted) => paint("not committed", DIM),
                Some(Freshness::Current) => paint("current", GREEN),
                _ => String::new(),
            }
        };
        println!("{:<11} {:<28} {}  {state}", m.kind.as_str(), m.id, m.title);
    }
    for (p, d) in &personal.problems {
        println!("{} {p}: {d}", paint("problem:", RED));
    }
    Ok(())
}

fn arch_cmd(cwd: &Path, check: bool, json: bool) -> Result<std::process::ExitCode> {
    let git = crate::git::Git::new(cwd);
    let root = PathBuf::from(git.run(["rev-parse", "--show-toplevel"])?.trim());
    let intent = Intent::load_dir(&root)?;
    let listed =
        crate::git::Git::new(&root).run(["ls-files", "-z", "-c", "-o", "--exclude-standard"])?;
    let mut files: Vec<String> = listed
        .split('\0')
        .filter(|f| !f.is_empty() && root.join(f).is_file())
        .map(str::to_string)
        .collect();
    files.sort();
    files.dedup();
    let report = crate::arch::check(&intent, &files);
    let code = if report.ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    };
    if json {
        println!("{}", serde_json::to_string(&report).unwrap_or_default());
        return Ok(code);
    }
    if !check {
        print_arch_tree(&intent);
    }
    for e in &report.errors {
        println!("{} {e}", paint("✗", RED));
    }
    for w in &report.warnings {
        println!("{} {w}", paint("!", YELLOW));
    }
    if report.ok() {
        println!(
            "{} {} elements, {} covered files each in one component",
            paint("✓", GREEN),
            report.elements,
            report.covered_files
        );
    }
    Ok(code)
}

fn print_arch_tree(intent: &Intent) {
    use crate::intent::{Element, ElementState};
    let els = &intent.architecture;
    fn show(els: &std::collections::BTreeMap<String, Element>, e: &Element, depth: usize) {
        let state = match e.state {
            ElementState::Active => String::new(),
            ElementState::Planned => paint(" (planned)", DIM),
            ElementState::Retired => paint(" (retired)", DIM),
        };
        let uses = if e.uses.is_empty() {
            String::new()
        } else {
            let to: Vec<&str> = e.uses.iter().map(|u| u.to.as_str()).collect();
            paint(&format!("  → {}", to.join(", ")), DIM)
        };
        println!(
            "{}{} {}{state}{uses}",
            "  ".repeat(depth),
            paint(e.level.as_str(), DIM),
            e.id
        );
        for c in els
            .values()
            .filter(|c| c.parent.as_deref() == Some(e.id.as_str()))
        {
            show(els, c, depth + 1);
        }
    }
    for e in els.values().filter(|e| e.parent.is_none()) {
        show(els, e, 0);
    }
}

fn stats_cmd(ws: &Workspace, json: bool) -> Result<()> {
    use crate::stats::{self, Totals, short};
    let store = ws.open_store()?;
    let runs = store.recent_runs(100_000)?;
    let blobs = ws.blobs();
    let s = stats::compute(&runs, |r| {
        let b = r.brief.as_deref()?;
        std::fs::metadata(blobs.path(b)).ok().map(|m| m.len())
    });
    if json {
        println!("{}", serde_json::to_string(&s).unwrap_or_default());
        return Ok(());
    }
    if s.all.runs == 0 {
        println!("no runs yet");
        return Ok(());
    }
    let cost = |t: &Totals| {
        t.cost
            .iter()
            .map(|(c, v)| format!("{v:.2} {c}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let line = |name: &str, t: &Totals| {
        let cache = t
            .cache_share()
            .map(|c| format!("  {:.0}% from cache", c * 100.0))
            .unwrap_or_default();
        let silent = t.runs - t.reported;
        let silent = if silent > 0 {
            paint(&format!("  ({silent} didn't report)"), DIM)
        } else {
            String::new()
        };
        let money = cost(t);
        let money = if money.is_empty() {
            money
        } else {
            format!("  {money}")
        };
        println!(
            "  {name:<12} {:>4} runs  {:>6} tokens  ({} in, {} out){cache}{money}{silent}",
            t.runs,
            short(t.spent),
            short(t.input),
            short(t.output)
        );
    };
    println!(
        "{}",
        paint(
            &format!(
                "{} runs, {} reported usage, {} tokens",
                s.all.runs,
                s.all.reported,
                short(s.all.spent)
            ),
            BOLD
        )
    );
    if s.all.reported == 0 {
        println!("none of the agents reported token usage (ACP makes it optional)");
    }
    println!("\nby agent");
    for (a, t) in &s.by_agent {
        line(a, t);
    }
    println!("\nby outcome");
    for (o, t) in &s.by_outcome {
        line(o, t);
    }
    if let Some(d) = s
        .by_outcome
        .get("discarded")
        .filter(|d| d.spent > 0 && s.all.spent > 0)
    {
        println!(
            "  {} of reported tokens went to attempts you threw away",
            paint(
                &format!("{:.0}%", d.spent as f64 * 100.0 / s.all.spent as f64),
                YELLOW
            )
        );
    }
    if !s.top_tasks.is_empty() {
        println!("\ntasks that took the most");
        for (task, t) in &s.top_tasks {
            line(task, t);
        }
    }
    if let (Some(median), Some((task, max))) = (s.brief_median, &s.brief_max) {
        println!(
            "\nbriefs: median ~{} tokens, largest ~{} ({task}); estimated from size",
            short(median),
            short(*max)
        );
    }
    Ok(())
}

fn log(ws: &Workspace, run: Option<String>, since: i64, follow: bool, json: bool) -> Result<()> {
    let store = ws.open_store()?;
    let mut seq = since;
    loop {
        let events = match &run {
            Some(r) => store.run_events(r, seq, 1000)?,
            None => store.events_after(seq, 1000)?,
        };
        for e in &events {
            seq = e.seq;
            if json {
                println!("{}", serde_json::to_string(e).unwrap_or_default());
            } else {
                println!(
                    "{:>6} {} {:<18} {}",
                    e.seq,
                    e.run.as_deref().unwrap_or("-"),
                    e.kind,
                    e.body
                );
            }
        }
        if !follow {
            return Ok(());
        }
        if events.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
}

fn agents_cmd(json: bool) -> Result<()> {
    let list = agents::all()?;
    if json {
        let v: Vec<_> = list
            .iter()
            .map(|a| json!({ "name": a.name, "command": a.command, "source": a.source, "pass_env": a.pass_env, "env": a.env.keys().collect::<Vec<_>>(), "mcp": a.mcp }))
            .collect();
        println!("{}", json!(v));
    } else {
        for a in list {
            println!(
                "{:<10} {:<48} {}",
                a.name,
                a.command.join(" "),
                paint(a.source, DIM)
            );
            if !a.pass_env.is_empty() {
                println!(
                    "{:<10} {}",
                    "",
                    paint(&format!("also gets: {}", a.pass_env.join(" ")), DIM)
                );
            }
        }
        let (_, applied) = agents::limited(
            &[],
            agents::limits()?,
            &agents::allowed_cpus(),
            agents::on_path,
        );
        if !applied.is_empty() {
            println!(
                "\n{}",
                paint(
                    &format!("every agent runs with: {}", applied.join(", ")),
                    DIM
                )
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answering_a_question_edits_front_matter_only() {
        let q = "+++\ntitle = \"Dedupe?\"\nblocks = [\"t\"]\n+++\nContext here.\n";
        let a = answer_question(q, "Yes, \"24h\"").expect("answer");
        let i = Intent::from_files(vec![(
            ".kitsu/questions/q.md".into(),
            a.clone().into_bytes(),
        )]);
        let parsed = &i.questions["q"];
        assert_eq!(parsed.state, QuestionState::Answered);
        assert_eq!(parsed.answer.as_deref(), Some("Yes, \"24h\""));
        assert!(a.ends_with("Context here.\n"));
    }

    #[test]
    fn new_files_parse_cleanly() {
        for k in Kind::ALL {
            let text = render_new(k, "A \"quoted\" title", &["src/**".into()], &[], &[], &[]);
            let path = format!(".kitsu/{}/x.md", k.dir());
            let i = Intent::from_files(vec![(path, text.into_bytes())]);
            assert!(i.problems.is_empty(), "{k:?}: {:?}", i.problems);
        }
    }
}
