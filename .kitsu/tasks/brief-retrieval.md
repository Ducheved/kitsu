+++
title = "Put the most relevant code in the brief, within a token budget"
scope = ["crates/kitsu/src/brief.rs", "crates/kitsu/src/index.rs"]
checks = ["test"]
after = ["index-lexical", "brief-cache-friendly", "retrieval-eval"]
+++
A "Relevant code" section: top chunks for the task, cited by path and line,
cut to budget, with what was left out and how to search for more.
