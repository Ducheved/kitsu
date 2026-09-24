+++
title = "Flag checks a change might break even when it doesn't touch what they guard"
scope = ["crates/kitsu/src/status.rs", "crates/kitsu/src/integrate.rs", "crates/kitsu/src/judge.rs"]
checks = ["test"]
after = ["judge-core"]
+++
One batched call per review: for each check with a `why`, the probability
that this diff breaks what it protects. Above threshold, the check becomes
required and the review shows it as "at risk". It can add requirements,
never remove them.
