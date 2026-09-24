+++
title = "Kitsu is an ACP client; the agent loop belongs to the agent"
rejected = [
  "Our own agent harness: we'd be one more harness, chasing model changes every month, and the useful part (state, evidence, review) would be locked to it",
  "Only an MCP server for existing agents: gives agents tools but no lifecycle, no isolation, no receipts; the human stays the scheduler",
  "Agents coordinating through chat messages: prose is not a synchronization primitive",
]
+++
Kitsu launches agents over ACP (v1, stdio), gives each a compiled brief,
answers permission requests, and records what happens. Which model, which
prompting strategy, how it plans: the agent's business, and replaceable.

The boundary with the agent is deliberately small and survives the v2 draft:
we don't offer `fs/*` or `terminal/*` (v2 removes them); agents use their
own tools inside an isolated worktree. They talk back through files
(`.kitsu/questions/`, `.kitsu/decisions/`) that any agent can write.

Revisit if: ACP stalls or fragments; or we need guarantees about individual
tool calls that only a harness we run could give.
