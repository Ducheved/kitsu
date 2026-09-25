+++
title = "Kitsu's own agent loop"
level = "component"
parent = "cli"
technology = "Rust, OpenAI-compatible Chat Completions"
paths = ["crates/kitsu/src/agent/**"]
uses = ["runs", "evidence", "status", "rules", "agent-tools", "store", "workspace", "gitio", "shared", "judge", { to = "model", why = "chat completions over HTTPS" }]
+++
A brain (the loop that calls the model, `brain.rs`) and a host (tools,
journal, policy, `finish` verification, `host.rs`) that talk only in
JSON-RPC strings (`protocol.rs`), so the brain can later run elsewhere.

Must never: let the brain touch the store, git or files directly; mark a
run verified on the model's word; write the API key anywhere; repeat a
tool effect whose outcome is unknown.
