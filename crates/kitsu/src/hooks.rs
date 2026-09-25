//! The contract inside the agents you already use: Claude Code, Codex and
//! Cursor call `kitsu gate` from their own hooks.
//!
//! - `gate stop` runs when the agent wants to finish. It answers from the
//!   diff (see `contract.rs`), never from what the agent says: required
//!   checks without evidence for these exact files run now, and a failure or
//!   an unapproved rule change sends the agent back with the reason. After
//!   `max_blocks` refusals in a row it lets the agent stop and says the
//!   change is not verified, so it never loops forever.
//! - `gate pre-tool` refuses file edits to rule paths before they happen.
//!   For shell commands it can only catch the obvious (`> tests/x`, `rm`,
//!   `sed -i` on a rule path, `kitsu gate approve`): a script can write any
//!   file without naming it. The stop gate and CI classify the diff, which
//!   is where that is caught.
//! - `hooks install` merges the two gates into each agent's project config,
//!   keeping everything that was there.
//!
//! Each vendor speaks its own protocol; the fields used here were checked
//! against their docs on 2026-09-25 (the tests quote them):
//! - Claude Code: <https://code.claude.com/docs/en/hooks>
//! - Codex: <https://developers.openai.com/codex/hooks>
//! - Cursor: <https://cursor.com/docs/hooks>
//!
//! On its own errors the gate fails open (exit 1, which all three treat as a
//! non-blocking hook error) and says why. A hook is a guardrail; `kitsu ci`
//! on the pull request is the gate an agent can't talk its way past.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::contract::{self, Change, Receipt};
use crate::error::{Error, Result};
use crate::git::Git;
use crate::status::{guarded_checks, required_checks};
use crate::util::content_id;
use crate::workspace::Workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Vendor {
    Claude,
    Codex,
    Cursor,
}

impl Vendor {
    pub const ALL: [Vendor; 3] = [Vendor::Claude, Vendor::Codex, Vendor::Cursor];

    pub fn name(self) -> &'static str {
        match self {
            Vendor::Claude => "claude",
            Vendor::Codex => "codex",
            Vendor::Cursor => "cursor",
        }
    }

    /// The project-level hook config, relative to the repository root.
    pub fn config_path(self) -> &'static str {
        match self {
            Vendor::Claude => ".claude/settings.json",
            Vendor::Codex => ".codex/hooks.json",
            Vendor::Cursor => ".cursor/hooks.json",
        }
    }
}

/// What a hook process prints and exits with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub stdout: String,
    pub stderr: String,
    pub code: u8,
}

impl Reply {
    fn out(v: Value) -> Reply {
        Reply {
            stdout: format!("{v}\n"),
            stderr: String::new(),
            code: 0,
        }
    }

    fn silent() -> Reply {
        Reply {
            stdout: String::new(),
            stderr: String::new(),
            code: 0,
        }
    }

    fn failed(e: &Error) -> Reply {
        Reply {
            stdout: String::new(),
            stderr: format!("kitsu gate: {e} (the gate did not run; this is not a pass)\n"),
            code: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Let the agent stop, with an optional note for the person.
    Allow(Option<String>),
    /// Send the agent back with this reason.
    Block(String),
}

/// The stop answer in each vendor's protocol.
///
/// - Claude `Stop`: `{"decision":"block","reason":..}`; allowing is exit 0
///   with no output, or a `systemMessage` for the person. `TaskCompleted`
///   blocks only through exit code 2 with the reason on stderr.
/// - Codex `Stop`: the same `decision`/`reason` (it becomes the next
///   prompt); it expects JSON on exit 0, so allowing is `{}`.
/// - Cursor `stop`: `{"followup_message": ..}` is sent as the next user
///   message; `{}` lets it stop.
pub fn render_stop(vendor: Vendor, event: &str, answer: &Answer) -> Reply {
    match (vendor, answer) {
        (Vendor::Claude, Answer::Block(r)) if event == "TaskCompleted" => Reply {
            stdout: String::new(),
            stderr: format!("{r}\n"),
            code: 2,
        },
        (Vendor::Claude | Vendor::Codex, Answer::Block(r)) => {
            Reply::out(json!({ "decision": "block", "reason": r }))
        }
        (Vendor::Cursor, Answer::Block(r)) => Reply::out(json!({ "followup_message": r })),
        (Vendor::Claude, Answer::Allow(None)) => Reply::silent(),
        (Vendor::Claude | Vendor::Codex, Answer::Allow(Some(n))) => {
            Reply::out(json!({ "systemMessage": n }))
        }
        (Vendor::Codex | Vendor::Cursor, Answer::Allow(_)) => Reply::out(json!({})),
    }
}

/// The pre-tool answer. Claude and Codex share `hookSpecificOutput` with
/// `permissionDecision: "deny"`; allowing prints nothing, so the agent's
/// normal permission flow still applies. Cursor wants an explicit
/// `permission` on every call (invalid output blocks the action).
pub fn render_pre_tool(vendor: Vendor, deny: Option<&str>) -> Reply {
    match (vendor, deny) {
        (Vendor::Claude | Vendor::Codex, Some(r)) => Reply::out(json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": r,
            }
        })),
        (Vendor::Claude | Vendor::Codex, None) => Reply::silent(),
        (Vendor::Cursor, Some(r)) => Reply::out(json!({
            "permission": "deny",
            "user_message": r,
            "agent_message": r,
        })),
        (Vendor::Cursor, None) => Reply::out(json!({ "permission": "allow" })),
    }
}

fn parse_input(input: &str) -> Result<Value> {
    if input.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(input).map_err(|e| Error::Protocol(format!("hook input is not JSON: {e}")))
}

/// Where the agent is working: `cwd` (Claude, Codex, Cursor tool hooks),
/// else the first workspace root (Cursor), else our own directory.
fn input_dir(input: &Value, fallback: &Path) -> PathBuf {
    input["cwd"]
        .as_str()
        .or_else(|| input["workspace_roots"][0].as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.to_path_buf())
}

// ---- gate stop --------------------------------------------------------------

pub struct StopOptions {
    pub base: Option<String>,
    pub task: Option<String>,
    /// Refusals in a row before the gate lets the agent stop anyway.
    pub max_blocks: u32,
}

