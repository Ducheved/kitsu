//! Scale probes. Not micro-benchmarks: each one times an operation the UI or
//! a worker actually performs, at sizes the design claims to handle.
//!
//!   cargo run --release -p kitsu --example scale -- state 1001
//!   cargo run --release -p kitsu --example scale -- ingest 1001 10
//!   cargo run --release -p kitsu --example scale -- procs 200
//!
//! Prints one JSON line per measurement so results can be diffed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use kitsu::brief::{self, Context};
use kitsu::intent::Intent;
use kitsu::run::RunEvent;
use kitsu::status::Snapshot;
use kitsu::store::{NewRun, Store};
use kitsu::workspace::Workspace;
use serde_json::json;

fn rss_kib(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn temp_repo(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kitsu-scale-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".kitsu/tasks")).expect("mkdir");
    std::fs::create_dir_all(root.join(".kitsu/invariants")).expect("mkdir");
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git")
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Bench"]);
    git(&["config", "user.email", "bench@example.com"]);
    git(&["config", "commit.gpgsign", "false"]);
    root
}

fn report(what: &str, v: serde_json::Value) {
    println!("{}", json!({ "probe": what, "result": v }));
}

/// N tasks, N/4 invariants, N runs spread across every state. Times what
/// `kitsu status` and the app's task list do on every refresh.
fn state(n: usize) {
    let root = temp_repo("state");
    std::fs::write(
        root.join(".kitsu/kitsu.toml"),
        "[checks.unit]\nrun = \"true\"\n",
    )
    .expect("cfg");
    for i in 0..n {
        let after = if i > 0 && i % 7 == 0 {
            format!("after = [\"t{}\"]\n", i - 1)
        } else {
            String::new()
        };
        std::fs::write(
            root.join(format!(".kitsu/tasks/t{i}.md")),
            format!("+++\ntitle = \"Task number {i}\"\nscope = [\"src/m{}/**\"]\nchecks = [\"unit\"]\n{after}+++\nBody of task {i}.\n", i % 50),
        )
        .expect("task");
    }
    for i in 0..n / 4 {
        std::fs::write(
            root.join(format!(".kitsu/invariants/i{i}.md")),
            format!("+++\ntitle = \"Invariant {i}\"\nscope = [\"src/m{}/**\"]\nchecks = [\"unit\"]\n+++\nWhy it matters.\n", i % 50),
        )
        .expect("inv");
    }
    std::fs::write(root.join("README.md"), "x").expect("readme");
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(&root)
        .status();
    let _ = Command::new("git")
        .args(["commit", "-qm", "init"])
        .current_dir(&root)
        .status();
    let ws = Workspace::discover(&root).expect("ws");
    let store = ws.open_store().expect("store");
    let head = ws.git().head().expect("head").expect("some");

    let t = Instant::now();
    let intent = Intent::load_dir(&root).expect("intent");
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    assert!(intent.problems.is_empty(), "{:?}", intent.problems.first());

    // Runs: 1/3 running, 1/3 failed, 1/3 finished without snapshot. No
    // snapshots means no git work: this isolates the store + derivation.
    let t = Instant::now();
    for i in 0..n {
        let id = format!("r{i}");
        store
            .insert_run(&NewRun {
                id: &id,
                task: &format!("t{i}"),
                agent: "test",
                base: &head,
                branch: "b",
                worktree: "/nonexistent",
                owner: "p",
                from_run: None,
                note: None,
            })
            .expect("run");
        store
            .apply_run_event(&id, &RunEvent::Started)
            .expect("start");
        match i % 3 {
            0 => {}
            1 => {
                store
                    .apply_run_event(&id, &RunEvent::Exited("x".into()))
                    .expect("fail");
            }
            _ => {
                store
                    .apply_run_event(&id, &RunEvent::TurnEnded("end_turn".into()))
                    .expect("end");
            }
        }
    }
    let insert_ms = t.elapsed().as_secs_f64() * 1000.0;

    let git = ws.git();
    let t = Instant::now();
    let views = Snapshot {
        intent: &intent,
        git: &git,
        store: &store,
    }
    .tasks()
    .expect("tasks");
    let status_ms = t.elapsed().as_secs_f64() * 1000.0;

    let task = &intent.tasks["t1"];
    let t = Instant::now();
    let b = brief::compile(
        &Context {
            intent: &intent,
            git: Some(&git),
            store: Some(&store),
            base: Some(&head),
            worktree: None,
            run: None,
            budget: brief::DEFAULT_BUDGET,
        },
        task,
    );
    let brief_ms = t.elapsed().as_secs_f64() * 1000.0;

    let db = std::fs::metadata(ws.state.join("state.db"))
        .map(|m| m.len())
        .unwrap_or(0)
        + std::fs::metadata(ws.state.join("state.db-wal"))
            .map(|m| m.len())
            .unwrap_or(0);
    report(
        "state",
        json!({
            "tasks": n, "invariants": n / 4, "runs": n,
            "load_intent_ms": load_ms, "insert_runs_ms": insert_ms, "status_ms": status_ms,
            "brief_ms": brief_ms, "brief_bytes": b.markdown.len(), "brief_invariants": b.included.iter().filter(|i| i.kind == "invariant").count(),
            "brief_omitted": b.omitted.len(), "views": views.len(), "db_bytes": db, "rss_kib": rss_kib(std::process::id())
        }),
    );
    let _ = std::fs::remove_dir_all(root);
}

