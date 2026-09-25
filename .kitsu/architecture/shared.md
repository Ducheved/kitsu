+++
title = "Errors and utilities"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/error.rs", "crates/kitsu/src/util.rs", "crates/kitsu/src/proc.rs"]
uses = []
+++
Leaf. Nothing here knows about runs, git or intent. `proc.rs` is the
shell every command runs under and how a process tree is stopped.
