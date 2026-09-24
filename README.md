# Kitsu

Kitsu is a workspace for doing real engineering work with coding agents,
where you stay the one who decides. It keeps track of the things agents
lose between steps: what the task is, what must stay true, what was decided
and what was rejected, and what has actually been verified. The agents
themselves are the ones you already use (Claude, Codex, Gemini, OpenCode,
anything that speaks [ACP](https://agentclientprotocol.com)).

It's early. The core loop works end to end and is tested hard on failure
cases; the rough edges are listed at the bottom.

## The problem

You ask an agent to fix the retry loop that's making outages worse. Step
one goes great. Twenty minutes and one context compaction later it "fixes"
a timeout by retrying forever, generates a fresh idempotency key on every
attempt (so a lost response becomes a second charge), and tells you all
tests pass. You catch it, explain the rules again, remind it what's already
done, and decide what it should do next.

At that point you're the agent's memory, its scheduler and its QA. A bigger
context window doesn't fix this, and neither does a memory plugin. The
problem isn't recall. It's that nobody owns the state, so it lives in your
head.

## What Kitsu does about it

```
 your rules (.kitsu/)        a task                 an agent (ACP)
        │                      │                         ▲
        └──── compiled into a brief with reasons ────────┘
                                                         │ works in its own git worktree
                                                         ▼
                 Kitsu snapshots it and runs the checks itself
                                                         │
     you review: files, checks, anything that touches the rules
                                                         │
   accept = squash onto your branch, re-check the combined result,
            fast-forward only if your branch didn't move
```

- **Rules are files in your repo.** Tasks, invariants, decisions (with the
  alternatives you rejected) and open questions are small Markdown files in
  `.kitsu/`. They're versioned, branched and reviewed with the code, and
  readable without Kitsu.
- **Every run gets a fresh brief.** It's compiled from the rules whose scope
  overlaps the task, with the reason each one is included, plus what earlier
  attempts did. No model writes it, so agent B gets exactly what agent A got.
  That's what makes a task hand-off-able.
- **Agents can't mark anything done.** Kitsu runs your checks on the agent's
  snapshot. "Tests pass" in the chat means nothing; the evidence row does.
- **Evidence is tied to content.** A check result belongs to the exact tree
  it ran on. Edit a file the check covers and it goes stale, and it tells you
  which file.
- **You're judged by your rules, not the agent's.** If a change edits a test
  or an invariant, you see it as a rule change and have to approve that
  exact diff.
- **Nothing lies after a crash.** Kill anything at any point. Interrupted runs
  stay interrupted with their partial work kept, orphaned agents get stopped,
  and whether an accept landed is answered by git, not guessed.

## Try it (no API key needed)

Kitsu ships a scripted ACP agent so you can see the whole loop without a model.

```sh
# Rust 1.98.1 is pinned in rust-toolchain.toml
cargo build --release -p kitsu

# A copy of the retry-storm example
cp -r fixtures/retry-storm/repo /tmp/payments && cd /tmp/payments
git init -q -b main && git add -A && git commit -qm init
export PATH="$OLDPWD/target/release:$PATH" AGENTS="$OLDPWD/fixtures/retry-storm/agents"

kitsu trust                        # allow checks and agents to run here
kitsu status
kitsu brief bounded-retries        # what an agent would be told

# An agent that bounds the retries but makes a new key per attempt:
KITSU_TEST_SCRIPT=$AGENTS/naive.toml kitsu run bounded-retries --agent test
kitsu status                       # ready for review, failing: idempotency
kitsu accept <run>                 # refused: the invariant's check fails

# One that gets it right:
KITSU_TEST_SCRIPT=$AGENTS/good.toml kitsu run bounded-retries --agent test
kitsu review <run> --diff
kitsu accept <run>                 # squashed onto main, task closed
```

`agents/cheat.toml` "fixes" the problem by weakening the test. Try accepting
that one.

## With a real agent

```sh
kitsu agents                       # presets, from the ACP registry
kitsu run <task> --agent claude    # or codex, gemini, opencode, goose, ...
```

The preset commands assume the agent's CLI or its ACP adapter is installed
and logged in. Override or add agents in `~/.config/kitsu/agents.toml`:

```toml
[agents.claude]
command = ["npx", "-y", "@agentclientprotocol/claude-agent-acp"]
```

Agent commands only ever come from your config, never from the repository.

## The desktop app

```sh
cd app && npm ci && npm run build && cd ..
cargo build --release -p kitsu-app --features custom-protocol
./target/release/kitsu-app ~/code/your-repo      # or run it from inside the repo
```

One list of work sorted by what needs you, one focused view per task. Start
an agent, answer its questions, watch it, review the change with its checks,
accept. It's all keyboard-driven: `j`/`k`, `⏎`, `r` run, `s` stop, `a`
accept, `x` discard, `n` new task, `⌘K` for everything, and a CodeMirror
editor with vim keys (`:w`, `:q`, `:e`). Press `?` for the rest.

Runs started from the window are ordinary `kitsu run` processes. Close the
window and they keep going.

Opened in a plain browser (`npm run dev`), the UI runs on fixture data and
says so in the corner.

## What's in `.kitsu/`

```
.kitsu/kitsu.toml                  checks and protected paths
.kitsu/tasks/bounded-retries.md    what to do, scope, which checks mean done
.kitsu/invariants/one-key-per-charge.md
.kitsu/decisions/retry-budget.md   the choice, and what was rejected and why
.kitsu/questions/*.md              open unknowns; they block tasks
```

```markdown
+++
title = "One idempotency key per logical charge, reused by every retry"
scope = ["payments.py"]
checks = ["idempotency"]
decision = "retry-budget"
+++
The upstream can charge the card and then lose the response. The only thing
that makes a retry safe is sending the same key again...
```

This repository uses Kitsu on itself; see [`.kitsu/`](.kitsu) for its own
invariants, decisions and what's next.

## CLI

| | |
|---|---|
| `kitsu status` | what needs you, what's running, what's ready |
| `kitsu next` | tasks that can start now, in dependency order |
| `kitsu new task "..."` | also `decision`, `invariant`, `question` |
| `kitsu brief <task>` | the brief an agent would get |
| `kitsu check [names]` | run checks, record evidence |
| `kitsu run <task> --agent X` | start an agent in its own worktree |
| `kitsu stop <run>` | ask it to stop (cancel reaches the agent right away) |
| `kitsu answer <id> <answer>` | answer a live permission request or an open question |
| `kitsu review <run> [--diff]` | files, checks, rule changes |
| `kitsu accept <run>` / `discard` | land it or throw it away |
| `kitsu log [-f]` | the event log |
| `kitsu recover` | settle anything left behind by a crash (also automatic) |

Add `--json` to any of them.

## What's verified and what isn't

Verified here means an automated test or a measurement in this repo does it.

| | |
|---|---|
| Brief compilation, scopes, evidence freshness, the run state machine (every state × event pair) | unit tests |
| Crash mid-turn, hang + stop, ignored cancel, duplicate completion, garbage on the wire, `kill -9` of the worker, permission asks, conflicts, combined-result checks, weakened tests, broken rule files, hand-off between agents, interrupted accepts | 18 end-to-end tests through the real binaries |
| 100 concurrent runs: 0 failures, ≤1.2% of one core idle, 2–3 ms status refresh | `examples/scale.rs stress`, in CI |
| Desktop app on Linux (WebKitGTK): open repo, review, accept with `a`, start a run | driven by hand under Xvfb |
| UI flows and screens | Chromium on fixture data |
| **Real agents** (Claude, Codex, Gemini adapters) | **not yet**: only the scripted agent. Protocol details beyond ACP v1 basics are untested |
| **macOS and Windows** | **not built or run yet**. Stop on Windows falls back to a 1 s poll; orphan cleanup is Linux-only |
| **Sandboxing** | **none**. Worktrees isolate changes, not processes. Agents run with your permissions |

## Docs

- [`docs/design.md`](docs/design.md): how it works, what owns what, what
  happens when things fail, measurements.
- [`docs/research.md`](docs/research.md): what we learned from Zed, Cursor,
  VS Code, JetBrains, OpenCode, Grok Build, CDEs and recent papers, and what
  we didn't take.

## License

MIT
