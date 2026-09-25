# Kitsu

**Agents do the work. Checks decide. You accept.**

Kitsu is an IDE and an agent harness for doing real engineering work with
coding agents without becoming their memory, scheduler and QA. It runs its
own agent loop, or the agents you already use (Claude, Codex, Gemini,
OpenCode, anything that speaks [ACP](https://agentclientprotocol.com)), each
in its own git worktree, and keeps what they lose between steps: the task,
the rules, what was decided and rejected, and what has actually been
verified.

## Three rules Kitsu is built on

1. **The checks decide, not the agent.** "Done" is a check result recorded
   on the exact tree it ran on. What an agent says about its work is never
   evidence: a false "all tests pass" costs a refused accept, not an outage.
2. **Kitsu owns the state; the model is replaceable.** Intent lives in
   files in your repo, execution in a journal Kitsu writes before every
   effect. Any agent can pick up any task, compaction can't drop your rules,
   and a crash resumes without repeating a single side effect.
3. **Nothing lands without you.** You review the diff, with every change to
   a rule, test or check shown as a rule change you approve by its exact
   diff. Accept re-checks the combined result and fast-forwards only if your
   branch didn't move.

Anything that would break one of these is out, however convenient; the
[roadmap](docs/roadmap.md) says what that ruled out and why.

It's early. The core loop works end to end and is tested hard on failure
cases; what isn't verified yet is listed at the bottom.

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

- **Rules are files in your repo.** Tasks, decisions (with the alternatives
  you rejected) and open questions are small Markdown files in `.kitsu/`;
  checks and the paths each one guards are in `.kitsu/kitsu.toml`. They're versioned, branched and reviewed with the code, and
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
  or a check, you see it as a rule change and have to approve that
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
kitsu accept <run>                 # refused: the check guarding payments.py fails

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

## Kitsu's own agent

Through an external agent, Kitsu can only put advice into someone else's
context: after Claude Code compacted its conversation, none of a brief sent
as a message survived. So Kitsu also has its own loop. It speaks three wire
formats (`provider`): OpenAI-compatible Chat Completions (OpenRouter, a
local server; the default), Anthropic Messages and OpenAI Responses:

```toml
[agents.kitsu]
native = { base_url = "https://openrouter.ai/api/v1", model = "<model id>", api_key_env = "OPENROUTER_API_KEY" }
# optional: context_window, max_output, turns (per run), tokens (per run)

[agents.claude-api]
native = { provider = "anthropic-messages", base_url = "https://api.anthropic.com", model = "<model id>", api_key_env = "ANTHROPIC_API_KEY", context_window = 200000 }

[agents.openai]
native = { provider = "openai-responses", base_url = "https://api.openai.com/v1", model = "<model id>", api_key_env = "OPENAI_API_KEY" }

# The key from `kitsu login openrouter` instead of a variable; works with
# all three formats through OpenRouter.
[agents.kitsu-or]
native = { provider = "anthropic-messages", base_url = "https://openrouter.ai/api", model = "anthropic/<model>", auth = "login:openrouter" }
```

`base_url` follows each vendor's convention: Anthropic's without the
version (`/v1/messages` is appended), OpenAI's with it (`/chat/completions`
or `/responses`). Plain `http` only to this machine.

```sh
kitsu run <task> --agent kitsu
kitsu run <task> --agent kitsu --from <run> --resume   # after a crash or a budget stop
kitsu login openrouter    # sign in in the browser; the key goes to the OS keychain
kitsu logout openrouter
```

What it does differently:

- **The brief never leaves the context.** It's a fixed system message; the
  run's state (what changed, where each required check stands on the files
  right now, budget left) is re-rendered as the last message of every
  request. Compaction touches only the conversation: old tool outputs become
  pointers, older steps a digest rebuilt from the journal.
- **Done is a request.** `finish` makes Kitsu run the required checks; a
  failure goes back to the model, and after three refusals the run stops.
- **Every effect is journaled before it happens.** After a crash, `--resume`
  settles a write by the file's hash and never re-runs a command whose
  outcome is unknown.
- **Tools instead of a shell,** with closed schemas and bounded output; the
  shell is one tool among twelve, under the same policy and CPU limits.
- **The key is named, not stored.** It's read from `api_key_env` or the OS
  keychain (`auth = "login:<provider>"`), sent in one header, and never
  written to the journal, logs or the tools' environment. `kitsu login`
  uses the provider's own sign-in for third-party apps (OpenRouter's OAuth
  PKCE, which issues a key you can revoke on openrouter.ai) and fails
  without a keychain rather than writing the key to a file. Anthropic and
  OpenAI don't offer third-party apps a sign-in with a Claude or ChatGPT
  subscription; to use one, run their own agent (`--agent claude`,
  `--agent codex`), which signs in itself.
- **Same loop, any format.** The conversation, the journal, compaction,
  resume and loop detection are the same for all three; a provider only
  renders the request. Anthropic requests carry cache marks on the harness,
  the brief and the conversation, never on the ledger; Responses runs
  stateless (`store: false`) and sends reasoning items back as they came,
  from the journal after a resume.

The loop is a brain (model calls) and a host (tools, journal, checks) that
talk only in JSON-RPC, so the brain can later run elsewhere while your
machine keeps the hands.

`--policy auto` lets the agent run commands in its worktree without asking,
except that commands reaching the network, changing the refs every worktree
shares (`git push`, `update-ref`, `branch -D`, ...), installing packages or
raising privileges still wait for you. It is not a sandbox.

`--policy triage` (experimental) is `ask`, except that a shell command
TypeSafe's System One rates low-risk and confined to the worktree, at or
above a threshold, runs without asking you. It's off until configured:

