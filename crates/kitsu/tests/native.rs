//! Kitsu's own agent loop, end to end: `kitsu run --agent kitsu` against
//! the scripted model server (fixtures/stub-model/server.py, OpenAI mode),
//! on a copy of fixtures/retry-storm. No model, no key, no network.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use kitsu::run::RunState;
use kitsu::store::Store;
use kitsu::workspace::Workspace;
use serde_json::{Value, json};

const KITSU: &str = env!("CARGO_BIN_EXE_kitsu");
/// Stands in for an API key. It must never be written anywhere.
const KEY: &str = "sk-SENTINEL-4f1d0c";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for e in std::fs::read_dir(from).expect("read_dir") {
        let e = e.expect("entry");
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_name() == "__pycache__" {
            continue;
        }
        if e.file_type().expect("type").is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).expect("copy");
        }
    }
}

struct Env {
    root: PathBuf,
    repo: PathBuf,
    cfg: PathBuf,
    stub: Child,
    log: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self.stub.kill();
        let _ = self.stub.wait();
        if !std::thread::panicking() && std::env::var_os("KITSU_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

impl Env {
    /// A repo, a config dir whose agents.toml points `kitsu` at a stub model
    /// that plays `script`.
    fn new(name: &str, script: &Value, native_extra: &str) -> Env {
        let root = std::env::temp_dir().join(format!("kitsu-native-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        let cfg = root.join("cfg");
        copy_dir(&fixtures().join("retry-storm/repo"), &repo);
        std::fs::create_dir_all(&cfg).expect("cfg");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Dev"]);
        git(&repo, &["config", "user.email", "dev@example.com"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);

        let script_path = root.join("script.json");
        std::fs::write(&script_path, script.to_string()).expect("script");
        let log = root.join("requests.jsonl");
        let mut stub = Command::new("python3")
            .arg(fixtures().join("stub-model/server.py"))
            .args(["--port", "0", "--log"])
            .arg(&log)
            .arg("--script")
            .arg(&script_path)
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .expect("stub model");
        let mut line = String::new();
        BufReader::new(stub.stderr.take().expect("stderr"))
            .read_line(&mut line)
            .expect("stub port");
        let port: u16 = line
            .trim()
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .expect("port line");
        std::fs::write(
            cfg.join("agents.toml"),
            format!(
                "[limits]\nnice = 0\ncpus = 0\n\n[agents.kitsu]\nnative = {{ base_url = \"http://127.0.0.1:{port}/v1\", model = \"stub\", api_key_env = \"KITSU_TEST_KEY\"{native_extra} }}\n"
            ),
        )
        .expect("agents.toml");
        let env = Env {
            root,
            repo,
            cfg,
            stub,
            log,
        };
        env.ok(&["trust"]);
        env
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(KITSU);
        c.args(args)
            .current_dir(&self.repo)
            .env("KITSU_CONFIG_DIR", &self.cfg)
            .env("KITSU_TEST_KEY", KEY)
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .env("KITSU_BACKOFF_MS", "10")
            .env("NO_COLOR", "1");
        c
    }

    fn ok(&self, args: &[&str]) -> String {
        let o = self.cmd(args).output().expect("kitsu");
        assert!(
            o.status.success(),
            "kitsu {args:?}:\n{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    fn run(&self, id: &str, extra: &[&str]) -> Output {
        let mut args = vec![
            "run",
            "bounded-retries",
            "--agent",
            "kitsu",
            "--id",
            id,
            "-q",
        ];
        args.extend_from_slice(extra);
        self.cmd(&args).output().expect("run")
    }

    fn store(&self) -> Store {
        Workspace::discover(&self.repo)
            .expect("ws")
            .open_store()
            .expect("store")
    }

    fn events(&self, id: &str) -> Vec<(String, Value)> {
        self.store()
            .run_events(id, 0, 10_000)
            .expect("events")
            .into_iter()
            .map(|e| (e.kind, e.body))
            .collect()
    }

    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).expect("log line"))
            .collect()
    }

    /// Every byte Kitsu wrote for this repo: state.db, blobs, run dirs.
    fn assert_key_never_written(&self) {
        fn walk(dir: &Path, hits: &mut Vec<PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, hits);
                } else if std::fs::read(&p)
                    .is_ok_and(|b| b.windows(KEY.len()).any(|w| w == KEY.as_bytes()))
                {
                    hits.push(p);
                }
            }
        }
        let mut hits = Vec::new();
        walk(&self.repo.join(".git/kitsu"), &mut hits);
        walk(&self.cfg, &mut hits);
        hits.retain(|p| !p.ends_with("agents.toml"));
        assert!(hits.is_empty(), "the key was written to {hits:?}");
    }
}

