+++
title = "Status, digest and stats"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/status.rs", "crates/kitsu/src/digest.rs", "crates/kitsu/src/stats.rs"]
uses = ["evidence", "rules", "store", "gitio"]
+++
Everything derived: what's ready, blocked or needs you, what changed since
you last looked, where the tokens went. Stores nothing of its own.