```toml
[judge]                  # the key is read from $TYPESAFE_API_KEY
[judge.permissions]
threshold = 0.9
```

No judge, an error, a timeout or a lower probability: you're asked, and
the ask shows the probability or why there is none. Commands naming paths
outside the worktree, `.git`, the network, package installs or `sudo` are
never sent to the judge. Every judgment is stored and the run's log says
which one let a command through.

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

Several projects share one window. The rail header switches between them
(`⌘O`, or `⌥1`…`⌥9`) and shows what needs you in each; Settings → Projects
adds, renames, reorders and removes them, and `g b` shows a project's
branches and worktrees, Kitsu's run worktrees marked with their task. The
list is `workspaces.toml` in the Kitsu config dir; every per-project thing
(rules, runs, trust) stays in that project. Every command the window sends
names its project, so nothing lands in the wrong one after a switch.

Runs started from the window are ordinary `kitsu run` processes. Close the
window and they keep going.

Opened in a plain browser (`npm run dev`), the UI runs on fixture data and
says so in the corner.

## What's in `.kitsu/`

```
.kitsu/kitsu.toml                  checks, what each guards, protected paths
.kitsu/tasks/bounded-retries.md    what to do, scope, which checks mean done
.kitsu/decisions/retry-budget.md   the choice, and what was rejected and why
.kitsu/questions/*.md              open unknowns; they block tasks
```

```toml
[checks.idempotency]
run = "python3 -m unittest -q test_idempotency"
scope = ["payments.py", "fake_upstream.py", "test_idempotency.py"]  # evidence goes stale when these change
guards = ["payments.py"]                                            # any change here needs it to pass
why = "One idempotency key per logical charge, reused by every retry."
```

A check's `run` is a POSIX `sh -c` command on every platform, and so is the
native loop's `shell` tool. On Windows that's the `sh.exe` from Git for
Windows (found on PATH or next to `git`), or the one `KITSU_SH` names.
Without one, checks are recorded as `error`, never as passing: Kitsu does
not fall back to `cmd.exe`, which would read `echo ok; exit 1` as a
command that succeeds (decision `sh-everywhere`).

This repository uses Kitsu on itself; see [`.kitsu/`](.kitsu) for its own
checks, decisions and what's next.

## CLI

| | |
|---|---|
| `kitsu status` | what needs you, what's running, what's ready |
| `kitsu next` | tasks that can start now, in dependency order |
| `kitsu new task "..."` | also `decision`, `question`, `memory`, `element` |
| `kitsu brief <task>` | the brief an agent would get |
| `kitsu check [names]` | run checks, record evidence |
| `kitsu run <task> --agent X` | start an agent in its own worktree |
| `kitsu stop <run>` | ask it to stop (cancel reaches the agent right away) |
| `kitsu answer <id> <answer>` | answer a live permission request or an open question |
| `kitsu review <run> [--diff]` | files, checks, rule changes |
| `kitsu accept <run>` / `discard` | land it or throw it away |
| `kitsu log [-f]` | the event log |
| `kitsu recover` | settle anything left behind by a crash (also automatic) |
| `kitsu workspaces [list\|add\|remove]` | the projects the window shows; removing one never touches its files |

Add `--json` to any of them.

## What's verified and what isn't

Verified here means an automated test or a measurement in this repo does it.

| | |
|---|---|
| Brief compilation, scopes, evidence freshness, the run state machine (every state × event pair) | unit tests |
| Crash mid-turn, hang + stop, ignored cancel, duplicate completion, garbage on the wire, `kill -9` of the worker, permission asks, conflicts, combined-result checks, weakened tests, broken rule files in your checkout or in the change, hand-off between agents, interrupted accepts | 26 end-to-end tests through the real binaries |
| 100 concurrent runs: 0 failures, ≤1.2% of one core idle, 2–3 ms status refresh | `examples/scale.rs stress`, in CI |
| Desktop app on Linux (WebKitGTK): open repo, review, accept with `a`, start a run | driven by hand under Xvfb |
| UI flows and screens | Chromium on fixture data |
| Kitsu's own loop: fixing the fixture task, false done ×3, compaction at the threshold and after an overflow, crash-and-resume at three points, loop signal, cancel, retries, path confinement, the key never written | 15 end-to-end tests against a scripted model server |
| Kitsu's own loop on a live model | **once**: the fixture task on OpenRouter; prompt caching measured (71–92% of the prompt from cache on turns 2–5, half the cost; one run, not repeated) |
| **Real agents** (Claude, Codex, Gemini adapters) | **partly**: Claude Code through `kitsu run` against a stub model (compaction probe) |
| **How good it is on real tasks**, own loop vs Codex vs OpenCode | **not measured yet**; an eval suite with held-out checks is in progress ([roadmap](docs/roadmap.md)) |
| **macOS and Windows** | **the CLI's test suite runs on both in CI; the app is not built there yet**. On Windows: checks need Git for Windows' `sh`; stop falls back to a 1 s poll; a stopped command's tree is killed with `taskkill /T`, but what it left running after its shell exited is not; orphan cleanup after a crash is Linux-only |
| **Sandboxing** | **none**. Worktrees isolate changes, not processes. Agents run with your permissions |

## Docs

- [`docs/roadmap.md`](docs/roadmap.md): what's being built now, next and
  later, and what we decided not to build.
- [`docs/design.md`](docs/design.md): how it works, what owns what, what
  happens when things fail, measurements.
- [`docs/research.md`](docs/research.md): what we learned from Zed, Cursor,
  VS Code, JetBrains, OpenCode, Grok Build, CDEs and recent papers, and what
  we didn't take.

## License

MIT
