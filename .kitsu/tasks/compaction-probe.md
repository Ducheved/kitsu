+++
title = "Find out whether each agent keeps the brief through its own compaction"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/agents.rs", "crates/kitsu/examples/**", "fixtures/**"]
checks = ["test"]
+++
A local stub model server (OpenAI- and Anthropic-compatible) that records
every request and scripts tool calls until the agent compacts. Point
Claude Code, OpenCode and Codex at it, run a Kitsu task, and check that
every open invariant's text is still in the post-compaction requests.
No API spend.

Done when the result per agent is recorded (N ≥ 5 sessions each). Wire
rules into an agent's own instruction channel only where the default loses
them. Also records whether OpenCode survives Kitsu refusing
`fs/write_text_file`, and whether agents call `kitsu mcp` tools at all.

Claude Code: done (decision `rules-channel`). The brief alone was gone in
0/10 post-compaction requests; with the anchor in the system prompt,
10/10. OpenCode and Codex still to run; they need their own stub endpoints
(OpenAI-style) in `fixtures/stub-model/server.py`.
