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
        if !std::thread::panicking() {
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
