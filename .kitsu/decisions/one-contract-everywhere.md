+++
title = "One contract, enforced in every agent's hooks and in CI, judged by diff"
state = "accepted"
scope = ["crates/kitsu/src/contract.rs", "crates/kitsu/src/hooks.rs", "crates/kitsu/src/intent.rs", "crates/kitsu/src/check.rs", ".github/actions/kitsu/**"]
rejected = [
  "An ACP proxy between the editor and the agent: Claude Code and Codex speak ACP only through adapters, ACP permissions are advisory, and it only helps editors that route through the proxy. The vendors' own hooks already block, with semantics a proxy can't enforce",
  "Enforcing by intercepting tool calls only: a script, an interpreter or `git apply` writes files without naming them, and a hook that times out lets the call through. The pre-tool gate is a convenience; the stop gate and CI classify the diff, which doesn't care how a file was written",
  "An LLM judging whether the work is done or the rules were respected: a model's verdict is another claim. Done is a check result on a tree hash; a rule change is a path in the diff",
]
+++
`.kitsu/kitsu.toml` (checks, what each `guards`, `[protect]` paths) is the
one contract. `kitsu gate stop` / `pre-tool` enforce it inside Claude Code,
Codex and Cursor through their own hook protocols, `kitsu ci` enforces it on
a pull request, and `kitsu diff` shows it to a person. All of them compute
the change by content (tree hashes, uncommitted files included) from a base,
judge it by the base's rules, reuse evidence already recorded for the exact
tree, and name rule changes by the same diff hash `kitsu accept --approve`
takes.

The base for a hook is `--base`, else where a linked worktree forked from
your checkout's branch, else the merge base with the upstream, else `HEAD`.
Commits on a branch with no upstream are not seen by the hook; CI sees them.

What this does not give you, said where it matters:

- Hooks are guardrails, not a sandbox. An agent with a shell running as you
  can edit its hook config (that is a rule change the next gate and CI
  flag), run `kitsu gate approve` (the pre-tool gate refuses it when it can
  see it), or read held-out checks' files in your config dir. A held-out
  check is out of sight, not secret; receipts say "could have peeked".
- Each vendor caps how often a stop hook can send the agent back (Claude 8
  in a row, Cursor `loop_limit`); the gate caps itself at `--max-blocks`
  (5) and then lets the agent stop with "not verified". After that, CI is
  the gate.
- On its own errors the gate fails open and says so.
- Approving a rule change in CI is an input (`approved-rule-diff`), not a
  PR comment or label yet.
