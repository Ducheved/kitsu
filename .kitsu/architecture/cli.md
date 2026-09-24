+++
title = "kitsu (CLI and run supervisor)"
level = "container"
parent = "kitsu"
technology = "Rust binary"
paths = ["crates/kitsu/src/*.rs"]
uses = [{ to = "agent", why = "spawns it, speaks ACP" }, { to = "git-repo", why = "worktrees, snapshots, fast-forward accept" }, { to = "state-db", why = "runs, evidence, events" }]
+++
The library and the binary. One `kitsu run` process owns one run for its
whole life; the desktop app starts the same processes.
