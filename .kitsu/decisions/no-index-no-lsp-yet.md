+++
title = "No embeddings index and no LSP in the first cut"
rejected = [
  "A vector DB up front: Agent Retrieval Bench (2026) found no retrieval family wins across tasks, and agents already bring grep",
  "Always-on LSP: OpenCode turned it off by default; its value over compiler and tests for our queries is unmeasured",
]
+++
The brief is built from explicit links: scopes on tasks, invariants and
decisions, matched by path. It's deterministic, explainable ("included
because task scope X overlaps Y") and cheap. The known gap: a change that
breaks a rule without touching its paths isn't caught by scope matching.
Checks on the combined result are the backstop.

Revisit when a retrieval experiment on real tasks shows a gain worth the
index's invalidation and memory cost.
