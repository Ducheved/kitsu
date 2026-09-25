//! What to do after somebody died.
//!
//! Recovery runs whenever a Kitsu process starts owning things (the app on
//! launch, `kitsu run`, `kitsu accept`) and on `kitsu recover`. It only
//! touches work whose owner is provably gone (its lock file is free), so it
//! is safe to run from several processes at once.
//!
//! It answers four questions, and says "unknown" where it can't:
//! 1. Runs whose owner died: mark interrupted, stop the orphaned agent if we
//!    can identify it, snapshot whatever it left in the worktree.
//! 2. Integrations interrupted mid-way: did the branch move? Git knows.
//! 3. Accepted or discarded runs whose worktree is still on disk: remove.
//! 4. Worktrees nobody claims: report, don't guess.

use std::path::PathBuf;

use serde::Serialize;

use crate::error::Result;
use crate::integrate::cleanup_run;
use crate::run::RunEvent;
use crate::runner;
use crate::store::{IntegrationState, Store};
use crate::workspace::{Instance, Liveness, Workspace};

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub interrupted: Vec<String>,
    /// Agent processes stopped because their custodian was gone.
    pub reaped: Vec<String>,
    /// Agents that may still be running; we could not prove what they are.
    pub unknown_processes: Vec<String>,
    pub integrations: Vec<(String, String)>,
    pub cleaned: Vec<String>,
    pub problems: Vec<String>,
    pub orphan_worktrees: Vec<String>,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.interrupted.is_empty()
            && self.reaped.is_empty()
            && self.unknown_processes.is_empty()
            && self.integrations.is_empty()
            && self.cleaned.is_empty()
            && self.problems.is_empty()
            && self.orphan_worktrees.is_empty()
    }
}

pub fn recover(ws: &Workspace, store: &Store) -> Result<Report> {
    let mut report = Report::default();
    let git = ws.git();

    for run in store.live_runs()? {
        let Some(owner) = &run.owner else { continue };
        if Instance::liveness(ws, owner)? == Liveness::Alive {
            continue;
        }
        if let Some(pid) = run.pid {
            match reap(pid, &run.id) {
                Reaped::Killed => report.reaped.push(format!("{} (pid {pid})", run.id)),
                Reaped::Gone => {}
                Reaped::Unknown => report.unknown_processes.push(format!(
                    "run {} agent pid {pid} may still be running",
                    run.id
                )),
            }
        }
        store.apply_run_event(&run.id, &RunEvent::OwnerLost)?;
        store.set_run_pid(&run.id, None)?;
        report.interrupted.push(run.id.clone());
        // Keep the partial work. No checks: nobody asked for them and the
        // work is incomplete by definition.
        if let Err(e) = runner::finish(ws, store, &run.id, false) {
            report.problems.push(format!(
                "could not snapshot interrupted run {}: {e}",
                run.id
            ));
        }
    }

    for i in store.unfinished_integrations()? {
        if Instance::liveness(ws, &i.owner)? == Liveness::Alive {
            continue;
        }
        let dir = ws.integration_dir(&i.id);
        let outcome = match (i.state, &i.candidate) {
            (IntegrationState::Applying, Some(candidate)) => {
                let head = git.branch_head(&i.target)?;
                let landed = match &head {
                    Some(h) => git.is_ancestor(candidate, h)?,
                    None => false,
                };
                if landed {
                    store.set_integration(
                        &i.id,
                        IntegrationState::Applied,
                        None,
                        Some("confirmed by recovery: the target contains the candidate"),
                    )?;
                    IntegrationState::Applied
                } else {
                    store.set_integration(
                        &i.id,
                        IntegrationState::Failed,
                        None,
                        Some("interrupted before the target moved"),
                    )?;
                    IntegrationState::Failed
                }
            }
            _ => {
                store.set_integration(
                    &i.id,
                    IntegrationState::Abandoned,
                    None,
                    Some("interrupted before applying; nothing changed on the target"),
                )?;
                IntegrationState::Abandoned
            }
        };
        if dir.exists() {
            let _guard = ws.lock_worktrees()?;
            let _ = git.worktree_remove(&dir);
        }
        report
            .integrations
            .push((i.id.clone(), outcome.as_str().to_string()));
    }

    // An applied integration whose run never got marked accepted: the
    // process died between the two writes.
    for run in store.recent_runs(10_000)? {
        if run.resolution.is_none()
            && store
                .integrations_for_run(&run.id)?
                .iter()
                .any(|i| i.state == IntegrationState::Applied)
        {
            store.resolve_run(&run.id, "accepted")?;
            report.cleaned.push(format!(
                "{} marked accepted (its integration had landed)",
                run.id
            ));
        }
    }

    let mut known: Vec<PathBuf> = Vec::new();
    for run in store.recent_runs(10_000)? {
        let wt = PathBuf::from(&run.worktree);
        known.push(wt.clone());
        if run.resolution.is_some() && wt.exists() {
            let notes = cleanup_run(ws, store, &run);
            if notes.is_empty() {
                report.cleaned.push(run.worktree.clone());
            } else {
                report.problems.extend(notes);
            }
        }
    }
    {
        let _guard = ws.lock_worktrees()?;
        let _ = git.prune_worktrees();
    }

    if let Ok(entries) = std::fs::read_dir(ws.state.join("worktrees")) {
        for e in entries.flatten() {
            let p = e.path();
            if !known.contains(&p) {
                report.orphan_worktrees.push(p.display().to_string());
            }
        }
    }
    Ok(report)
}

// Only Linux can prove who a pid is (see `reap`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Reaped {
    Killed,
    Gone,
    Unknown,
}

/// Stop an agent whose custodian died, but only if we can prove the pid is
/// still that agent. On Linux the agent's environment carries
/// `KITSU_RUN=<id>`, which survives pid reuse checks. Elsewhere we don't
/// guess.
fn reap(pid: u32, run: &str) -> Reaped {
    #[cfg(target_os = "linux")]
    {
        let environ = match std::fs::read(format!("/proc/{pid}/environ")) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Reaped::Gone,
            Err(_) => return Reaped::Unknown,
        };
        let marker = format!("KITSU_RUN={run}");
        if !environ.split(|b| *b == 0).any(|kv| kv == marker.as_bytes()) {
            // Same number, different process.
            return Reaped::Gone;
        }
        let ok = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok { Reaped::Killed } else { Reaped::Unknown }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, run);
        Reaped::Unknown
    }
}
