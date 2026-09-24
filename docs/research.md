# Research notes

What other tools learned the hard way, and what we took or didn't. Read on
2026-09-24. Where a site was unreachable from the build environment, the text
was read from a verbatim mirror or search index, and that's noted. Numbers are
the authors' own unless said otherwise.

## Protocols and editors

**Agent Client Protocol** (agentclientprotocol.com; spec repo and Rust SDK
read at HEAD, 2026-09-24). Stable wire version is 1. `session/new` takes
`mcpServers`, so a client can hand tools to any agent. `session/cancel` is a
notification; the agent must still answer the prompt with
`stopReason: "cancelled"`, and pending permission requests must be answered
`cancelled`. Permission requests are advisory: the agent decides whether to
ask. A v2 draft (2026-07-20) removes client `fs/*` and `terminal/*` and
moves turn state into notifications. 42 agents in the registry, including
Claude (`@agentclientprotocol/claude-agent-acp`), Codex, Gemini
(`gemini --acp`), OpenCode (`opencode acp`), Goose, Junie, Grok Build,
Copilot, Cursor.
*Took:* be a client; use only the v1 subset that survives v2; don't offer
fs/terminal; treat permissions as UX. *Didn't take:* the full SDK (runtime and
schema crates bring more than the ~10 messages we use).

**Zed** (docs and source at HEAD; blog posts were unreachable, read via
search index). 1.0 shipped 2026-04-29. Parallel agent threads with optional
per-thread worktrees; archive saves the worktree's git state. Zed sandboxes
only its own agent's terminal, not external ACP agents. DeltaDB/Delta (Aug
2026, private beta) versions edits and conversation together with anchors
that survive code movement.
*Took:* worktree per attempt; don't embed another editor's buffer model
(the old "why not embed Neovim" argument holds for any second editor core).
*Didn't take:* a CRDT store. Our provenance need is smaller: run → brief →
snapshot → evidence → accept commit, all by content id or commit.

**VS Code** (vscode-docs repo). Extension host isolation protects UI
responsiveness, not security. The 2018 text buffer rewrite lost to JS↔C++
round trips, not algorithms. Agent Host (2026-08-26): the agent runtime moved
out of the window process because closing a window killed sessions; state
is a snapshot plus ordered actions, replayed on reconnect. Its docs say a
worktree "is a Git code-isolation boundary, not a security boundary" and that
passing tests in each worktree doesn't prove the combined change works.
*Took:* runs are not owned by the window; checks run on the combined result;
say out loud that worktrees aren't a sandbox. *Didn't take:* a host
protocol; one process per run plus the database is enough locally.

**JetBrains** (blog read via search index). Fleet ended in Dec 2025: a
second general-purpose IDE "did not create enough value". Air (public
preview Mar 2026) runs several ACP agents with Local / worktree / Docker
isolation and uses the IntelliJ engine for code intelligence. IntelliJ
exposes its semantic index to external agents over MCP.
*Took:* don't try to out-IDE the IDEs; the value is around the agent, not in
another editor. *Open:* reusing an existing semantic engine is better than
building one, if we ever need one.

## Agent harnesses

**Cursor, "Scaling long-running autonomous coding"** (2026-01-14, read via
mirror). Agents coordinating through a shared file with locks: locks held too
long, forgotten, or skipped; "twenty agents would slow down to the effective
throughput of two or three". Optimistic concurrency made agents risk-averse.
What worked: each agent owns one thing, no peer chatter, results flow back as
handoffs. "Many of our improvements came from removing complexity."
*Took:* single owner per run; structured results instead of chat; no shared
mutable coordination state.

