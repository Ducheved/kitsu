+++
title = "Agents never write to your checkout"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/integrate.rs", "crates/kitsu/src/workspace.rs"]
checks = ["test"]
decision = "process-per-run"
+++
A run works in its own worktree under the git common dir. The only things
that write your working tree are things you did: accepting (a fast-forward
git would refuse if it clobbered your changes), saving a file in the
editor, answering a question, closing a task.

Without this, "what's on disk" has two writers and neither your unsaved
buffer nor the agent's edit can be trusted.
