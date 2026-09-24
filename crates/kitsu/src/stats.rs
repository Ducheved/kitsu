//! Where the tokens went.
//!
//! Numbers come from what agents reported over ACP, which is optional, so
//! every total says how many runs it covers. A run that reported nothing is
//! counted as "not reported", never as zero tokens. Brief sizes are
//! Kitsu's own, estimated from bytes, and labeled as estimates.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::store::{RunRow, Usage};

/// Rough tokens for text we produced ourselves. Good enough to compare
/// briefs with each other; not a bill.
pub fn estimate_tokens(bytes: u64) -> u64 {
    bytes.div_ceil(4)
}

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct Totals {
    pub runs: u64,
    /// Runs whose agent reported any token numbers.
    pub reported: u64,
    pub input: u64,
    pub output: u64,
    pub cached_read: u64,
    /// Sum of `spent()` over reporting runs.
    pub spent: u64,
    /// Cost by currency, for runs that reported one.
    pub cost: BTreeMap<String, f64>,
}

impl Totals {
    fn add(&mut self, u: Option<&Usage>) {
        self.runs += 1;
        let Some(u) = u.filter(|u| u.spent().is_some() || u.cost.is_some()) else {
            return;
        };
        self.reported += 1;
        self.input += u.input.unwrap_or(0);
        self.output += u.output.unwrap_or(0);
        self.cached_read += u.cached_read.unwrap_or(0);
        self.spent += u.spent().unwrap_or(0);
        if let (Some(c), Some(cur)) = (u.cost, &u.currency) {
            *self.cost.entry(cur.clone()).or_default() += c;
        }
    }

    /// Share of input served from the provider's prompt cache, when the
    /// agent reported both numbers. Assumes input excludes cache reads, as
    /// Anthropic reports it; ACP doesn't say, so treat this as indicative.
    pub fn cache_share(&self) -> Option<f64> {
        (self.input > 0 && self.cached_read > 0)
            .then(|| self.cached_read as f64 / (self.input + self.cached_read) as f64)
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Stats {
    pub all: Totals,
    pub by_agent: BTreeMap<String, Totals>,
    /// Runs that were accepted, discarded, or not decided yet.
    pub by_outcome: BTreeMap<String, Totals>,
    /// The tasks that took the most tokens, most first.
    pub top_tasks: Vec<(String, Totals)>,
    /// Estimated brief tokens: median and largest, over runs that have one.
    pub brief_median: Option<u64>,
    pub brief_max: Option<(String, u64)>,
}

/// `brief_bytes` returns the size of a run's brief if it has one.
pub fn compute(runs: &[RunRow], brief_bytes: impl Fn(&RunRow) -> Option<u64>) -> Stats {
    let mut s = Stats::default();
    let mut by_task: BTreeMap<String, Totals> = BTreeMap::new();
    let mut briefs: Vec<(u64, &str)> = Vec::new();
    for r in runs {
        let u = r.usage.as_ref();
        s.all.add(u);
        s.by_agent.entry(r.agent.clone()).or_default().add(u);
        let outcome = r.resolution.clone().unwrap_or_else(|| "undecided".into());
        s.by_outcome.entry(outcome).or_default().add(u);
        by_task.entry(r.task.clone()).or_default().add(u);
        if let Some(b) = brief_bytes(r) {
            briefs.push((estimate_tokens(b), &r.task));
        }
    }
    let mut tasks: Vec<(String, Totals)> =
        by_task.into_iter().filter(|(_, t)| t.spent > 0).collect();
    tasks.sort_by(|a, b| b.1.spent.cmp(&a.1.spent).then_with(|| a.0.cmp(&b.0)));
    tasks.truncate(5);
    s.top_tasks = tasks;
    briefs.sort();
    s.brief_median = briefs.get(briefs.len() / 2).map(|b| b.0);
    s.brief_max = briefs.last().map(|b| (b.1.to_string(), b.0));
    s
}

/// 1234 -> "1.2k", 3400000 -> "3.4M".
pub fn short(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => trim(n as f64 / 1e3, "k"),
        _ => trim(n as f64 / 1e6, "M"),
    }
}

fn trim(v: f64, unit: &str) -> String {
    let s = if v >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    };
    format!("{}{unit}", s.strip_suffix(".0").unwrap_or(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::RunState;

    fn run(
        id: &str,
        task: &str,
        agent: &str,
        resolution: Option<&str>,
        usage: Option<Usage>,
    ) -> RunRow {
        RunRow {
            id: id.into(),
            task: task.into(),
            agent: agent.into(),
            base: String::new(),
            branch: String::new(),
            worktree: String::new(),
            state: RunState::Finished,
            stop_reason: None,
            detail: None,
            cancel_requested: false,
            owner: None,
            pid: None,
            brief: Some(format!("b-{id}")),
            snapshot: None,
            resolution: resolution.map(Into::into),
            from_run: None,
            note: None,
            created_at: 0,
            ended_at: None,
            snapshot_tree: None,
            changed: None,
            usage,
        }
    }

    fn used(input: u64, output: u64, cached: Option<u64>, cost: Option<f64>) -> Usage {
        Usage {
            input: Some(input),
            output: Some(output),
            cached_read: cached,
            cost,
            currency: cost.map(|_| "USD".into()),
            ..Usage::default()
        }
    }

    #[test]
    fn silent_runs_are_counted_but_not_as_zero() {
        let runs = [
            run(
                "a",
                "t1",
                "claude",
                Some("accepted"),
                Some(used(1000, 200, Some(3000), Some(0.5))),
            ),
            run(
                "b",
                "t1",
                "claude",
                Some("discarded"),
                Some(used(500, 100, None, Some(0.25))),
            ),
            run("c", "t2", "codex", None, None),
            // Context occupancy alone is not spending.
            run(
                "d",
                "t2",
                "codex",
                None,
                Some(Usage {
                    context_used: Some(9000),
                    ..Usage::default()
                }),
            ),
        ];
        let s = compute(&runs, |_| Some(4000));
        assert_eq!(s.all.runs, 4);
        assert_eq!(s.all.reported, 2);
        assert_eq!(s.all.spent, 1800);
        assert_eq!(s.all.cost["USD"], 0.75);
        assert_eq!(s.by_agent["codex"].reported, 0);
        assert_eq!(s.by_outcome["discarded"].spent, 600);
        assert_eq!(
            s.top_tasks.len(),
            1,
            "a task nobody reported on isn't ranked"
        );
        assert_eq!(s.brief_median, Some(1000));
        // cache share: 3000 cached of 1500 + 3000 input seen
        let share = s.all.cache_share().expect("share");
        assert!((share - 3000.0 / 4500.0).abs() < 1e-9);
    }

    #[test]
    fn short_numbers() {
        assert_eq!(short(999), "999");
        assert_eq!(short(1000), "1k");
        assert_eq!(short(1234), "1.2k");
        assert_eq!(short(250_000), "250k");
        assert_eq!(short(3_400_000), "3.4M");
    }
}
