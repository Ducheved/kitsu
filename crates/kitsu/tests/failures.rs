//! End-to-end behavior through the real binaries: `kitsu` driving
//! `kitsu-test-agent` over ACP, on a copy of fixtures/retry-storm.
//!
//! Each test sets up its own repo and config dir, so they run in parallel.
//! Most of them are about what happens when something goes wrong, because
//! the happy path is the easy part.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use kitsu::run::RunState;
use kitsu::store::{IntegrationState, Store};
use kitsu::workspace::Workspace;

const KITSU: &str = env!("CARGO_BIN_EXE_kitsu");

struct Env {
    root: PathBuf,
    repo: PathBuf,
    cfg: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/retry-storm")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for e in std::fs::read_dir(from).expect("read fixture") {
        let e = e.expect("entry");
        let dest = to.join(e.file_name());
        if e.file_type().expect("type").is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).expect("copy");
        }
    }
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

impl Env {
    fn new(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("kitsu-it-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        let cfg = root.join("cfg");
        copy_dir(&fixture().join("repo"), &repo);
        std::fs::create_dir_all(&cfg).expect("cfg");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Dev"]);
        git(&repo, &["config", "user.email", "dev@example.com"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        let env = Env { root, repo, cfg };
        env.ok(&["trust"]);
        env
    }

    fn script(&self, name: &str, body: &str) -> PathBuf {
        let p = self.root.join(format!("{name}.toml"));
        std::fs::write(&p, body).expect("script");
        p
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(KITSU);
        c.args(args)
            .current_dir(&self.repo)
            .env("KITSU_CONFIG_DIR", &self.cfg)
            .env("NO_COLOR", "1")
            .env("KITSU_CANCEL_GRACE_MS", "800");
        c
    }

    fn out(&self, args: &[&str]) -> Output {
        self.cmd(args).output().expect("kitsu")
    }

    fn ok(&self, args: &[&str]) -> String {
        let o = self.out(args);
        assert!(
            o.status.success(),
            "kitsu {args:?} failed:\n{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    fn run_with(&self, script: &Path, id: &str, extra: &[&str]) -> Output {
        let mut args = vec![
            "run",
            "bounded-retries",
            "--agent",
            "test",
            "--id",
            id,
            "-q",
        ];
        args.extend_from_slice(extra);
        self.cmd(&args)
            .env("KITSU_TEST_SCRIPT", script)
            .output()
            .expect("run")
    }

    fn spawn_run(&self, script: &Path, id: &str, extra: &[&str]) -> Child {
        let mut args = vec![
            "run",
            "bounded-retries",
            "--agent",
            "test",
            "--id",
            id,
            "-q",
        ];
        args.extend_from_slice(extra);
        self.cmd(&args)
            .env("KITSU_TEST_SCRIPT", script)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn run")
    }

    fn store(&self) -> Store {
        Workspace::discover(&self.repo)
            .expect("ws")
            .open_store()
            .expect("store")
    }

    fn wait_for(&self, what: &str, mut f: impl FnMut(&Store) -> bool) {
        let store = self.store();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !f(&store) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn state(&self, id: &str) -> RunState {
        self.store().run(id).expect("run").state
    }

    fn events(&self, id: &str) -> Vec<String> {
        self.store()
            .run_events(id, 0, 10_000)
            .expect("events")
            .into_iter()
            .map(|e| e.kind)
            .collect()
    }
}

fn agent(name: &str) -> PathBuf {
    fixture().join("agents").join(format!("{name}.toml"))
}

#[test]
fn good_change_is_verified_accepted_and_closes_the_task() {
    let env = Env::new("good");
    assert!(env.run_with(&agent("good"), "rgood", &[]).status.success());
    let status = env.ok(&["status"]);
    assert!(status.contains("ready for review, checks pass"), "{status}");
    let out = env.ok(&["accept", "rgood"]);
    assert!(out.contains("task closed"), "{out}");
    let log = git(&env.repo, &["log", "--format=%an|%s|%b", "-1"]);
    assert!(
        log.starts_with("Dev|Stop retrying forever"),
        "accepted commit is the human's: {log}"
    );
    assert!(log.contains("Kitsu-Run: rgood"));
    assert!(
        git(&env.repo, &["status", "--porcelain"]).trim().is_empty(),
        "accept leaves a clean tree"
    );
    assert!(env.ok(&["status"]).contains("done"));
    // The run's worktree and branch are gone.
    assert!(!env.repo.join(".git/kitsu/worktrees/rgood").exists());
    assert!(
        git(&env.repo, &["branch", "--list", "kitsu/run/*"])
            .trim()
            .is_empty()
    );
}

#[test]
fn the_agents_claim_does_not_count_the_check_does() {
    let env = Env::new("naive");
    let out = env.run_with(&agent("naive"), "rnaive", &[]);
    assert!(out.status.success());
    let status = env.ok(&["status"]);
    assert!(status.contains("failing: idempotency"), "{status}");
    let o = env.out(&["accept", "rnaive"]);
    assert_eq!(
        o.status.code(),
        Some(2),
        "accept must refuse: {}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert_eq!(
        git(&env.repo, &["rev-list", "--count", "HEAD"]).trim(),
        "1",
        "nothing landed"
    );
}

#[test]
fn weakening_a_protected_test_needs_approval_bound_to_that_diff() {
    let env = Env::new("cheat");
    env.run_with(&agent("cheat"), "rcheat", &[]);
    let o = env.out(&["--json", "accept", "rcheat"]);
    assert_eq!(o.status.code(), Some(3));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("json");
    assert_eq!(v["result"], "needs_approval");
    assert_eq!(v["paths"][0], "test_idempotency.py");
    let token = v["token"].as_str().expect("token").to_string();
    let o = env.out(&["accept", "rcheat", "--approve", "0000000000000000"]);
    assert_eq!(o.status.code(), Some(3), "a wrong token is not an approval");
    // Right token: now it's judged by the checks from YOUR checkout, which
    // still include the unweakened test.
    let o = env.out(&["accept", "rcheat", "--approve", &token]);
    assert_eq!(
        o.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
}

#[test]
fn agent_crash_mid_turn_fails_the_run_and_keeps_partial_work() {
    let env = Env::new("crash");
    let s = env.script("crash", "[[steps]]\nwrite = { path = \"notes.txt\", content = \"half done\" }\n[[steps]]\ncrash = true\n");
    let o = env.run_with(&s, "rcrash", &[]);
    assert_eq!(o.status.code(), Some(2));
    let run = env.store().run("rcrash").expect("run");
    assert_eq!(run.state, RunState::Failed);
    assert!(
        run.detail.as_deref().unwrap_or_default().contains("exited"),
        "{:?}",
        run.detail
    );
    let snap = run.snapshot.expect("partial work is snapshotted");
    assert_eq!(
        git(&env.repo, &["show", &format!("{snap}:notes.txt")]),
        "half done"
    );
    assert!(env.ok(&["status"]).contains("failed"));
}

#[test]
fn stop_cancels_a_cooperative_agent() {
    let env = Env::new("stop");
    let s = env.script(
        "slow",
        "[[steps]]\nsay = \"thinking\"\n[[steps]]\nsleep_ms = 20000\n",
    );
    let mut child = env.spawn_run(&s, "rstop", &[]);
    env.wait_for("running", |st| {
        st.run("rstop")
            .map(|r| r.state == RunState::Running)
            .unwrap_or(false)
    });
    let t = Instant::now();
    env.ok(&["stop", "rstop"]);
    let _ = child.wait();
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "cancel took {:?}",
        t.elapsed()
    );
    let run = env.store().run("rstop").expect("run");
    assert_eq!(run.state, RunState::Finished);
    assert_eq!(run.stop_reason.as_deref(), Some("cancelled"));
    assert!(run.cancel_requested);
}

#[test]
fn an_agent_that_ignores_cancel_is_killed_after_the_grace_period() {
    let env = Env::new("ignore");
    let s = env.script("stubborn", "ignore_cancel = true\n[[steps]]\nhang = true\n");
    let mut child = env.spawn_run(&s, "rstub", &[]);
    env.wait_for("running", |st| {
        st.run("rstub")
            .map(|r| r.state == RunState::Running)
            .unwrap_or(false)
    });
    env.ok(&["stop", "rstub"]);
    let _ = child.wait();
    let run = env.store().run("rstub").expect("run");
    assert_eq!(run.state, RunState::Failed);
    assert!(
        run.detail
            .as_deref()
            .unwrap_or_default()
            .contains("did not stop"),
        "{:?}",
        run.detail
    );
}

#[test]
fn a_second_completion_is_recorded_not_believed() {
    let env = Env::new("dup");
    let s = env.script(
        "dup",
        "duplicate_response = true\n[[steps]]\nsay = \"done\"\n",
    );
    env.run_with(&s, "rdup", &[]);
    assert_eq!(env.state("rdup"), RunState::Finished);
    assert!(
        env.events("rdup")
            .iter()
            .any(|k| k == "protocol.duplicate_response"),
        "{:?}",
        env.events("rdup")
    );
}

#[test]
fn garbage_on_the_wire_is_a_protocol_failure() {
    let env = Env::new("garbage");
    let s = env.script(
        "garbage",
        "[[steps]]\ngarbage = true\n[[steps]]\nsleep_ms = 5000\n",
    );
    let t = Instant::now();
    env.run_with(&s, "rgarb", &[]);
    assert!(
        t.elapsed() < Duration::from_secs(4),
        "should fail fast, not wait out the script"
    );
    let run = env.store().run("rgarb").expect("run");
    assert_eq!(run.state, RunState::Failed);
    assert!(
        run.detail
            .as_deref()
            .unwrap_or_default()
            .contains("invalid JSON"),
        "{:?}",
        run.detail
    );
}

#[test]
fn killing_the_run_process_leaves_an_honest_interrupted_run() {
    let env = Env::new("kill9");
    let s = env.script("hang", "[[steps]]\nwrite = { path = \"wip.txt\", content = \"partial\" }\n[[steps]]\nhang = true\n");
    let mut child = env.spawn_run(&s, "rkill", &[]);
    env.wait_for("running", |st| {
        st.run("rkill")
            .map(|r| r.state == RunState::Running && r.pid.is_some())
            .unwrap_or(false)
    });
    let agent_pid = env.store().run("rkill").expect("run").pid.expect("pid");
    // Give the agent a moment to write its file.
    std::thread::sleep(Duration::from_millis(300));
    child.kill().expect("kill -9");
    let _ = child.wait();
    // The owner is dead but the row still says running: nobody pretends to know.
    assert_eq!(env.state("rkill"), RunState::Running);
    let report = env.ok(&["recover"]);
    assert!(report.contains("rkill was interrupted"), "{report}");
    let run = env.store().run("rkill").expect("run");
    assert_eq!(run.state, RunState::Interrupted);
    assert_eq!(
        git(
            &env.repo,
            &[
                "show",
                &format!("{}:wip.txt", run.snapshot.expect("snapshot"))
            ]
        ),
        "partial"
    );
    #[cfg(target_os = "linux")]
    {
        assert!(report.contains("stopped orphaned agent"), "{report}");
        std::thread::sleep(Duration::from_millis(200));
        let alive = std::fs::read_to_string(format!("/proc/{agent_pid}/stat"))
            .map(|s| !s.contains(") Z "))
            .unwrap_or(false);
        assert!(!alive, "orphaned agent {agent_pid} is still running");
    }
    // Recovery is idempotent.
    assert!(env.ok(&["recover"]).contains("nothing to recover"));
}

#[test]
fn permission_requests_wait_for_a_human_and_first_answer_wins() {
    let env = Env::new("ask");
    let s = env.script("ask", "[[steps]]\nask = { title = \"Run the payment tests\", kind = \"execute\" }\n[[steps]]\nwrite = { path = \"ran.txt\", content = \"yes\" }\n");
    let mut child = env.spawn_run(&s, "rask", &[]);
    env.wait_for("an ask", |st| !st.open_asks().expect("asks").is_empty());
    let status = env.ok(&["status"]);
    assert!(
        status.contains("waiting on a question from you"),
        "{status}"
    );
    let ask = env.store().open_asks().expect("asks")[0].id.to_string();
    assert!(
        !env.out(&["answer", &ask, "sure"]).status.success(),
        "answers must be one of the offered options"
    );
    env.ok(&["answer", &ask, "allow"]);
    assert!(
        env.ok(&["answer", &ask, "deny"])
            .contains("already answered")
    );
    let _ = child.wait();
    let run = env.store().run("rask").expect("run");
    assert_eq!(run.state, RunState::Finished);
    assert_eq!(
        git(
            &env.repo,
            &["show", &format!("{}:ran.txt", run.snapshot.expect("snap"))]
        ),
        "yes"
    );
}

#[test]
fn auto_policy_still_asks_before_touching_files_outside_the_worktree() {
    let env = Env::new("outside");
    // The test agent asks for an "execute" with no path under auto policy:
    // allowed. We check the decision was recorded as policy, not human.
    let s = env.script(
        "auto",
        "[[steps]]\nask = { title = \"cargo test\", kind = \"execute\" }\n",
    );
    env.run_with(&s, "rauto", &["--policy", "auto"]);
    let store = env.store();
    let perms: Vec<_> = store
        .run_events("rauto", 0, 100)
        .expect("ev")
        .into_iter()
        .filter(|e| e.kind == "permission")
        .collect();
    assert_eq!(perms.len(), 1);
    assert_eq!(perms[0].body["by"], "policy auto");
}

#[test]
fn accept_refuses_when_your_branch_moved_into_a_conflict() {
    let env = Env::new("conflict");
    env.run_with(&agent("good"), "rgood", &[]);
    // Meanwhile, you edited the same function and committed.
    let p = env.repo.join("payments.py");
    let text = std::fs::read_to_string(&p).expect("read").replace(
        "keep trying until it goes through",
        "keep trying (TODO: bound this)",
    );
    std::fs::write(&p, text).expect("write");
    git(&env.repo, &["commit", "-qam", "comment"]);
    let o = env.out(&["accept", "rgood"]);
    assert_eq!(
        o.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("payments.py"));
    // Nothing landed and the run is still reviewable.
    assert_eq!(
        git(&env.repo, &["log", "-1", "--format=%s"]).trim(),
        "comment"
    );
    assert!(env.store().run("rgood").expect("run").resolution.is_none());
}

#[test]
fn checks_run_on_the_combined_result_not_just_the_worktree() {
    let env = Env::new("combined");
    env.run_with(&agent("good"), "rgood", &[]);
    // A non-conflicting commit on main that breaks the retry test.
    let p = env.repo.join("test_retries.py");
    let text = std::fs::read_to_string(&p)
        .expect("read")
        .replace("assertLessEqual(up.calls, 3", "assertLessEqual(up.calls, 2");
    std::fs::write(&p, text).expect("write");
    git(&env.repo, &["commit", "-qam", "tighten budget"]);
    let o = env.out(&["accept", "rgood"]);
    assert_eq!(
        o.status.code(),
        Some(2),
        "green in its worktree, red combined: {}",
        String::from_utf8_lossy(&o.stdout)
    );
    // The next attempt's brief must not report the combined failure as the
    // attempt's own result, or the attempt's pass as a pass after merging.
    let brief = env.ok(&["brief", "bounded-retries"]);
    let line = |prefix: &str| {
        brief
            .lines()
            .find(|l| l.trim_start().starts_with(prefix))
            .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{brief}"))
            .to_string()
    };
    let own = line("Checks on its own change:");
    assert!(
        own.contains("retries pass") && !own.contains("fail"),
        "{own}"
    );
    assert!(
        line("Checks on other trees").contains("retries fail"),
        "{brief}"
    );
}

#[test]
fn evidence_goes_stale_when_you_edit_what_it_covers() {
    let env = Env::new("stale");
    // Make the checks pass on main first.
    env.run_with(&agent("good"), "rgood", &[]);
    env.ok(&["accept", "rgood"]);
    let _ = env.out(&["check"]);
    let show = env.ok(&["show", "one-key-per-charge"]);
    assert!(show.contains("pass"), "{show}");
    // Outside the idempotency check's scope: still fresh.
    std::fs::write(env.repo.join("README.md"), "hello").expect("write");
    let show = env.ok(&["show", "one-key-per-charge"]);
    assert!(show.contains("pass (unchanged scope)"), "{show}");
    // Inside it: stale, and it says which file.
    let p = env.repo.join("payments.py");
    std::fs::write(
        &p,
        std::fs::read_to_string(&p).expect("read") + "\n# touched\n",
    )
    .expect("write");
    let show = env.ok(&["show", "one-key-per-charge"]);
    assert!(show.contains("stale since: payments.py"), "{show}");
}

#[test]
fn an_interrupted_integration_is_settled_by_asking_git() {
    let env = Env::new("integ");
    env.run_with(&agent("good"), "rgood", &[]);
    let ws = Workspace::discover(&env.repo).expect("ws");
    let store = ws.open_store().expect("store");
    let head = git(&env.repo, &["rev-parse", "HEAD"]).trim().to_string();
    // A dead owner that got as far as "applying" and then the branch did
    // move: recovery must call it applied, not retry it.
    store
        .insert_integration("i-landed", "rgood", "main", &head, true, "p-dead")
        .expect("insert");
    store
        .set_integration("i-landed", IntegrationState::Verifying, Some(&head), None)
        .expect("verify");
    store
        .set_integration("i-landed", IntegrationState::Applying, None, None)
        .expect("apply");
    // And one that died while still verifying.
    store
        .insert_integration("i-early", "rgood", "main", &head, true, "p-dead2")
        .expect("insert");
    let report = env.ok(&["--json", "recover"]);
    let v: serde_json::Value = serde_json::from_str(&report).expect("json");
    let got: Vec<(String, String)> =
        serde_json::from_value(v["integrations"].clone()).expect("list");
    assert!(
        got.contains(&("i-landed".into(), "applied".into())),
        "{got:?}"
    );
    assert!(
        got.contains(&("i-early".into(), "abandoned".into())),
        "{got:?}"
    );
    assert_eq!(
        store.run("rgood").expect("run").resolution.as_deref(),
        Some("accepted")
    );
}

#[test]
fn untrusted_repositories_do_not_run_anything() {
    let env = Env::new("untrusted");
    std::fs::remove_file(env.cfg.join("trusted")).expect("untrust");
    let o = env.run_with(&agent("good"), "rgood", &[]);
    assert_eq!(o.status.code(), Some(3));
    assert_eq!(env.out(&["check"]).status.code(), Some(3));
}

#[test]
fn a_broken_rule_file_blocks_accept_and_is_loud_in_the_brief() {
    let env = Env::new("broken");
    env.run_with(&agent("good"), "rgood", &[]);
    std::fs::write(
        env.repo.join(".kitsu/invariants/one-key-per-charge.md"),
        "+++\nchecks = oops\n+++\n",
    )
    .expect("write");
    let brief = env.ok(&["brief", "bounded-retries"]);
    assert!(brief.contains("could not be read"), "{brief}");
    let o = env.out(&["accept", "rgood"]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn a_different_agent_can_continue_from_a_previous_run() {
    let env = Env::new("handoff");
    let first = env.script(
        "first",
        "[[steps]]\nwrite = { path = \"step1.txt\", content = \"one\" }\n",
    );
    env.run_with(&first, "rone", &[]);
    let second = env.script("second", "[[steps]]\nread = \"step1.txt\"\n[[steps]]\nwrite = { path = \"step2.txt\", content = \"two\" }\n");
    let o = env.run_with(&second, "rtwo", &["--from", "rone", "--note", "keep step1"]);
    assert!(o.status.success());
    let ws = Workspace::discover(&env.repo).expect("ws");
    let store = ws.open_store().expect("store");
    let run = store.run("rtwo").expect("run");
    let brief = String::from_utf8(
        ws.blobs()
            .get(run.brief.as_deref().expect("brief"))
            .expect("blob"),
    )
    .expect("utf8");
    assert!(brief.contains("continuing earlier work"));
    assert!(
        brief.contains("Run `rone`"),
        "earlier attempt is in the brief:\n{brief}"
    );
    assert!(brief.contains("keep step1"));
    let snap = run.snapshot.expect("snap");
    assert_eq!(
        git(&env.repo, &["show", &format!("{snap}:step1.txt")]),
        "one"
    );
    assert_eq!(
        git(&env.repo, &["show", &format!("{snap}:step2.txt")]),
        "two"
    );
}

#[test]
fn token_usage_is_recorded_when_reported_and_never_invented() {
    let env = Env::new("usage");
    let reporting = env.script(
        "reporting",
        "usage = { input = 1200, output = 300, used = 5000, size = 200000, cost = 0.02 }\n[[steps]]\nsay = \"done\"\n",
    );
    let silent = env.script("silent", "[[steps]]\nsay = \"done\"\n");
    assert!(
        env.run_with(&reporting, "rtok", &["--no-verify"])
            .status
            .success()
    );
    assert!(
        env.run_with(&silent, "rquiet", &["--no-verify"])
            .status
            .success()
    );
    let store = env.store();
    let u = store
        .run("rtok")
        .expect("run")
        .usage
        .expect("usage recorded");
    assert_eq!(
        (u.input, u.output, u.total),
        (Some(1200), Some(300), Some(1500))
    );
    assert_eq!((u.context_used, u.context_size), (Some(5000), Some(200000)));
    assert_eq!((u.cost, u.currency.as_deref()), (Some(0.02), Some("USD")));
    assert_eq!(
        store.run("rquiet").expect("run").usage,
        None,
        "silence is not zero"
    );

    let text = env.ok(&["stats"]);
    assert!(
        text.contains("2 runs, 1 reported usage, 1.5k tokens"),
        "{text}"
    );
    assert!(text.contains("(1 didn't report)"), "{text}");
    let json: serde_json::Value =
        serde_json::from_str(&env.ok(&["stats", "--json"])).expect("json");
    assert_eq!(json["all"]["spent"], 1500);
    assert_eq!(json["all"]["reported"], 1);
    assert!(
        json["brief_median"].as_u64().unwrap_or(0) > 0,
        "briefs are measured: {json}"
    );
}

#[test]
fn memory_goes_stale_with_its_code_and_reaches_the_brief() {
    let env = Env::new("memory");
    let path = env.ok(&[
        "remember",
        "Upstream dedupes idempotency keys for 24 hours",
        "--kind",
        "fact",
        "--scope",
        "payments.py",
        "--body",
        "From their API docs, section Idempotency.",
    ]);
    assert!(
        path.trim()
            .ends_with(".kitsu/memory/upstream-dedupes-idempotency-keys-for-24-hours.md"),
        "{path}"
    );
    git(&env.repo, &["add", "-A"]);
    git(&env.repo, &["commit", "-qm", "remember dedupe"]);
    let list = env.ok(&["memory"]);
    assert!(list.contains("current"), "{list}");

    let brief = env.ok(&["brief", "bounded-retries"]);
    assert!(
        brief.contains("Upstream dedupes idempotency keys for 24 hours"),
        "{brief}"
    );
    assert!(!brief.contains("May be out of date"), "{brief}");

    let p = env.repo.join("payments.py");
    let text = std::fs::read_to_string(&p).expect("read");
    std::fs::write(&p, format!("{text}\n# touched\n")).expect("write");
    git(&env.repo, &["commit", "-qam", "touch payments"]);
    let list = env.ok(&["memory"]);
    assert!(list.contains("stale: payments.py changed"), "{list}");
    let brief = env.ok(&["brief", "bounded-retries"]);
    assert!(
        brief.contains("May be out of date: payments.py changed"),
        "{brief}"
    );

    let bad = env.out(&["remember", "x", "--kind", "rumor"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown memory kind"));
}
