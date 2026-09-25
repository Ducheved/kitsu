# How Kitsu works

This is the design as built, not a wish list. Where something isn't done or
isn't known, it says so. The executable parts of the design (checks and
what they guard, decisions, next tasks) live in [`.kitsu/`](../.kitsu) and are checked by
Kitsu itself.

## 1. The problem, narrowed

Agents are good at a single step. Across many steps they lose the
architecture: the ownership rule from step 3, the retry budget from step 5,
the fact that step 7 was already verified. Today the human carries all of
that and re-teaches it. Kitsu's job is to carry it instead, in a form that:

- survives compaction, a crash, a model switch and a different agent;
- is small enough to hand to the next step without the whole history;
- can't be quietly rewritten by the agent it constrains;
- separates what was *verified* from what was *claimed*.

It is not trying to make models smarter, run a thousand agents, or replace
your editor. It is trying to make the state outside the model explicit,
owned and honest.

## 2. What Kitsu owns, and what it doesn't

Every piece of state has one owner. If two things could both claim it, one of
them is a projection.

| State | Owner | Kitsu's role |
|---|---|---|
| Source code, history, branches | git | reads; commits snapshots on `kitsu/run/*`; fast-forwards on accept |
| Tasks, checks, decisions, questions | git (files in `.kitsu/`) | parses, validates, links; never stores a copy |
| Whether a task is open or done | the task file (`state`), set by a human | writes it only when you accept-and-close or close it yourself |
| Task status (ready, running, review, blocked) | nobody: derived on every read | computes it |
| Runs, their lifecycle, custody | `state.db` | sole writer via transactions |
| Evidence (check results) | `state.db`, keyed by check fingerprint + tree hash | runs checks, records results |
| Check logs, briefs | content-addressed blobs next to the db | writes once |
| The agent loop, model, tools, its own memory | the agent (external, over ACP) | none; replaceable |
| The same, for Kitsu's own loop | Kitsu: the brain owns the conversation, the host owns tools, journal and `finish` | see `crates/kitsu/src/agent/` and decision `own-loop` |
| Unsaved editor text | the window | saves with a version check |
| Language intelligence | not in the first cut | see `.kitsu/decisions/no-index-no-lsp-yet.md` |
| Process isolation | nobody yet | documented gap; `.kitsu/tasks/sandbox-linux.md` |

Layout:

```
<repo>/.kitsu/                    intent, tracked by git
<git-common-dir>/kitsu/
  state.db                        runs, evidence, events, asks, integrations
  blobs/                          logs and briefs, by SHA-256
  worktrees/<run>/                one checkout per run (invisible to git status)
  integrate/<id>/                 scratch checkouts for building accept candidates
  runs/<run>/brief.md, agent.log  what the agent was told, its stderr
  owners/<instance>.lock          held for the life of each Kitsu process
```

## 3. A run, start to finish

```
kitsu run T  ──►  row inserted (starting)          ← intent before any effect
            ──►  git worktree add                   ← reconcilable: the path exists or not
            ──►  brief compiled, stored as a blob   ← what the agent was told, forever
            ──►  agent spawned, ACP initialize + session/new
            ──►  running; session/prompt with the brief
                    ├─ session/update  → a few semantic events/sec, batched
                    ├─ request_permission → policy, or a human ask
                    └─ stop (row says stopping) → session/cancel within a tick
            ──►  prompt returns stopReason → finished
            ──►  stdin closed; drain the rest of stdout until EOF
            ──►  worktree snapshot commit; changed paths + tree stored once
            ──►  required checks run on the snapshot → evidence
```

The reducer in `run.rs` is the only thing that decides transitions, and it
is tested over every (state, event) pair:

| from \ event | Started | StartFailed | TurnEnded | CancelRequested | Exited | ProtocolError | OwnerLost |
|---|---|---|---|---|---|---|---|
| starting | running | failed | illegal | stopping | illegal | failed | interrupted |
| running | illegal | illegal | finished | stopping | failed | failed | interrupted |
| stopping | (same) | failed | finished | (same) | failed | failed | interrupted |
| finished | illegal | illegal | duplicate | illegal | (same) | (same) | (same) |
| failed / interrupted | illegal | illegal | illegal | illegal | (same) | (same) | (same) |

"Finished" only means the agent returned from its turn. It says nothing
about whether the work is right. That's what evidence is for.

### Failure walks