**Cursor, "What we've learned building cloud agents"** (~2026-05/06, read
via mirror). Work-stealing ran at "one 9"; Temporal got them past two.
Agent loop, machine state and conversation state decoupled; moved from
"eternal" workflows to short, task-scoped ones; missing environment shows
up as quietly worse output, not an error; logic moved out of the harness
into tools as models improved.
*Took:* task-scoped runs; lifecycle separate from transcript; verify the
environment instead of trusting output. *Didn't take:* Temporal; the
failures that forced it (node loss, pod replacement) don't exist locally,
and git already gives us receipts.

**Cursor, "Agent swarms and the new model economics"** (2026-07-20). Split
brain: two planners deciding the same thing differently, "a disagreement"
merge tooling can't fix. Fix: decisions in docs, code carries checked
references to them.
*Took:* decisions as first-class records linked to the code they govern.

**Anthropic, "Effective harnesses for long-running agents"** (2025-11-26).
Compaction wasn't enough. What worked: a progress file, a feature list with
a pass/fail field the agent may only flip, git, a smoke test at the start of
each session. Failure modes: declaring victory early, marking features done
without end-to-end testing.
*Took:* all of it, moved out of prompts into the tool: Kitsu owns pass/fail,
the agent can't flip it.

**OpenAI, "Harness engineering"** (2026-02-11, read via mirror). One big
AGENTS.md "failed in predictable ways"; a short map plus docs as the system
of record worked; rules promoted into lints with remediation text.
*Took:* short brief, rules as checks. **"Unlocking the Codex harness"**
(2026-02-04): cross-provider protocols "converge on the common subset". A
fair warning for ACP, and why we use only its core.

**OpenCode** (anomalyco/opencode @ 6df0d5d). SQLite session store with
durable events (per-aggregate sequence, replay) separate from live pub/sub.
The legacy SSE stream has no resume ids. Large tool output is truncated for
the model and saved to a file with a path (bounded and addressable).
LSP is off by default. Permission rules allow/ask/deny; no OS sandbox.
*Took:* durable events are the feed, with sequence numbers; bounded logs with
the full file addressable. *Didn't take:* LSP-by-default.

**Grok Build** (xai-org/grok-build @ f0e3be1, crate 1.0.41). Compaction,
rewind and memory reacquisition are separate mechanisms with separate
ledgers; rewind across a compaction replays the durable log. Subagents get
worktrees; cancel propagates to children by default, otherwise they're
backgrounded and keep running. Sandbox via Landlock/Seatbelt, default off.
Telemetry and trace upload are resolved from remote settings.
A wire-level analysis of 0.2.93 (cereblab gist, July 2026) captured repo
bundles being uploaded; the author's update says xAI disabled it server-side
on July 14. The author lists what wasn't tested; we didn't reproduce it.
*Took:* keep checkpoint, compaction and recovery separate; make child
lifetimes explicit. *For later:* an egress check with fake secrets.

## Cloud dev environments

**Gitpod, "We're leaving Kubernetes"** (2024). Dev environments are
stateful, bursty and need broad permissions; Kubernetes assumes the
opposite. **GitHub Codespaces** (2021): 45 min to ~10 s came from prebuilds
and deferring history, i.e. moving cost earlier and hiding staleness.
**CodeSandbox**: fast fork and cheap discard from CoW snapshots.
*Took:* worktrees are stateful; never reclaim them silently; keep discard
cheap. *For later:* warm worktrees keyed by base commit and lockfile hash,
with staleness shown.

## Papers

- **SWE-EVO** (arXiv 2512.18470, v6 2026-05). 48 release-note-driven tasks;
  the same model scores 72.8% on SWE-bench Verified and 22.9% here. Scores
  regressions, not just the target. n=48.
- **FeatureBench** (2602.10975, ICLR 2026). Feature tasks spanning commits;
  "passed" far above "resolved": plausible code that breaks other features.
