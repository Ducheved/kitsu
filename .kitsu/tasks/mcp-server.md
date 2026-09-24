+++
title = "Let agents query Kitsu over MCP"
scope = ["crates/kitsu/src/mcp.rs", "crates/kitsu/src/runner.rs"]
checks = ["test"]
after = ["index-lexical", "memory-typed"]
state = "done"
+++
`kitsu mcp`: search, memory lookup, rules for a path, the run's brief, check
status. Passed to every agent in ACP `session/new`, so reading less of the
repository is an option the agent can take. Works the same under ACP v2,
which moves client tools to MCP.

Shipped read-only: `orient`, `search`, `rules_for`, `brief`, `memory`.
No mutating tools yet, so no effect can be left with an unknown outcome;
the first one needs the replay contract (Pi's `replay: never|safe`) and a
kill-during-call test. Whether agents read less with it is unmeasured
until a live run (live-agent-smoke).
