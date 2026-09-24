+++
title = "Send the rules through the agent's system prompt where it has one; the first message doesn't survive compaction"
scope = ["crates/kitsu/src/brief.rs", "crates/kitsu/src/runner.rs", "crates/kitsu/src/agents.rs"]
rejected = [
  "Only the brief as the first prompt: measured, the rules are gone after Claude Code compacts",
  "Writing a CLAUDE.md into the worktree: it would show up in the agent's diff and in review",
  "A CLAUDE.md in a parent directory of the worktree: relies on undocumented lookup order, and the main checkout is already a parent",
  "Kitsu summarising or compacting for the agent: not our layer (acp-client-not-harness)",
]
+++
Measured with the compaction probe (`fixtures/stub-model/`): real Claude
Code (claude-agent-acp 0.81.2, Agent SDK 0.3.280) run by Kitsu against a
scripted model that forces a context overflow. After Claude Code
compacted, the requests carried its summary, re-read files and system
reminders, and none of the brief: the invariant text was in 0 of 10
post-compaction requests (5 sessions). With the brief's anchor appended to
the system prompt (`_meta.systemPrompt.append`, which the adapter forwards
to the SDK), 10 of 10, and the task title and "Done means" with it.

So every brief now has an `anchor` (task, done means, what must hold,
where the full brief is: about 300 tokens here), and the runner appends it
to the system prompt of agents whose `_meta` configures one. It's recorded
in the run's `run.brief` event, since it is part of what the agent was
told.

Cost: the anchor is per task, so the large system block misses the prompt
cache once per run (about 7k tokens for Claude Code); the tool list and the
first system block before it still hit, and within a run it's stable.

Codex keeps user messages verbatim through compaction by design (up to 20k
tokens); OpenCode and Pi summarise all but a recent tail. Neither is
measured yet; wire a channel for them only if the probe shows the rules
lost.

A side finding: an agent inherits Kitsu's environment. The first probe run
passed the host's session settings and tokens through to Claude Code
(task `agent-env`).
