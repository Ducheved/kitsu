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
- Agents run their own ripgrep. Kitsu can start agent processes with lower
  CPU and I/O priority (nice/ionice, a cgroup CPU quota where available)
  and a `RIPGREP_CONFIG_PATH` with a thread cap and ignores; whether an
  agent's bundled ripgrep honours it is unknown.

Gate: on a synthetic 1M-line workspace, peak agent search CPU ≤ 2 cores
and no UI frame over 50 ms while it runs, versus the unthrottled baseline.
Keep each piece only if it moves its number.