pub fn gate_stop(vendor: Vendor, input: &str, cwd: &Path, opts: &StopOptions) -> Reply {
    let parsed = match parse_input(input) {
        Ok(v) => v,
        Err(e) => return Reply::failed(&e),
    };
    let event = parsed["hook_event_name"].as_str().unwrap_or("Stop");
    match stop_answer(&parsed, cwd, opts) {
        Ok(a) => render_stop(vendor, event, &a),
        Err(e) => Reply::failed(&e),
    }
}

/// Serializes gates on one clone: Cursor also runs Claude's hooks, so two
/// can fire for one stop. The second then finds the first one's evidence.
fn lock(ws: &Workspace) -> Result<File> {
    std::fs::create_dir_all(&ws.state).map_err(|e| Error::io(ws.state.display().to_string(), e))?;
    let path = ws.state.join("gate.lock");
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| Error::io(path.display().to_string(), e))?;
    f.lock()
        .map_err(|e| Error::io(format!("locking {}", path.display()), e))?;
    Ok(f)
}

fn stop_answer(input: &Value, cwd: &Path, opts: &StopOptions) -> Result<Answer> {
    // Cursor only acts on a follow-up when the loop completed (it sends
    // `status` to Claude-format hooks it runs, too).
    if input["status"].as_str().is_some_and(|s| s != "completed") {
        return Ok(Answer::Allow(None));
    }
    let dir = input_dir(input, cwd);
    let ws = match Workspace::discover(&dir) {
        Ok(w) => w,
        // Not a repository: nothing to judge (a user-level hook in ~).
        Err(Error::NotFound(_)) => return Ok(Answer::Allow(None)),
        Err(e) => return Err(e),
    };
    let git = Git::new(Git::new(&dir).toplevel()?);
    let _guard = lock(&ws)?;
    if !ws.is_trusted()? {
        return Ok(Answer::Allow(Some(
            "Kitsu gate: this repository isn't trusted, so no checks ran and nothing is verified. `kitsu trust` in it turns the gate on.".into(),
        )));
    }
    let store = ws.open_store()?;
    let session = input["session_id"]
        .as_str()
        .or_else(|| input["conversation_id"].as_str())
        .unwrap_or("-");
    let key = format!("gate.blocks.{}", content_id(session.as_bytes()));

    let base = contract::resolve_base(&ws, &git, opts.base.as_deref())?;
    let change = Change::to_worktree(&ws, &git, base)?;
    if change.changed.is_empty() {
        store.set_meta(&key, "0")?;
        return Ok(Answer::Allow(None));
    }
    // Inside a Kitsu run's worktree the task's own checks count too.
    let task = opts.task.clone().or_else(|| {
        ws.run_for_path(git.dir())
            .and_then(|r| store.run(&r).ok())
            .map(|r| r.task)
    });
    let required = match task.as_ref().and_then(|t| change.rules.tasks.get(t)) {
        Some(t) => required_checks(&change.rules, t, Some(&change.changed)),
        None => guarded_checks(&change.rules, &change.changed),
    };
    let receipts = contract::verify(
        &ws,
        &store,
        git.dir(),
        &change.rules,
        &change.to_tree,
        &required,
    )?;
    let token = change.rule_token(&git)?;
    let unapproved = match &token {
        Some(t) => !contract::is_approved(&store, t)?,
        None => false,
    };
    let failing: Vec<&Receipt> = receipts.iter().filter(|r| !r.passed()).collect();
    let problems = (!change.rules.problems.is_empty()).then(|| {
        format!(
            "Kitsu gate: rule files at the base are broken, so some checks can't be enforced: {}",
            change
                .rules
                .problems
                .iter()
                .map(|p| format!("{}: {}", p.path, p.detail))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    if failing.is_empty() && !unapproved {
        store.set_meta(&key, "0")?;
        return Ok(Answer::Allow(problems));
    }

    // Consecutive refusals. Cursor counts them for us (also when it runs
    // Claude's hooks); Claude and Codex say whether this stop follows a
    // refusal; events that say neither (TaskCompleted) use our own count.
    let so_far: u32 = match (
        input["loop_count"].as_u64(),
        input["stop_hook_active"].as_bool(),
    ) {
        (Some(n), _) => n.min(u32::MAX as u64) as u32,
        (None, Some(false)) => 0,
        _ => store.meta(&key)?.and_then(|v| v.parse().ok()).unwrap_or(0),
    };
    let what: Vec<String> = failing
        .iter()
        .map(|r| format!("`{}` {}", r.check, r.outcome.as_str()))
        .chain(unapproved.then(|| "an unapproved rule change".to_string()))
        .collect();
    if so_far >= opts.max_blocks {
        store.set_meta(&key, "0")?;
        return Ok(Answer::Allow(Some(format!(
            "Kitsu gate: let the agent stop after {so_far} refusals in a row. Still: {}. This change is NOT verified.",
            what.join(", ")
        ))));
    }
    store.set_meta(&key, &(so_far + 1).to_string())?;
    Ok(Answer::Block(block_reason(
        &ws,
        &change,
        &failing,
        unapproved.then_some(token).flatten().as_deref(),
        so_far + 1,
        opts.max_blocks,
    )))
}

/// Model-visible text stays under what every vendor passes through whole
/// (Claude caps hook strings at 10,000 characters, Codex previews above
/// about 2,500 tokens).
const REASON_BUDGET: usize = 7_000;

fn block_reason(
    ws: &Workspace,
    change: &Change,
    failing: &[&Receipt],
    token: Option<&str>,
    n: u32,
    max: u32,
) -> String {
    let mut s = format!(
        "Kitsu: not done yet. This repository's rules (.kitsu/kitsu.toml at {}, {}) decide when a change is done, judged on your files as they are now (tree {}).\n",
        contract::short(&change.base),
        change.base_why,
        contract::short(&change.to_tree)
    );
    let per = if failing.is_empty() {
        0
    } else {
        (REASON_BUDGET / failing.len()).max(400)
    };
    for r in failing {
        let def = &change.rules.config.checks[&r.check];
        let exit = r
            .exit_code
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| r.outcome.as_str().to_string());
        if r.held_out {
            s.push_str(&format!(
                "\n- held-out check `{}` {} ({exit}). Its tests and output are kept from you on purpose: make the behavior right, don't hunt for the test.\n",
                r.check,
                r.outcome.as_str()
            ));
            if let Some(why) = &def.why {
                s.push_str(&format!("  It checks: {}\n", why.trim()));
            }
            continue;
        }
        s.push_str(&format!(
            "\n- check `{}` {} ({exit}, {:.1}s), required because it {}: `{}`\n",
            r.check,
            r.outcome.as_str(),
            r.duration_ms as f64 / 1000.0,
            r.required_by.join(", "),
            def.run
        ));
        if let Some(why) = &def.why {
            s.push_str(&format!("  Why it matters: {}\n", why.trim()));
        }
        let log = contract::log_text(ws, r);
        if !log.trim().is_empty() {
            s.push_str("  Output:\n");
            for l in contract::excerpt(log.trim_end(), per).lines() {
                s.push_str("    ");
                s.push_str(l);
                s.push('\n');
            }
        }
    }
    if let Some(t) = token {
        s.push_str(&format!(
            "\n- rule change without approval: you changed {}. Those are the rules this work is judged by. Put them back, or stop and tell the user why they need to change; only they can approve exactly this diff (hash {t}).\n",
            change.rule_paths().join(", ")
        ));
    }
    s.push_str(&format!(
        "\nFix this and finish again (refusal {n} of at most {max}; after that you may stop, and the change is reported as not verified)."
    ));
    s
}

// ---- gate pre-tool ----------------------------------------------------------

pub fn gate_pre_tool(vendor: Vendor, input: &str, cwd: &Path) -> Reply {
    let parsed = match parse_input(input) {
        Ok(v) => v,
        Err(e) => return Reply::failed(&e),
    };
    match pre_tool_answer(&parsed, cwd) {
        Ok(deny) => render_pre_tool(vendor, deny.as_deref()),
        Err(e) => Reply::failed(&e),
    }
}

/// The files a tool call would write, as far as can be told cheaply, and
/// whether it is a Kitsu command only a person should run.
pub fn tool_targets(tool: &str, input: &Value) -> (Vec<String>, bool) {
    match tool {
        "Bash" | "PowerShell" | "Shell" => {
            let cmd = input["command"].as_str().unwrap_or("");
            (shell_write_targets(cmd), runs_kitsu_approval(cmd))
        }
        // Codex: the patch text is in `command`.
        "apply_patch" => (
            patch_targets(input["command"].as_str().unwrap_or("")),
            false,
        ),
        _ => (
            [
                "file_path",
                "notebook_path",
                "path",
                "target_file",
                "filePath",
            ]
            .iter()
            .filter_map(|k| input[*k].as_str())
            .map(String::from)
            .collect(),
            false,
        ),
    }
}

fn pre_tool_answer(input: &Value, cwd: &Path) -> Result<Option<String>> {
    let tool = input["tool_name"].as_str().unwrap_or("");
    let (targets, approval) = tool_targets(tool, &input["tool_input"]);
    if approval {
        return Ok(Some(
            "Kitsu: approving a rule change, trusting a repository or removing Kitsu's hooks is for the person you work for, not for an agent. Ask them.".into(),
        ));
    }
    if targets.is_empty() || std::env::var("KITSU_GATE_RULE_EDITS").is_ok_and(|v| v == "1") {
        return Ok(None);
    }
    let dir = input_dir(input, cwd);
    let ws = match Workspace::discover(&dir) {
        Ok(w) => w,
        Err(Error::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let git = Git::new(Git::new(&dir).toplevel()?);
    let (base, _) = contract::resolve_base(&ws, &git, None)?;
    let rules = contract::rules_at(&git, &base)?;
    let protected = rules.protected();
    let hits: Vec<String> = targets
        .iter()
        .filter_map(|t| repo_relative(git.dir(), &dir, t))
        .filter(|p| protected.contains(p))
        .collect();
    if hits.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "Kitsu: {} is a rule of this repository (.kitsu/ and the [protect] paths in .kitsu/kitsu.toml): the work is judged by it, so an agent doesn't edit it. If the task really needs it changed, stop and say so; the user can change it, or restart you with KITSU_GATE_RULE_EDITS=1 to allow the edit and review it as a rule change.",
        hits.join(", ")
    )))
}

/// `p` relative to the repository at `top`, or `None` when it's outside.
fn repo_relative(top: &Path, cwd: &Path, p: &str) -> Option<String> {
    let raw = Path::new(p);
    let abs = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    };
    let abs = normalize(&abs);
    // Symlinked temp dirs (macOS /var -> /private/var): try both spellings.
    let canon_abs = abs
        .parent()
        .and_then(|d| std::fs::canonicalize(d).ok())
        .and_then(|d| abs.file_name().map(|n| d.join(n)));
    let tops = [Some(top.to_path_buf()), std::fs::canonicalize(top).ok()];
    for t in tops.iter().flatten() {
        for a in [Some(&abs), canon_abs.as_ref()].into_iter().flatten() {
            if let Ok(rel) = a.strip_prefix(t) {
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                if !parts.is_empty() {
                    return Some(parts.join("/"));
                }
            }
        }
    }
    None
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Paths a Codex `apply_patch` adds, updates, deletes or moves to.
pub fn patch_targets(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|l| {
            [
                "*** Add File: ",
                "*** Update File: ",
                "*** Delete File: ",
                "*** Move to: ",
            ]
            .iter()
            .find_map(|p| l.strip_prefix(p))
        })
        .map(|p| p.trim().to_string())
        .collect()
}

