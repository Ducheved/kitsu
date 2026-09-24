+++
title = "Brief compiler"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/brief.rs"]
uses = ["rules", "evidence", "status", "store", "gitio"]
+++
The deterministic, budgeted brief for a run: task, rules in scope, earlier
attempts, memory (advisory, last) and what was left out and why.
Same inputs, same bytes: that is what keeps agents' prompt caches warm.
