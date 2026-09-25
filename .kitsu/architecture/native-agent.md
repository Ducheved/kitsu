+++
title = "Kitsu's own agent loop"
level = "component"
parent = "cli"
technology = "Rust; Chat Completions, Anthropic Messages, OpenAI Responses"
paths = ["crates/kitsu/src/agent/**"]
uses = ["runs", "evidence", "status", "rules", "agent-tools", "store", "workspace", "gitio", "shared", { to = "model", why = "chat completions, messages or responses over HTTPS; the OpenRouter login" }, { to = "keychain", why = "the key `kitsu login` keeps" }]
+++
A brain (the loop that calls the model, `brain.rs`) and a host (tools,
journal, policy, `finish` verification, `host.rs`) that talk only in
JSON-RPC strings (`protocol.rs`), so the brain can later run elsewhere.
The providers (`provider.rs`, `anthropic.rs`, `responses.rs`) render one
conversation into their wire format; `login.rs` is `kitsu login`.

Must never: let the brain touch the store, git or files directly; mark a
run verified on the model's word; write the API key anywhere; repeat a
tool effect whose outcome is unknown; keep a login key anywhere but the
OS keychain.