/// Split a shell command into simple commands of words, with redirection
/// targets marked. Quotes and backslashes are honored; expansions are not.
fn shell_commands(cmd: &str) -> Vec<(Vec<String>, Vec<String>)> {
    let mut out = Vec::new();
    let (mut words, mut redirs): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let mut cur = String::new();
    let mut in_word = false;
    // What the next word is: 0 a word, 1 an output file, 2 an input file.
    let mut redirect_next = 0u8;
    let mut chars = cmd.chars().peekable();
    let flush = |cur: &mut String,
                 in_word: &mut bool,
                 redirect_next: &mut u8,
                 words: &mut Vec<String>,
                 redirs: &mut Vec<String>| {
        if *in_word {
            match *redirect_next {
                1 => redirs.push(std::mem::take(cur)),
                2 => cur.clear(),
                _ => words.push(std::mem::take(cur)),
            }
            *redirect_next = 0;
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    cur.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                cur.push(n);
                            }
                        }
                        _ => cur.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '>' => {
                // `2>`, `&>`: the digits or & before it are not a word.
                if in_word && cur.chars().all(|d| d.is_ascii_digit() || d == '&') {
                    cur.clear();
                    in_word = false;
                }
                flush(
                    &mut cur,
                    &mut in_word,
                    &mut redirect_next,
                    &mut words,
                    &mut redirs,
                );
                while matches!(chars.peek(), Some('>') | Some('|')) {
                    chars.next();
                }
                if chars.peek() == Some(&'&') {
                    // `>&2` duplicates a descriptor, not a file.
                    chars.next();
                    while chars
                        .peek()
                        .is_some_and(|d| d.is_ascii_digit() || *d == '-')
                    {
                        chars.next();
                    }
                    continue;
                }
                redirect_next = 1;
            }
            '<' => {
                flush(
                    &mut cur,
                    &mut in_word,
                    &mut redirect_next,
                    &mut words,
                    &mut redirs,
                );
                while matches!(chars.peek(), Some('<') | Some('-')) {
                    chars.next();
                }
                redirect_next = 2;
            }
            ';' | '|' | '&' | '(' | ')' | '\n' | '`' => {
                flush(
                    &mut cur,
                    &mut in_word,
                    &mut redirect_next,
                    &mut words,
                    &mut redirs,
                );
                if !words.is_empty() || !redirs.is_empty() {
                    out.push((std::mem::take(&mut words), std::mem::take(&mut redirs)));
                }
            }
            c if c.is_whitespace() => {
                flush(
                    &mut cur,
                    &mut in_word,
                    &mut redirect_next,
                    &mut words,
                    &mut redirs,
                );
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    flush(
        &mut cur,
        &mut in_word,
        &mut redirect_next,
        &mut words,
        &mut redirs,
    );
    if !words.is_empty() || !redirs.is_empty() {
        out.push((words, redirs));
    }
    out
}

/// Files a shell command obviously writes: redirection targets, and the
/// arguments of commands that write, move or delete what they're given.
/// Anything cleverer (a script, an interpreter, `find -exec`) is not seen.
pub fn shell_write_targets(cmd: &str) -> Vec<String> {
    const WRITERS: &[&str] = &[
        "rm", "rmdir", "mv", "cp", "tee", "truncate", "touch", "chmod", "chown", "ln", "install",
        "unlink", "shred", "rsync", "mkdir",
    ];
    let mut out = Vec::new();
    for (words, redirs) in shell_commands(cmd) {
        out.extend(redirs);
        let mut w: &[String] = &words;
        while let Some(first) = w.first() {
            let wrapper = (first.contains('=') && !first.starts_with('-'))
                || ["sudo", "env", "command", "exec", "nohup", "time", "xargs"]
                    .contains(&first.as_str());
            if !wrapper {
                break;
            }
            w = &w[1..];
        }
        let Some((verb, args)) = w.split_first() else {
            continue;
        };
        let verb = verb.rsplit('/').next().unwrap_or(verb);
        let operands = || args.iter().filter(|a| !a.starts_with('-')).cloned();
        let writes = WRITERS.contains(&verb)
            || (matches!(verb, "sed" | "perl")
                && args
                    .iter()
                    .any(|a| a.starts_with("-i") || a == "--in-place" || a.starts_with("-pi")));
        if writes {
            out.extend(operands());
        } else if verb == "dd" {
            out.extend(
                args.iter()
                    .filter_map(|a| a.strip_prefix("of="))
                    .map(String::from),
            );
        } else if verb == "git"
            && args
                .first()
                .is_some_and(|s| ["rm", "mv", "checkout", "restore"].contains(&s.as_str()))
        {
            out.extend(args[1..].iter().filter(|a| !a.starts_with('-')).cloned());
        }
    }
    out
}

/// Is this a Kitsu command that only a person should run?
fn runs_kitsu_approval(cmd: &str) -> bool {
    shell_commands(cmd).iter().any(|(words, _)| {
        let kitsu = words.iter().any(|w| w.contains("kitsu"));
        let asks = words.windows(2).any(|p| {
            matches!(
                (p[0].as_str(), p[1].as_str()),
                ("gate", "approve") | ("hooks", "uninstall") | ("hooks", "install")
            )
        });
        let trusts = words.first().is_some_and(|w| w.ends_with("kitsu"))
            && words.get(1).is_some_and(|w| w == "trust");
        (kitsu && asks) || trusts
    })
}

// ---- hooks install ----------------------------------------------------------

/// JSON that keeps its keys in the order they were written, so merging a
/// hook into someone's settings file doesn't reorder the rest of it.
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Json, D::Error> {
        d.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_bool<E>(self, b: bool) -> std::result::Result<Json, E> {
        Ok(Json::Bool(b))
    }
    fn visit_i64<E>(self, n: i64) -> std::result::Result<Json, E> {
        Ok(Json::Num(n.into()))
    }
    fn visit_u64<E>(self, n: u64) -> std::result::Result<Json, E> {
        Ok(Json::Num(n.into()))
    }
    fn visit_f64<E: de::Error>(self, n: f64) -> std::result::Result<Json, E> {
        serde_json::Number::from_f64(n)
            .map(Json::Num)
            .ok_or_else(|| E::custom("not a finite number"))
    }
    fn visit_str<E>(self, s: &str) -> std::result::Result<Json, E> {
        Ok(Json::Str(s.to_string()))
    }
    fn visit_string<E>(self, s: String) -> std::result::Result<Json, E> {
        Ok(Json::Str(s))
    }
    fn visit_unit<E>(self) -> std::result::Result<Json, E> {
        Ok(Json::Null)
    }
    fn visit_none<E>(self) -> std::result::Result<Json, E> {
        Ok(Json::Null)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Json, D::Error> {
        Json::deserialize(d)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Json, A::Error> {
        let mut v = Vec::new();
        while let Some(x) = a.next_element()? {
            v.push(x);
        }
        Ok(Json::Arr(v))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<Json, A::Error> {
        let mut v = Vec::new();
        while let Some((k, x)) = a.next_entry::<String, Json>()? {
            v.push((k, x));
        }
        Ok(Json::Obj(v))
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Json::Null => s.serialize_unit(),
            Json::Bool(b) => s.serialize_bool(*b),
            Json::Num(n) => n.serialize(s),
            Json::Str(x) => s.serialize_str(x),
            Json::Arr(v) => {
                let mut q = s.serialize_seq(Some(v.len()))?;
                for x in v {
                    q.serialize_element(x)?;
                }
                q.end()
            }
            Json::Obj(m) => {
                let mut q = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    q.serialize_entry(k, v)?;
                }
                q.end()
            }
        }
    }
}

impl Json {
    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Set `key`, keeping its position if it's there, appending otherwise.
    fn set(&mut self, key: &str, value: Json) {
        if let Json::Obj(m) = self {
            match m.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value,
                None => m.push((key.to_string(), value)),
            }
        }
    }

    fn remove(&mut self, key: &str) {
        if let Json::Obj(m) = self {
            m.retain(|(k, _)| k != key);
        }
    }

    /// `key`'s value, inserted as `default` if missing. Errors when it is
    /// there but not the kind `default` is: we don't guess what a config
    /// we don't understand means.
    fn entry(&mut self, key: &str, default: Json, file: &Path) -> Result<&mut Json> {
        let Json::Obj(m) = self else {
            return Err(parse_err(file, "expected a JSON object"));
        };
        let i = match m.iter().position(|(k, _)| k == key) {
            Some(i) => i,
            None => {
                m.push((key.to_string(), default.clone()));
                m.len() - 1
            }
        };
        let v = &mut m[i].1;
        if std::mem::discriminant(v) != std::mem::discriminant(&default) {
            return Err(parse_err(
                file,
                &format!("`{key}` is not what {} expects here", file.display()),
            ));
        }
        Ok(v)
    }
}

fn parse_err(file: &Path, detail: &str) -> Error {
    Error::Parse {
        path: file.to_path_buf(),
        detail: format!("{detail}; left untouched"),
    }
}

struct Want {
    event: &'static str,
    matcher: Option<&'static str>,
    marker: String,
    command: String,
    timeout: u64,
}

/// The two gates for one vendor. Stop gets 30 minutes (a test suite can be
/// slow; a timed-out hook is treated as no answer by all three); the
/// pre-tool gate is a few git calls.
fn wants(vendor: Vendor, bin: &str) -> Vec<Want> {
    let v = vendor.name();
    let (stop, pre, pre_matcher) = match vendor {
        Vendor::Claude => (
            "Stop",
            "PreToolUse",
            "Bash|PowerShell|Edit|Write|NotebookEdit",
        ),
        Vendor::Codex => ("Stop", "PreToolUse", "^(apply_patch|Bash)$"),
        Vendor::Cursor => ("stop", "preToolUse", "Write|Delete"),
    };
    let bin = shell_quote(bin);
    vec![
        Want {
            event: stop,
            matcher: None,
            marker: format!("gate stop --for {v}"),
            command: format!("{bin} gate stop --for {v}"),
            timeout: 1800,
        },
        Want {
            event: pre,
            matcher: Some(pre_matcher),
            marker: format!("gate pre-tool --for {v}"),
            command: format!("{bin} gate pre-tool --for {v}"),
            timeout: 30,
        },
    ]
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./-+:=".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn is_ours(handler: &Json, marker: &str) -> bool {
    handler
        .get("command")
        .and_then(Json::str)
        .is_some_and(|c| c.contains(marker))
}

/// Claude's and Codex's shape: event -> [{matcher, hooks: [handler]}].
fn install_nested(root: &mut Json, wants: &[Want], file: &Path) -> Result<()> {
    let hooks = root.entry("hooks", Json::Obj(Vec::new()), file)?;
    for w in wants {
        let handler = Json::obj(vec![
            ("type", Json::Str("command".into())),
            ("command", Json::Str(w.command.clone())),
            ("timeout", Json::Num(w.timeout.into())),
            (
                "statusMessage",
                Json::Str(if w.matcher.is_none() {
                    "Kitsu: checking the change against the repository's rules".into()
                } else {
                    "Kitsu: is this a rule file?".into()
                }),
            ),
        ]);
        let Json::Arr(groups) = hooks.entry(w.event, Json::Arr(Vec::new()), file)? else {
            unreachable!("entry checked the kind");
        };
        let mut found = false;
        for g in groups.iter_mut() {
            let Some(Json::Arr(hs)) = g.get("hooks") else {
                continue;
            };
            if !hs.iter().any(|h| is_ours(h, &w.marker)) {
                continue;
            }
            found = true;
            let only_ours = hs.iter().all(|h| is_ours(h, &w.marker));
            let mut hs = hs.clone();
            for h in hs.iter_mut().filter(|h| is_ours(h, &w.marker)) {
                h.set("command", Json::Str(w.command.clone()));
                h.set("timeout", Json::Num(w.timeout.into()));
            }
            g.set("hooks", Json::Arr(hs));
            if only_ours && let Some(m) = w.matcher {
                g.set("matcher", Json::Str(m.into()));
            }
        }
        if !found {
            let mut g = Vec::new();
            if let Some(m) = w.matcher {
                g.push(("matcher", Json::Str(m.into())));
            }
            g.push(("hooks", Json::Arr(vec![handler])));
            groups.push(Json::obj(g));
        }
    }
    Ok(())
}

/// Cursor's shape: {version: 1, hooks: {event: [handler]}}.
fn install_flat(root: &mut Json, wants: &[Want], file: &Path) -> Result<()> {
    if root.get("version").is_none() {
        if let Json::Obj(m) = root {
            m.insert(0, ("version".into(), Json::Num(1.into())));
        } else {
            return Err(parse_err(file, "expected a JSON object"));
        }
    }
    let hooks = root.entry("hooks", Json::Obj(Vec::new()), file)?;
    for w in wants {
        let Json::Arr(list) = hooks.entry(w.event, Json::Arr(Vec::new()), file)? else {
            unreachable!("entry checked the kind");
        };
        let mut found = false;
        for h in list.iter_mut().filter(|h| is_ours(h, &w.marker)) {
            found = true;
            h.set("command", Json::Str(w.command.clone()));
            h.set("timeout", Json::Num(w.timeout.into()));
        }
        if !found {
            let mut h = vec![
                ("command", Json::Str(w.command.clone())),
                ("timeout", Json::Num(w.timeout.into())),
            ];
            match w.matcher {
                Some(m) => h.push(("matcher", Json::Str(m.into()))),
                // Cursor's own cap on stop follow-ups; ours is max_blocks.
                None => h.push(("loop_limit", Json::Num(5.into()))),
            }
            list.push(Json::obj(h));
        }
    }
    Ok(())
}

/// Remove every handler with one of `markers`, and whatever that leaves
/// empty. Nothing else is touched.
fn uninstall(root: &mut Json, markers: &[String], nested: bool) {
    let Some(Json::Obj(events)) = root.get("hooks").cloned() else {
        return;
    };
    let ours = |h: &Json| markers.iter().any(|m| is_ours(h, m));
    let mut kept_events = Vec::new();
    let mut touched = false;
    for (event, list) in events {
        let Json::Arr(items) = list else {
            kept_events.push((event, list));
            continue;
        };
        let before = items.len();
        let mut kept = Vec::new();
        for mut item in items {
            if !nested {
                if ours(&item) {
                    touched = true;
                } else {
                    kept.push(item);
                }
                continue;
            }
            match item.get("hooks") {
                Some(Json::Arr(hs)) if hs.iter().any(ours) => {
                    touched = true;
                    let rest: Vec<Json> = hs.iter().filter(|h| !ours(h)).cloned().collect();
                    if !rest.is_empty() {
                        item.set("hooks", Json::Arr(rest));
                        kept.push(item);
                    }
                }
                _ => kept.push(item),
            }
        }
        if kept.is_empty() && before > 0 && touched {
            continue;
        }
        kept_events.push((event, Json::Arr(kept)));
    }
    if !touched {
        return;
    }
    if kept_events.is_empty() {
        root.remove("hooks");
    } else {
        root.set("hooks", Json::Obj(kept_events));
    }
}

/// One file `hooks install` or `uninstall` would write.
#[derive(Debug, Clone, Serialize)]
pub struct Planned {
    pub vendor: Vendor,
    pub path: PathBuf,
    pub before: Option<String>,
    /// `None`: the file is removed (uninstall left nothing in it).
    pub after: Option<String>,
}

impl Planned {
    pub fn changes(&self) -> bool {
        self.before != self.after
    }

    pub fn apply(&self) -> Result<()> {
        match &self.after {
            Some(text) => {
                if let Some(d) = self.path.parent() {
                    std::fs::create_dir_all(d)
                        .map_err(|e| Error::io(d.display().to_string(), e))?;
                }
                std::fs::write(&self.path, text)
                    .map_err(|e| Error::io(self.path.display().to_string(), e))
            }
            None => std::fs::remove_file(&self.path)
                .map_err(|e| Error::io(self.path.display().to_string(), e)),
        }
    }

    /// A line diff of before and after, for `--dry-run`.
    pub fn diff(&self) -> String {
        line_diff(
            self.before.as_deref().unwrap_or(""),
            self.after.as_deref().unwrap_or(""),
        )
    }
}

fn read_json(path: &Path) -> Result<(Option<String>, Json)> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let json = serde_json::from_str::<Json>(&text).map_err(|e| Error::Parse {
                path: path.to_path_buf(),
                detail: format!("{e}; left untouched, fix it or install by hand"),
            })?;
            Ok((Some(text), json))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((None, Json::Obj(Vec::new()))),
        Err(e) => Err(Error::io(path.display().to_string(), e)),
    }
}

