+++
title = "An architecture model checked against the code, and docs tasks"
scope = ["crates/kitsu/src/intent.rs", "crates/kitsu/src/arch.rs", "crates/kitsu/src/cli.rs", "crates/kitsu/src/brief.rs"]
checks = ["test"]
+++
One file per C4 element in `.kitsu/architecture/` (level, parent, paths,
uses, docs; the body is the doc). `kitsu arch check`: references and
levels resolve, every `paths` glob matches a file, every file matching
`cover` belongs to exactly one component, and a vacuity guard. Later,
behind their gates: declared `uses` against real dependencies, structural
staleness (file set or dependencies changed, not any line), a C4 slice in
the brief, `kind = "docs"` tasks. LikeC4 and Mermaid as exports only.

Gate for the base: the first honest model of Kitsu itself catches a real
coverage or path error. It already did once in the research prototype
(`index.rs` had no component).
