+++
title = "The task graph is a promise, execution state is a receipt, memory is a rumor with a source"
scope = ["crates/kitsu/src/intent.rs", "crates/kitsu/src/status.rs", "crates/kitsu/src/memory.rs", "crates/kitsu/src/brief.rs", "crates/kitsu/src/mcp.rs", "crates/kitsu/src/store.rs"]
rejected = [
  "A memory bank or agent-maintained progress file (Cline): plan state in memory makes agents resume stale work (Hermes patched this with prompt text for months)",
  "Memory that can carry procedures or scripts (Letta, Hermes skills): a background model's guess becomes executed code",
  "A background writer that merges without a human (Letta auto, Hermes default): nothing measures whether what it wrote is true",
  "Routing or readiness read from state any participant can write (AG2 context_vars, LangGraph update_state as a node)",
  "A static AND/OR plan tree or MAGE-style execution tree now: attempts are separate runs with lineage, and no failure has been seen that needs more",
]
+++
Three things, kept apart:

- **Intent** (`.kitsu/tasks`, invariants, decisions, questions) says what
  should happen and what is required. Written by people, or proposed by
  agents and accepted through review.
- **Execution state** (`state.db`: runs, events, evidence, asks) says what
  happened, to which tree. Written only by Kitsu, through the reducer.
  Status is derived from it and from intent, never stored.
- **Memory** (`.kitsu/memory`, personal notes) says what earlier work
  believed. It changes what an agent is told, never what is required,
  ready, allowed or done. Staleness and supersession are derived.

Allowed flows: intent → runs; execution → intent only on a human command
(accept-and-close, answer); execution → brief as machine-recorded facts
only; memory → brief, labeled as possibly wrong; execution → memory only
as a proposed file. Advisory inputs (memory, judgments, retrieval) may add
a requirement or a warning, never remove one.

Enforced by invariants `memory-has-no-authority`,
`task-files-hold-intent-only` and `run-row-is-the-fold-of-its-events`.

A "Dreamer" (background consolidation) is allowed only as an ordinary run
that proposes a diff under `.kitsu/memory/` and `.kitsu/questions/`,
grounded in check transitions and human answers (never in briefs or
earlier notes, to avoid an echo chamber), and only after it beats the
baseline of agents writing notes during their own runs. The first version
is deterministic (`kitsu memory doctor`) and waits for ≥ 20 real notes.
