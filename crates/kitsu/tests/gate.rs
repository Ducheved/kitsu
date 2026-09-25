//! The contract in other agents' hooks and in CI, through the real binary:
//! `kitsu gate`, `kitsu hooks`, `kitsu diff`, `kitsu ci`.
//!
//! Hook inputs follow the vendors' documented shapes (Claude Code
//! https://code.claude.com/docs/en/hooks, Codex
//! https://developers.openai.com/codex/hooks, Cursor
//! https://cursor.com/docs/hooks; read 2026-09-25). No real agent runs here.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const KITSU: &str = env!("CARGO_BIN_EXE_kitsu");

const CONFIG: &str = r#"[checks.unit]
run = "grep -q ok src/a.txt"
guards = ["src/**"]
why = "a.txt says ok"

[checks.hidden]
run = "sh \"$KITSU_HELD_OUT/check.sh\""
guards = ["src/**"]
held_out = true

[protect]
paths = ["tests/**"]
"#;

struct Env {
    root: PathBuf,
    repo: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
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
        let root = std::env::temp_dir().join(format!("kitsu-gate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        let env = Env { root, repo };
        env.write(".kitsu/kitsu.toml", CONFIG);
        env.write("src/a.txt", "ok\n");
        env.write("tests/t.sh", "true\n");
        // The held-out check lives outside the repository.
        let held = env.root.join("held/hidden");
        std::fs::create_dir_all(&held).expect("held");
        std::fs::write(
            held.join("check.sh"),
            "grep -q ok src/a.txt || { echo SECRET-ASSERTION; exit 1; }\n",
        )
        .expect("held");
        git(&env.repo, &["init", "-q", "-b", "main"]);
        git(&env.repo, &["config", "user.name", "Dev"]);
        git(&env.repo, &["config", "user.email", "dev@example.com"]);
        git(&env.repo, &["config", "commit.gpgsign", "false"]);
        git(&env.repo, &["add", "-A"]);
        git(&env.repo, &["commit", "-qm", "init"]);
        assert!(env.run(&["trust"], "").status.success());
        env
    }

    fn write(&self, path: &str, body: &str) {
        let p = self.repo.join(path);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, body).expect("write");
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(KITSU)
            .args(args)
            .current_dir(&self.repo)
            .env("KITSU_CONFIG_DIR", self.root.join("cfg"))
            .env("KITSU_HELD_OUT_DIR", self.root.join("held"))
            .env_remove("KITSU_GATE_RULE_EDITS")
            .env_remove("GITHUB_BASE_REF")
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("kitsu");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("wait")
    }

    /// A hook call: exit code, stdout, stderr.
    fn hook(&self, args: &[&str], input: &Value) -> (i32, String, String) {
        let o = self.run(args, &input.to_string());
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }

    fn claude_stop(&self, active: bool) -> Value {
        json!({
            "session_id": "abc123",
            "transcript_path": null,
            "cwd": self.repo,
            "permission_mode": "default",
            "hook_event_name": "Stop",
            "stop_hook_active": active,
            "last_assistant_message": "All tests pass.",
            "background_tasks": [],
            "session_crons": []
        })
    }
}

