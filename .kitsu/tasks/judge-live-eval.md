+++
title = "Measure whether typed judgments help, with real calls"
scope = ["crates/kitsu/src/judge.rs"]
checks = ["test"]
after = ["judge-permissions", "judge-invariants", "judge-rerank"]
+++
False-allow and false-ask rates on a labeled set of tool calls; guard
flags against planted violations; tokens saved by reranking versus recall
lost.
