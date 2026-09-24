//! Task status, derived.
//!
//! Nothing stores "this task is blocked" or "this task is done-ish". Status
//! is computed from three authorities, each owning its own part:
//!
//! - intent files (git): is the task open, what does it depend on, which
//!   questions block it, which checks define done;
//! - runs (state.db): is an agent on it, did the attempt end, was it
//!   accepted or thrown away;
//! - evidence (state.db): do the checks pass on the tree that matters.
//!
//! An agent cannot move a task to done. Only a human closing the task file
//! does that, and accepting a run is how that usually happens.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::check::{CheckStatus, status_at};
use crate::error::Result;
use crate::git::Git;
use crate::intent::{CheckDef, Intent, InvariantState, Task, TaskState};
use crate::run::RunState;
use crate::scope::Scope;
use crate::store::{RunRow, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    /// A human has to do something before this can move.
    NeedsYou,
    /// An agent is on it.
    Working,
    /// Nothing is stopping it. Someone could start it now.
    Ready,
    /// Waiting on other tasks.
    Waiting,
    /// Done or dropped.
    Quiet,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    Running {
        run: String,
        stopping: bool,
    },
    /// The running agent is waiting for a human answer.
    Asking {
        run: String,
        asks: usize,
    },
    /// An attempt finished and nobody accepted or discarded it yet.
    Review {
        run: String,
        verdict: Verdict,
    },
    Failed {
        run: String,
        detail: String,
    },
    Interrupted {
        run: String,
    },
    BlockedByQuestion {
        questions: Vec<String>,
    },
    BlockedByTasks {
        tasks: Vec<String>,
    },
    Ready,
    Done,
    Dropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every required check passes on the run's snapshot.
    Verified,
    /// At least one required check fails.
    Failing,
    /// Some required check has no evidence for the snapshot yet.
    Unverified,
    /// The agent changed nothing.
    Empty,
    /// There is no snapshot to judge.
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub attention: Attention,
    /// One line a person can read without knowing the model.
    pub reason: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequiredCheck {
    pub name: String,
    /// Why this check is required, e.g. "task acceptance" or
    /// "invariant idempotency-key (src/client/**)".
    pub why: Vec<String>,
}

/// The checks that decide whether a change for `task` is acceptable:
/// the task's own acceptance checks, plus the checks of every active
/// invariant whose scope the change touches.
///
/// `changed` is the list of paths the change actually modified, when known.
/// Before any change exists, invariants are matched against the task's
/// declared scope instead (conservatively, see `Scope::may_overlap`).
pub fn required_checks(
    intent: &Intent,
    task: &Task,
    changed: Option<&[String]>,
) -> Vec<RequiredCheck> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in &task.checks {
        out.entry(c.clone())
            .or_default()
            .push("task acceptance".into());
    }
    for inv in intent
        .invariants
        .values()
        .filter(|i| i.state == InvariantState::Active)
    {
        let hit = match changed {
            Some(paths) => {
                inv.scope.touches_any(paths.iter().map(String::as_str))
                    || inv.scope.may_overlap(&task.scope)
            }
            None => inv.scope.may_overlap(&task.scope),
        };
        if hit {
            for c in &inv.checks {
                out.entry(c.clone())
                    .or_default()
                    .push(format!("invariant {}", inv.id));
            }
        }
    }
    out.into_iter()
        .filter(|(name, _)| intent.config.checks.contains_key(name))
        .map(|(name, why)| RequiredCheck { name, why })
        .collect()
}

pub fn verdict(
    git: &Git,
    store: &Store,
    intent: &Intent,
    task: &Task,
    run: &RunRow,
) -> Result<(Verdict, Vec<(String, CheckStatus)>)> {
    let (Some(tree), Some(changed)) = (&run.snapshot_tree, &run.changed) else {
        return Ok((Verdict::Unknown, Vec::new()));
    };
    if changed.is_empty() {
        return Ok((Verdict::Empty, Vec::new()));
    }
    let mut results = Vec::new();
    for req in required_checks(intent, task, Some(changed)) {
        let def: &CheckDef = &intent.config.checks[&req.name];
        results.push((req.name, status_at(git, store, def, tree)?));
    }
    let v = if results.iter().any(|(_, s)| matches!(s, CheckStatus::Current { outcome, .. } | CheckStatus::Carried { outcome, .. } if *outcome != crate::store::CheckOutcome::Pass)) {
        Verdict::Failing
    } else if results.iter().all(|(_, s)| s.passing()) {
        Verdict::Verified
    } else {
        Verdict::Unverified
    };
    Ok((v, results))
}

pub struct Snapshot<'a> {
    pub intent: &'a Intent,
    pub git: &'a Git,
    pub store: &'a Store,
}

