# Eval suite: the same tasks, checks and model, three agents

Nine small task repos and a runner that puts each through `kitsu run` with
Kitsu's own loop (`kitsu-native`), OpenCode over ACP and Codex over ACP, all
on one OpenRouter model, then scores every run with tests the agent never
saw. It answers the question decision `own-loop` leaves open: does our loop
produce more verified work, or cheaper work, than the agents people already
use?

```
fixtures/eval/
  run.py               the runner (Python 3.11+, stdlib only)
  heldout_runner.py    runs the held-out tests on a run's snapshot
  <task>/
    EVAL.md            what the task measures and the expected outcome
    eval.toml          task id, expected outcome (done | blocked), files to restore
    repo/              the repository the agent gets: code, visible tests, .kitsu/
    heldout/           hidden tests, copied in only at scoring time
    dryrun/good, bad/  a right and a plausible wrong solution, for the dry run
```

## The tasks

| task | kind | what it tests |
|---|---|---|
| `off-by-one` | bug fix, failing test | fixes to the docstring's contract, not just the two visible cases (page < 1 must raise) |
| `parse-duration` | feature from a spec | implements a five-line spec, including what it rejects; a lenient parser passes the visible tests |
| `extract-formatter` | refactor | one `format_amount` for two formatting styles, output byte-identical; the naive merge puts separators into the CSV |
| `webhook-retry` | rule only in a decision | bounded retries are visible; the stable `Webhook-Id`, backoff and no-retry-on-4xx are only in decision `webhook-delivery` |
| `cent-short` | search across files | the float-rounding bug is in three files, only one has a test; analytics must stay float |
| `norway-invoices` | right outcome is **blocked** | the VAT rate isn't in the repo and a decision forbids guessing it; adding 25% and saying done is a false success |
| `cli-json-flag` | feature on a CLI | `list --json` to a five-point interface spec: sorting, key filtering, `[]`, exit 2 elsewhere |
| `stale-cache` | bug fix, contract | the visible test shows expiry; the docstring also requires LRU on get, expired-first eviction, live `len()` |
| `slow-dedupe` | performance + constraint | fast *and* still works on dict rows; `dict.fromkeys` passes the visible tests and breaks every real import |

Each repo is under 120 lines. Task ids and titles don't say what the trap is.
`python3 fixtures/eval/run.py --check-fixtures` proves every task is what it
claims: the base fails its visible checks (the blocked task passes them),
the good variant passes visible and held-out, the bad variant passes visible
and fails held-out.

## What a run records

Per (trial, task, agent), in that order so a budget stop hits every agent
alike: a fresh copy of the repo (`git init`, one commit), `kitsu run <task>
--agent <agent> --policy auto` with a wall-time limit (20 min), then:

- **claimed**: `done`, `blocked`, `no_change`, `stopped:<why>` or `failed`.
  `blocked` is Kitsu's `finish` with outcome blocked, or a new
  `.kitsu/questions/*.md` (every brief asks for one when a human is needed;
  it's the only "blocked" an ACP agent can express). A normal end with no
  change at all is `no_change` for every agent alike.
- **visible**: Kitsu's own check results on the run's snapshot (`kitsu review --json`).
- **held-out**: the snapshot extracted to a fresh directory, the original
  visible tests (and support files named in `restore`) copied back over it,
  the held-out tests copied in, all run with `python3 -I` so nothing in the
  tree can shadow the standard library.
- **solved** = claimed the expected outcome and held-out passes.
  **false success** = claimed `done` while held-out fails, or `done` on the
  blocked task; `false_success_visible_green` says Kitsu's visible checks
  passed it too.
- **cost_usd**: OpenRouter's `GET /api/v1/key` → `data.usage` after the run
  (polled until it stops moving) minus before. Anything that arrives later
  is added to that run as `late_cost`. `reported_cost_usd` and `tokens` are
  what the agent itself reported, for comparison.
- **requests, models_sent, reasoning_sent**: from a recording proxy on
  127.0.0.1 that every agent is pointed at. It forwards requests and
  responses unchanged and logs, per request, the model, the reasoning
  setting and the tool count (never a header). So "the same model" is
  checked, and ACP agents' model requests are counted. `--no-record` turns
  it off.
- **protected_touched**, **asked_human**, **wall_s**, **turns** (Kitsu's loop
  only), **tool_calls**, **last_message**, **peeked** (see below).

Output: `results/<timestamp>/runs.jsonl`, `summary.md` (rewritten after
every run), and per run `logs/<n>.{out,err,events.json,requests.jsonl}`.

## Budget

- The run's reserve is `600k input + 40k output tokens` at the model's price
  from OpenRouter's public list (`--est-input-tokens`, `--est-output-tokens`,
  or `--run-reserve-usd`), raised to 1.5x the most expensive run so far.
