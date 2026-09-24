+++
title = "A run's stored state is where its recorded transitions end"
state = "accepted"
scope = ["crates/kitsu/src/store.rs", "crates/kitsu/src/run.rs", "crates/kitsu/src/recover.rs", "crates/kitsu/src/runner.rs"]
+++
Every change to a run's state goes through `Store::apply_run_event`, which
runs the reducer and records the transition. So the recorded `run.state`
events form a chain from `starting`, and it ends at the state in the row.
A shortcut that writes the row directly would leave the two disagreeing,
and recovery and review would each believe a different story.

`Store::run_history_mismatches` checks the chain. Every e2e scenario runs
it on every run when it finishes (crash, kill, duplicate answer, ignored
cancel, accept races), and a unit test proves a direct UPDATE is caught.
Runs also record the Kitsu version they were started under.

Enforced by `test`.
