+++
title = "Store and run reducer"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/store.rs", "crates/kitsu/src/run.rs"]
uses = ["shared"]
+++
All SQL, and the pure run-state reducer the run row is the fold of.
Writes are `BEGIN IMMEDIATE`; a run row changes only by appending events.
Seam: nothing else opens `state.db`.
