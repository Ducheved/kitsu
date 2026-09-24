+++
title = "Order briefs so model prompt caches hit"
scope = ["crates/kitsu/src/brief.rs"]
checks = ["test"]
after = ["token-accounting"]
state = "done"
+++
Done differently than planned; see decision `prompt-cache`. The shared
prefix breaks in Claude Code's system prompt (it carries the per-run
worktree path) before the brief starts, so reordering the brief would buy
nothing. The claude preset now asks for a static system prompt, and the
brief budget is in estimated tokens. Measuring the hit rate is part of
live-agent-smoke.