impl Snapshot<'_> {
    pub fn tasks(&self) -> Result<Vec<TaskView>> {
        // Every attempt nobody has accepted or discarded yet, newest first.
        // Several can exist for one task (best-of-N, a retry after a crash);
        // hiding all but the latest would hide the one that worked.
        let mut open: BTreeMap<String, Vec<RunRow>> = BTreeMap::new();
        for r in self.store.unresolved_runs()? {
            open.entry(r.task.clone()).or_default().push(r);
        }
        let mut asks: BTreeMap<String, usize> = BTreeMap::new();
        for a in self.store.open_asks()? {
            *asks.entry(a.run).or_default() += 1;
        }
        let none = Vec::new();
        let mut views = Vec::with_capacity(self.intent.tasks.len());
        for task in self.intent.tasks.values() {
            let runs = open.get(&task.id).unwrap_or(&none);
            let (status, attention, reason) = self.classify(task, runs, &asks)?;
            views.push(TaskView {
                id: task.id.clone(),
                title: task.title.clone(),
                status,
                attention,
                reason,
                path: task.source.path.clone(),
            });
        }
        views.sort_by(|a, b| a.attention.cmp(&b.attention).then_with(|| a.id.cmp(&b.id)));
        Ok(views)
    }

    fn classify(
        &self,
        task: &Task,
        runs: &[RunRow],
        asks: &BTreeMap<String, usize>,
    ) -> Result<(Status, Attention, String)> {
        let leftover = |base: &str| match runs.len() {
            0 => base.to_string(),
            1 => format!("{base}; attempt {} was never reviewed", runs[0].id),
            n => format!("{base}; {n} attempts were never reviewed"),
        };
        match task.state {
            TaskState::Done => return Ok((Status::Done, Attention::Quiet, leftover("done"))),
            TaskState::Dropped => {
                return Ok((Status::Dropped, Attention::Quiet, leftover("dropped")));
            }
            TaskState::Open => {}
        }
        let others = |n: usize| match n {
            0 => String::new(),
            1 => " (+1 other attempt)".to_string(),
            n => format!(" (+{n} other attempts)"),
        };

        if let Some(run) = runs.iter().find(|r| !r.state.is_terminal()) {
            let id = run.id.clone();
            if let Some(&n) = asks.get(&run.id) {
                let what = if n == 1 {
                    "a question".to_string()
                } else {
                    format!("{n} questions")
                };
                return Ok((
                    Status::Asking { run: id, asks: n },
                    Attention::NeedsYou,
                    format!("{} is waiting on {what} from you", run.agent),
                ));
            }
            let stopping = run.state == RunState::Stopping;
            let verb = if stopping {
                "is stopping"
            } else {
                "is working on it"
            };
            let reason = format!("{} {verb}{}", run.agent, others(runs.len() - 1));
            return Ok((
                Status::Running { run: id, stopping },
                Attention::Working,
                reason,
            ));
        }

        let mut best: Option<(Verdict, &RunRow, CheckResults)> = None;
        for run in runs.iter().filter(|r| r.state == RunState::Finished) {
            let (v, results) = verdict(self.git, self.store, self.intent, task, run)?;
            if best.as_ref().is_none_or(|(b, _, _)| rank(v) < rank(*b)) {
                best = Some((v, run, results));
            }
        }
        if let Some((v, run, results)) = best {
            let more = others(runs.len() - 1);
            let reason = match v {
                Verdict::Verified => format!("ready for review, checks pass{more}"),
                Verdict::Failing => {
                    let failing: Vec<&str> = results
                        .iter()
                        .filter(|(_, s)| {
                            matches!(s, CheckStatus::Current { outcome, .. } | CheckStatus::Carried { outcome, .. } if *outcome != crate::store::CheckOutcome::Pass)
                        })
                        .map(|(n, _)| n.as_str())
                        .collect();
                    format!("ready for review, failing: {}{more}", failing.join(", "))
                }
                Verdict::Unverified => format!("ready for review, not verified yet{more}"),
                Verdict::Empty => format!("{} finished without changing anything{more}", run.agent),
                Verdict::Unknown => format!("finished, no snapshot recorded{more}"),
            };
            return Ok((
                Status::Review {
                    run: run.id.clone(),
                    verdict: v,
                },
                Attention::NeedsYou,
                reason,
            ));
        }

        if let Some(run) = runs.first() {
            let id = run.id.clone();
            if run.state == RunState::Interrupted {
                return Ok((
                    Status::Interrupted { run: id },
                    Attention::NeedsYou,
                    "run was interrupted; its partial work is kept".into(),
                ));
            }
            let detail = run.detail.clone().unwrap_or_else(|| "agent failed".into());
            let reason = format!("{} failed: {detail}", run.agent);
            return Ok((
                Status::Failed { run: id, detail },
                Attention::NeedsYou,
                reason,
            ));
        }

        let questions: Vec<String> = self
            .intent
            .open_questions_blocking(&task.id)
            .map(|q| q.id.clone())
            .collect();
        if !questions.is_empty() {
            let reason = format!("blocked on {}", questions.join(", "));
            return Ok((
                Status::BlockedByQuestion { questions },
                Attention::NeedsYou,
                reason,
            ));
        }
        let open_deps: Vec<String> = task
            .after
            .iter()
            .filter(|d| {
                self.intent
                    .tasks
                    .get(*d)
                    .is_some_and(|t| t.state == TaskState::Open)
            })
            .cloned()
            .collect();
        if !open_deps.is_empty() {
            let reason = format!("waiting on {}", open_deps.join(", "));
            return Ok((
                Status::BlockedByTasks { tasks: open_deps },
                Attention::Waiting,
                reason,
            ));
        }
        Ok((Status::Ready, Attention::Ready, "ready to start".into()))
    }
}