fn parsed(stdout: &str) -> Value {
    serde_json::from_str(stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {stdout:?}"))
}

#[test]
fn stop_gate_blocks_until_the_checks_pass_and_never_loops_forever() {
    let env = Env::new("stop");
    env.write("src/a.txt", "bad\n");
    let claude = ["gate", "stop", "--for", "claude", "--max-blocks", "2"];

    let (code, out, err) = env.hook(&claude, &env.claude_stop(false));
    assert_eq!(code, 0, "{err}");
    let v = parsed(&out);
    assert_eq!(v["decision"], "block");
    let reason = v["reason"].as_str().expect("reason");
    assert!(reason.contains("check `unit` fail"), "{reason}");
    assert!(reason.contains("held-out check `hidden` fail"), "{reason}");
    // A held-out check's output and command never reach the agent.
    assert!(!reason.contains("SECRET-ASSERTION"), "{reason}");
    assert!(!reason.contains("check.sh"), "{reason}");
    // Nor through `kitsu check`, which an agent with a shell can run too.
    let o = env.run(&["check", "hidden"], "");
    assert_eq!(o.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&o.stderr).contains("SECRET-ASSERTION"));

    // Continuing after a refusal: once more, then the cap lets it stop and
    // says the change is not verified.
    let (_, out, _) = env.hook(&claude, &env.claude_stop(true));
    assert_eq!(parsed(&out)["decision"], "block");
    let (code, out, _) = env.hook(&claude, &env.claude_stop(true));
    assert_eq!(code, 0);
    let v = parsed(&out);
    assert!(v.get("decision").is_none(), "{v}");
    assert!(
        v["systemMessage"]
            .as_str()
            .expect("note")
            .contains("NOT verified")
    );

    // The same answer in Codex's and Cursor's protocols.
    let codex = json!({"cwd": env.repo, "hook_event_name": "Stop", "session_id": "c1", "turn_id": "t", "model": "m", "permission_mode": "default", "transcript_path": null, "last_assistant_message": null, "stop_hook_active": false});
    let (_, out, _) = env.hook(&["gate", "stop", "--for", "codex"], &codex);
    assert_eq!(parsed(&out)["decision"], "block");
    let cursor = json!({"conversation_id": "k", "hook_event_name": "stop", "workspace_roots": [env.repo], "status": "completed", "loop_count": 0});
    let (_, out, _) = env.hook(&["gate", "stop", "--for", "cursor"], &cursor);
    assert!(parsed(&out)["followup_message"].as_str().is_some(), "{out}");
    // Claude's TaskCompleted blocks through exit code 2 and stderr.
    let task = json!({"session_id": "t2", "cwd": env.repo, "hook_event_name": "TaskCompleted", "task_id": "task-001", "task_subject": "x"});
    let (code, out, err) = env.hook(&["gate", "stop", "--for", "claude"], &task);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.contains("check `unit` fail"), "{err}");

    // Fixed: every vendor lets it stop, from evidence where it exists.
    env.write("src/a.txt", "ok\n");
    let (code, out, err) = env.hook(&claude, &env.claude_stop(false));
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    let (_, out, _) = env.hook(&["gate", "stop", "--for", "codex"], &codex);
    assert_eq!(parsed(&out), json!({}));

    // A rule change needs a person's approval of exactly that diff.
    env.write("tests/t.sh", "exit 0\n");
    let (_, out, _) = env.hook(&claude, &env.claude_stop(false));
    let reason = parsed(&out)["reason"].as_str().expect("reason").to_string();
    assert!(
        reason.contains("rule change without approval: you changed tests/t.sh"),
        "{reason}"
    );
    let hash = reason
        .split("(hash ")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .expect("hash");
    let diff = env.run(&["diff", "HEAD", "--json"], "");
    assert_eq!(
        parsed(&String::from_utf8_lossy(&diff.stdout))["token"],
        hash
    );
    assert!(env.run(&["gate", "approve", hash], "").status.success());
    let (code, out, _) = env.hook(&claude, &env.claude_stop(false));
    assert_eq!((code, out.as_str()), (0, ""));
}

