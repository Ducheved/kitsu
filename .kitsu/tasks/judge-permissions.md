+++
title = "Triage agent tool calls by risk before asking you"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/judge.rs"]
checks = ["test"]
after = ["judge-core"]
+++
AutoMode, but a human stays in the loop: low-risk calls inside the worktree
are allowed, risky ones come to you with the probability shown, and if the
judge fails the call comes to you. Tool arguments are data, never
instructions.