/// Pretty JSON with the indent the file already used.
fn write_json(json: &Json, like: Option<&str>) -> Result<String> {
    let indent = like
        .and_then(|t| {
            t.lines()
                .map(|l| &l[..l.len() - l.trim_start().len()])
                .find(|w| !w.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "  ".into());
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    json.serialize(&mut ser)
        .map_err(|e| Error::Invalid(format!("writing JSON: {e}")))?;
    buf.push(b'\n');
    String::from_utf8(buf).map_err(|e| Error::Invalid(e.to_string()))
}

pub fn plan(root: &Path, vendors: &[Vendor], bin: &str, install: bool) -> Result<Vec<Planned>> {
    let mut out = Vec::new();
    for &v in vendors {
        let path = root.join(v.config_path());
        let (before, json) = read_json(&path)?;
        let mut next = json.clone();
        let w = wants(v, bin);
        let nested = v != Vendor::Cursor;
        if install {
            if nested {
                install_nested(&mut next, &w, &path)?;
            } else {
                install_flat(&mut next, &w, &path)?;
            }
        } else {
            uninstall(
                &mut next,
                &w.iter().map(|w| w.marker.clone()).collect::<Vec<_>>(),
                nested,
            );
        }
        let after = if next == json {
            before.clone()
        } else {
            let empty = match &next {
                Json::Obj(m) => m.iter().all(|(k, _)| k == "version"),
                _ => false,
            };
            if !install && empty {
                None
            } else {
                Some(write_json(&next, before.as_deref())?)
            }
        };
        out.push(Planned {
            vendor: v,
            path,
            before,
            after,
        });
    }
    Ok(out)
}

/// A minimal unified-style diff: `-`/`+` for changed lines, two lines of
/// context around them. Config files are small; this is O(n·m).
fn line_diff(a: &str, b: &str) -> String {
    let a: Vec<&str> = a.lines().collect();
    let b: Vec<&str> = b.lines().collect();
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            ops.push((' ', a[i]));
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
            ops.push(('-', a[i]));
            i += 1;
        } else {
            ops.push(('+', b[j]));
            j += 1;
        }
    }
    let near = |k: usize| {
        let lo = k.saturating_sub(2);
        let hi = (k + 2).min(ops.len() - 1);
        (lo..=hi).any(|x| ops[x].0 != ' ')
    };
    let mut out = String::new();
    let mut gap = false;
    for (k, (op, line)) in ops.iter().enumerate() {
        if *op != ' ' || near(k) {
            if gap {
                out.push_str("  ...\n");
                gap = false;
            }
            out.push(*op);
            out.push(' ');
            out.push_str(line);
            out.push('\n');
        } else {
            gap = !out.is_empty();
        }
    }
    out
}

