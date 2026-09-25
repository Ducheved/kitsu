+++
title = "Typed judgments"
level = "component"
parent = "cli"
technology = "Rust, TypeSafe System One over HTTPS"
paths = ["crates/kitsu/src/judge.rs"]
uses = ["store", "shared", { to = "typesafe", why = "yes/no, choice and score questions with probabilities" }]
+++
Asks a yes/no, a choice from a closed set or a score about some data, and
stores every judgment with the hash of its inputs, the answers and
probabilities, the model, latency and tokens. Off unless `[judge]` is in
agents.toml; off, failed, slow or malformed, the answer is unknown.

Used by the native agent's host (`--policy triage`), before a shell command
would wait for you.

Must never: turn unknown into yes, no or a default; put text computed at
run time (tool arguments, file contents) in a question's instructions;
write or log the key; override a rule that refuses or always asks.
