+++
title = "Kitsu has its own agent loop; external agents stay supported over ACP"
state = "accepted"
scope = ["crates/kitsu/src/agent/**", "crates/kitsu/src/runner.rs", "crates/kitsu/src/agents.rs"]
supersedes = ["acp-client-not-harness"]
rejected = [
  "Only external agents: through them Kitsu can only put advice into someone else's context. The compaction probe measured it: the brief sent as a message survived in 0 of 10 requests after Claude Code compacted",
  "Our loop as a separate ACP binary: the journal and the owner lock would sit across a process boundary, and the API key would travel through the environment",
  "Per-model shims and lenient argument parsing (what the researched harnesses do): a model that can't follow a closed schema gets the parser's error back and fails visibly",
  "An LLM summary for compaction in v1: Hermes measured verbatim assistant text plus a mechanical index beating it; ours is regenerated from the journal",
]
+++
`[agents.<name>] native = {...}` in agents.toml runs Kitsu's own loop in
the `kitsu run` process. Brain and host talk only in JSON-RPC strings, so
a brain elsewhere later needs a transport; the host, which touches your
files and runs checks, stays local.

What the loop gives that an external agent can't:
- the brief is a pinned system message, and the run's state (changes,
  where each required check stands, budget) is re-rendered as the last
  message of every request;
- `finish` is a request the checks answer; three refusals stop the run;
- every tool effect is journaled before it happens.

Claude Code, Codex and the rest keep working over ACP as before: the
decision that Kitsu owns state, evidence and review, not the agent,
stands for both.

Revisit if: the live comparison (same tasks, our loop against Claude Code)
shows no gain in verified runs or cost.
