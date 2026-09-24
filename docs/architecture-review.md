# Architecture review: long-horizon memory, context and planning

An adversarial review of Kitsu against the "Agent OS / Long-Horizon Memory
/ Dynamic Planning" research brief (2026-09-24). The brief's rule applies
throughout: no mechanism without a demonstrated failure class and a
measurable contract it fixes. Claims are marked **verified** (code or a
test that fails without the fix), **smell** (reproducible, not yet a
failure), **hypothesis** or **unknown**.

Inputs: the code at the commit this file lands in; the six research
reports on Codex, OpenCode, Pi, Cline, Grok Build, DeepSeek Harness,
Hermes, Letta, mem0, Graphiti, cognee, LangGraph, Temporal (via
pydantic-ai), AG2 and Graphify (summarised in `docs/research.md` and the
decisions they produced). Papers named in the brief are treated as
hypotheses; arxiv.org is not reachable from the environment this was
written in, so paper claims are taken from the brief and from the
repositories that implement them.

---

## 0. What Kitsu is, in one paragraph

Kitsu does not run a model loop. External agents (Claude Code, Codex,
Gemini, OpenCode) run over ACP, one process and one git worktree per run.
Kitsu owns everything around the loop: what the task is (intent files in
git), what must hold (invariants with checks), what was decided and
rejected, what is unknown (questions), what earlier work learned (typed,
anchored memory notes), what actually happened (events, evidence bound to
tree hashes, in SQLite), and whether a change is accepted (a human, after
checks on the combined result). The model sees a brief compiled from that
state and can query it through `kitsu mcp`. This is the brief's
"authoritative evidence → deterministic projections → bounded context
view → model → external verification" shape, with one difference: the
bounded view is built per run, and within a run the agent owns its own
context.

---

## 1. Current-state reconstruction

The production path, from `kitsu run` (the app starts runs with the same
command, `app/src-tauri/src/commands.rs` `start_run`).

| Step | Owner | Durable state | Transaction boundary | Crash behaviour | Retry | Authority |
|---|---|---|---|---|---|---|
| User intent | human | `.kitsu/tasks/<id>.md` in git | the file / a commit | nothing to lose | n/a | the file; `TaskFront` rejects unknown fields (`intent.rs`) |
| Run created | `runner::prepare` | `runs` row `starting` + `run.created` event (with Kitsu version) | one `BEGIN IMMEDIATE` (`store.rs` `insert_run`) | row exists, no worktree → recovery marks `interrupted` | none automatic; a human re-runs (`--from`) | the reducer (`run.rs::reduce`) |
| Worktree | runner, under `worktrees.lock` | `.git/kitsu/worktrees/<run>` + branch | git | orphaned worktree is cleaned by recovery | none | git |
| Context build | `brief::compile` (deterministic, no model) | brief blob (content-addressed), id on the run | blob write then `set_run_brief` | the run has no brief → it never started the agent | n/a | intent at the base commit + store + the person's notes |
| Model decision | the external agent | none in Kitsu | outside Kitsu | unknown to Kitsu | the agent's | none: agent text is never a fact |
| Tool execution | the agent (its tools; Kitsu answers only permission asks and MCP reads) | `permission`, `ask.open/answered`, `agent.tool` (title, kind, status, paths; not the output) | batched appends, ≤ 4 tx/s | lost only if unflushed at crash (≤ 250 ms) | the agent's | the permission policy (`runner::decide`), never `allow_always` |
| Evidence | `check::CheckRun` (Kitsu runs the checks itself) | `evidence` keyed by `(fingerprint, tree)`; log head 256 KiB + tail 4 MiB | one insert | a check killed mid-run leaves no row → "not run" | a human re-runs | the tree hash (`git::worktree_tree`, flags that hide edits are cleared) |
| Next context | the agent | none in Kitsu | — | — | — | — |
| Compaction | the agent | none in Kitsu; ACP v1 can't report it | — | — | — | — |
| Re-orientation | `kitsu mcp` `orient` | read-only | none | — | idempotent | recorded state, rules from the main checkout |
| Recovery | `recover::reconcile` (every command) | owner locks, runs, integrations | per run | idempotent; asks git whether an accept landed (`is_ancestor`) | yes, it's a fold | the OS lock and git |
| Completion | human `accept` | squash commit, `integrations`, task `state = "done"` | ff-only CAS on the target | re-run of accept settles it from git | no | checks on the combined tree + human approval |

