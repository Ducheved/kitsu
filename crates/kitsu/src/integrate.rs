//! Accepting a run: getting an agent's change onto the user's branch.
//!
//! ```text
//! validate ─► rules from YOUR checkout ─► protected-path approval
//!    ─► build candidate (squash onto target head, in a scratch worktree)
//!    ─► run required checks ON THE CANDIDATE
//!    ─► fast-forward the target (fails if it moved) ─► receipt
//! ```
//!
//! Three things this refuses to do:
//! - judge a change by rules the change itself edited. Checks and what they guard
//!   come from the main worktree, which the agent never writes to.
//! - trust per-worktree green. Two changes that each pass can break each
//!   other; the checks run on the combined result.
//! - move a branch that moved. The final step is a fast-forward from the
//!   exact head the candidate was built on.

use std::path::PathBuf;

use serde::Serialize;

use crate::check::{CheckRun, CheckStatus, status_at};
use crate::error::{Error, Result};
use crate::git::{FileStat, Git};
use crate::intent::{self, Intent};
use crate::status::{protected_changes, required_checks};
use crate::store::{IntegrationState, RunRow, Store};
use crate::util::{content_id, short_id};
use crate::workspace::{Instance, Workspace};

/// What a reviewer needs before deciding.
#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub run: String,
    pub target: String,
    pub target_head: String,
    /// Merge base of target and the run's snapshot. The diff is from here.
    pub from: String,
    pub files: Vec<FileStat>,
    /// Changed paths under protected scopes (`.kitsu/**`, `[protect]`).
    pub protected: Vec<String>,
    /// Pass this back to `accept` to approve exactly these protected changes.
    /// It is a hash of their diff; if they change, the approval doesn't carry.
    pub approval_token: Option<String>,
    pub checks: Vec<(String, CheckStatus)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Accepted {
    Applied {
        commit: String,
        closed_task: bool,
        notes: Vec<String>,
    },
    NeedsApproval {
        paths: Vec<String>,
        token: String,
    },
    Conflict {
        paths: Vec<String>,
    },
    ChecksFailed {
        failing: Vec<String>,
        candidate: String,
    },
}

pub struct AcceptOptions {
    pub close_task: bool,
    pub approval: Option<String>,
}

fn target_of(git: &Git) -> Result<(String, String)> {
    let branch = git.current_branch()?.ok_or_else(|| {
        Error::Invalid(
            "your checkout is on a detached HEAD; check out the branch to accept into".into(),
        )
    })?;
    let head = git
        .branch_head(&branch)?
        .ok_or_else(|| Error::Invalid(format!("branch {branch} has no commits")))?;
    Ok((branch, head))
}

fn reviewable(run: &RunRow) -> Result<&str> {
    if !run.state.is_terminal() {
        return Err(Error::Invalid(format!(
            "run {} is still {}",
            run.id,
            run.state.as_str()
        )));
    }
    if let Some(r) = &run.resolution {
        return Err(Error::Conflict(format!("run {} was already {r}", run.id)));
    }
    run.snapshot
        .as_deref()
        .ok_or_else(|| Error::Invalid(format!("run {} has no snapshot", run.id)))
}

fn protected_diff_token(
    git: &Git,
    from: &str,
    to: &str,
    paths: &[String],
) -> Result<Option<String>> {
    if paths.is_empty() {
        return Ok(None);
    }
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let diff = git.diff(from, to, &refs)?;
    Ok(Some(content_id(diff.as_bytes())))
}

pub fn review(ws: &Workspace, store: &Store, run_id: &str) -> Result<Review> {
    let run = store.run(run_id)?;
    let snapshot = run
        .snapshot
        .clone()
        .ok_or_else(|| Error::Invalid(format!("run {run_id} has no snapshot yet")))?;
    let git = ws.git();
    let (target, head) = target_of(&git)?;
    let from = git.merge_base(&head, &snapshot)?;
    let changed = git.changed_paths(&from, &snapshot)?;
    let intent = Intent::load_dir(&ws.root)?;
    let protected = protected_changes(&intent, &changed);
    let approval_token = protected_diff_token(&git, &from, &snapshot, &protected)?;
    let mut checks = Vec::new();
    if let Some(task) = intent.tasks.get(&run.task) {
        let tree = git.tree_of(&snapshot)?;
        for req in required_checks(&intent, task, Some(&changed)) {
            checks.push((
                req.name.clone(),
                status_at(&git, store, &intent.config.checks[&req.name], &tree)?,
            ));
        }
    }
    Ok(Review {
        run: run.id,
        target,
        target_head: head,
        files: git.numstat(&from, &snapshot)?,
        from,
        protected,
        approval_token,
        checks,
    })
}

