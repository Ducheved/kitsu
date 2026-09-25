+++
title = "Typed judgments: probabilities from TypeSafe System One, unknown when there are none, and never more than a skipped question"
state = "proposed"
scope = ["crates/kitsu/src/judge.rs", "crates/kitsu/src/agent/host.rs", "crates/kitsu/src/store.rs", "crates/kitsu/src/agents.rs"]
rejected = [
  "Unknown as no, deny or a default: each caller has its own careful path (for permissions: ask you; for guard flags it will be: require the check), so `judge.rs` returns unknown and lets the caller take it",
  "Blocking calls the judge calls risky, as TypeSafe's AutoMode middleware does: here the judge can only skip a question you would have been asked, never refuse, so a wrong 'risky' costs one question and a wrong 'safe' is what the threshold and the screen are for",
  "Letting the judge allow what a rule refuses or always asks (paths outside the worktree, .git, network, privileges, arguments that don't parse): those never reach it",
  "Tool arguments or file contents in a question's `instructions` (the API takes any JSON there): instructions and criteria are `&'static str`, so only the data field can carry what was computed at run time",
  "Cutting data to fit the request window: the cut could be the part that matters; a request over 64 KiB is not sent and the judgment is unknown",
  "Storing the judged data in state.db: tool arguments are already in `tool.begin`, file contents can hold secrets; the SHA-256 of the request tells whether two judgments saw the same thing",
  "Turning auto-allow on from agents.toml alone: configuring the judge changes nothing until a run asks for `--policy triage`",
  "Retrying timeouts (the SDK does, up to 30 s): someone is waiting for the ask; a timeout already spent its attempt, and all attempts together get twice the per-attempt timeout",
  "Triage for ACP agents now: their permission requests are answered inside the run's event loop, where a judgment of up to 20 s would stall updates; they keep `ask` until that loop can wait on one",
  "The Python SDK in a subprocess, or OpenRouter's route to Jev (what Hermes measured): a second runtime for one POST, or one more party that sees every judged command",
  "Jev for compaction or context pruning: Hermes measured its ranking no better than recency at a matched budget (77.8 vs 77.8), and its default threshold dropped 100% of 851 candidates (SCORECARD-2026-09-19-jev)",
]
+++
Some decisions Kitsu makes are semantic, not lexical: is this command
harmless, is this chunk relevant to the task, could this diff break what a
check guards. `judge.rs` asks those as typed questions (yes/no, a choice
from a closed set, a score) and gets probabilities back, from TypeSafe's
System One (`POST /v1/systemone`, `Authorization: Bearer`, model
`jev-latest`; shapes from typesafe-sdk `_schemas/models.py`). No `[judge]`
in agents.toml, no `$TYPESAFE_API_KEY`, an error, a timeout, a malformed
or missing answer: the verdict is `Unknown`, a value of its own.

Every judgment is a row in `judgments` (state.db v3) and a `judge.done`
event: purpose, question kinds, inputs hash, answers with probabilities
(or why none), model, latency, attempts, tokens, request id. System One
reports tokens, not cost ($0.042 per million input tokens through
OpenRouter in Hermes' measurement; output tokens are free per the API
schema). Retries: a refused connection, 408, 429, 500, 502, 503, 504, at
most `retries` (default 2), honoring `retry-after-ms` / `retry-after`
only while the overall budget lasts.

First use: `kitsu run --policy triage` with Kitsu's own loop. A shell
command that `ask` would put to you is first screened by the host (a path
outside the worktree, `.git`, the network, package installs, privileges:
you decide, the judge isn't asked); otherwise the judge is asked whether it
is low-risk and confined to the worktree, with the command as data. At or
above the threshold (default 0.9) with a stored judgment, it runs and the
`permission` event names the judgment. Below it, unknown, or unscreened
trouble: you are asked, and the ask shows the probability or the reason.

Triage saves questions; it is not a security boundary. The screen is a
word list that `python -c`, `sh -c "$(…)"` or a script the agent just wrote
walk around, and the judge only reads the command line. With no sandbox, a
prompt-injected or hostile agent is exactly as dangerous under `triage` as
under `auto`; use it only where you'd use `auto`, and keep `ask` for
untrusted input until `sandbox-linux` lands.

The threshold starts high because calibration on this question is
unmeasured: in Hermes' compaction eval Jev's probabilities for a keep
question never passed 0.20. `judge-live-eval` decides: false-allow rate on
a labeled set of tool calls.

Revisit if: the live eval shows any false allow of a destructive or
outside-the-worktree command at the default threshold (raise it or drop
triage), or asks saved are under a third of what `ask` would ask (not worth
a network call per command). Each later use (rerank, guard flags, routing)
brings its own eval before it is on by default.

Comes from decisions `unknown-stays-unknown`, `no-sandbox-yet` and
`status-is-derived`: a judgment may spare you a question or (later) add a
required check, never remove a requirement, refuse for you, or mark
anything done.
