+++
title = "Run the retry-storm fixture end to end with a real agent"
scope = ["crates/kitsu/src/acp.rs", "crates/kitsu/src/runner.rs", "crates/kitsu/src/agents.rs"]
checks = ["test"]
+++
Everything so far is verified against `kitsu-test-agent`, which speaks ACP
exactly the way we wrote it. Run `claude`, `codex` and `gemini` adapters on
fixtures/retry-storm, three trials each, and record: did the brief's
invariant survive (idempotency check), permission requests seen, protocol
surprises, wall time, cost.

Needs an API budget first.
