+++
title = "Run the retry-storm fixture end to end with a real agent"
scope = ["crates/kitsu/src/acp.rs", "crates/kitsu/src/runner.rs", "crates/kitsu/src/agents.rs"]
checks = ["test"]
+++
Everything so far is verified against `kitsu-test-agent`, which speaks ACP
exactly the way we wrote it. Run `claude`, `codex` and `gemini` adapters on
fixtures/retry-storm, three trials each, and record: did the brief's
guarded rule survives (idempotency check), permission requests seen, protocol
surprises, wall time, cost (`kitsu stats`).

For claude, run two tasks back to back and check the second reports cache
reads on the system prompt (decision `prompt-cache`). If it doesn't, the
`excludeDynamicSections` switch isn't doing what its docs say.

Needs an API budget first.