/// The correct fix, from the scripted ACP agent's fixture.
fn good_payments() -> String {
    let t: toml::Table = toml::from_str(
        &std::fs::read_to_string(fixtures().join("retry-storm/agents/good.toml"))
            .expect("good.toml"),
    )
    .expect("toml");
    t["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find_map(|s| {
            s.get("write")
                .and_then(|w| w.get("content"))
                .and_then(|c| c.as_str())
        })
        .expect("write step")
        .to_string()
}

/// Bounded retries, but a fresh key per attempt: fails idempotency.
fn naive_payments() -> String {
    good_payments().replace(
        "    key = str(uuid.uuid4())\n    for attempt in range(MAX_ATTEMPTS):\n",
        "    for attempt in range(MAX_ATTEMPTS):\n        key = str(uuid.uuid4())\n",
    )
}

fn stop_reason(env: &Env, id: &str) -> (RunState, Option<String>) {
    let r = env.store().run(id).expect("run");
    (r.state, r.stop_reason)
}

#[test]
fn the_loop_fixes_the_task_and_the_checks_not_the_model_say_done() {
    let script = json!([
        { "tool": "read_file", "args": { "path": "payments.py" }, "expect": "Done means" },
        { "tool": "write_file", "args": { "path": "payments.py", "content": good_payments() } },
        { "tool": "run_check", "args": { "name": "retries" } },
        { "tool": "run_check", "args": { "name": "idempotency" }, "expect": "Check `retries`: pass" },
        { "tool": "finish", "args": { "outcome": "done", "summary": "Bounded to 3, one key per charge." }, "expect": "idempotency" },
    ]);
    let env = Env::new("happy", &script, "");
    let o = env.run("rn1", &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        stop_reason(&env, "rn1"),
        (RunState::Finished, Some("verified".into()))
    );

    // Every effect was journaled before and after.
    let ev = env.events("rn1");
    let begins = ev.iter().filter(|(k, _)| k == "tool.begin").count();
    let ends: Vec<&Value> = ev
        .iter()
        .filter(|(k, _)| k == "tool.end")
        .map(|(_, b)| b)
        .collect();
    assert_eq!((begins, ends.len()), (5, 5));
    assert!(
        ends.iter()
            .all(|b| b["outcome"] == "done" && b["is_error"] == false),
        "{ends:#?}"
    );
    assert!(ev.iter().any(|(k, b)| k == "tool.begin"
        && b["tool"] == "write_file"
        && b["pre"]["sha_after"].is_string()));
    // The UI's projections exist, same shapes as the ACP path.
    assert!(ev.iter().any(|(k, b)| k == "agent.tool"
        && b["title"] == "Write payments.py"
        && b["status"] == "completed"));

    // What the model saw: the harness, then the brief, then the ledger last.
    let reqs = env.requests();
    assert_eq!(reqs.len(), 5);
    let first = &reqs[0]["body"]["messages"];
    assert!(
        first[0]["content"]
            .as_str()
            .expect("harness")
            .starts_with("You are Kitsu's coding agent")
    );
    assert!(
        first[1]["content"]
            .as_str()
            .expect("brief")
            .contains("## Done means")
    );
    let last_msg = |r: &Value| {
        r["body"]["messages"]
            .as_array()
            .and_then(|m| m.last())
            .and_then(|m| m["content"].as_str())
            .unwrap_or("")
            .to_string()
    };
    assert!(
        last_msg(&reqs[0]).contains("`idempotency`") && last_msg(&reqs[0]).contains("unverified"),
        "{}",
        last_msg(&reqs[0])
    );
    assert!(
        last_msg(&reqs[4]).contains("`idempotency`")
            && last_msg(&reqs[4]).contains("pass on your current files"),
        "{}",
        last_msg(&reqs[4])
    );
    // The prefix is byte-identical on every request (prompt cache).
    for r in &reqs {
        assert_eq!(r["body"]["messages"][0], first[0]);
        assert_eq!(r["body"]["messages"][1], first[1]);
    }

    // After the loop, the ordinary finish: snapshot and checks on it.
    let status = env.ok(&["status"]);
    assert!(status.contains("ready for review, checks pass"), "{status}");
    env.assert_key_never_written();
}