pub fn accept(
    ws: &Workspace,
    store: &Store,
    me: &Instance,
    run_id: &str,
    opts: &AcceptOptions,
) -> Result<Accepted> {
    if !ws.is_trusted()? {
        return Err(Error::Denied(
            "repository is not trusted; accepting runs its checks".into(),
        ));
    }
    let run = store.run(run_id)?;
    let snapshot = reviewable(&run)?.to_string();
    let git = ws.git();
    let intent = Intent::load_dir(&ws.root)?;
    if !intent.problems.is_empty() {
        let list: Vec<String> = intent
            .problems
            .iter()
            .map(|p| format!("{}: {}", p.path, p.detail))
            .collect();
        return Err(Error::Denied(format!(
            "fix these rule files first, the checks they define can't be enforced:\n  {}",
            list.join("\n  ")
        )));
    }
    let task = intent
        .tasks
        .get(&run.task)
        .ok_or_else(|| Error::NotFound(format!("task {} (in your checkout)", run.task)))?;
    let (target, head) = target_of(&git)?;
    let from = git.merge_base(&head, &snapshot)?;
    let changed = git.changed_paths(&from, &snapshot)?;
    if changed.is_empty() {
        return Err(Error::Invalid(format!(
            "run {run_id} changed nothing relative to {target}"
        )));
    }
    let protected = protected_changes(&intent, &changed);
    if let Some(token) = protected_diff_token(&git, &from, &snapshot, &protected)?
        && opts.approval.as_deref() != Some(token.as_str())
    {
        return Ok(Accepted::NeedsApproval {
            paths: protected,
            token,
        });
    }

    let id = short_id('i');
    store.insert_integration(&id, run_id, &target, &head, opts.close_task, &me.id)?;
    let dir = ws.integration_dir(&id);
    let result = build_and_apply(
        ws, store, &git, &intent, task, &run, &snapshot, &id, &dir, &target, &head, opts,
    );
    if let Ok(_guard) = ws.lock_worktrees() {
        let _ = ws.git().worktree_remove(&dir);
    }
    if let Err(e) = &result {
        let _ = store.set_integration(&id, IntegrationState::Failed, None, Some(&e.to_string()));
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn build_and_apply(
    ws: &Workspace,
    store: &Store,
    git: &Git,
    intent: &Intent,
    task: &intent::Task,
    run: &RunRow,
    snapshot: &str,
    id: &str,
    dir: &PathBuf,
    target: &str,
    head: &str,
    opts: &AcceptOptions,
) -> Result<Accepted> {
    {
        let _guard = ws.lock_worktrees()?;
        git.worktree_add_detached(dir, head)?;
    }
    let scratch = Git::new(dir);
    if let Err(paths) = scratch.merge_squash(snapshot)? {
        store.set_integration(
            id,
            IntegrationState::Conflict,
            None,
            Some(&paths.join(", ")),
        )?;
        return Ok(Accepted::Conflict { paths });
    }

    // Close the task in the same commit when the task file is committed and
    // untouched in your checkout. Otherwise we edit your working copy after
    // the fast-forward and you commit it with your next change.
    let task_path = task.source.path.clone();
    let close_in_commit = opts.close_task && git.is_clean_path(&task_path)?;
    if close_in_commit {
        let p = dir.join(&task_path);
        let text =
            std::fs::read_to_string(&p).map_err(|e| Error::io(p.display().to_string(), e))?;
        let edited = intent::set_state(&text, "done").map_err(|e| Error::Parse {
            path: p.clone(),
            detail: e,
        })?;
        std::fs::write(&p, edited).map_err(|e| Error::io(p.display().to_string(), e))?;
        scratch.add_path(&task_path)?;
    }
    let message = format!(
        "{}\n\nKitsu-Task: {}\nKitsu-Run: {}",
        task.title, task.id, run.id
    );
    let candidate = scratch.commit_as_user(&message, &ws.no_hooks())?;
    store.set_integration(id, IntegrationState::Verifying, Some(&candidate), None)?;

    let changed = git.changed_paths(head, &candidate)?;
    let tree = git.tree_of(&candidate)?;
    let cr = CheckRun {
        ws,
        store,
        dir,
        run: Some(&run.id),
    };
    let mut failing = Vec::new();
    for req in required_checks(intent, task, Some(&changed)) {
        let def = &intent.config.checks[&req.name];
        // Same tree as the run's snapshot (the usual case when nobody else
        // committed meanwhile): the evidence already exists, don't rerun.
        let status = match status_at(git, store, def, &tree)? {
            s @ CheckStatus::Current { .. } => s,
            _ => {
                cr.execute(def)?;
                status_at(git, store, def, &tree)?
            }
        };
        if !status.passing() {
            failing.push(req.name.clone());
        }
    }
    if !failing.is_empty() {
        store.set_integration(
            id,
            IntegrationState::Rejected,
            None,
            Some(&format!("failing: {}", failing.join(", "))),
        )?;
        return Ok(Accepted::ChecksFailed { failing, candidate });
    }

    store.set_integration(id, IntegrationState::Applying, None, None)?;
    git.merge_ff_only(&candidate).map_err(|e| match e {
        Error::Conflict(m) => Error::Conflict(format!(
            "{target} moved while checks ran, or your uncommitted changes overlap: {m}"
        )),
        e => e,
    })?;
    store.set_integration(id, IntegrationState::Applied, None, None)?;
    store.resolve_run(&run.id, "accepted")?;

    let mut notes = Vec::new();
    let mut closed = close_in_commit;
    if opts.close_task && !close_in_commit {
        let p = ws.root.join(&task_path);
        match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| intent::set_state(&t, "done"))
        {
            Ok(edited) => {
                match std::fs::write(&p, edited) {
                    Ok(()) => {
                        closed = true;
                        notes.push(format!("marked {task_path} done in your working copy; commit it when convenient"));
                    }
                    Err(e) => notes.push(format!("could not mark {task_path} done: {e}")),
                }
            }
            Err(e) => notes.push(format!("could not mark {task_path} done: {e}")),
        }
    }
    notes.extend(cleanup_run(ws, store, run));
    Ok(Accepted::Applied {
        commit: candidate,
        closed_task: closed,
        notes,
    })
}

pub fn discard(ws: &Workspace, store: &Store, run_id: &str) -> Result<Vec<String>> {
    let run = store.run(run_id)?;
    if !run.state.is_terminal() {
        return Err(Error::Invalid(format!(
            "run {run_id} is still {}; stop it first",
            run.state.as_str()
        )));
    }
    store.resolve_run(run_id, "discarded")?;
    Ok(cleanup_run(ws, store, &run))
}

/// Remove a resolved run's worktree and branch. Failures are reported and
/// recorded, and recovery retries them; they are never silently dropped.
pub fn cleanup_run(ws: &Workspace, store: &Store, run: &RunRow) -> Vec<String> {
    let mut notes = Vec::new();
    let git = ws.git();
    let wt = PathBuf::from(&run.worktree);
    let removed = if wt.exists() {
        match ws.lock_worktrees() {
            Ok(_guard) => git.worktree_remove(&wt),
            Err(e) => Err(e),
        }
    } else {
        Ok(())
    };
    if let Err(e) = removed {
        notes.push(format!("could not remove worktree {}: {e}", wt.display()));
        let _ = store.append(
            Some(&run.id),
            "cleanup.failed",
            &serde_json::json!({ "path": run.worktree, "error": e.to_string() }),
        );
        return notes;
    }
    if git.branch_head(&run.branch).ok().flatten().is_some()
        && let Err(e) = git.delete_branch(&run.branch)
    {
        notes.push(format!("could not delete branch {}: {e}", run.branch));
    }
    notes
}
