+++
title = "A local search index over code, rules, memory and past runs"
scope = ["crates/kitsu/src/index.rs", "crates/kitsu/src/cli.rs"]
checks = ["test"]
state = "done"
+++
SQLite FTS5, chunked by lines with identifier-aware tokens, rebuilt
incrementally by git blob id: git is the authority, the index is a cache
that knows how to invalidate itself. `kitsu search <query>`. No embeddings
until the eval says they help.

Measured on openai/codex (8,650 files, 80 MB of text): first index 5.6 s,
38 MB on disk (storing chunk text made it 202 MB; git already has the
text, so the index keeps positions only), warm search 25 to 35 ms, a
one-file commit re-indexes in about 0.2 s.