/// `streams` live runs, each producing `rate` agent updates per second for
/// 10 simulated seconds, recorded the way the worker records them: batched
/// per run every 250 ms. Measures how many write transactions per second a
/// single SQLite file sustains for this shape, and the bytes it costs.
fn ingest(streams: usize, rate: usize) {
    let root = temp_repo("ingest");
    let ws = Workspace::discover(&root).expect("ws");
    let store = ws.open_store().expect("store");
    for i in 0..streams {
        store
            .insert_run(&NewRun {
                id: &format!("r{i}"),
                task: "t",
                agent: "a",
                base: "b",
                branch: "b",
                worktree: "/x",
                owner: "p",
                from_run: None,
                note: None,
            })
            .expect("run");
    }
    let payload = "x".repeat(256);
    let secs = 10;
    let ticks = secs * 4; // 250 ms flush interval
    let per_tick = (rate as f64 / 4.0).ceil() as usize;
    let before = std::fs::metadata(ws.state.join("state.db-wal"))
        .map(|m| m.len())
        .unwrap_or(0);
    let t = Instant::now();
    let mut txs = 0usize;
    let mut events = 0usize;
    for _ in 0..ticks {
        for i in 0..streams {
            let batch: Vec<(String, serde_json::Value)> = (0..per_tick)
                .map(|_| {
                    (
                        "agent.tool".to_string(),
                        json!({ "id": "t", "title": payload, "status": "in_progress" }),
                    )
                })
                .collect();
            events += batch.len();
            store.append_batch(&format!("r{i}"), &batch).expect("batch");
            txs += 1;
        }
    }
    let wall = t.elapsed().as_secs_f64();
    let wal = std::fs::metadata(ws.state.join("state.db-wal"))
        .map(|m| m.len())
        .unwrap_or(0);
    let db = std::fs::metadata(ws.state.join("state.db"))
        .map(|m| m.len())
        .unwrap_or(0);
    report(
        "ingest",
        json!({
            "streams": streams, "updates_per_stream_per_s": rate, "simulated_s": secs,
            "transactions": txs, "events": events, "wall_s": wall,
            "tx_per_s": txs as f64 / wall, "needed_tx_per_s": streams * 4,
            "realtime_factor": secs as f64 / wall,
            "db_bytes": db, "wal_bytes_growth": wal.saturating_sub(before), "bytes_per_event": (db + wal) as f64 / events as f64
        }),
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Real processes: `n` concurrent `kitsu run` workers, each driving a real
/// `kitsu-test-agent` that streams a message and sleeps. Measures what one
/// supervised run costs in memory and how long startup takes, excluding any
/// model (the agent is a stub, which is the point: this is Kitsu's share).
fn procs(n: usize) {
    let root = temp_repo("procs");
    let kitsu = sibling("kitsu");
    std::fs::write(root.join(".kitsu/kitsu.toml"), "").expect("cfg");
    for i in 0..n {
        std::fs::write(
            root.join(format!(".kitsu/tasks/t{i}.md")),
            format!("+++\ntitle = \"t{i}\"\n+++\n"),
        )
        .expect("task");
    }
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(&root)
        .status();
    let _ = Command::new("git")
        .args(["commit", "-qm", "init"])
        .current_dir(&root)
        .status();
    let cfg = root.join("cfg");
    std::fs::create_dir_all(&cfg).expect("cfg");
    let script = root.join("agent.toml");
    std::fs::write(&script, "[[steps]]\nsay = \"working on it, streaming a little text so the recorder has something to do\"\n[[steps]]\nsleep_ms = 45000\n").expect("script");
    let status = Command::new(&kitsu)
        .args(["trust"])
        .current_dir(&root)
        .env("KITSU_CONFIG_DIR", &cfg)
        .status()
        .expect("trust");
    assert!(status.success());

    let t = Instant::now();
    let mut children = Vec::new();
    for i in 0..n {
        let c = Command::new(&kitsu)
            .args([
                "run",
                &format!("t{i}"),
                "--agent",
                "test",
                "--id",
                &format!("r{i}"),
                "-q",
                "--no-verify",
            ])
            .current_dir(&root)
            .env("KITSU_CONFIG_DIR", &cfg)
            .env("KITSU_TEST_SCRIPT", &script)
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(root.join(format!("r{i}.err"))).expect("err file"))
            .spawn()
            .expect("spawn");
        children.push(c);
    }
    let ws = Workspace::discover(&root).expect("ws");
    let store = Store::open(&ws.state.join("state.db")).expect("store");
    let mut all_running_s = None;
    let (mut peak, mut workers, mut agents) = (0usize, 0u64, 0u64);
    while t.elapsed().as_secs() < 90 {
        let live = store.live_runs().expect("live");
        let running: Vec<_> = live
            .iter()
            .filter(|r| r.state == kitsu::run::RunState::Running)
            .collect();
        if running.len() > peak {
            peak = running.len();
            workers = children.iter().map(|c| rss_kib(c.id())).sum();
            agents = running.iter().filter_map(|r| r.pid).map(rss_kib).sum();
        }
        if running.len() == n && all_running_s.is_none() {
            all_running_s = Some(t.elapsed().as_secs_f64());
        }
        if live.is_empty() && t.elapsed().as_secs() > 5 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut failed = 0;
    let mut stderr_samples = Vec::new();
    for (i, mut c) in children.into_iter().enumerate() {
        if !c.wait().map(|s| s.success()).unwrap_or(false) {
            failed += 1;
            if stderr_samples.len() < 3 {
                stderr_samples.push(
                    std::fs::read_to_string(root.join(format!("r{i}.err"))).unwrap_or_default(),
                );
            }
        }
    }
    let wall = t.elapsed().as_secs_f64();
    let failures: Vec<String> = store
        .recent_runs(10 * n)
        .expect("runs")
        .into_iter()
        .filter(|r| r.state != kitsu::run::RunState::Finished)
        .take(3)
        .map(|r| {
            let err =
                std::fs::read_to_string(root.join(format!("{}.err", r.id))).unwrap_or_default();
            format!(
                "{} {}: {} | stderr: {}",
                r.id,
                r.state.as_str(),
                r.detail.unwrap_or_default(),
                err.trim()
            )
        })
        .collect();
    let missing = n - store.recent_runs(10 * n).expect("runs").len();
    let worktree_bytes = dir_size(&ws.state.join("worktrees"));
    report(
        "procs",
        json!({
            "runs": n, "all_running_after_s": all_running_s, "total_wall_s": wall, "failed": failed,
            "sample_failures": failures, "never_recorded": missing, "stderr_of_failed": stderr_samples, "peak_running": peak,
            "worker_rss_kib_at_peak": workers, "worker_rss_kib_each": workers / peak.max(1) as u64,
            "stub_agent_rss_kib_at_peak": agents, "worktrees_bytes": worktree_bytes,
            "cpus": std::thread::available_parallelism().map(|p| p.get()).unwrap_or(0)
        }),
    );
    let _ = std::fs::remove_dir_all(root);
}

fn dir_size(p: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(p) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

fn sibling(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("exe");
    let dir = exe.parent().and_then(Path::parent).expect("target dir");
    dir.join(name)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize, d: usize| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    match args.first().map(String::as_str) {
        Some("state") => state(num(1, 1001)),
        Some("ingest") => ingest(num(1, 1001), num(2, 10)),
        Some("procs") => procs(num(1, 100)),
        _ => eprintln!("usage: scale state [N] | ingest [STREAMS] [RATE] | procs [N]"),
    }
}
