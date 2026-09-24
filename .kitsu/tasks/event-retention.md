+++
title = "Prune agent chatter from runs that are long resolved"
scope = ["crates/kitsu/src/store.rs", "crates/kitsu/src/recover.rs"]
checks = ["test"]
+++
Events cost ~400 bytes each. At the stress profile's worst rate that is
gigabytes per busy day. Keep run state transitions, evidence, asks and
integrations forever; drop `agent.message`/`agent.tool` rows for runs
resolved more than 30 days ago. Never prune anything tied to a run that is
unresolved or an integration that isn't terminal.
