# Roadmap

Every item says what you get and how we'll know it's done. Items that are
tasks in [`.kitsu/tasks/`](../.kitsu/tasks) name the task; the task file is
the source of truth, this page is the overview.

Everything here has to keep the [three rules](../README.md#three-rules-kitsu-is-built-on):
the checks decide, Kitsu owns the state, nothing lands without you.

## Now

| What you get | Done when |
|---|---|
| **An honest number for "how good is it".** Kitsu's own loop, OpenCode and Codex on the same small tasks, the same model and the same checks, scored by held-out tests the agent never saw (`live-agent-smoke`) | a table per agent: tasks passed on held-out checks, false "done" claims, cost from OpenRouter's own accounting, wall time; the first run within a $10 budget |
| **More providers for the own loop.** Native Anthropic Messages (with prompt caching) and OpenAI's Responses API next to the OpenAI-compatible chat API | the same end-to-end tests (finish, retries, crash and resume, overflow) pass against each |
| **Sign in instead of pasting keys** where the provider officially offers it to third-party apps (OpenRouter's OAuth PKCE); credentials in the OS keychain. A Claude or ChatGPT subscription is used through the vendor's own CLI over ACP, which signs in itself | `kitsu login openrouter` works end to end; no key or token in a file, log or journal |
| **Installers for Windows, macOS and Linux on every commit,** and releases versioned by the rules: SemVer from Conventional Commits, a changelog, tagged releases with checksums | a green run on all three platforms with downloadable builds; `kitsu --version` names the exact commit |
| **A task-graph constructor.** Build chains of tasks by dragging blocks; edges are `after`, and the result is ordinary task files | edit a graph in the app, see the diff of `.kitsu/tasks/`, accept it like any other change |
| **Typed judgments (Jev)** for the few decisions that are really semantic, starting with permission triage: low-risk calls inside the worktree can go through without asking you, everything else still asks, with the probability shown (`judge-core`, `judge-permissions`) | recorded-response tests for every case; a failed judgment always means "ask you". Measuring whether it helps needs a TypeSafe key (`typesafe-key`) |

## Next

| What you get | Done when |
|---|---|
| **A planner.** "Plan: <goal>" is an ordinary run whose only output is proposed task files, reviewed as a graph diff with anything that loosens a rule in red. A plan never applies itself | a plan run proposes tasks through a checked tool; bad proposals come back as errors; accepting needs your approval of that exact diff |
| **An autopilot** that starts ready tasks in parallel, each in its own worktree, never two with overlapping scopes at once, within a run/token/time budget. It launches runs and never owns them: kill it and they keep going; restart it and nothing starts twice | end-to-end tests for the limit, overlap, a killed and restarted autopilot, two autopilots racing |
| **Stacked runs.** B can start from A's verified but not yet accepted change, and is accepted only after A, showing only its own diff | tests for accept order and for A being discarded |
| **Many projects in one window.** Several folders and repos, a switcher, branches and worktrees per repo, per-repo settings; every command names its repo so nothing lands in the wrong one | open two repos, run in both, accept in one; the other is untouched |
| **A read-only explorer.** The own loop can send a helper to read and summarise part of the code in its own budget and context; it has no write tools, and what it returns is a summary with pointers, not a fact | journaled like any tool call; the parent's context grows by the summary only |
| **MCP tools, LSP diagnostics and a live conversation** in the own loop: talk to a running agent, see its output as it streams, get compiler errors right after an edit | each with end-to-end tests against the scripted model |
| **A sandbox** for the processes agents start, Linux first (`sandbox-linux`) | a check that tries to write outside the worktree or reach the network fails under the sandbox |

## Later

- **A cloud brain.** The own loop is already a brain and a host that talk
  only JSON-RPC; a remote brain needs a transport, and your machine keeps the
  hands: files, tools, checks, the journal.
- **Windows and macOS parity** for stopping agents and cleaning up orphans
  (`windows-support`).
- **Auto-accept**, only for paths you list in `kitsu.toml`, only when every
  required check passed on that exact change, and first in shadow mode: it
  says what it would have accepted until at least 30 of those match your own
  decisions.
- **Typed judgments beyond permissions:** checks a diff might break without
  touching what they guard, relevance of context before it costs tokens, a
  cheaper model for simple tasks (`judge-invariants`, `judge-rerank`,
  `judge-routing`), each behind its own measurement (`judge-live-eval`).

## Not doing, and why

| | Why not |
|---|---|
| **Subagents that write code** | they share a checkout and an owner with their parent, and their output gets trusted as fact. Parallel work goes through the task graph instead: each piece is a reviewed task in its own worktree. We'll revisit if runs keep ending on budget on tasks you later split by hand |
| **Tasks spanning several repos** | an accept can't move two repos atomically, and one check can't verify two trees |
| **Model-written briefs, memory as summaries** | the brief must be the same for every agent and reproducible from files; a summary replacing the record is how rules get lost |
| **Invariants as a separate kind of rule** | replaced by checks with `guards` and `why`: a rule nothing can run was a rule nothing enforced |
| **Retrying failed runs automatically** | a failure is information for you; an automatic retry spends budget to hide it |
