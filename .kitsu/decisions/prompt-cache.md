+++
title = "Get prompt-cache hits from the agent's system prompt, not from reordering the brief"
scope = ["crates/kitsu/src/brief.rs", "crates/kitsu/src/agents.rs"]
rejected = [
  "Rules before the task in the brief, for a shared prefix: the prefix already breaks earlier, in the agent's system prompt, so this buys nothing and puts the task after the rules",
  "A custom system prompt for Claude: we'd own Claude Code's prompt and drift from it",
]
+++
Provider prompt caches match on a byte-identical prefix. For a run, that
prefix is the agent's system prompt and tool list, then our brief.

Claude Code's default system prompt includes the working directory and git
status (Claude Agent SDK 0.3.282, `sdk.d.ts`, `excludeDynamicSections`).
Every Kitsu run has its own worktree, so the prompt differs on every run and
the cache misses on the whole system prompt and tool list, which is the
largest fixed cost of a turn. Someone using Claude Code in one directory
doesn't pay this; Kitsu's isolation causes it.

So the `claude` preset sends `_meta.systemPrompt.excludeDynamicSections =
true` on `session/new` (the claude-agent-acp adapter forwards it). The SDK
moves those sections into the first user message. The brief states the
worktree path itself. Other agents get no `_meta`; ACP says agents must not
assume anything about keys they don't know, and we don't know what they do
with it.

The brief keeps the task first. Its size is the lever we control, and the
budget is in estimated tokens so it compares with what `kitsu stats` shows.

Not yet measured: whether the second of two runs in a row reports cache
reads. That needs a real agent (live-agent-smoke).

Revisit if: Codex or Gemini expose a similar switch, or measurements show
the hit rate doesn't move.

Update: the rules now ride at the end of the system prompt (decision
`rules-channel`), so the large system block is per task and misses once
per run; the tool list and the first system block still match across
runs.
