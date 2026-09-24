+++
title = "Task status is computed, never stored"
scope = ["crates/kitsu/src/status.rs", "crates/kitsu/src/store.rs", "crates/kitsu/src/runner.rs"]
checks = ["test"]
decision = "intent-in-git"
+++
Ready, blocked, running, in review, verified: all derived from intent
files, runs and evidence on every read. The one stored bit is `state` in a
task file, and only a human sets it (usually by accepting).

An agent must not be able to write "done". If a status column appears,
something will eventually update it without the evidence to back it.