type CheckResults = Vec<(String, CheckStatus)>;

fn rank(v: Verdict) -> u8 {
    match v {
        Verdict::Verified => 0,
        Verdict::Unverified => 1,
        Verdict::Failing => 2,
        Verdict::Empty => 3,
        Verdict::Unknown => 4,
    }
}

/// Tasks that could start right now, in dependency order. This is the
/// deterministic answer to "what's next", so no model has to guess it.
pub fn ready_order(intent: &Intent) -> Vec<String> {
    let open: BTreeSet<&str> = intent
        .tasks
        .values()
        .filter(|t| t.state == TaskState::Open)
        .map(|t| t.id.as_str())
        .collect();
    intent
        .tasks
        .values()
        .filter(|t| t.state == TaskState::Open)
        .filter(|t| t.after.iter().all(|d| !open.contains(d.as_str())))
        .filter(|t| intent.open_questions_blocking(&t.id).next().is_none())
        .map(|t| t.id.clone())
        .collect()
}

/// Is any path in `changed` protected by policy?
pub fn protected_changes(intent: &Intent, changed: &[String]) -> Vec<String> {
    let p: Scope = intent.protected();
    changed.iter().filter(|c| p.contains(c)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(files: &[(&str, &str)]) -> Intent {
        Intent::from_files(
            files
                .iter()
                .map(|(p, c)| (p.to_string(), c.as_bytes().to_vec()))
                .collect(),
        )
    }

    const CFG: &str = "[checks.unit]\nrun = \"true\"\n[checks.idem]\nrun = \"true\"\n[checks.docs]\nrun = \"true\"\n";

    #[test]
    fn required_checks_follow_invariant_scope() {
        let i = intent(&[
            (".kitsu/kitsu.toml", CFG),
            (
                ".kitsu/tasks/t.md",
                "+++\nscope = [\"src/client/**\"]\nchecks = [\"unit\"]\n+++\n",
            ),
            (
                ".kitsu/invariants/idem.md",
                "+++\nscope = [\"src/client/**\"]\nchecks = [\"idem\"]\n+++\n",
            ),
            (
                ".kitsu/invariants/docs.md",
                "+++\nscope = [\"docs/**\"]\nchecks = [\"docs\"]\n+++\n",
            ),
            (
                ".kitsu/invariants/old.md",
                "+++\nstate = \"retired\"\nchecks = [\"docs\"]\n+++\n",
            ),
        ]);
        assert!(i.problems.is_empty(), "{:?}", i.problems);
        let t = &i.tasks["t"];
        let names = |v: Vec<RequiredCheck>| v.into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(required_checks(&i, t, None)), ["idem", "unit"]);
        // The change strayed into docs/: the docs invariant now applies.
        let changed = vec!["src/client/a.rs".to_string(), "docs/x.md".to_string()];
        assert_eq!(
            names(required_checks(&i, t, Some(&changed))),
            ["docs", "idem", "unit"]
        );
    }

    #[test]
    fn ready_order_respects_deps_and_questions() {
        let i = intent(&[
            (".kitsu/tasks/a.md", "+++\n+++\n"),
            (".kitsu/tasks/b.md", "+++\nafter = [\"a\"]\n+++\n"),
            (".kitsu/tasks/c.md", "+++\n+++\n"),
            (".kitsu/tasks/d.md", "+++\nstate = \"done\"\n+++\n"),
            (".kitsu/tasks/e.md", "+++\nafter = [\"d\"]\n+++\n"),
            (".kitsu/questions/q.md", "+++\nblocks = [\"c\"]\n+++\n"),
        ]);
        assert_eq!(ready_order(&i), ["a", "e"]);
    }

    #[test]
    fn protected_paths_include_intent_itself() {
        let i = intent(&[(".kitsu/kitsu.toml", "[protect]\npaths = [\"tests/**\"]\n")]);
        let changed = vec![
            ".kitsu/invariants/x.md".to_string(),
            "tests/t.rs".to_string(),
            "src/a.rs".to_string(),
        ];
        assert_eq!(
            protected_changes(&i, &changed),
            [".kitsu/invariants/x.md", "tests/t.rs"]
        );
    }
}
