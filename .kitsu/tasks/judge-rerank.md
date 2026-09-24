+++
title = "Drop irrelevant context before it costs tokens"
scope = ["crates/kitsu/src/brief.rs", "crates/kitsu/src/judge.rs"]
checks = ["test"]
after = ["judge-core", "brief-retrieval"]
+++
Batch-score retrieved chunks and optional memory notes for relevance to the
task in one call; keep what clears the bar. Report tokens saved against
the unfiltered brief.
