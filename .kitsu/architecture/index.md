+++
title = "Code index"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/index.rs"]
uses = ["gitio", "shared"]
+++
Lexical search over a commit (contentless FTS5 keyed by blob id). A
rebuildable cache: deleting `index.db` loses nothing.
