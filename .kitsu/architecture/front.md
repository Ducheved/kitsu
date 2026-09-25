+++
title = "CLI front end"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/cli.rs", "crates/kitsu/src/main.rs", "crates/kitsu/src/lib.rs"]
uses = ["runs", "integration", "brief", "status", "evidence", "index", "agent-tools", "model-check", "native-agent", "rules", "store", "workspace", "projects", "gitio", "shared"]
+++
clap commands, human output and `--json`. No logic of its own worth
testing apart from the commands it calls.
