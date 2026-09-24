+++
title = "Measure retrieval before trusting it"
scope = ["crates/kitsu/src/index.rs", "crates/kitsu/examples/**", "fixtures/**"]
checks = ["test"]
after = ["index-lexical"]
state = "done"
+++
Queries with known answers over this repo and the fixtures: recall@5,
recall@20, latency, index time and size. The number decides whether
embeddings or a reranker are worth adding.

Result and gates: decision `lexical-index`. The eval runs as
`cargo run --release --example retrieval`.