- A run doesn't start if spent + reserve > budget. A running one is stopped
  (`kitsu stop`) when its own spend passes 3x the reserve or the rest of the
  budget, checked every 15 s. If usage can't be read the suite stops.
- If the key has a credit limit below the budget, the limit is the budget.
  **Make a key for this with a $10 credit limit** on openrouter.ai/settings/keys
  and run nothing else on it: the key's usage is the only cost the runner
  trusts, and OpenRouter enforcing the limit is the backstop if everything
  here is wrong.
- The key is read from `$OPENROUTER_API_KEY` only. Everything written under
  `--out` is redacted and then searched for the key (exit 3 if found). The
  key endpoint's `label` is never read into the results.

## Setup (verified 2026-09-25)

```sh
cargo build --release -p kitsu            # the runner finds target/release/kitsu

# OpenCode 1.18.32; ACP is `opencode acp`
npm install -g opencode-ai@1.18.32
# Codex's ACP adapter 1.13.1 (bundles @openai/codex 0.156.1). The old
# @zed-industries/codex-acp is deprecated in favour of this one.
npm install -g @agentclientprotocol/codex-acp@1.13.1

opencode --version && codex-acp --version
python3 fixtures/eval/run.py --check-fixtures
python3 fixtures/eval/run.py --dry-run
```

Codex needs no login: with a custom `model_provider` whose key comes from
`auth.command`, it never asks for ChatGPT or OpenAI auth (checked against a
local stub: session started, `/v1/models` and `/v1/responses` called with
the key).

### What the runner writes for each agent

All generated into the run's scratch dir; nothing touches your own
`~/.config/opencode`, `~/.codex` or `~/.config/kitsu`.

`agents.toml` (per run, `KITSU_CONFIG_DIR`):

```toml
[agents.kitsu-native]
native = { provider = "openai-chat", base_url = "https://openrouter.ai/api/v1", model = "<model>", api_key_env = "OPENROUTER_API_KEY", turns = 60, tokens = 3000000, context_window = 200000, max_output = 16000 }

[agents.opencode]
command = ["opencode", "acp"]
env = { OPENCODE_CONFIG = "<work>/opencode/opencode.json", XDG_CONFIG_HOME = "<work>/opencode/config", XDG_DATA_HOME = "...", XDG_CACHE_HOME = "...", XDG_STATE_HOME = "..." }
pass_env = ["OPENROUTER_API_KEY"]

[agents.codex]
command = ["codex-acp"]
env = { CODEX_HOME = "<work>/codex-home", NO_BROWSER = "1" }
pass_env = ["OPENROUTER_API_KEY"]
```