#[test]
fn pre_tool_gate_denies_edits_to_rule_paths() {
    let env = Env::new("pretool");
    let claude = ["gate", "pre-tool", "--for", "claude"];
    let edit = |path: PathBuf| {
        json!({"session_id": "s", "cwd": env.repo, "hook_event_name": "PreToolUse", "tool_name": "Edit", "tool_use_id": "t",
               "tool_input": {"file_path": path, "old_string": "a", "new_string": "b", "replace_all": false}})
    };
    let (code, out, _) = env.hook(&claude, &edit(env.repo.join("tests/t.sh")));
    assert_eq!(code, 0);
    let v = parsed(&out);
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(
        v["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .expect("reason")
            .contains("tests/t.sh")
    );
    let (_, out, _) = env.hook(&claude, &edit(env.repo.join(".kitsu/kitsu.toml")));
    assert_eq!(
        parsed(&out)["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    let (code, out, _) = env.hook(&claude, &edit(env.repo.join("src/a.txt")));
    assert_eq!((code, out.as_str()), (0, ""));

    let bash = |cmd: &str| json!({"cwd": env.repo.join("src"), "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": cmd}});
    let (_, out, _) = env.hook(&claude, &bash("echo x > ../tests/t.sh"));
    assert_eq!(
        parsed(&out)["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    let (_, out, _) = env.hook(&claude, &bash("kitsu gate approve 0123456789abcdef"));
    assert_eq!(
        parsed(&out)["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    let (_, out, _) = env.hook(&claude, &bash("cat ../tests/t.sh"));
    assert_eq!(out, "");

    let patch = json!({"cwd": env.repo, "hook_event_name": "PreToolUse", "tool_name": "apply_patch",
        "tool_input": {"command": "*** Begin Patch\n*** Update File: tests/t.sh\n@@\n-true\n+exit 0\n*** End Patch"}});
    let (_, out, _) = env.hook(&["gate", "pre-tool", "--for", "codex"], &patch);
    assert_eq!(
        parsed(&out)["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );

    let cursor = |p: &str| json!({"conversation_id": "c", "hook_event_name": "preToolUse", "cwd": env.repo, "tool_name": "Write", "tool_input": {"file_path": env.repo.join(p)}});
    let (_, out, _) = env.hook(
        &["gate", "pre-tool", "--for", "cursor"],
        &cursor("tests/t.sh"),
    );
    assert_eq!(parsed(&out)["permission"], "deny");
    let (_, out, _) = env.hook(
        &["gate", "pre-tool", "--for", "cursor"],
        &cursor("src/a.txt"),
    );
    assert_eq!(parsed(&out), json!({"permission": "allow"}));
}

#[test]
fn hooks_install_merges_and_uninstall_gives_the_file_back() {
    let env = Env::new("install");
    let mine = "{\n  \"permissions\": { \"deny\": [\"Read(./.env)\"] },\n  \"hooks\": { \"Stop\": [ { \"hooks\": [ { \"type\": \"command\", \"command\": \"notify-send done\" } ] } ] }\n}\n";
    env.write(".claude/settings.json", mine);
    let settings = env.repo.join(".claude/settings.json");

    let o = env.run(
        &["hooks", "install", "--for", "claude,codex", "--dry-run"],
        "",
    );
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success() && text.contains("would change"),
        "{text}"
    );
    assert!(
        text.contains("+ ") && text.contains("gate stop --for claude"),
        "{text}"
    );
    assert_eq!(std::fs::read_to_string(&settings).expect("read"), mine);
    assert!(!env.repo.join(".codex/hooks.json").exists());

    assert!(
        env.run(&["hooks", "install", "--for", "claude,codex"], "")
            .status
            .success()
    );
    let v = parsed(&std::fs::read_to_string(&settings).expect("read"));
    assert_eq!(v["permissions"]["deny"][0], "Read(./.env)");
    assert_eq!(
        v["hooks"]["Stop"][0]["hooks"][0]["command"],
        "notify-send done"
    );
    assert_eq!(
        v["hooks"]["Stop"][1]["hooks"][0]["command"],
        "kitsu gate stop --for claude"
    );
    let codex =
        parsed(&std::fs::read_to_string(env.repo.join(".codex/hooks.json")).expect("codex"));
    assert_eq!(
        codex["hooks"]["PreToolUse"][0]["matcher"],
        "^(apply_patch|Bash)$"
    );

    let o = env.run(&["hooks", "install", "--for", "claude,codex"], "");
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(text.matches("nothing to change").count(), 2, "{text}");

    assert!(
        env.run(&["hooks", "uninstall", "--for", "claude,codex"], "")
            .status
            .success()
    );
    let back = parsed(&std::fs::read_to_string(&settings).expect("read"));
    assert_eq!(back, parsed(mine));
    assert!(!env.repo.join(".codex/hooks.json").exists());

    // A config it can't read is refused, not overwritten.
    env.write(".cursor/hooks.json", "{ \"version\": 1, \"hooks\": ");
    let o = env.run(&["hooks", "install", "--for", "cursor"], "");
    assert!(!o.status.success());
    assert_eq!(
        std::fs::read_to_string(env.repo.join(".cursor/hooks.json")).expect("read"),
        "{ \"version\": 1, \"hooks\": "
    );
}

#[test]
fn diff_and_ci_judge_a_branch_by_the_base_rules() {
    let env = Env::new("ci");
    git(&env.repo, &["checkout", "-qb", "feature"]);
    env.write("src/a.txt", "bad\n");
    // The change also "fixes" the rules it's judged by.
    env.write(
        ".kitsu/kitsu.toml",
        &CONFIG.replace("grep -q ok src/a.txt", "true"),
    );
    git(&env.repo, &["commit", "-qam", "change"]);

    let o = env.run(&["diff", "main..feature", "--json"], "");
    let d = parsed(&String::from_utf8_lossy(&o.stdout));
    assert_eq!(d["rule_paths"], json!([".kitsu/kitsu.toml"]));
    assert_eq!(d["check_changes"][0]["check"], "unit");
    assert_eq!(d["check_changes"][0]["fields"], json!(["run"]));
    let required: Vec<&str> = d["required"]
        .as_array()
        .expect("required")
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert_eq!(required, ["hidden", "unit"]);
    let token = d["token"].as_str().expect("token").to_string();

    let outputs = env.root.join("gh-output");
    let ci = |extra: &[&str]| {
        let mut args = vec!["ci", "--base", "main"];
        args.extend_from_slice(extra);
        let mut c = Command::new(KITSU);
        c.args(&args)
            .current_dir(&env.repo)
            .env("KITSU_CONFIG_DIR", env.root.join("cfg"))
            .env("KITSU_HELD_OUT_DIR", env.root.join("held"))
            .env("GITHUB_OUTPUT", &outputs)
            .env("NO_COLOR", "1");
        c.output().expect("ci")
    };
    // Judged by main's check, not the weakened one: it fails.
    let o = ci(&[]);
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(2), "{text}");
    assert!(
        text.contains("| check | command | tree | exit | duration | result |"),
        "{text}"
    );
    assert!(text.contains("| unit |") && text.contains("fail"), "{text}");
    assert!(
        text.contains("held out; an agent with a shell could have read it"),
        "{text}"
    );

    env.write("src/a.txt", "ok\n");
    git(&env.repo, &["commit", "-qam", "fix"]);
    let o = ci(&[]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    let o = ci(&["--approved-rule-diff", "0000000000000000"]);
    assert_eq!(o.status.code(), Some(3));
    let o = ci(&["--approved-rule-diff", &token]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    let out = std::fs::read_to_string(&outputs).expect("GITHUB_OUTPUT");
    assert!(
        out.contains(&format!("rule-diff={token}\napproved=true\nfailing=\n")),
        "{out}"
    );
}
