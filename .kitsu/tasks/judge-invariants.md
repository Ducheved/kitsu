+++
title = "Flag invariants a change might break even when it doesn't touch their paths"
scope = ["crates/kitsu/src/status.rs", "crates/kitsu/src/integrate.rs", "crates/kitsu/src/judge.rs"]
checks = ["test"]
after = ["judge-core"]
+++
One batched call per review: for each active invariant, the probability
that this diff violates it. Above threshold, the invariant's checks become
required and the review shows it as "at risk". It can add requirements,
never remove them.
