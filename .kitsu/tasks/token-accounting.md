+++
title = "Count what each run costs in tokens"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/store.rs", "crates/kitsu/src/brief.rs", "app/src/**"]
checks = ["test", "ui"]
+++
Record the agent's reported usage (ACP usage_update) per run, the brief's
size, and show tokens per run and per task. Without the number, "saves
tokens" is a slogan.