**Agent succeeds, Kitsu crashes before recording it.** The worker dies; its
owner lock is released by the OS. The run row still says `running`, and
nothing pretends otherwise. The next Kitsu process to start (or `kitsu
recover`) sees a dead owner, stops the orphaned agent if it can prove the pid
is still that agent (Linux: `KITSU_RUN=<id>` in `/proc/<pid>/environ`),
marks the run `interrupted`, and snapshots the worktree. The work is kept and
reviewable; the run is not called finished, because we don't know that it
was. Tested by `killing_the_run_process_leaves_an_honest_interrupted_run`.

**Completion delivered twice.** The reader keeps the ids it has answered. A
second response to `session/prompt` is recorded as
`protocol.duplicate_response` and ignored; a response to an id we never
issued is a protocol violation and ends the run. At the run level, a second
`TurnEnded` is `duplicate`, recorded, no state change. Tested by
`a_second_completion_is_recorded_not_believed`.

**You cancel as the agent completes.** Cancel is recorded first (running →
stopping), then forwarded. If the agent's `end_turn` arrives anyway, the run
is `finished` with stop reason `end_turn` and `cancel_requested = true`:
what actually happened, not what was asked for. If the agent ignores cancel,
it's killed after a grace period and the run is `failed` with that reason.
Tested by `stop_cancels_a_cooperative_agent` and
`an_agent_that_ignores_cancel_is_killed_after_the_grace_period`.

**Your branch moves while an agent works.** The run's base stays what it was.
Accept builds the candidate by squashing the run onto the *current* head,
runs the required checks on that combined tree, then fast-forwards only from
the exact head it built on. If you committed meanwhile, the fast-forward
fails and nothing lands. If the combination conflicts, you get the paths and
can continue the run on top of the new head. Tested by
`accept_refuses_when_your_branch_moved_into_a_conflict` and
`checks_run_on_the_combined_result_not_just_the_worktree`.

**Crash in the middle of accept.** The integration row moves
preparing → verifying → applying → applied, and `applying` is written before
the ref moves. Recovery for a dead owner: anything before `applying` is
abandoned (nothing changed on your branch); `applying` is settled by asking
git whether the candidate is an ancestor of the branch head. Git is the
receipt. Tested by `an_interrupted_integration_is_settled_by_asking_git`.

**Compaction drops a rule mid-task.** Kitsu can't see inside the agent's
context. What it can do: the brief lives at `$KITSU_BRIEF` for the agent to
re-read, every run starts with a fresh brief instead of a long conversation,
and the rule's check runs on the result regardless of what the agent
remembers. The naive fixture agent is exactly this failure, and it is caught
at review.

