+++
title = "Kitsu's own loop marks cache breakpoints for Anthropic models; the ledger stays after the last mark"
scope = ["crates/kitsu/src/agent/provider.rs", "crates/kitsu/src/agent/context.rs"]
state = "proposed"
rejected = [
  "Top-level automatic `cache_control`: its breakpoint lands on the last block, the ledger, which changes on every request, so each request writes a cache nothing reads",
  "Marks for every model: OpenAI, DeepSeek and Gemini cache on their own, and a plain OpenAI-compatible server may refuse a field it doesn't know",
  "A setting in agents.toml: nothing to choose; the provider decides whether marks are needed",
]
+++
Anthropic models cache only what the request marks with `cache_control`
(OpenRouter passes the marks through). Without marks every turn paid for the
whole prompt: the live smoke on `retry-storm` with
`anthropic/claude-sonnet-5` showed `cached: 0` on all five turns.

`OpenAiChat::body` marks three messages when the model id is
`anthropic/…` (OpenRouter's naming): the harness prompt (same for every
run), the brief (same for the whole run) and the message right before the
ledger (the conversation so far). The ledger is re-rendered every request
and stays unmarked after it, as `context` already arranges.

Measured on the same fixture and model: turns 2–5 read 91–96% of the prompt
from cache, and the run cost $0.032 instead of $0.064.

Not yet measured: a hit across runs on the harness and tool list (needs two
runs within the 5-minute TTL, and a prefix above the model's minimum).
Anthropic looks back 20 blocks from a mark for an earlier entry, so a turn
with more than ~20 tool calls misses the conversation part once; the
system marks still hit.

Revisit if: another provider behind the same API needs marks, or turns
with many parallel calls become common.
