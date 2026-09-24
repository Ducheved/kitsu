+++
title = "Agent tools (MCP)"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/mcp.rs"]
uses = ["rules", "evidence", "status", "index", "store", "workspace", "gitio", "shared"]
+++
Kitsu's read-only tools for the agent of a run, over MCP (stdio):
orient, search, rules_for, brief, memory. Results are bounded. Must never
grow a tool that writes, runs commands or changes state.
