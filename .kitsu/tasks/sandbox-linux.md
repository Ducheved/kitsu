+++
title = "Contain agent processes on Linux with bubblewrap"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/agents.rs"]
checks = ["test"]
+++
Worktree writable, repo `.git` read-only except the run's own worktree
metadata, home directory hidden except what the agent's own config needs,
network allowed (the agent calls its model) but visible in the UI. Detect
the Ubuntu 24.04 userns restriction and say what to install instead of
silently running unconfined.