- **SWE-Bench Pro** (2509.16941). Context overflow and endless re-reading
  are measured failure modes; its Docker images leaked future commits
  (issue #93), a reminder that the environment is part of the eval.
- **What Does Context Compression Cost an Agent?** (2608.16370). Completion
  flat, retrieval calls up in 6/6 comparisons when execution-relevant state
  is dropped. Measure reacquisition, not just pass rate.
- **TRACE** (2608.06503). Judge a compaction by what the agent does after it.
- **Implicit compression for SWE agents** (2605.11051). Latent compression
  that works single-shot fails over multiple steps.
- **Agent Retrieval Bench** (2607.24882). No retrieval family dominates.
- **MAST, Why Do Multi-Agent Systems Fail?** (2503.13657). Most failures are
  specification and coordination, not capability. Older models.
- **Ouroboros** (2608.08311, razzant/ouroboros). A self-modifying agent kept
  sane by controls outside what it can edit (constitution, review gate, halt,
  spend caps) and benchmarks on frozen snapshots; reward hacking found in its
  own trajectories. Self-reported results.

What they support: long-horizon work is much harder than single issues;
losing state costs reacquisition even when success looks flat; verifiers get
gamed; nothing proves one retrieval method. What they don't: that an
invariant-preserving tool raises success rates. That's the experiment we
still owe (`.kitsu/tasks/live-agent-smoke.md`).

A note on "Ouroboros": the original brief for this project meant a private
Agent OS project whose source wasn't available here. The public paper above
is a different project with the same name; its lessons are included for
what they're worth, not as a stand-in.

## Harness, memory and orchestration code, read in September 2026

Sixteen repositories read at their HEAD on 2026-09-24, for mechanisms and
for the hacks that stand in for missing contracts. The findings that
changed Kitsu are in `docs/architecture-review.md`; the separation they
argue for is decision `dag-state-memory`.

- **Coding harnesses** (Codex, OpenCode, Pi, Cline, Grok Build, DeepSeek
  Harness). What survives compaction differs per agent: Codex keeps user
  messages verbatim up to 20k tokens (none in its token-budget mode),
  OpenCode and Pi summarise all but a recent tail. In all of them rules
  files are re-rendered, not summarised. Completion is a model claim
  (Codex `/goal`), or an LLM panel that fails open (Grok). Permissions are
  keyed on tool names (OpenCode's read-only agents keep `bash`), default to
  allow (OpenCode, Cline), or enforce read-only with command blacklists
  (Cline). Worth taking: Codex Guardian's failure semantics (fail closed,
  stale-authorisation check, circuit breaker), Pi's `replay: never|safe`
  tool contract, Grok's deterministic post-compaction reminder, DeepSeek's
  "a rule in AGENTS.md names the script that enforces it".
- **Memory runtimes** (Letta, Hermes). Both run a background writer that
  rewrites what the next session loads, merged without human review by
  default, and both let memory hold executable procedures. Hermes's own
  compaction eval found Jev-based compaction no better than recency at
  equal budget, and a mechanical identifier index beating the summariser on
  exact facts.
- **Memory systems** (mem0, Graphiti/Zep, cognee). mem0's open-source write
  path is add-only and never resolves contradictions; Graphiti's bi-temporal
  invalidation is real but switches itself off when a date is missing, and
  default search returns invalidated facts; cognee's supersession tags are
  read by nothing. None of their benchmarks (DMR, LoCoMo, LongMemEval,
  BEAM) tests facts tied to code that go stale when the code changes.
- **Orchestration** (LangGraph, Temporal via pydantic-ai, AG2). The
  recurring failure is one structure doing two jobs: the transcript as the
  plan, the log and the memory (AG2 classic, the Temporal example), a
  message channel that is state and memory at once (LangGraph), writable
  context that routing reads (AG2's `context_vars`).
- **Docs** (Graphify, Structurizr, LikeC4, adr-tools, MADR). Graph
  communities churn too much to be architecture elements (one added line
  moved 14% of nodes); no existing C4 format parses without a JVM or a
  young grammar, so a Kitsu-native model with checks against code is
  proposed, exporting to LikeC4 and Mermaid.
