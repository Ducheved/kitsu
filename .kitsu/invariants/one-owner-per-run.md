+++
title = "Each run has exactly one live owner"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/recover.rs", "crates/kitsu/src/workspace.rs", "crates/kitsu/src/store.rs"]
checks = ["test"]
decision = "process-per-run"
+++
The process that created a run holds its owner lock until it exits. Only
that process talks to the agent. Recovery touches a run only after the lock
is provably free, so two processes never both think they're in charge.
Liveness comes from the OS lock, not from heartbeat writes.