/// Is `bin` something the agent's hook shell can run?
pub fn on_path(bin: &str) -> bool {
    if bin.contains('/') {
        return Path::new(bin).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- protocol fixtures, copied from the vendors' docs on 2026-09-25 ----

    /// https://code.claude.com/docs/en/hooks.md, "Stop input".
    const CLAUDE_STOP_INPUT: &str = r#"{
  "session_id": "abc123",
  "transcript_path": "~/.claude/projects/.../00893aaf-19fa-41d2-8238-13269b9b3ca0.jsonl",
  "cwd": "/Users/...",
  "permission_mode": "default",
  "hook_event_name": "Stop",
  "stop_hook_active": true,
  "last_assistant_message": "I've completed the refactoring. Here's a summary...",
  "background_tasks": [],
  "session_crons": []
}"#;
    /// Same page, "Stop decision control".
    const CLAUDE_STOP_BLOCK: &str = r#"{
  "decision": "block",
  "reason": "Must be provided when Claude is blocked from stopping"
}"#;
    /// Same page, "PreToolUse decision control" (Tabs: PreToolUse).
    const CLAUDE_PRE_DENY: &str = r#"{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "Database writes are not allowed"
  }
}"#;
    /// Same page, "PreToolUse input" (Write on Windows) and "Edit".
    const CLAUDE_PRE_WRITE_INPUT: &str = r#"{
  "hook_event_name": "PreToolUse",
  "tool_name": "Write",
  "tool_input": {
    "file_path": "C:\\project\\src\\index.ts",
    "content": "..."
  }
}"#;
    /// https://developers.openai.com/codex/hooks.md, "Stop".
    const CODEX_STOP_BLOCK: &str = r#"{
  "decision": "block",
  "reason": "Run one more pass over the failing tests."
}"#;
    /// Same page, "PreToolUse": the deny shape.
    const CODEX_PRE_DENY: &str = r#"{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "Destructive command blocked by hook."
  }
}"#;
    /// Required fields of codex-rs/hooks/schema/generated/
    /// stop.command.input.schema.json (openai/codex main, 2026-09-25).
    const CODEX_STOP_INPUT: &str = r#"{"cwd": "/tmp/x", "hook_event_name": "Stop", "last_assistant_message": null, "model": "gpt-5", "permission_mode": "default", "session_id": "s1", "stop_hook_active": false, "transcript_path": null, "turn_id": "t1"}"#;
    /// https://cursor.com/docs/hooks.md, "stop".
    const CURSOR_STOP_INPUT: &str = r#"{ "status": "completed", "loop_count": 0 }"#;
    const CURSOR_STOP_OUTPUT: &str = r#"{ "followup_message": "<message text>" }"#;
    /// Same page, "preToolUse" output (deny).
    const CURSOR_PRE_OUTPUT_KEYS: [&str; 3] = ["permission", "user_message", "agent_message"];

    fn keys(v: &Value) -> Vec<String> {
        v.as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn json_of(r: &Reply) -> Value {
        serde_json::from_str(&r.stdout).expect("reply is JSON")
    }

    #[test]
    fn stop_replies_match_each_vendors_documented_shape() {
        let block = Answer::Block("fix it".into());
        for (vendor, fixture) in [
            (Vendor::Claude, CLAUDE_STOP_BLOCK),
            (Vendor::Codex, CODEX_STOP_BLOCK),
            (Vendor::Cursor, CURSOR_STOP_OUTPUT),
        ] {
            let r = render_stop(vendor, "Stop", &block);
            assert_eq!(r.code, 0, "{vendor:?}");
            let doc: Value = serde_json::from_str(fixture).expect("fixture");
            let ours = json_of(&r);
            assert_eq!(keys(&ours), keys(&doc), "{vendor:?}");
            let text = ours["reason"]
                .as_str()
                .or(ours["followup_message"].as_str());
            assert_eq!(text, Some("fix it"));
        }
        // Claude TaskCompleted blocks only through exit 2 and stderr.
        let r = render_stop(Vendor::Claude, "TaskCompleted", &block);
        assert_eq!(
            (r.code, r.stdout.as_str(), r.stderr.as_str()),
            (2, "", "fix it\n")
        );
        // Allowing: Claude says nothing; Codex and Cursor get an empty object
        // (Codex treats plain text on Stop as invalid).
        assert_eq!(
            render_stop(Vendor::Claude, "Stop", &Answer::Allow(None)),
            Reply::silent()
        );
        for v in [Vendor::Codex, Vendor::Cursor] {
            assert_eq!(
                json_of(&render_stop(v, "Stop", &Answer::Allow(None))),
                json!({})
            );
        }
        let note = render_stop(Vendor::Claude, "Stop", &Answer::Allow(Some("n".into())));
        assert_eq!(json_of(&note), json!({ "systemMessage": "n" }));
    }

    #[test]
    fn pre_tool_replies_match_each_vendors_documented_shape() {
        for (vendor, fixture) in [
            (Vendor::Claude, CLAUDE_PRE_DENY),
            (Vendor::Codex, CODEX_PRE_DENY),
        ] {
            let ours = json_of(&render_pre_tool(vendor, Some("no")));
            let doc: Value = serde_json::from_str(fixture).expect("fixture");
            assert_eq!(keys(&ours), keys(&doc));
            assert_eq!(
                keys(&ours["hookSpecificOutput"]).len(),
                keys(&doc["hookSpecificOutput"]).len()
            );
            assert_eq!(ours["hookSpecificOutput"]["permissionDecision"], "deny");
            assert_eq!(ours["hookSpecificOutput"]["hookEventName"], "PreToolUse");
            assert_eq!(render_pre_tool(vendor, None), Reply::silent());
        }
        let ours = json_of(&render_pre_tool(Vendor::Cursor, Some("no")));
        let mut k = keys(&ours);
        k.sort();
        let mut want: Vec<String> = CURSOR_PRE_OUTPUT_KEYS
            .iter()
            .map(|s| s.to_string())
            .collect();
        want.sort();
        assert_eq!(k, want);
        assert_eq!(
            json_of(&render_pre_tool(Vendor::Cursor, None)),
            json!({ "permission": "allow" })
        );
    }

    #[test]
    fn inputs_from_the_docs_parse_into_what_the_gate_reads() {
        let c = parse_input(CLAUDE_STOP_INPUT).expect("claude");
        assert_eq!(c["stop_hook_active"], true);
        assert_eq!(
            input_dir(&c, Path::new("/fallback")),
            PathBuf::from("/Users/...")
        );
        let x = parse_input(CODEX_STOP_INPUT).expect("codex");
        assert_eq!(x["stop_hook_active"], false);
        let cu = parse_input(CURSOR_STOP_INPUT).expect("cursor");
        assert_eq!(
            input_dir(&cu, Path::new("/fallback")),
            PathBuf::from("/fallback")
        );
        assert_eq!(cu["loop_count"].as_u64(), Some(0));
        let w = parse_input(CLAUDE_PRE_WRITE_INPUT).expect("write");
        let (t, approval) = tool_targets("Write", &w["tool_input"]);
        assert_eq!(t, ["C:\\project\\src\\index.ts"]);
        assert!(!approval);
        assert!(parse_input("not json").is_err());
        assert_eq!(parse_input("").expect("empty"), json!({}));
    }

    #[test]
    fn cursor_stop_that_did_not_complete_is_left_alone() {
        let v = json!({ "status": "aborted", "loop_count": 0 });
        let opts = StopOptions {
            base: None,
            task: None,
            max_blocks: 5,
        };
        assert_eq!(
            stop_answer(&v, Path::new("/"), &opts).expect("answer"),
            Answer::Allow(None)
        );
    }

    #[test]
    fn shell_targets_catch_the_obvious_and_say_nothing_else() {
        let t = |c: &str| shell_write_targets(c);
        assert_eq!(t("echo x > tests/a.rs"), ["tests/a.rs"]);
        assert_eq!(t("echo x >>tests/a.rs 2>&1"), ["tests/a.rs"]);
        assert_eq!(t("cargo test 2> err.log"), ["err.log"]);
        assert_eq!(t("cat tests/a.rs | grep x"), Vec::<String>::new());
        assert_eq!(t("rm -f 'tests/a b.rs' && ls"), ["tests/a b.rs"]);
        assert_eq!(t("sed -i 's/a/b/' tests/a.rs"), ["s/a/b/", "tests/a.rs"]);
        assert_eq!(t("sed 's/a/b/' tests/a.rs"), Vec::<String>::new());
        assert_eq!(t("FOO=1 sudo mv a.rs tests/b.rs"), ["a.rs", "tests/b.rs"]);
        assert_eq!(t("git checkout -- tests/a.rs"), ["tests/a.rs"]);
        assert_eq!(t("dd if=/dev/zero of=tests/x"), ["tests/x"]);
        assert_eq!(t("echo 1 >&2; tee -a tests/x < in"), ["tests/x"]);
        // Out of reach, by design: this is what the stop gate is for.
        assert_eq!(
            t("python3 -c 'open(\"tests/a.rs\",\"w\")'"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_agent_cannot_approve_its_own_rule_change() {
        assert!(runs_kitsu_approval("kitsu gate approve 0123456789abcdef"));
        assert!(runs_kitsu_approval("cd x && ~/bin/kitsu hooks uninstall"));
        assert!(runs_kitsu_approval(
            "cargo run -q -p kitsu -- gate approve abc"
        ));
        assert!(runs_kitsu_approval("kitsu trust"));
        assert!(!runs_kitsu_approval("kitsu status && kitsu diff HEAD"));
        assert!(!runs_kitsu_approval("echo gate approve"));
    }

    #[test]
    fn codex_patches_name_their_files() {
        let p = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-a\n+b\n*** Add File: tests/new.rs\n+x\n*** Delete File: old.rs\n*** Update File: x.rs\n*** Move to: tests/y.rs\n*** End Patch";
        assert_eq!(
            patch_targets(p),
            ["src/a.rs", "tests/new.rs", "old.rs", "x.rs", "tests/y.rs"]
        );
    }

    #[test]
    fn paths_resolve_against_the_repository() {
        let top = Path::new("/r");
        assert_eq!(
            repo_relative(top, Path::new("/r/src"), "../tests/a.rs").as_deref(),
            Some("tests/a.rs")
        );
        assert_eq!(
            repo_relative(top, Path::new("/r"), "/r/.kitsu/kitsu.toml").as_deref(),
            Some(".kitsu/kitsu.toml")
        );
        assert_eq!(repo_relative(top, Path::new("/r"), "/etc/passwd"), None);
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("kitsu-hooks-{}", crate::util::short_id('h')));
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    #[test]
    fn install_merges_keeps_order_and_is_idempotent() {
        let root = tmp();
        let path = root.join(".claude/settings.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let mine = "{\n    \"permissions\": {\"deny\": [\"Edit(/secrets/**)\"]},\n    \"hooks\": {\n        \"Stop\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"say done\"}]}]\n    },\n    \"env\": {\"A\": \"1\"}\n}\n";
        std::fs::write(&path, mine).expect("write");

        let p = plan(&root, &[Vendor::Claude, Vendor::Cursor], "kitsu", true).expect("plan");
        assert!(p.iter().all(Planned::changes));
        for x in &p {
            x.apply().expect("apply");
        }
        let text = std::fs::read_to_string(&path).expect("read");
        // The user's keys keep their order and indent; their hook stays.
        let order: Vec<usize> = ["\"permissions\"", "\"hooks\"", "\"env\""]
            .iter()
            .map(|k| text.find(k).expect("key"))
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
        assert!(text.contains("\n    \"permissions\""), "{text}");
        let v: Value = serde_json::from_str(&text).expect("json");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        assert_eq!(
            v["hooks"]["Stop"][1]["hooks"][0]["command"],
            "kitsu gate stop --for claude"
        );
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["matcher"],
            "Bash|PowerShell|Edit|Write|NotebookEdit"
        );
        let cursor: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(".cursor/hooks.json")).expect("cursor"),
        )
        .expect("json");
        assert_eq!(cursor["version"], 1);
        assert_eq!(cursor["hooks"]["stop"][0]["loop_limit"], 5);

        // Again: nothing to do.
        let again = plan(&root, &[Vendor::Claude, Vendor::Cursor], "kitsu", true).expect("plan");
        assert!(again.iter().all(|x| !x.changes()));
        // A new binary path updates our entries in place.
        let moved = plan(&root, &[Vendor::Claude], "/opt/k it/kitsu", true).expect("plan");
        assert!(moved[0].diff().contains("+ "), "{}", moved[0].diff());
        assert!(
            moved[0]
                .after
                .as_deref()
                .expect("after")
                .contains("'/opt/k it/kitsu' gate stop")
        );

        // Uninstall gives the user's file back, and removes ours entirely.
        let un = plan(&root, &[Vendor::Claude, Vendor::Cursor], "kitsu", false).expect("plan");
        for x in &un {
            x.apply().expect("apply");
        }
        let back: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        let orig: Value = serde_json::from_str(mine).expect("json");
        assert_eq!(back, orig);
        assert!(!root.join(".cursor/hooks.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_refuses_what_it_cannot_read() {
        let root = tmp();
        std::fs::create_dir_all(root.join(".codex")).expect("mkdir");
        std::fs::write(root.join(".codex/hooks.json"), "{ \"hooks\": [1, 2] }").expect("write");
        assert!(matches!(
            plan(&root, &[Vendor::Codex], "kitsu", true),
            Err(Error::Parse { .. })
        ));
        std::fs::write(root.join(".codex/hooks.json"), "{ nope").expect("write");
        assert!(matches!(
            plan(&root, &[Vendor::Codex], "kitsu", true),
            Err(Error::Parse { .. })
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn line_diff_shows_changes_with_context() {
        let d = line_diff("a\nb\nc\nd\ne\nf\ng\n", "a\nb\nc\nX\ne\nf\ng\n");
        assert_eq!(d, "  b\n  c\n- d\n+ X\n  e\n  f\n");
        assert_eq!(line_diff("", "x\n"), "+ x\n");
    }
}
