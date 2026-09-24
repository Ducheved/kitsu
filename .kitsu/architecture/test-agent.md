+++
title = "Scripted ACP agent"
level = "container"
parent = "kitsu"
technology = "Rust binary"
paths = ["crates/kitsu/src/bin/**"]
+++
A deterministic agent for tests and the demo. Plays a TOML script of ACP
messages; never calls a model.