**A rule file is broken.** Parse problems are never dropped. The brief lists
them under a warning ("the constraints in these files are NOT in this
brief"), status shows them, and accept refuses until they're fixed, because
the checks they define can't be enforced. Tested by
`a_broken_rule_file_blocks_accept_and_is_loud_in_the_brief`.

## 4. The brief

The brief replaces "remember everything" with "compile what applies, say
why, say what's missing":

1. The task, its body, and what done means (its checks, plus every check
   whose `guards` overlap the task's scope, each with the reason it's
   required, its `why`, and its current status on the base commit).
2. Broken rule files, loudly.
3. Decisions that apply, with rejected alternatives. This is where "don't
   add infinite retry" lives.
5. Questions about the task: answers if answered; if open, "don't guess".
6. Earlier attempts: who, outcome, files changed, check results, the note
   they were given.
7. How to work here: isolated worktree, how to ask, how to propose a
   decision, what's protected.
8. What was left out for budget and how to read it.

Scope overlap is by literal path prefix, so it can include things that don't
really apply (safe) but never misses a rule whose paths intersect. It *can*
miss a rule that a change breaks without touching its paths; checks on the
combined result are the backstop, and `.kitsu/tasks/semantic-scope.md` is
the experiment to find out how often that happens.

The brief is deterministic (tested), built without a model, and stored per
run. That is also the answer to "can agent B continue agent A's task?":
`kitsu run T --from <A's run> --agent B` gives B the same rules, A's changes
already in the worktree, and A's recorded outcome. What B does not get is A's
private reasoning, and it doesn't need it.

## 5. Evidence and freshness

A check is a named shell command. Its **fingerprint** is a hash of the
command and timeout, so editing the command invalidates old results. Each
run of a check records: fingerprint, the tree hash it ran on (computed from
the working directory including untracked files, via a scratch index so your
staging area is untouched), the tree hash after, outcome, exit code,
duration and a log blob.

On a given tree a check is:

- **current**: evidence exists for this exact tree;
- **carried**: evidence is for another tree, but nothing inside the check's
  declared `scope` changed since (only if a scope is declared);
- **stale**: evidence is for another tree and relevant files changed (it
  lists them);
- **unverified**: never ran with this definition.

If the tree changed while the check ran (a test writing files, you editing),
the result is recorded but bound to no tree. The CLI names the files that
moved so you can gitignore build output.

## 6. Trust boundaries

| Boundary | What crosses | What protects it |
|---|---|---|
| Repository → Kitsu | rule files, check commands | nothing runs until `kitsu trust`; agent commands never come from the repo |
| Agent → your checkout | nothing by design | separate worktree; **not enforced by the OS** |
| Agent → rules | edits to `.kitsu/**`, protected paths | shown as rule changes; approval bound to a hash of that diff; checks come from your checkout |
| Agent → Kitsu | ACP messages | bounded lines (32 MiB), unknown fields ignored, protocol violations end the run |
| Agent text → window | messages, titles | escaping Markdown subset; no HTML, no links |
| Window → system | typed commands only | no fs/shell/http plugins; paths checked against the repo root (symlinks resolved); CSP |
| Kitsu → git hooks | Kitsu's own commits | hooks disabled |
| Kitsu → network | nothing | Kitsu makes no network calls; agents make their own |

What this does **not** protect against, stated plainly: an agent can run any
command you could, read your home directory, and reach the network. The
permission policy (`ask` asks before commands and anything outside the
worktree; `auto` allows commands inside it) is approval UX that the agent
chooses to consult. Containment is the `sandbox-linux` task.

## 7. Concurrency and ordering

- **One owner per run**, a process, holding an OS lock. Recovery only acts on
  runs whose owner's lock is free, so any number of Kitsu processes can run
  recovery at once.
- **All writes are `BEGIN IMMEDIATE` transactions.** Deferred read-then-write
  transactions fail with SQLITE_BUSY under contention without waiting; the
  300-run probe found that.
- **Git worktree mutations are serialized** across processes with a lock
  file. Concurrent `git worktree add` can make other processes' `git worktree
  list` fail; the same probe found that.
- **Ordering** of events is the SQLite sequence, never timestamps. The agent
  stream is read in order; the reader's `select!` is biased toward the
  stream so updates that arrived before the prompt response are recorded
  before the completion.
- **Backpressure**: the ACP reader feeds a bounded channel; if Kitsu falls
  behind, it stops reading and the agent blocks on its pipe. Check logs keep
  the first 256 KiB and last 4 MiB. The window gets a dirty flag, not a
  stream. A check in `.kitsu/kitsu.toml` fails the build if an unbounded
  channel appears.
- **Wake-ups**: `stop` and answers are written to the database first, then
  the owner gets SIGUSR1 (pid from its lock file, trusted only while the lock
  is held). A 1 Hz poll covers lost signals and platforms without them.

## 8. Persistence and write discipline

Every write has a consumer. There are no writes caused by time passing.

| Write | When | Consumer |
|---|---|---|
| run row + `run.created` | once per run | status, recovery |
| run state transition + event | per transition (≤ ~6 per run) | status, digest, recovery |
| agent events | batched: ≤ 4 transactions/s per live run; tool calls on start and status change only; message text per ~8 KiB or 1.5 s; model reasoning never stored | run timeline |
| snapshot commit, tree, changed paths | once per run | review, status (no git on refresh) |
| evidence | once per check run | status, brief, accept |
| ask / answer | per permission request | the waiting worker |
| integration states | ≤ 5 per accept | recovery |
| `meta` last-seen | on user action, skipped if unchanged | digest |

`synchronous = NORMAL` in WAL mode: durable across process crashes; a power
loss can drop the last transactions. Every effect is reconcilable against
git or the filesystem, so the worst case is a run whose row is missing and
whose worktree shows up as an orphan.

Known open issue: events are ~400 bytes each and nothing prunes them yet.
Normal use is small; a very busy day could reach gigabytes.
`.kitsu/tasks/event-retention.md`.

## 9. Performance

Budgets, and what they measured on one machine: Intel Xeon @ 2.8 GHz,
4 vCPU, 15 GiB, Linux 6.18, release build, the scripted agent (so this is
Kitsu's share, not a model's).

| | Measured | Budget |
|---|---|---|
| Launch to a running agent | ~50 ms | — |
| Full run incl. snapshot and two Python checks | ~340 ms (Kitsu's part ~50 ms) | — |
| Status refresh, 100 runs in review | 1.9 ms (was 471 ms before storing snapshot facts) | 50 ms |
| 100 concurrent runs, failures | 0 | 0 |
| 100 idle supervisors, CPU | 0–1.2% of one core across runs (was 8.4% with a 10 Hz poll) | 5% |
| Supervisor memory | ~7 MB RSS each | 12 MB |
| 1,001 tasks: load rules / status / brief | 10 ms / 4 ms / 4 ms | — |
| 10,000 tasks: load rules / status | 102 ms / 47 ms | — |
| Event ingest, one SQLite file | ~14,900 tx/s (3.7× what 1,001 busy streams need) | — |
| 300 and 500 concurrent real runs | 0 failures (after the two fixes above) | — |
| Desktop UI startup bundle | ~100 KB JS (36 KB gzipped); the editor loads on first use | — |

`cargo run --release -p kitsu --example scale -- stress 100` reproduces the
budgeted row and exits non-zero if a budget is blown; CI runs it.

Not measured yet: editor typing latency and diff open time inside the real
webviews (WebKitGTK, WebView2, WKWebView). Those are the numbers that would
justify or kill the Tauri choice (kill criteria in
`.kitsu/decisions/tauri-window.md`).

About scale: the realistic heavy case is one person with a handful of agents,
maybe a dozen. The 100-run profile is a stress gate that keeps per-run costs
honest, not a use case.

## 10. What got deleted before it was built

Each of these was in the brief for this project. Each failed "delete it:
what breaks?"

| Removed | Why it can go | What would bring it back |
|---|---|---|
| Our own agent harness | ACP gives lifecycle, streaming, permissions, cancel | a guarantee we need about individual tool calls |
| MCP server for agents | files (`.kitsu/questions/`, decisions) and `$KITSU_BRIEF` cover asking and re-reading; checks run by Kitsu, not the agent | agents that can't write files; ACP v2 client tools |
| Vector DB / "memory" | the state that must survive is typed and linked by scope | a retrieval experiment showing a real gain |
| LSP integration | no measured win over compiler + tests for our queries | review wants diagnostics on the snapshot |
| Event bus / live stream | durable events at a few per second *are* the live feed; the UI pulls on a dirty flag | a view that needs token-level streaming |
| A daemon | one process per run gives custody and lifecycle independence | runs on other machines |
| Heartbeats | OS file locks tell liveness exactly | a remote owner |
| Stored task status | derived in 2 ms | never, on purpose |
| Workflow engine (Temporal etc.) | local, single-owner lifecycles with git as the receipt store | distributed execution |
| A separate ADR folder | decisions are `.kitsu/decisions/`, checked and linked | — |
| A DAG document | tasks with `after` are the DAG; `kitsu next` walks it | — |

## 11. Pivot map

| If this turns out wrong | Blast radius | Cheap now |
|---|---|---|
| Path scopes miss too many broken rules | module (P1): `scope.rs`, `brief.rs`, `status.rs` | scopes are just globs; symbol scopes can be added as another scope kind |
| SQLite can't keep up / needs sharing across machines | substrate (P3): `store.rs` | one module owns all SQL; schema is versioned |
| WebView is too slow for editing | substrate (P3): `app/` only | the window owns no state; the CLI is complete without it |
| ACP fragments or v2 changes turn semantics | module (P1): `acp.rs`, `runner.rs` | the client uses a small, tolerant subset |
| Files in the repo are the wrong home for intent | domain contract (P2) | intent is read through one loader for dirs and commits |
| The product frame is wrong (people want agents in their editor, not a new window) | frame (P4) | the CLI and the rule files work from any editor today |

## 12. Known gaps

- Real agents haven't been run through it; only the scripted one.
- No OS sandbox.
- macOS and Windows: not built, not run.
- No event retention.
- Accepting while an open question blocks the task only warns.
- Brief budget is characters, not tokens.
- `kitsu brief` from inside a run's worktree needs `kitsu` on PATH; the path
  in `$KITSU_BRIEF` always works.
