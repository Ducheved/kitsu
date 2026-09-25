+++
title = "Run lifecycle"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/acp.rs", "crates/kitsu/src/agents.rs"]
uses = ["brief", "evidence", "status", "rules", "agent-tools", "native-agent", "store", "workspace", "gitio", "shared"]
+++
Prepares the worktree and brief, starts the agent with an allowlisted
environment, speaks ACP, records every event, answers permissions by
policy and hands anything else to you.
Seam: the ACP client (`acp.rs`) knows nothing of runs.