**Answers to the brief's §22 questions** (the ones that aren't in the
table):

- *Single authoritative source for a run:* the `runs` row, which is
  required to be the end of its `run.state` event chain (invariant
  `run-row-is-the-fold-of-its-events`, checked after every e2e scenario).
  For intent: git. For what the agent was told: the brief blob.
- *Duplicated state:* the run's snapshot tree and changed paths are cached
  on the row (`snapshot_tree`, `changed`); both are derivable from git and
  fixed once written. Nothing else is stored twice.
- *What survives a process crash:* everything committed to SQLite (WAL,
  `synchronous = NORMAL`: a power loss can drop the last transactions, a
  process crash can't), git, blobs. Agent output not yet flushed (≤ 250 ms).
- *What survives the agent's compaction:* whatever the agent keeps. Kitsu's
  brief stays on disk (`$KITSU_BRIEF`) and through MCP `brief`/`orient`.
  Whether each agent keeps the brief verbatim is **unknown** (§4).
- *Model-generated artifacts treated as facts:* none found. Agent text goes
  into `agent.message` events, shown to humans, never into status, never
  into the next brief (the "earlier attempts" section is machine-recorded;
  the accept-time evidence bug that made it misreport is fixed, §3).
- *How DONE is established:* checks Kitsu runs, on the tree they ran on,
  plus a human accept. "Finished" means the turn returned, not that work
  is done (`run.rs`).
- *How a goal is born and superseded:* a person writes a task file (or
  accepts an agent's proposed one through review). No conversation creates
  a task. A task is closed only by accept-and-close.
- *Proposal vs accepted task:* an agent can only propose files in its
  worktree; they land through review, where `.kitsu/**` is protected and
  approval is bound to the hash of that diff.
- *Rejected branches in active context:* a discarded run's worktree is
  gone; its record stays, and the next brief lists it as an earlier attempt
  with its outcome, never its prose.
- *Can retrieval block execution:* no. The index only feeds `search`; brief
  inclusion is by path scope.
- *Rebuildable stores:* `index.db` (a cache keyed by blob id), the cached
  snapshot fields. **Not rebuildable: `state.db`.** Its evidence and run
  history are unique; losing it loses what was verified and what happened,
  not what was intended or decided.
- *Large tool outputs:* the agent's own. Check logs keep 256 KiB of head
  and 4 MiB of tail; the middle is dropped and marked.
- *Retry semantics:* no automatic retry of any effect. Human re-runs are
  recorded as `from_run` lineage.
- *Invalid premise → downstream work:* the task DAG unblocks only on a
  dependency's human-set state; evidence goes stale when files in a check's
  scope change; memory notes go stale when their anchors change. There is
  no "decision D was revoked, so tasks built on it are suspect" (§5, F03).
- *Pinned snapshot vs ambient filesystem:* every run works on a worktree
  from a recorded base commit; evidence is bound to tree hashes, never to
  "the current directory".

---

## 2. Verified invariants

Each has a check or test that fails when it's broken.

| Invariant | Enforced by |
|---|---|
| Agents never write your checkout | worktree per run; accept is the only path in |
| Rules come from your checkout, not the candidate's | integrate reads intent from the target; `mcp` reads the main checkout |
| One owner per run | OS file lock; `killing_the_run_process_*` e2e tests |
| Status is derived, never stored | no status column; `status.rs` |
| Unknown stays unknown | "unverified", "not reported", "outcome unknown" states; token usage absent ≠ 0 |
| Every queue is bounded | check `bounded` (it caught one of this review's own test channels) |
| The webview has no generic powers | check `webview-powers` |
| Memory has no authority | check `memory-no-authority` + `tests/separation.rs` (adversarial notes change nothing but the brief) |
| Task files hold intent only | `tests/separation.rs`; check `intent-writers` |
| A run's row is the end of its transitions | `Store::run_history_mismatches` after every e2e scenario; a unit test with a direct UPDATE |
| Evidence is bound to the tree the check saw | `git::worktree_tree_sees_changes_git_was_told_to_ignore` |

---

## 3. Verified failures (found and fixed during this review)

Each fix has a test that fails on the old code.

1. **Evidence tree hash missed hidden edits.** `worktree_tree` copied the
   index; files marked assume-unchanged or skip-worktree (and edits hidden
   by `core.ignoreStat`/fsmonitor) weren't hashed, so a check that ran on
   modified content was recorded against the old content. Fixed in the
   scratch index only. (From Cline's checkpoint code, which documents the
   same trap.)
2. **The brief misreported earlier attempts.** Evidence tagged with a run
   includes accept-time checks on the combined tree; the brief printed them
   as the attempt's own result. A failed accept read as a failing attempt.
3. **Auto-answer could grant a standing permission.** When only
   `allow_always` was offered, policy picked it; in OpenCode that allows
   every edit in every session. Now it asks the human.
4. **`git cat-file --batch` could deadlock** when both pipes filled (ids
   written before output was read). Latent for the rules directory, real
   for the index.
5. **A note's `run` provenance accepted any string.** Now it must be a run
   id.

One failure is **not yet explained**: `check::tests::freshness_follows_scope`
failed once under heavy load and passed in 95 reruns. It is recorded as a
task with hypotheses, not labelled flaky.

---

## 4. Unknowns

- Whether each agent keeps Kitsu's brief verbatim through its own
  compaction. Codex keeps user messages up to 20k tokens (its token-budget
  mode keeps none); OpenCode and Pi summarise all but a 2k–20k tail. Needs
  the probe in §8 (E1).
- Whether `excludeDynamicSections` actually makes Claude Code's system
  prompt hit the provider cache across runs (decision `prompt-cache`).
- Whether agents use `kitsu mcp` tools when offered, and whether that
  reduces reads. Needs live runs.
- How codex-acp maps approval policy, sandbox mode and compaction.
- What OpenCode does when Kitsu refuses its `fs/write_text_file` call (it
  never checks client capabilities; the rejection is unhandled).
- Whether a ripgrep config injected through `RIPGREP_CONFIG_PATH` reaches
  the ripgrep bundled inside agents.

---

## 5. The failure taxonomy (brief §17) against Kitsu

| # | Failure | Kitsu today | Status |
|---|---|---|---|
| F01 | Architecture intent lost after compaction | invariants re-sent only in the brief; `orient`/`rules_for` re-fetch on demand | hypothesis: depends on the agent (E1) |
| F02 | Rejected hypothesis survives as fact | agent prose never enters state or the next brief | holds by construction |
| F03 | New evidence invalidates descendants | evidence staleness by scope; no link from a revoked decision to tasks built on it | **gap**, no observed failure yet |
| F04 | False DONE | agents can't mark done; checks are run by Kitsu | holds (tested) |
| F05 | Wrong retrieval blocks the right file | retrieval only feeds `search` | holds |
| F06 | Retrieval overfits path vocabulary | path words give a bonus; eval has 6 identifier and 28 prose questions | **smell**: eval written by the index author, no path-word ablation yet |
| F07 | Fabricated task commitment | tasks exist only as files a human wrote or accepted | holds |
| F08 | Summary loses uncertainty | Kitsu writes no summaries | n/a (agent side) |
| F09 | Summary loses negative evidence | failed attempts listed in the next brief with outcome | holds |
| F10 | Cross-session gotcha forgotten | memory notes, `gotcha` kind first in the brief | holds if someone writes the note |
| F11 | Stale project rule | notes: anchors + git staleness, `retired`, `supersedes`; invariants: `retired` | holds for notes; rules rely on review |
| F12 | Skill overgeneralisation | no skills; notes are scoped | n/a |
| F13 | Retry loop without progress | no detection | **gap**; gate: doom-loop signal (3 identical tool calls) must fire in ≥ 30% of rejected runs and ≤ 5% of accepted |
| F14/F15 | Recovery starts too late / too early | a human chooses `--from`; nothing automatic | holds by not automating |
| F16 | Memory construction lag | notes are files; nothing is consolidated in the background | n/a |
| F17 | Memory poisoning by model prose | a note lands only through review, shown as "proposed note" | holds |
| F18 | Index corruption | `index.db` is a cache; any version mismatch rebuilds it | holds |
| F19 | Conflicting memories | `key` conflicts are a reported problem; neither wins by recency | holds for declared keys |
| F20 | Tool output too large | check logs bounded head/tail with a marker; MCP results cut at 24 KiB with a marker | holds for Kitsu's own outputs |
| F21 | User correction fails to supersede old goal | task file edit; answered questions flow into the brief | holds |
| F22 | Plan becomes authority | agents' plans are `agent.plan` events for display | holds |
| F23 | Reflection becomes fact | no reflection stored | holds |
| F24 | Procedure skips verification | no executable memory | holds |

---

## 6. Mechanisms: integration matrix

"Decision" is one of *already solved*, *investigate* (with an experiment),
or *reject for now*. Nothing is marked "adopt" without evidence.

| Mechanism (source) | Failure it targets | Evidence the failure exists here | Minimal implementation | Simpler alternative | Kill criterion | Decision |
|---|---|---|---|---|---|---|
| Append-only event journal (Scroll, DeepSeek) | lost history | none: events already append-only | — | — | — | already solved |
| Lossless evicted context with pointers (Scroll, ACM) | F20, reacquisition | agent-side | `orient` + brief blob path; artifact ids for evidence logs | — | — | partly solved; investigate artifact ids for check logs (E3) |
| Typed bounded context per decision (AgenticSTS, CAT) | whole-transcript prompts | per-run brief already typed with why/left-out | — | — | — | already solved at run granularity |
| Agent-managed compaction (CAT, ACM, ReSum) | compaction at bad moments | agent-side; Hermes measured Jev compaction no better than recency | — | — | — | reject: not Kitsu's layer |
| Deterministic re-orientation packet (Grok) | F01 after compaction | unknown (E1) | MCP `orient` (shipped) | brief file | E2 shows no drop in reacquisition → remove from brief text | investigate |
| Provenance on derived memory (Agent Zero, graphiti) | F17 | self-reported `by`/`run` | derive "landed via run R" from integrations | show the commit author | no mismatch in 30 live runs → drop | investigate |
| Non-destructive supersession (All-Mem, graphiti) | F11, F19 | a wrong note could only be deleted | `state`/`supersedes`/`key` (shipped) | — | — | done |
| Markdown truth + rebuildable index (tigerless) | F18 | — | exactly Kitsu's layout | — | — | already solved |
| Execution-state tree / branch validity (MAGE) | F02, F14 | no observed failure: attempts are separate runs with lineage | — | `from_run` + discard | — | reject for now |
| Decision premises with invalidation (MAGE-lite) | F03 | none observed | `premises = [...]` on decisions; mark dependents suspect | review | no case in 3 months of use | reject for now |
| AND/OR trees (STRUCTUREDAGENT) | alternatives in plans | tasks have `after` only; alternatives are separate tasks | — | two tasks and a question | — | reject for now |
| Temporal knowledge graph (Zep) | F11 queries | commit-space validity covers the queries seen | — | git + supersedes | — | reject for now |
| Vector memory (mem0, A-MEM, MemoryOS) | recall | retrieval eval: all misses are ranking misses | — | FTS5 | ≥ +10 recall@5 on a held-out set | reject for now |
| Background consolidation / Dreamer (Letta, Hermes, Grok v2, Codex) | F10 at scale | 1 real note in this repo | `kitsu memory doctor` (deterministic) → a run kind that proposes diffs | agent writes a note during its run (the brief asks) | gates in decision `dag-state-memory` | investigate after ≥ 20 notes |
| Learned skills with promotion gates (Voyager, ReasoningBank) | F10 procedures | none | — | checks and decisions are the procedures | — | reject for now |
| Self-reflection stored (Reflexion) | — | — | — | — | — | reject: reflection is a hypothesis, not state |
| Workflow engine (Temporal, Burr, LangGraph) | durability | covered by the OS lock + git receipts + the reducer | — | — | runs across machines | reject |
| Typed judgments (Jev) | semantic decisions (scope misses, permission triage) | no labelled data yet | `judge-core` with stored judgments, fail to "unknown" | rules | per use: its own eval gate | investigate, blocked on a key |

---

## 7. Minimal target architecture

What exists already is the target; the additions are small and each has a
gate.

```
 git: intent (tasks, invariants, decisions, questions, notes)
  |                                  \
  |                                   `--> index.db (cache, by blob id)
  v
 state.db: runs + events + evidence + asks + integrations   (append-mostly;
  |                                                           row = end of its chain)
  v
 derived: status, freshness, supersession, required checks  (never stored)
  |
  v
 brief (per run, stored)  +  kitsu mcp (orient, search, rules_for, memory)
  |
  v
 external agent over ACP (its own loop and compaction)
  |
  v
 checks run by Kitsu on the snapshot, then on the combined tree
  |
  v
 human accept  --> git
```

Missing pieces, in the order they'd be built: a compaction probe (E1),
stored judgments (`judge-core`), a turn budget if live runs show silent
agents, a doom-loop signal if recorded runs justify it.

---

## 8. Experiments

Every experiment reports a baseline, a treatment, and a kill threshold.

| # | Question | Fixture | Baseline | Treatment | Metric | Success | Kill |
|---|---|---|---|---|---|---|---|
| E1 | Does the brief survive each agent's compaction? | scripted long session against a **local recording stub model** (no API spend): Claude Code, OpenCode, Codex pointed at it | default | rules also sent through the agent's instruction channel | invariant texts verbatim in post-compaction requests | N/N (N ≥ 5) per agent | default already N/N → don't wire channels |
| E2 | Does `orient` reduce reacquisition? | retry-storm + 2 more fixtures, forced compaction | brief only | brief + MCP | re-reads of already-read files, re-runs of current checks before first edit | ≥ 30% fewer, pass rate not lower, paired and order-alternating | < 10% → drop the brief line |
| E3 | Do evidence log pointers help after failures? | fixtures with a failing check | log tail in brief | artifact id + `evidence` tool | tokens, correct fix rate | fewer tokens at equal fix rate | no difference |
| E4 | Held-out retrieval | questions written by someone other than the index author, path-word ablation | FTS5 | +embeddings | recall@5, per identifier/prose | +10 points | < +10 |
| E5 | Dreamer vs in-run notes | fixtures with a planted gotcha | agent writes a note during its run | dream run proposes notes | repeat of the same check failure next task | fewer in ≥ 6/10 pairs and ≥ baseline + 2 | otherwise drop |
| E6 | CPU of agents' searches on 1M SLOC | 10 repos, 1M SLOC | agent defaults | `nice`/cgroup quota + `RIPGREP_CONFIG_PATH` threads | peak cores, p95 UI frame time | ≤ 2 cores, no UI jank | no measurable change |

E1 and E6 need no API budget. E2, E3, E5 need live runs (question
`live-inference-budget`).

---

## 9. Migration sequence

1. (done) Separation invariants, supersession, fold check, version stamp,
   MCP read-only tools, the five fixes in §3.
2. E1 with a stub model server; wire instruction channels only where it
   fails.
3. Scale: an index over several repositories, a lazy file tree for 100k+
   files, CPU limits for agent processes (E6).
4. Architecture model (`.kitsu/architecture/`, `kitsu arch check`) and docs
   tasks, from the docs research: its gate already fired once.
5. `judge-core` once a key exists; each judgment use behind its own gate.
6. E2/E3/E5 when there is a live budget.

---

## 10. Kill list: not building

- An own agent loop or compaction. It's a race of model-specific shims
  (Pi carries "Opus 4.6 sends edits as a string"; Codex parses leniently
  "because gpt-4.1").
- A vector database or an LLM-extracted knowledge graph as "memory".
- Any memory write that lands without review, any memory with scripts,
  "authoritative" memory in prompts.
- LLM judges as completion authority, and any verification that fails open.
- Command blacklists as a read-only guarantee; default-allow permissions.
- A workflow engine beside the run supervisor.
- A memory bank or agent-maintained progress file as state.
- Automatic retries of agent turns.

---

## 11. Open research questions

- Is path scope enough to find the rules a change can break? (`semantic-scope`
  measures the miss rate before anything semantic is built.)
- At what note count does a deterministic `memory doctor` pay for itself,
  and does an LLM Dreamer ever beat notes written during the run?
- Can a single brief serve agents with very different compaction (Codex
  keeps user messages; OpenCode keeps a tail), or do rules need a
  per-agent channel?
- Where does ACP v1 lose information Kitsu needs (compaction events,
  usage), and is that worth an adapter extension?
