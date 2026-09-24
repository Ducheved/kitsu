+++
title = "Suggest a cheaper model when the task is simple"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/judge.rs", "app/src/**"]
checks = ["test"]
after = ["judge-core", "token-accounting"]
+++
A Choice over the model tiers an agent exposes (ACP config option with
category "model"). Shown as a suggestion with its confidence; applied only
when the person turned it on.
