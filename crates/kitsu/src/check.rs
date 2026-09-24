//! Checks turn commands into evidence.
//!
//! A check is a shell command declared in `.kitsu/kitsu.toml`. Running it
//! records one evidence row: which command (by fingerprint), against which
//! tree, with what outcome, and where the full log is. An agent saying
//! "tests pass" is a claim; this is the only way Kitsu learns that they do.
//!
//! Evidence is bound to the working tree's content hash, not to a branch or a
//! timestamp. If the tree changes while the check runs, the result is kept
//! but marked unbound, because it describes neither tree.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::git::Git;
use crate::intent::CheckDef;
use crate::store::{CheckOutcome, EvidenceRow, NewEvidence, Store};
use crate::util::now_ms;
use crate::workspace::Workspace;

/// Keep the first 256 KiB and the last 4 MiB of output. Test output that
/// fails usually says why at the end; the start says what ran.
const LOG_HEAD: usize = 256 * 1024;
const LOG_TAIL: usize = 4 * 1024 * 1024;

pub struct CheckRun<'a> {
    pub ws: &'a Workspace,
    pub store: &'a Store,
    /// Directory to run in: the main worktree or a run's worktree.
    pub dir: &'a Path,
    pub run: Option<&'a str>,
}

impl CheckRun<'_> {
    pub fn execute(&self, check: &CheckDef) -> Result<EvidenceRow> {
        let git = Git::new(self.dir);
        let scratch = self.ws.scratch();
        let tree = git.worktree_tree(&scratch)?;
        let started_at = now_ms();
        let t0 = Instant::now();
        let result = run_command(
            &check.run,
            self.dir,
            Duration::from_secs(check.timeout_secs),
            check,
        );
        let duration_ms = t0.elapsed().as_millis() as i64;
        let tree_after = git.worktree_tree(&scratch)?;
        let (outcome, exit_code, log) = match result {
            Ok(r) => (r.outcome, r.exit_code, r.log),
            Err(e) => (
                CheckOutcome::Error,
                None,
                format!("kitsu: could not run check: {e}\n").into_bytes(),
            ),
        };
        let log_bytes = log.len() as i64;
        let log_id = self.ws.blobs().put(&log)?;
        let id = self.store.insert_evidence(&NewEvidence {
            check_name: &check.name,
            fingerprint: &check.fingerprint(),
            command: &check.run,
            tree: &tree,
            tree_after: (tree_after != tree).then_some(tree_after.as_str()),
            outcome,
            exit_code,
            duration_ms,
            log: Some(&log_id),
            log_bytes: Some(log_bytes),
            run: self.run,
            started_at,
        })?;
        self.store.evidence(id)
    }
}

struct Ran {
    outcome: CheckOutcome,
    exit_code: Option<i32>,
    log: Vec<u8>,
}

fn shell(run: &str) -> Command {
    #[cfg(windows)]
    {
        let mut c = Command::new("cmd");
        c.args(["/C", &format!("{run} 2>&1")]);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("sh");
        // `exec 2>&1` first so every command in a compound `run` interleaves
        // stderr with stdout in the order it was written.
        c.args(["-c", &format!("exec 2>&1\n{run}")]);
        c
    }
}

fn run_command(run: &str, dir: &Path, timeout: Duration, check: &CheckDef) -> Result<Ran> {
    let mut cmd = shell(run);
    cmd.current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("KITSU_CHECK", &check.name);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group, so a timeout kills the test runner's children
        // too instead of leaving them holding the pipe open.
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::io(format!("spawning `{run}`"), e))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Invalid("no stdout pipe".into()))?;
    let reader = std::thread::spawn(move || {
        let mut log = BoundedLog::default();
        let mut buf = [0u8; 64 * 1024];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => log.push(&buf[..n]),
            }
        }
        log.finish()
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child
            .try_wait()
            .map_err(|e| Error::io("waiting for check", e))?
        {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => break None,
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let timed_out = status.is_none();
    if timed_out {
        kill_group(&mut child);
    }
    let status = match status {
        Some(s) => s,
        None => child
            .wait()
            .map_err(|e| Error::io("reaping timed out check", e))?,
    };
    let mut log = reader
        .join()
        .map_err(|_| Error::Invalid("log reader thread panicked".into()))?;
    let outcome = if timed_out {
        log.extend_from_slice(
            format!(
                "\nkitsu: timed out after {}s, process group killed\n",
                timeout.as_secs()
            )
            .as_bytes(),
        );
        CheckOutcome::Timeout
    } else if status.success() {
        CheckOutcome::Pass
    } else {
        CheckOutcome::Fail
    };
    Ok(Ran {
        outcome,
        exit_code: status.code(),
        log,
    })
}

fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pgid = child.id().to_string();
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pgid}")])
            .status();
    }
    let _ = child.kill();
}

#[derive(Default)]
struct BoundedLog {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    total: usize,
}

impl BoundedLog {
    fn push(&mut self, mut bytes: &[u8]) {
        self.total += bytes.len();
        if self.head.len() < LOG_HEAD {
            let take = (LOG_HEAD - self.head.len()).min(bytes.len());
            self.head.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
        }
        self.tail.extend(bytes);
        let excess = self.tail.len().saturating_sub(LOG_TAIL);
        self.tail.drain(..excess);
    }

    fn finish(self) -> Vec<u8> {
        let omitted = self.total - self.head.len() - self.tail.len();
        let mut out = self.head;
        if omitted > 0 {
            out.extend_from_slice(
                format!("\n[kitsu: {omitted} bytes of output omitted here]\n").as_bytes(),
            );
        }
        out.extend(self.tail);
        out
    }
}

/// What we know about one check on one tree.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckStatus {
    /// Evidence exists for exactly this tree.
    Current {
        outcome: CheckOutcome,
        evidence: i64,
    },
    /// Evidence is for another tree, but nothing inside the check's declared
    /// scope changed since. As strong as the scope declaration is honest.
    Carried {
        outcome: CheckOutcome,
        evidence: i64,
        from_tree: String,
    },
    /// Evidence is for another tree and relevant files changed.
    Stale {
        outcome: CheckOutcome,
        evidence: i64,
        changed: Vec<String>,
        more: usize,
    },
    /// Never ran with this definition.
    Unverified,
}

impl CheckStatus {
    pub fn passing(&self) -> bool {
        matches!(
            self,
            CheckStatus::Current {
                outcome: CheckOutcome::Pass,
                ..
            } | CheckStatus::Carried {
                outcome: CheckOutcome::Pass,
                ..
            }
        )
    }

    pub fn word(&self) -> &'static str {
        match self {
            CheckStatus::Current { outcome, .. } | CheckStatus::Carried { outcome, .. } => {
                outcome.as_str()
            }
            CheckStatus::Stale { .. } => "stale",
            CheckStatus::Unverified => "unverified",
        }
    }
}

