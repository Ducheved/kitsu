+++
title = "Measure how often scope matching misses a rule a change breaks"
scope = ["crates/kitsu/src/scope.rs", "crates/kitsu/src/brief.rs", "crates/kitsu/src/status.rs"]
checks = ["test"]
after = ["live-agent-smoke"]
+++
Path scopes miss a change in `api/` that breaks an ownership rule scoped to
`services/project/`. Before adding symbol-level scopes (LSP references,
grep for the owning type), collect cases from real runs and see whether the
combined-result checks already catch them.