#[test]
fn a_false_done_is_refused_three_times_then_the_run_stops_unverified() {
    let script = json!([
        { "tool": "write_file", "args": { "path": "payments.py", "content": naive_payments() } },
        { "tool": "finish", "args": { "outcome": "done", "summary": "Done." } },
        { "text": "I'm confident it's done.", "expect": "Not done: 1 of" },
        { "tool": "finish", "args": { "outcome": "done", "summary": "Really done." } },
        { "text": "unreachable" },
    ]);
    let env = Env::new("falsedone", &script, "");
    let o = env.run("rn2", &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        stop_reason(&env, "rn2"),
        (RunState::Finished, Some("unverified".into()))
    );
    let reqs = env.requests();
    assert_eq!(reqs.len(), 4, "no request after the third refusal");
    // The text-only reply counted as a finish, and its refusal reached the
    // model as a message after that reply.
    let msgs = reqs[3]["body"]["messages"].as_array().expect("messages");
    let at = msgs
        .iter()
        .position(|m| m["role"] == "assistant" && m["content"] == "I'm confident it's done.")
        .expect("the text-only reply");
    assert_eq!(msgs[at + 1]["role"], "user", "{msgs:#?}");
    assert!(
        msgs[at + 1]["content"]
            .as_str()
            .expect("notice")
            .starts_with("Not done: 1 of"),
        "{msgs:#?}"
    );
    assert!(
        env.events("rn2")
            .iter()
            .all(|(k, b)| !(k == "run.state" && b["stop_reason"] == "verified"))
    );
    let status = env.ok(&["status"]);
    assert!(status.contains("failing: idempotency"), "{status}");
}

#[test]
fn a_missing_key_fails_the_start_and_says_which_variable() {
    let env = Env::new("nokey", &json!([]), "");
    let o = env
        .cmd(&[
            "run",
            "bounded-retries",
            "--agent",
            "kitsu",
            "--id",
            "rn3",
            "-q",
        ])
        .env_remove("KITSU_TEST_KEY")
        .output()
        .expect("run");
    let _ = o;
    let r = env.store().run("rn3").expect("run");
    assert_eq!(r.state, RunState::Failed);
    assert!(
        r.detail
            .as_deref()
            .unwrap_or("")
            .contains("$KITSU_TEST_KEY is not set"),
        "{:?}",
        r.detail
    );
    assert!(env.requests().is_empty(), "no request without a key");
}

