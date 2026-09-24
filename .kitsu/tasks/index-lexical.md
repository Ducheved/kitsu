+++
title = "A local search index over code, rules, memory and past runs"
scope = ["crates/kitsu/src/index.rs", "crates/kitsu/src/cli.rs"]
checks = ["test"]
+++
SQLite FTS5, chunked by lines with identifier-aware tokens, rebuilt
incrementally by git blob id: git is the authority, the index is a cache
that knows how to invalidate itself. `kitsu search <query>`. No embeddings
until the eval says they help.
