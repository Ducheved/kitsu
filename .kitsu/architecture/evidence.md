+++
title = "Checks and evidence"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/check.rs"]
uses = ["rules", "store", "gitio", "workspace", "shared"]
+++
Runs checks and binds each result to the tree hash it ran on. A result
never outlives its tree; not run stays "unverified", never "passed".