(`base_url` is the recording proxy, `http://127.0.0.1:<port>/v1`, unless
`--no-record`; `context_window` is the model's, capped at 200k.)

`opencode.json`:

```json
{
  "model": "openrouter/<model>",
  "small_model": "openrouter/<model>",
  "enabled_providers": ["openrouter"],
  "provider": { "openrouter": {
    "options": { "apiKey": "{env:OPENROUTER_API_KEY}" },
    "models": { "<model>": { "options": { "reasoning": { "effort": "<default_effort>" } } } } } },
  "autoupdate": false, "share": "disabled", "permission": { "webfetch": "deny" }
}
```

`small_model` is the same model on purpose: OpenCode's title request would
otherwise go to a different one.

`$CODEX_HOME/config.toml`, per OpenRouter's Codex guide
(docs/cookbook/coding-agents/codex-cli):

```toml
model = "<model>"
model_provider = "openrouter"
web_search = "disabled"
model_reasoning_effort = "<default_effort>"   # only if the model has one

[model_providers.openrouter]
name = "openrouter"
base_url = "https://openrouter.ai/api/v1"

[model_providers.openrouter.auth]
command = "sh"
args = ["-c", "echo $OPENROUTER_API_KEY"]
```

Codex removed `wire_api = "chat"` (its source: "`wire_api = "chat"` is no
longer supported"); it speaks only the Responses API. OpenRouter serves one
at `/api/v1/responses` (GA, stateless: Codex sends `store: false`, which it
accepts), and a Codex-format model catalog at `/api/v1/models?client_version=...`,
which the `auth.command` form makes Codex fetch. So Codex can run on any
OpenRouter model; for non-OpenAI models OpenRouter translates.

## Running it

```sh
export OPENROUTER_API_KEY=...        # the dedicated, limited key
python3 fixtures/eval/run.py --suggest-models            # prices now, for 27 runs
python3 fixtures/eval/run.py --agents kitsu-native,opencode,codex --tasks all \
    --trials 1 --model <openrouter id> --budget-usd 10 --out results/
```

Start with one task and one agent (`--tasks off-by-one --agents opencode`)
to see a real run end to end before spending the rest. Runs are serial:
the per-run cost is a difference of the key's usage.

A smoke test with the real agents and no key: point everything at a local
OpenAI-compatible stub with `--base-url http://127.0.0.1:PORT/v1 --key-url
http://127.0.0.1:PORT/api/v1/key --run-reserve-usd 0.01` and a fake
`OPENROUTER_API_KEY`. That's how the configs above were checked.

## Fairness, and what differs on purpose

- **Same:** tasks, repos, briefs (Kitsu compiles one brief, whoever runs),
  checks, model, key, `--policy auto` (everything inside the worktree is
  allowed; anything outside waits for a human and so runs into the wall
  time), Kitsu's MCP server offered to both ACP agents, no web.
- **Different by design:** each harness's own system prompt, tools,
  context management and retry behaviour. That is what's being compared.
- **Reasoning effort.** Kitsu's loop sends no reasoning setting, so
  OpenRouter uses the model's `default_effort`; the runner pins OpenCode and
  Codex to that same value (`--reasoning-effort` to change it). The proxy
  shows what really went out, and it disagrees for OpenCode 1.18.32 on
  `openai/gpt-5.6-luna`: its main requests carry `reasoning: {effort:
  "none"}` (plus an unused top-level `reasoningEffort`) whatever
  `opencode.json` says; only its title request honours the setting. On
  models without effort levels (e.g. `minimax/minimax-m3`) none of the three
  sends one. Check "What each agent sent" in `summary.md` before reading
  the results.
- **No sandbox.** Agents run with your permissions, and the held-out tests
  and dryrun solutions are on the same disk. They are never in the agent's
  repository or its git history, the scratch dir is outside this checkout
  and named neutrally, and a run whose events mention `heldout`, `dryrun`,
  `fixtures/eval` or this directory is flagged `peeked` (`?` in the table).
  A determined agent could still find them; the flag makes it visible, it
  doesn't prevent it.
- **Blocked is asymmetric in the protocol.** Kitsu's loop has
  `finish(outcome = "blocked")`; an ACP agent can only write the question
  file the brief asks for. An ACP agent that says "I need finance" in chat
  and changes nothing is `no_change`, not `blocked`. Read `last_message`.

## Verified here, and not

Verified offline in this repository: all nine fixtures (`--check-fixtures`);
the whole suite in `--dry-run` (stub model for Kitsu's loop,
`kitsu-test-agent` in the ACP slots, fake key endpoint), also from
`cargo test` (`crates/kitsu/tests/eval_harness.rs`); OpenCode 1.18.32 and
codex-acp 1.13.1 starting under `kitsu run` with the generated configs,
through the recording proxy, against a local stub (model id, key
passing, Codex's catalog fetch and Responses request, OpenCode's two
requests).

Not verified: anything against OpenRouter itself (no key here), so:
OpenRouter's usage settling time; Codex on OpenRouter's Responses API with
a real model and real tool calls; OpenCode's tool calls through
`@openrouter/ai-sdk-provider`; whether Codex's own sandbox gets in the way
inside Kitsu's worktree (`.git` there is a file); real costs against the
estimate; how often agents peek.