pub fn status_at(git: &Git, store: &Store, check: &CheckDef, tree: &str) -> Result<CheckStatus> {
    let fp = check.fingerprint();
    if let Some(e) = store.evidence_at(&fp, tree)? {
        return Ok(CheckStatus::Current {
            outcome: e.outcome,
            evidence: e.id,
        });
    }
    let Some(last) = store.latest_evidence(&fp)? else {
        return Ok(CheckStatus::Unverified);
    };
    // If git can't diff the two trees (objects pruned), we can't claim
    // anything carried over.
    let changed = match git.changed_paths(&last.tree, tree) {
        Ok(c) => c,
        Err(_) => {
            return Ok(CheckStatus::Stale {
                outcome: last.outcome,
                evidence: last.id,
                changed: Vec::new(),
                more: 0,
            });
        }
    };
    let relevant: Vec<String> = if check.scope.is_everything() {
        changed
    } else {
        changed
            .into_iter()
            .filter(|p| check.scope.contains(p))
            .collect()
    };
    if relevant.is_empty() && !check.scope.is_everything() {
        return Ok(CheckStatus::Carried {
            outcome: last.outcome,
            evidence: last.id,
            from_tree: last.tree,
        });
    }
    const SHOW: usize = 8;
    let more = relevant.len().saturating_sub(SHOW);
    Ok(CheckStatus::Stale {
        outcome: last.outcome,
        evidence: last.id,
        changed: relevant.into_iter().take(SHOW).collect(),
        more,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::TempRepo;
    use crate::scope::Scope;

    fn check(run: &str, scope: &[&str]) -> CheckDef {
        CheckDef {
            name: "c".into(),
            run: run.into(),
            timeout_secs: 5,
            scope: Scope::new(scope.iter().copied()),
        }
    }

    #[test]
    fn records_pass_fail_and_timeout() {
        let repo = TempRepo::new(&[("a.txt", "a")]);
        let ws = Workspace::discover(&repo.root).expect("ws");
        let store = ws.open_store().expect("store");
        let cr = CheckRun {
            ws: &ws,
            store: &store,
            dir: &repo.root,
            run: None,
        };
        let pass = cr
            .execute(&check("echo ok; echo err >&2", &[]))
            .expect("pass");
        assert_eq!(pass.outcome, CheckOutcome::Pass);
        let log = ws
            .blobs()
            .get(pass.log.as_deref().expect("log"))
            .expect("blob");
        assert_eq!(String::from_utf8_lossy(&log), "ok\nerr\n");
        assert_eq!(
            cr.execute(&check("exit 3", &[])).expect("fail").exit_code,
            Some(3)
        );
        let mut slow = check("sleep 30", &[]);
        slow.timeout_secs = 1;
        let t = Instant::now();
        assert_eq!(
            cr.execute(&slow).expect("timeout").outcome,
            CheckOutcome::Timeout
        );
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_check_that_edits_files_is_not_bound() {
        let repo = TempRepo::new(&[("a.txt", "a")]);
        let ws = Workspace::discover(&repo.root).expect("ws");
        let store = ws.open_store().expect("store");
        let cr = CheckRun {
            ws: &ws,
            store: &store,
            dir: &repo.root,
            run: None,
        };
        let c = check("echo x > generated.txt", &[]);
        let e = cr.execute(&c).expect("ran");
        assert!(!e.is_bound());
        let tree = repo.git().worktree_tree(&ws.scratch()).expect("tree");
        assert!(matches!(
            status_at(&repo.git(), &store, &c, &tree).expect("status"),
            CheckStatus::Unverified
        ));
    }

    #[test]
    fn freshness_follows_scope() {
        let repo = TempRepo::new(&[("src/lib.rs", "1"), ("docs/a.md", "1")]);
        let ws = Workspace::discover(&repo.root).expect("ws");
        let store = ws.open_store().expect("store");
        let git = repo.git();
        let cr = CheckRun {
            ws: &ws,
            store: &store,
            dir: &repo.root,
            run: None,
        };
        let scoped = check("true", &["src/**"]);
        let unscoped = check("true ", &[]);
        cr.execute(&scoped).expect("scoped");
        cr.execute(&unscoped).expect("unscoped");

        repo.write("docs/a.md", "2");
        let t = git.worktree_tree(&ws.scratch()).expect("tree");
        assert!(matches!(
            status_at(&git, &store, &scoped, &t).expect("s"),
            CheckStatus::Carried { .. }
        ));
        assert!(matches!(
            status_at(&git, &store, &unscoped, &t).expect("s"),
            CheckStatus::Stale { .. }
        ));

        repo.write("src/lib.rs", "2");
        let t = git.worktree_tree(&ws.scratch()).expect("tree");
        match status_at(&git, &store, &scoped, &t).expect("s") {
            CheckStatus::Stale { changed, .. } => {
                assert_eq!(changed, vec!["src/lib.rs".to_string()])
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bounded_log_keeps_head_and_tail() {
        let mut l = BoundedLog::default();
        l.push(&vec![b'a'; LOG_HEAD]);
        l.push(&[b'b'; 100]);
        l.push(&vec![b'c'; LOG_TAIL]);
        let out = l.finish();
        assert!(out.starts_with(b"aaaa"));
        assert!(out.ends_with(b"cccc"));
        assert!(String::from_utf8_lossy(&out).contains("100 bytes of output omitted"));
    }
}
