+++
title = "Stop what a command left running on Windows too"
scope = ["crates/kitsu/src/proc.rs", "crates/kitsu/src/agent/host.rs", "crates/kitsu/src/runner.rs", "crates/kitsu/src/check.rs"]
checks = ["test"]
+++
On Unix a command's process group is killed when its call ends. On Windows
Kitsu kills the tree with `taskkill /T` while the root is alive, but once sh
has exited what it started in the background (`npm run dev &`) can't be
found, so it keeps running; the model is told so. Put each command, agent
and check in a Job Object with kill-on-close so the OS ends them together.
The workspace forbids `unsafe`, so this needs a small maintained crate that
wraps Job Objects safely (compare `process-wrap`, `win32job`), or a decision
to allow one reviewed `unsafe` block.

Done when `a_background_process_does_not_hold_the_call` asserts on Windows
what it asserts on Unix: the leftover process is gone.
