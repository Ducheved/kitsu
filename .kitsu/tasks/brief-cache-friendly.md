+++
title = "Order briefs so model prompt caches hit"
scope = ["crates/kitsu/src/brief.rs"]
checks = ["test"]
after = ["token-accounting"]
+++
Repository-wide rules first and byte-stable across runs, task-specific parts
last. Budget in estimated tokens, not characters. Measure prefix stability
across consecutive runs of different tasks.