#[test]
fn a_plain_http_endpoint_elsewhere_is_refused_before_any_run() {
    let env = Env::new("plainhttp", &json!([]), "");
    std::fs::write(
        env.cfg.join("agents.toml"),
        "[agents.kitsu]\nnative = { base_url = \"http://models.example.com/v1\", model = \"m\", api_key_env = \"KITSU_TEST_KEY\" }\n",
    )
    .expect("agents.toml");
    let o = env.cmd(&["agents"]).output().expect("agents");
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("must be https"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn compaction_keeps_the_brief_and_the_newest_step_and_one_overflow_is_survived() {
    let big: String = (1..=60)
        .map(|i| format!("{i:03} {}\n", "x".repeat(95)))
        .collect();
    let script = json!([
        { "tool": "write_file", "args": { "path": "big.txt", "content": big } },
        { "tool": "read_file", "args": { "path": "big.txt" } },
        { "tool": "read_file", "args": { "path": "big.txt", "start_line": 2 } },
        // Over 80% of the window: old outputs became pointers first.
        { "tool": "write_file", "args": { "path": "payments.py", "content": good_payments() }, "expect": "elided at compaction 1" },
        { "overflow": true },
        // The provider said it didn't fit: compacted harder, retried once.
        { "tool": "finish", "args": { "outcome": "done", "summary": "Bounded, one key." }, "expect": "compaction 2" },
    ]);
    let env = Env::new(
        "compact",
        &script,
        ", context_window = 13000, max_output = 1000",
    );
    let o = env.run("rn4", &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        stop_reason(&env, "rn4"),
        (RunState::Finished, Some("verified".into()))
    );

    let ev = env.events("rn4");
    let compactions: Vec<&Value> = ev
        .iter()
        .filter(|(k, _)| k == "ctx.compacted")
        .map(|(_, b)| b)
        .collect();
    assert_eq!(compactions.len(), 2, "{compactions:#?}");
    assert_eq!(
        (
            compactions[0]["trigger"].as_str(),
            compactions[1]["trigger"].as_str()
        ),
        (Some("threshold"), Some("overflow"))
    );
    assert!(
        compactions
            .iter()
            .all(|c| c["after"].as_u64() < c["before"].as_u64())
    );

    let reqs = env.requests();
    assert_eq!(reqs.len(), 6);
    let first = &reqs[0]["body"]["messages"];
    for r in &reqs {
        let m = &r["body"]["messages"];
        assert_eq!(
            (&m[0], &m[1]),
            (&first[0], &first[1]),
            "the harness and the brief never change"
        );
        let last = m.as_array().and_then(|a| a.last()).expect("last");
        assert!(
            last["content"]
                .as_str()
                .is_some_and(|t| t.starts_with("# Kitsu: state of your run")),
            "the ledger is last"
        );
    }
    // Right after the first compaction the newest step is still verbatim.
    let m = reqs[3]["body"]["messages"].as_array().expect("messages");
    let tool_results: Vec<&str> = m
        .iter()
        .filter(|x| x["role"] == "tool")
        .filter_map(|x| x["content"].as_str())
        .collect();
    assert!(
        tool_results
            .last()
            .is_some_and(|t| t.starts_with("2\t002 ")),
        "{tool_results:?}"
    );
    assert!(
        tool_results
            .first()
            .is_some_and(|t| t.starts_with("[kitsu: output of write_file big.txt elided")),
        "{tool_results:?}"
    );
    // Every tool call in every request is answered: no step was split.
    for r in &reqs {
        let m = r["body"]["messages"].as_array().expect("messages");
        let asked: usize = m
            .iter()
            .filter_map(|x| x["tool_calls"].as_array())
            .map(|c| c.len())
            .sum();
        let answered = m.iter().filter(|x| x["role"] == "tool").count();
        assert_eq!(asked, answered);
    }
}

#[test]
fn two_overflows_in_a_row_stop_the_run_instead_of_looping() {
    let big: String = (1..=60)
        .map(|i| format!("{i:03} {}\n", "x".repeat(95)))
        .collect();
    let env = Env::new(
        "overflow2",
        &json!([
            { "tool": "write_file", "args": { "path": "big.txt", "content": big } },
            { "tool": "read_file", "args": { "path": "big.txt" } },
            { "tool": "read_file", "args": { "path": "big.txt", "start_line": 2 } },
            { "overflow": true },
            { "overflow": true },
            { "text": "unreachable" },
        ]),
        "",
    );
    let o = env.run("rn5", &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        stop_reason(&env, "rn5"),
        (RunState::Finished, Some("context_overflow".into()))
    );
    assert_eq!(
        env.requests().len(),
        5,
        "compacted once, retried once, then stopped"
    );
    let compactions = env
        .events("rn5")
        .iter()
        .filter(|(k, _)| k == "ctx.compacted")
        .count();
    assert_eq!(compactions, 1);

    // Nothing to shrink: the same request would overflow again, so it isn't sent.
    let env = Env::new(
        "overflow0",
        &json!([{ "overflow": true }, { "text": "unreachable" }]),
        "",
    );
    assert!(env.run("rn6", &[]).status.success());
    assert_eq!(
        stop_reason(&env, "rn6"),
        (RunState::Finished, Some("context_overflow".into()))
    );
    assert_eq!(env.requests().len(), 1);
}

/// Crash run `a` at `fault`, recover, resume it as `b`. Returns b's worktree.
fn crash_and_resume(env: &Env, fault: &str, extra: &[&str]) -> PathBuf {
    let mut args = vec![
        "run",
        "bounded-retries",
        "--agent",
        "kitsu",
        "--id",
        "ra",
        "-q",
    ];
    args.extend_from_slice(extra);
    let o = env
        .cmd(&args)
        .env("KITSU_FAULT", fault)
        .output()
        .expect("run a");
    assert!(!o.status.success(), "the fault should have killed the run");
    env.ok(&["recover"]);
    assert_eq!(
        env.store().run("ra").expect("ra").state,
        RunState::Interrupted
    );
    let mut args = vec![
        "run",
        "bounded-retries",
        "--agent",
        "kitsu",
        "--id",
        "rb",
        "--from",
        "ra",
        "--resume",
        "-q",
    ];
    args.extend_from_slice(extra);
    let o = env.cmd(&args).output().expect("run b");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    env.repo.join(".git/kitsu/worktrees/rb")
}

fn begins(env: &Env, call: &str) -> usize {
    ["ra", "rb"]
        .iter()
        .flat_map(|r| env.events(r))
        .filter(|(k, b)| k == "tool.begin" && b["call"] == call)
        .count()
}

#[test]
fn an_edit_that_landed_before_the_crash_is_settled_as_applied_not_repeated() {
    let old = "\"\"\"Charges a card through an upstream payment API.\"\"\"";
    let script = json!([
        { "tool": "edit_file", "args": { "path": "payments.py", "old": old, "new": format!("{old}\n# marker") } },
        { "tool": "finish", "args": { "outcome": "blocked", "summary": "stopping" }, "expect": "the change was applied. It was not repeated." },
    ]);
    let env = Env::new("crash-edit", &script, "");
    let wt = crash_and_resume(&env, "after_effect:edit_file", &[]);
    let text = std::fs::read_to_string(wt.join("payments.py")).expect("payments.py");
    assert_eq!(text.matches("# marker").count(), 1, "applied exactly once");
    assert_eq!(begins(&env, "ra/c1"), 1, "the effect never started twice");
    let end = env
        .events("rb")
        .into_iter()
        .find(|(k, b)| k == "tool.end" && b["call"] == "ra/c1")
        .expect("settled");
    assert_eq!(
        (end.1["outcome"].as_str(), end.1["reconciled"].as_bool()),
        (Some("applied"), Some(true))
    );
    assert_eq!(
        stop_reason(&env, "rb"),
        (RunState::Finished, Some("blocked".into()))
    );
}

#[test]
fn a_write_that_never_happened_is_settled_as_not_applied() {
    let script = json!([
        { "tool": "write_file", "args": { "path": "payments.py", "content": "broken\n" } },
        { "tool": "finish", "args": { "outcome": "blocked", "summary": "stopping" }, "expect": "the change was not applied" },
    ]);
    let env = Env::new("crash-write", &script, "");
    let wt = crash_and_resume(&env, "before_effect:write_file", &[]);
    let text = std::fs::read_to_string(wt.join("payments.py")).expect("payments.py");
    assert!(
        text.starts_with("\"\"\"Charges a card"),
        "unchanged: {text}"
    );
    let end = env
        .events("rb")
        .into_iter()
        .find(|(k, b)| k == "tool.end" && b["call"] == "ra/c1")
        .expect("settled");
    assert_eq!(end.1["outcome"], "not_applied");
}

#[test]
fn a_command_cut_off_by_a_crash_is_unknown_and_never_run_again() {
    let script = json!([
        { "tool": "shell", "args": { "command": "echo x >> log.txt" } },
        { "tool": "finish", "args": { "outcome": "blocked", "summary": "stopping" }, "expect": "it was not run again. Files changed since it started: log.txt." },
    ]);
    let env = Env::new("crash-shell", &script, "");
    let wt = crash_and_resume(&env, "after_effect:shell", &["--policy", "auto"]);
    let log = std::fs::read_to_string(wt.join("log.txt")).expect("log.txt");
    assert_eq!(log, "x\n", "ran once");
    let end = env
        .events("rb")
        .into_iter()
        .find(|(k, b)| k == "tool.end" && b["call"] == "ra/c1")
        .expect("settled");
    assert_eq!(end.1["outcome"], "unknown");
}

#[test]
fn resume_is_only_for_kitsus_own_loop() {
    let env = Env::new("resume-acp", &json!([]), "");
    std::fs::write(env.root.join("s.toml"), "[[steps]]\nsay = \"hi\"\n").expect("script");
    let o = env
        .cmd(&[
            "run",
            "bounded-retries",
            "--agent",
            "test",
            "--id",
            "rt",
            "-q",
            "--no-verify",
        ])
        .env("KITSU_TEST_SCRIPT", env.root.join("s.toml"))
        .output()
        .expect("run");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = env
        .cmd(&[
            "run",
            "bounded-retries",
            "--agent",
            "test",
            "--from",
            "rt",
            "--resume",
            "-q",
        ])
        .output()
        .expect("resume");
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("--resume continues Kitsu's own agent loop"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn going_in_circles_is_warned_then_stopped() {
    let read = json!({ "tool": "read_file", "args": { "path": "payments.py" } });
    let grep = json!({ "tool": "grep", "args": { "pattern": "uuid" } });
    let script = json!([read, read, read, { "tool": "grep", "args": { "pattern": "uuid" }, "expect": "third identical call" }, grep, grep, { "text": "unreachable" }]);
    let env = Env::new("loop", &script, "");
    assert!(env.run("rn7", &[]).status.success());
    assert_eq!(
        stop_reason(&env, "rn7"),
        (RunState::Finished, Some("doom_loop".into()))
    );
    let signals: Vec<Value> = env
        .events("rn7")
        .into_iter()
        .filter(|(k, _)| k == "loop.signal")
        .map(|(_, b)| b)
        .collect();
    assert_eq!(
        signals.iter().map(|s| s["n"].as_u64()).collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
    assert_eq!(env.requests().len(), 6);
}

#[test]
fn the_same_check_after_each_edit_is_progress_not_a_loop() {
    let old = "\"\"\"Charges a card through an upstream payment API.\"\"\"";
    let check = json!({ "tool": "run_check", "args": { "name": "retries" } });
    let edit = |n: u32| json!({ "tool": "edit_file", "args": { "path": "payments.py", "old": old, "new": format!("{old}\n# try {n}") } });
    let script = json!([check, edit(1), check, edit(2), check, { "tool": "finish", "args": { "outcome": "blocked", "summary": "enough" } }]);
    let env = Env::new("progress", &script, "");
    assert!(env.run("rn8", &[]).status.success());
    assert_eq!(
        stop_reason(&env, "rn8"),
        (RunState::Finished, Some("blocked".into()))
    );
    assert!(env.events("rn8").iter().all(|(k, _)| k != "loop.signal"));
}

#[test]
fn rate_limits_and_server_errors_are_retried_a_bounded_number_of_times() {
    let script = json!([
        { "status": 429, "retry_after": 0 },
        { "status": 503 },
        { "tool": "finish", "args": { "outcome": "blocked", "summary": "ok" } },
    ]);
    let env = Env::new("retry", &script, "");
    assert!(env.run("rn9", &[]).status.success());
    assert_eq!(
        stop_reason(&env, "rn9"),
        (RunState::Finished, Some("blocked".into()))
    );
    let errors: Vec<Value> = env
        .events("rn9")
        .into_iter()
        .filter(|(k, _)| k == "model.error")
        .map(|(_, b)| b)
        .collect();
    assert_eq!(
        errors
            .iter()
            .map(|e| e["class"].as_str())
            .collect::<Vec<_>>(),
        [Some("rate_limited"), Some("transient")]
    );

    let env = Env::new(
        "retry5",
        &json!([{ "status": 500 }, { "status": 500 }, { "status": 500 }, { "status": 500 }, { "status": 500 }, { "text": "unreachable" }]),
        "",
    );
    assert!(
        !env.run("rn10", &[]).status.success(),
        "a failed run exits non-zero"
    );
    let r = env.store().run("rn10").expect("run");
    assert_eq!(
        r.state,
        RunState::Failed,
        "five failures in a row end the run"
    );
    assert!(
        r.detail.as_deref().unwrap_or("").contains("HTTP 500"),
        "{:?}",
        r.detail
    );
    assert_eq!(env.requests().len(), 5);

    let env = Env::new(
        "fatal",
        &json!([{ "status": 401 }, { "text": "unreachable" }]),
        "",
    );
    assert!(!env.run("rn11", &[]).status.success());
    assert_eq!(
        env.store().run("rn11").expect("run").state,
        RunState::Failed
    );
    assert_eq!(env.requests().len(), 1, "a bad key isn't retried");
}

#[test]
fn stop_kills_a_long_command_and_ends_the_run_cancelled() {
    let script = json!([{ "tool": "shell", "args": { "command": "sleep 30; echo late > late.txt" } }, { "text": "unreachable" }]);
    let env = Env::new("cancel", &script, "");
    let mut child = env
        .cmd(&[
            "run",
            "bounded-retries",
            "--agent",
            "kitsu",
            "--id",
            "rn12",
            "-q",
            "--policy",
            "auto",
        ])
        .spawn()
        .expect("spawn");
    let t0 = std::time::Instant::now();
    while !env
        .events("rn12")
        .iter()
        .any(|(k, b)| k == "tool.begin" && b["tool"] == "shell")
    {
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(20),
            "the command never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    env.ok(&["stop", "rn12"]);
    let status = child.wait().expect("wait");
    assert!(status.success());
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(15),
        "stopped promptly"
    );
    assert_eq!(
        stop_reason(&env, "rn12"),
        (RunState::Finished, Some("cancelled".into()))
    );
    let end = env
        .events("rn12")
        .into_iter()
        .find(|(k, b)| k == "tool.end" && b["tool"] == "shell")
        .expect("end");
    assert_eq!(end.1["outcome"], "cancelled");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!env.repo.join(".git/kitsu/worktrees/rn12/late.txt").exists());
}

#[test]
fn paths_outside_the_worktree_are_refused_symlinks_included() {
    let script = json!([
        { "tool": "write_file", "args": { "path": "../escape.txt", "content": "x" } },
        { "tool": "write_file", "args": { "path": ".git/config", "content": "x" } },
        { "tool": "shell", "args": { "command": "ln -s /tmp out" } },
        { "tool": "write_file", "args": { "path": "out/kitsu-escape.txt", "content": "x" } },
        { "tool": "finish", "args": { "outcome": "blocked", "summary": "done probing" }, "expect": "resolves outside your worktree" },
    ]);
    let env = Env::new("confine", &script, "");
    assert!(env.run("rn13", &["--policy", "auto"]).status.success());
    let ends: Vec<Value> = env
        .events("rn13")
        .into_iter()
        .filter(|(k, _)| k == "tool.end")
        .map(|(_, b)| b)
        .collect();
    assert_eq!(ends[0]["outcome"], "invalid");
    assert_eq!(ends[1]["outcome"], "invalid");
    assert_eq!(ends[3]["outcome"], "invalid");
    assert!(!env.repo.join(".git/kitsu/worktrees/escape.txt").exists());
    assert!(!Path::new("/tmp/kitsu-escape.txt").exists());
}

type Heard = std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>;

/// A stand-in for TypeSafe System One: one recorded response body (in the
/// schema of typesafe-sdk's `_schemas/models.py`) per request, in order.
/// Keeps each request's head and JSON body.
fn fake_judge(bodies: Vec<String>) -> (String, Heard) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let heard: Heard = Default::default();
    let log = heard.clone();
    std::thread::spawn(move || {
        for body in bodies {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let mut r = BufReader::new(s.try_clone().expect("clone"));
            let (mut head, mut len) = (String::new(), 0usize);
            loop {
                let mut l = String::new();
                if r.read_line(&mut l).unwrap_or(0) == 0 || l.trim().is_empty() {
                    break;
                }
                if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                head.push_str(&l);
            }
            let mut b = vec![0u8; len];
            let _ = r.read_exact(&mut b);
            log.lock()
                .expect("lock")
                .push((head, serde_json::from_slice(&b).unwrap_or(Value::Null)));
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    (url, heard)
}

fn low_risk(p: f64) -> String {
    format!(
        r#"{{"model":"jev-latest","answers":{{"low_risk":{{"type":"noul","noul":{p}}}}},"usage":{{"input_tokens":230,"output_tokens":1}}}}"#
    )
}

#[test]
fn triage_runs_what_the_judge_calls_low_risk_and_asks_you_the_rest_with_its_probability() {
    let script = json!([
        { "tool": "shell", "args": { "command": "echo judged > judged.txt" } },
        { "tool": "shell", "args": { "command": "echo risky > risky.txt" } },
        { "tool": "shell", "args": { "command": "curl -s https://example.com > page.html" } },
        { "tool": "finish", "args": { "outcome": "blocked", "summary": "probing" } },
    ]);
    let env = Env::new("triage", &script, "");
    let (url, heard) = fake_judge(vec![low_risk(0.97), low_risk(0.35)]);
    let toml = env.cfg.join("agents.toml");
    let mut cfg = std::fs::read_to_string(&toml).expect("agents.toml");
    cfg.push_str(&format!(
        "\n[judge]\nbase_url = \"{url}\"\napi_key_env = \"KITSU_TEST_KEY\"\n"
    ));
    std::fs::write(&toml, cfg).expect("write");

    let mut child = env
        .cmd(&[
            "run",
            "bounded-retries",
            "--agent",
            "kitsu",
            "--id",
            "rn20",
            "-q",
            "--policy",
            "triage",
        ])
        .spawn()
        .expect("spawn");
    let answer = |want: f64| -> Value {
        let t0 = std::time::Instant::now();
        loop {
            if let Some(a) = env.store().open_asks().expect("asks").into_iter().next() {
                assert_eq!(a.request["judge"]["p_yes"].as_f64(), Some(want));
                env.ok(&["answer", &a.id.to_string(), "reject_once"]);
                return a.request;
            }
            assert!(
                t0.elapsed() < std::time::Duration::from_secs(30),
                "no ask came"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    };
    // The second command: the judge said 35%, below 90%: you see it.
    let asked = answer(0.35);
    let note = asked["judge"]["note"].as_str().expect("note");
    assert!(note.contains("35%") && note.contains("90%"), "{note}");
    // The third: curl is never the judge's to allow, and isn't sent.
    let t0 = std::time::Instant::now();
    let screened = loop {
        if let Some(a) = env.store().open_asks().expect("asks").into_iter().next() {
            break a;
        }
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(30),
            "no second ask"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert_eq!(screened.request["judge"]["screened"], "uses the network");
    let status = env.ok(&["status"]);
    assert!(
        status.contains("Not judged: the command uses the network."),
        "{status}"
    );
    env.ok(&["answer", &screened.id.to_string(), "reject_once"]);
    assert!(child.wait().expect("wait").success());

    let wt = env.repo.join(".git/kitsu/worktrees/rn20");
    assert!(
        wt.join("judged.txt").exists(),
        "the judged command ran without an ask"
    );
    assert!(!wt.join("risky.txt").exists());
    assert!(!wt.join("page.html").exists());

    let ev = env.events("rn20");
    let perms: Vec<&Value> = ev
        .iter()
        .filter(|(k, _)| k == "permission")
        .map(|(_, b)| b)
        .collect();
    assert_eq!(perms.len(), 3, "{perms:#?}");
    assert_eq!(perms[0]["by"], "judge");
    assert_eq!(perms[0]["decision"], "allow_once");
    assert_eq!(perms[0]["p_yes"], 0.97);
    assert_eq!(
        (perms[1]["by"].as_str(), perms[1]["p_yes"].as_f64()),
        (Some("human"), Some(0.35))
    );
    assert_eq!(perms[2]["by"], "human");
    // The auto-allow points at its stored judgment.
    let store = env.store();
    let j = store
        .judgment(perms[0]["judgment"].as_i64().expect("judgment id"))
        .expect("row");
    assert_eq!(
        (j.purpose.as_str(), j.run.as_deref(), j.outcome.as_str()),
        ("permission", Some("rn20"), "answered")
    );
    assert_eq!(j.answers["low_risk"]["p_yes"], 0.97);
    assert_eq!(store.judgments_for_run("rn20").expect("rows").len(), 2);
    let config = ev
        .iter()
        .find(|(k, _)| k == "judge.config")
        .expect("judge.config")
        .1
        .clone();
    assert_eq!(config["on"], true);
    assert_eq!(config["permission_threshold"], 0.9);
    // Review can see why.
    let shown = env.ok(&["show", "rn20"]);
    assert!(shown.contains("(judge, judgment"), "{shown}");

    // What the judge got: the key in its header only, the command as data.
    let heard = heard.lock().expect("lock").clone();
    assert_eq!(heard.len(), 2, "the screened command was never sent");
    assert!(heard[0].0.starts_with("POST /v1/systemone "));
    assert!(
        heard[0]
            .0
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {KEY}").to_ascii_lowercase())
    );
    assert_eq!(
        heard[0].1["state"]["tool_call"]["arguments"]["command"],
        "echo judged > judged.txt"
    );
    assert!(!heard[0].1["questions"].to_string().contains("judged"));
    env.assert_key_never_written();
}

#[test]
fn triage_without_a_judge_asks_you_and_says_why() {
    let script = json!([
        { "tool": "shell", "args": { "command": "echo hi > hi.txt" } },
        { "tool": "finish", "args": { "outcome": "blocked", "summary": "probing" } },
    ]);
    let env = Env::new("triage-off", &script, "");
    let mut child = env
        .cmd(&[
            "run",
            "bounded-retries",
            "--agent",
            "kitsu",
            "--id",
            "rn21",
            "-q",
            "--policy",
            "triage",
        ])
        .spawn()
        .expect("spawn");
    let t0 = std::time::Instant::now();
    let ask = loop {
        if let Some(a) = env.store().open_asks().expect("asks").into_iter().next() {
            break a;
        }
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(30),
            "no ask came"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert_eq!(ask.request["judge"]["unknown"], "unconfigured");
    assert_eq!(ask.request["judge"]["p_yes"], Value::Null);
    env.ok(&["answer", &ask.id.to_string(), "allow_once"]);
    assert!(child.wait().expect("wait").success());
    assert!(env.repo.join(".git/kitsu/worktrees/rn21/hi.txt").exists());
    let ev = env.events("rn21");
    assert!(
        ev.iter()
            .any(|(k, b)| k == "judge.config" && b["on"] == false)
    );
    assert!(
        ev.iter()
            .any(|(k, b)| k == "permission" && b["by"] == "human" && b["judgment"].is_i64())
    );
}
