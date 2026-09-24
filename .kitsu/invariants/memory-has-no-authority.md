+++
title = "Memory changes what an agent is told, never what is required, ready, allowed or done"
scope = ["crates/kitsu/src/status.rs", "crates/kitsu/src/integrate.rs", "crates/kitsu/src/check.rs", "crates/kitsu/src/run.rs", "crates/kitsu/src/recover.rs", "crates/kitsu/src/store.rs", "crates/kitsu/src/memory.rs", "crates/kitsu/src/intent.rs"]
checks = ["memory-no-authority", "test"]
decision = "intent-in-git"
+++
Notes are quoted in the brief, below the rules, with their source, and
labeled as something that can be wrong. Status, required checks, `kitsu
next`, permission decisions, verdicts and accept never read them. A note
can't carry `checks`, `after`, `blocks`, `state` or anything executable;
such a note doesn't load and is reported. If a note seems to require
something, it belongs in an invariant or a task.

Every harness studied (Hermes, Letta, Cline, Grok, AG2, LangGraph) lets
memory or its equivalent steer execution somewhere: "authoritative" recall,
skills with scripts written by a background model, routing read from
context variables. Pinned by `tests/separation.rs`.
