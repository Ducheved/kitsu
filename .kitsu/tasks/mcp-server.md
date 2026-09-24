+++
title = "Let agents query Kitsu over MCP"
scope = ["crates/kitsu/src/mcp.rs", "crates/kitsu/src/runner.rs"]
checks = ["test"]
after = ["index-lexical", "memory-typed"]
+++
`kitsu mcp`: search, memory lookup, rules for a path, the run's brief, check
status. Passed to every agent in ACP `session/new`, so reading less of the
repository is an option the agent can take. Works the same under ACP v2,
which moves client tools to MCP.
