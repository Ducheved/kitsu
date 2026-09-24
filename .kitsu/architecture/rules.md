+++
title = "Intent and memory"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/intent.rs", "crates/kitsu/src/scope.rs", "crates/kitsu/src/memory.rs"]
uses = ["shared", "gitio"]
+++
Parses `.kitsu/` (tasks, decisions, questions, memory,
architecture) from a directory or a commit with the same code, and scopes.
Memory freshness is derived from git history. Memory never reaches status,
checks, runs or accept.
