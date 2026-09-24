+++
title = "Accept and recovery"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/integrate.rs", "crates/kitsu/src/recover.rs"]
uses = ["evidence", "status", "rules", "runs", "store", "workspace", "gitio", "shared"]
+++
Squashes a run, verifies the combined tree, fast-forwards with a
compare-and-swap. Recovery settles runs whose owner died.
