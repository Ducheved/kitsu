+++
title = "Stay fast on a 1M-line monorepo or 20 repositories, and keep agents' searches off every core"
scope = ["crates/kitsu/src/index.rs", "crates/kitsu/src/runner.rs", "app/src/components/Explorer.svelte", "app/src-tauri/src/commands.rs"]
checks = ["test", "ui"]
+++
The index reads tracked files only, so node_modules and build output are
out by construction; measured on openai/codex (8.6k files) at 5.6 s cold,
25 ms warm. What doesn't scale yet:

- The file tree fetches every path in one call and builds the whole tree.
  Needs lazy listing by directory past ~20k files.
- One index per repository. A workspace of several repositories needs one
  query over all of them.

Done: agents' CPU. Agents run their own ripgrep, and Claude Code's
native build runs it with `--no-config`, so a `RIPGREP_CONFIG_PATH` thread
cap never reaches it. Kitsu now starts every agent under `nice` and, on
Linux, `taskset` (all CPUs but one by default; `[limits]` in agents.toml);
children inherit both, and ripgrep sizes its thread pool from the CPUs it
may use. `fixtures/cpu-limits/bench.py`, 4 cores, three agents' bursts over
1M lines: foreground frame p99 24-28 ms unlimited, 9-14 ms at nice 10
(same wall time), 6-9 ms with one core left out (+10% wall time).
Not measured: `ionice` (the tree was in page cache, so it moved nothing
and isn't applied), macOS (nice only), Windows (nothing yet).

Gate for the rest: on a synthetic 1M-line workspace, no UI frame over
50 ms, versus today. Keep each piece only if it moves its number.
